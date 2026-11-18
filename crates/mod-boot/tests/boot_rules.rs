//! BOOT-20 to BOOT-26 (`mvp/tests/mod-boot.md`, Rules; kai's ruling 2a7de258, option A). DB-free.
mod support;

use cortex_module::SectionName;
use mod_boot::seam::gather;
use mod_boot::{Inputs, assemble};
use support::*;

const FIX: &str = "(the project lead runs `cortex rules set`)";

fn rules_text(rec: u64, revision: i32, path: &str, text: &str) -> cortex_module::SectionView {
    let mut s = rules(rec, revision, path);
    s.body = text.to_owned();
    s
}

#[test]
fn boot_20_two_pointer_lines_and_no_rules_text() {
    let i = Inputs {
        rules_project: Some(rules_text(
            2,
            7,
            "/path/to/docs/a/RULES.md",
            "RULE-TEXT-PROJECT",
        )),
        rules_worker: Some(rules_text(
            3,
            1,
            "/path/to/docs/a/rules-w1.md",
            "RULE-TEXT-W1",
        )),
        ..inputs_w1()
    };
    let b = assemble(&i, 12_288).expect("fits");
    assert_eq!(
        section_text(&b.markdown, 3),
        "## 3 Rules\nproject: v7 sha256:020202020202 /path/to/docs/a/RULES.md\n\
         yours: v1 sha256:030303030303 /path/to/docs/a/rules-w1.md\n\
         read them before a task: cortex rules\npersona: none\n"
    );
    let json = serde_json::to_string(&b.boot).expect("json");
    for out in [&b.markdown, &json] {
        assert!(!out.contains("RULE-TEXT"), "no rules text in boot");
    }
    let p = &b.boot.rules;
    assert_eq!(p.len(), 2);
    assert_eq!(
        (
            p[0].section.as_str(),
            p[0].revision,
            p[0].sha256.as_str(),
            p[0].source_path.as_deref()
        ),
        (
            "rules-project",
            7,
            "02".repeat(32).as_str(),
            Some("/path/to/docs/a/RULES.md")
        )
    );
    assert_eq!(p[1].section, format!("rules-worker:{}", w1()));
    assert_eq!((p[1].revision, p[1].sha256.clone()), (1, "03".repeat(32)));
    assert_eq!(p[0].record_id, record(2).to_string());
    assert_eq!(b.boot.sections[2].status, "ok");
}

#[test]
fn boot_21_neither_layer_set() {
    let i = Inputs {
        rules_project: None,
        rules_worker: None,
        ..inputs_w1()
    };
    let b = assemble(&i, 12_288).expect("fits");
    assert_eq!(
        section_text(&b.markdown, 3),
        format!(
            "## 3 Rules\nproject: none {FIX}\nyours: none {FIX}\nread them before a task: cortex rules\npersona: none\n"
        )
    );
    assert!(b.boot.rules.is_empty());
    assert_eq!(b.boot.sections[2].status, "none");
}

#[test]
fn boot_22_a_new_revision_replaces_the_old_pointer() {
    let mut i = inputs_w1();
    let mut r = rules(2, 8, "/path/to/docs/a/RULES.md");
    r.body_sha256 = sha(9);
    i.rules_project = Some(r);
    let b = assemble(&i, 12_288).expect("fits");
    let s3 = section_text(&b.markdown, 3);
    assert!(s3.contains("project: v8 sha256:090909090909 /path/to/docs/a/RULES.md\n"));
    assert!(!s3.contains("v7"));
}

/// Only W2's layer is set; W1's boot reads its own layer only.
#[tokio::test]
async fn boot_23_another_workers_layer_never_appears() {
    let mut w = world();
    w.a.sections.push((
        SectionName::RulesWorker(w2()),
        rules(4, 3, "/path/to/docs/a/rules-W2-ONLY.md"),
    ));
    let mut r = Reads::for_project(&w, project_a());
    let i = gather(&mut r, &principal_w1()).await.expect("gathers");
    assert!(i.rules_worker.is_none());
    let b = assemble(&i, 12_288).expect("fits");
    assert!(section_text(&b.markdown, 3).contains(&format!("yours: none {FIX}\n")));
    let json = serde_json::to_string(&b.boot).expect("json");
    for out in [&b.markdown, &json] {
        assert!(!out.contains("W2-ONLY"));
        assert!(!out.contains(&format!("rules-worker:{}", w2())));
    }
}

#[test]
fn boot_24_an_invalidated_project_layer_shows_none() {
    let i = Inputs {
        rules_project: None,
        ..inputs_w1()
    };
    let b = assemble(&i, 12_288).expect("fits");
    let s3 = section_text(&b.markdown, 3);
    assert!(s3.starts_with(&format!("## 3 Rules\nproject: none {FIX}\nyours: v1 ")));
    assert_eq!(b.boot.rules.len(), 1);
    assert_eq!(b.boot.sections[2].status, "ok");
}

#[test]
fn boot_25_persona_is_none_and_not_in_the_json() {
    let b = assemble(&inputs_w1(), 12_288).expect("fits");
    assert!(section_text(&b.markdown, 3).ends_with("persona: none\n"));
    let json = serde_json::to_string(&b.boot).expect("json");
    assert!(!json.contains("persona"));
}

/// The server reads no files (S2 §3.5): a source path that names nothing still shows the stored
/// revision and hash.
#[test]
fn boot_26_a_missing_source_file_changes_nothing() {
    let gone = "/nonexistent/mod-boot-26/RULES.md";
    assert!(!std::path::Path::new(gone).exists());
    let i = Inputs {
        rules_project: Some(rules(2, 7, gone)),
        ..inputs_w1()
    };
    let b = assemble(&i, 12_288).expect("fits");
    assert!(
        section_text(&b.markdown, 3).contains(&format!("project: v7 sha256:020202020202 {gone}\n"))
    );
    assert_eq!(b.boot.rules[0].source_path.as_deref(), Some(gone));
}
