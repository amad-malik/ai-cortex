//! Stage A (DB-free) cells for `TokenVerifier::verify` over in-memory doubles (L1): TOK-02 (no lookup for a
//! malformed bearer), TOK-04, TOK-10 to TOK-17, TOK-45, DEF-2, DEF-4, GEN-1, GEN-2 and GEN-5 to GEN-7 (the decision
//! side; the SQL side is stage B). Written from `mvp/tests/mod-tokens.md` r5. RED: `verify` and the pure rules
//! are `todo!()`.
mod common;

use std::sync::atomic::Ordering;

use common::mem::{DAY, Fixture, NOW};
use cortex_module::{AuthError, Role, TokenVerifier, WorkerKind};
use cortex_types::{Secret, Timestamp};
use mod_tokens::token;
use mod_tokens::verify::{TOUCH_INTERVAL_MICROS, TokenState, renew_due, state_of, touch_due};

fn ts(micros: i64) -> Timestamp {
    Timestamp::from_unix_micros(micros)
}

#[tokio::test]
async fn tok_02_a_malformed_bearer_is_malformed_and_costs_no_lookup() {
    let f = Fixture::new();
    let v = f.verifier();
    for bad in ["", "ctx2_", "ctx1_x", "not a token", &"A".repeat(10_000)] {
        assert_eq!(
            v.verify(&Secret::new(bad)).await.err(),
            Some(AuthError::Malformed)
        );
    }
    assert_eq!(
        f.store.0.opens.load(Ordering::SeqCst),
        0,
        "no session opened"
    );
    assert_eq!(f.store.0.lookups.load(Ordering::SeqCst), 0, "no lookup");
}

#[tokio::test]
async fn tok_04_exactly_one_digest_comparison_whatever_the_token() {
    let f = Fixture::new();
    let (v, c) = f.counting_verifier();
    // An unknown id: one comparison, against a zero digest.
    let unknown = token::generate(&*f.clock).expect("generated");
    assert_eq!(
        v.verify(unknown.token()).await.err(),
        Some(AuthError::Unknown)
    );
    assert_eq!(c.0.load(Ordering::SeqCst), 1, "unknown id");
    // A known id with a wrong secret: W1's id with W2's secret.
    let w1 = f.bearer(f.w1);
    let w2 = f.bearer(f.w2);
    let crossed = format!(
        "{}.{}",
        w1.expose().split_once('.').expect("dot").0,
        w2.expose().split_once('.').expect("dot").1
    );
    assert_eq!(
        v.verify(&Secret::new(crossed)).await.err(),
        Some(AuthError::Unknown)
    );
    assert_eq!(c.0.load(Ordering::SeqCst), 2, "known id, wrong secret");
    // The right secret.
    assert!(v.verify(&w1).await.is_ok());
    assert_eq!(c.0.load(Ordering::SeqCst), 3, "known id, right secret");
}

#[tokio::test]
async fn tok_10_the_principal_equals_the_identity_view() {
    let f = Fixture::new();
    let v = f.verifier();
    let expect = [
        (f.lead, Role::ProjectLead, f.a),
        (f.team_lead, Role::TeamLead(f.team), f.a),
        (f.w1, Role::Member(Some(f.team)), f.a),
        (f.w4, Role::Member(None), f.a),
        (f.wb1, Role::Member(None), f.b),
    ];
    for (w, role, project) in expect {
        let p = v.verify(&f.bearer(w)).await.expect("verifies");
        assert_eq!(p.worker, w);
        assert_eq!(p.role, role);
        assert_eq!(p.kind, WorkerKind::Worker);
        assert_eq!(
            p.project, project,
            "the ROW's project (WB1's token gives B)"
        );
        assert_eq!(p.token, f.id(w));
        let row = f.store.0.rows.lock().expect("lock")[&f.id(w)].clone();
        assert_eq!(p.expires_at, row.expires_at);
        assert!(!p.renew_due);
    }
}

#[tokio::test]
async fn tok_11_unknown_id_wrong_secret_and_crossed_secret_are_unknown_and_garbage_is_malformed() {
    let f = Fixture::new();
    let v = f.verifier();
    let unknown = token::generate(&*f.clock).expect("generated");
    assert_eq!(
        v.verify(unknown.token()).await.err(),
        Some(AuthError::Unknown)
    );
    let w1 = f.bearer(f.w1);
    let (head, _) = w1.expose().split_once('.').expect("dot");
    let other = token::generate(&*f.clock).expect("generated");
    let wrong = format!(
        "{head}.{}",
        other.token().expose().split_once('.').expect("dot").1
    );
    assert_eq!(
        v.verify(&Secret::new(wrong)).await.err(),
        Some(AuthError::Unknown)
    );
    let w2 = f.bearer(f.w2);
    let crossed = format!("{head}.{}", w2.expose().split_once('.').expect("dot").1);
    assert_eq!(
        v.verify(&Secret::new(crossed)).await.err(),
        Some(AuthError::Unknown)
    );
    assert_eq!(
        v.verify(&Secret::new("garbage")).await.err(),
        Some(AuthError::Malformed)
    );
}

#[tokio::test]
async fn tok_12_a_revoked_token_is_revoked_with_no_cache() {
    let f = Fixture::new();
    let v = f.verifier();
    assert!(v.verify(&f.bearer(f.w1)).await.is_ok());
    f.edit(f.w1, |r| {
        r.revoked_at = Some(ts(NOW));
        r.revoke_reason = Some("lost".into());
    });
    assert_eq!(
        v.verify(&f.bearer(f.w1)).await.err(),
        Some(AuthError::Revoked)
    );
    assert!(v.verify(&f.bearer(f.w2)).await.is_ok(), "W2 is untouched");
}

#[tokio::test]
async fn tok_13_valid_exactly_while_now_is_before_expires_at() {
    let f = Fixture::new();
    let v = f.verifier();
    let exp = f.store.0.rows.lock().expect("lock")[&f.id(f.w1)]
        .expires_at
        .unix_micros();
    f.set_now(exp - 1);
    assert!(
        v.verify(&f.bearer(f.w1)).await.is_ok(),
        "one microsecond before"
    );
    f.set_now(exp);
    assert_eq!(
        v.verify(&f.bearer(f.w1)).await.err(),
        Some(AuthError::Expired),
        "at expires_at"
    );
}

#[tokio::test]
async fn tok_14_a_retired_worker_is_retired() {
    let f = Fixture::new();
    f.edit(f.w1, |r| r.worker_state = "retired".into());
    assert_eq!(
        f.verifier().verify(&f.bearer(f.w1)).await.err(),
        Some(AuthError::Retired)
    );
    // The identity view says inactive although the lookup row still says active (a worker retired in between):
    // the same answer, and nothing is written.
    f.store
        .0
        .workers
        .lock()
        .expect("lock")
        .get_mut(&f.w2)
        .expect("w2")
        .active = false;
    assert_eq!(
        f.verifier().verify(&f.bearer(f.w2)).await.err(),
        Some(AuthError::Retired)
    );
    assert!(f.store.0.touches.lock().expect("lock").is_empty());
}

#[tokio::test]
async fn tok_15_precedence_revoked_then_expired_then_stale_then_retired_and_a_wrong_secret_is_unknown()
 {
    let f = Fixture::new();
    let v = f.verifier();
    let exp = f.store.0.rows.lock().expect("lock")[&f.id(f.w1)]
        .expires_at
        .unix_micros();
    f.set_now(exp + DAY); // everything below is past its expiry unless a cell says otherwise
    // revoked AND expired AND retired -> Revoked
    f.edit(f.w1, |r| {
        r.revoked_at = Some(ts(NOW));
        r.revoke_reason = Some("lost".into());
        r.worker_state = "retired".into();
    });
    assert_eq!(
        v.verify(&f.bearer(f.w1)).await.err(),
        Some(AuthError::Revoked)
    );
    // expired AND retired -> Expired
    f.edit(f.w2, |r| r.worker_state = "retired".into());
    assert_eq!(
        v.verify(&f.bearer(f.w2)).await.err(),
        Some(AuthError::Expired)
    );
    // superseded AND retired -> Revoked (not yet expired for this one)
    f.set_now(NOW);
    f.edit(f.w4, |r| {
        r.revoked_at = Some(ts(NOW));
        r.revoke_reason = Some("reissued".into());
        r.worker_state = "retired".into();
    });
    assert_eq!(
        v.verify(&f.bearer(f.w4)).await.err(),
        Some(AuthError::Revoked)
    );
    // A WRONG secret on any of them is Unknown: the state is disclosed only to a holder of the secret.
    for w in [f.w1, f.w2, f.w4] {
        let (head, _) = f.tokens[&w].0.split_once('.').expect("dot");
        let other = token::generate(&*f.clock).expect("generated");
        let wrong = format!(
            "{head}.{}",
            other.token().expose().split_once('.').expect("dot").1
        );
        assert_eq!(
            v.verify(&Secret::new(wrong)).await.err(),
            Some(AuthError::Unknown)
        );
    }
}

#[tokio::test]
async fn tok_16_a_store_that_is_down_is_unavailable_never_unknown() {
    let f = Fixture::new();
    f.store.0.down.store(true, Ordering::SeqCst);
    let v = f.verifier();
    assert_eq!(
        v.verify(&f.bearer(f.w1)).await.err(),
        Some(AuthError::Unavailable)
    );
    // The first request after the store returns succeeds.
    f.store.0.down.store(false, Ordering::SeqCst);
    assert!(v.verify(&f.bearer(f.w1)).await.is_ok());
}

#[tokio::test]
async fn tok_17_last_used_at_is_written_at_most_once_in_60_seconds_and_only_last_used_at() {
    let f = Fixture::new();
    let v = f.verifier();
    let touches = || f.store.0.touches.lock().expect("lock").len();
    assert!(v.verify(&f.bearer(f.w1)).await.is_ok());
    assert_eq!(touches(), 1, "never used: written");
    f.set_now(NOW + 30_000_000);
    assert!(v.verify(&f.bearer(f.w1)).await.is_ok());
    assert_eq!(touches(), 1, "30 s later: no write");
    f.set_now(NOW + 61_000_000);
    assert!(v.verify(&f.bearer(f.w1)).await.is_ok());
    assert_eq!(touches(), 2, "61 s later: written");
    // A refused token writes nothing.
    f.edit(f.w2, |r| {
        r.revoked_at = Some(ts(NOW));
        r.revoke_reason = Some("lost".into());
    });
    assert!(v.verify(&f.bearer(f.w2)).await.is_err());
    assert_eq!(touches(), 2, "a refusal writes nothing");
    // The pure rule: exactly 60 s is due, one microsecond less is not.
    assert!(!touch_due(
        Some(ts(NOW)),
        ts(NOW + TOUCH_INTERVAL_MICROS - 1)
    ));
    assert!(touch_due(Some(ts(NOW)), ts(NOW + TOUCH_INTERVAL_MICROS)));
    assert!(touch_due(None, ts(NOW)));
}

#[tokio::test]
async fn gen_01_02_a_stale_generation_is_revoked_after_expired_and_before_retired() {
    let f = Fixture::new();
    let v = f.verifier();
    // GEN-1: the install counter advanced after W1's token was issued.
    f.edit(f.w1, |r| r.current_install_gen = Some(1));
    assert_eq!(
        v.verify(&f.bearer(f.w1)).await.err(),
        Some(AuthError::Revoked)
    );
    // GEN-2: only the project counter of A advanced: A's tokens refuse, B's verify.
    f.edit(f.lead, |r| r.current_project_gen = Some(1));
    assert_eq!(
        v.verify(&f.bearer(f.lead)).await.err(),
        Some(AuthError::Revoked)
    );
    assert!(v.verify(&f.bearer(f.wb1)).await.is_ok(), "B is valid");
    // Order: expired and stale -> Expired; stale and retired -> Revoked.
    let exp = f.store.0.rows.lock().expect("lock")[&f.id(f.w2)]
        .expires_at
        .unix_micros();
    f.edit(f.w2, |r| r.current_project_gen = Some(1));
    f.set_now(exp);
    assert_eq!(
        v.verify(&f.bearer(f.w2)).await.err(),
        Some(AuthError::Expired)
    );
    f.set_now(NOW);
    f.edit(f.w4, |r| {
        r.current_project_gen = Some(1);
        r.worker_state = "retired".into();
    });
    assert_eq!(
        v.verify(&f.bearer(f.w4)).await.err(),
        Some(AuthError::Revoked)
    );
}

#[tokio::test]
async fn gen_05_06_a_missing_generation_row_is_generation_zero_and_a_raise_refuses_the_old_token() {
    let f = Fixture::new();
    let v = f.verifier();
    // GEN-5: no rows; the token stored 0 and 0; equality 0 = 0.
    f.edit(f.w1, |r| {
        r.current_install_gen = None;
        r.current_project_gen = None;
    });
    assert!(
        v.verify(&f.bearer(f.w1)).await.is_ok(),
        "missing = 0 = stored 0"
    );
    // GEN-6: the row is created as 1: stored 0 != 1.
    f.edit(f.w1, |r| r.current_install_gen = Some(1));
    assert_eq!(
        v.verify(&f.bearer(f.w1)).await.err(),
        Some(AuthError::Revoked)
    );
    f.edit(f.w1, |r| {
        r.current_install_gen = None;
        r.current_project_gen = Some(1);
    });
    assert_eq!(
        v.verify(&f.bearer(f.w1)).await.err(),
        Some(AuthError::Revoked),
        "the project counter, too"
    );
    // A token issued after the raise stored 1 and works.
    f.edit(f.w2, |r| {
        r.install_gen = 1;
        r.project_gen = 1;
        r.current_install_gen = Some(1);
        r.current_project_gen = Some(1);
    });
    assert!(v.verify(&f.bearer(f.w2)).await.is_ok());
}

#[tokio::test]
async fn gen_07_a_null_current_generation_is_zero_and_never_a_pass() {
    // T0 stored 0 and 0; T1 stored 1 and 1. The lookup returns NULL for the install value, then for the project
    // value, then for both. The counter that is NOT null holds the value that matches the token's stored one
    // (0 for T0, 1 for T1), so the NULL counter alone decides: it is read as generation 0. T0 verifies; T1 is
    // Revoked. A verifier that read NULL as "unknown, so pass" would accept T1 in every case.
    for (null_install, null_project) in [(true, false), (false, true), (true, true)] {
        let f = Fixture::new();
        f.edit(f.w2, |r| {
            r.install_gen = 1;
            r.project_gen = 1;
            r.current_install_gen = Some(1);
            r.current_project_gen = Some(1);
        });
        f.store.0.null_install.store(null_install, Ordering::SeqCst);
        f.store.0.null_project.store(null_project, Ordering::SeqCst);
        let v = f.verifier();
        assert!(
            v.verify(&f.bearer(f.w1)).await.is_ok(),
            "T0 verifies ({null_install},{null_project})"
        );
        assert_eq!(
            v.verify(&f.bearer(f.w2)).await.err(),
            Some(AuthError::Revoked),
            "T1 is revoked ({null_install},{null_project})"
        );
    }
}

#[test]
fn def_2_renew_due_flips_exactly_at_renew_after() {
    let ra = ts(NOW + 60 * DAY);
    assert!(!renew_due(ra, ts(NOW + 60 * DAY - 1)));
    assert!(renew_due(ra, ra));
}

#[tokio::test]
async fn def_2b_the_principal_carries_renew_due() {
    let f = Fixture::new();
    let v = f.verifier();
    let ra = f.store.0.rows.lock().expect("lock")[&f.id(f.w1)]
        .renew_after
        .unix_micros();
    f.set_now(ra - 1);
    assert!(!v.verify(&f.bearer(f.w1)).await.expect("ok").renew_due);
    f.set_now(ra);
    assert!(v.verify(&f.bearer(f.w1)).await.expect("ok").renew_due);
}

#[test]
fn tok_45_the_state_a_token_reads_as() {
    let exp = ts(NOW + 90 * DAY);
    let now = ts(NOW);
    let after = ts(NOW + 91 * DAY);
    let rev = Some(ts(NOW - 1));
    assert_eq!(state_of(None, None, exp, now), TokenState::Active);
    assert_eq!(
        state_of(None, None, exp, exp),
        TokenState::Expired,
        "now >= expires_at"
    );
    assert_eq!(state_of(rev, Some("lost"), exp, now), TokenState::Revoked);
    for superseded in ["superseded", "reissued", "replaced_unused"] {
        assert_eq!(
            state_of(rev, Some(superseded), exp, now),
            TokenState::Superseded,
            "{superseded}"
        );
    }
    for revoked in ["retired", "self", "compromised", "lost_key"] {
        assert_eq!(
            state_of(rev, Some(revoked), exp, now),
            TokenState::Revoked,
            "{revoked}"
        );
    }
    // The revocation wins over expiry.
    assert_eq!(state_of(rev, Some("lost"), exp, after), TokenState::Revoked);
    assert_eq!(
        state_of(rev, Some("reissued"), exp, after),
        TokenState::Superseded
    );
    assert_eq!(TokenState::Superseded.as_str(), "superseded");
}

/// DEF-4: the 401 codes are only the five; there is no `Superseded` or `Stale` variant, so neither can be
/// produced. This match has no wildcard: adding a variant to `AuthError` stops this file compiling until the
/// cell is looked at.
#[test]
fn def_4_the_auth_errors_are_exactly_the_seven_without_superseded_or_stale() {
    fn code(e: AuthError) -> &'static str {
        match e {
            AuthError::Missing => "token_missing",
            AuthError::Malformed | AuthError::Unknown => "token_invalid",
            AuthError::Expired => "token_expired",
            AuthError::Revoked => "token_revoked",
            AuthError::Retired => "worker_retired",
            AuthError::Unavailable => "unavailable",
        }
    }
    assert_eq!(code(AuthError::Revoked), "token_revoked");
}
