#[derive(serde::Deserialize)]
struct Z { #[serde(flatten)] z: Z0 }
fn main() {}
