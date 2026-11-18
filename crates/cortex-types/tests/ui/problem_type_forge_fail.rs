// F1A-R0-02: a ProblemType cannot be made from a string outside the minter.
fn main() {
    let t = cortex_types::ProblemType("invented_code");
    let _ = t;
}
