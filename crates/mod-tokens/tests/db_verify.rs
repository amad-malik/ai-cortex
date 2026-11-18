//! Stage B (database) cells: the SQL store of the verifier, the generation counters, and the grants, constraints
//! and isolation of `tokens.sql` (TOK-03, 10 to 12, 14 to 17, 19, 46, 80 to 84, GEN-1 to GEN-3, GEN-5, GEN-6 on a
//! database). Needs `supervise.sh`'s throwaway Postgres (`CV2_DB=1`, `CV2_PG_PORT`, `CV2_PG_PASSWORD`).
//! RED: `SqlStore` is `todo!()`.
mod common;

use std::sync::Arc;

use common::db::{DAY, Env, NOW, SHORT, SqlIdentity, setup};
use cortex_module::{AuthError, Role, TokenVerifier};
use cortex_testkit::sqlstate;
use cortex_types::{ProjectId, Secret, TokenId, WorkerId};
use mod_tokens::VerifyStore;
use mod_tokens::store::SqlStore;
use mod_tokens::token;
use sha2::{Digest, Sha256};
use sqlx::Row;

async fn insert(
    env: &Env,
    worker: WorkerId,
    project: ProjectId,
    install_gen: i64,
    project_gen: i64,
) -> (String, TokenId) {
    common::db::insert_token(
        &env.pool,
        &*env.clock,
        worker,
        project,
        install_gen,
        project_gen,
    )
    .await
}

fn secret(b: &str) -> Secret {
    Secret::new(b)
}

/// Test SQL is built from our own ids and numbers, never from input.
fn safe(sql: impl Into<String>) -> sqlx::AssertSqlSafe<String> {
    sqlx::AssertSqlSafe(sql.into())
}

async fn admin_i64(env: &Env, sql: impl Into<String>) -> Option<i64> {
    let mut a = env.db.admin().await;
    sqlx::query_scalar::<_, Option<i64>>(safe(sql))
        .fetch_one(&mut a)
        .await
        .expect("admin query")
}

async fn admin_exec(env: &Env, sql: impl Into<String>) {
    let mut a = env.db.admin().await;
    sqlx::raw_sql(safe(sql))
        .execute(&mut a)
        .await
        .expect("admin exec");
}

fn last_used(id: TokenId) -> String {
    format!(
        "SELECT (extract(epoch from last_used_at) * 1000000)::bigint FROM tokens.tokens WHERE id = '{id}'"
    )
}

/// vera's TOKENS-V3: two verifications that read the SAME row (both saw `last_used_at` NULL or overdue) must write
/// `last_used_at` once. The earlier check on the row read in the first transaction is only an optimisation; the
/// UPDATE itself carries the freshness condition against the STORED value, and never moves the stamp backwards.
#[tokio::test]
async fn tok_17b_two_verifications_that_read_the_same_row_write_last_used_at_once_and_never_backwards()
 {
    let env = setup("tk_touch2").await;
    let s = env.seed;
    let (_b, id) = insert(&env, s.w1, s.a, 0, 0).await;
    let store = SqlStore::new(env.pool.clone(), Arc::new(SqlIdentity), env.clock.clone());
    let at = |micros: i64| mod_tokens_ts(micros);
    let xmin =
        |id: TokenId| format!("SELECT xmin::text::bigint FROM tokens.tokens WHERE id = '{id}'");

    // Both sessions read the row BEFORE either writes: both see last_used_at NULL.
    let mut a = store.open().await.expect("session A");
    let mut b = store.open().await.expect("session B");
    let ra = a.lookup(&id).await.expect("A lookup").expect("A row");
    let rb = b.lookup(&id).await.expect("B lookup").expect("B row");
    assert!(ra.last_used_at.is_none() && rb.last_used_at.is_none());
    let (project, worker) = (ra.project, ra.worker);

    // A writes and commits.
    a.worker(&project, &worker).await.expect("A worker");
    a.touch(&project, &id, at(NOW)).await.expect("A touch");
    a.commit().await.expect("A commit");
    assert_eq!(admin_i64(&env, &last_used(id)).await, Some(NOW));
    let after_a = admin_i64(&env, &xmin(id)).await;

    // B decided on the stale row, one second later: its UPDATE changes no row.
    b.worker(&project, &worker).await.expect("B worker");
    b.touch(&project, &id, at(NOW + 1_000_000))
        .await
        .expect("B touch");
    b.commit().await.expect("B commit");
    assert_eq!(
        admin_i64(&env, &last_used(id)).await,
        Some(NOW),
        "within the minute of A's write, B writes nothing"
    );
    assert_eq!(
        admin_i64(&env, &xmin(id)).await,
        after_a,
        "the row version did not move"
    );

    // A later use that really is due still writes: exactly 60 s after is due, 59.999999 s is not.
    let touch_at = |when: i64| {
        let store = &store;
        let (project, worker) = (project, worker);
        async move {
            let mut s = store.open().await.expect("session");
            s.worker(&project, &worker).await.expect("worker");
            s.touch(&project, &id, at(when)).await.expect("touch");
            s.commit().await.expect("commit");
        }
    };
    touch_at(NOW + 59_999_999).await;
    assert_eq!(
        admin_i64(&env, &last_used(id)).await,
        Some(NOW),
        "59.999999 s is not due"
    );
    touch_at(NOW + 60_000_000).await;
    assert_eq!(
        admin_i64(&env, &last_used(id)).await,
        Some(NOW + 60_000_000),
        "60 s is due"
    );

    // Never backwards: a session whose clock is behind the stored stamp changes nothing, however far behind.
    let stored = NOW + 60_000_000;
    for earlier in [stored - 1, stored - 59_999_999, stored - 60_000_000, NOW, 0] {
        touch_at(earlier).await;
        assert_eq!(
            admin_i64(&env, &last_used(id)).await,
            Some(stored),
            "a touch at {earlier} moved the stamp"
        );
    }
    // And first_used_at is still never written.
    assert_eq!(
        admin_i64(
            &env,
            format!("SELECT (extract(epoch from first_used_at) * 1000000)::bigint FROM tokens.tokens WHERE id = '{id}'")
        )
        .await,
        None
    );
}

fn mod_tokens_ts(micros: i64) -> cortex_types::Timestamp {
    cortex_types::Timestamp::from_unix_micros(micros)
}

#[tokio::test]
async fn tok_03_only_the_digest_of_the_whole_token_is_stored() {
    let env = setup("tk_digest").await;
    let s = env.seed;
    let (bearer, id) = insert(&env, s.w1, s.a, 0, 0).await;
    let (b2, id2) = insert(&env, s.w1, s.a, 0, 0).await;
    let mut a = env.db.admin().await;
    let digest: Vec<u8> =
        sqlx::query_scalar("SELECT digest FROM tokens.tokens WHERE id = $1::uuid")
            .bind(id.to_string())
            .fetch_one(&mut a)
            .await
            .expect("digest");
    let want: [u8; 32] = Sha256::digest(bearer.as_bytes()).into();
    assert_eq!(digest, want.to_vec(), "SHA-256 over the whole string");
    let secret_only: [u8; 32] = Sha256::digest(&bearer.as_bytes()[38..]).into();
    assert_ne!(digest, secret_only.to_vec());
    // Render every column of every row as text: neither the secret nor the token is there.
    let rows: Vec<String> = sqlx::query_scalar("SELECT t::text FROM tokens.tokens t")
        .fetch_all(&mut a)
        .await
        .expect("rows");
    for text in rows {
        for b in [&bearer, &b2] {
            assert!(
                !text.contains(b.as_str()) && !text.contains(&b[38..]),
                "the row shows a secret"
            );
        }
    }
    let d2: Vec<u8> = sqlx::query_scalar("SELECT digest FROM tokens.tokens WHERE id = $1::uuid")
        .bind(id2.to_string())
        .fetch_one(&mut a)
        .await
        .expect("digest 2");
    assert_ne!(
        digest, d2,
        "two tokens of one worker have different digests"
    );
}

#[tokio::test]
async fn tok_10_11_12_14_the_principal_the_unknown_cases_revocation_and_retirement_on_a_database() {
    let env = setup("tk_verify").await;
    let s = env.seed;
    let v = env.verifier();
    let team = s.t;
    let mut tokens = std::collections::HashMap::new();
    for (w, p) in [
        (s.lead, s.a),
        (s.tl, s.a),
        (s.w1, s.a),
        (s.w2, s.a),
        (s.w4, s.a),
        (s.wb1, s.b),
    ] {
        tokens.insert(w, insert(&env, w, p, 0, 0).await);
    }
    // TOK-10: the principal equals the identity view.
    for (w, role, project) in [
        (s.lead, Role::ProjectLead, s.a),
        (s.tl, Role::TeamLead(team), s.a),
        (s.w1, Role::Member(Some(team)), s.a),
        (s.w4, Role::Member(None), s.a),
        (s.wb1, Role::Member(None), s.b),
    ] {
        let (bearer, id) = &tokens[&w];
        let p = v.verify(&secret(bearer)).await.expect("verifies");
        assert_eq!(
            (p.worker, p.role, p.project, p.token),
            (w, role, project, *id)
        );
        assert!(!p.renew_due);
    }
    // TOK-11 (L2): unknown id and wrong secrets are Unknown, garbage is Malformed.
    let unknown = token::generate(&*env.clock).expect("generated");
    assert_eq!(
        v.verify(unknown.token()).await.err(),
        Some(AuthError::Unknown)
    );
    let (w1, _) = &tokens[&s.w1];
    let (w2, _) = &tokens[&s.w2];
    let wrong = format!("{}.{}", &w1[..37], &unknown.token().expose()[38..]);
    let crossed = format!("{}.{}", &w1[..37], &w2[38..]);
    assert_eq!(
        v.verify(&secret(&wrong)).await.err(),
        Some(AuthError::Unknown)
    );
    assert_eq!(
        v.verify(&secret(&crossed)).await.err(),
        Some(AuthError::Unknown)
    );
    assert_eq!(
        v.verify(&secret("garbage")).await.err(),
        Some(AuthError::Malformed)
    );
    // TOK-12: revoke W1's token; verify on another pooled connection (no cache).
    let w1_id = tokens[&s.w1].1;
    admin_exec(&env, &format!("UPDATE tokens.tokens SET revoked_at = now(), revoke_reason = 'lost' WHERE id = '{w1_id}'")).await;
    assert_eq!(v.verify(&secret(w1)).await.err(), Some(AuthError::Revoked));
    assert!(
        v.verify(&secret(w2)).await.is_ok(),
        "W2's token is untouched"
    );
    // TOK-14: retire W2 through identity; its token is Retired, and a retired worker never recovers.
    admin_exec(
        &env,
        &format!(
            "UPDATE identity.workers SET state = 'retired', retired_at = now() WHERE id = '{}'",
            s.w2
        ),
    )
    .await;
    assert_eq!(v.verify(&secret(w2)).await.err(), Some(AuthError::Retired));
}

#[tokio::test]
async fn tok_13_15_expiry_boundary_and_precedence_on_a_database() {
    let env = setup("tk_order").await;
    let s = env.seed;
    let v = env.verifier();
    let (b1, id1) = insert(&env, s.w1, s.a, 0, 0).await;
    env.set_now(NOW + 90 * DAY - 1);
    assert!(
        v.verify(&secret(&b1)).await.is_ok(),
        "one microsecond before expires_at"
    );
    env.set_now(NOW + 90 * DAY);
    assert_eq!(
        v.verify(&secret(&b1)).await.err(),
        Some(AuthError::Expired),
        "at expires_at"
    );
    // revoked AND expired AND retired -> Revoked; expired AND retired -> Expired.
    let (b2, id2) = insert(&env, s.w2, s.a, 0, 0).await;
    admin_exec(&env, &format!("UPDATE identity.workers SET state = 'retired', retired_at = now() WHERE id IN ('{}', '{}')", s.w1, s.w2)).await;
    assert_eq!(
        v.verify(&secret(&b1)).await.err(),
        Some(AuthError::Expired),
        "expired and retired"
    );
    admin_exec(&env, &format!("UPDATE tokens.tokens SET revoked_at = now(), revoke_reason = 'reissued' WHERE id = '{id2}'")).await;
    assert_eq!(
        v.verify(&secret(&b2)).await.err(),
        Some(AuthError::Revoked),
        "superseded, expired and retired"
    );
    // A wrong secret on any of them is Unknown.
    let other = token::generate(&*env.clock).expect("generated");
    for (b, _id) in [(&b1, id1), (&b2, id2)] {
        let wrong = format!("{}.{}", &b[..37], &other.token().expose()[38..]);
        assert_eq!(
            v.verify(&secret(&wrong)).await.err(),
            Some(AuthError::Unknown)
        );
    }
}

#[tokio::test]
async fn tok_16_a_dead_database_and_a_statement_timeout_are_unavailable_and_the_next_request_succeeds()
 {
    let env = setup("tk_down").await;
    let s = env.seed;
    let (b1, _id) = insert(&env, s.w1, s.a, 0, 0).await;
    // A pool to a port where nothing listens.
    let dead = Arc::new(cortex_db::DbPool::connect_lazy(
        &env.db
            .dead_db_config(&["tokens", "identity", "platform"], env.clock.clone()),
    ));
    let down = env.verifier_with(SqlStore::new(
        dead,
        Arc::new(SqlIdentity),
        env.clock.clone(),
    ));
    assert_eq!(
        down.verify(&secret(&b1)).await.err(),
        Some(AuthError::Unavailable)
    );
    // A statement timeout: another session holds the table exclusively, so the lookup waits.
    let short = env.verifier_with(
        SqlStore::new(env.pool.clone(), Arc::new(SqlIdentity), env.clock.clone())
            .with_statement_timeout(SHORT),
    );
    let mut blocker = env.db.admin().await;
    sqlx::raw_sql("BEGIN; LOCK TABLE tokens.tokens IN ACCESS EXCLUSIVE MODE;")
        .execute(&mut blocker)
        .await
        .expect("lock");
    assert_eq!(
        short.verify(&secret(&b1)).await.err(),
        Some(AuthError::Unavailable)
    );
    sqlx::raw_sql("ROLLBACK")
        .execute(&mut blocker)
        .await
        .expect("release");
    assert!(
        short.verify(&secret(&b1)).await.is_ok(),
        "the first request after the lock is gone succeeds"
    );
}

#[tokio::test]
async fn tok_17_46_only_last_used_at_changes_at_most_once_a_minute_and_only_the_presented_token_moves()
 {
    let env = setup("tk_touch").await;
    let s = env.seed;
    let v = env.verifier();
    let (b1, id1) = insert(&env, s.w1, s.a, 0, 0).await;
    let (_b2, id2) = insert(&env, s.w2, s.a, 0, 0).await;
    let snapshot = || {
        "SELECT string_agg(t.id::text || digest::text || issued_at::text || expires_at::text || renew_after::text \
         || coalesce(revoked_at::text, '') || coalesce(first_used_at::text, '') || coalesce(predecessor_id::text, ''), ',' ORDER BY t.id) \
         FROM tokens.tokens t"
    };
    let mut a = env.db.admin().await;
    let before: Option<String> = sqlx::query_scalar(snapshot())
        .fetch_one(&mut a)
        .await
        .expect("snapshot");
    let platform_before = admin_i64(&env, "SELECT count(*) FROM platform.idempotency").await;
    assert!(v.verify(&secret(&b1)).await.is_ok());
    let lu1 = admin_i64(&env, &last_used(id1)).await;
    assert_eq!(lu1, Some(NOW), "last_used_at is the injected now");
    let xmin1 = admin_i64(
        &env,
        &format!("SELECT xmin::text::bigint FROM tokens.tokens WHERE id = '{id1}'"),
    )
    .await;
    // 30 s later: no write (the row version is unchanged).
    env.set_now(NOW + 30_000_000);
    assert!(v.verify(&secret(&b1)).await.is_ok());
    assert_eq!(
        admin_i64(
            &env,
            &format!("SELECT xmin::text::bigint FROM tokens.tokens WHERE id = '{id1}'")
        )
        .await,
        xmin1
    );
    // 61 s later: a write.
    env.set_now(NOW + 61_000_000);
    assert!(v.verify(&secret(&b1)).await.is_ok());
    assert_eq!(
        admin_i64(&env, &last_used(id1)).await,
        Some(NOW + 61_000_000)
    );
    // Nothing but last_used_at changed, first_used_at stays NULL, nothing else was written, and X's metadata read
    // with Y's token (id2) did not move X (TOK-46).
    let after: Option<String> = sqlx::query_scalar(snapshot())
        .fetch_one(&mut a)
        .await
        .expect("snapshot");
    assert_eq!(before, after, "only last_used_at changed");
    assert_eq!(
        admin_i64(&env, &last_used(id2)).await,
        None,
        "presenting W1's token never moves W2's"
    );
    assert_eq!(
        admin_i64(&env, "SELECT count(*) FROM platform.idempotency").await,
        platform_before
    );
    assert_eq!(
        admin_i64(
            &env,
            "SELECT count(*) FROM tokens.tokens WHERE first_used_at IS NOT NULL"
        )
        .await,
        Some(0)
    );
}

#[tokio::test]
async fn tok_19_the_lookup_is_the_one_door_across_projects_and_only_the_module_role_may_call_it() {
    let env = setup("tk_lookup").await;
    let s = env.seed;
    let (_ba, ida) = insert(&env, s.w1, s.a, 0, 0).await;
    let (_bb, idb) = insert(&env, s.wb1, s.b, 0, 0).await;
    let mut c = env.db.admin().await;
    sqlx::raw_sql("SET ROLE cortex_m_tokens")
        .execute(&mut c)
        .await
        .expect("role");
    sqlx::query("SELECT set_config('cortex.project_id', $1, false)")
        .bind(s.b.to_string())
        .execute(&mut c)
        .await
        .expect("project");
    let ids: Vec<String> = sqlx::query_scalar("SELECT id::text FROM tokens.tokens")
        .fetch_all(&mut c)
        .await
        .expect("select");
    assert_eq!(
        ids,
        vec![idb.to_string()],
        "the table shows only project B's rows"
    );
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM tokens.verify_lookup($1::uuid)")
        .bind(ida.to_string())
        .fetch_one(&mut c)
        .await
        .expect("lookup of A's token from B's session");
    assert_eq!(
        n, 1,
        "the function returns A's row (by design: the project comes from the token)"
    );
    let cols = sqlx::query("SELECT * FROM tokens.verify_lookup($1::uuid)")
        .bind(ida.to_string())
        .fetch_one(&mut c)
        .await
        .expect("row");
    let names: Vec<String> = cols
        .columns()
        .iter()
        .map(|c| sqlx::Column::name(c).to_owned())
        .collect();
    assert_eq!(
        names,
        [
            "id",
            "worker_id",
            "project_id",
            "digest",
            "install_gen",
            "project_gen",
            "issued_at",
            "expires_at",
            "renew_after",
            "last_used_at",
            "first_used_at",
            "predecessor_id",
            "revoked_at",
            "revoke_reason",
            "worker_state",
            "current_install_gen",
            "current_project_gen"
        ]
    );
    // EXECUTE is refused for other roles.
    for role in ["cortex_m_audit", "cortex_m_test_a", "cortex_m_identity"] {
        let mut d = env.db.admin().await;
        sqlx::raw_sql(safe(format!("SET ROLE {role}")))
            .execute(&mut d)
            .await
            .expect("role");
        let e = sqlx::query("SELECT * FROM tokens.verify_lookup($1::uuid)")
            .bind(ida.to_string())
            .fetch_all(&mut d)
            .await
            .expect_err("refused");
        assert_eq!(sqlstate(&e).as_deref(), Some("42501"), "{role}");
    }
}

#[tokio::test]
async fn gen_01_02_03_05_06_the_generation_counters_on_a_database() {
    let env = setup("tk_gen").await;
    let s = env.seed;
    let v = env.verifier();
    // GEN-5: no generation rows exist; a token stamped 0 and 0 verifies (a missing row is generation 0).
    let (t0, _) = insert(&env, s.w1, s.a, 0, 0).await;
    let (tb0, _) = insert(&env, s.wb1, s.b, 0, 0).await;
    assert!(
        v.verify(&secret(&t0)).await.is_ok(),
        "missing rows = 0 = stored 0"
    );
    assert_eq!(
        admin_i64(&env, "SELECT count(*) FROM tokens.generations").await,
        Some(0),
        "verify creates no row"
    );
    // GEN-6: the install row is created as 1 (as cortex_install_admin: the module role has SELECT only).
    let mut host = env.db.admin().await;
    sqlx::raw_sql("SET ROLE cortex_install_admin")
        .execute(&mut host)
        .await
        .expect("role");
    sqlx::query(
        "INSERT INTO tokens.generations (scope, scope_id, gen) VALUES ('install', $1::uuid, 1)",
    )
    .bind(s.inst.to_string())
    .execute(&mut host)
    .await
    .expect("install gen");
    assert_eq!(
        v.verify(&secret(&t0)).await.err(),
        Some(AuthError::Revoked),
        "stored 0 != current 1"
    );
    assert_eq!(
        v.verify(&secret(&tb0)).await.err(),
        Some(AuthError::Revoked),
        "B's tokens too after an install advance (GEN-1)"
    );
    let (t1, _) = insert(&env, s.w1, s.a, 1, 0).await;
    assert!(
        v.verify(&secret(&t1)).await.is_ok(),
        "a token issued after the advance stored 1"
    );
    // GEN-2: advance project A's generation only: A's tokens refuse, B's (stamped 1/0) verify.
    let (tb1, _) = insert(&env, s.wb1, s.b, 1, 0).await;
    sqlx::query(
        "INSERT INTO tokens.generations (scope, scope_id, gen) VALUES ('project', $1::uuid, 1)",
    )
    .bind(s.a.to_string())
    .execute(&mut host)
    .await
    .expect("project gen");
    assert_eq!(
        v.verify(&secret(&t1)).await.err(),
        Some(AuthError::Revoked),
        "A: stored 0 != project gen 1"
    );
    assert!(v.verify(&secret(&tb1)).await.is_ok(), "B is valid");
    // GEN-3 (the restore witness, emulated at the data level): a revoked token comes back valid in the restored data;
    // WITH an advance of the install generation it is refused, WITHOUT it it verifies again (the control).
    let (tr, idr) = insert(&env, s.w2, s.a, 1, 1).await;
    admin_exec(
        &env,
        &format!(
            "UPDATE tokens.tokens SET revoked_at = now(), revoke_reason = 'lost' WHERE id = '{idr}'"
        ),
    )
    .await;
    assert_eq!(v.verify(&secret(&tr)).await.err(), Some(AuthError::Revoked));
    admin_exec(
        &env,
        &format!(
            "UPDATE tokens.tokens SET revoked_at = NULL, revoke_reason = NULL WHERE id = '{idr}'"
        ),
    )
    .await;
    assert!(
        v.verify(&secret(&tr)).await.is_ok(),
        "CONTROL: without the advance the restored token verifies again"
    );
    sqlx::query("UPDATE tokens.generations SET gen = 2 WHERE scope = 'install'")
        .execute(&mut host)
        .await
        .expect("advance");
    assert_eq!(
        v.verify(&secret(&tr)).await.err(),
        Some(AuthError::Revoked),
        "with the advance the restored token is refused"
    );
}

#[tokio::test]
async fn tok_80_81_the_module_roles_grants_and_who_else_may_touch_the_schema() {
    let env = setup("tk_grants").await;
    let s = env.seed;
    let (_b, id) = insert(&env, s.w1, s.a, 0, 0).await;
    let (_bb, idb) = insert(&env, s.wb1, s.b, 0, 0).await;
    let as_tokens = || async {
        let mut c = env.db.admin().await;
        sqlx::raw_sql("SET ROLE cortex_m_tokens")
            .execute(&mut c)
            .await
            .expect("role");
        sqlx::query("SELECT set_config('cortex.project_id', $1, false)")
            .bind(s.a.to_string())
            .execute(&mut c)
            .await
            .expect("project");
        c
    };
    let code = |e: sqlx::Error| sqlstate(&e).unwrap_or_default();
    let mut c = as_tokens().await;
    // SELECT shows only A.
    let ids: Vec<String> = sqlx::query_scalar("SELECT id::text FROM tokens.tokens")
        .fetch_all(&mut c)
        .await
        .expect("select");
    assert_eq!(ids, vec![id.to_string()]);
    // INSERT with B's project_id fails RLS (42501).
    let e = sqlx::query(
        "INSERT INTO tokens.tokens (id, worker_id, project_id, digest, install_gen, project_gen, issued_at, expires_at, renew_after) \
         SELECT gen_random_uuid(), worker_id, $1::uuid, digest, 0, 0, issued_at, expires_at, renew_after FROM tokens.tokens LIMIT 1",
    )
    .bind(s.b.to_string())
    .execute(&mut c)
    .await
    .expect_err("RLS");
    assert_eq!(code(e), "42501");
    // The seven columns the role may not update, DELETE, TRUNCATE and the generation writes: 42501 each.
    for col_set in [
        "digest = digest",
        "expires_at = expires_at",
        "issued_at = issued_at",
        "worker_id = worker_id",
        "project_id = project_id",
        "renew_after = renew_after",
        "issued_by = issued_by",
    ] {
        let mut c = as_tokens().await;
        let e = sqlx::query(safe(format!("UPDATE tokens.tokens SET {col_set}")))
            .execute(&mut c)
            .await
            .expect_err(col_set);
        assert_eq!(code(e), "42501", "{col_set}");
    }
    for sql in [
        "DELETE FROM tokens.tokens",
        "TRUNCATE tokens.tokens",
        "INSERT INTO tokens.generations (scope, scope_id, gen) VALUES ('install', gen_random_uuid(), 9)",
        "UPDATE tokens.generations SET gen = gen + 1",
    ] {
        let mut c = as_tokens().await;
        let e = sqlx::query(sqlx::AssertSqlSafe(sql))
            .execute(&mut c)
            .await
            .expect_err(sql);
        assert_eq!(code(e), "42501", "{sql}");
    }
    // The four allowed columns update (within A); B's row is out of reach.
    let mut c = as_tokens().await;
    let n = sqlx::query("UPDATE tokens.tokens SET last_used_at = now(), first_used_at = now(), revoked_at = now(), revoke_reason = 'lost'")
        .execute(&mut c)
        .await
        .expect("allowed columns")
        .rows_affected();
    assert_eq!(n, 1, "only A's row");
    assert_eq!(
        admin_i64(
            &env,
            &format!(
                "SELECT count(*) FROM tokens.tokens WHERE id = '{idb}' AND revoked_at IS NULL"
            )
        )
        .await,
        Some(1)
    );
    // TOK-81: other module roles have no access to the schema.
    for role in ["cortex_m_identity", "cortex_m_audit", "cortex_m_test_a"] {
        let mut d = env.db.admin().await;
        sqlx::raw_sql(safe(format!("SET ROLE {role}")))
            .execute(&mut d)
            .await
            .expect("role");
        let e = sqlx::query("SELECT * FROM tokens.tokens")
            .fetch_all(&mut d)
            .await
            .expect_err(role);
        assert_eq!(code(e), "42501", "{role}");
    }
}

#[tokio::test]
async fn tok_82_83_84_constraints_history_and_the_two_project_matrix() {
    let env = setup("tk_cons").await;
    let s = env.seed;
    let (_b, _id) = insert(&env, s.w1, s.a, 0, 0).await;
    let code = |e: sqlx::Error| sqlstate(&e).unwrap_or_default();
    let base = |expires: &str,
                renew: &str,
                revoked: &str,
                reason: &str,
                worker: WorkerId,
                project: ProjectId,
                digest_len: i32| {
        format!(
            "INSERT INTO tokens.tokens (id, worker_id, project_id, digest, install_gen, project_gen, issued_at, expires_at, renew_after, revoked_at, revoke_reason) \
             VALUES (gen_random_uuid(), '{worker}', '{project}', decode(repeat('ab', {digest_len}), 'hex'), 0, 0, \
                     timestamptz 'epoch' + {NOW} * interval '1 microsecond', {expires}, {renew}, {revoked}, {reason})"
        )
    };
    let ok_exp = format!(
        "timestamptz 'epoch' + {} * interval '1 microsecond'",
        NOW + DAY
    );
    let ok_ren = format!(
        "timestamptz 'epoch' + {} * interval '1 microsecond'",
        NOW + DAY / 2
    );
    let mut c = env.db.admin().await;
    // Sanity: the well-formed row is accepted by the same statement shape.
    sqlx::query(safe(base(&ok_exp, &ok_ren, "NULL", "NULL", s.w2, s.a, 32)))
        .execute(&mut c)
        .await
        .expect("a valid row inserts");
    for (what, sql) in [
        (
            "revoked_at without a reason",
            base(&ok_exp, &ok_ren, "now()", "NULL", s.w3, s.a, 32),
        ),
        (
            "a reason that is not a code",
            base(&ok_exp, &ok_ren, "now()", "'Lost it'", s.w3, s.a, 32),
        ),
        (
            "expires_at <= issued_at",
            base(
                &format!("timestamptz 'epoch' + {NOW} * interval '1 microsecond'"),
                &ok_ren,
                "NULL",
                "NULL",
                s.w3,
                s.a,
                32,
            ),
        ),
        (
            "renew_after > expires_at",
            base(
                &ok_exp,
                &format!(
                    "timestamptz 'epoch' + {} * interval '1 microsecond'",
                    NOW + 2 * DAY
                ),
                "NULL",
                "NULL",
                s.w3,
                s.a,
                32,
            ),
        ),
        (
            "a pair identity does not have",
            base(&ok_exp, &ok_ren, "NULL", "NULL", s.wb1, s.a, 32),
        ),
        (
            "a digest of 31 bytes",
            base(&ok_exp, &ok_ren, "NULL", "NULL", s.w3, s.a, 31),
        ),
        (
            "a digest of 33 bytes",
            base(&ok_exp, &ok_ren, "NULL", "NULL", s.w3, s.a, 33),
        ),
    ] {
        let e = sqlx::query(safe(sql))
            .execute(&mut c)
            .await
            .expect_err(what);
        assert!(["23514", "23503"].contains(&code(e).as_str()), "{what}");
    }
    // TOK-83: history is never deleted: the rows equal the tokens issued.
    assert_eq!(
        admin_i64(&env, "SELECT count(*) FROM tokens.tokens").await,
        Some(2)
    );
    // TOK-84: with the project unset or empty an ordinary SELECT returns nothing; a non-UUID errors.
    for (setting, expect_err) in [("", false), ("not-a-uuid", true)] {
        let mut d = env.db.admin().await;
        sqlx::raw_sql("SET ROLE cortex_m_tokens")
            .execute(&mut d)
            .await
            .expect("role");
        sqlx::query("SELECT set_config('cortex.project_id', $1, false)")
            .bind(setting)
            .execute(&mut d)
            .await
            .expect("setting");
        let r = sqlx::query("SELECT id FROM tokens.tokens")
            .fetch_all(&mut d)
            .await;
        match (r, expect_err) {
            (Ok(rows), false) => assert!(rows.is_empty(), "empty project returns zero rows"),
            (Err(_), true) => {}
            (other, _) => panic!("unexpected: {:?}", other.map(|r| r.len())),
        }
    }
    let mut d = env.db.admin().await;
    sqlx::raw_sql("SET ROLE cortex_m_tokens")
        .execute(&mut d)
        .await
        .expect("role");
    let unset = sqlx::query("SELECT id FROM tokens.tokens")
        .fetch_all(&mut d)
        .await
        .expect("unset");
    assert!(unset.is_empty(), "unset project returns zero rows");
}
