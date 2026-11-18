//! The built `cortex` binary against the std-only fake server, with a cleared environment: CL-03 to CL-06, DR-01 to DR-05,
//! EX-02 to EX-05, RT-05, SC-01, TF-12 (mvp/tests/cortex-client-and-cli.md). DB-free.
mod common;

use common::support::fake::{Fake, Reply};
use common::{HEALTH_OK, cortex, health_fake};

const CANARY: &str = "CANARY-7d1e90b4a5c3f268";

fn lines(s: &str) -> Vec<&str> {
    s.lines().collect()
}

#[test]
fn cl_03_the_global_flags_come_before_or_after_the_command_with_the_same_result() {
    let fake = health_fake(HEALTH_OK);
    let url = fake.url();
    let a = cortex(&["--format", "json", "--server", &url, "doctor"], &[]);
    let b = cortex(&["doctor", "--format", "json", "--server", &url], &[]);
    let c = cortex(&["--server", &url, "doctor", "--format", "json"], &[]);
    assert_eq!(
        (a.code, b.code, c.code),
        (0, 0, 0),
        "{} {} {}",
        a.stderr,
        b.stderr,
        c.stderr
    );
    assert_eq!(a.stdout, b.stdout);
    assert_eq!(a.stdout, c.stdout);
    assert!(a.stdout.trim_start().starts_with('{'), "json: {}", a.stdout);

    let md = cortex(&["doctor", "--server", &url], &[]);
    assert_eq!(md.code, 0, "{}", md.stderr);
    assert!(
        !md.stdout.trim_start().starts_with('{'),
        "the default format is md: {}",
        md.stdout
    );

    let bad = cortex(&["--format", "xml", "doctor", "--server", &url], &[]);
    assert_eq!(bad.code, 2);
    assert!(bad.stdout.is_empty());
}

#[test]
fn cl_04_version_is_one_line_and_touches_nothing() {
    let fake = health_fake(HEALTH_OK);
    let url = fake.url();
    for flag in ["--version", "-V"] {
        let r = cortex(&[flag, "--server", &url], &[("CORTEX_TOKEN", CANARY)]);
        assert_eq!(r.code, 0, "{flag}: {}", r.stderr);
        assert_eq!(lines(&r.stdout).len(), 1, "{flag}: {:?}", r.stdout);
        let want = format!("cortex {} (", env!("CARGO_PKG_VERSION"));
        assert!(
            r.stdout.starts_with(&want) && r.stdout.ends_with(")\n"),
            "{flag}: {:?}",
            r.stdout
        );
        assert!(r.stderr.is_empty());
    }
    assert_eq!(fake.count(), 0, "--version makes no request");
    assert_eq!(fake.connections(), 0);
}

#[test]
fn cl_05_usage_help_and_unknown_commands() {
    let env = [
        ("CORTEX_TOKEN", CANARY),
        ("CORTEX_SERVER", "http://127.0.0.1:9"),
    ];
    let none = cortex(&[], &env);
    assert_eq!(none.code, 2, "no arguments is a usage error");
    assert!(none.stdout.is_empty() && !none.stderr.is_empty());
    for args in [vec!["--help"], vec!["doctor", "--help"]] {
        let r = cortex(&args, &env);
        assert_eq!(r.code, 0, "{args:?}: {}", r.stderr);
        assert!(
            r.stdout.contains("doctor") && r.stderr.is_empty(),
            "{args:?}"
        );
    }
    let typo = cortex(&["doctr"], &env);
    assert_eq!(typo.code, 2);
    assert!(
        typo.stderr.contains("doctor"),
        "a suggestion: {}",
        typo.stderr
    );
    for r in [&none, &typo] {
        assert!(!r.stdout.contains(CANARY) && !r.stderr.contains(CANARY));
        assert!(
            !r.stderr.contains("127.0.0.1:9"),
            "no environment value is printed"
        );
    }
}

#[test]
fn cl_06_the_help_is_stable_and_never_carries_an_escape_byte() {
    let plain = cortex(&["--help"], &[]);
    let again = cortex(&["--help"], &[]);
    assert_eq!(plain.code, 0, "{}", plain.stderr);
    assert!(
        plain.stdout.contains("doctor") && plain.stdout.contains("--format"),
        "the help is not empty: {:?}",
        plain.stdout
    );
    assert_eq!(plain.stdout, again.stdout);
    for env in [
        vec![],
        vec![("NO_COLOR", "1")],
        vec![("CLICOLOR_FORCE", "1")],
        vec![("CLICOLOR", "1"), ("TERM", "xterm-256color")],
        vec![("CLICOLOR_FORCE", "1"), ("NO_COLOR", "1")],
    ] {
        let r = cortex(&["--help"], &env);
        assert_eq!(r.code, 0, "{env:?}: {}", r.stderr);
        assert!(
            !r.stdout.contains('\u{1b}') && !r.stderr.contains('\u{1b}'),
            "{env:?}"
        );
        assert_eq!(r.stdout, plain.stdout, "{env:?}");
        let e = cortex(&["doctr"], &env);
        assert_eq!(e.code, 2, "{env:?}: {}", e.stderr);
        assert!(!e.stderr.contains('\u{1b}'), "usage errors too: {env:?}");
    }
}

#[test]
fn dr_01_doctor_is_one_get_without_authorization_and_prints_the_health() {
    let fake = health_fake(HEALTH_OK);
    let url = fake.url();
    let r = cortex(&["doctor", "--server", &url], &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stderr.is_empty(), "{}", r.stderr);
    assert_eq!(
        r.stdout,
        "state: ready\nversion: 0.2.1\nready: true\nbackup: ok\nlast_good_dump_at: none\npieces:\n  identity: enabled=true ready=true\n  records: enabled=true ready=false\n"
    );
    let j = cortex(&["doctor", "--server", &url, "--format", "json"], &[]);
    assert_eq!(
        j.stdout,
        format!("{HEALTH_OK}\n"),
        "json: the object as received"
    );
    let reqs = fake.requests();
    assert_eq!(reqs.len(), 2);
    for q in &reqs {
        assert_eq!(
            (q.method.as_str(), q.target.as_str()),
            ("GET", "/v2/health")
        );
        assert!(q.header("authorization").is_none(), "{}", q.head);
    }
    // a dump time that is set is shown
    let fake = health_fake(&HEALTH_OK.replace(
        "\"last_good_dump_at\":null",
        "\"last_good_dump_at\":\"2026-11-06T10:00:00Z\"",
    ));
    let r = cortex(&["doctor", "--server", &fake.url()], &[]);
    assert!(
        r.stdout
            .contains("last_good_dump_at: 2026-11-06T10:00:00Z\n"),
        "{}",
        r.stdout
    );
}

#[test]
fn dr_02_not_ready_exits_7_and_a_stale_backup_is_one_stderr_line() {
    let not_ready = HEALTH_OK.replace("\"ready\":true,\"pieces\"", "\"ready\":false,\"pieces\"");
    let fake = health_fake(&not_ready);
    let r = cortex(&["doctor", "--server", &fake.url()], &[]);
    assert_eq!(r.code, 7, "{}", r.stderr);
    assert!(
        r.stdout.contains("ready: false\n"),
        "the output is as usual: {}",
        r.stdout
    );
    for state in ["stale", "none"] {
        let fake = health_fake(
            &HEALTH_OK.replace("\"backup\":\"ok\"", &format!("\"backup\":\"{state}\"")),
        );
        let r = cortex(&["doctor", "--server", &fake.url()], &[]);
        assert_eq!(r.code, 0, "{state}: {}", r.stderr);
        assert_eq!(r.stderr, format!("backup: {state}\n"));
        assert!(r.stdout.contains(&format!("backup: {state}\n")));
    }
}

#[test]
fn dr_03_doctor_reads_no_token_source() {
    let fake = health_fake(HEALTH_OK);
    let url = fake.url();
    let dir = common::support::dir("dr03");
    let bad = common::support::write(&dir, "tok", format!("{CANARY}\n").as_bytes(), 0o644);
    let bad_s = bad.display().to_string();
    let runs = [
        cortex(&["doctor", "--server", &url], &[]),
        cortex(&["doctor", "--server", &url, "--token-file", &bad_s], &[]),
        cortex(
            &["doctor", "--server", &url],
            &[
                ("CORTEX_TOKEN", "ab cd"),
                ("CORTEX_TOKEN_FILE", "/nonexistent"),
            ],
        ),
    ];
    for r in &runs {
        assert_eq!(r.code, 0, "{}", r.stderr);
        assert!(r.stderr.is_empty() && !r.stdout.contains(CANARY));
    }
    for q in fake.requests() {
        assert!(q.header("authorization").is_none(), "{}", q.head);
    }
}

#[test]
fn dr_04_a_health_body_that_is_not_the_health_document_prints_nothing_and_exits_1() {
    let bad_bodies = [
        HEALTH_OK.replace("\"ready\":true,\"pieces\"", "\"ready\":\"yes\",\"pieces\""),
        HEALTH_OK.replace(",\"backup\":\"ok\"", ""),
        HEALTH_OK.replace("\"state\":\"ready\",", ""),
        HEALTH_OK.replace("\"pieces\":[{\"name\":\"identity\",\"enabled\":true,\"ready\":true},{\"name\":\"records\",\"enabled\":true,\"ready\":false}]", "\"pieces\":[{\"name\":\"identity\"}]"),
        HEALTH_OK.replace("\"backup\":\"ok\"", "\"backup\":\"great\""),
        "[]".to_owned(),
        "not json".to_owned(),
    ];
    for body in bad_bodies {
        let fake = health_fake(&body);
        let r = cortex(&["doctor", "--server", &fake.url()], &[]);
        assert_eq!(r.code, 1, "{body}: {}", r.stderr);
        assert!(r.stdout.is_empty(), "nothing half-printed: {}", r.stdout);
        assert_eq!(lines(&r.stderr).len(), 1, "{}", r.stderr);
    }
    // an extra member is tolerated
    let fake =
        health_fake(&HEALTH_OK.replace("\"backup\":\"ok\"", "\"backup\":\"ok\",\"extra\":1"));
    assert_eq!(cortex(&["doctor", "--server", &fake.url()], &[]).code, 0);
}

#[test]
fn dr_05_a_server_that_is_down_is_one_stderr_line() {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let url = format!("http://127.0.0.1:{port}");
    let r = cortex(&["doctor", "--server", &url], &[]);
    assert_eq!(r.code, 1);
    assert!(r.stdout.is_empty());
    assert_eq!(lines(&r.stderr).len(), 1, "{}", r.stderr);
    assert!(
        r.stderr.contains(&format!("127.0.0.1:{port}")),
        "{}",
        r.stderr
    );
    for bad in ["panicked", "RUST_BACKTRACE", "thread '"] {
        assert!(!r.stderr.contains(bad), "{}", r.stderr);
    }
}

#[test]
fn ex_02_one_scenario_per_exit_code_through_doctor() {
    let table: [(u16, &str, i32); 8] = [
        (401, "token_invalid", 3),
        (403, "missing_scope", 3),
        (404, "not_found", 4),
        (409, "conflict", 5),
        (422, "invalid_request", 6),
        (428, "idempotency_key_required", 6),
        (503, "unavailable", 7),
        (500, "internal_error", 1),
    ];
    for (status, code, want) in table {
        let fake = Fake::start();
        fake.set_default(Reply::problem(status, code));
        let r = cortex(&["doctor", "--server", &fake.url()], &[]);
        assert_eq!(r.code, want, "HTTP {status} {code}: {}", r.stderr);
    }
    let ok = health_fake(HEALTH_OK);
    assert_eq!(cortex(&["doctor", "--server", &ok.url()], &[]).code, 0);
    assert_eq!(cortex(&["--nope"], &[]).code, 2);
    // 429 on every attempt: four attempts, three waits of one second (the real sleeper)
    let busy = Fake::start();
    busy.set_default(Reply::problem(429, "busy").header("Retry-After", "1"));
    let r = cortex(&["doctor", "--server", &busy.url()], &[]);
    assert_eq!(r.code, 7, "{}", r.stderr);
    assert_eq!(busy.count(), 4);
}

#[test]
fn ex_03_stdout_is_the_result_and_stderr_the_diagnostic() {
    let ok = health_fake(HEALTH_OK);
    let r = cortex(&["doctor", "--server", &ok.url()], &[]);
    assert!(r.stderr.is_empty() && !r.stdout.is_empty());
    let bad = Fake::start();
    bad.set_default(Reply::problem(404, "not_found"));
    let r = cortex(&["doctor", "--server", &bad.url()], &[]);
    assert!(r.stdout.is_empty(), "{}", r.stdout);
    assert_eq!(lines(&r.stderr).len(), 1, "{}", r.stderr);
}

#[test]
fn ex_04_a_json_failure_is_one_json_line_on_stderr_and_md_is_the_display() {
    let bad = Fake::start();
    bad.set_default(Reply::problem_with(
        404,
        "not_found",
        "Not found",
        Some("no such thing"),
        "Check the id.",
    ));
    let j = cortex(&["doctor", "--format", "json", "--server", &bad.url()], &[]);
    assert_eq!(j.code, 4);
    assert!(j.stdout.is_empty());
    assert_eq!(lines(&j.stderr).len(), 1, "{}", j.stderr);
    let v: serde_json::Value = serde_json::from_str(j.stderr.trim()).expect("one JSON object");
    let e = &v["error"];
    assert_eq!(e["code"], "not_found");
    assert_eq!(e["title"], "Not found");
    assert_eq!(e["status"], 404);
    assert_eq!(e["detail"], "no such thing");
    assert_eq!(e["fix"], "Check the id.");
    assert_eq!(e["request_id"], "0198d2c0-7000-7000-8000-000000000001");
    assert_eq!(e["exit"], 4);
    let md = cortex(&["doctor", "--server", &bad.url()], &[]);
    assert_eq!(
        md.stderr,
        "Not found (not_found, HTTP 404): no such thing Fix: Check the id. [request 0198d2c0-7000-7000-8000-000000000001]\n"
    );
    // a failure with no response still has one JSON line
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let r = cortex(
        &[
            "doctor",
            "--format",
            "json",
            "--server",
            &format!("http://127.0.0.1:{port}"),
        ],
        &[],
    );
    let v: serde_json::Value = serde_json::from_str(r.stderr.trim()).expect("one JSON object");
    assert_eq!(
        (
            r.code,
            v["error"]["code"].as_str(),
            v["error"]["exit"].as_i64()
        ),
        (1, Some("transport"), Some(1))
    );
}

#[test]
fn ex_05_a_local_failure_exits_1_before_any_request() {
    let fake = health_fake(HEALTH_OK);
    let https = fake.url().replace("http://", "https://");
    let runs = [
        cortex(&["doctor", "--server", "http://example.com:80"], &[]),
        cortex(&["doctor", "--server", &https], &[]),
        cortex(&["doctor"], &[("CORTEX_SERVER", "http://10.0.0.5:80")]),
        cortex(&["doctor", "--server", "unix:///run/cortex.sock"], &[]),
    ];
    for r in &runs {
        assert_eq!(r.code, 1, "{}", r.stderr);
        assert!(r.stdout.is_empty());
        assert_eq!(lines(&r.stderr).len(), 1, "{}", r.stderr);
    }
    assert!(runs[0].stderr.contains("loopback"), "{}", runs[0].stderr);
    assert_eq!(fake.connections(), 0, "no connection was made");
}

#[test]
fn rt_05_proxy_variables_are_never_used() {
    let health = health_fake(HEALTH_OK);
    let proxy = Fake::start();
    let p = proxy.url();
    let env: Vec<(&str, &str)> = [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
    ]
    .into_iter()
    .map(|k| (k, p.as_str()))
    .collect();
    let r = cortex(&["doctor", "--server", &health.url()], &env);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(health.count(), 1, "the client connected directly");
    assert_eq!(proxy.connections(), 0, "the fake proxy saw no connection");
    // and with a NO_PROXY that does not exempt the loopback address
    let mut env2 = env.clone();
    env2.push(("NO_PROXY", "example.org"));
    let r = cortex(&["doctor", "--server", &health.url()], &env2);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(proxy.connections(), 0);
}

#[test]
fn sc_01_a_canary_in_every_token_source_is_never_printed_or_sent() {
    let fake = health_fake(HEALTH_OK);
    let url = fake.url();
    let dir = common::support::dir("sc01");
    let file = common::support::write(&dir, "tok", format!("{CANARY}\n").as_bytes(), 0o600);
    let file_s = file.display().to_string();
    let env = [
        ("CORTEX_TOKEN", CANARY),
        ("CORTEX_TOKEN_FILE", file_s.as_str()),
    ];
    let down = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://127.0.0.1:{}", l.local_addr().unwrap().port())
    };
    let bad404 = Fake::start();
    bad404.set_default(Reply::problem_with(
        404,
        "not_found",
        "Not found",
        Some(CANARY),
        "fix",
    ));
    let scenarios: Vec<Vec<String>> = vec![
        vec![
            "doctor".into(),
            "--server".into(),
            url.clone(),
            "--token-file".into(),
            file_s.clone(),
        ],
        vec![
            "doctor".into(),
            "--server".into(),
            url.clone(),
            "--format".into(),
            "json".into(),
        ],
        vec!["doctor".into(), "--server".into(), down.clone()],
        vec![
            "doctor".into(),
            "--server".into(),
            down.clone(),
            "--format".into(),
            "json".into(),
        ],
        vec![
            "doctor".into(),
            "--server".into(),
            "http://example.com:80".into(),
        ],
        vec!["--version".into()],
        vec!["--help".into()],
        vec!["doctr".into()],
        vec!["--nope".into(), CANARY.into()],
    ];
    for args in &scenarios {
        let a: Vec<&str> = args.iter().map(String::as_str).collect();
        let r = cortex(&a, &env);
        assert!(
            !r.stdout.contains(CANARY) && !r.stderr.contains(CANARY),
            "{a:?}: stdout {:?} stderr {:?}",
            r.stdout,
            r.stderr
        );
    }
    // a server that echoes the canary back in a Problem: the Problem is for a TOKEN route, but doctor shows it too
    let r = cortex(&["doctor", "--server", &bad404.url()], &env);
    assert_eq!(r.code, 4);
    for q in fake.requests().iter().chain(bad404.requests().iter()) {
        assert!(!q.head.contains(CANARY), "the canary was sent: {}", q.head);
    }
}

#[test]
fn tf_12_there_is_no_flag_that_takes_the_token_itself() {
    for args in [
        vec!["--token", CANARY, "doctor"],
        vec!["doctor", "--token", CANARY],
    ] {
        let r = cortex(&args, &[]);
        assert_eq!(r.code, 2, "{args:?}: {}", r.stderr);
        assert!(r.stdout.is_empty());
        assert!(!r.stderr.contains(CANARY), "{}", r.stderr);
    }
    let with_value = format!("--token={CANARY}");
    let r = cortex(&[&with_value, "doctor"], &[]);
    assert_eq!(r.code, 2);
    assert!(
        !r.stderr.contains(CANARY),
        "the value of --token= is not echoed: {}",
        r.stderr
    );
    let help = cortex(&["--help"], &[]).stdout;
    assert!(help.contains("--token-file"));
    assert!(
        !help
            .lines()
            .any(|l| l.contains("--token ") || l.trim_end().ends_with("--token")),
        "{help}"
    );
}

// ---- PR-10 through the binary (ellis F-1; cox's decisions of 6 Nov 18:37Z) -------------------------------------------------

fn unsafe_oracle(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{2028}' | '\u{2029}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
                | '\u{61c}' | '\u{e0000}'..='\u{e007f}'
        )
}

fn hostile(tag: &str) -> String {
    format!(
        "{tag}:A\x1b[31mB\rC\x07D\0E\x7fF\u{9b}G\u{2028}H\u{200e}I\u{202e}J\u{2066}K\u{61c}L\u{e0041}M\nN\tO"
    )
}

fn problem_fake(code: &str, request_id: &str, title: &str, detail: &str, fix: &str) -> Fake {
    let body = serde_json::json!({ "type": code, "title": title, "status": 400, "detail": detail, "fix": fix, "request_id": request_id });
    let fake = Fake::start();
    fake.set_default(Reply::http(
        400,
        &[("Content-Type", "application/problem+json")],
        body.to_string().as_bytes(),
    ));
    fake
}

const RID: &str = "0198d2c0-7000-7000-8000-000000000001";

#[test]
fn pr_10_the_md_stderr_of_a_hostile_problem_is_one_safe_bounded_line() {
    let fake = problem_fake(
        "bad_request",
        RID,
        &hostile("T"),
        &hostile("D"),
        &hostile("F"),
    );
    let r = cortex(&["doctor", "--server", &fake.url()], &[]);
    assert_eq!(r.code, 6, "{}", r.stderr);
    assert!(r.stdout.is_empty());
    assert!(
        r.stderr.ends_with('\n') && r.stderr.matches('\n').count() == 1,
        "one line: {:?}",
        r.stderr
    );
    assert!(
        !r.stderr.chars().any(|c| unsafe_oracle(c) && c != '\n'),
        "no unsafe character: {:?}",
        r.stderr
    );
    assert!(
        r.stderr.contains("T:A?[31mB?C?D?E?F?G?H?I?J?K?L?M?N?O"),
        "{:?}",
        r.stderr
    );
    // 5,000 multibyte characters in each free-text member: the line stays small and the cut is visible
    let long = "\u{e9}".repeat(5000);
    let fake = problem_fake("bad_request", RID, &long, &long, &long);
    let r = cortex(&["doctor", "--server", &fake.url()], &[]);
    assert!(
        r.stderr.chars().count() < 2000,
        "bounded: {} characters",
        r.stderr.chars().count()
    );
    assert!(
        r.stderr.contains(&format!("{}...", "\u{e9}".repeat(197))),
        "the title cut, visible"
    );
    assert!(
        r.stderr.contains(&format!("{}...", "\u{e9}".repeat(997))),
        "the detail cut"
    );
    assert!(
        r.stderr.contains(&format!("{}...", "\u{e9}".repeat(497))),
        "the fix cut"
    );
}

#[test]
fn pr_10_the_json_line_is_lossless_safe_and_bounded() {
    let fake = problem_fake(
        "bad_request",
        RID,
        &hostile("T"),
        &hostile("D"),
        &hostile("F"),
    );
    let r = cortex(
        &["doctor", "--format", "json", "--server", &fake.url()],
        &[],
    );
    assert_eq!(r.code, 6, "{}", r.stderr);
    assert!(r.stdout.is_empty());
    assert!(
        r.stderr.ends_with('\n') && r.stderr.matches('\n').count() == 1,
        "one line: {:?}",
        r.stderr
    );
    assert!(
        !r.stderr.chars().any(|c| unsafe_oracle(c) && c != '\n'),
        "no raw unsafe character (DEL, C1 and the format characters are escapes): {:?}",
        r.stderr
    );
    let v: serde_json::Value = serde_json::from_str(r.stderr.trim()).expect("one JSON object");
    let e = &v["error"];
    assert_eq!(
        e["title"],
        hostile("T").as_str(),
        "lossless: the escapes parse back to the sent text"
    );
    assert_eq!(e["detail"], hostile("D").as_str());
    assert_eq!(e["fix"], hostile("F").as_str());
    assert_eq!(
        (
            e["code"].as_str(),
            e["request_id"].as_str(),
            e["exit"].as_i64()
        ),
        (Some("bad_request"), Some(RID), Some(6))
    );
    // over the budgets: cut by characters with the visible marker, still one safe line
    let long = "\u{e9}".repeat(5000);
    let fake = problem_fake("bad_request", RID, &long, &long, &long);
    let r = cortex(
        &["doctor", "--format", "json", "--server", &fake.url()],
        &[],
    );
    let v: serde_json::Value = serde_json::from_str(r.stderr.trim()).expect("one JSON object");
    for (member, budget) in [("title", 200), ("detail", 1000), ("fix", 500)] {
        let t = v["error"][member].as_str().unwrap();
        assert_eq!(t.chars().count(), budget, "{member}");
        assert!(t.ends_with("..."), "{member}");
    }
}

#[test]
fn pr_10_a_hostile_type_or_request_id_is_not_a_problem_and_prints_safely() {
    for (what, code, rid) in [
        ("type", "bad\x1b[2J", RID),
        ("request_id", "bad_request", "id\u{9b}x"),
    ] {
        let fake = problem_fake(code, rid, "T", "D", "F");
        for format in ["md", "json"] {
            let r = cortex(
                &["doctor", "--format", format, "--server", &fake.url()],
                &[],
            );
            assert_eq!(r.code, 6, "{what} {format}: {}", r.stderr);
            assert!(
                r.stderr.matches('\n').count() == 1,
                "{what} {format}: {:?}",
                r.stderr
            );
            assert!(
                !r.stderr.chars().any(|c| unsafe_oracle(c) && c != '\n'),
                "{what} {format}: {:?}",
                r.stderr
            );
        }
    }
}

// ---- R-1 (ellis, cox's batch of 7 Nov): `doctor` shows the Health strings through the ONE list and a budget --------------------

fn health_with(state: &str, version: &str, piece: &str, dump: &str) -> String {
    serde_json::json!({
        "state": state, "version": version, "ready": true,
        "pieces": [{ "name": piece, "enabled": true, "ready": true }],
        "last_good_dump_at": dump, "backup": "ok"
    })
    .to_string()
}

#[test]
fn r1_doctor_md_shows_health_strings_without_unsafe_characters_and_cut_to_a_budget() {
    let fake = health_fake(&health_with(
        &hostile("S"),
        &hostile("V"),
        &hostile("P"),
        &hostile("D"),
    ));
    let r = cortex(&["doctor", "--server", &fake.url()], &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stderr.is_empty(), "{}", r.stderr);
    assert!(
        !r.stdout.chars().any(|c| unsafe_oracle(c) && c != '\n'),
        "no unsafe character: {:?}",
        r.stdout
    );
    let shown = "?[31mB?C?D?E?F?G?H?I?J?K?L?M?N?O";
    for line in [
        format!("state: S:A{shown}\n"),
        format!("version: V:A{shown}\n"),
        format!("last_good_dump_at: D:A{shown}\n"),
        format!("  P:A{shown}: enabled=true ready=true\n"),
    ] {
        assert!(
            r.stdout.contains(&line),
            "missing {line:?} in {:?}",
            r.stdout
        );
    }
    // 5,000 characters in each: every value is cut to 64 characters with the visible marker
    let long = "\u{e9}".repeat(5000);
    let fake = health_fake(&health_with(&long, &long, &long, &long));
    let r = cortex(&["doctor", "--server", &fake.url()], &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let cut = format!("{}...", "\u{e9}".repeat(61));
    for line in [
        format!("state: {cut}\n"),
        format!("version: {cut}\n"),
        format!("last_good_dump_at: {cut}\n"),
        format!("  {cut}: enabled=true ready=true\n"),
    ] {
        assert!(
            r.stdout.contains(&line),
            "missing a cut line in {:?}",
            r.stdout
        );
    }
    assert!(
        r.stdout.chars().count() < 500,
        "bounded: {} characters",
        r.stdout.chars().count()
    );
}

#[test]
fn r1_doctor_json_still_prints_the_body_as_received_ellis_f4_is_not_in_this_batch() {
    let body = health_with("ready", "0.2.1", "identity", "2026-11-06T10:00:00Z");
    let fake = health_fake(&body);
    let r = cortex(
        &["doctor", "--format", "json", "--server", &fake.url()],
        &[],
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout, format!("{body}\n"));
}
