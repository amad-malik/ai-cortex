#[derive(serde::Deserialize)]
#[serde(untagged)]
pub enum U { A { x: u8 }, B { y: u8 } }
#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum I { Create { name: String } }
#[derive(serde::Deserialize)]
#[serde(tag = "t", content = "c")]
pub enum A { Create { name: String } }
mod more;
