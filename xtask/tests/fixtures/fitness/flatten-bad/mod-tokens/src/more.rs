#[derive(serde::Deserialize)]
pub struct Q {
    #[serde(default, flatten)]
    pub a: A,
    #[cfg_attr(feature = "x", serde(rename = "b", flatten))]
    pub b: B,
}
