// Delta 4: an `Authorization` (the erased proof the replay lookup takes) cannot be written by hand
// outside the crate, so a lookup that takes `&Authorization` cannot run before `authorize`.
#![allow(unreachable_code)]
use cortex_module::Authorization;

fn main() {
    let _forged = Authorization {
        inner: Box::new(()),
        authority: panic!(),
    };
}
