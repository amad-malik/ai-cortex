//! Wave 0 plug: the identity fixture registers through S0 on its own. DB-free.
use std::sync::OnceLock;

use cortex_module::{Assembly, Piece, Port, Registry};
use cortex_types::{InstallationId, ModuleId, ProblemCodeMinter};
use plug_identity::{FIXTURE_INSTALLATION, ID, PlugIdentity, SCOPE_READ};

fn minter() -> &'static ProblemCodeMinter {
    static M: OnceLock<ProblemCodeMinter> = OnceLock::new();
    M.get_or_init(|| ProblemCodeMinter::take().expect("the first take in this process"))
}

#[test]
fn w0_plug_identity_registers_provides_identity_and_mounts_the_roster() {
    let enabled = [ModuleId::parse(ID).expect("id")];
    let reg = Registry::assemble(Assembly {
        compiled: vec![Box::new(PlugIdentity::new()) as Box<dyn Piece>],
        enabled: &enabled,
        minter: minter(),
        chain_codes: &["internal_error", "unavailable", "busy", "conflict"],
    })
    .expect("the fixture assembles");
    assert!(reg.ports().has(Port::Identity));
    let routes: Vec<(String, &str, &str)> = reg
        .routes()
        .flat_map(|(_, r)| {
            r.entries()
                .map(|e| (e.method.to_string(), e.path, e.scope))
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(
        routes,
        [("GET".to_owned(), "/v2/identity/roster", SCOPE_READ)]
    );
    let m = PlugIdentity::new().migrations();
    assert_eq!((m.schema, m.steps.len()), (ID, 1));
}

/// Delta 5: the fixture's installation default is a valid installation id, and the migration's
/// column default is the very same text (so a seed that names none reads back `FIXTURE_INSTALLATION`).
/// DB-free; the row read itself is the DB test's.
#[test]
fn w0_plug_identity_the_fixture_installation_is_the_migrations_default() {
    assert!(InstallationId::parse(FIXTURE_INSTALLATION).is_ok());
    let m = PlugIdentity::new().migrations();
    let up = m.steps[0].up;
    assert!(
        up.contains(&format!(
            "installation_id uuid NOT NULL DEFAULT '{FIXTURE_INSTALLATION}'"
        )),
        "the projects table carries the installation column with the fixture default"
    );
}
