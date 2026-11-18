//! Piece 14, real ports: `RealDumper` (brief r1 section 3, "Dumper::dump"). R1, R3, R4, R6, R8 through a fake
//! `<home>/bin/cortex-dump`.
#[path = "real_fakes.rs"]
mod fakes;

use cortex_upgrade::ports::{DumpLabel, DumpResult, Dumper, OneLine, PortError};
use cortex_upgrade::real::RealDumper;
use cortex_upgrade::version::parse_version;
use fakes::Harness;
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::time::Duration;

fn label() -> DumpLabel {
    DumpLabel::new(
        &parse_version("0.2.0").unwrap(),
        &parse_version("0.3.1").unwrap(),
    )
}

fn dumper(h: &Harness) -> RealDumper {
    RealDumper::with_grace(h.home.clone(), h.release.clone(), Duration::from_secs(2))
}

#[test]
fn rd_01_the_tool_is_started_by_its_absolute_path_with_exactly_this_vector() {
    let h = Harness::new("rd1", "exit 0\n");
    let tool = h.dump_tool("echo /x/dumps/pre-upgrade/set-1\nexit 0\n");
    dumper(&h).dump(&label(), Duration::from_secs(90)).unwrap();
    let calls = h.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].program, tool.display().to_string());
    assert_eq!(
        calls[0].argv,
        [
            "--target",
            &h.ctr("db"),
            "--out",
            &format!("{}/dumps", h.home.display()),
            "--label",
            "pre-upgrade-0.2.0-to-0.3.1",
            "--timeout",
            "90"
        ]
    );
}

#[test]
fn rd_02_the_exit_code_is_reported_whatever_it_is_and_the_lines_are_kept_bounded() {
    for code in [0, 1, 2, 3, 4, 7, 255] {
        let h = Harness::new("rd2", "exit 0\n");
        h.dump_tool(&format!("echo noise\necho /set/path\necho 'warn one' >&2\necho 'restore warning: x' >&2\nexit {code}\n"));
        let r = dumper(&h).dump(&label(), Duration::from_secs(60)).unwrap();
        assert_eq!(
            r,
            DumpResult {
                exit: code,
                last_stdout_line: Some("/set/path".to_string()),
                last_stderr_line: OneLine::new("restore warning: x"),
            },
            "exit {code}"
        );
    }
}

#[test]
fn rd_03_the_last_stdout_line_is_none_when_there_is_none_or_it_is_over_4096_bytes() {
    let run = |body: &str| {
        let h = Harness::new("rd3", "exit 0\n");
        h.dump_tool(body);
        dumper(&h)
            .dump(&label(), Duration::from_secs(60))
            .unwrap()
            .last_stdout_line
    };
    assert_eq!(run("exit 0\n"), None);
    assert_eq!(run("printf '\\n\\n   \\n'\nexit 0\n"), None);
    assert_eq!(
        run(&format!("echo '{}'\nexit 0\n", "p".repeat(4096))),
        Some("p".repeat(4096))
    );
    assert_eq!(run(&format!("echo '{}'\nexit 0\n", "p".repeat(4097))), None);
    assert_eq!(
        run("echo first\necho last\nprintf '\\n\\n'\nexit 0\n"),
        Some("last".to_string())
    );
}

#[test]
fn rd_04_a_tool_ended_by_a_signal_is_failed() {
    let h = Harness::new("rd4", "exit 0\n");
    h.dump_tool("kill -9 $$\n");
    let r = dumper(&h).dump(&label(), Duration::from_secs(60));
    assert!(matches!(r, Err(PortError::Failed(_))), "{r:?}");
}

#[test]
fn rd_05_a_tool_that_is_not_a_regular_executable_file_is_not_installed_and_is_never_started() {
    let not_installed = PortError::Failed(OneLine::new("the dump tool is not installed"));
    // missing
    let h = Harness::new("rd5a", "exit 0\n");
    assert_eq!(
        dumper(&h).dump(&label(), Duration::from_secs(60)),
        Err(not_installed.clone())
    );
    assert!(h.calls().is_empty());
    // not executable
    let h = Harness::new("rd5b", "exit 0\n");
    let p = h.dump_tool("exit 0\n");
    fs::set_permissions(&p, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        dumper(&h).dump(&label(), Duration::from_secs(60)),
        Err(not_installed.clone())
    );
    assert!(h.calls().is_empty());
    // a symlink to an executable
    let h = Harness::new("rd5c", "exit 0\n");
    let real = h.tmp.path().join("elsewhere");
    fakes::script(&real, &h.log, "exit 0\n");
    fs::create_dir_all(h.home.join("bin")).unwrap();
    symlink(&real, h.home.join("bin/cortex-dump")).unwrap();
    assert_eq!(
        dumper(&h).dump(&label(), Duration::from_secs(60)),
        Err(not_installed.clone())
    );
    assert!(h.calls().is_empty());
    // a directory
    let h = Harness::new("rd5d", "exit 0\n");
    fs::create_dir_all(h.home.join("bin/cortex-dump")).unwrap();
    assert_eq!(
        dumper(&h).dump(&label(), Duration::from_secs(60)),
        Err(not_installed)
    );
}

#[test]
fn rd_06_a_current_release_without_a_valid_project_name_is_failed_and_nothing_is_started() {
    let h = Harness::new("rd6", "exit 0\n");
    h.dump_tool("exit 0\n");
    h.set_compose("name: Bad\n");
    let r = dumper(&h).dump(&label(), Duration::from_secs(60));
    assert!(matches!(r, Err(PortError::Failed(_))), "{r:?}");
    assert!(h.calls().is_empty());
}

#[test]
fn rd_07_a_relative_home_is_refused() {
    let h = Harness::new("rd7", "exit 0\n");
    h.dump_tool("exit 0\n");
    let d = RealDumper::new("relative/home".into(), h.release.clone());
    assert_eq!(
        d.dump(&label(), Duration::from_secs(60)),
        Err(PortError::Failed(OneLine::new(
            "the home directory is not an absolute path"
        )))
    );
    assert!(h.calls().is_empty());
}

#[test]
fn rd_08_a_tool_that_outlives_the_timeout_plus_the_grace_is_timed_out() {
    let h = Harness::new("rd8", "exit 0\n");
    h.dump_tool("sleep 30\n");
    let d = RealDumper::with_grace(
        h.home.clone(),
        h.release.clone(),
        Duration::from_millis(500),
    );
    let t = std::time::Instant::now();
    assert_eq!(
        d.dump(&label(), Duration::from_secs(1)),
        Err(PortError::TimedOut)
    );
    // timeout 1 s + grace 0.5 s, and a second to spare (R6)
    assert!(
        t.elapsed() < Duration::from_millis(1500) + Duration::from_secs(1),
        "{:?}",
        t.elapsed()
    );
    assert!(
        t.elapsed() >= Duration::from_millis(1500),
        "the outer deadline is timeout plus the grace: {:?}",
        t.elapsed()
    );
}

#[test]
fn rd_09_the_production_grace_is_30_seconds() {
    let d = RealDumper::new("/h".into(), "/h/releases/1".into());
    let shown = format!("{d:?}");
    assert!(
        shown.contains("30s") || shown.contains("secs: 30"),
        "{shown}"
    );
}

#[test]
fn rd_10_a_stdout_of_more_than_1_mib_has_no_reliable_last_line() {
    // 130000 short lines (about 1.4 MiB): the kept first MiB ends in the middle of the output, so what is "last" there is
    // not the tool's last line (the set path on success): None, never a partial line.
    let h = Harness::new("rd10", "exit 0\n");
    h.dump_tool("i=0; while [ $i -lt 130000 ]; do echo \"line $i\"; i=$((i+1)); done\nexit 0\n");
    let r = dumper(&h).dump(&label(), Duration::from_secs(60)).unwrap();
    assert_eq!(r.exit, 0);
    assert_eq!(r.last_stdout_line, None);
}

/// vera's UPREAL-L4: the allow-list helper held the dump label by its prefix only. The label is `pre-upgrade-<from>-to-<to>`, made
/// of two checked versions: letters, digits and `. - +`, at most 144 characters in all.
#[test]
fn rd_11_the_allow_list_helper_holds_the_dump_label_to_its_whole_shape() {
    let h = Harness::new("rd11", "exit 0\n");
    h.dump_tool("exit 0\n");
    RealDumper::new(h.home.clone(), h.release.clone())
        .dump(&label(), Duration::from_secs(20))
        .unwrap();
    let base = h.calls()[0].clone(); // a genuine call: its environment and vector are right
    let ok = |l: &str| -> bool {
        let mut c = base.clone();
        let i = c.argv.iter().position(|a| a == "--label").unwrap();
        c.argv[i + 1] = l.to_string();
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            fakes::check_closed(&c, &h.home, &h.release)
        }))
        .is_ok()
    };
    assert!(ok("pre-upgrade-0.2.0-to-0.3.0"));
    assert!(ok("pre-upgrade-0.2.0-rc.1+build.5-to-0.3.0"));
    assert!(ok(
        &base.argv[base.argv.iter().position(|a| a == "--label").unwrap() + 1]
    ));
    for bad in [
        "pre-upgrade-",
        "pre-upgrade-0.2.0",
        "pre-upgrade-0.2.0-to-",
        "pre-upgrade--to-0.3.0",
        "pre-upgrade-0.2.0-to-0.3.0;rm",
        "pre-upgrade-0.2.0_rc-to-0.3.0",
        "pre-upgrade-0.2.0-to-0.3.0 x",
        "pre-upgrade-$(id)-to-0.3.0",
        "pre-upgrade-0.2.0-to-0.3.0*",
        "pre-upgrade-0.2.0-to-0.3.0\n",
        "pre-upgrade-0.2.0-to-0.3.0/../x",
        "xpre-upgrade-0.2.0-to-0.3.0",
        "",
    ] {
        assert!(!ok(bad), "{bad:?}");
    }
    let long = format!("pre-upgrade-{}-to-{}", "1".repeat(64), "2".repeat(64));
    assert_eq!(long.len(), 144);
    assert!(ok(&long));
    assert!(!ok(&format!("{long}3")));
}
