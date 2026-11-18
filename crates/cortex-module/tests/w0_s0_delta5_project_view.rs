//! Wave 0 S0 delta 5, part 1: `ProjectView` carries the installation id (OQ-T2; cox 825a9022,
//! kai's resume row 918d33a1). DB-free.
//!
//! Why: S3's `Whoami.installation` is required, and `IdentityApi::project` returned no installation
//! id; the tokens role cannot read `identity.installations`. The provider fills the field from
//! `identity.projects.installation_id` under the project's RLS.
//!
//! The field must NOT reach an HTTP body: the `Project` body of `GET /v2/projects/{id}` stays
//! `{id, name}` (sophi's ID-59 and X-10). S0 has no serialiser for `ProjectView`, so the guard here
//! is that none exists: a derive of `Serialize` on `ProjectView` stops this file compiling.
//!
//! RED: `ProjectView` has no `installation` field (compile error E0560).
use cortex_module::ProjectView;
use cortex_types::{InstallationId, ProjectId, SystemClock, WorkerId};

fn view(installation: InstallationId) -> ProjectView {
    ProjectView {
        id: ProjectId::new(&SystemClock),
        name: "helix".to_owned(),
        state: "active".to_owned(),
        lead: WorkerId::new(&SystemClock),
        installation,
        token_lifetime_days: 90,
    }
}

#[test]
fn d5_pv_1_carries_the_installation_it_was_built_with() {
    let inst = InstallationId::new(&SystemClock);
    let v = view(inst);
    assert_eq!(v.installation, inst);
    // A control: the installation is its own id, not the project's or the lead's.
    assert_ne!(v.installation.to_string(), v.id.to_string());
    assert_ne!(v.installation.to_string(), v.lead.to_string());
}

#[test]
fn d5_pv_2_equality_and_clone_include_the_installation() {
    let a = InstallationId::new(&SystemClock);
    let b = InstallationId::new(&SystemClock);
    assert_ne!(a, b, "two fresh installation ids differ");
    let one = view(a);
    let same = one.clone();
    assert_eq!(one, same, "a clone is equal, installation included");
    let mut other = one.clone();
    other.installation = b;
    assert_ne!(
        one, other,
        "two views that differ only in the installation are not equal"
    );
}

/// `ProjectView` must never become a body: the installation id is not an HTTP field (ID-59, X-10).
/// The probe answers "is `T: Serialize`?" without a dependency: an inherent method that exists only
/// when the bound holds wins over the trait method that answers `false`. The control in the same
/// case: `InstallationId` IS `Serialize`, so the probe can fire.
#[test]
fn d5_pv_3_project_view_is_not_serialisable() {
    use std::marker::PhantomData;

    struct Probe<T>(PhantomData<T>);
    trait Fallback {
        fn is_serialize(&self) -> bool {
            false
        }
    }
    impl<T> Fallback for Probe<T> {}
    impl<T: serde::Serialize> Probe<T> {
        fn is_serialize(&self) -> bool {
            true
        }
    }

    assert!(
        Probe::<InstallationId>(PhantomData).is_serialize(),
        "control: the probe sees a type that is Serialize"
    );
    assert!(
        !Probe::<ProjectView>(PhantomData).is_serialize(),
        "ProjectView must not implement Serialize: the installation id must not reach an HTTP body"
    );
}
