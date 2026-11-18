#[derive(serde::Deserialize)]
struct T { #[serde(flatten)] base: Base }
