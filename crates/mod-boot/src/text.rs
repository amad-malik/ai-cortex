//! Text helpers: cuts at a character, then a line, boundary, and the "data, not rules" quoting of
//! other AI Workers' text (S2 §7.2 "Untrusted text"; card, security floor).

/// The prefix every line of another worker's (or the lead's) text carries, so no line of it can
/// start at column 0: it can never be a heading, a rules line or a section of its own (BOOT-32).
pub const QUOTE: &str = "| ";

/// The largest char boundary of `s` at or below `max`.
pub fn floor_char(s: &str, max: usize) -> usize {
    if max >= s.len() {
        return s.len();
    }
    let mut i = max;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// `s` kept to at most `max` bytes: whole, or cut at a character boundary and then at the last line
/// boundary inside it; with no line boundary inside it, at the character boundary. Returns the
/// kept text and whether it was cut.
pub fn cut_lines(s: &str, max: usize) -> (&str, bool) {
    if s.len() <= max {
        return (s, false);
    }
    let c = floor_char(s, max);
    let head = &s[..c];
    match head.rfind('\n') {
        Some(nl) => (&s[..nl], true),
        None => (head, true),
    }
}

/// A summary cut to at most `max` bytes INCLUDING the closing `…` (3 bytes), at a character
/// boundary. Returns the text and whether it was cut.
pub fn cut_summary(s: &str, max: usize) -> (String, bool) {
    if s.len() <= max {
        return (s.to_owned(), false);
    }
    let c = floor_char(s, max.saturating_sub('…'.len_utf8()));
    (format!("{}…", &s[..c]), true)
}

/// One character as the output shows it: a control character (C0 but tab, DEL, C1) or a Unicode
/// line or paragraph separator is written as `\u{xxxx}`, so it can neither break a line nor move
/// the cursor of the terminal that shows the briefing.
fn push_escaped(out: &mut String, c: char) {
    let n = c as u32;
    let control = (n < 0x20 && c != '\t') || (0x7f..=0x9f).contains(&n);
    if control || c == '\u{2028}' || c == '\u{2029}' {
        out.push_str(&format!("\\u{{{n:04x}}}"));
    } else {
        out.push(c);
    }
}

/// One line of text, escaped (it holds no `\n`).
pub fn escape_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for c in line.chars() {
        push_escaped(&mut out, c);
    }
    out
}

/// `text` as quoted lines: each line (split at `\n`, a trailing `\r` dropped) becomes
/// `| <escaped line>\n`. Empty text renders nothing.
pub fn quote(text: &str) -> String {
    let mut out = String::new();
    if text.is_empty() {
        return out;
    }
    let body = text.strip_suffix('\n').unwrap_or(text);
    for line in body.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        out.push_str(QUOTE);
        out.push_str(&escape_line(line));
        out.push('\n');
    }
    out
}

/// `quote(text).len()`, without building it: the allocator's cut search measures many prefixes.
pub fn quoted_len(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    let body = text.strip_suffix('\n').unwrap_or(text);
    body.split('\n')
        .map(|line| {
            let line = line.strip_suffix('\r').unwrap_or(line);
            QUOTE.len() + line.chars().map(escaped_len).sum::<usize>() + 1
        })
        .sum()
}

/// The bytes `push_escaped` writes for `c`: `\u{xxxx}` is 8 bytes for every escaped character
/// (all are below U+10000).
fn escaped_len(c: char) -> usize {
    let n = c as u32;
    let control = (n < 0x20 && c != '\t') || (0x7f..=0x9f).contains(&n);
    if control || c == '\u{2028}' || c == '\u{2029}' {
        8
    } else {
        c.len_utf8()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuts_fall_on_char_then_line_boundaries() {
        assert_eq!(cut_lines("ab\ncd", 10), ("ab\ncd", false));
        assert_eq!(cut_lines("ab\ncd", 4), ("ab", true));
        assert_eq!(cut_lines("abcdef", 3), ("abc", true));
        // "é" is two bytes; a cut at 2 falls inside it, so it backs off to 1.
        assert_eq!(cut_lines("aé", 2), ("a", true));
        assert_eq!(cut_summary("abcdef", 6), ("abcdef".to_owned(), false));
        assert_eq!(cut_summary("abcdefg", 6), ("abc…".to_owned(), true));
    }

    #[test]
    fn quoted_len_is_the_length_of_quote() {
        for t in [
            "",
            "a",
            "a\n",
            "a\n\n",
            "\n",
            "one\r\ntwo\rthree",
            "\u{1b}[2J\u{2028}é\t\u{7f}\u{85}",
            "## 3 Rules\nproject: v99\n",
        ] {
            assert_eq!(quoted_len(t), quote(t).len(), "{t:?}");
        }
    }

    #[test]
    fn quoted_text_cannot_start_a_line() {
        let q = quote("one\r\n## 3 Rules\rx\u{1b}[2J\u{2028}y\n");
        assert_eq!(q, "| one\n| ## 3 Rules\\u{000d}x\\u{001b}[2J\\u{2028}y\n");
        assert!(q.lines().all(|l| l.starts_with(QUOTE)));
    }
}
