//! F1-14 Platform schema grants. [RULED(L-7)] (S2 §5, §6; A7 item 1c.)
//! F1-15 Idempotency. [BASE] (S2 §5, K13 at the platform level.)
//! F1-16 The injected clock. [BASE] (S2 §12, S1 §13.) Its release-server bullets ("the setter
//! exists only with testkit's feature"; "a release build of cortex-server has no setter") need
//! `cortex-server`, so they are checked at F-1e; here, `platform.now()` follows the injected clock.
mod common;

use std::sync::Arc;
use std::time::Duration;

use cortex_db::idempotency::{begin as idem_begin, finish, require_key, sweep};
use cortex_db::migrate::{apply, platform_migrations};
use cortex_db::{DbPool, Idem, IdemError, IdemKey, ModuleTx, OwnerConn, StoredResponse, TxOptions};
use cortex_testkit::{TestClock, ctx, sqlstate, test_db, within};
use cortex_types::{ProjectId, Sha256, SystemClock, Timestamp, TokenId, WorkerId};
use sqlx::Row;

async fn platform_ready() -> &'static cortex_testkit::TestDb {
    let db = test_db().await;
    let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
    apply(&mut owner, &platform_migrations())
        .await
        .expect("platform migrations");
    db
}

#[tokio::test]
async fn f1_14_module_roles_insert_and_select_platform_objects_and_refs_only() {
    within(60, async {
        let db = platform_ready().await;
        let mut admin = db.admin().await;
        for role in ["cortex_m_test_a", "cortex_m_test_b"] {
            for table in ["platform.objects", "platform.refs"] {
                for (privilege, want) in [("INSERT", true), ("SELECT", true), ("UPDATE", false), ("DELETE", false)] {
                    let got: bool = sqlx::query("select has_table_privilege($1, $2, $3)")
                        .bind(role)
                        .bind(table)
                        .bind(privilege)
                        .fetch_one(&mut admin)
                        .await
                        .expect("privilege")
                        .get(0);
                    assert_eq!(got, want, "{role} {privilege} on {table}");
                }
            }
        }
        let owner: String = sqlx::query("select tableowner::text from pg_tables where schemaname = 'platform' and tablename = 'idempotency'")
            .fetch_one(&mut admin)
            .await
            .expect("idempotency table")
            .get(0);
        assert_eq!(owner, "cortex_owner");
        let pool = DbPool::connect(&db.db_config(common::MODULES, common::clock())).await.expect("connect");
        let mut tx = common::begin(&pool, "test_a", None).await;
        let e = sqlx::query("select count(*) from platform.idempotency").fetch_one(tx.executor().expect("executor")).await.expect_err("a module role reads idempotency rows");
        assert_eq!(sqlstate(&e).as_deref(), Some("42501"));
        tx.rollback().await.ok();
    })
    .await
}

fn digest(body: &str) -> Sha256 {
    let mut b = [0u8; 32];
    for (i, c) in body.bytes().enumerate() {
        b[i % 32] ^= c;
    }
    Sha256(b)
}

/// The test operation: one row in `test_a.t` (the effect), and its stored response.
async fn operation(
    pool: &DbPool,
    p: ProjectId,
    who: WorkerId,
    key: &str,
    body: &str,
    secret: bool,
) -> Result<(u16, StoredResponse), IdemError> {
    let mut tx = ModuleTx::begin(pool, ctx("test_a", Some(p)), TxOptions::default()).await?;
    let key = IdemKey::parse(key)?;
    require_key(true, Some(&key))?;
    match idem_begin(&mut tx, who, &key, "POST /test", digest(body)).await? {
        Idem::Replay { status, response } => {
            tx.rollback().await?;
            Ok((status, response))
        }
        Idem::Proceed(token) => {
            sqlx::query("insert into test_a.t (project_id, v) values ($1, $2)")
                .bind(common::uuid_of(p))
                .bind(format!("effect:{key:?}"))
                .execute(tx.executor()?)
                .await
                .map_err(|e| cortex_db::DbError::Other(e.to_string()))?;
            let response = if secret {
                StoredResponse::SecretNotStored {
                    token_id: TokenId::new(&SystemClock),
                }
            } else {
                StoredResponse::Body(serde_json::json!({ "body": body }))
            };
            finish(&mut tx, token, 201, response.clone()).await?;
            tx.commit().await?;
            Ok((201, response))
        }
    }
}

async fn effects(db: &cortex_testkit::TestDb, key: &str) -> i64 {
    let mut admin = db.admin().await;
    sqlx::query("select count(*) from test_a.t where v = $1")
        .bind(format!("effect:{:?}", IdemKey::parse(key).expect("key")))
        .fetch_one(&mut admin)
        .await
        .expect("effects")
        .get(0)
}

#[tokio::test]
async fn f1_15_idempotency_at_the_platform_level() {
    within(120, async {
        let db = platform_ready().await;
        let clock = Arc::new(TestClock::at(Timestamp::from_unix_micros(
            1_700_000_000_000_000,
        )));
        let pool = Arc::new(
            DbPool::connect(&db.db_config(common::MODULES, clock.clone()))
                .await
                .expect("connect"),
        );
        let (w1, w2) = (WorkerId::new(&SystemClock), WorkerId::new(&SystemClock));
        // One project per logical operation and its retries (mike's F1B-R0-02).
        let (p, other) = (common::project(), common::project());

        // A dropped response, retried with the same key and body: the same body, one effect.
        let first = operation(&pool, p, w1, REPLAY, "a", false)
            .await
            .expect("first");
        let again = operation(&pool, p, w1, REPLAY, "a", false)
            .await
            .expect("retry");
        assert_eq!(first, again, "the retry did not replay the stored response");
        assert_eq!(effects(db, REPLAY).await, 1);

        // A different body with the same key: 422.
        assert_eq!(
            operation(&pool, p, w1, REPLAY, "b", false).await,
            Err(IdemError::KeyReused)
        );

        // The same principal, key, route and body from ANOTHER project: the row is bound to its
        // project, so this is 422 too, never a replay across projects, and nothing runs.
        assert_eq!(
            operation(&pool, other, w1, REPLAY, "a", false).await,
            Err(IdemError::KeyReused),
            "a stored response was replayed to another project"
        );
        assert_eq!(effects(db, REPLAY).await, 1, "the other project's call ran");

        // Ten concurrent requests with one key, one project: one effect.
        let mut set = tokio::task::JoinSet::new();
        for _ in 0..10 {
            let pool = Arc::clone(&pool);
            set.spawn(async move { operation(&pool, p, w1, TEN, "x", false).await });
        }
        while let Some(r) = set.join_next().await {
            r.expect("task")
                .expect("each concurrent request succeeds or replays");
        }
        assert_eq!(
            effects(db, TEN).await,
            1,
            "ten concurrent requests made more than one effect"
        );

        // No key on a mutating operation: 428; a non-mutating one needs none.
        assert_eq!(require_key(true, None), Err(IdemError::KeyRequired));
        assert_eq!(require_key(false, None), Ok(()));

        // The same key from two principals: two effects.
        operation(&pool, p, w2, REPLAY, "a", false)
            .await
            .expect("another principal");
        assert_eq!(effects(db, REPLAY).await, 2);

        // A secret-bearing response stores only the marker; its retry gets 409.
        operation(&pool, p, w1, SECRET, "s", true)
            .await
            .expect("secret");
        assert!(matches!(
            operation(&pool, p, w1, SECRET, "s", true).await,
            Err(IdemError::AlreadyIssued { .. })
        ));

        // Rows older than 24 h are deleted under the injected clock.
        clock.set(Timestamp::from_unix_micros(
            1_700_000_000_000_000 + 25 * 3600 * 1_000_000,
        ));
        let mut tx = ModuleTx::begin(&pool, ctx("test_a", None), TxOptions::default())
            .await
            .expect("begin");
        let deleted = sweep(&mut tx, Duration::from_secs(24 * 3600))
            .await
            .expect("sweep");
        tx.commit().await.expect("commit");
        assert!(deleted >= 4, "the sweep deleted {deleted} rows");
    })
    .await
}

/// Valid keys (8 to 128 printable ASCII; frozen S2 §5).
const REPLAY: &str = "key-replay-0001";
const TEN: &str = "key-ten-concurrent";
const SECRET: &str = "key-secret-0001";

/// mike's F1B-R0-01: the frozen grammar at its boundaries. No database needed.
#[test]
fn f1_15_the_key_grammar_is_8_to_128_printable_ascii() {
    let ok = |s: &str| IdemKey::parse(s).is_ok();
    assert!(!ok(&"k".repeat(7)), "7 characters accepted");
    assert!(ok(&"k".repeat(8)), "8 characters refused");
    assert!(ok(&"k".repeat(128)), "128 characters refused");
    assert!(!ok(&"k".repeat(129)), "129 characters accepted");
    assert!(ok("with space"), "a space is printable ASCII");
    assert!(ok("~!@#$%^&*()_+{}|:<>?"), "printable punctuation refused");
    for bad in [
        "",
        "k-ten",
        "tab\there!",
        "del\u{7f}xxxxx",
        "nul\u{0}xxxxx",
        "non-ascii-é-key",
    ] {
        assert_eq!(
            IdemKey::parse(bad),
            Err(IdemError::BadKey),
            "{bad:?} was not refused"
        );
    }
}

#[tokio::test]
async fn f1_16_platform_now_follows_the_injected_clock() {
    within(60, async {
        let db = platform_ready().await;
        let at = Timestamp::from_unix_micros(1_800_000_000_123_456);
        let pool = DbPool::connect(&db.db_config(common::MODULES, Arc::new(TestClock::at(at))))
            .await
            .expect("connect");
        let mut tx = common::begin(&pool, "test_a", None).await;
        let micros: i64 =
            sqlx::query("select (extract(epoch from platform.now()) * 1000000)::bigint")
                .fetch_one(tx.executor().expect("executor"))
                .await
                .expect("platform.now()")
                .get(0);
        tx.rollback().await.ok();
        assert_eq!(
            micros,
            at.unix_micros(),
            "platform.now() is not the injected clock's instant"
        );
    })
    .await
}
