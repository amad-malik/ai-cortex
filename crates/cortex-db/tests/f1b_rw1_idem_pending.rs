//! F-1b rework 1, kai's small item (i), IDEM-PENDING-TEST (vera's retained Low): the same key
//! begun twice in ONE transaction, before `finish`, is `KeyReused` (422); nothing runs twice. This
//! pins the behaviour a1d1171 already has (a regression test, not a RED): kai confirms the
//! contract wording at R3.
mod common;

use cortex_db::idempotency::begin as idem_begin;
use cortex_db::migrate::{apply, platform_migrations};
use cortex_db::{DbPool, Idem, IdemError, IdemKey, OwnerConn};
use cortex_testkit::{test_db, within};
use cortex_types::{Sha256, SystemClock, WorkerId};

#[tokio::test]
async fn rw1_the_same_key_twice_before_finish_in_one_transaction_is_key_reused() {
    within(60, async {
        let db = test_db().await;
        let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
        apply(&mut owner, &platform_migrations())
            .await
            .expect("platform migrations");
        let pool = DbPool::connect(&db.db_config(common::MODULES, common::clock()))
            .await
            .expect("connect");
        let mut tx = common::begin(&pool, "test_a", Some(common::project())).await;
        let who = WorkerId::new(&SystemClock);
        let key = IdemKey::parse("rw1-pending-key").expect("key");
        let body = Sha256([7; 32]);
        let first = idem_begin(&mut tx, who, &key, "POST /rw1", body)
            .await
            .expect("the first begin");
        assert!(matches!(first, Idem::Proceed(_)), "{first:?}");
        let second = idem_begin(&mut tx, who, &key, "POST /rw1", body).await;
        assert_eq!(
            second.err(),
            Some(IdemError::KeyReused),
            "a reserved, unfinished key in the same transaction"
        );
        tx.rollback().await.ok();
    })
    .await
}
