// mike's carry item 3: the minter is not Clone.
fn main() {
    let m = cortex_types::ProblemCodeMinter::take().expect("this process's minter");
    let _second = m.clone();
}
