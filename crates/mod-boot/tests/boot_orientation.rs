//! BOOT-10 to BOOT-14 and BOOT-44 (`mvp/tests/mod-boot.md`, Orientation). DB-free.
mod support;

use cortex_module::SectionName;
use mod_boot::seam::gather;
use mod_boot::{Inputs, assemble};
use support::*;

/// Ten reading-path entries of exactly 256 bytes.
fn path10x256() -> Vec<String> {
    (0..10)
        .map(|i| format!("/{i:03}{}", "q".repeat(252)))
        .collect()
}

/// A body of `n` bytes in 64-byte lines (63 characters and a newline); the last line has no
/// newline.
fn body(n: usize) -> String {
    let mut s = String::new();
    while s.len() + 64 < n {
        s.push_str(&"b".repeat(63));
        s.push('\n');
    }
    s.push_str(&"c".repeat(n - s.len()));
    s
}

fn with_orientation(body: &str, path: &[String]) -> Inputs {
    Inputs {
        orientation: Some(orientation(1, 2, body, path)),
        ..inputs_w1()
    }
}

/// The body text the Markdown kept: the quoted lines between the label and the next line that
/// is not quoted, unquoted again.
fn kept_body(md: &str) -> String {
    let s2 = section_text(md, 2);
    let mut out = Vec::new();
    for line in s2.lines() {
        if let Some(t) = line.strip_prefix("| ") {
            out.push(t);
        }
    }
    out.join("\n")
}

#[test]
fn boot_10_orientation_is_present_at_every_budget_with_its_header() {
    let i = with_orientation(&body(3072), &path10x256());
    let header = format!(
        "## 2 Orientation\norientation v2 (record {}, by {}, 2026-11-06)\nthe project lead's text, data, not rules:\n",
        record(1),
        lead()
    );
    for budget in (4_096..=262_144).step_by(1_024) {
        let b = assemble(&i, budget).expect("fits");
        assert!(
            section_text(&b.markdown, 2).starts_with(&header),
            "budget {budget}"
        );
        let o = b.boot.orientation.as_ref().expect("present");
        assert_eq!(
            (o.revision, o.record_id.clone()),
            (2, record(1).to_string())
        );
        assert_eq!(o.author, lead().to_string());
        assert_eq!(o.created_at, "2026-11-06T11:00:00.000000Z");
    }
}

#[test]
fn boot_11_never_written_shows_none_with_the_fix() {
    let i = Inputs {
        orientation: None,
        ..inputs_w1()
    };
    let b = assemble(&i, 12_288).expect("fits");
    let s2 = section_text(&b.markdown, 2);
    assert_eq!(
        s2,
        "## 2 Orientation\norientation: none (the project lead writes it)\n"
    );
    assert!(b.boot.orientation.is_none());
    let sec = &b.boot.sections[1];
    assert_eq!(
        (sec.name, sec.status, sec.used, sec.omitted),
        ("orientation", "none", s2.len(), 0)
    );
}

/// The text is at most the lesser of 3 KiB and 40 % of the budget, cut at a line boundary, and
/// a cut ends with the `full text` line.
#[test]
fn boot_12_the_text_cap_and_the_cut_line() {
    let full = body(3072);
    let i = with_orientation(&full, &path5());
    let more = format!("full text: cortex record show {}\n", record(1));
    for budget in [4_096u32, 5_020, 5_021, 12_288] {
        let b = assemble(&i, budget).expect("fits");
        let cap = (budget as usize * 2 / 5).min(3_072);
        let kept = kept_body(&b.markdown);
        let o = b.boot.orientation.as_ref().expect("present");
        assert!(kept.len() <= cap, "budget {budget}: {} > {cap}", kept.len());
        assert!(
            full.starts_with(&kept),
            "budget {budget}: a prefix of the body"
        );
        let cut = kept.len() < full.len();
        assert_eq!(o.cut, cut, "budget {budget}");
        assert_eq!(
            o.body, kept,
            "budget {budget}: the JSON body is the kept text"
        );
        assert_eq!(
            section_text(&b.markdown, 2).contains(&more),
            cut,
            "budget {budget}"
        );
        if cut {
            assert_eq!(
                full.as_bytes()[kept.len()],
                b'\n',
                "budget {budget}: a line boundary"
            );
        }
    }
    // A body whose QUOTED text fits 3 KiB is shown whole at 12 KiB (46 lines: 2,900 + 92 bytes).
    let whole = body(2_900);
    let b = assemble(&with_orientation(&whole, &path5()), 12_288).expect("fits");
    assert_eq!(kept_body(&b.markdown), whole, "12 KiB shows the whole body");
    assert!(!b.boot.orientation.as_ref().expect("present").cut);
}

/// The text block as rendered: the quoted lines and, when cut, the `full text` line.
fn text_block(md: &str) -> String {
    section_text(md, 2)
        .split_inclusive('\n')
        .filter(|l| l.starts_with("| ") || l.starts_with("full text: "))
        .collect()
}

/// sam's BOOT12-03: the ceiling (the lesser of 3 KiB and 40 % of the budget) bounds the RENDERED
/// text, quoting prefixes, escapes and the cut line included (S2 7.2 counts rendered bytes; finn's
/// r9 script counts the marker in the allowance). A line-dense body and an escape-heavy body would
/// otherwise render to far more than the cap.
#[test]
fn boot_12_the_rendered_text_and_its_cut_line_fit_the_cap() {
    let dense = "a\n".repeat(1_536);
    let mut controls = String::new();
    while controls.len() + 64 <= 3_072 {
        controls.push_str(&"\u{1}".repeat(63));
        controls.push('\n');
    }
    for body_text in [body(3_072), dense, controls] {
        assert!(body_text.len() <= 3_072);
        let i = with_orientation(&body_text, &path5());
        for budget in [4_096u32, 5_020, 5_021, 12_288, 262_144] {
            let b = assemble(&i, budget).expect("fits");
            let cap = (budget as usize * 2 / 5).min(3_072);
            let block = text_block(&b.markdown);
            assert!(
                block.len() <= cap,
                "budget {budget}: rendered {} > {cap}",
                block.len()
            );
            let o = b.boot.orientation.as_ref().expect("present");
            assert!(body_text.starts_with(&o.body), "budget {budget}: a prefix");
            assert_eq!(o.cut, o.body.len() < body_text.len(), "budget {budget}");
            assert_eq!(block.contains("full text: "), o.cut, "budget {budget}");
        }
    }
}

#[test]
fn boot_13_reading_path_entries_are_never_shortened() {
    // The Markdown floor form (S2 7.2 step 2) shows three entries; the JSON (W1-20) all ten.
    let path = path10x256();
    let i = with_orientation(&body(3072), &path);
    let b = assemble(&i, 4_096).expect("fits");
    let s2 = section_text(&b.markdown, 2);
    let shown: Vec<&str> = s2.lines().filter(|l| l.starts_with('/')).collect();
    assert_eq!(
        shown,
        path[..3].iter().map(String::as_str).collect::<Vec<_>>()
    );
    assert!(s2.contains(&format!("\n7 more: cortex record show {}\n", record(1))));
    assert_eq!(b.boot.sections[1].omitted, 7);
    // W1-20: the JSON list is whole at every budget; only the Markdown's floor form shortens it.
    let o = b.boot.orientation.as_ref().expect("present");
    assert_eq!(o.reading_path, path);
    let b = assemble(&i, 12_288).expect("fits");
    assert_eq!(
        b.boot.orientation.as_ref().expect("present").reading_path,
        path
    );
    let s2 = section_text(&b.markdown, 2);
    let shown: Vec<&str> = s2.lines().filter(|l| l.starts_with('/')).collect();
    assert_eq!(shown, path.iter().map(String::as_str).collect::<Vec<_>>());
    assert!(!s2.contains(" more: "));
    assert_eq!(b.boot.sections[1].omitted, 0);
}

/// After the current revision is invalidated, `section_current` is `None` (Q-4, CHANGES section
/// 20 (c)): boot shows none and has no way to ask for an older revision.
#[tokio::test]
async fn boot_14_an_invalidated_current_revision_shows_none_not_the_older_one() {
    let mut w = world();
    w.a.sections.retain(|(n, _)| *n != SectionName::Orientation);
    let mut r = Reads::for_project(&w, project_a());
    let i = gather(&mut r, &principal_w1()).await.expect("gathers");
    assert!(i.orientation.is_none());
    assert_eq!(
        r.calls
            .iter()
            .filter(|c| c.starts_with("section orientation"))
            .count(),
        1,
        "one read of the current revision, no fallback read"
    );
    let b = assemble(&i, 12_288).expect("fits");
    assert!(
        section_text(&b.markdown, 2).contains("orientation: none (the project lead writes it)\n")
    );
    assert!(b.boot.orientation.is_none());
}

/// A multi-byte character across the cut point: valid UTF-8, cut at the character boundary and
/// then at the line boundary; with no line boundary before it, at the character boundary.
#[test]
fn boot_44_a_multibyte_character_across_the_cut() {
    // At 4096 bytes the cap is 1638; byte 1638 is the second byte of "é".
    let with_nl = format!(
        "{}\n{}é{}",
        "x".repeat(1000),
        "y".repeat(636),
        "z".repeat(500)
    );
    let b = assemble(&with_orientation(&with_nl, &path5()), 4_096).expect("fits");
    assert_eq!(kept_body(&b.markdown), "x".repeat(1000));
    // With no line boundary, the cut is at the character boundary that leaves room in the 1,638
    // bytes for the quote prefix and its newline (3) and the `full text` line (BOOT12-03: the
    // rendered text).
    let no_nl = format!("{}é{}", "y".repeat(1637), "z".repeat(500));
    let b = assemble(&with_orientation(&no_nl, &path5()), 4_096).expect("fits");
    let marker = format!("full text: cortex record show {}\n", record(1));
    assert_eq!(kept_body(&b.markdown), "y".repeat(1_638 - 3 - marker.len()));
    assert!(b.boot.orientation.as_ref().expect("present").cut);
}
