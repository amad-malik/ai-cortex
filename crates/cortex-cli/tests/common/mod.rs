//! Shared helpers for the process-level tests: the std-only fake server of cortex-client's tests, and a runner for the built
//! `cortex` binary with a CLEARED environment (only the variables a test names are set).
#![allow(dead_code)]

#[path = "../../../cortex-client/tests/support/mod.rs"]
pub mod support;

use std::process::Command;
use support::fake::{Fake, Reply};

pub struct Run {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub fn cortex(args: &[&str], env: &[(&str, &str)]) -> Run {
    let out = Command::new(env!("CARGO_BIN_EXE_cortex"))
        .args(args)
        .env_clear()
        .envs(env.iter().copied())
        .output()
        .expect("run the cortex binary");
    Run {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

pub const HEALTH_OK: &str = r#"{"state":"ready","version":"0.2.1","ready":true,"pieces":[{"name":"identity","enabled":true,"ready":true},{"name":"records","enabled":true,"ready":false}],"last_good_dump_at":null,"backup":"ok"}"#;

/// A fake whose default answer is `body` as a 200 JSON document.
pub fn health_fake(body: &str) -> Fake {
    let f = Fake::start();
    f.set_default(Reply::json(200, body));
    f
}
