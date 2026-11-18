//! Wave 1 `wave1/ports3` item 1 (cox W1-39 follow-up, row on finn's gap message 12:5xZ): `ProjectView` carries the
//! project's `token_lifetime_days`, so the tokens piece can give a new token the PROJECT's lifetime (S1 A3, 7 to 365
//! days, default 90) without reading `identity.projects`, which its role cannot. Same shape as the installation id
//! (delta 5). DB-free. The field must NOT reach an HTTP body: `ProjectView` is still not `Serialize`.
//!
//! RED: `ProjectView` has no `token_lifetime_days` field (compile error E0560).
use cortex_module::ProjectView;
use cortex_types::{InstallationId, ProjectId, SystemClock, WorkerId};

fn view(days: i32) -> ProjectView {
    ProjectView {
        id: ProjectId::new(&SystemClock),
        name: "helix".to_owned(),
        state: "open".to_owned(),
        lead: WorkerId::new(&SystemClock),
        installation: InstallationId::new(&SystemClock),
        token_lifetime_days: days,
    }
}

#[test]
fn w1_p3_pv_1_carries_the_lifetime_it_was_built_with() {
    for days in [7, 90, 365] {
        assert_eq!(view(days).token_lifetime_days, days);
    }
}

#[test]
fn w1_p3_pv_2_equality_and_clone_include_the_lifetime() {
    let a = view(90);
    let mut b = a.clone();
    assert_eq!(a, b);
    b.token_lifetime_days = 7;
    assert_ne!(a, b, "the lifetime is part of the value");
    assert_eq!(b.clone(), b);
}
