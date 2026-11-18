//! Shared test helpers (no tempfile crate is in the lock: directories live under CARGO_TARGET_TMPDIR).
#![allow(dead_code)]

pub mod fake;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static N: AtomicU64 = AtomicU64::new(0);

/// A fresh directory, mode 0700, unique per call.
pub fn dir(label: &str) -> PathBuf {
    let n = N.fetch_add(1, Ordering::SeqCst);
    let p = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("{label}-{}-{n}", std::process::id()));
    fs::create_dir_all(&p).expect("create test dir");
    set_mode(&p, 0o700);
    p
}

pub fn set_mode(p: &Path, mode: u32) {
    fs::set_permissions(p, fs::Permissions::from_mode(mode)).expect("chmod");
}

/// Write `bytes` to `dir/name` with `mode`.
pub fn write(dir: &Path, name: &str, bytes: &[u8], mode: u32) -> PathBuf {
    let p = dir.join(name);
    // An earlier call may have left it read-only (0400) or unreadable (0000): replace it, do not write through it.
    let _ = fs::remove_file(&p);
    fs::write(&p, bytes).expect("write test file");
    set_mode(&p, mode);
    p
}

use cortex_client::{Client, ClientConfig, Sleeper, Token};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const TOKEN: &str = "wtok-4fa9c2d81b7e6035-CANARY";

/// A sleeper that records the waits and never sleeps.
#[derive(Default)]
pub struct RecSleeper(pub Mutex<Vec<Duration>>);

impl Sleeper for RecSleeper {
    fn sleep(&self, wait: Duration) {
        self.0.lock().unwrap().push(wait);
    }
}

impl RecSleeper {
    pub fn waits(&self) -> Vec<Duration> {
        self.0.lock().unwrap().clone()
    }
}

/// A client for `fake` that holds `TOKEN` and a recording sleeper.
pub fn client(fake: &fake::Fake) -> (Client, Arc<RecSleeper>) {
    client_with(fake, |_| {})
}

pub fn client_with(
    fake: &fake::Fake,
    tweak: impl FnOnce(&mut ClientConfig),
) -> (Client, Arc<RecSleeper>) {
    let mut cfg = ClientConfig::new(fake.endpoint(), Some(Token::parse(TOKEN, "test").unwrap()));
    tweak(&mut cfg);
    let sleeper = Arc::new(RecSleeper::default());
    (Client::with_sleeper(cfg, sleeper.clone()), sleeper)
}

pub const UUID_A: &str = "0198d2c0-7000-7000-8000-00000000000a";

pub fn get(path: &str) -> cortex_client::Request {
    cortex_client::Request::new(
        cortex_client::Method::Get,
        path,
        &[],
        cortex_client::Mutates::No,
    )
    .unwrap()
}

pub fn post(path: &str, mutates: cortex_client::Mutates) -> cortex_client::Request {
    cortex_client::Request::new(cortex_client::Method::Post, path, &[], mutates).unwrap()
}

/// The S3 twin the repository carries (`tools/contract/cortex-v2-skeleton.openapi.r2.7.json`, tate's lane, the r2.7 pin
/// `1d2c4b2f`), or the file named by env CORTEX_S3_TWIN: drift between the client and the contract shows here, and the tests run
/// anywhere the repository is checked out.
pub fn twin() -> serde_json::Value {
    let path = std::env::var("CORTEX_S3_TWIN")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tools/contract/cortex-v2-skeleton.openapi.r2.7.json")
        });
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read the S3 twin at {}: {e}", path.display()));
    serde_json::from_str(&text).expect("the twin is JSON")
}
