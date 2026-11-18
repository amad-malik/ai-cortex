//! The bootstrap decision (contract r0.3; W1-REISSUE, W1-3, W1-18) over a fake store. DB-free. RED first.
//!
//! The atomicity of project, lead, token and audit row is the real store's (a host transaction); here the fake records the
//! calls so the ORDER (find, precheck the file, mint, write the file, commit; rollback on any failure) is proved.

use async_trait::async_trait;
use cortex_server::bootstrap::{BootstrapStore, Found, Outcome, REFUSED_LINE, StoreError, run};
use cortex_server::cli::{BootstrapArgs, EXIT_FAILURE, EXIT_OK, EXIT_REFUSED};
use cortex_types::Secret;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

const PROJECT: &str = "zz-project-9";
const LEAD: &str = "zz-lead-9";
const TOKEN: &str = "ctx2_TOPSECRET-TOKEN-VALUE";

#[derive(Default)]
struct Fake {
    found: Option<Result<Found, StoreError>>,
    mint: Option<Result<Secret, StoreError>>,
    commit_fails: bool,
    /// On commit: make this directory read-only first (so the token file cannot be unlinked), then fail the commit.
    lock_dir_on_commit: Option<PathBuf>,
    /// On commit: remove this file first (someone else already removed the token file), then fail the commit.
    remove_on_commit: Option<PathBuf>,
    finds: u32,
    mints: u32,
    commits: u32,
    rollbacks: u32,
}

#[async_trait]
impl BootstrapStore for Fake {
    async fn find(&mut self, _p: &str, _l: &str) -> Result<Found, StoreError> {
        self.finds += 1;
        self.found.take().unwrap_or(Ok(Found::Nothing))
    }
    async fn create_or_reissue(&mut self, _p: &str, _l: &str) -> Result<Secret, StoreError> {
        self.mints += 1;
        self.mint.take().unwrap_or_else(|| Ok(Secret::new(TOKEN)))
    }
    async fn commit(&mut self) -> Result<(), StoreError> {
        self.commits += 1;
        if let Some(f) = &self.remove_on_commit {
            fs::remove_file(f).expect("remove the token file");
        }
        if let Some(d) = &self.lock_dir_on_commit {
            fs::set_permissions(d, fs::Permissions::from_mode(0o500)).expect("lock the dir");
        }
        if self.commit_fails {
            Err(StoreError("commit".into()))
        } else {
            Ok(())
        }
    }
    async fn rollback(&mut self) {
        self.rollbacks += 1;
    }
}

fn args(name: &str, require: bool) -> (BootstrapArgs, PathBuf) {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("bootstrap-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("dir");
    let tf = d.join("bootstrap.token");
    (
        BootstrapArgs {
            config: PathBuf::from("/x"),
            project: PROJECT.into(),
            lead: LEAD.into(),
            token_file: tf.clone(),
            require_existing: require,
        },
        tf,
    )
}

fn no_names_or_token(o: &Outcome) {
    let all = format!("{o:?}");
    for bad in [PROJECT, LEAD, "TOPSECRET"] {
        assert!(!all.contains(bad), "the outcome leaks {bad}: {all}");
    }
}

#[tokio::test]
async fn bs_01_require_existing_refuses_every_missing_case_with_exit_3_and_creates_nothing() {
    for (i, found) in [
        Found::Nothing,
        Found::ProjectOnly,
        Found::LeadNotProjectLead,
    ]
    .into_iter()
    .enumerate()
    {
        let (a, tf) = args(&format!("refuse{i}"), true);
        let mut f = Fake {
            found: Some(Ok(found)),
            ..Default::default()
        };
        let o = run(&mut f, &a).await;
        assert_eq!(o.code, EXIT_REFUSED, "{found:?}");
        assert_eq!(
            o.stderr.as_deref(),
            Some(REFUSED_LINE),
            "{found:?}: ONE fixed line"
        );
        assert_eq!(f.mints, 0, "{found:?}: nothing may be created or minted");
        assert!(!tf.exists(), "{found:?}: no token file");
        no_names_or_token(&o);
    }
}

#[tokio::test]
async fn bs_02_the_refusal_line_is_fixed_text_without_a_name_or_an_id() {
    assert!(REFUSED_LINE.starts_with("bootstrap refused: --require-existing"));
    assert!(
        !REFUSED_LINE.contains(PROJECT)
            && !REFUSED_LINE.contains(LEAD)
            && !REFUSED_LINE.contains('\n')
    );
}

#[tokio::test]
async fn bs_03_require_existing_with_the_project_and_its_lead_reissues() {
    let (a, tf) = args("reissue", true);
    let mut f = Fake {
        found: Some(Ok(Found::ProjectAndLead)),
        ..Default::default()
    };
    let o = run(&mut f, &a).await;
    assert_eq!((o.code, o.stderr.clone()), (EXIT_OK, None));
    assert_eq!((f.mints, f.commits, f.rollbacks), (1, 1, 0));
    assert_eq!(fs::read(&tf).expect("token file"), TOKEN.as_bytes());
    assert_eq!(
        fs::metadata(&tf).expect("meta").permissions().mode() & 0o777,
        0o600
    );
    no_names_or_token(&o);
}

#[tokio::test]
async fn bs_04_without_the_flag_the_first_run_creates() {
    let (a, tf) = args("first", false);
    let mut f = Fake {
        found: Some(Ok(Found::Nothing)),
        ..Default::default()
    };
    let o = run(&mut f, &a).await;
    assert_eq!(o.code, EXIT_OK);
    assert_eq!((f.mints, f.commits), (1, 1));
    assert_eq!(fs::read(&tf).expect("token file"), TOKEN.as_bytes());
}

#[tokio::test]
async fn bs_05_a_different_lead_for_an_existing_project_is_a_failure_not_a_refusal() {
    let (a, tf) = args("conflict", false);
    let mut f = Fake {
        found: Some(Ok(Found::LeadNotProjectLead)),
        mint: Some(Err(StoreError("conflict".into()))),
        ..Default::default()
    };
    let o = run(&mut f, &a).await;
    assert_eq!(o.code, EXIT_FAILURE);
    assert_ne!(o.code, EXIT_REFUSED);
    assert_eq!((f.commits, f.rollbacks), (0, 1));
    assert!(!tf.exists());
    no_names_or_token(&o);
}

#[tokio::test]
async fn bs_06_a_store_error_on_find_fails_and_creates_nothing() {
    let (a, tf) = args("finderr", true);
    let mut f = Fake {
        found: Some(Err(StoreError("db".into()))),
        ..Default::default()
    };
    let o = run(&mut f, &a).await;
    assert_eq!(o.code, EXIT_FAILURE);
    assert_eq!(f.mints, 0);
    assert!(!tf.exists());
}

#[tokio::test]
async fn bs_07_an_existing_token_file_is_checked_before_minting() {
    // Minting re-issues the lead's token and supersedes the old one: it must never happen when the file cannot be delivered.
    let (a, tf) = args("fileexists", false);
    fs::write(&tf, b"OLD").expect("seed");
    let mut f = Fake {
        found: Some(Ok(Found::ProjectAndLead)),
        ..Default::default()
    };
    let o = run(&mut f, &a).await;
    assert_eq!(o.code, EXIT_FAILURE);
    assert_eq!(
        f.mints, 0,
        "no token may be minted when the destination exists"
    );
    assert_eq!(fs::read(&tf).expect("read"), b"OLD");
    assert_eq!(f.rollbacks, 1);
}

#[tokio::test]
async fn bs_08_a_failed_file_write_after_the_mint_rolls_back_and_leaves_nothing() {
    let (mut a, tf) = args("nodir", false);
    a.token_file = tf
        .parent()
        .expect("parent")
        .join("no-such-dir")
        .join("bootstrap.token");
    let mut f = Fake {
        found: Some(Ok(Found::Nothing)),
        ..Default::default()
    };
    let o = run(&mut f, &a).await;
    assert_eq!(o.code, EXIT_FAILURE);
    assert_eq!(
        (f.mints, f.commits, f.rollbacks),
        (1, 0, 1),
        "minted, then the write failed: the transaction rolls back"
    );
    assert!(!a.token_file.exists());
    no_names_or_token(&o);
}

#[tokio::test]
async fn bs_09_a_failed_commit_removes_the_file_so_no_dead_token_is_left() {
    let (a, tf) = args("commitfail", false);
    let mut f = Fake {
        found: Some(Ok(Found::Nothing)),
        commit_fails: true,
        ..Default::default()
    };
    let o = run(&mut f, &a).await;
    assert_eq!(o.code, EXIT_FAILURE);
    assert_eq!(f.commits, 1);
    assert!(
        !tf.exists(),
        "a token file for a rolled-back token must not remain"
    );
    // vera SERVER-I2-V2: the commit outcome is unknown, and the line says so and says what to do.
    let line = o.stderr.clone().expect("a line");
    assert!(line.contains("unknown") && line.contains("again"), "{line}");
    // vera's Low: a failed commit is TERMINAL. The outcome of the commit is unknown, so no rollback is attempted and nothing is retried.
    assert_eq!((f.commits, f.rollbacks, f.mints), (1, 0, 1));
    no_names_or_token(&o);
}

#[tokio::test]
async fn bs_10_a_token_file_that_cannot_be_removed_after_a_failed_commit_is_said_so() {
    // vera's Low: an unlink failure was ignored. The file then holds a token that is NOT valid; the operator must be told.
    let (a, tf) = args("unlinkfail", false);
    let dir = tf.parent().expect("parent").to_path_buf();
    let mut f = Fake {
        found: Some(Ok(Found::Nothing)),
        commit_fails: true,
        lock_dir_on_commit: Some(dir.clone()),
        ..Default::default()
    };
    let o = run(&mut f, &a).await;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).expect("unlock the dir");
    assert_eq!(o.code, EXIT_FAILURE);
    let line = o.stderr.clone().expect("a line");
    // vera SERVER-I2-V2: the commit outcome is UNKNOWN (an acknowledgement can be lost after the token was committed), so the leftover
    // file must never be called invalid; it may hold a live token.
    assert!(
        line.contains("could not be removed")
            && line.contains("unknown")
            && line.contains("may hold a valid token")
            && !line.contains("NOT valid"),
        "{line}"
    );
    assert!(!line.contains('\n'));
    assert!(
        tf.exists(),
        "the file is really still there (the cell is not vacuous)"
    );
    no_names_or_token(&o);
}

#[tokio::test]
async fn bs_11_a_precheck_error_other_than_not_found_fails_before_the_mint() {
    // vera's Low: the precheck treated every error as "absent". The parent is a regular file, so the check says ENOTDIR, not NotFound.
    let (mut a, tf) = args("enotdir", false);
    let file_as_dir = tf.parent().expect("parent").join("plain-file");
    fs::write(&file_as_dir, b"x").expect("seed");
    a.token_file = file_as_dir.join("bootstrap.token");
    let mut f = Fake {
        found: Some(Ok(Found::Nothing)),
        ..Default::default()
    };
    let o = run(&mut f, &a).await;
    assert_eq!(o.code, EXIT_FAILURE);
    assert_eq!(
        f.mints, 0,
        "no token may be minted when the destination cannot even be checked"
    );
    assert_eq!(f.rollbacks, 1);
    assert!(
        o.stderr
            .as_deref()
            .is_some_and(|s| s.contains("could not check the token file")),
        "{o:?}"
    );
    no_names_or_token(&o);
}

#[tokio::test]
async fn bs_12_a_token_file_that_is_already_gone_when_the_commit_fails_is_not_reported_as_stuck() {
    // hale's survivor BB: the NotFound arm of the unlink after a failed commit. The file is gone (not "stuck"), so the line is the plain
    // "outcome is unknown; run bootstrap again", never "could not be removed".
    let (a, tf) = args("gone", false);
    let mut f = Fake {
        found: Some(Ok(Found::Nothing)),
        commit_fails: true,
        remove_on_commit: Some(tf.clone()),
        ..Default::default()
    };
    let o = run(&mut f, &a).await;
    assert_eq!(o.code, EXIT_FAILURE);
    let line = o.stderr.clone().expect("a line");
    assert!(line.contains("unknown") && line.contains("again"), "{line}");
    assert!(!line.contains("could not be removed"), "{line}");
    assert!(!tf.exists());
    no_names_or_token(&o);
}

/// vera SERVER-I3-V1: the minted token must travel in a type that is zeroised on drop, so that a failed audit write or a cancelled
/// future cannot leave the plaintext in freed memory. The type is `cortex_types::Secret` (zeroise-on-drop is that type's own F1-03
/// property); this cell ties the store's return type to it at compile time. A `Vec<u8>` or a `String` here does not compile.
#[allow(dead_code)]
async fn hand_over<S: BootstrapStore>(store: &mut S) -> Result<Secret, StoreError> {
    store.create_or_reissue(PROJECT, LEAD).await
}

#[tokio::test]
async fn bs_13_the_minted_token_is_handed_over_as_a_secret_and_reaches_the_file_exactly() {
    let mut f = Fake::default();
    let token = hand_over(&mut f).await.expect("mint");
    assert_eq!(token.expose(), TOKEN);
    let (a, tf) = args("secret-type", false);
    let mut f = Fake::default();
    let o = run(&mut f, &a).await;
    assert_eq!(o.code, EXIT_OK);
    assert_eq!(fs::read(&tf).expect("token file"), TOKEN.as_bytes());
}
