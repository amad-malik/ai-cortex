// Delta 5: a `DetailValue` holding a free string cannot be written outside the module: the only
// string-holding value is a `DetailCode`, whose field is private, and `AuditDetails::code` is the
// only constructor, which validates.
use cortex_module::{DetailCode, DetailValue};

fn main() {
    let _forged = DetailValue::Code(DetailCode(String::from("ctx2_not_a_code.at.all")));
}
