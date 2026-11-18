//! PR-10 (the helper): the ONE list of unsafe characters (cox, 6 Nov 18:37Z), the cut by characters, the json escape.
//! One cell per class, with boundary pairs on both sides of every range. No socket.
use cortex_client::display::{bounded, is_unsafe_char, json_escape_unsafe, safe_text};

/// `(class, characters that MUST be unsafe)`.
const UNSAFE: &[(&str, &[char])] = &[
    (
        "C0",
        &[
            '\0', '\x07', '\x08', '\t', '\n', '\x0b', '\x0c', '\r', '\x1b', '\x1f',
        ],
    ),
    ("DEL", &['\x7f']),
    ("C1", &['\u{80}', '\u{85}', '\u{9b}', '\u{9f}']),
    ("line and paragraph separators", &['\u{2028}', '\u{2029}']),
    ("bidi marks", &['\u{200e}', '\u{200f}']),
    (
        "bidi embeddings and overrides",
        &['\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}', '\u{202e}'],
    ),
    (
        "bidi isolates",
        &['\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}'],
    ),
    // cox, 7 Nov (row 94018c97, ellis N-1): the two additions
    ("arabic letter mark", &['\u{61c}']),
    (
        "tag block (outside the BMP)",
        &[
            '\u{e0000}',
            '\u{e0001}',
            '\u{e0020}',
            '\u{e0041}',
            '\u{e007e}',
            '\u{e007f}',
        ],
    ),
];

/// Characters just outside every range, and ordinary text: these MUST stay.
const SAFE: &[char] = &[
    '\u{61b}',
    '\u{61d}',
    '\u{dffff}',
    '\u{e0080}',
    ' ',
    '~',
    'A',
    '\u{a0}',
    '\u{a1}',
    '\u{e9}',
    '\u{2010}',
    '\u{2027}',
    '\u{202f}',
    '\u{2065}',
    '\u{206a}',
    '\u{200b}',
    '\u{200d}',
    '\u{feff}',
    '\u{3b1}',
    '\u{65e5}',
    '\u{1f600}',
];

#[test]
fn pr_10_every_class_is_unsafe_and_the_boundaries_hold() {
    for (class, chars) in UNSAFE {
        for &c in *chars {
            assert!(
                is_unsafe_char(c),
                "{class}: U+{:04X} must be unsafe",
                c as u32
            );
        }
    }
    for &c in SAFE {
        assert!(!is_unsafe_char(c), "U+{:04X} must stay", c as u32);
    }
    // the edges of the ranges, both sides
    assert!(is_unsafe_char('\x1f') && !is_unsafe_char('\x20'));
    assert!(!is_unsafe_char('\x7e') && is_unsafe_char('\x7f'));
    assert!(is_unsafe_char('\u{9f}') && !is_unsafe_char('\u{a0}'));
    assert!(!is_unsafe_char('\u{200d}') && is_unsafe_char('\u{200e}'));
    assert!(is_unsafe_char('\u{200f}') && !is_unsafe_char('\u{2010}'));
    assert!(!is_unsafe_char('\u{2027}') && is_unsafe_char('\u{2028}'));
    assert!(is_unsafe_char('\u{2029}') && is_unsafe_char('\u{202a}'));
    assert!(is_unsafe_char('\u{202e}') && !is_unsafe_char('\u{202f}'));
    assert!(!is_unsafe_char('\u{61b}') && is_unsafe_char('\u{61c}') && !is_unsafe_char('\u{61d}'));
    assert!(!is_unsafe_char('\u{dffff}') && is_unsafe_char('\u{e0000}'));
    assert!(is_unsafe_char('\u{e007f}') && !is_unsafe_char('\u{e0080}'));
    assert!(!is_unsafe_char('\u{2065}') && is_unsafe_char('\u{2066}'));
    assert!(is_unsafe_char('\u{2069}') && !is_unsafe_char('\u{206a}'));
}

#[test]
fn pr_10_safe_text_maps_every_unsafe_character_to_a_question_mark_and_keeps_ordinary_text() {
    assert_eq!(
        safe_text("A\x1b[31mB\rC\x07D\0E\x7fF", 100),
        "A?[31mB?C?D?E?F"
    );
    for (class, chars) in UNSAFE {
        let s: String = chars.iter().collect();
        let want: String = chars.iter().map(|_| '?').collect();
        assert_eq!(safe_text(&s, 100), want, "{class}");
    }
    let ordinary = "Ünïcödé \u{3b5}\u{3bb}\u{3bb}\u{3b7}\u{3bd}\u{3b9}\u{3ba}\u{3ac} \u{65e5}\u{672c}\u{8a9e} \u{1f600} family \u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467} nbsp\u{a0}end";
    assert_eq!(
        safe_text(ordinary, 100),
        ordinary,
        "ordinary Unicode, including a zero-width joiner, is unchanged"
    );
    assert_eq!(safe_text("", 10), "");
}

#[test]
fn pr_10_the_budget_counts_characters_and_the_marker_is_inside_it() {
    let at = "a".repeat(200);
    assert_eq!(safe_text(&at, 200), at, "exactly the budget is unchanged");
    let over = "a".repeat(201);
    let cut = safe_text(&over, 200);
    assert_eq!(cut.chars().count(), 200);
    assert_eq!(cut, format!("{}...", "a".repeat(197)));
    // 200 two-byte characters are 400 bytes and still within a budget of 200 characters
    let wide = "\u{e9}".repeat(200);
    assert_eq!(safe_text(&wide, 200), wide);
    // 5,000 two-byte and 300 four-byte characters are cut on a character boundary
    for ch in ['\u{e9}', '\u{1f600}'] {
        let long: String = std::iter::repeat_n(ch, 5000).collect();
        let cut = safe_text(&long, 200);
        assert_eq!(cut.chars().count(), 200, "{ch}");
        assert!(cut.ends_with("..."), "{ch}");
        assert_eq!(
            cut.chars().take(197).collect::<String>(),
            std::iter::repeat_n(ch, 197).collect::<String>()
        );
    }
    // tiny budgets
    assert_eq!(safe_text("abcde", 4), "a...");
    assert_eq!(safe_text("abcde", 3), "...");
    assert_eq!(safe_text("abcde", 0), "");
    assert_eq!(safe_text("abc", 3), "abc");
}

#[test]
fn pr_10_bounded_cuts_like_safe_text_and_changes_nothing_else() {
    let s = format!("x\x1b{}", "b".repeat(300));
    let cut = bounded(&s, 200);
    assert_eq!(cut.chars().count(), 200);
    assert!(
        cut.starts_with("x\x1b") && cut.ends_with("..."),
        "controls are not mapped by bounded"
    );
    assert_eq!(bounded("short\x1b", 200), "short\x1b");
}

#[test]
fn pr_10_the_json_escape_is_lossless_and_leaves_no_unsafe_character_in_the_line() {
    let all: String = UNSAFE.iter().flat_map(|(_, c)| c.iter()).collect();
    let mixed = format!("plain \u{e9} \u{65e5} {all} end");
    let line =
        serde_json::to_string(&serde_json::json!({ "error": { "title": mixed, "n": 7 } })).unwrap();
    let escaped = json_escape_unsafe(&line);
    // checked against the TABLE above (the ruling's list written out), not against the library's own predicate
    let in_table = |c: char| UNSAFE.iter().any(|(_, cs)| cs.contains(&c));
    assert!(
        !escaped.chars().any(in_table),
        "no unsafe character is left raw: {escaped:?}"
    );
    assert!(
        !escaped.chars().any(is_unsafe_char),
        "nor by the library's own list: {escaped:?}"
    );
    let back: serde_json::Value = serde_json::from_str(&escaped).expect("still JSON");
    assert_eq!(
        back["error"]["title"],
        mixed.as_str(),
        "the same string comes back"
    );
    assert_eq!(back["error"]["n"], 7);
    // text with nothing unsafe in it is returned as it was
    let clean = r#"{"a":"b é \n"}"#;
    assert_eq!(json_escape_unsafe(clean), clean);
    // the escapes are the lower-case four-digit form
    assert!(json_escape_unsafe("\"\u{7f}\"").contains("\\u007f"));
    assert!(json_escape_unsafe("\"\u{202e}\"").contains("\\u202e"));
}

#[test]
fn pr_10_a_character_outside_the_bmp_is_escaped_as_a_surrogate_pair() {
    // the expected escape is computed with std, not with the library
    for c in ['\u{e0000}', '\u{e0041}', '\u{e007f}'] {
        let mut b = [0u16; 2];
        let units = c.encode_utf16(&mut b);
        assert_eq!(units.len(), 2, "outside the BMP");
        let want = format!("\"\\u{:04x}\\u{:04x}\"", units[0], units[1]);
        assert_eq!(
            json_escape_unsafe(&format!("\"{c}\"")),
            want,
            "U+{:05X}",
            c as u32
        );
        let back: String = serde_json::from_str(&json_escape_unsafe(&format!("\"{c}\""))).unwrap();
        assert_eq!(
            back,
            c.to_string(),
            "the pair parses back to the same character"
        );
    }
    assert!(
        json_escape_unsafe("\"\u{61c}\"").contains("\\u061c"),
        "ALM is in the BMP: one escape"
    );
}
