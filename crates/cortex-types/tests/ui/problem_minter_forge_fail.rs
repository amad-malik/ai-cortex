// F1A-R0-02: the minter cannot be built outside `take`.
fn main() {
    let m = cortex_types::ProblemCodeMinter { _sealed: () };
    let _ = m;
}
