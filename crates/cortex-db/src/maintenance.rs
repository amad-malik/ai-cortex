//! Runtime DDL only through maintenance functions (CONTRACTS r7 §3 "Runtime DDL"; A7 r4 item 2,
//! L-8; TESTS r7 F1-31). F-1b RED (b2): every behaviour under test is a `todo!()` stub.
//!
//! - **The trust boundary.** Module crates and migrations are trusted release artifacts. These
//!   functions guard against bugs and wrong arguments in that code; they do not contain hostile
//!   SQL run under a module role, nor an untrusted migration (A7-L8-01, -02).
//! - **One release-pinned inventory** (`INVENTORY`), built into the binary. A module adds an
//!   entry only by a reviewed diff to lane A.
//! - **One reviewed template** (`templates::partition_maintenance`) produces each module's
//!   `create_partition`, `drop_partition`, `maintenance_runs` table and audit trigger. Each
//!   module supplies its own `drop_eligible`, which is inventoried too.
//! - **The startup check** (`check_definer_functions`) compares the catalogue with the inventory;
//!   `cortex-server` calls it before binding, and `doctor` runs it too.
//! - **The audit write** (kai's ruling T-2 on SAM-R2-03; the F1B-GREEN-AUDIT-SUBXID limit): the
//!   only supported `AuditSink` write is a plain `INSERT` at the top level of the transaction,
//!   since the audit trigger matches the record's `xmin` to the top-level transaction id, so one
//!   made inside a subtransaction (a PL/pgSQL `EXCEPTION` block, a savepoint) is not supported
//!   and is refused at commit, failing closed.
use crate::pool::OwnerConn;

/// What an inventory entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaintKind {
    Create,
    Drop,
    Eligible,
    AuditTrigger,
}

/// A month window, in whole months relative to M, the first day of the month of `platform.now()`,
/// both ends inclusive. Pinned separately for create and for drop, and per module (A7-L8-02).
///
/// r2 (nemo's M-1 confirmation r1, `0502cdae`, kai's addendum ed9bb740): `earliest: None` is
/// **unbounded into the past**: every valid historical month back to the start of the date domain
/// is inside, with no lower limit, so an old eligible leaf never ages out of the envelope (pins,
/// a failed sink or a retention change can postpone eligibility for years). The lower side does
/// no date arithmetic, so it cannot overflow, and there is no invented maximum retention.
/// Records: create `{ earliest: Some(0), latest: 2 }` (M to M+2), drop `{ earliest: None,
/// latest: -1 }` (strictly before M). The envelope is a safety bound, not eligibility: that is
/// `drop_eligible`'s, under the lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub earliest: Option<i32>,
    pub latest: i32,
}

/// One approved function: its exact identity and everything the startup check compares.
#[derive(Debug, Clone)]
pub struct MaintenanceFn {
    pub schema: &'static str,
    pub name: &'static str,
    /// Exact argument types, as `pg_get_function_identity_arguments` would give them.
    pub arg_types: &'static [&'static str],
    pub returns: &'static str,
    pub kind: MaintKind,
    /// The module whose role `cortex_m_<module>` is the only `EXECUTE` grantee besides the
    /// owner, WITHOUT grant option (r6). `None` for a helper: the owner's entry only.
    /// (A module name, not a `ModuleId`, so that the inventory can be a `static`.)
    pub module: Option<&'static str>,
    /// sha256 of `prosrc`, from the reviewed template (or the module's reviewed predicate).
    pub body_sha256: [u8; 32],
    /// Helpers it calls, each itself an inventory entry (`schema.name`).
    pub helpers: &'static [&'static str],
    /// The month window the function enforces; `None` for a predicate or a trigger function.
    pub window: Option<Window>,
}

/// The release inventory. Empty until M-1 (`mod-records`) and M-2 (`cortex-events`) add their
/// entries by reviewed diffs.
pub static INVENTORY: &[MaintenanceFn] = &[];

/// `schema.name(argtypes)`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FnId {
    pub schema: String,
    pub name: String,
    pub args: String,
}

/// `schema.table.trigger`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TriggerId {
    pub schema: String,
    pub table: String,
    pub trigger: String,
}

/// Why the startup check refuses (A7 r4 item 2; CONTRACTS r7 §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefinerError {
    Missing(FnId),
    Extra(FnId),
    Moved(FnId),
    Overloaded(FnId),
    NotDefiner(FnId),
    Signature(FnId),
    Language(FnId),
    Body(FnId),
    Helper(FnId),
    SearchPath(FnId),
    Owner(FnId),
    Grantees(FnId),
    /// r6: a grant option held by anyone but the owner, or a NULL ACL.
    GrantOption(FnId),
    /// r6: the audit constraint trigger (A7-L8-05).
    Trigger(TriggerId, TriggerFault),
}

impl std::fmt::Display for FnId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}({})", self.schema, self.name, self.args)
    }
}

impl DefinerError {
    /// The problem text startup refuses with: `definer_function_unsafe(<id>, <reason>)`.
    pub fn problem(&self) -> String {
        let (id, reason) = match self {
            DefinerError::Missing(f) => (f.to_string(), "missing"),
            DefinerError::Extra(f) => (f.to_string(), "extra"),
            DefinerError::Moved(f) => (f.to_string(), "moved"),
            DefinerError::Overloaded(f) => (f.to_string(), "overloaded"),
            DefinerError::NotDefiner(f) => (f.to_string(), "not_definer"),
            DefinerError::Signature(f) => (f.to_string(), "signature"),
            DefinerError::Language(f) => (f.to_string(), "language"),
            DefinerError::Body(f) => (f.to_string(), "body"),
            DefinerError::Helper(f) => (f.to_string(), "helper"),
            DefinerError::SearchPath(f) => (f.to_string(), "search_path"),
            DefinerError::Owner(f) => (f.to_string(), "owner"),
            DefinerError::Grantees(f) => (f.to_string(), "grantees"),
            DefinerError::GrantOption(f) => (f.to_string(), "grant_option"),
            DefinerError::Trigger(t, fault) => {
                let id = format!("{}.{}.{}", t.schema, t.table, t.trigger);
                return format!("definer_function_unsafe({id}, trigger_{})", fault.as_str());
            }
        };
        format!("definer_function_unsafe({id}, {reason})")
    }
}

/// How an audit trigger departs from the pinned one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerFault {
    Missing,
    Disabled,
    WrongTable,
    WrongTiming,
    WrongEvent,
    NotRowLevel,
    NotConstraint,
    NotDeferred,
    HasWhen,
    WrongFunction,
}

impl TriggerFault {
    fn as_str(self) -> &'static str {
        match self {
            TriggerFault::Missing => "missing",
            TriggerFault::Disabled => "disabled",
            TriggerFault::WrongTable => "wrong_table",
            TriggerFault::WrongTiming => "wrong_timing",
            TriggerFault::WrongEvent => "wrong_event",
            TriggerFault::NotRowLevel => "not_row_level",
            TriggerFault::NotConstraint => "not_constraint",
            TriggerFault::NotDeferred => "not_deferred",
            TriggerFault::HasWhen => "has_when",
            TriggerFault::WrongFunction => "wrong_function",
        }
    }
}

/// One function as the catalogue has it.
struct CatalogFn {
    oid: i64,
    id: FnId,
    returns: String,
    language: String,
    definer: bool,
    config: Option<Vec<String>>,
    owner: String,
    body_sha256: Vec<u8>,
    acl_null: bool,
    /// `grantee privilege grantable`, PUBLIC as `PUBLIC`.
    acl: Vec<(String, String, bool)>,
}

const CATALOG: &str = r#"
SELECT p.oid::bigint, n.nspname::text, p.proname::text, oidvectortypes(p.proargtypes),
       format_type(p.prorettype, NULL), l.lanname::text, p.prosecdef, p.proconfig,
       p.proowner::regrole::text, sha256(convert_to(p.prosrc, 'UTF8')), p.proacl IS NULL,
       coalesce(ARRAY(SELECT CASE WHEN a.grantee = 0 THEN 'PUBLIC' ELSE a.grantee::regrole::text END
                      FROM aclexplode(p.proacl) a ORDER BY a.grantee, a.privilege_type), '{}'),
       coalesce(ARRAY(SELECT a.privilege_type FROM aclexplode(p.proacl) a ORDER BY a.grantee, a.privilege_type), '{}'),
       coalesce(ARRAY(SELECT a.is_grantable FROM aclexplode(p.proacl) a ORDER BY a.grantee, a.privilege_type), '{}')
FROM pg_proc p
JOIN pg_namespace n ON n.oid = p.pronamespace
JOIN pg_language l ON l.oid = p.prolang
WHERE (n.nspname = ANY($1) AND p.prosecdef) OR p.proname = ANY($2)
"#;

const TRIGGERS: &str = r#"
SELECT c.relname::text, t.tgtype::int, t.tgenabled::text, t.tgconstraint <> 0, t.tgdeferrable,
       t.tginitdeferred, t.tgqual IS NOT NULL, t.tgfoid::bigint
FROM pg_trigger t
JOIN pg_class c ON c.oid = t.tgrelid
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE n.nspname = $1 AND t.tgname = $2 AND NOT t.tgisinternal
"#;

fn fn_id(f: &MaintenanceFn) -> FnId {
    FnId {
        schema: f.schema.to_owned(),
        name: f.name.to_owned(),
        args: f.arg_types.join(", "),
    }
}

fn function_errors(f: &MaintenanceFn, c: &CatalogFn, owner: &str) -> Vec<DefinerError> {
    let id = fn_id(f);
    let mut e = Vec::new();
    if c.returns != f.returns {
        e.push(DefinerError::Signature(id.clone()));
    }
    if c.language != "plpgsql" {
        e.push(DefinerError::Language(id.clone()));
    }
    if !c.definer {
        e.push(DefinerError::NotDefiner(id.clone()));
    }
    let want = vec![format!("search_path=pg_catalog, {}, pg_temp", f.schema)];
    if c.config.as_ref() != Some(&want) {
        e.push(DefinerError::SearchPath(id.clone()));
    }
    if c.owner != owner {
        e.push(DefinerError::Owner(id.clone()));
    }
    if c.body_sha256 != f.body_sha256 {
        e.push(DefinerError::Body(id.clone()));
    }
    // The ACL as stored (r6; vera's R0 note F1): a NULL ACL is PostgreSQL's default, PUBLIC
    // EXECUTE, and refuses. The owner's entry must be there with EXECUTE; its stored grant bit is
    // not compared. Every other entry is exactly the module role's EXECUTE, without grant option.
    if c.acl_null {
        e.push(DefinerError::GrantOption(id.clone()));
        return e;
    }
    let owner_execute = c.acl.iter().any(|(g, p, _)| g == owner && p == "EXECUTE");
    let others: std::collections::BTreeSet<(String, String)> = c
        .acl
        .iter()
        .filter(|(g, _, _)| g != owner)
        .map(|(g, p, _)| (g.clone(), p.clone()))
        .collect();
    let expected: std::collections::BTreeSet<(String, String)> = f
        .module
        .map(|m| (format!("cortex_m_{m}"), "EXECUTE".to_owned()))
        .into_iter()
        .collect();
    if !owner_execute || others != expected {
        e.push(DefinerError::Grantees(id.clone()));
    }
    if c.acl
        .iter()
        .any(|(g, _, grantable)| g != owner && *grantable)
    {
        e.push(DefinerError::GrantOption(id));
    }
    e
}

/// Reads `pg_proc` for every `prosecdef` function in `module_schemas`, plus every inventory
/// identity wherever it now lives, and each module's audit trigger from `pg_trigger` and
/// `pg_constraint`. The two sets must be equal, and every pinned field must match.
pub async fn check_definer_functions(
    owner: &mut OwnerConn,
    inv: &[MaintenanceFn],
    module_schemas: &[&str],
) -> Result<(), Vec<DefinerError>> {
    use sqlx::Row;
    let fail = |what: &str| {
        vec![DefinerError::Missing(FnId {
            schema: "?".into(),
            name: format!("the catalogue could not be read: {what}"),
            args: String::new(),
        })]
    };
    let conn = owner.conn();
    let me: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&mut *conn)
        .await
        .map_err(|_| fail("current_user"))?;
    let schemas: Vec<String> = module_schemas.iter().map(|s| (*s).to_owned()).collect();
    let names: Vec<String> = inv.iter().map(|f| f.name.to_owned()).collect();
    let rows = sqlx::query(CATALOG)
        .bind(&schemas)
        .bind(&names)
        .fetch_all(&mut *conn)
        .await
        .map_err(|_| fail("pg_proc"))?;
    let mut cat = Vec::new();
    for r in rows {
        let get = |e: sqlx::Error| {
            let _ = e;
            fail("a pg_proc row")
        };
        let grantees: Vec<String> = r.try_get(11).map_err(get)?;
        let privileges: Vec<String> = r.try_get(12).map_err(get)?;
        let grantable: Vec<bool> = r.try_get(13).map_err(get)?;
        cat.push(CatalogFn {
            oid: r.try_get(0).map_err(get)?,
            id: FnId {
                schema: r.try_get(1).map_err(get)?,
                name: r.try_get(2).map_err(get)?,
                args: r.try_get(3).map_err(get)?,
            },
            returns: r.try_get(4).map_err(get)?,
            language: r.try_get(5).map_err(get)?,
            definer: r.try_get(6).map_err(get)?,
            config: r.try_get(7).map_err(get)?,
            owner: r.try_get(8).map_err(get)?,
            body_sha256: r.try_get(9).map_err(get)?,
            acl_null: r.try_get(10).map_err(get)?,
            acl: grantees
                .into_iter()
                .zip(privileges)
                .zip(grantable)
                .map(|((g, p), b)| (g, p, b))
                .collect(),
        });
    }
    let ids: Vec<FnId> = inv.iter().map(fn_id).collect();
    let mut errors = Vec::new();
    // Each inventory entry's own faults, then its helpers' (a helper with any fault, or with no
    // inventory entry of its own, is the caller's `Helper`).
    let mut own: Vec<Vec<DefinerError>> = Vec::new();
    for (f, id) in inv.iter().zip(&ids) {
        let found = cat.iter().find(|c| c.id == *id);
        let e = match found {
            Some(c) => function_errors(f, c, &me),
            None => {
                let elsewhere = cat.iter().any(|c| {
                    c.id.name == id.name
                        && c.id.args == id.args
                        && c.id.schema != id.schema
                        && !ids.contains(&c.id)
                });
                let same_name = cat
                    .iter()
                    .any(|c| c.id.schema == id.schema && c.id.name == id.name);
                vec![if elsewhere {
                    DefinerError::Moved(id.clone())
                } else if same_name {
                    DefinerError::Signature(id.clone())
                } else {
                    DefinerError::Missing(id.clone())
                }]
            }
        };
        own.push(e);
    }
    for (i, f) in inv.iter().enumerate() {
        errors.extend(own[i].iter().cloned());
        let helper_bad = f.helpers.iter().any(|h| {
            let mut matching = inv
                .iter()
                .enumerate()
                .filter(|(_, g)| format!("{}.{}", g.schema, g.name) == *h)
                .peekable();
            matching.peek().is_none() || matching.any(|(j, _)| !own[j].is_empty())
        });
        if helper_bad {
            errors.push(DefinerError::Helper(ids[i].clone()));
        }
    }
    // Every definer function in the module schemas must be an inventory identity.
    for c in &cat {
        if schemas.contains(&c.id.schema) && c.definer && !ids.contains(&c.id) {
            let same_name = ids
                .iter()
                .any(|i| i.schema == c.id.schema && i.name == c.id.name);
            errors.push(if same_name {
                DefinerError::Overloaded(c.id.clone())
            } else {
                DefinerError::Extra(c.id.clone())
            });
        }
    }
    // Each module's audit constraint trigger (r6, A7-L8-05).
    for f in inv.iter().filter(|f| f.kind == MaintKind::AuditTrigger) {
        let tid = TriggerId {
            schema: f.schema.to_owned(),
            table: templates::RUNS_TABLE.to_owned(),
            trigger: templates::AUDIT_TRIGGER.to_owned(),
        };
        let audit_fn = cat.iter().find(|c| c.id == fn_id(f)).map(|c| c.oid);
        let rows = sqlx::query(TRIGGERS)
            .bind(f.schema)
            .bind(templates::AUDIT_TRIGGER)
            .fetch_all(&mut *conn)
            .await
            .map_err(|_| fail("pg_trigger"))?;
        let mut on_runs = None;
        for r in &rows {
            let table: String = r.try_get(0).map_err(|_| fail("a pg_trigger row"))?;
            if table == templates::RUNS_TABLE {
                on_runs = Some(r);
            }
        }
        let Some(r) = on_runs else {
            let fault = if rows.is_empty() {
                TriggerFault::Missing
            } else {
                TriggerFault::WrongTable
            };
            errors.push(DefinerError::Trigger(tid, fault));
            continue;
        };
        let bad = |_: sqlx::Error| fail("a pg_trigger row");
        let tgtype: i32 = r.try_get(1).map_err(bad)?;
        let enabled: String = r.try_get(2).map_err(bad)?;
        let constraint: bool = r.try_get(3).map_err(bad)?;
        let deferrable: bool = r.try_get(4).map_err(bad)?;
        let initdeferred: bool = r.try_get(5).map_err(bad)?;
        let has_when: bool = r.try_get(6).map_err(bad)?;
        let foid: i64 = r.try_get(7).map_err(bad)?;
        // tgtype: 1 ROW, 2 BEFORE, 4 INSERT, 8 DELETE, 16 UPDATE, 32 TRUNCATE, 64 INSTEAD.
        let mut faults = Vec::new();
        if tgtype & 1 == 0 {
            faults.push(TriggerFault::NotRowLevel);
        }
        if tgtype & (2 | 64) != 0 {
            faults.push(TriggerFault::WrongTiming);
        }
        if tgtype & 4 == 0 || tgtype & (8 | 16 | 32) != 0 {
            faults.push(TriggerFault::WrongEvent);
        }
        if !constraint {
            faults.push(TriggerFault::NotConstraint);
        }
        if !(deferrable && initdeferred) {
            faults.push(TriggerFault::NotDeferred);
        }
        if enabled != "A" {
            faults.push(TriggerFault::Disabled);
        }
        if has_when {
            faults.push(TriggerFault::HasWhen);
        }
        if audit_fn != Some(foid) {
            faults.push(TriggerFault::WrongFunction);
        }
        errors.extend(
            faults
                .into_iter()
                .map(|x| DefinerError::Trigger(tid.clone(), x)),
        );
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// The template's errors: SQLSTATE class P0, raised by the generated functions (A7 r4 item 2).
/// r2 (vera's F1B-B2-01): the SQLSTATE identifies the error, and **the message is exactly its
/// code** (`partition_conflict`, …), with no DETAIL or HINT and never row data. In the template
/// every raise is `RAISE EXCEPTION USING ERRCODE = '<sqlstate>', MESSAGE = /*cortex:msg*/'<code>'`.
/// A deferred refusal at `COMMIT` reaches the caller as `DbError::Other(<code>)` exactly (E12):
/// the mapping is by SQLSTATE, so the raw message never passes through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaintError {
    Conflict,
    NotEligible,
    BadValue,
    NotAuthorised,
    Unaudited,
    AccessUnsafe,
}

impl MaintError {
    pub const ALL: [MaintError; 6] = [
        MaintError::Conflict,
        MaintError::NotEligible,
        MaintError::BadValue,
        MaintError::NotAuthorised,
        MaintError::Unaudited,
        MaintError::AccessUnsafe,
    ];

    pub const fn sqlstate(self) -> &'static str {
        match self {
            MaintError::Conflict => "P0C01",
            MaintError::NotEligible => "P0C02",
            MaintError::BadValue => "P0C03",
            MaintError::NotAuthorised => "P0C04",
            MaintError::Unaudited => "P0C05",
            MaintError::AccessUnsafe => "P0C06",
        }
    }

    pub const fn code(self) -> &'static str {
        match self {
            MaintError::Conflict => "partition_conflict",
            MaintError::NotEligible => "partition_not_eligible",
            MaintError::BadValue => "partition_bad_value",
            MaintError::NotAuthorised => "partition_not_authorised",
            MaintError::Unaudited => "maintenance_unaudited",
            MaintError::AccessUnsafe => "partition_access_unsafe",
        }
    }

    pub fn from_sqlstate(s: &str) -> Option<MaintError> {
        MaintError::ALL.into_iter().find(|e| e.sqlstate() == s)
    }
}

/// The reviewed template (A7 r4 item 2's seven steps).
pub mod templates {
    use super::{MaintKind, Window};

    /// Where a run's audit record is looked for: the deferred trigger requires a row of
    /// `relation` with this run's `run_id`, written in the same transaction.
    #[derive(Debug, Clone)]
    pub struct AuditBinding {
        pub relation: &'static str,
        pub run_id_column: &'static str,
    }

    /// Who a child grant is for (r2, E3, mike's F1B-R0-09).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum GrantRole {
        /// `cortex_m_<module>`.
        Module,
        /// Another named role, for example records' retention role, which alone gets `DELETE`
        /// (A7 r4 item 2 step 6; S2 §3.8). A trusted, reviewed name, never request data.
        Named(&'static str),
    }

    /// One role's grants on a created child.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Grant {
        pub role: GrantRole,
        /// Never `UPDATE`, `TRUNCATE`, `REFERENCES` or `TRIGGER`.
        pub privileges: &'static [&'static str],
    }

    /// One module's partitioned table. `root_table` is list-partitioned by `project_id`; each
    /// project's partition is range-partitioned by `range_column`; the template creates and drops
    /// the monthly leaves under it.
    #[derive(Debug, Clone)]
    pub struct PartitionSpec {
        /// The module; its role is `cortex_m_<module>`.
        pub module: &'static str,
        pub schema: &'static str,
        pub root_table: &'static str,
        /// The column `root_table` is list-partitioned by, the project's (rework 9; kai's T-2
        /// addendum 3): the `parent` check requires the root's key to be exactly this column.
        pub list_column: &'static str,
        /// The `date` column each project's partition is range-partitioned by (rework 8; kai's
        /// T-2 addendum): the `parent` check requires exactly this column.
        pub range_column: &'static str,
        /// r2 (E3): every grant the child gets, per role. The repair grants exactly these and
        /// revokes everything else, and the verify reads the child's effective ACL back and
        /// requires exactly these (plus the owner's own), in the same function, before it
        /// returns. Each GRANT line carries `-- cortex:grant <role>`.
        pub grants: &'static [Grant],
        pub create_window: Window,
        pub drop_window: Window,
        pub audit: AuditBinding,
    }

    /// One generated function.
    #[derive(Debug, Clone)]
    pub struct GeneratedFn {
        pub name: String,
        pub arg_types: Vec<String>,
        pub returns: String,
        pub kind: MaintKind,
        /// Exactly what `pg_proc.prosrc` will hold; its sha256 is the inventory's pin.
        pub prosrc: String,
    }

    /// The migration SQL the template produces, run as `cortex_owner`, and the functions it
    /// defines.
    ///
    /// **Markers (F5, mike):** every guarded step in a body sits between
    /// `-- cortex:check <name>` and `-- cortex:end <name>`. The access step has
    /// `-- cortex:hook after-repair` between the repair and the verify. r2 (vera's F1B-B2-04):
    /// both functions take their prevalidation locks inside `-- cortex:check lock`, and have
    /// `-- cortex:hook after-validation` after the target's validation and before any DDL. Each
    /// raise's message is `/*cortex:msg*/'<code>'`, and each child GRANT line ends
    /// `-- cortex:grant <role>`. The testkit-only variant
    /// derives each control from this output by an edit at a named marker, and a missing marker is
    /// an error, never a silently unchanged variant. The markers are comments, part of `prosrc`
    /// and of its pinned hash.
    #[derive(Debug, Clone)]
    pub struct MigrationSql {
        pub up: String,
        pub functions: Vec<GeneratedFn>,
    }

    /// The check names a body carries. `project` is the binding to the transaction's project;
    /// `project-exists` is the existence and `open` check.
    pub const CHECKS: &[&str] = &[
        "values",
        "window",
        "project",
        "project-exists",
        "lock",
        "parent",
        "conflict",
        "ancestry",
        "bounds",
        "owner",
        "eligible",
        "verify-access",
        "audit-row",
    ];

    /// The hooks a body carries (r2): `after-validation` and `after-repair`.
    pub const HOOKS: &[&str] = &["after-validation", "after-repair"];

    /// The project's list partition under `root`: `<root>_<project, 32 hex>`. A naming rule,
    /// shared by the template and its fixtures.
    pub fn project_partition_name(root: &str, project: &uuid::Uuid) -> String {
        format!("{root}_{}", project.simple())
    }

    /// A month's leaf: `<root>_<project, 32 hex>_<yyyymm>`.
    pub fn month_partition_name(root: &str, project: &uuid::Uuid, year: i32, month: u32) -> String {
        format!("{root}_{}_{year:04}{month:02}", project.simple())
    }

    /// The template's fixed names: `<schema>.create_partition(project_id uuid, month date)
    /// returns text` ('created' or 'exists'); `<schema>.drop_partition(project_id uuid, month
    /// date, run_id uuid) returns text` ('dropped' or 'absent'); `<schema>.maintenance_runs`;
    /// its trigger function `<schema>.maintenance_audit()` and constraint trigger
    /// `maintenance_audited`. The module supplies `<schema>.drop_eligible(project_id uuid, month
    /// date) returns boolean`.
    ///
    /// **For the testkit variant (F5),** `create_partition`'s body uses the PL/pgSQL variables
    /// `v_parent regclass` (the project's list partition), `v_name text` (the leaf's name),
    /// `v_from date` and `v_to date`, and reads the project table as
    /// `/*cortex:qref*/identity_pub.projects`.
    pub const CREATE_FN: &str = "create_partition";
    pub const DROP_FN: &str = "drop_partition";
    pub const ELIGIBLE_FN: &str = "drop_eligible";
    pub const AUDIT_FN: &str = "maintenance_audit";
    pub const AUDIT_TRIGGER: &str = "maintenance_audited";
    pub const RUNS_TABLE: &str = "maintenance_runs";

    /// A trusted name from a reviewed spec, spliced into SQL: lowercase ASCII, digits and `_`.
    fn ident(s: &str) -> &str {
        assert!(
            !s.is_empty()
                && s.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                && !s.as_bytes()[0].is_ascii_digit(),
            "a spec name must be a plain lowercase identifier: {s:?}"
        );
        s
    }

    fn role_of(spec: &PartitionSpec, role: GrantRole) -> String {
        match role {
            GrantRole::Module => format!("cortex_m_{}", ident(spec.module)),
            GrantRole::Named(r) => ident(r).to_owned(),
        }
    }

    /// `RAISE EXCEPTION` with the error's SQLSTATE and its code as the whole message.
    fn raise(e: super::MaintError) -> String {
        format!(
            "RAISE EXCEPTION USING ERRCODE = '{}', MESSAGE = /*cortex:msg*/'{}';",
            e.sqlstate(),
            e.code()
        )
    }

    /// The window's two checks on `<arg>`, against `v_m` (M, the first day of this month).
    fn window_check(arg: &str, w: Window) -> String {
        let mut c = Vec::new();
        if let Some(e) = w.earliest {
            c.push(format!("{arg} < (v_m + interval '{e} months')::date"));
        }
        c.push(format!(
            "{arg} > (v_m + interval '{} months')::date",
            w.latest
        ));
        c.join(" OR ")
    }

    /// The checks every call makes before it resolves anything: the values, the window, the
    /// binding to the transaction's project, and the project's existence and state.
    fn prelude(f: &str, extra_null: &str, w: Window) -> String {
        use super::MaintError::{BadValue, NotAuthorised};
        format!(
            r#"  -- cortex:check values
  IF {f}.project_id IS NULL OR {f}.month IS NULL{extra_null} OR NOT isfinite({f}.month)
     OR {f}.month <> date_trunc('month', {f}.month::timestamp)::date THEN
    {bad}
  END IF;
  -- cortex:end values
  v_m := date_trunc('month', platform.now() AT TIME ZONE 'UTC')::date;
  -- cortex:check window
  IF {window} THEN
    {bad}
  END IF;
  -- cortex:end window
  -- cortex:check project
  v_project := nullif(current_setting('cortex.project_id', true), '')::uuid;
  IF v_project IS NULL OR v_project <> {f}.project_id THEN
    {no}
  END IF;
  -- cortex:end project
  -- cortex:check project-exists
  SELECT pr.state INTO v_state FROM /*cortex:qref*/identity_pub.projects pr WHERE pr.id = {f}.project_id;
  IF v_state IS DISTINCT FROM 'open' THEN
    {no}
  END IF;
  -- cortex:end project-exists
  v_from := {f}.month;
  v_to := ({f}.month + interval '1 month')::date;
"#,
            bad = raise(BadValue),
            no = raise(NotAuthorised),
            window = window_check(&format!("{f}.month"), w),
        )
    }

    /// The target's names: the project's list partition under the root (`v_parent`, NULL if it
    /// is missing or is not the root's list partition) and the leaf (`v_name`, `v_rel`).
    fn resolve(f: &str, schema: &str, root: &str) -> String {
        format!(
            r#"  v_root := '{schema}.{root}'::regclass;
  v_parent := to_regclass(format('%I.%I', '{schema}', '{root}_' || replace({f}.project_id::text, '-', '')));
  IF v_parent IS NOT NULL AND NOT EXISTS (
       SELECT 1 FROM pg_inherits i JOIN pg_class c ON c.oid = i.inhrelid
       WHERE i.inhrelid = v_parent AND i.inhparent = v_root AND c.relkind = 'p') THEN
    v_parent := NULL;
  END IF;
  v_name := '{root}_' || replace({f}.project_id::text, '-', '') || '_' || to_char({f}.month, 'YYYYMM');
  v_rel := to_regclass(format('%I.%I', '{schema}', v_name));
"#
        )
    }

    /// The project's list partition is what its name says (rework 7, sam's SAM-R2-01), checked
    /// under the parent's lock: a non-default partition of the root routing exactly this project,
    /// itself partitioned by RANGE on exactly `col`, the spec's `range_column`, a `date` column
    /// (the leaves' bounds are dates; rework 8 pins the name). A name, a kind and a root edge
    /// alone do not say so. The root's own key must be LIST on exactly `list`, the spec's
    /// `list_column`, so that the bound means this project (rework 9). A parent `resolve` did not
    /// find stays NULL and is handled as before.
    fn parent_check(f: &str, list: &str, col: &str) -> String {
        use super::MaintError::Conflict;
        format!(
            r#"  -- cortex:check parent
  IF v_parent IS NOT NULL AND NOT EXISTS (
       SELECT 1 FROM pg_inherits i
         JOIN pg_class c ON c.oid = i.inhrelid
         JOIN pg_partitioned_table r ON r.partrelid = i.inhparent
         JOIN pg_partitioned_table k ON k.partrelid = i.inhrelid
         JOIN pg_attribute a ON a.attrelid = i.inhrelid AND a.attnum = k.partattrs[0]
         JOIN pg_attribute ra ON ra.attrelid = i.inhparent AND ra.attnum = r.partattrs[0]
       WHERE i.inhrelid = v_parent AND i.inhparent = v_root AND c.relkind = 'p'
         AND r.partdefid <> c.oid
         AND pg_get_expr(c.relpartbound, c.oid) = format('FOR VALUES IN (%L)', {f}.project_id)
         AND k.partstrat = 'r' AND k.partnatts = 1 AND a.atttypid = 'date'::regtype
         AND a.attname = '{col}'
         AND r.partstrat = 'l' AND r.partnatts = 1 AND ra.attname = '{list}') THEN
    {conflict}
  END IF;
  -- cortex:end parent
"#,
            conflict = raise(Conflict),
        )
    }

    /// Reads the found relation's kind, partition flag, parent, bounds and owner.
    const READ_REL: &str = r#"    SELECT c.relkind, c.relispartition, c.relowner, pg_get_expr(c.relpartbound, c.oid)
      INTO v_kind, v_ispart, v_owner, v_bound FROM pg_class c WHERE c.oid = v_rel;
    SELECT i.inhparent INTO v_up FROM pg_inherits i WHERE i.inhrelid = v_rel;
"#;

    const DECLARE: &str = r#"DECLARE
  v_m date;
  v_project uuid;
  v_state text;
  v_root regclass;
  v_parent regclass;
  v_name text;
  v_from date;
  v_to date;
  v_rel regclass;
  v_kind "char";
  v_ispart boolean;
  v_owner oid;
  v_bound text;
  v_up oid;
  v_grantee oid;
"#;

    fn create_body(spec: &PartitionSpec) -> String {
        use super::MaintError::{AccessUnsafe, Conflict};
        let (s, root) = (ident(spec.schema), ident(spec.root_table));
        let f = CREATE_FN;
        let mut grants = String::new();
        let mut expected = Vec::new();
        for g in spec.grants {
            let role = role_of(spec, g.role);
            for p in g.privileges {
                assert!(
                    ["SELECT", "INSERT", "DELETE"].contains(p),
                    "a child grant is SELECT, INSERT or DELETE only: {p}"
                );
                expected.push(format!("('{role}', '{p}')"));
            }
            grants.push_str(&format!(
                "  EXECUTE format('GRANT {} ON %I.%I TO {role}', '{s}', v_name); -- cortex:grant {role}\n",
                g.privileges.join(", ")
            ));
        }
        let expected = expected.join(", ");
        format!(
            r#"
{declare}BEGIN
{prelude}{resolve}  IF v_parent IS NULL THEN
    {conflict}
  END IF;
  -- The parent's lock keeps any partition from being created, attached, detached or dropped under
  -- it until commit. A same-name relation is not locked: one of another owner cannot be, and a
  -- refusal must be a conflict, never a permission error.
  -- cortex:check lock
  EXECUTE format('LOCK TABLE %s IN SHARE UPDATE EXCLUSIVE MODE', v_parent);
  v_rel := to_regclass(format('%I.%I', '{s}', v_name));
  -- cortex:end lock
{parent}  IF v_rel IS NOT NULL THEN
{read_rel}    -- cortex:check conflict
    IF v_kind <> 'r' OR NOT v_ispart OR v_up IS DISTINCT FROM v_parent::oid THEN
      {conflict}
    END IF;
    -- cortex:end conflict
    -- cortex:check bounds
    IF v_bound IS DISTINCT FROM format('FOR VALUES FROM (%L) TO (%L)', v_from, v_to) THEN
      {conflict}
    END IF;
    -- cortex:end bounds
    -- cortex:check owner
    IF v_owner <> (SELECT c.relowner FROM pg_class c WHERE c.oid = v_root) THEN
      {conflict}
    END IF;
    -- cortex:end owner
    RETURN 'exists';
  END IF;
  -- cortex:hook after-validation
  EXECUTE format('CREATE TABLE %I.%I PARTITION OF %s FOR VALUES FROM (%L) TO (%L)', '{s}', v_name, v_parent, v_from, v_to);
  v_rel := format('%I.%I', '{s}', v_name)::regclass;
  EXECUTE format('REVOKE ALL ON %I.%I FROM PUBLIC', '{s}', v_name);
  FOR v_grantee IN SELECT DISTINCT a.grantee FROM pg_class c, aclexplode(c.relacl) a
                   WHERE c.oid = v_rel AND a.grantee <> c.relowner AND a.grantee <> 0 LOOP
    EXECUTE format('REVOKE ALL ON %I.%I FROM %s', '{s}', v_name, v_grantee::regrole);
  END LOOP;
{grants}  EXECUTE format('ALTER TABLE %I.%I ENABLE ROW LEVEL SECURITY', '{s}', v_name);
  EXECUTE format('ALTER TABLE %I.%I FORCE ROW LEVEL SECURITY', '{s}', v_name);
  EXECUTE format('CREATE POLICY by_project ON %I.%I USING (project_id = nullif(current_setting(''cortex.project_id'', true), '''')::uuid) WITH CHECK (project_id = nullif(current_setting(''cortex.project_id'', true), '''')::uuid)', '{s}', v_name);
  -- cortex:hook after-repair
  -- cortex:check verify-access
  IF EXISTS (
       (SELECT a.grantee, a.privilege_type, a.is_grantable FROM pg_class c, aclexplode(c.relacl) a
        WHERE c.oid = v_rel AND a.grantee <> c.relowner
        EXCEPT
        SELECT r.oid, e.priv, false FROM (VALUES {expected}) e(role, priv) JOIN pg_roles r ON r.rolname = e.role)
       UNION ALL
       (SELECT r.oid, e.priv, false FROM (VALUES {expected}) e(role, priv) JOIN pg_roles r ON r.rolname = e.role
        EXCEPT
        SELECT a.grantee, a.privilege_type, a.is_grantable FROM pg_class c, aclexplode(c.relacl) a
        WHERE c.oid = v_rel AND a.grantee <> c.relowner))
     OR (SELECT count(*) FROM (VALUES {expected}) e(role, priv)) <> (SELECT count(*) FROM (VALUES {expected}) e(role, priv) JOIN pg_roles r ON r.rolname = e.role)
     OR NOT (SELECT c.relrowsecurity AND c.relforcerowsecurity FROM pg_class c WHERE c.oid = v_rel)
     OR (SELECT count(*) FROM pg_policy pol WHERE pol.polrelid = v_rel) <> 1
     OR NOT EXISTS (
       SELECT 1 FROM pg_policy pol, pg_policy want
       WHERE pol.polrelid = v_rel AND want.polrelid = v_root AND want.polname = 'by_project'
         AND pol.polcmd = '*' AND pol.polpermissive AND pol.polroles = '{{0}}'::oid[]
         AND pg_get_expr(pol.polqual, pol.polrelid) = pg_get_expr(want.polqual, want.polrelid)
         AND pg_get_expr(pol.polwithcheck, pol.polrelid) = pg_get_expr(want.polwithcheck, want.polrelid)) THEN
    {unsafe}
  END IF;
  -- cortex:end verify-access
  RETURN 'created';
END
"#,
            declare = DECLARE,
            prelude = prelude(f, "", spec.create_window),
            resolve = resolve(f, s, root),
            parent = parent_check(f, ident(spec.list_column), ident(spec.range_column)),
            read_rel = READ_REL,
            conflict = raise(Conflict),
            unsafe = raise(AccessUnsafe),
        )
    }

    fn drop_body(spec: &PartitionSpec) -> String {
        use super::MaintError::{Conflict, NotEligible};
        let (s, root) = (ident(spec.schema), ident(spec.root_table));
        let f = DROP_FN;
        format!(
            r#"
{declare}BEGIN
{prelude}{resolve}  -- A leaf of another owner is a conflict before any lock: the definer cannot lock it, and a
  -- refusal must never be a permission error. The owner is checked again under the lock.
  -- cortex:check owner
  IF v_rel IS NOT NULL
     AND (SELECT c.relowner FROM pg_class c WHERE c.oid = v_rel) <> (SELECT c.relowner FROM pg_class c WHERE c.oid = v_root) THEN
    {conflict}
  END IF;
  -- cortex:end owner
  -- cortex:check lock
  IF v_parent IS NOT NULL THEN
    EXECUTE format('LOCK TABLE %s IN ACCESS EXCLUSIVE MODE', v_parent);
  END IF;
  IF v_rel IS NOT NULL THEN
    EXECUTE format('LOCK TABLE %s IN ACCESS EXCLUSIVE MODE', v_rel);
  END IF;
  v_rel := to_regclass(format('%I.%I', '{s}', v_name));
  -- cortex:end lock
{parent}  IF v_rel IS NULL THEN
    RETURN 'absent';
  END IF;
{read_rel}  -- cortex:check ancestry
  IF v_parent IS NULL OR v_kind <> 'r' OR NOT v_ispart OR v_up IS DISTINCT FROM v_parent::oid
     OR v_bound IS DISTINCT FROM format('FOR VALUES FROM (%L) TO (%L)', v_from, v_to) THEN
    {conflict}
  END IF;
  -- cortex:end ancestry
  -- cortex:check owner
  IF v_owner <> (SELECT c.relowner FROM pg_class c WHERE c.oid = v_root) THEN
    {conflict}
  END IF;
  -- cortex:end owner
  -- cortex:check eligible
  IF NOT coalesce({s}.{eligible}({f}.project_id, {f}.month), false) THEN
    {not_eligible}
  END IF;
  -- cortex:end eligible
  -- cortex:hook after-validation
  EXECUTE format('DROP TABLE %I.%I', '{s}', v_name);
  -- cortex:check audit-row
  INSERT INTO {s}.{runs} (run_id, project_id, month, relation, action)
    VALUES ({f}.run_id, {f}.project_id, {f}.month, v_name, 'drop');
  -- cortex:end audit-row
  RETURN 'dropped';
END
"#,
            declare = DECLARE,
            prelude = prelude(f, &format!(" OR {f}.run_id IS NULL"), spec.drop_window),
            resolve = resolve(f, s, root),
            parent = parent_check(f, ident(spec.list_column), ident(spec.range_column)),
            read_rel = READ_REL,
            conflict = raise(Conflict),
            not_eligible = raise(NotEligible),
            eligible = ELIGIBLE_FN,
            runs = RUNS_TABLE,
        )
    }

    fn audit_body(spec: &PartitionSpec) -> String {
        let (rel_schema, rel_table) = spec
            .audit
            .relation
            .split_once('.')
            .expect("the audit relation is schema.table");
        let (rel_schema, rel_table, col) = (
            ident(rel_schema),
            ident(rel_table),
            ident(spec.audit.run_id_column),
        );
        format!(
            r#"
BEGIN
  -- The run's audit record: a row of {rel_schema}.{rel_table} with this run's id, written in this
  -- same transaction (its xmin is this transaction's id), so a stale record does not count.
  IF NOT EXISTS (
       SELECT 1 FROM {rel_schema}.{rel_table} a
       WHERE a.{col} = NEW.run_id
         AND a.xmin::text = (pg_current_xact_id()::text::bigint % 4294967296)::text) THEN
    {unaudited}
  END IF;
  RETURN NULL;
END
"#,
            unaudited = raise(super::MaintError::Unaudited),
        )
    }

    /// One `CREATE FUNCTION`, its body between `$body$` markers exactly as `prosrc` holds it.
    fn define(schema: &str, name: &str, args: &str, returns: &str, body: &str) -> String {
        format!(
            "CREATE FUNCTION {schema}.{name}({args}) RETURNS {returns}
             LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, {schema}, pg_temp AS $body${body}$body$;
"
        )
    }

    /// `maintenance_runs`'s exact grants (rework 1, vera's F1B-GREEN-DEFAULT-ACL). The owner's
    /// default privileges, for PUBLIC or a named role, may have added grants to the new table and
    /// its identity sequence: every grant to anyone but the owner is revoked, the module role gets
    /// SELECT, and the result is read back (exactly that one entry, not grantable, and none on the
    /// sequence) before the migration can go on.
    fn runs_acl(s: &str, module_role: &str) -> String {
        let objects = format!(
            "('{s}.{RUNS_TABLE}'::pg_catalog.regclass, \
             pg_catalog.pg_get_serial_sequence('{s}.{RUNS_TABLE}', 'id')::pg_catalog.regclass)"
        );
        format!(
            "DO $$
DECLARE r record;
BEGIN
  FOR r IN SELECT DISTINCT c.oid::pg_catalog.regclass::text AS rel, c.relkind,
                  CASE WHEN a.grantee = 0 THEN 'PUBLIC' ELSE pg_catalog.quote_ident(g.rolname) END AS who
           FROM pg_catalog.pg_class c
           CROSS JOIN LATERAL pg_catalog.aclexplode(c.relacl) a
           LEFT JOIN pg_catalog.pg_roles g ON g.oid = a.grantee
           WHERE c.oid IN {objects} AND a.grantee <> c.relowner LOOP
    EXECUTE pg_catalog.format('REVOKE ALL ON %s %s FROM %s',
                              CASE WHEN r.relkind = 'S' THEN 'SEQUENCE' ELSE 'TABLE' END, r.rel, r.who);
  END LOOP;
END $$;
GRANT SELECT ON {s}.{RUNS_TABLE} TO {module_role};
DO $$
BEGIN
  IF EXISTS (
    (SELECT c.oid, a.grantee, a.privilege_type, a.is_grantable
       FROM pg_catalog.pg_class c CROSS JOIN LATERAL pg_catalog.aclexplode(c.relacl) a
       WHERE c.oid IN {objects} AND a.grantee <> c.relowner
     EXCEPT
     SELECT '{s}.{RUNS_TABLE}'::pg_catalog.regclass::oid, '{module_role}'::pg_catalog.regrole::oid, 'SELECT', false)
    UNION ALL
    (SELECT '{s}.{RUNS_TABLE}'::pg_catalog.regclass::oid, '{module_role}'::pg_catalog.regrole::oid, 'SELECT', false
     EXCEPT
     SELECT c.oid, a.grantee, a.privilege_type, a.is_grantable
       FROM pg_catalog.pg_class c CROSS JOIN LATERAL pg_catalog.aclexplode(c.relacl) a
       WHERE c.oid IN {objects} AND a.grantee <> c.relowner)
  ) THEN
    RAISE EXCEPTION 'maintenance_runs grants are not exactly the allowed set';
  END IF;
END $$;
"
        )
    }

    pub fn partition_maintenance(spec: &PartitionSpec) -> MigrationSql {
        let s = ident(spec.schema);
        let module_role = format!("cortex_m_{}", ident(spec.module));
        let runs_acl = runs_acl(s, &module_role);
        let (create, drop, audit) = (create_body(spec), drop_body(spec), audit_body(spec));
        let mut up = String::new();
        up.push_str(&format!(
            "CREATE TABLE {s}.{RUNS_TABLE} (
  id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  run_id uuid NOT NULL,
               project_id uuid NOT NULL,
  month date NOT NULL,
  relation text NOT NULL,
  action text NOT NULL,
               at timestamptz NOT NULL DEFAULT platform.now()
);
{runs_acl}"
        ));
        up.push_str(&define(s, AUDIT_FN, "", "trigger", &audit));
        up.push_str(&format!(
            "REVOKE ALL ON FUNCTION {s}.{AUDIT_FN}() FROM PUBLIC;
"
        ));
        up.push_str(&format!(
            "CREATE CONSTRAINT TRIGGER {AUDIT_TRIGGER} AFTER INSERT ON {s}.{RUNS_TABLE}
               DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION {s}.{AUDIT_FN}();
             ALTER TABLE {s}.{RUNS_TABLE} ENABLE ALWAYS TRIGGER {AUDIT_TRIGGER};
"
        ));
        up.push_str(&define(
            s,
            CREATE_FN,
            "project_id uuid, month date",
            "text",
            &create,
        ));
        up.push_str(&format!(
            "REVOKE ALL ON FUNCTION {s}.{CREATE_FN}(uuid, date) FROM PUBLIC;
             GRANT EXECUTE ON FUNCTION {s}.{CREATE_FN}(uuid, date) TO {module_role};
"
        ));
        up.push_str(&define(
            s,
            DROP_FN,
            "project_id uuid, month date, run_id uuid",
            "text",
            &drop,
        ));
        up.push_str(&format!(
            "REVOKE ALL ON FUNCTION {s}.{DROP_FN}(uuid, date, uuid) FROM PUBLIC;
             GRANT EXECUTE ON FUNCTION {s}.{DROP_FN}(uuid, date, uuid) TO {module_role};
"
        ));
        let gen_fn = |name: &str, args: &[&str], returns: &str, kind: MaintKind, prosrc: String| {
            GeneratedFn {
                name: name.to_owned(),
                arg_types: args.iter().map(|a| (*a).to_owned()).collect(),
                returns: returns.to_owned(),
                kind,
                prosrc,
            }
        };
        MigrationSql {
            up,
            functions: vec![
                gen_fn(AUDIT_FN, &[], "trigger", MaintKind::AuditTrigger, audit),
                gen_fn(
                    CREATE_FN,
                    &["uuid", "date"],
                    "text",
                    MaintKind::Create,
                    create,
                ),
                gen_fn(
                    DROP_FN,
                    &["uuid", "date", "uuid"],
                    "text",
                    MaintKind::Drop,
                    drop,
                ),
            ],
        }
    }
}
