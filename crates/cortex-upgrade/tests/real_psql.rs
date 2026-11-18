//! Piece 14, real ports: `RealPsql` (brief r1 section 3, "Psql::trust_rule_count"). R1, R3, R4, R6, R8 through a fake
//! `podman`.
#[path = "real_fakes.rs"]
mod fakes;

use cortex_upgrade::ports::{NoAnswer, Psql};
use cortex_upgrade::real::RealPsql;
use fakes::{Harness, SQL};
use std::time::Duration;

fn psql(h: &Harness) -> RealPsql {
    RealPsql::with(
        h.podman.as_os_str(),
        Duration::from_secs(20),
        h.release.clone(),
    )
}

#[test]
fn rp_01_the_count_is_asked_with_exactly_this_vector_and_the_answer_is_the_trimmed_stdout() {
    let h = Harness::new("rp1", "printf '  0 \\n'\n");
    assert_eq!(psql(&h).trust_rule_count(), Ok("0".to_string()));
    let calls = h.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].argv,
        [
            "exec",
            "-i",
            "--user",
            "postgres",
            &h.ctr("db"),
            "psql",
            "-U",
            "postgres",
            "-d",
            "cortex",
            "-tAc",
            SQL
        ]
    );
}

#[test]
fn rp_02_an_answer_over_64_bytes_is_no_answer_and_is_never_shortened() {
    let long: String = (0..200)
        .map(|i| char::from(b'a' + (i % 26) as u8))
        .collect();
    // 65 bytes is over; 64 is the most that passes, whole
    let h = Harness::new("rp2", &format!("echo '{}'\n", &long[..65]));
    assert_eq!(psql(&h).trust_rule_count(), Err(NoAnswer));
    let h = Harness::new("rp2a", &format!("echo '{long}'\n"));
    assert_eq!(psql(&h).trust_rule_count(), Err(NoAnswer));
    let h = Harness::new("rp2b", &format!("echo '{}'\n", &long[..64]));
    assert_eq!(psql(&h).trust_rule_count().unwrap(), long[..64]);
    // not UTF-8: replaced, still bounded and never a failure to answer
    let h = Harness::new("rp2c", "printf '3\\377\\n'\n");
    let got = psql(&h).trust_rule_count().unwrap();
    assert!(got.starts_with('3') && got.len() <= 64 + 3, "{got:?}");
}

#[test]
fn rp_08_a_poisoned_tail_is_no_answer_and_a_valid_zero_is_still_zero() {
    // mike's case: `0`, 63 spaces, `1` is 65 bytes after trimming. Shortened to 64 it would read `0` once trimmed.
    let poisoned = format!("0{}1", " ".repeat(63));
    let h = Harness::new("rp8", &format!("printf '{poisoned}\\n'\n"));
    assert_eq!(psql(&h).trust_rule_count(), Err(NoAnswer));
    // the same shape at exactly 64 bytes is returned whole (the engine judges the text, it does not trust it)
    let at_64 = format!("0{}1", " ".repeat(62));
    let h = Harness::new("rp8a", &format!("printf '{at_64}\\n'\n"));
    assert_eq!(psql(&h).trust_rule_count(), Ok(at_64));
    // a valid zero, with and without the padding psql and the shell add
    for out in ["0\\n", "  0  \\n", "0"] {
        let h = Harness::new("rp8b", &format!("printf '{out}'\n"));
        assert_eq!(psql(&h).trust_rule_count(), Ok("0".to_string()), "{out:?}");
    }
}

#[test]
fn rp_09_an_answer_that_is_not_all_there_is_is_no_answer() {
    // A `0` first and 1.5 MiB of blank lines, then a `1` after the 1 MiB the runner keeps: the kept bytes trim to a valid
    // `0`, but the child said more.
    let h = Harness::new(
        "rp9",
        "echo 0\nhead -c 1572864 /dev/zero | tr '\\0' '\\n'\necho 1\n",
    );
    assert_eq!(psql(&h).trust_rule_count(), Err(NoAnswer));
}

#[test]
fn rp_03_anything_but_exit_zero_is_no_answer() {
    for body in [
        "echo 5; exit 1",
        "echo 5; exit 2",
        "echo 5 >&2; exit 125",
        "exit 127",
    ] {
        let h = Harness::new("rp3", body);
        assert_eq!(psql(&h).trust_rule_count(), Err(NoAnswer), "{body}");
    }
}

#[test]
fn rp_04_a_database_container_that_has_no_valid_name_is_no_answer_and_nothing_is_started() {
    let h = Harness::new("rp4", "echo 0\n");
    h.set_compose("services: {}\n");
    assert_eq!(psql(&h).trust_rule_count(), Err(NoAnswer));
    assert!(h.calls().is_empty());
}

#[test]
fn rp_05_a_program_that_cannot_start_is_no_answer() {
    let h = Harness::new("rp5", "echo 0\n");
    let p = RealPsql::with(
        "/nonexistent-dir-for-the-cell/podman",
        Duration::from_secs(5),
        h.release.clone(),
    );
    assert_eq!(p.trust_rule_count(), Err(NoAnswer));
}

#[test]
fn rp_06_a_slow_database_is_no_answer_within_the_deadline() {
    let h = Harness::new("rp6", "sleep 30\n");
    let p = RealPsql::with(
        h.podman.as_os_str(),
        Duration::from_millis(700),
        h.release.clone(),
    );
    let t = std::time::Instant::now();
    assert_eq!(p.trust_rule_count(), Err(NoAnswer));
    assert!(
        t.elapsed() < Duration::from_millis(700) + Duration::from_secs(1),
        "{:?}",
        t.elapsed()
    );
}

#[test]
fn rp_07_the_production_deadline_is_30_seconds() {
    // The constructor the engine uses: 30 s as the brief says. (The cell cannot wait 30 s; it reads the Debug text.)
    let p = RealPsql::new(std::path::PathBuf::from("/x/releases/1"));
    let shown = format!("{p:?}");
    assert!(
        shown.contains("30s") || shown.contains("secs: 30"),
        "{shown}"
    );
}
