// mike's carry item 3: the minter is not Copy; using it after a move does not compile.
fn keep(_: cortex_types::ProblemCodeMinter) {}

fn main() {
    let m = cortex_types::ProblemCodeMinter::take().expect("this process's minter");
    keep(m);
    keep(m);
}
