//! Bounded, escaped text for messages that mention a value from outside.

/// At most 40 characters of `s`, escaped so that no control character survives, in double quotes;
/// `...` after the quote when something was cut. A message never carries more than this.
pub(crate) fn cut(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars().take(40) {
        out.extend(c.escape_debug());
    }
    out.push('"');
    if s.chars().nth(40).is_some() {
        out.push_str("...");
    }
    out
}

/// A path for a message (R6A-01, mike's second finding): the WHOLE path (the operator needs it to fix the problem), with every control character shown ESCAPED
/// (`\u{1}`, `\n`) and never raw: C0, DEL and the C1 controls (U+009B is the one-character CSI). A path made of ordinary characters is shown exactly as
/// `Path::display` shows it. A canonical path can hold a control character that the operator's own text did not (a link resolves to it), and a message
/// goes to the operator's terminal.
pub(crate) fn shown(p: impl AsRef<std::path::Path>) -> String {
    let mut out = String::new();
    for c in p.as_ref().display().to_string().chars() {
        if c.is_control() {
            out.extend(c.escape_debug());
        } else {
            out.push(c);
        }
    }
    out
}
