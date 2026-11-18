// Delta 5: the legal use of the closed type compiles and runs; entries read back.
use cortex_module::{AuditDetails, DetailRef, DetailValue, PortError};
use cortex_types::{SystemClock, WorkerId};

fn main() -> Result<(), PortError> {
    let w = WorkerId::new(&SystemClock);
    let d = AuditDetails::new()
        .id("worker", w)?
        .code("reason", "lost_key")?
        .flag("self_review", true)?;
    assert_eq!(d.entries()[0].1, DetailValue::Ref(DetailRef::Worker(w)));
    assert_eq!(d.len(), 3);
    Ok(())
}
