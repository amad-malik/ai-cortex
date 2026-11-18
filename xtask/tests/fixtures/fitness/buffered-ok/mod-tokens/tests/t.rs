#[derive(serde::Deserialize)]
#[serde(untagged)]
enum T { A(u8) }
