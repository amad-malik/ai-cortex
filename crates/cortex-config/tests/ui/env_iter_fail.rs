// F1-05 r2: Env has no iterator over its values.
fn main() {
    let env = cortex_config::Env::from_pairs(&[("HOME", "/home/w")]);
    for (k, v) in env.iter() {
        let _ = (k, v);
    }
}
