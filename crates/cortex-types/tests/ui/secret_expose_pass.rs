// F1-03 pass partner: a Secret can be built and read, but only through `expose()`.
use cortex_types::Secret;

fn main() {
    let s = Secret::new("hunter2");
    assert_eq!(s.expose(), "hunter2");
}
