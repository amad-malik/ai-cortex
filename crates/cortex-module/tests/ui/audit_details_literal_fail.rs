// Delta 5: `AuditDetails` cannot be built from its parts outside the module: its field is private,
// so the validating builders are the only way in.
use cortex_module::AuditDetails;

fn main() {
    let _forged = AuditDetails(Vec::new());
}
