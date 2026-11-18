#[derive(serde::Deserialize)]
#[serde(untagged)]
enum B { A(u8), C(String) }
fn main() {}
