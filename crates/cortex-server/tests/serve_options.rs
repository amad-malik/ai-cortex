//! The options `cortex-server serve` runs with (piece 11 `serve`, mira; cox 17:59Z: the LAST_GOOD mount). DB-free. RED first.
use std::path::PathBuf;

use cortex_module::Registry;
use cortex_server::fault::Fault;
use cortex_server::serve::{
    LAST_GOOD_PATH, Refused, default_options, no_checks, production_checks,
};

#[test]
fn so1_the_last_good_file_is_in_the_dump_status_directory_and_nowhere_else() {
    assert_eq!(LAST_GOOD_PATH, "/var/lib/cortex-dumps/LAST_GOOD");
    assert_eq!(
        default_options(None).last_good,
        Some(PathBuf::from("/var/lib/cortex-dumps/LAST_GOOD"))
    );
    // Whatever the fault variable says, it does not move the file.
    assert_eq!(
        default_options(Some("ready-503")).last_good,
        Some(PathBuf::from(LAST_GOOD_PATH))
    );
}

#[test]
fn so2_the_version_is_the_crates_machine_version() {
    let v = default_options(None).version;
    assert_eq!(v, env!("CARGO_PKG_VERSION"));
    assert!(!v.is_empty());
}

#[test]
fn so3_the_test_fault_follows_the_build_and_an_unknown_one_is_ignored() {
    assert_eq!(default_options(None).fault, Fault::None);
    assert_eq!(default_options(Some("")).fault, Fault::None);
    assert_eq!(
        default_options(Some("bogus-fault-name")).fault,
        Fault::None,
        "an unknown fault is ignored"
    );
    let want = if cfg!(feature = "test-faults") {
        Fault::Ready503
    } else {
        Fault::None
    };
    assert_eq!(default_options(Some("ready-503")).fault, want);
}

#[test]
fn so4_the_real_command_runs_the_production_checks_and_not_the_fixtures_none() {
    let want: fn(&Registry) -> Result<(), Refused> = production_checks;
    let fixture: fn(&Registry) -> Result<(), Refused> = no_checks;
    let got = default_options(None).checks;
    assert!(
        std::ptr::fn_addr_eq(got, want),
        "default_options runs production_checks"
    );
    assert!(!std::ptr::fn_addr_eq(got, fixture));
}
