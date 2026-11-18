// F1-05 r2: Env cannot be iterated by value either.
fn main() {
    let env = cortex_config::Env::from_pairs(&[("HOME", "/home/w")]);
    for pair in env {
        let _ = pair;
    }
}
