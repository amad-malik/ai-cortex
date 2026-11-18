//! The test-only fault switch (contract r0.3 section 4). DB-free. RED first.
//!
//! The two builds are tested through the `compiled_in` parameter so the default-features run proves both.

use cortex_server::fault::{Fault, WARN_IGNORED, WARN_UNKNOWN, fault_with, warning_with};

#[test]
fn ft_01_with_the_feature_ready_503_is_the_one_fault() {
    assert_eq!(fault_with(true, Some("ready-503")), Fault::Ready503);
    assert_eq!(fault_with(true, None), Fault::None);
    for other in [
        "",
        "ready_503",
        "READY-503",
        "ready-503 ",
        " ready-503",
        "ready-504",
        "x",
    ] {
        assert_eq!(fault_with(true, Some(other)), Fault::None, "{other:?}");
    }
}

#[test]
fn ft_02_without_the_feature_the_variable_is_ignored_with_a_warning() {
    assert_eq!(
        fault_with(false, Some("ready-503")),
        Fault::None,
        "the production build never injects a fault"
    );
    assert_eq!(fault_with(false, None), Fault::None);
    assert_eq!(warning_with(false, Some("ready-503")), Some(WARN_IGNORED));
    assert_eq!(warning_with(false, Some("anything")), Some(WARN_IGNORED));
    assert_eq!(warning_with(false, None), None, "no variable, no warning");
    assert_eq!(
        warning_with(false, Some("")),
        None,
        "an empty variable is not set"
    );
}

#[test]
fn ft_03_with_the_feature_a_fault_name_it_does_not_know_warns() {
    assert_eq!(warning_with(true, Some("ready-503")), None);
    assert_eq!(warning_with(true, None), None);
    assert_eq!(warning_with(true, Some("")), None);
    assert_eq!(warning_with(true, Some("ready-504")), Some(WARN_UNKNOWN));
}

#[test]
fn ft_04_a_warning_never_echoes_the_value() {
    for w in [WARN_IGNORED, WARN_UNKNOWN] {
        assert!(!w.contains("ready-504") && !w.contains('\n'), "{w}");
    }
}

#[test]
fn ft_05_this_build_matches_its_feature() {
    use cortex_server::fault::{fault, warning};
    let compiled = cfg!(feature = "test-faults");
    assert_eq!(
        fault(Some("ready-503")),
        if compiled {
            Fault::Ready503
        } else {
            Fault::None
        }
    );
    assert_eq!(
        warning(Some("ready-503")),
        if compiled { None } else { Some(WARN_IGNORED) }
    );
}
