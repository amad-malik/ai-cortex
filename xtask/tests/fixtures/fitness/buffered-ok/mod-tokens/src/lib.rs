// #[serde(untagged)] in a comment is not an attribute.
/* block: #[serde(tag = "t", content = "c")] */
pub const A: &str = "#[serde(untagged)]";
pub const B: &str = r#"#[serde(tag = "t")] x"#;
// The default representation (externally tagged) is allowed, with and without other options.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub enum External { Create { name: String }, Drop }
#[derive(serde::Deserialize)]
pub struct Plain {
    #[serde(rename = "untagged")]
    pub a: u8,
    #[serde(alias = "tag", default = "content")]
    pub b: u8,
    #[serde(skip)]
    pub c: u8,
    #[serde(default, rename(deserialize = "x"), skip_serializing_if = "Option::is_none")]
    pub d: Option<u8>,
    // a nested list that mentions the words is not a top-level key
    #[serde(bound(deserialize = tag))]
    pub e: u8,
    pub tag: String,
    pub content: Vec<u8>,
    pub untagged: bool,
}
pub fn names() -> u8 { let tag = 1; let content = 2; let untagged = 3; tag + content + untagged }
// A nested list may use the words after a comma; only the TOP level of serde(..) is read.
#[serde(bound(serialize = x, tag = y, content = z), rename_all = "snake_case")]
pub struct Nested;
// A namespace that merely ends in serde is not serde.
#[xserde(untagged)]
pub struct Prefixed;
// Another attribute namespace that happens to use the same words is not serde's.
#[strum(tag = "x")]
pub struct Other;
#[arg(untagged, content = "c")]
pub struct Cli;
// A custom word that merely starts with serde is not serde.
#[serde_with(untagged)]
pub struct Sw;
// The alternate delimiters of a list are read the same way, and the same namespace rules hold.
#[serde_with { untagged }]
pub struct SwBrace;
#[xserde [untagged]]
pub struct XBracket;
#[strum { tag = "x" }]
pub struct StrumBrace;
#[serde { rename_all = "snake_case", deny_unknown_fields }]
pub enum BraceOk { A, B }
#[serde [bound(serialize = x, tag = y)]]
pub struct BracketNested;
