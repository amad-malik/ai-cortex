pub fn nothing() {}
#[cfg(test)]
mod t {
    #[derive(serde::Deserialize)]
    #[serde(content = "c", tag = "t")]
    enum E { A(u8) }
}
