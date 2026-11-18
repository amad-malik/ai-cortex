//! The fixed environment of every child (R2): `env_clear()`, then `PATH`, `HOME` and `XDG_RUNTIME_DIR` copied from the
//! parent (each only when set), and `LANG=C`. Nothing else: no `CORTEX_*`, no `*_PROXY`, no `COMPOSE_*`, no `PODMAN_*`,
//! no `CONTAINER_HOST`, no `DOCKER_HOST`. ONE function, used by [`super::run::run_spec`], which every wrapper goes through.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::time::Duration;

/// The names copied from the parent.
const COPIED: [&str; 3] = ["PATH", "HOME", "XDG_RUNTIME_DIR"];
/// The names an extra variable may never supply, whether or not the parent has them: the copied ones and `LANG`.
const RESERVED: [&str; 4] = ["PATH", "HOME", "XDG_RUNTIME_DIR", "LANG"];

/// The environment a child gets: the fixed names, and the `extra` variables the caller was given by its CONSTRUCTOR
/// (only the compose child has any: [`ComposeEnv`]). ONE function for all three wrappers. An extra that has the name
/// of a fixed one is dropped, whether or not the parent has that name, so an extra can never supply or replace `PATH`,
/// `HOME`, `XDG_RUNTIME_DIR` or `LANG`.
pub fn child_env(extra: &[(OsString, OsString)]) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = COPIED
        .iter()
        .filter_map(|k| std::env::var_os(k).map(|v| (OsString::from(*k), v)))
        .collect();
    env.push((OsString::from("LANG"), OsString::from("C")));
    for (k, v) in extra {
        if !RESERVED
            .iter()
            .any(|name| k.as_os_str() == OsStr::new(name))
        {
            env.push((k.clone(), v.clone()));
        }
    }
    env
}

/// A value given to [`ComposeEnv`] that may not become an environment variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadComposeEnv;

/// The three variables the release's `compose.yml` needs and `stack.env` does not hold (they are `install.sh`'s;
/// hue's finding of 6 Nov, cox's amendment of R2 at 18:58Z). They are built from the install's home and local port
/// given to the CONSTRUCTOR, never read from the parent's environment, and only the compose child gets them: the other
/// programs get the four fixed names and nothing more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeEnv {
    home: PathBuf,
    port: u16,
    /// The verified absolute compose provider (R-6; cox W1-68, W1-91): when set, the compose child is told to use THIS one, by `PODMAN_COMPOSE_PROVIDER`.
    provider: Option<PathBuf>,
}

impl ComposeEnv {
    pub fn new(home: PathBuf, port: u16) -> ComposeEnv {
        ComposeEnv {
            home,
            port,
            provider: None,
        }
    }

    /// Names the verified absolute provider the compose child must use (`PODMAN_COMPOSE_PROVIDER`; see [`ComposeEnv::vars`]).
    pub fn with_provider(mut self, provider: PathBuf) -> ComposeEnv {
        self.provider = Some(provider);
        self
    }

    /// `CORTEX_CONFIG=<home>/cortex.toml`, `CORTEX_DUMP_STATUS=<home>/dump-status`, `CORTEX_PORT=<port>`, after the
    /// checks: the home is an absolute path without NUL, newline or carriage return, and the port is 1 to 65535; and, when a provider was given
    /// (R-6), LAST, `PODMAN_COMPOSE_PROVIDER=<provider>` after its own check: an absolute path without any control character (podman is told its
    /// provider by the environment; the argument vector does not change).
    pub fn vars(&self) -> Result<Vec<(OsString, OsString)>, BadComposeEnv> {
        use std::os::unix::ffi::OsStrExt;
        let bytes = self.home.as_os_str().as_bytes();
        if !self.home.is_absolute()
            || bytes.iter().any(|b| matches!(b, 0 | b'\n' | b'\r'))
            || self.port == 0
        {
            return Err(BadComposeEnv);
        }
        let mut vars = vec![
            (
                OsString::from("CORTEX_CONFIG"),
                self.home.join("cortex.toml").into_os_string(),
            ),
            (
                OsString::from("CORTEX_DUMP_STATUS"),
                self.home.join("dump-status").into_os_string(),
            ),
            (
                OsString::from("CORTEX_PORT"),
                OsString::from(self.port.to_string()),
            ),
        ];
        if let Some(provider) = &self.provider {
            if !crate::provider::compose_env_path_ok(provider) {
                return Err(BadComposeEnv);
            }
            vars.push((
                OsString::from("PODMAN_COMPOSE_PROVIDER"),
                provider.clone().into_os_string(),
            ));
        }
        Ok(vars)
    }
}

/// The largest deadline `db-roles.sh --deadline` takes: digits only, no leading zero, at most 11 (hue's design, cox W1-55).
pub const MAX_ROLES_DEADLINE: u64 = 99_999_999_999;

/// The engine's ABSOLUTE deadline for the roles script, in epoch seconds: the wall clock at the launch (`now_secs`) plus
/// the port's `timeout`, each in whole seconds (rounded down, so it is never later than the real deadline). `None` when it
/// would not fit the script's argument: over 11 digits, zero (the script takes no leading zero, and `0` is a deadline in
/// 1970), or the sum overflows.
pub fn roles_deadline(now_secs: u64, timeout: Duration) -> Option<u64> {
    now_secs
        .checked_add(timeout.as_secs())
        .filter(|e| (1..=MAX_ROLES_DEADLINE).contains(e))
}
