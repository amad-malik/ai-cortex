//! `Containers` over `podman` (brief r1 section 3). Each call starts ONE `podman` with a fixed argument vector (the
//! closed list of section 3; cells `rc_*`). The release directory comes with every call.

use super::env::{ComposeEnv, roles_deadline};
use super::name::{container_name, project_name};
use super::run::{Outcome, Spec, failed, last_line, run, run_failure, run_spec, stderr_failure};
use crate::manifest::Digest;
use crate::ports::{Containers, PortError, Service, ServiceState};
use cortex_types::ModuleId;
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, Instant};

/// The fixed program name used in the failure lines.
const PODMAN: &str = "podman";
/// The roles script, at the top of the release (cox 14:30Z, Q-7).
const ROLES_SCRIPT: &str = "db-roles.sh";
/// The only project the roles script reaches: `db-roles.sh` runs `podman exec` in `cv2-s5-db` unless
/// `CORTEX_DB_CONTAINER` names another `cv2-s5-*` container, and this wrapper never sets it (R2: four names). A release of
/// any other project would have its roles applied to the wrong database, so `run_roles` refuses it (mike, 6 Nov; cox).
pub(crate) const ROLES_PROJECT: &str = "cv2-s5";
/// The fixed line for a `podman` answer of which only a part was read (cut at the cap, or a stream still open).
const INCOMPLETE: &str = "podman's answer is not complete";

/// W1-64: no script output proves a settled refusal until the executed bytes
/// are bound to the verified release on every path, without a replacement race.
pub const ROLES_SETTLED_REFUSALS: &[(i32, &str, &str)] = &[];

/// The deadlines of the calls that have no timeout argument (the engine's own polling and bounds sit above them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deadlines {
    /// `podman load` and the `image inspect` after it, each.
    pub load: Duration,
    /// `podman compose ... up -d` (it returns when the containers are started; the migrate wait is the engine's).
    pub up: Duration,
    /// `podman inspect` (one reading).
    pub inspect: Duration,
}

impl Default for Deadlines {
    fn default() -> Deadlines {
        Deadlines {
            load: Duration::from_secs(900),
            up: Duration::from_secs(600),
            inspect: Duration::from_secs(30),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RealContainers {
    podman: OsString,
    deadlines: Deadlines,
    compose_env: ComposeEnv,
    /// R-6c (cox W1-105): set only by [`RealContainers::for_check_with`]. A check adapter changes nothing: `compose_up` is refused before anything is
    /// started, and (R-6d, cox row 49621ff3) so are `run_roles` and `load_image`, so that "a check changes nothing" does not rest on the engine's flow alone.
    check_only: bool,
}

impl RealContainers {
    /// The real one: `podman` from PATH, the default deadlines, the install's home and local port, which the release's
    /// `compose.yml` needs (see [`ComposeEnv`]), and the verified absolute compose provider the entry read once (R-6).
    pub fn new(
        home: std::path::PathBuf,
        port: u16,
        compose_provider: std::path::PathBuf,
    ) -> RealContainers {
        RealContainers::with(
            OsString::from(PODMAN),
            Deadlines::default(),
            ComposeEnv::new(home, port).with_provider(compose_provider),
        )
    }

    /// The real one for a CHECK ONLY (R-6b): no compose provider, because a check
    /// starts no container (the engine's check never reaches `restart`, and so never calls `compose_up`). Named so that it can be found and
    /// cannot be mistaken for [`RealContainers::new`], which REQUIRES the provider the entry read. R-6c (cox W1-105): and an adapter made here REFUSES
    /// `compose_up`, `run_roles` and `load_image` itself (see `check_only`), so that "a check starts no container" is a property of the adapter and not only
    /// of the engine's flow.
    pub fn for_check(home: std::path::PathBuf, port: u16) -> RealContainers {
        RealContainers::for_check_with(
            OsString::from(PODMAN),
            Deadlines::default(),
            ComposeEnv::new(home, port),
        )
    }

    /// [`RealContainers::for_check`] with the program, the deadlines and the compose variables given (R-6c): for the cells, so that a check adapter can
    /// be made with a fake `podman` by absolute path and a test can never reach the real one. `for_check` is this with `podman` from PATH.
    pub fn for_check_with(
        podman: impl Into<OsString>,
        deadlines: Deadlines,
        compose_env: ComposeEnv,
    ) -> RealContainers {
        RealContainers {
            check_only: true,
            ..RealContainers::with(podman, deadlines, compose_env)
        }
    }

    /// The program, the deadlines and the compose variables, for the cells (a fake `podman` by absolute path).
    pub fn with(
        podman: impl Into<OsString>,
        deadlines: Deadlines,
        compose_env: ComposeEnv,
    ) -> RealContainers {
        RealContainers {
            podman: podman.into(),
            deadlines,
            compose_env,
            check_only: false,
        }
    }

    fn podman(&self, args: Vec<OsString>, deadline: Duration) -> Result<Outcome, PortError> {
        run(&self.podman, &args, deadline).map_err(|e| run_failure(e, PODMAN))
    }
}

fn os(s: &str) -> OsString {
    OsString::from(s)
}

/// `^[A-Za-z0-9][A-Za-z0-9._:/@-]{0,255}$`
fn image_ref_ok(r: &str) -> bool {
    let b = r.as_bytes();
    !b.is_empty()
        && b.len() <= 256
        && b[0].is_ascii_alphanumeric()
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || b"._:/@-".contains(c))
}

/// `sha256:` and exactly 64 lowercase hex digits.
fn parse_digest(text: &str) -> Option<Digest> {
    let hex = text.strip_prefix("sha256:")?;
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, pair) in hex.as_bytes().chunks(2).enumerate() {
        let nib = |c: u8| match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            _ => None,
        };
        out[i] = nib(pair[0])? << 4 | nib(pair[1])?;
    }
    Some(Digest(out))
}

/// The reference of the ONE image a `podman load` reported: exactly one stdout line starts with `Loaded image`, it is the
/// last non-empty one, and what follows its prefix is a plain reference.
fn loaded_ref(stdout: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(stdout);
    let lines: Vec<&str> = text
        .split('\n')
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let mut loaded = lines.iter().filter(|l| l.starts_with("Loaded image"));
    let line = loaded.next()?;
    if loaded.next().is_some() || lines.last() != Some(line) {
        return None;
    }
    let r = line
        .strip_prefix("Loaded image: ")
        .or_else(|| line.strip_prefix("Loaded image(s): "))?;
    image_ref_ok(r).then(|| r.to_string())
}

impl Containers for RealContainers {
    fn load_image(&self, archive: &Path) -> Result<Digest, PortError> {
        // R-6d (cox row 49621ff3): `podman load` changes the image store; a check adapter refuses it before `podman` is started
        if self.check_only {
            return Err(failed(
                "a check loads no image; the load is refused by the check adapter",
            ));
        }
        let o = self.podman(
            vec![os("load"), os("--input"), archive.as_os_str().to_owned()],
            self.deadlines.load,
        )?;
        if o.code != Some(0) {
            return Err(stderr_failure(&o));
        }
        // One image means ALL the output says one image: a first MiB that does is not enough when the child said more.
        if o.stdout_cut {
            return Err(failed(INCOMPLETE));
        }
        let image = loaded_ref(&o.stdout)
            .ok_or_else(|| failed("the archive did not load exactly one image"))?;
        let o = self.podman(
            vec![
                os("image"),
                os("inspect"),
                os("--format"),
                os("{{.Digest}}"),
                OsString::from(image),
            ],
            self.deadlines.load,
        )?;
        if o.code != Some(0) {
            return Err(stderr_failure(&o));
        }
        if o.stdout_cut {
            return Err(failed("the loaded image has no digest"));
        }
        last_line(&o.stdout)
            .and_then(|l| parse_digest(&l))
            .ok_or_else(|| failed("the loaded image has no digest"))
    }

    fn compose_up(&self, release_dir: &Path, services: &[Service]) -> Result<(), PortError> {
        // R-6c (cox W1-105, sam's R6UT-SAM01): a check adapter starts no container, whatever the engine asks of it; the refusal is the adapter's own and
        // comes before anything else, so the `podman` of this adapter is never started
        if self.check_only {
            return Err(failed(
                "a check starts no container; compose is refused by the check adapter",
            ));
        }
        if services.is_empty() {
            return Err(failed("no service to start"));
        }
        // The compose file and `stack.env` are named by this directory and the child runs IN it (hue: `compose.yml`
        // bind-mounts `./initdb`): a relative one would be applied twice.
        if !release_dir.is_absolute() {
            return Err(failed("the release directory is not an absolute path"));
        }
        // R4: the release must name its project validly, even though `compose up` takes the file itself.
        container_name(release_dir, Service::Db).map_err(|_| no_project())?;
        // Cox's amendment of R2: three variables from the constructor, validated before anything starts.
        let vars = self
            .compose_env
            .vars()
            .map_err(|_| failed("the compose variables are not valid"))?;
        let mut args = vec![
            os("compose"),
            os("-f"),
            release_dir.join("compose.yml").into_os_string(),
            os("--env-file"),
            release_dir.join("stack.env").into_os_string(),
            os("up"),
            os("-d"),
        ];
        args.extend(services.iter().map(|s| os(s.name())));
        let o = run_spec(
            &Spec {
                program: &self.podman,
                args: &args,
                cwd: release_dir,
                extra_env: &vars,
            },
            self.deadlines.up,
        )
        .map_err(|e| run_failure(e, PODMAN))?;
        if o.code == Some(0) {
            Ok(())
        } else {
            Err(stderr_failure(&o))
        }
    }

    fn state(&self, release_dir: &Path, service: Service) -> Result<ServiceState, PortError> {
        let ctr = container_name(release_dir, service).map_err(|_| no_project())?;
        let o = self.podman(
            vec![
                os("inspect"),
                os("--format"),
                os("{{.Name}}|{{.State.Status}}|{{.State.ExitCode}}|{{with .State.Health}}{{.Status}}{{end}}"),
                OsString::from(&ctr),
            ],
            self.deadlines.inspect,
        )?;
        if o.code != Some(0) {
            // podman 5.8.2: exit 125 and `Error: no such object: "<name>"` for a container that does not exist.
            let missing = format!("Error: no such object: \"{ctr}\"");
            // Only from a stderr that is all there: a cut or still-open one may hide a later line (vera's UPREAL-L3).
            if o.code == Some(125)
                && !o.stderr_cut
                && last_line(&o.stderr).as_deref() == Some(missing.as_str())
            {
                return Ok(ServiceState::Missing);
            }
            return Err(stderr_failure(&o));
        }
        if o.stdout_cut {
            return Err(failed(INCOMPLETE));
        }
        let line = last_line(&o.stdout).ok_or_else(|| failed("podman gave no reading"))?;
        let fields: Vec<&str> = line.split('|').collect();
        let [name, status, exit_code, health] = fields[..] else {
            return Err(failed("podman gave a reading that is not four fields"));
        };
        if name != ctr {
            return Err(failed("podman answered for another container"));
        }
        let code: i32 = exit_code
            .parse()
            .map_err(|_| failed("podman gave an exit code that is not a number"))?;
        Ok(match (status, health) {
            ("running", "healthy") => ServiceState::Healthy,
            ("running", "unhealthy") => ServiceState::Unhealthy,
            ("running", "starting") => ServiceState::Starting,
            ("running", "") => ServiceState::Running,
            ("exited", _) => ServiceState::Exited(code),
            _ => ServiceState::Other,
        })
    }

    fn run_roles(
        &self,
        release_dir: &Path,
        modules: &[ModuleId],
        timeout: Duration,
    ) -> Result<(), PortError> {
        // R-6d (cox row 49621ff3): the roles script changes database roles; a check adapter refuses it before the script is looked at or started
        if self.check_only {
            return Err(failed(
                "a check runs no roles step; the step is refused by the check adapter",
            ));
        }
        if modules.is_empty() {
            // Fail closed: a roles script without an argument is never a way to run it (the manifest cannot hold an
            // empty list, so the engine never sends one).
            return Err(failed("no module ids"));
        }
        if !release_dir.is_absolute() {
            return Err(failed("the release directory is not an absolute path"));
        }
        let script = release_dir.join(ROLES_SCRIPT);
        // A regular file (not a link) that its owner can execute: nothing is started otherwise.
        let ok = std::fs::symlink_metadata(&script)
            .map(|m| m.file_type().is_file() && m.permissions().mode() & 0o100 != 0)
            .unwrap_or(false);
        if !ok {
            return Err(failed("the roles script is not an executable regular file"));
        }
        // The last check before the start is the project the script reaches (see ROLES_PROJECT): a release without a
        // valid project name is refused as for every other call, and one of another project by its own fixed line,
        // which holds no name.
        let project = project_name(release_dir).map_err(|_| no_project())?;
        if project != ROLES_PROJECT {
            return Err(failed(
                "the release is not of the project the roles script targets",
            ));
        }
        // The engine's ABSOLUTE deadline for the script (cox W1-55, hue's option 1): the wall clock at this launch plus the
        // port's timeout, each in whole seconds (rounded down, so it is never later than the real deadline). The script reads
        // no clock of its own for a deadline: its last statement refuses to commit after `E - 9` (idle 5 + commit 2 + slack
        // 2), so that no roles transaction commits after the engine has given up. One value, computed once, when everything
        // else has been checked and nothing has been started.
        let out_of_range = || failed("the roles deadline is out of range");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| out_of_range())?
            .as_secs();
        let deadline = roles_deadline(now, timeout).ok_or_else(out_of_range)?;
        let mut args: Vec<OsString> = vec![os("--deadline"), os(&deadline.to_string())];
        args.extend(modules.iter().map(|m| os(m.as_str())));
        let started = Instant::now();
        let o =
            run(script.as_os_str(), &args, timeout).map_err(|e| run_failure(e, ROLES_SCRIPT))?;
        if o.code == Some(0) && !o.stdout_cut && !o.stderr_cut {
            Ok(())
        } else {
            // No nonzero exit, signal or incomplete stream can settle the transaction.
            std::thread::sleep(timeout.saturating_sub(started.elapsed()));
            Err(PortError::TimedOut)
        }
    }
}

fn no_project() -> PortError {
    failed("the release has no valid project name")
}
