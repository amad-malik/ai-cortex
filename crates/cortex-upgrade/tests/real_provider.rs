//! R-6, stage E (cox W1-68, W1-91 Q1): the REAL adapter's compose child carries the verified provider the entry read once, by
//! `PODMAN_COMPOSE_PROVIDER=<absolute path>` (podman is told its provider by the environment, the argument vector does not change), on every
//! `podman compose` call and on no other child. A provider that is not an absolute path without control characters is refused before anything starts.
//! The parent's own value of the variable never reaches a child (the environment is cleared first). No real `podman`: the fakes of `real_fakes.rs`.
#[path = "real_fakes.rs"]
mod fakes;

use cortex_types::ModuleId;
use cortex_upgrade::ports::{Containers, PortError, Service};
use cortex_upgrade::real::{ComposeEnv, Deadlines, RealContainers};
use fakes::{Harness, Paths};
use std::path::PathBuf;
use std::time::Duration;

const PROVIDER: &str = "/usr/bin/podman-compose";
const MARK: &str = "CORTEX_REAL_PROVIDER_CHILD";

fn quick() -> Deadlines {
    Deadlines {
        load: Duration::from_secs(20),
        up: Duration::from_secs(20),
        inspect: Duration::from_secs(20),
    }
}

fn harness(tag: &str) -> Harness {
    Harness::new(
        tag,
        "case \"$1\" in\n inspect) echo 'cortexv2-server|running|0|healthy' ;;\nesac\n",
    )
}

fn containers(p: &Paths, provider: Option<&str>) -> RealContainers {
    let mut env = ComposeEnv::new(p.home.clone(), fakes::PORT);
    if let Some(x) = provider {
        env = env.with_provider(PathBuf::from(x));
    }
    RealContainers::with(p.podman.as_os_str(), quick(), env)
}

fn paths_of(h: &Harness) -> Paths {
    Paths::at(&h.tmp.path().display().to_string())
}

fn get<'a>(c: &'a fakes::Call, k: &str) -> Option<&'a str> {
    c.env.iter().find_map(|l| l.strip_prefix(&format!("{k}=")))
}

#[test]
fn rp_01_the_compose_child_carries_the_provider_and_the_argument_vector_is_unchanged() {
    let h = harness("rp1");
    let p = paths_of(&h);
    containers(&p, Some(PROVIDER))
        .compose_up(&p.release, &[Service::Db, Service::Server])
        .expect("compose_up");
    let calls = h.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    let c = &calls[0];
    assert_eq!(
        c.argv,
        [
            "compose".to_string(),
            "-f".into(),
            format!("{}/compose.yml", p.release.display()),
            "--env-file".into(),
            format!("{}/stack.env", p.release.display()),
            "up".into(),
            "-d".into(),
            "db".into(),
            "server".into(),
        ],
        "podman is told its provider by the environment: the vector has no provider in it"
    );
    assert_eq!(get(c, "PODMAN_COMPOSE_PROVIDER"), Some(PROVIDER));
    assert_eq!(
        c.env
            .iter()
            .filter(|l| l.starts_with("PODMAN_COMPOSE_PROVIDER="))
            .count(),
        1
    );
    let names = c.env_names();
    for n in [
        "CORTEX_CONFIG",
        "CORTEX_DUMP_STATUS",
        "CORTEX_PORT",
        "PODMAN_COMPOSE_PROVIDER",
    ] {
        assert!(names.contains(&n.to_string()), "{n}: {names:?}");
    }
    assert!(
        !c.argv
            .iter()
            .any(|a| a == "docker-compose" || a == "podman-compose" || a == PROVIDER),
        "no provider is named in the vector, by bare name or by path: {:?}",
        c.argv
    );
}

#[test]
fn rp_02_no_other_child_carries_it() {
    let h = harness("rp2");
    let p = paths_of(&h);
    let c = containers(&p, Some(PROVIDER));
    c.compose_up(&p.release, &[Service::Db])
        .expect("compose_up");
    c.state(&p.release, Service::Server).expect("state");
    let calls = h.calls();
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert_eq!(get(&calls[0], "PODMAN_COMPOSE_PROVIDER"), Some(PROVIDER));
    assert_eq!(calls[1].argv.first().map(String::as_str), Some("inspect"));
    assert_eq!(
        get(&calls[1], "PODMAN_COMPOSE_PROVIDER"),
        None,
        "the inspect child gets the four fixed names and nothing more"
    );
}

#[test]
fn rp_03_a_provider_that_is_not_an_absolute_path_without_control_characters_starts_nothing() {
    for bad in [
        "",
        "podman-compose",
        "bin/podman-compose",
        "./podman-compose",
        "/a\nb",
        "/a\rb",
        "/a\0b",
        "/a\tb",
        "/a\x1bb",
        "/a\x7fb",
    ] {
        let h = harness("rp3");
        let p = paths_of(&h);
        let r = containers(&p, Some(bad)).compose_up(&p.release, &[Service::Db]);
        match r {
            Err(PortError::Failed(line)) => assert!(
                line.to_string().contains("compose variables are not valid"),
                "{bad:?}: {line}"
            ),
            other => panic!("{bad:?}: expected a refusal, got {other:?}"),
        }
        assert!(h.calls().is_empty(), "{bad:?}: nothing was started");
    }
}

#[test]
fn rp_04_the_variables_of_the_constructor_are_the_three_and_the_provider_last_and_only_when_given()
{
    let home = PathBuf::from("/srv/cortex");
    let plain = ComposeEnv::new(home.clone(), 8601).vars().unwrap();
    let names: Vec<String> = plain
        .iter()
        .map(|(k, _)| k.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        ["CORTEX_CONFIG", "CORTEX_DUMP_STATUS", "CORTEX_PORT"]
    );
    let with = ComposeEnv::new(home.clone(), 8601)
        .with_provider(PathBuf::from(PROVIDER))
        .vars()
        .unwrap();
    let names: Vec<String> = with
        .iter()
        .map(|(k, _)| k.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        [
            "CORTEX_CONFIG",
            "CORTEX_DUMP_STATUS",
            "CORTEX_PORT",
            "PODMAN_COMPOSE_PROVIDER"
        ]
    );
    assert_eq!(with[3].1, std::ffi::OsString::from(PROVIDER));
    // the production constructor requires one
    let c = RealContainers::new(home, 8601, PathBuf::from(PROVIDER));
    assert!(format!("{c:?}").contains(PROVIDER), "{c:?}");
}

#[test]
fn rp_05_the_parents_own_value_never_reaches_a_child_and_ours_wins() {
    if let Ok(base) = std::env::var(MARK) {
        let p = Paths::at(&base);
        containers(&p, Some(PROVIDER))
            .compose_up(&p.release, &[Service::Db])
            .unwrap();
        containers(&p, None)
            .compose_up(&p.release, &[Service::Db])
            .unwrap();
        return;
    }
    let h = harness("rp5");
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "rp_05_the_parents_own_value_never_reaches_a_child_and_ours_wins",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(MARK, h.tmp.path().display().to_string())
        .env("PODMAN_COMPOSE_PROVIDER", "/evil/provider")
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/home/someone")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let calls = h.calls();
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert_eq!(get(&calls[0], "PODMAN_COMPOSE_PROVIDER"), Some(PROVIDER));
    assert_eq!(
        get(&calls[1], "PODMAN_COMPOSE_PROVIDER"),
        None,
        "without a provider the parent's value does not leak either: the environment is cleared"
    );
}

#[test]
fn rp_06_the_check_constructor_has_no_provider_and_the_production_one_has() {
    // `for_check` (R-6b): no provider, named so that it cannot be confused with `new`
    let home = PathBuf::from("/srv/cortex");
    let check = RealContainers::for_check(home.clone(), 8601);
    let shown = format!("{check:?}");
    assert!(shown.contains("provider: None"), "{shown}");
    assert!(!shown.contains(PROVIDER), "{shown}");
    // R-6c: the flag that makes the check adapter refuse compose is on the check constructor (the production one for a check) ...
    assert!(shown.contains("check_only: true"), "{shown}");
    let prod = RealContainers::new(home, 8601, PathBuf::from(PROVIDER));
    let shown = format!("{prod:?}");
    assert!(
        shown.contains(PROVIDER) && !shown.contains("provider: None"),
        "{shown}"
    );
    // ... and NOT on the production adapter that starts containers
    assert!(shown.contains("check_only: false"), "{shown}");
    // and through a fake podman: a compose call of an ORDINARY adapter that has no provider (`with`, not the check-only adapter: rp_07 covers that one) carries no provider variable at all
    let h = harness("rp6");
    let p = paths_of(&h);
    let c = RealContainers::with(
        p.podman.as_os_str(),
        quick(),
        ComposeEnv::new(p.home.clone(), fakes::PORT),
    );
    c.compose_up(&p.release, &[Service::Db])
        .expect("compose_up");
    let calls = h.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(get(&calls[0], "PODMAN_COMPOSE_PROVIDER"), None);
}

#[test]
fn rp_07_the_check_adapter_refuses_compose_itself_and_starts_no_podman() {
    // R-6c (cox W1-105, sam's R6UT-SAM01): the check adapter has no provider and starts no container; if a path ever reached `compose_up` with it, it must
    // find a refusal and the fake `podman` must never be started. A fake `podman` by absolute path, so that this cell can never reach the real one.
    let h = harness("rp7");
    let p = paths_of(&h);
    let check = RealContainers::for_check_with(
        p.podman.as_os_str(),
        quick(),
        ComposeEnv::new(p.home.clone(), fakes::PORT),
    );
    let e = check
        .compose_up(&p.release, &[Service::Db])
        .expect_err("a check adapter refuses compose");
    let shown = format!("{e:?}");
    assert!(shown.contains("a check starts no container"), "{shown}");
    assert!(
        h.calls().is_empty(),
        "podman was not started: {:?}",
        h.calls()
    );
    // controls: an adapter that is not a check adapter still composes, with and without a provider (the cells' constructor, and the production shape)
    for (tag, provider) in [("rp7a", Some(PROVIDER)), ("rp7b", None)] {
        let h = harness(tag);
        let p = paths_of(&h);
        containers(&p, provider)
            .compose_up(&p.release, &[Service::Db])
            .expect("compose_up");
        assert_eq!(h.calls().len(), 1, "{tag}: {:?}", h.calls());
    }
}

#[test]
fn rp_08_the_check_adapter_refuses_the_roles_step_itself_and_starts_nothing() {
    // R-6d (cox row 49621ff3): the check adapter changes nothing; the roles step (a script that changes database roles) is refused before the script is
    // looked at or started. The fake roles script logs a call if it runs, and the fake `podman` by absolute path means no cell can reach the real one.
    let h = harness("rp8");
    h.roles_script("exit 0\n");
    let audit = [ModuleId::parse("audit").unwrap()];
    let check = RealContainers::for_check_with(
        h.podman.as_os_str(),
        quick(),
        ComposeEnv::new(h.home.clone(), fakes::PORT),
    );
    let e = check
        .run_roles(&h.release, &audit, Duration::from_secs(20))
        .expect_err("a check adapter refuses the roles step");
    let shown = format!("{e:?}");
    assert!(shown.contains("a check runs no roles step"), "{shown}");
    assert!(h.calls().is_empty(), "nothing was started: {:?}", h.calls());
    // control: an adapter that is not a check adapter runs the same script
    let h = harness("rp8c");
    h.roles_script("exit 0\n");
    containers(&paths_of(&h), None)
        .run_roles(&h.release, &audit, Duration::from_secs(20))
        .expect("run_roles");
    assert_eq!(h.calls().len(), 1, "{:?}", h.calls());
}

#[test]
fn rp_09_the_check_adapter_refuses_to_load_an_image_itself_and_starts_no_podman() {
    // R-6d (cox row 49621ff3): `podman load` changes the image store; a check adapter refuses it before `podman` is started.
    let body = format!(
        "case \"$1\" in\n load) printf '%b' 'Loaded image: localhost/cortex-server:0.2.0\\n' ;;\n image) printf '%b' '{}\\n' ;;\nesac\n",
        fakes::DIGEST
    );
    let h = Harness::new("rp9", &body);
    let archive = h.tmp.path().join("images.tar");
    let check = RealContainers::for_check_with(
        h.podman.as_os_str(),
        quick(),
        ComposeEnv::new(h.home.clone(), fakes::PORT),
    );
    let e = check
        .load_image(&archive)
        .expect_err("a check adapter refuses to load an image");
    let shown = format!("{e:?}");
    assert!(shown.contains("a check loads no image"), "{shown}");
    assert!(
        h.calls().is_empty(),
        "podman was not started: {:?}",
        h.calls()
    );
    // control: an adapter that is not a check adapter loads it (a load and an image inspect)
    let h = Harness::new("rp9c", &body);
    let archive = h.tmp.path().join("images.tar");
    containers(&paths_of(&h), None)
        .load_image(&archive)
        .expect("load_image");
    assert_eq!(h.calls().len(), 2, "{:?}", h.calls());
}
