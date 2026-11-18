//! mike's F1B-R0-06: cluster role setup is serialised, so concurrent first databases on a fresh
//! cluster cannot race the role check-then-create. Eight isolated databases are created at once.
//! **At GREEN this binary also runs alone, on a fresh supervisor cluster** (no other binary has
//! created the roles yet):
//! `cv2-admit timeout --kill-after=120 780 crates/cortex-testkit/supervise.sh cargo test -p cortex-db --test testkit_parallel_setup`.
//! It needs no `cortex-db` behaviour, so it can pass at RED; it is a guard, not a RED.
use cortex_testkit::{isolated_db, within};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn testkit_concurrent_first_databases_all_set_up() {
    within(120, async {
        let mut set = tokio::task::JoinSet::new();
        for n in 0..8 {
            set.spawn(async move { isolated_db(&format!("par{n}")).await.database });
        }
        let mut names = Vec::new();
        while let Some(r) = set.join_next().await {
            names.push(r.expect("a concurrent setup panicked"));
        }
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 8, "eight distinct databases");
    })
    .await
}
