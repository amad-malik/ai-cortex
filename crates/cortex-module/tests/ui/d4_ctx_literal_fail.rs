// Delta 4: a `Ctx` cannot be written by hand outside the crate (private fields), so a piece cannot
// give itself a route rule, a policy or an identity of its own.
#![allow(unreachable_code)]
use cortex_module::Ctx;

fn forge() -> Ctx {
    Ctx {
        authority: panic!(),
        path: Vec::new(),
    }
}

fn main() {
    let _ = forge;
}
