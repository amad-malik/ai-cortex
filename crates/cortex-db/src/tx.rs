//! `ModuleTx`: one transaction per module operation (CONTRACTS §3, r6 and r7).
//!
//! **Lock order (r6).** 1 `Idempotency`; 2 `ProducerGate` and `ReplyTarget`, ascending, before the
//! owner's first row lock; 3 the owner's row locks and its non-audit delegated writes;
//! 4 `AuditChain`, last. The ledger refuses a class lower than one held, and any class once
//! `AuditChain` is held. **The seal:** once `AuditChain` is taken inside `delegate(m, …)`, the
//! transaction is sealed; afterwards only `delegate(m, …)` again (the audit-scoped path: its
//! `executor()` and a re-take of the same chain's lock), `commit` and `rollback` remain.
//! `executor()` for the owner, `delegate` to any other module, `advisory_lock` of any other key,
//! and a second chain each return `LockOrder` and poison the transaction.
//!
//! **The audit scope is a delegation, not a module id** (rework 1, vera's
//! F1B-GREEN-OWNER-AUDIT-SEAL). `AuditChain` is taken only inside a `delegate`; asked for in the
//! owner's own scope it is `LockOrder { held: OwnerRows, wanted: AuditChain }` before any SQL, and
//! poisons. After the seal, the owner's own executor is refused even when the owner's module is
//! the audit module.
//!
//! **An abandoned delegation fails closed** (rework 1, vera's F1B-GREEN-DELEGATE-CANCEL). A guard
//! is armed before a delegation's first await and disarmed only when `delegate` returns. If the
//! `delegate` future is dropped first (a timeout, a `select!`), the transaction is abandoned:
//! `executor`, `delegate` and `advisory_lock` refuse with `DelegationFailed`, and `commit` rolls
//! back and refuses with `DelegationFailed`. Only rollback remains. **So does an interrupted lock
//! acquisition** (rework 7, sam's SAM-R2-02): a request already sent is not withdrawn when its
//! future is dropped (a timeout the caller catches), and the server may grant it later, unseen by
//! the ledger; so `advisory_lock` arms the same guard before it sends the request and disarms it
//! only once the ledger and the seal record the lock.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use cortex_types::{ModuleId, ProjectId, RequestId, WorkerId};
use sqlx::postgres::PgConnectOptions;
use sqlx::{Connection, Row};

use crate::error::DbError;
use crate::pool::{DbPool, ModuleRegistry, module_role};

#[cfg(feature = "testkit")]
fn nonlocal() -> bool {
    crate::testkit::nonlocal()
}
#[cfg(not(feature = "testkit"))]
fn nonlocal() -> bool {
    false
}
#[cfg(feature = "testkit")]
fn seal_off() -> bool {
    crate::testkit::seal_off()
}
#[cfg(not(feature = "testkit"))]
fn seal_off() -> bool {
    false
}

/// `begin`'s one round trip (Z-2's param form): the role, the project when given, the clock and the
/// statement timeout when given, then the backend pid (for cancel-on-drop) and the isolation level, read back
/// for the READ COMMITTED check.
/// The role and project are transaction-local except under the testkit nonlocal control.
/// The binds, in order: role, [project], now, [timeout].
fn begin_sql(project: bool, timeout: bool, local: bool) -> &'static str {
    match (project, timeout, local) {
        (true, true, true) => {
            "SELECT set_config('role', $1, true), set_config('cortex.project_id', $2, true), \
             set_config('cortex.now', $3, true), set_config('statement_timeout', $4, true), \
             pg_backend_pid(), current_setting('transaction_isolation')"
        }
        (true, false, true) => {
            "SELECT set_config('role', $1, true), set_config('cortex.project_id', $2, true), \
             set_config('cortex.now', $3, true), pg_backend_pid(), current_setting('transaction_isolation')"
        }
        (false, true, true) => {
            "SELECT set_config('role', $1, true), set_config('cortex.now', $2, true), \
             set_config('statement_timeout', $3, true), pg_backend_pid(), current_setting('transaction_isolation')"
        }
        (false, false, true) => {
            "SELECT set_config('role', $1, true), set_config('cortex.now', $2, true), \
             pg_backend_pid(), current_setting('transaction_isolation')"
        }
        (true, true, false) => {
            "SELECT set_config('role', $1, false), set_config('cortex.project_id', $2, false), \
             set_config('cortex.now', $3, true), set_config('statement_timeout', $4, true), \
             pg_backend_pid(), current_setting('transaction_isolation')"
        }
        (true, false, false) => {
            "SELECT set_config('role', $1, false), set_config('cortex.project_id', $2, false), \
             set_config('cortex.now', $3, true), pg_backend_pid(), current_setting('transaction_isolation')"
        }
        (false, true, false) => {
            "SELECT set_config('role', $1, false), set_config('cortex.now', $2, true), \
             set_config('statement_timeout', $3, true), pg_backend_pid(), current_setting('transaction_isolation')"
        }
        (false, false, false) => {
            "SELECT set_config('role', $1, false), set_config('cortex.now', $2, true), \
             pg_backend_pid(), current_setting('transaction_isolation')"
        }
    }
}

/// The lock order's rank of a step (r6): 1 Idempotency, 2 the gates in class order, 3 the owner's
/// rows, 4 AuditChain.
fn rank(step: &LockStep) -> u8 {
    match step {
        LockStep::Idempotency => 1,
        LockStep::ProducerGate => 3,
        LockStep::ReplyTarget => 4,
        LockStep::OwnerRows => 5,
        LockStep::AuditChain { .. } => 6,
    }
}

/// The advisory-lock classes, as `(class, key)` pairs (nemo's F1-08). `Sequencer` is
/// session-level on a dedicated session, never in a `ModuleTx`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AdvisoryClass {
    Idempotency = 1,
    Sequencer = 2,
    ProducerGate = 3,
    /// reserved, A6B
    ReplyTarget = 4,
    /// r4: I-1's append lock
    AuditChain = 5,
}

/// A step of the lock order, as the ledger records it (r6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockStep {
    Idempotency,
    ProducerGate,
    ReplyTarget,
    /// The owner's own statements and its non-audit delegated writes. The ledger cannot see the
    /// row locks a caller's SQL takes; this step appears in `LockOrder` when they are refused.
    OwnerRows,
    AuditChain {
        chain: i32,
    },
}

/// The steps a `ModuleTx` has taken, in order.
#[derive(Debug, Clone, Default)]
pub struct LockLedger {
    pub(crate) steps: Vec<LockStep>,
    pub(crate) sealed_by: Option<(ModuleId, i32)>,
}

impl LockLedger {
    pub fn steps(&self) -> &[LockStep] {
        &self.steps
    }
    pub fn sealed(&self) -> bool {
        self.sealed_by.is_some()
    }
}

/// The trusted actor and project for delegated writers.
#[derive(Debug, Clone)]
pub struct TxContext {
    pub module: ModuleId,
    pub project: Option<ProjectId>,
    pub actor: Actor,
    pub request_id: RequestId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    Worker(WorkerId),
    Service(WorkerId),
    HostOperator { uid: u32 },
    Recovery,
    System(ModuleId),
}

/// Values are M-8's (S2 §9.1); F-1b provides the mechanism.
#[derive(Debug, Clone, Default)]
pub struct TxOptions {
    pub statement_timeout: Option<Duration>,
}

/// The host-socket role (S1 section 9; W1-11): `identity.installations` and the install-level rows have
/// policies only for it. It is named HERE and nowhere else in the workspace's product source; a module
/// transaction's role is always `cortex_m_<module>` (see `module_role`), so no module id can reach it.
pub const HOST_ROLE: &str = "cortex_install_admin";

/// Who a host-path transaction is for (W1-11): the operator at the host socket, not a worker. The project
/// is `None` for an install-level action, and may be `Some(new id)` when the action creates that project.
#[derive(Debug, Clone)]
pub struct HostContext {
    pub uid: u32,
    pub project: Option<ProjectId>,
    pub request_id: RequestId,
}

/// The role `delegate` returns to after it ran `f` for a delegation made from scope `back` (the module
/// whose scope the transaction was in) with `depth` delegations still running: at depth 0 the transaction
/// is back in its owner's scope, whose role is the transaction's `base_role` (`cortex_m_<module>`, or the
/// host role of a host-path transaction); in a nested delegation it is the enclosing module's role.
fn restore_role(base_role: &str, depth: u32, back: &ModuleId) -> String {
    if depth == 0 {
        base_role.to_owned()
    } else {
        format!("cortex_m_{}", back.as_str())
    }
}

/// One transaction under a module's role and project.
pub struct ModuleTx {
    /// The transaction. `None` only after `commit` or `rollback` took it (both consume the `ModuleTx`), so every
    /// other method finds it; a `ModuleTx` dropped with it still here is an abandoned one (see `Drop`).
    pub(crate) tx: Option<sqlx::Transaction<'static, sqlx::Postgres>>,
    /// The backend this transaction runs on, for cancel-on-drop.
    pub(crate) backend_pid: i32,
    /// The pool's login options, for the cancel connection of an abandoned transaction.
    pub(crate) options: Arc<PgConnectOptions>,
    pub(crate) ctx: TxContext,
    pub(crate) current: ModuleId,
    pub(crate) ledger: LockLedger,
    pub(crate) poisoned: bool,
    /// The role the transaction started under, and the one `delegate` returns to at depth 0:
    /// `cortex_m_<module>` for a module transaction, `HOST_ROLE` for a host-path one.
    pub(crate) base_role: String,
    /// The pool's trusted registry: `delegate` checks its target against it.
    pub(crate) registry: ModuleRegistry,
    /// How many `delegate` calls are running: 0 is the owner's own scope.
    pub(crate) depth: u32,
    /// Set by a `DelegationGuard` dropped while armed: a `delegate` future was dropped before it
    /// returned, so the role and scope it left behind are unknown.
    pub(crate) abandoned: Arc<AtomicBool>,
}

/// Armed before a delegation's first await, disarmed when `delegate` returns; and armed before a
/// lock request is sent, disarmed once the ledger records it. Dropped while armed, it marks the
/// transaction abandoned.
struct DelegationGuard {
    abandoned: Arc<AtomicBool>,
    armed: bool,
}

impl DelegationGuard {
    fn arm(abandoned: &Arc<AtomicBool>) -> DelegationGuard {
        DelegationGuard {
            abandoned: abandoned.clone(),
            armed: true,
        }
    }

    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for DelegationGuard {
    fn drop(&mut self) {
        if self.armed {
            self.abandoned.store(true, Ordering::SeqCst);
        }
    }
}

impl ModuleTx {
    /// Checks the module against the registry before taking a connection (r7), then `BEGIN` and
    /// one round trip: role `cortex_m_<module>`, `cortex.project_id`, `cortex.now` and the
    /// statement timeout, all transaction-local; and checks READ COMMITTED.
    pub async fn begin(
        pool: &DbPool,
        ctx: TxContext,
        opts: TxOptions,
    ) -> Result<ModuleTx, DbError> {
        let role = module_role(&pool.registry, &ctx.module)?;
        Self::open(pool, role, ctx, opts).await
    }

    /// A HOST-PATH transaction (S1 section 9; W1-11): the role is the constant `HOST_ROLE`, the actor is
    /// `HostOperator { uid }`, and `TxContext::module` is the reserved id `platform`. It is for the
    /// host-socket paths (create the install, create a project) that no module role may do.
    ///
    /// - **No module id reaches it.** The role comes from the constant, never from a registry entry,
    ///   and `module_role` only ever yields `cortex_m_<id>`; `begin` of any `ModuleId` is therefore never
    ///   this role. A product crate cannot name or switch to the role either: an xtask rule
    ///   (`RoleSwitchOutsideCortexDb`) forbids it outside `cortex-db` and `cortex-testkit`.
    /// - **The project is optional.** `None` is an install-level action (no `cortex.project_id`). `Some`
    ///   may name a project that does not exist yet, for an action that creates it; the value is only
    ///   set as the transaction-local setting, nothing is looked up.
    /// - **`delegate` comes back to the host role**, not to a module's: the restore uses `base_role`
    ///   at depth 0.
    ///
    /// Same isolation check, clock and statement timeout as `begin`.
    pub async fn begin_host(
        pool: &DbPool,
        host: HostContext,
        opts: TxOptions,
    ) -> Result<ModuleTx, DbError> {
        let ctx = TxContext {
            module: ModuleId::parse("platform").expect("a valid module id"),
            project: host.project,
            actor: Actor::HostOperator { uid: host.uid },
            request_id: host.request_id,
        };
        Self::open(pool, HOST_ROLE.to_owned(), ctx, opts).await
    }

    async fn open(
        pool: &DbPool,
        role: String,
        ctx: TxContext,
        opts: TxOptions,
    ) -> Result<ModuleTx, DbError> {
        let mut tx = pool.pool.begin().await.map_err(DbError::unavailable)?;
        let project = ctx.project.map(|p| p.to_string());
        let now = pool.clock.now().unix_micros().to_string();
        let timeout = opts
            .statement_timeout
            .map(|d| d.as_millis().max(1).to_string());
        let sql = begin_sql(project.is_some(), timeout.is_some(), !nonlocal());
        let mut q = sqlx::query(sql).bind(&role);
        if let Some(p) = &project {
            q = q.bind(p);
        }
        q = q.bind(&now);
        if let Some(t) = &timeout {
            q = q.bind(t);
        }
        let row = q.fetch_one(&mut *tx).await.map_err(DbError::from_sqlx)?;
        let isolation: String = row.try_get(row.len() - 1).map_err(DbError::from_sqlx)?;
        let backend_pid: i32 = row.try_get(row.len() - 2).map_err(DbError::from_sqlx)?;
        if isolation != "read committed" {
            tx.rollback().await.ok();
            return Err(DbError::Isolation);
        }
        Ok(ModuleTx {
            tx: Some(tx),
            backend_pid,
            options: Arc::clone(&pool.options),
            current: ctx.module.clone(),
            ctx,
            ledger: LockLedger::default(),
            poisoned: false,
            base_role: role,
            registry: pool.registry.clone(),
            depth: 0,
            abandoned: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn context(&self) -> &TxContext {
        &self.ctx
    }

    /// `DelegationFailed`, and poisoned, once a delegation was abandoned.
    fn refuse_if_abandoned(&mut self) -> Result<(), DbError> {
        if self.abandoned.load(Ordering::SeqCst) {
            self.poisoned = true;
            return Err(DbError::DelegationFailed);
        }
        Ok(())
    }

    /// Inside a `delegate` to `m`: the audit scope when `m` sealed the transaction.
    fn in_scope_of(&self, m: &ModuleId) -> bool {
        self.depth > 0 && *m == self.current
    }

    /// The caller's own statements. Refused with `LockOrder` once the transaction is sealed,
    /// except inside a `delegate` to the module that holds `AuditChain`; refused with
    /// `DelegationFailed` after an abandoned delegation.
    pub fn executor(&mut self) -> Result<&mut sqlx::PgConnection, DbError> {
        self.refuse_if_abandoned()?;
        if let Some((m, chain)) = &self.ledger.sealed_by
            && !seal_off()
            && !self.in_scope_of(m)
        {
            let held = LockStep::AuditChain { chain: *chain };
            self.poisoned = true;
            return Err(DbError::LockOrder {
                held,
                wanted: LockStep::OwnerRows,
            });
        }
        Ok(self.conn())
    }

    async fn set_role(&mut self, role: &str) -> Result<(), DbError> {
        sqlx::query("SELECT set_config('role', $1, true)")
            .bind(role)
            .execute(self.conn())
            .await
            .map(|_| ())
            .map_err(DbError::from_sqlx)
    }

    /// Switches the role to `cortex_m_<to>` (registry-checked), keeps the project and actor, runs
    /// `f`, and restores the role, on success and on error. After any error, the registry refusal
    /// included, the transaction is poisoned. If this future is dropped before it returns, the
    /// transaction is abandoned (the module comment).
    pub async fn delegate<T>(
        &mut self,
        to: &ModuleId,
        f: impl AsyncFnOnce(&mut ModuleTx) -> Result<T, DbError>,
    ) -> Result<T, DbError> {
        self.refuse_if_abandoned()?;
        // Before any SQL, and it poisons (vera's F1B-GREEN-DELEGATE-PREFLIGHT).
        let role = match module_role(&self.registry, to) {
            Ok(role) => role,
            Err(e) => {
                self.poisoned = true;
                return Err(e);
            }
        };
        if let Some((m, chain)) = &self.ledger.sealed_by
            && !seal_off()
            && m != to
        {
            let held = LockStep::AuditChain { chain: *chain };
            self.poisoned = true;
            return Err(DbError::LockOrder {
                held,
                wanted: LockStep::OwnerRows,
            });
        }
        // Armed before the first await: from here to the restore, a dropped future abandons.
        let guard = DelegationGuard::arm(&self.abandoned);
        if let Err(e) = self.set_role(&role).await {
            self.poisoned = true;
            guard.disarm();
            return Err(e);
        }
        let back = std::mem::replace(&mut self.current, to.clone());
        self.depth += 1;
        let r = f(self).await;
        self.depth -= 1;
        // Back at the owner's scope (depth 0) the role is the transaction's own base role: a module's
        // `cortex_m_<module>`, or the host role of a host-path transaction. In a nested delegation it
        // is the enclosing module's.
        let back_role = restore_role(&self.base_role, self.depth, &back);
        self.current = back;
        // The role comes back on success and on error. After a failed statement the transaction is
        // aborted and this restore fails too; the closure's own error is the one returned.
        let restored = self.set_role(&back_role).await;
        guard.disarm();
        match (r, restored) {
            (Ok(v), Ok(())) => Ok(v),
            (Ok(_), Err(e)) | (Err(e), _) => {
                self.poisoned = true;
                Err(e)
            }
        }
    }

    /// `pg_advisory_xact_lock(class, key)` in the two-integer form (class and key are `int4`, as
    /// `hashtext` and S2 §4.5's `hash32` give), as its own statement, after the ledger allows it.
    /// `AuditChain` only inside a `delegate`: in the owner's own scope it is refused before any SQL.
    pub async fn advisory_lock(&mut self, class: AdvisoryClass, key: i32) -> Result<(), DbError> {
        self.refuse_if_abandoned()?;
        let wanted = match class {
            AdvisoryClass::Idempotency => LockStep::Idempotency,
            AdvisoryClass::ProducerGate => LockStep::ProducerGate,
            AdvisoryClass::ReplyTarget => LockStep::ReplyTarget,
            AdvisoryClass::AuditChain => LockStep::AuditChain { chain: key },
            AdvisoryClass::Sequencer => {
                self.poisoned = true;
                return Err(DbError::Other(
                    "the sequencer lock is session-level, never in a ModuleTx".into(),
                ));
            }
        };
        let mut retake = false;
        if !seal_off() {
            if matches!(wanted, LockStep::AuditChain { .. }) && self.depth == 0 {
                // The owner's own scope is not an audit scope (vera's F1B-GREEN-OWNER-AUDIT-SEAL).
                self.poisoned = true;
                return Err(DbError::LockOrder {
                    held: LockStep::OwnerRows,
                    wanted,
                });
            }
            if let Some((m, chain)) = &self.ledger.sealed_by {
                // Sealed: only a re-take of the same chain's lock, inside its own delegate.
                if wanted == (LockStep::AuditChain { chain: *chain }) && self.in_scope_of(m) {
                    retake = true;
                } else {
                    let held = LockStep::AuditChain { chain: *chain };
                    self.poisoned = true;
                    return Err(DbError::LockOrder { held, wanted });
                }
            } else if let Some(held) = self
                .ledger
                .steps
                .iter()
                .copied()
                .max_by_key(rank)
                .filter(|h| rank(h) > rank(&wanted))
            {
                self.poisoned = true;
                return Err(DbError::LockOrder { held, wanted });
            }
        }
        // Armed before the request is sent: from here until the ledger records the lock, a dropped
        // future abandons the transaction (the module comment).
        let guard = DelegationGuard::arm(&self.abandoned);
        let r = sqlx::query("SELECT pg_advisory_xact_lock($1, $2)")
            .bind(class as i32)
            .bind(key)
            .execute(self.conn())
            .await;
        if let Err(e) = r {
            self.poisoned = true;
            guard.disarm();
            return Err(DbError::from_sqlx(e));
        }
        if !retake {
            self.ledger.steps.push(wanted);
        }
        if let LockStep::AuditChain { chain } = wanted
            && self.ledger.sealed_by.is_none()
        {
            self.ledger.sealed_by = Some((self.current.clone(), chain));
        }
        guard.disarm();
        Ok(())
    }

    pub fn lock_ledger(&self) -> &LockLedger {
        &self.ledger
    }

    /// Refuses with `DelegationFailed` if poisoned or abandoned (the transaction is then rolled
    /// back).
    /// Commits, or (for a poisoned or abandoned transaction) rolls back and refuses with `DelegationFailed`.
    ///
    /// **If this future is dropped before it completes** (a timeout around `commit`), the commit is NOT rolled back by
    /// the drop itself: the transaction goes to a cleanup task ([`TerminalOp`]) that resumes the SAME commit future. That
    /// future MAY send the COMMIT (it is not necessarily on the connection yet, whatever the caller saw) and MAY complete
    /// it: if the work ahead of it ends by itself within the grace and nothing fails, the server executes the COMMIT and
    /// the data IS committed (cell `cd_10`); if that work has to be cancelled, or it ends in an error, the transaction
    /// is rolled back and no COMMIT may ever have been sent. **The caller must treat the outcome of a cancelled
    /// `commit` as unknown.**
    pub async fn commit(mut self) -> Result<(), DbError> {
        let tx = self.take_tx();
        let poisoned = self.poisoned || self.abandoned.load(Ordering::SeqCst);
        let work: Terminal = if poisoned {
            Box::pin(tx.rollback())
        } else {
            Box::pin(tx.commit())
        };
        let done = TerminalOp::new(work, self.backend_pid, Arc::clone(&self.options))
            .run()
            .await;
        if poisoned {
            return Err(DbError::DelegationFailed);
        }
        done.map_err(DbError::from_sqlx)
    }

    pub async fn rollback(mut self) -> Result<(), DbError> {
        let work: Terminal = Box::pin(self.take_tx().rollback());
        TerminalOp::new(work, self.backend_pid, Arc::clone(&self.options))
            .run()
            .await
            .map_err(DbError::from_sqlx)
    }

    /// The connection of the transaction. A `ModuleTx` holds it until `commit` or `rollback` consumes it.
    fn conn(&mut self) -> &mut sqlx::PgConnection {
        &mut *self
            .tx
            .as_mut()
            .expect("a ModuleTx holds its transaction until commit or rollback")
    }

    fn take_tx(&mut self) -> sqlx::Transaction<'static, sqlx::Postgres> {
        self.tx
            .take()
            .expect("a ModuleTx holds its transaction until commit or rollback")
    }
}

/// How long an abandoned transaction's rollback may take before the connection counts as busy with a statement
/// nobody reads any more (a rollback of an idle connection is one round trip).
const CANCEL_GRACE: Duration = Duration::from_millis(100);
/// How long the cancel connection may take to connect and send.
const CANCEL_TIMEOUT: Duration = Duration::from_secs(3);
/// How long the cleanup TASK waits for the rollback after the cancel. **This bounds the task, not the release of the
/// backend, its locks or the pool place** (vera's DBCANCEL-L1): when it expires, the future is dropped and sqlx's own
/// pool return takes over, whose ping has no timeout and waits for the statement to end by itself or by its
/// `statement_timeout`.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

fn drain_timeout() -> Duration {
    #[cfg(feature = "testkit")]
    if let Some(d) = crate::testkit::drain_override() {
        return d;
    }
    DRAIN_TIMEOUT
}

/// What `commit` and `rollback` wait for: sqlx's own terminal operation on the transaction.
type Terminal =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), sqlx::Error>> + Send>>;

/// Owns a terminal operation's future while `commit` or `rollback` awaits it (vera's DBCANCEL-V1). The operation takes
/// the transaction out of the `ModuleTx`, so if the caller cancels THAT await (a timeout around `rollback`, say) while
/// an earlier abandoned statement still runs, the `ModuleTx` is already empty and its `Drop` has nothing to clean up.
/// This guard has the same duty instead: dropped before the operation finished, it hands the SAME future (which still
/// owns the transaction and so the connection) to [`abandon`], exactly once.
struct TerminalOp {
    work: Option<Terminal>,
    pid: i32,
    options: Arc<PgConnectOptions>,
}

impl TerminalOp {
    fn new(work: Terminal, pid: i32, options: Arc<PgConnectOptions>) -> TerminalOp {
        TerminalOp {
            work: Some(work),
            pid,
            options,
        }
    }

    /// Polls the operation to its end; only then is the guard disarmed.
    async fn run(mut self) -> Result<(), sqlx::Error> {
        let done = match self.work.as_mut() {
            Some(work) => work.await,
            None => Ok(()),
        };
        self.work = None;
        done
    }
}

impl Drop for TerminalOp {
    fn drop(&mut self) {
        if let Some(work) = self.work.take() {
            spawn_abandon(work, self.pid, Arc::clone(&self.options));
        }
    }
}

fn spawn_abandon(work: Terminal, pid: i32, options: Arc<PgConnectOptions>) {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            handle.spawn(abandon(work, pid, options));
        }
        // No runtime, no task: the transaction is dropped as before.
        Err(_) => drop(work),
    }
}

/// **Cancel-on-drop** (lane A; cox 14:48Z, nemo's finding). A `ModuleTx` dropped without `commit` or `rollback`
/// (an error path, a handler timeout, a `select!`) may be dropped with a statement in flight: sqlx would queue a
/// rollback for the connection's next use and the pool would wait, with the place held, until the backend
/// finished or `statement_timeout` fired, and a module role cannot `pg_cancel_backend` (the target's session role
/// is the pool's login role).
///
/// So the drop hands the transaction to a task that runs the rollback NOW. A rollback of an idle connection ends
/// within the grace and nothing else happens (the same one round trip sqlx would have made later). If it does
/// not, a statement is still running on a connection this task OWNS (it is not in the pool), and the task sends
/// `pg_cancel_backend(pid)` from a separate connection of the pool's own login role, then lets the rollback
/// finish. Because the connection is held throughout, the cancel cannot reach another request's statement.
/// The same policy covers a `commit` or `rollback` whose own future is cancelled ([`TerminalOp`]).
///
/// **Preconditions and limits.** The cancel names the backend by its bare PID on a NEW connection, so it assumes
/// the pool talks to ONE physical server directly (the supported topology: a Unix socket or a direct TCP address;
/// no connection pooler or load balancer in between that could send the new connection elsewhere). If the cancel
/// connection is refused or times out nothing is cancelled: the task waits at most `DRAIN_TIMEOUT` and then leaves
/// the connection to sqlx's pool return, which keeps the backend, its locks and the pool place until the statement
/// ends by itself or by `statement_timeout`: best effort, and the old behaviour. No runtime, no task.
impl Drop for ModuleTx {
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            spawn_abandon(
                Box::pin(tx.rollback()),
                self.backend_pid,
                Arc::clone(&self.options),
            );
        }
    }
}

async fn abandon(mut work: Terminal, pid: i32, options: Arc<PgConnectOptions>) {
    if tokio::time::timeout(CANCEL_GRACE, &mut work).await.is_err() {
        send_cancel(&options, pid).await;
        let _ = tokio::time::timeout(drain_timeout(), work).await;
    }
    #[cfg(feature = "testkit")]
    crate::testkit::note_finished(pid);
}

/// `pg_cancel_backend(pid)` from a new connection of the pool's login role (so it is allowed to signal the
/// backend), not from the pool: a pool whose places are all held by abandoned statements could not serve it.
async fn send_cancel(options: &PgConnectOptions, pid: i32) {
    #[cfg(feature = "testkit")]
    if crate::testkit::cancel_refused() {
        return;
    }
    let attempt = async {
        let mut c = sqlx::PgConnection::connect_with(options).await?;
        sqlx::query("select pg_cancel_backend($1)")
            .bind(pid)
            .execute(&mut c)
            .await?;
        c.close().await
    };
    if let Ok(Ok(())) = tokio::time::timeout(CANCEL_TIMEOUT, attempt).await {
        #[cfg(feature = "testkit")]
        crate::testkit::note_cancel(pid);
    }
}
// Drop without commit rolls back (see `Drop`). No savepoints and no nested commit are exposed.

#[cfg(test)]
mod w1_restore_role_tests {
    use super::*;

    fn m(id: &str) -> ModuleId {
        ModuleId::parse(id).expect("a valid module id")
    }

    /// W1-11: a delegation out of a HOST-PATH transaction comes back to the host role, never to a module's.
    #[test]
    fn w1_h_6_the_restore_role_at_depth_zero_is_the_base_role() {
        // A host transaction: the owner's scope is the reserved `platform` id, the base role is the host role.
        assert_eq!(restore_role(HOST_ROLE, 0, &m("platform")), HOST_ROLE);
        // A module transaction keeps its own role.
        assert_eq!(
            restore_role("cortex_m_test_a", 0, &m("test_a")),
            "cortex_m_test_a"
        );
        // The base role, not the scope's module id, decides at depth 0 (the two differ for a host transaction).
        assert_ne!(
            restore_role(HOST_ROLE, 0, &m("platform")),
            "cortex_m_platform"
        );
    }

    #[test]
    fn w1_h_7_inside_a_nested_delegation_the_restore_role_is_the_enclosing_modules() {
        assert_eq!(
            restore_role(HOST_ROLE, 1, &m("test_audit")),
            "cortex_m_test_audit",
            "still inside a delegation: the enclosing module's role, even in a host transaction"
        );
        assert_eq!(
            restore_role("cortex_m_test_a", 2, &m("test_b")),
            "cortex_m_test_b"
        );
    }
}
