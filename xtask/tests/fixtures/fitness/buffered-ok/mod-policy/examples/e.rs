#[derive(serde::Deserialize)]
#[serde(untagged)]
enum E { A(u8) }
