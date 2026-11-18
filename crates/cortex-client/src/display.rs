//! Text from a server on its way to a terminal (cox's decisions on ellis F-1, 6 Nov 18:37Z): ONE list of unsafe characters,
//! ONE helper. In md every unsafe character becomes `?`; in the json line every one becomes a lossless `\uXXXX` escape.
//! A member is cut by CHARACTERS (Unicode scalar values) to a budget and the cut ends with `...` INSIDE the budget.

/// `title` (decoded Problem), in characters.
pub const BUDGET_TITLE: usize = 200;
/// `detail`, in characters.
pub const BUDGET_DETAIL: usize = 1000;
/// `fix`, in characters.
pub const BUDGET_FIX: usize = 500;
/// `type` and `request_id`, in characters (they are also validated to printable ASCII of this length).
pub const BUDGET_IDENT: usize = 64;

/// Unsafe on a terminal: `char::is_control()` (C0, DEL, C1) plus the format characters that can disguise text or hide it:
/// U+2028, U+2029, the bidi marks U+200E and U+200F, U+061C (ALM), the embeddings and overrides U+202A to U+202E, the isolates
/// U+2066 to U+2069, and the tag block U+E0000 to U+E007F (invisible characters that can carry text a reader does not see). Cox
/// decided the list on 6 Nov 18:37Z and extended it with ALM and the tag block on 7 Nov 12:24Z (ellis N-1).
pub fn is_unsafe_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{2028}'
                | '\u{2029}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{61c}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
                | '\u{e0000}'..='\u{e007f}'
        )
}

/// At most `budget` characters of `text`, mapped by `map`; a longer text ends with `...` inside the budget. Only the first
/// `budget + 1` characters of a (possibly 16 MiB) text are looked at.
fn cut(text: &str, budget: usize, map: impl Fn(char) -> char) -> String {
    let mut chars = text.chars();
    let head: Vec<char> = chars.by_ref().take(budget).collect();
    if chars.next().is_none() {
        return head.into_iter().map(map).collect();
    }
    if budget <= 3 {
        return ".".repeat(budget);
    }
    let mut out: String = head.into_iter().take(budget - 3).map(map).collect();
    out.push_str("...");
    out
}

/// `text` cut to `budget` characters; a longer text ends with `...` inside the budget. Nothing else changes.
pub fn bounded(text: &str, budget: usize) -> String {
    cut(text, budget, |c| c)
}

/// Every unsafe character becomes `?`, then [`bounded`].
pub fn safe_text(text: &str, budget: usize) -> String {
    cut(text, budget, |c| if is_unsafe_char(c) { '?' } else { c })
}

/// In a serialized JSON line, every unsafe character becomes a `\uXXXX` escape (the line parses to the same strings). A character
/// outside the Basic Multilingual Plane (the tag block) is written as the UTF-16 surrogate pair JSON requires: two escapes.
pub fn json_escape_unsafe(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    let mut units = [0u16; 2];
    for c in json.chars() {
        if is_unsafe_char(c) {
            for unit in c.encode_utf16(&mut units) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        } else {
            out.push(c);
        }
    }
    out
}
