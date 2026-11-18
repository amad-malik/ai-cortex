// the attribute below is the violation
#[derive(serde::Deserialize)]
pub struct Req {
    pub name: String,
    #[serde(flatten)]
    pub base: Base,
}
mod more;
