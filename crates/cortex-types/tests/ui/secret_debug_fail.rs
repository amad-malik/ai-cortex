// F1-03: a Secret cannot be formatted with {:?}.
fn main() {
    let s = cortex_types::Secret::new("hunter2");
    println!("{:?}", s);
}
