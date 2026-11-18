// The chain's wrapper is the only caller of idempotency::begin( and idempotency::finish(.
/* a block comment: idempotency::begin(), /* nested idempotency::finish() */ still a comment */
/// A doc comment may name idempotency::begin too.
pub fn doc() -> &'static str {
    "idempotency::begin( in a plain string literal"
}
pub const RAW: &str = r#"cortex_db::idempotency::finish(tx).await; and a " quote inside"#;
pub const MULTI: &str = "line one
    idempotency::begin(
    line three \" idempotency::finish(";
pub const CH: (char, char) = ('"', '\'');
pub fn life<'a>(s: &'a str) -> &'a str {
    s
}
pub const RAW_IN_A_STRING: &str = "cortex_db::idempotency::r#begin(tx).await; idempotency::r#finish(tx)";
// idempotency::r#begin( in a comment
