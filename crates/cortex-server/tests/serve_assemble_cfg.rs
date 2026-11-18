//! The production `assemble(&Config)` over the REAL pieces (cox 20:07Z: mod-audit is merged, the real set assembles for the first time).
//! It takes the process's one problem-code minter, so this file holds exactly one test and one call. DB-free. RED first.
//! The config enables EXACTLY the pieces this binary compiled in (read from `compiled()`, so the test follows the feature mix); the
//! assembly must succeed, and, when the policy is compiled in, the production checks must pass on the assembled registry.
use std::fs;
use std::path::PathBuf;

use cortex_config::{Env, load};
use cortex_server::assembly::{assemble, compiled, ordered_modules, startup_line};
use cortex_server::serve::production_checks;

#[test]
fn as9_the_real_set_assembles_from_the_config_and_passes_the_production_checks() {
    let ids: Vec<&'static str> = compiled().iter().map(|p| p.manifest().id).collect();
    assert_eq!(
        ids[0], "audit",
        "audit is first: identity writes through its sink"
    );
    let list = ids
        .iter()
        .map(|i| format!("\"{i}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let dir =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("asm-cfg-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("dir");
    let file = dir.join("cortex.toml");
    fs::write(&file, format!("[modules]\nenabled = [{list}]\n")).expect("config");
    let cfg = load(&file, &Env::from_pairs(&[("HOME", "/home/x")])).expect("a valid config");
    let registry = match assemble(&cfg) {
        Ok(r) => r,
        Err(errs) => panic!("the real set does not assemble: {}", startup_line(&errs)),
    };
    // Every compiled piece is wired, in migration order, with `platform` first for the pool.
    let wired: Vec<&str> = registry.pieces().map(|p| p.manifest().id).collect();
    assert_eq!(wired, ids);
    let modules: Vec<String> = ordered_modules(&registry)
        .iter()
        .map(|m| m.as_str().to_owned())
        .collect();
    assert_eq!(modules[0], "platform");
    assert_eq!(&modules[1..], &ids[..]);
    // The ports the chain needs: the verifier (tokens) always; the scope policy when the policy is compiled in.
    assert!(
        registry.ports().token_verifier().is_some(),
        "tokens provides the verifier"
    );
    #[cfg(feature = "mod-policy")]
    {
        assert!(registry.ports().scope_policy().is_some());
        production_checks(&registry).expect("the real manifests pass the policy table");
    }
    #[cfg(not(feature = "mod-policy"))]
    assert!(production_checks(&registry).is_ok());
}
