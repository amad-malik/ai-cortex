#[derive(serde::Deserialize)]
pub struct R {
    #[serde (
        default,
        flatten
    )]
    pub base: Base,
}
#[cfg(test)]
mod t {
    #[derive(serde::Deserialize)]
    struct Inner { #[serde(flatten)] x: X }
}
