//! The driver of the LIVE roles proof (cox W1-55: "E computed by the ADAPTER at launch, not by the harness").
//!
//! Without `CORTEX_REAL_LIVE` the second cell does nothing (the partition runs it with the rest, DB-free), and the first
//! cell checks the driver's own argument parser. With it, ONE call of the real `RealContainers::run_roles` is made on a
//! release directory the harness built (`compose.yml` of project `cv2-s5`, a `db-roles.sh` that is the harness's wrapper
//! around the committed script), and the driver prints the wall clock at the launch, the result and the wall clock at the
//! return, as `LIVE-LAUNCH`, `LIVE-RESULT` and `LIVE-RETURN` lines. The `--deadline E` the adapter passed is logged by the
//! wrapper. The spec is one string: `<home>|<release dir>|<timeout seconds>|<module id>,<module id>...`.
use cortex_types::ModuleId;
use cortex_upgrade::ports::Containers;
use cortex_upgrade::real::RealContainers;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The local port the compose variables name (not used by `run_roles`; the constructor needs one).
const PORT: u16 = 8611;

#[derive(Debug, PartialEq, Eq)]
struct Live {
    home: PathBuf,
    release: PathBuf,
    timeout: Duration,
    modules: Vec<String>,
}

fn parse(spec: &str) -> Result<Live, &'static str> {
    let parts: Vec<&str> = spec.split('|').collect();
    let [home, release, secs, mods] = parts[..] else {
        return Err("the spec has not four fields");
    };
    let (home, release) = (PathBuf::from(home), PathBuf::from(release));
    if !home.is_absolute() || !release.is_absolute() {
        return Err("a directory is not absolute");
    }
    let secs: u64 = secs.parse().map_err(|_| "the timeout is not a number")?;
    let modules: Vec<String> = mods.split(',').map(str::to_owned).collect();
    if mods.is_empty() || modules.iter().any(|m| ModuleId::parse(m).is_err()) {
        return Err("a module id is not valid");
    }
    Ok(Live {
        home,
        release,
        timeout: Duration::from_secs(secs),
        modules,
    })
}

fn epoch_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

#[test]
fn rv_01_the_driver_reads_its_spec_and_refuses_a_bad_one() {
    assert_eq!(
        parse("/h|/h/r|45|audit,tokens"),
        Ok(Live {
            home: "/h".into(),
            release: "/h/r".into(),
            timeout: Duration::from_secs(45),
            modules: vec!["audit".into(), "tokens".into()],
        })
    );
    for bad in [
        "",
        "/h|/h/r|45",
        "/h|/h/r|45|audit|x",
        "h|/h/r|45|audit",
        "/h|r|45|audit",
        "/h|/h/r|x|audit",
        "/h|/h/r|-1|audit",
        "/h|/h/r|45|",
        "/h|/h/r|45|audit,",
        "/h|/h/r|45|Audit",
        "/h|/h/r|45|a;b",
    ] {
        assert!(parse(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn rv_02_live_run_roles_from_the_environment() {
    let Ok(spec) = std::env::var("CORTEX_REAL_LIVE") else {
        return; // not a live run: nothing to do
    };
    let live = parse(&spec).unwrap_or_else(|e| panic!("CORTEX_REAL_LIVE: {e}"));
    let modules: Vec<ModuleId> = live
        .modules
        .iter()
        .map(|m| ModuleId::parse(m).expect("checked"))
        .collect();
    // a live run of the roles step only: no compose call, so no provider (the production constructor requires one)
    let containers = RealContainers::with(
        "podman",
        cortex_upgrade::real::Deadlines::default(),
        cortex_upgrade::real::ComposeEnv::new(live.home.clone(), PORT),
    );
    println!("LIVE-LAUNCH epoch_ms={}", epoch_ms());
    let result = containers.run_roles(&live.release, &modules, live.timeout);
    println!("LIVE-RETURN epoch_ms={}", epoch_ms());
    println!("LIVE-RESULT {result:?}");
}
