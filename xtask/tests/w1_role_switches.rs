//! Wave 1 ports (d), W1-11 (cox): "a test that NO module id can reach" the host role. The role is named in
//! `cortex-db` only; a module transaction's role is always `cortex_m_<module>` (proved in `cortex-db`). The
//! remaining way to the role would be a piece's own SQL: `SET ROLE cortex_install_admin`, or
//! `set_config('role', ...)`, since `cortex_app` is a member of every role (S1 section 9). So product source
//! outside `cortex-db` (and `cortex-testkit`, which provisions the roles) may not name the host role or switch
//! the role itself. Same boundary as the other source rules: `src/` of every crate and `build.rs`; `tests/`,
//! `benches/`, `examples/` exempt; comments stripped; the text IS read inside string literals, because the SQL
//! is a string.
//!
//! Limits (stated): a role name assembled at run time (`format!`, concatenation) is not seen.
//!
//! RED: the rule does not exist, so the bad fixture reports nothing.
use std::path::{Path, PathBuf};

use xtask::{Rule, Violation, fitness};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/fitness")
        .join(name)
        .join("Cargo.toml")
}

fn v(krate: &str, depends_on: &str) -> Violation {
    Violation {
        rule: Rule::RoleSwitchOutsideCortexDb,
        krate: krate.into(),
        depends_on: depends_on.into(),
    }
}

#[test]
fn w1_rs_1_cortex_db_the_testkit_tests_comments_and_look_alikes_are_not_flagged() {
    assert_eq!(fitness(&fixture("role-ok")).expect("fitness runs"), vec![]);
}

#[test]
fn w1_rs_2_a_piece_naming_the_host_role_or_switching_the_role_is_found() {
    assert_eq!(
        fitness(&fixture("role-bad")).expect("fitness runs"),
        vec![
            v("mod-identity", "SET ROLE (mod-identity/src/t.rs:2)"),
            v(
                "mod-identity",
                "cortex_install_admin (mod-identity/src/lib.rs:1)"
            ),
            v("mod-policy", "RESET ROLE (mod-policy/src/other.rs:1)"),
            v("mod-policy", "SET LOCAL ROLE (mod-policy/src/lib.rs:1)"),
            v(
                "mod-tokens",
                "SET SESSION AUTHORIZATION (mod-tokens/build.rs:1)"
            ),
            v("mod-tokens", "set_config('role' (mod-tokens/src/lib.rs:2)"),
        ],
        "the role name, SET ROLE in a #[cfg(test)] module in src/, RESET ROLE, SET LOCAL ROLE split over lines, \
         SET SESSION AUTHORIZATION in build.rs and set_config('role' are found; the comment, cortex-db, \
         cortex-testkit, tests/, benches/ and examples/ are not"
    );
}

#[test]
fn w1_rs_3_this_workspace_names_the_host_role_only_in_cortex_db_and_the_testkit() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .join("Cargo.toml");
    assert_eq!(fitness(&root).expect("fitness runs"), vec![]);
}

/// GREEN-side cell (added with the rule, not part of the RED commit): the spellings a piece could use. The rule
/// reads string CONTENT, so it decodes the escapes a Rust string allows (`\n`, `\'`, `\x27`, `\u{27}`), joins a
/// string continuation, reads raw and byte strings, tolerates spaces around the punctuation of
/// `set_config('role'`, and matches at identifier boundaries. **Stated limit (`concat!`):** the macro's
/// separate literals are not joined across the comma, so `concat!("SET ", "LOCAL ROLE x")` (line 11) is NOT
/// found; neither is a name assembled at run time.
#[test]
fn w1_rs_4_the_spellings_are_read_and_the_look_alikes_and_the_stated_limit_are_not() {
    let want = |what: &str, line: usize| Violation {
        rule: Rule::RoleSwitchOutsideCortexDb,
        krate: "mod-forms".into(),
        depends_on: format!("{what} (mod-forms/src/lib.rs:{line})"),
    };
    let mut expected = vec![
        want("SET ROLE", 1),
        want("set_config('role'", 2),
        want("set_config('role'", 3),
        want("set_config('role'", 4),
        want("set_config('role'", 5),
        want("SET ROLE", 6),
        want("RESET ROLE", 7),
        want("SET ROLE", 9),
        want("SET SESSION ROLE", 12),
        want("RESET SESSION AUTHORIZATION", 12),
        want("cortex_install_admin", 13),
        want("SET LOCAL SESSION AUTHORIZATION", 19),
        // vera's Low 1: a static SQL key in E'..' or dollar quotes is the same key.
        want("set_config('role'", 20),
        want("set_config('role'", 21),
        want("set_config('role'", 22),
        // vera's Low 2: a newline written as \x0a inside a string is not a source line; the next SET ROLE is on 24.
        want("SET ROLE", 23),
        want("SET ROLE", 24),
    ];
    expected.sort();
    assert_eq!(
        fitness(&fixture("role-forms")).expect("fitness runs"),
        expected,
        "raw string, escaped quote, \\x27, \\u{{27}}, spaced punctuation, \\n, a continuation, a word split by a \
         continuation, session forms and a byte string are found; line 11 (concat!) and the look-alikes on \
         lines 14 to 18 are not (line 19 is the `SET LOCAL SESSION AUTHORIZATION` form)"
    );
}
