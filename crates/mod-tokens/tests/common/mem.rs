//! In-memory doubles for the verification seams (L1: no database). The store holds rows and worker views, counts
//! lookups, opens, touches and commits, and can be told to fail or to return a NULL current generation.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cortex_module::{Role, WorkerKind, WorkerView};
use cortex_testkit as _;
use cortex_types::{
    Clock, ProjectId, SystemClock, TeamId, TestClock, Timestamp, TokenId, WorkerId,
};
use mod_tokens::token;
use mod_tokens::verify::{
    DigestCompare, StoreError, StoredToken, SubtleCompare, Verifier, VerifySession, VerifyStore,
};

pub const DAY: i64 = 86_400_000_000;
pub const NOW: i64 = 1_800_000_000_000_000;

#[derive(Default)]
pub struct Inner {
    pub rows: Mutex<HashMap<TokenId, StoredToken>>,
    pub workers: Mutex<HashMap<WorkerId, WorkerView>>,
    pub opens: AtomicUsize,
    pub lookups: AtomicUsize,
    pub touches: Mutex<Vec<(TokenId, Timestamp)>>,
    pub commits: AtomicUsize,
    pub down: AtomicBool,
    pub null_install: AtomicBool,
    pub null_project: AtomicBool,
}

#[derive(Clone, Default)]
pub struct Mem(pub Arc<Inner>);

struct Session(Arc<Inner>);

#[async_trait]
impl VerifyStore for Mem {
    async fn open(&self) -> Result<Box<dyn VerifySession>, StoreError> {
        self.0.opens.fetch_add(1, Ordering::SeqCst);
        if self.0.down.load(Ordering::SeqCst) {
            return Err(StoreError);
        }
        Ok(Box::new(Session(self.0.clone())))
    }
}

#[async_trait]
impl VerifySession for Session {
    async fn lookup(&mut self, id: &TokenId) -> Result<Option<StoredToken>, StoreError> {
        self.0.lookups.fetch_add(1, Ordering::SeqCst);
        let mut row = self.0.rows.lock().expect("lock").get(id).cloned();
        if let Some(r) = row.as_mut() {
            if self.0.null_install.load(Ordering::SeqCst) {
                r.current_install_gen = None;
            }
            if self.0.null_project.load(Ordering::SeqCst) {
                r.current_project_gen = None;
            }
        }
        Ok(row)
    }
    async fn worker(
        &mut self,
        _project: &ProjectId,
        w: &WorkerId,
    ) -> Result<Option<WorkerView>, StoreError> {
        Ok(self.0.workers.lock().expect("lock").get(w).cloned())
    }
    async fn touch(
        &mut self,
        _project: &ProjectId,
        id: &TokenId,
        now: Timestamp,
    ) -> Result<(), StoreError> {
        self.0.touches.lock().expect("lock").push((*id, now));
        if let Some(r) = self.0.rows.lock().expect("lock").get_mut(id) {
            r.last_used_at = Some(now);
        }
        Ok(())
    }
    async fn commit(self: Box<Self>) -> Result<(), StoreError> {
        self.0.commits.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// Counts comparisons, then delegates to `subtle`.
#[derive(Default)]
pub struct Counting(pub AtomicUsize);

impl DigestCompare for Counting {
    fn equal(&self, stored: &[u8; 32], presented: &[u8; 32]) -> bool {
        self.0.fetch_add(1, Ordering::SeqCst);
        SubtleCompare.equal(stored, presented)
    }
}

/// The fixture: project A with a lead, a team lead, two members and a worker with no team; project B with one
/// worker. Every worker has one token.
pub struct Fixture {
    pub clock: Arc<TestClock>,
    pub store: Mem,
    pub a: ProjectId,
    pub b: ProjectId,
    pub team: TeamId,
    pub lead: WorkerId,
    pub team_lead: WorkerId,
    pub w1: WorkerId,
    pub w2: WorkerId,
    pub w4: WorkerId,
    pub wb1: WorkerId,
    /// worker -> (bearer, token id)
    pub tokens: HashMap<WorkerId, (String, TokenId)>,
}

impl Fixture {
    pub fn new() -> Fixture {
        let sys = SystemClock;
        let clock = Arc::new(TestClock::at(Timestamp::from_unix_micros(NOW)));
        let mut f = Fixture {
            clock,
            store: Mem::default(),
            a: ProjectId::new(&sys),
            b: ProjectId::new(&sys),
            team: TeamId::new(&sys),
            lead: WorkerId::new(&sys),
            team_lead: WorkerId::new(&sys),
            w1: WorkerId::new(&sys),
            w2: WorkerId::new(&sys),
            w4: WorkerId::new(&sys),
            wb1: WorkerId::new(&sys),
            tokens: HashMap::new(),
        };
        let team = f.team;
        for (w, role, project) in [
            (f.lead, Role::ProjectLead, f.a),
            (f.team_lead, Role::TeamLead(team), f.a),
            (f.w1, Role::Member(Some(team)), f.a),
            (f.w2, Role::Member(Some(team)), f.a),
            (f.w4, Role::Member(None), f.a),
            (f.wb1, Role::Member(None), f.b),
        ] {
            f.store.0.workers.lock().expect("lock").insert(
                w,
                WorkerView {
                    id: w,
                    name: "w".into(),
                    kind: WorkerKind::Worker,
                    role,
                    job_roles: vec![],
                    active: true,
                },
            );
            f.issue(w, project);
        }
        f
    }

    pub fn now(&self) -> Timestamp {
        self.clock.now()
    }

    pub fn set_now(&self, micros: i64) {
        self.clock.set(Timestamp::from_unix_micros(micros));
    }

    /// Issues a token for `w` into the store, with both generations 0 and the current ones 0.
    pub fn issue(&mut self, w: WorkerId, project: ProjectId) -> (String, TokenId) {
        let g = token::generate(&*self.clock).expect("generated");
        let issued = self.now();
        let expires = Timestamp::from_unix_micros(issued.unix_micros() + 90 * DAY);
        let renew = Timestamp::from_unix_micros(issued.unix_micros() + 60 * DAY);
        let row = StoredToken {
            id: g.id(),
            worker: w,
            project,
            digest: g.digest(),
            install_gen: 0,
            project_gen: 0,
            issued_at: issued,
            expires_at: expires,
            renew_after: renew,
            last_used_at: None,
            revoked_at: None,
            revoke_reason: None,
            worker_state: "active".into(),
            current_install_gen: Some(0),
            current_project_gen: Some(0),
        };
        self.store.0.rows.lock().expect("lock").insert(g.id(), row);
        let bearer = g.token().expose().to_owned();
        self.tokens.insert(w, (bearer.clone(), g.id()));
        (bearer, g.id())
    }

    pub fn bearer(&self, w: WorkerId) -> cortex_types::Secret {
        cortex_types::Secret::new(self.tokens[&w].0.clone())
    }

    pub fn id(&self, w: WorkerId) -> TokenId {
        self.tokens[&w].1
    }

    pub fn edit(&self, w: WorkerId, f: impl FnOnce(&mut StoredToken)) {
        let id = self.id(w);
        f(self
            .store
            .0
            .rows
            .lock()
            .expect("lock")
            .get_mut(&id)
            .expect("row"));
    }

    pub fn verifier(&self) -> Verifier {
        Verifier::new(Arc::new(self.store.clone()), self.clock.clone())
    }

    pub fn counting_verifier(&self) -> (Verifier, Arc<Counting>) {
        let c = Arc::new(Counting::default());
        (
            Verifier::with_compare(Arc::new(self.store.clone()), self.clock.clone(), c.clone()),
            c,
        )
    }
}
