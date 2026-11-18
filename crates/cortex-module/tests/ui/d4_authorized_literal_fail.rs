// Delta 4: an `Authorized` cannot be written by hand outside the crate: its fields are private and
// there is no constructor. Only `Ctx::authorize` makes one (and since the D4-V1 rework it carries the
// chain's authority, which a piece cannot name either).
#![allow(unreachable_code)]
use cortex_module::Authorized;

fn main() {
    let _forged = Authorized {
        value: (),
        authority: panic!(),
    };
}
