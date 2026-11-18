// F1-05 r2 pass partner: a value is read explicitly, by its key.
fn main() {
    let env = cortex_config::Env::from_pairs(&[("HOME", "/home/w")]);
    assert_eq!(env.get("HOME"), Some("/home/w"));
}
