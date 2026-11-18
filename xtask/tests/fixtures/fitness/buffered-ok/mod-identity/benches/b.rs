#[derive(serde::Deserialize)]
#[serde(tag = "t", content = "c")]
enum B { A(u8) }
