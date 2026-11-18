//! Piece 14, the ports of the engine (`src/ports.rs`): the small behaviours every port shares, and that the traits can
//! be used as trait objects (the engine holds `&dyn`). A signature change in this file is a compile error here.

use cortex_types::{ModuleId, Secret};
use cortex_upgrade::manifest::Digest;
use cortex_upgrade::ports::*;
use cortex_upgrade::version::parse_version;
use std::path::Path;
use std::time::Duration;

#[test]
fn po_01_the_services_are_a_closed_list_with_their_compose_names() {
    assert_eq!(Service::ALL.len(), 3);
    let names: Vec<&str> = Service::ALL.iter().map(|s| s.name()).collect();
    assert_eq!(names, ["db", "migrate", "server"]);
    assert_eq!(Service::Db.name(), "db");
    assert_eq!(Service::Migrate.name(), "migrate");
    assert_eq!(Service::Server.name(), "server");
}

#[test]
fn po_02_one_line_is_escaped_and_bounded() {
    assert_eq!(OneLine::new("").as_str(), "");
    assert_eq!(OneLine::new("plain text 123").as_str(), "plain text 123");
    // a control character never survives
    assert_eq!(OneLine::new("a\nb").as_str(), "a\\nb");
    assert_eq!(OneLine::new("a\tb\rc").as_str(), "a\\tb\\rc");
    assert_eq!(OneLine::new("\u{1b}[31mred").as_str(), "\\u{1b}[31mred");
    assert_eq!(OneLine::new("a\0b").as_str(), "a\\0b");
    // quotes are readable text and stay; a backslash is escaped so an escape can be told from text; a bidi override is not text
    assert_eq!(OneLine::new("say \"hi\" it's").as_str(), "say \"hi\" it's");
    assert_eq!(OneLine::new("a\\nb").as_str(), "a\\\\nb");
    assert_eq!(OneLine::new("x\u{202e}y").as_str(), "x\\u{202e}y");
    // 200 characters pass, 201 are cut to 200 and say so
    let exact = "x".repeat(200);
    assert_eq!(OneLine::new(&exact).as_str(), exact);
    let over = "x".repeat(201);
    assert_eq!(
        OneLine::new(&over).as_str(),
        format!("{}...", "x".repeat(200))
    );
    assert_eq!(OneLine::new(&"y".repeat(100_000)).as_str().len(), 203);
    // characters, not bytes: a multi-byte character is never cut in half
    let wide = "é".repeat(201);
    assert_eq!(
        OneLine::new(&wide).as_str(),
        format!("{}...", "é".repeat(200))
    );
    // the cut comes first, the escape after: 200 newlines are 200 escapes
    let nl = "\n".repeat(201);
    assert_eq!(
        OneLine::new(&nl).as_str(),
        format!("{}...", "\\n".repeat(200))
    );
    assert_eq!(format!("{}", OneLine::new("a\nb")), "a\\nb");
}

#[test]
fn po_03_the_last_non_empty_line_of_an_output() {
    let l = |b: &[u8]| OneLine::last_line_of(b).as_str().to_string();
    assert_eq!(l(b""), "");
    assert_eq!(l(b"one"), "one");
    assert_eq!(l(b"one\ntwo\n"), "two");
    assert_eq!(l(b"one\ntwo"), "two");
    assert_eq!(
        l(b"one\ntwo\n\n\n"),
        "two",
        "trailing empty lines are skipped"
    );
    assert_eq!(l(b"one\ntwo\n  \n\t\n"), "two", "a line of blanks is empty");
    assert_eq!(l(b"one\r\ntwo\r\n"), "two", "CRLF");
    assert_eq!(l(b"\n\n\n"), "");
    assert_eq!(
        l(b"first\n  indented line  \n"),
        "indented line",
        "the line is trimmed"
    );
    // not UTF-8: replaced, never an error and never raw
    assert_eq!(l(b"ok\n\xff\xfe bad\n"), "\u{fffd}\u{fffd} bad");
    // control characters inside the line are escaped
    assert_eq!(l(b"ok\n\x1b[1mbold\n"), "\\u{1b}[1mbold");
    // the same bound as OneLine::new
    let long = format!("a\n{}\n", "z".repeat(5000));
    assert_eq!(l(long.as_bytes()), format!("{}...", "z".repeat(200)));
    // a large output without a newline is still one bounded line
    assert_eq!(l(&vec![b'q'; 1 << 20]).len(), 203);
}

#[test]
fn po_04_a_dump_label_is_made_only_from_two_versions() {
    let a = parse_version("0.2.1-rc.1").unwrap();
    let b = parse_version("0.2.1-rc.2").unwrap();
    assert_eq!(
        DumpLabel::new(&a, &b).as_str(),
        "pre-upgrade-0.2.1-rc.1-to-0.2.1-rc.2"
    );
    let c = parse_version("1.10.3").unwrap();
    assert_eq!(
        DumpLabel::new(&c, &a).as_str(),
        "pre-upgrade-1.10.3-to-0.2.1-rc.1"
    );
    assert!(
        DumpLabel::new(&a, &b)
            .as_str()
            .bytes()
            .all(|x| x.is_ascii_alphanumeric() || b".-".contains(&x))
    );
}

#[test]
fn po_05_the_system_pace_moves_forward_and_sleeps() {
    let p = SystemPace::new();
    let t0 = p.now();
    assert!(t0 < Duration::from_secs(5), "{t0:?}");
    p.sleep(Duration::from_millis(60));
    let t1 = p.now();
    assert!(t1 >= t0 + Duration::from_millis(60), "{t0:?} then {t1:?}");
    assert!(t1 < t0 + Duration::from_secs(5), "{t0:?} then {t1:?}");
    assert!(p.now() >= t1, "never goes back");
}

#[test]
fn po_06_a_port_error_prints_its_one_line_and_nothing_else() {
    let e = PortError::Failed(OneLine::new("podman: no such image\nSYNTHETIC"));
    assert_eq!(e.to_string(), "failed: podman: no such image\\nSYNTHETIC");
    assert_eq!(PortError::TimedOut.to_string(), "timed out");
}

// ---------------------------------------------------------------- the traits are usable as trait objects

struct All;
impl Stack for All {
    fn ready(&self) -> Ready {
        Ready::Yes
    }
    fn whoami(&self, _t: &Secret) -> Whoami {
        Whoami::Ok
    }
}
impl Psql for All {
    fn trust_rule_count(&self) -> Result<String, NoAnswer> {
        Ok("0".into())
    }
}
impl Containers for All {
    fn load_image(&self, _a: &Path) -> Result<Digest, PortError> {
        Ok(Digest([7; 32]))
    }
    fn compose_up(&self, _d: &Path, _s: &[Service]) -> Result<(), PortError> {
        Ok(())
    }
    fn state(&self, _d: &Path, _s: Service) -> Result<ServiceState, PortError> {
        Ok(ServiceState::Healthy)
    }
    fn run_roles(&self, _d: &Path, _m: &[ModuleId], _t: Duration) -> Result<(), PortError> {
        Ok(())
    }
}
impl Dumper for All {
    fn dump(&self, _l: &DumpLabel, _t: Duration) -> Result<DumpResult, PortError> {
        Ok(DumpResult {
            exit: 0,
            last_stdout_line: None,
            last_stderr_line: OneLine::new(""),
        })
    }
}
impl Pace for All {
    fn now(&self) -> Duration {
        Duration::ZERO
    }
    fn sleep(&self, _d: Duration) {}
}

#[test]
fn po_07_every_port_is_object_safe_and_the_shapes_are_these() {
    let all = All;
    let stack: &dyn Stack = &all;
    let psql: &dyn Psql = &all;
    let containers: &dyn Containers = &all;
    let dumper: &dyn Dumper = &all;
    let pace: &dyn Pace = &all;
    assert_eq!(stack.ready(), Ready::Yes);
    assert_eq!(stack.whoami(&Secret::new("t")), Whoami::Ok);
    assert_eq!(psql.trust_rule_count(), Ok("0".to_string()));
    let d = Path::new("/nonexistent");
    assert_eq!(containers.load_image(d), Ok(Digest([7; 32])));
    assert_eq!(containers.compose_up(d, &[Service::Db]), Ok(()));
    assert_eq!(
        containers.state(d, Service::Migrate),
        Ok(ServiceState::Healthy)
    );
    assert_eq!(containers.run_roles(d, &[], Duration::from_secs(1)), Ok(()));
    let a = parse_version("0.2.1").unwrap();
    assert_eq!(
        dumper
            .dump(&DumpLabel::new(&a, &a), Duration::from_secs(1))
            .unwrap()
            .exit,
        0
    );
    assert_eq!(pace.now(), Duration::ZERO);
    // the states a one-shot container can be in
    assert_ne!(ServiceState::Exited(0), ServiceState::Exited(1));
}
