//! Verification (`TokenVerifier::verify`, the only path that yields a principal; S1 4.2; cells TOK-02, 04, 10 to
//! 17, 45, DEF-2, DEF-4, GEN-1 to GEN-7).
//!
//! **Two seams keep the rules testable without a database.** [`VerifyStore`] opens a [`VerifySession`]: the
//! database part of one verification (the `tokens.verify_lookup` row, the worker's facts through `IdentityApi`
//! in the token's project, the `last_used_at` touch, the commit). [`DigestCompare`] is the one digest
//! comparison, so a cell can count it. The decision is a pure function, [`check`].
//!
//! **Order (S1 4.2 step 3, kai's R-20):** revoked or superseded, then expired, then a stale generation (which is
//! `Revoked`: no new S0 variant), then a retired worker. A wrong secret is `Unknown` whatever the state of the
//! token: the state of a token is disclosed only to a holder of its secret. **A missing generation row is
//! generation 0 (DV-4); the comparison is EQUALITY; a NULL never passes as "unknown".**
//!
use std::sync::Arc;

use async_trait::async_trait;
use cortex_module::{AuthError, Principal, TokenVerifier, WorkerView};
use cortex_types::{Clock, ProjectId, Secret, Timestamp, TokenId, WorkerId};
use subtle::ConstantTimeEq;

use crate::token;

/// `last_used_at` is written at most once per this many microseconds (S1 4.2: 60 s).
pub const TOUCH_INTERVAL_MICROS: i64 = 60_000_000;

/// What `tokens.verify_lookup` returns for one token (S2 `tokens.sql`): the verification fields only.
#[derive(Clone)]
pub struct StoredToken {
    pub id: TokenId,
    pub worker: WorkerId,
    pub project: ProjectId,
    pub digest: [u8; 32],
    pub install_gen: i64,
    pub project_gen: i64,
    pub issued_at: Timestamp,
    pub expires_at: Timestamp,
    pub renew_after: Timestamp,
    pub last_used_at: Option<Timestamp>,
    pub revoked_at: Option<Timestamp>,
    pub revoke_reason: Option<String>,
    /// `identity.workers.state` as the function returns it (`active` or `retired`).
    pub worker_state: String,
    /// The CURRENT counters. `None` is a missing row, which means generation 0 (DV-4).
    pub current_install_gen: Option<i64>,
    pub current_project_gen: Option<i64>,
}

impl std::fmt::Debug for StoredToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredToken")
            .field("id", &self.id)
            .field("worker", &self.worker)
            .finish_non_exhaustive()
    }
}

/// The token store could not be reached or answered with an error: `AuthError::Unavailable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("token store unavailable")]
pub struct StoreError;

/// The database part of ONE verification.
#[async_trait]
pub trait VerifySession: Send {
    /// `tokens.verify_lookup(id)`: the row, whatever its project (the project comes from the token).
    async fn lookup(&mut self, id: &TokenId) -> Result<Option<StoredToken>, StoreError>;
    /// The worker's facts through `IdentityApi::worker`, in the token's project.
    async fn worker(
        &mut self,
        project: &ProjectId,
        w: &WorkerId,
    ) -> Result<Option<WorkerView>, StoreError>;
    /// Writes `last_used_at = now` (only `last_used_at`, never `first_used_at`; OQ-T3).
    async fn touch(
        &mut self,
        project: &ProjectId,
        id: &TokenId,
        now: Timestamp,
    ) -> Result<(), StoreError>;
    /// Commits whatever was written, whatever happens to the request (C6).
    async fn commit(self: Box<Self>) -> Result<(), StoreError>;
}

/// Opens one session per verification.
#[async_trait]
pub trait VerifyStore: Send + Sync {
    async fn open(&self) -> Result<Box<dyn VerifySession>, StoreError>;
}

/// The digest comparison: ONE call per verification that found a row or not (v1's dummy-digest rule).
pub trait DigestCompare: Send + Sync {
    fn equal(&self, stored: &[u8; 32], presented: &[u8; 32]) -> bool;
}

/// `subtle`'s constant-time comparison.
pub struct SubtleCompare;

impl DigestCompare for SubtleCompare {
    fn equal(&self, stored: &[u8; 32], presented: &[u8; 32]) -> bool {
        stored.ct_eq(presented).into()
    }
}

/// `TokenMeta.state` (S3 OQ-T4): superseded for the reasons `superseded`, `reissued` and `replaced_unused`;
/// revoked for every other code; expired when not revoked and `now >= expires_at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenState {
    Active,
    Revoked,
    Superseded,
    Expired,
}

impl TokenState {
    pub fn as_str(self) -> &'static str {
        match self {
            TokenState::Active => "active",
            TokenState::Revoked => "revoked",
            TokenState::Superseded => "superseded",
            TokenState::Expired => "expired",
        }
    }
}

/// The state a token reads as. A revocation wins over expiry.
pub fn state_of(
    revoked_at: Option<Timestamp>,
    revoke_reason: Option<&str>,
    expires_at: Timestamp,
    now: Timestamp,
) -> TokenState {
    match revoked_at {
        Some(_) => match revoke_reason {
            Some("superseded" | "reissued" | "replaced_unused") => TokenState::Superseded,
            _ => TokenState::Revoked,
        },
        None if now >= expires_at => TokenState::Expired,
        None => TokenState::Active,
    }
}

/// The pure decision for a row whose digest matched: revoked or superseded, expired, stale generation,
/// retired. `Ok` means the token authenticates.
pub fn check(row: &StoredToken, now: Timestamp) -> Result<(), AuthError> {
    if row.revoked_at.is_some() {
        return Err(AuthError::Revoked);
    }
    if now >= row.expires_at {
        return Err(AuthError::Expired);
    }
    // EQUALITY with the current counters; a missing row (None) is generation 0, never "unknown, so pass".
    if row.install_gen != row.current_install_gen.unwrap_or(0)
        || row.project_gen != row.current_project_gen.unwrap_or(0)
    {
        return Err(AuthError::Revoked);
    }
    // Fail closed: any state but `active` is a retired worker.
    if row.worker_state != "active" {
        return Err(AuthError::Retired);
    }
    Ok(())
}

/// Whether `last_used_at` is due: never written, or at least [`TOUCH_INTERVAL_MICROS`] ago.
pub fn touch_due(last_used_at: Option<Timestamp>, now: Timestamp) -> bool {
    match last_used_at {
        None => true,
        Some(last) => now.unix_micros() - last.unix_micros() >= TOUCH_INTERVAL_MICROS,
    }
}

/// `Principal.renew_due`: `now >= renew_after` (DEF-2).
pub fn renew_due(renew_after: Timestamp, now: Timestamp) -> bool {
    now >= renew_after
}

/// The `TokenVerifier` of this piece.
pub struct Verifier {
    store: Arc<dyn VerifyStore>,
    compare: Arc<dyn DigestCompare>,
    clock: Arc<dyn Clock>,
}

impl Verifier {
    /// With `subtle`'s comparison.
    pub fn new(store: Arc<dyn VerifyStore>, clock: Arc<dyn Clock>) -> Verifier {
        Verifier {
            store,
            compare: Arc::new(SubtleCompare),
            clock,
        }
    }

    /// With another comparison (a cell counts them).
    pub fn with_compare(
        store: Arc<dyn VerifyStore>,
        clock: Arc<dyn Clock>,
        compare: Arc<dyn DigestCompare>,
    ) -> Verifier {
        Verifier {
            store,
            compare,
            clock,
        }
    }
}

#[async_trait]
impl TokenVerifier for Verifier {
    async fn verify(&self, bearer: &Secret) -> Result<Principal, AuthError> {
        // Step 1: the format, with no lookup for a malformed bearer (TOK-02).
        let parsed = token::parse(bearer.expose()).map_err(|_| AuthError::Malformed)?;
        let unavailable = |_: StoreError| AuthError::Unavailable;
        let now = self.clock.now();
        // Step 2: the row by its id, whatever its project (the project comes from the token).
        let mut session = self.store.open().await.map_err(unavailable)?;
        let row = session.lookup(&parsed.id()).await.map_err(unavailable)?;
        // Step 3: ONE constant-time comparison whatever was found; an unknown id is compared against a zero
        // digest (v1's dummy-digest rule), so a wrong secret and an unknown id cost the same and answer alike.
        let zero = [0_u8; 32];
        let stored = row.as_ref().map_or(&zero, |r| &r.digest);
        let same = self.compare.equal(stored, &parsed.digest());
        let row = match row {
            Some(r) if same => r,
            _ => return Err(AuthError::Unknown),
        };
        // Step 4: the state of the token, in the ruled order.
        check(&row, now)?;
        // Step 5: the worker's facts, in the token's project (role, team, kind).
        let view = session
            .worker(&row.project, &row.worker)
            .await
            .map_err(unavailable)?
            .ok_or(AuthError::Unavailable)?;
        if !view.active {
            return Err(AuthError::Retired);
        }
        // Step 6: last_used_at, at most once a minute, and the commit (C6).
        if touch_due(row.last_used_at, now) {
            session
                .touch(&row.project, &row.id, now)
                .await
                .map_err(unavailable)?;
        }
        session.commit().await.map_err(unavailable)?;
        Ok(Principal {
            worker: row.worker,
            project: row.project,
            kind: view.kind,
            role: view.role,
            token: row.id,
            expires_at: row.expires_at,
            renew_due: renew_due(row.renew_after, now),
        })
    }
}
