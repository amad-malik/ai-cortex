//! The production `assemble(&Config)` refuses a real set that leaves a core piece out, naming it. One assemble per process (the minter), so
//! this is its own binary. DB-free. RED first. (Before mod-audit merged, identity alone was refused for want of an audit sink.)
use std::fs;
use std::path::PathBuf;

use cortex_config::{Env, load};
use cortex_server::assembly::{assemble, startup_line};

#[test]
fn as12_identity_alone_is_refused_naming_the_core_pieces_left_out() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("asm-missing-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("dir");
    let file = dir.join("cortex.toml");
    fs::write(&file, "[modules]\nenabled = [\"identity\"]\n").expect("config");
    let cfg = load(&file, &Env::from_pairs(&[("HOME", "/home/x")])).expect("a valid config");
    let line = startup_line(&assemble(&cfg).expect_err("a core piece left out is refused"));
    assert!(
        line.contains("audit") && line.contains("tokens"),
        "both core pieces that were left out are named: {line}"
    );
    assert!(!line.contains('\n'));
}
