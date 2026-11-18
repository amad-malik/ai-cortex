//! `cargo run -p xtask -- <check>` (AGENTS.md). Exit 0 when the check passes, 1 when it finds a
//! violation, 2 on a usage or tool error.
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask sits in the repository root")
        .to_path_buf()
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = repo_root();
    match args.first().map(String::as_str) {
        Some("fitness") => match xtask::fitness(&root.join("Cargo.toml")) {
            Ok(v) if v.is_empty() => ExitCode::SUCCESS,
            Ok(v) => {
                for x in v {
                    eprintln!(
                        "fitness: {:?}: {} depends on {}",
                        x.rule, x.krate, x.depends_on
                    );
                }
                ExitCode::from(1)
            }
            Err(e) => {
                eprintln!("fitness: {e}");
                ExitCode::from(2)
            }
        },
        Some("licences") => {
            let manifest = args
                .get(1)
                .map(PathBuf::from)
                .unwrap_or_else(|| root.join("Cargo.toml"));
            match xtask::licences(&manifest, &root.join("deny.toml")) {
                Ok(xtask::LicenceVerdict::Accepted) => ExitCode::SUCCESS,
                Ok(xtask::LicenceVerdict::Refused { detail }) => {
                    eprintln!("licences: refused: {detail}");
                    ExitCode::from(1)
                }
                Err(e) => {
                    eprintln!("licences: {e}");
                    ExitCode::from(2)
                }
            }
        }
        _ => {
            eprintln!("usage: cargo run -p xtask -- fitness | licences [<Cargo.toml>]");
            ExitCode::from(2)
        }
    }
}
