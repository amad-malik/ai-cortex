//! The test-only fault switch (contract r0.3 section 4): `CORTEX_TEST_FAULT=ready-503` makes `GET /v2/health/ready` answer 503
//! while everything else works, ONLY in a build with the `test-faults` feature (off by default, never in a release build). Without the
//! feature the variable is ignored, with a startup warning if it is set. A warning never echoes the value.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    None,
    Ready503,
}

pub const WARN_IGNORED: &str =
    "CORTEX_TEST_FAULT is set, but this build has no test-faults feature: it is ignored";
pub const WARN_UNKNOWN: &str =
    "CORTEX_TEST_FAULT names a fault this build does not know: it is ignored";

const READY_503: &str = "ready-503";

/// The decision with the build's capability as a parameter, so both builds are testable in one run.
pub fn fault_with(compiled_in: bool, var: Option<&str>) -> Fault {
    match var {
        Some(READY_503) if compiled_in => Fault::Ready503,
        _ => Fault::None,
    }
}

/// The startup warning, if any, with the build's capability as a parameter. An empty variable counts as not set.
pub fn warning_with(compiled_in: bool, var: Option<&str>) -> Option<&'static str> {
    match var {
        None | Some("") => None,
        Some(_) if !compiled_in => Some(WARN_IGNORED),
        Some(READY_503) => None,
        Some(_) => Some(WARN_UNKNOWN),
    }
}

/// This build's fault for the variable `CORTEX_TEST_FAULT`.
pub fn fault(var: Option<&str>) -> Fault {
    fault_with(cfg!(feature = "test-faults"), var)
}

/// This build's startup warning for the variable `CORTEX_TEST_FAULT`.
pub fn warning(var: Option<&str>) -> Option<&'static str> {
    warning_with(cfg!(feature = "test-faults"), var)
}
