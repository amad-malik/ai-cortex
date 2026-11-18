// #[serde(flatten)] in a comment is not an attribute.
/* block: #[serde(flatten)] base: Base, */
pub const A: &str = "#[serde(flatten)]";
pub const B: &str = r#"#[serde(default, flatten)] x: X"#;
pub fn iter(v: Vec<Option<u8>>) -> Vec<u8> { v.into_iter().flatten().collect() }
pub fn names() -> u8 { let flatten = 1; flatten }
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct Plain { pub a: u8 }
pub mod flatten { pub fn flatten() {} }
