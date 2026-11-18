#[derive(serde::Deserialize)]
struct B { #[serde(flatten)] base: Base }
