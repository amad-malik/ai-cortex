#[derive(serde::Deserialize)]
struct E { #[serde(flatten)] base: Base }
