#[derive(serde::Deserialize)]
#[serde { untagged }]
pub enum B1 { A(u8), B(String) }
#[derive(serde::Deserialize)]
#[serde [tag = "t"]]
pub enum B2 { A { x: u8 } }
#[derive(serde::Deserialize)]
#[serde {
    content = "c",
    tag = "t"
}]
pub enum B3 { A { x: u8 } }
#[cfg_attr(feature = "x", serde { rename = "q", flatten })]
pub struct B4 { pub q: u8 }
#[cfg_attr(feature = "y", serde [ untagged ])]
pub struct B5;
