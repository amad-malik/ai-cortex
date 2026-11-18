//! ID-01, ID-02, ID-56 (job roles), ID-57 (one validator, one table): the label grammar.
//! DB-free. RED: the validators are `todo!()`.
mod common;

use common::{Shape, twin};
use mod_identity::labels::{
    JOB_ROLES_MAX, JobRolesError, job_role_valid, job_roles_check, name_key, name_valid,
};

fn long(n: usize) -> String {
    "a".repeat(n)
}

#[test]
fn id_01_accepted_names_and_the_32_boundary() {
    let thirty_two = format!("a{}x", "B9-".repeat(10)); // 1 + 30 + 1
    assert_eq!(thirty_two.len(), 32);
    for ok in [
        "sophi",
        "Sophi",
        "a",
        "A",
        "a1-b2",
        "a-",
        "a--b",
        "KAI",
        "x9",
        &long(32),
        &thirty_two,
    ] {
        assert!(name_valid(ok), "{ok:?} is a valid name");
    }
}

#[test]
fn id_02_refused_names_none_is_a_name() {
    let refused: Vec<String> = vec![
        String::new(),
        long(33),
        long(1000),
        "1abc".into(),
        "-abc".into(),
        "ab_c".into(),
        "ab c".into(),
        "ab.c".into(),
        "sophi\n".into(), // the `$` trap (F-11): the check must match to the END only
        "sophi\r\n".into(),
        " sophi".into(),
        "sophi ".into(),
        "\u{0455}ophi".into(), // Cyrillic dze
        "\u{FF41}bc".into(),   // full-width a
        "a\u{1F600}".into(),
        "\u{212A}evin".into(), // Kelvin sign: lower() maps it to ASCII k (F-10)
        "a\0b".into(),
        "caf\u{e9}".into(),
    ];
    for bad in &refused {
        assert!(!name_valid(bad), "{bad:?} must be refused");
        assert_eq!(name_key(bad), None, "{bad:?} has no key");
    }
}

#[test]
fn id_03_the_key_is_the_ascii_lowercase_of_a_valid_name() {
    assert_eq!(name_key("Sophi").as_deref(), Some("sophi"));
    assert_eq!(name_key("SOPHI").as_deref(), Some("sophi"));
    assert_eq!(name_key("a-B9").as_deref(), Some("a-b9"));
    assert_eq!(name_key("KAI").as_deref(), Some("kai"));
    assert_eq!(name_key(&long(32)).as_deref(), Some(long(32).as_str()));
}

#[test]
fn id_56_job_role_boundary_pairs() {
    for ok in ["dev", "qa", "a", "a-", "a1", "x9-y", &long(32)] {
        assert!(job_role_valid(ok), "{ok:?} is a valid job role");
    }
    for bad in [
        "",
        "QA",
        "Dev",
        "1qa",
        "_a",
        "a_b",
        "a b",
        "a,b",
        "dev\n",
        "\u{e9}a",
        "a\0",
        &long(33),
    ] {
        assert!(!job_role_valid(bad), "{bad:?} must be refused");
    }
}

fn v(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn id_12b_job_role_lists_grammar_duplicates_and_the_count() {
    assert_eq!(job_roles_check(&[]), Ok(()));
    assert_eq!(job_roles_check(&v(&["dev", "qa"])), Ok(()));
    assert_eq!(job_roles_check(&v(&["qa", "dev"])), Ok(()));
    let eight: Vec<String> = (0..JOB_ROLES_MAX).map(|i| format!("r{i}")).collect();
    assert_eq!(job_roles_check(&eight), Ok(()), "8 roles are accepted");
    let nine: Vec<String> = (0..=JOB_ROLES_MAX).map(|i| format!("r{i}")).collect();
    assert_eq!(job_roles_check(&nine), Err(JobRolesError::TooMany));
    assert_eq!(
        job_roles_check(&v(&["dev", "dev"])),
        Err(JobRolesError::Duplicate)
    );
    assert_eq!(
        job_roles_check(&v(&["dev", "qa", "dev"])),
        Err(JobRolesError::Duplicate)
    );
    // F-12: ONE element that holds a comma is two roles to a joined-string check; here it is refused.
    assert_eq!(
        job_roles_check(&v(&["qa,dev"])),
        Err(JobRolesError::Grammar)
    );
    assert_eq!(job_roles_check(&v(&["QA"])), Err(JobRolesError::Grammar));
    assert_eq!(job_roles_check(&v(&[""])), Err(JobRolesError::Grammar));
}

/// The strings the differential test feeds both sides: every string over a small alphabet up to
/// length 3, plus boundary lengths 30..=34 over each alphabet character, plus fixed traps.
fn domain() -> Vec<String> {
    let alphabet = [
        'a', 'z', 'A', 'Z', '0', '9', '-', '_', ' ', '\n', '\u{e9}', '\u{212A}',
    ];
    let mut out = vec![String::new()];
    let mut frontier = vec![String::new()];
    for _ in 0..3 {
        let mut next = Vec::new();
        for s in &frontier {
            for c in alphabet {
                let mut t = s.clone();
                t.push(c);
                next.push(t);
            }
        }
        out.extend(next.iter().cloned());
        frontier = next;
    }
    for len in 30..=34 {
        for c in alphabet {
            out.push(format!("a{}", c.to_string().repeat(len - 1)));
            out.push(format!("{}b", "a".repeat(len - 1)));
        }
    }
    out.extend([
        "sophi\n".into(),
        "a\0b".into(),
        "Kevin".into(),
        "\u{212A}evin".into(),
    ]);
    out
}

#[test]
fn id_57_one_validator_one_table_the_code_equals_the_twins_grammar() {
    let t = twin();
    let g = &t["x-label-grammar"];
    let name_pat = g["name"]["pattern"].as_str().expect("name pattern");
    let role_pat = g["job_role"]["pattern"].as_str().expect("job_role pattern");
    // A change of the grammar in S3 must fail here and force a re-read of the code.
    assert_eq!(name_pat, "^[A-Za-z][A-Za-z0-9-]{0,31}$");
    assert_eq!(role_pat, "^[a-z][a-z0-9-]{0,31}$");
    assert_eq!(g["name"]["maxLength"], 32);
    assert_eq!(g["job_role"]["maxLength"], 32);
    let name_shape = Shape::from_pattern(name_pat);
    let role_shape = Shape::from_pattern(role_pat);
    let mut n = 0;
    for s in domain() {
        assert_eq!(
            name_valid(&s),
            name_shape.matches(&s),
            "name differs from the twin's pattern for {s:?}"
        );
        assert_eq!(
            job_role_valid(&s),
            role_shape.matches(&s),
            "job role differs from the twin's pattern for {s:?}"
        );
        n += 1;
    }
    assert!(n > 2000, "the differential domain is large enough: {n}");
}
