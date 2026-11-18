#[derive(serde::Deserialize)]
pub enum V {
    #[serde(untagged)]
    Loose(Vec<u8>),
    Plain(u8),
}
# [ serde ( flatten ) ]
pub struct Spaced { pub b: B }
#[derive(serde::Deserialize)]
pub struct Cfg {
    #[cfg_attr(all(feature = "a", not(feature = "b")), serde(rename = "q", untagged))]
    pub q: u8,
    #[cfg_attr(
        feature = "x",
        serde(
            default,
            tag = "z"
        )
    )]
    pub z: u8,
}
#[cfg_attr(feature = "a", serde(rename = "a"), serde(untagged))]
pub struct Two;
