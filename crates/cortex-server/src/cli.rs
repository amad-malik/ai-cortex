//! The command line of `cortex-server` (contract r0.3 section 1). Hand-written: clap is not in `Cargo.lock`.
//!
//! ```text
//! cortex-server --version
//! cortex-server serve     [--config PATH]
//! cortex-server migrate   [--config PATH]
//! cortex-server bootstrap [--config PATH] --project P --lead L --token-file PATH [--require-existing]
//! ```
//! `--config` defaults to [`DEFAULT_CONFIG`] (the S5 compose file mounts the config there).

use std::path::PathBuf;

/// Where the S5 compose install mounts the config.
pub const DEFAULT_CONFIG: &str = "/etc/cortex/cortex.toml";

/// Exit codes. 2 is the usage error (the convention every command-line parser uses); 3 is the `--require-existing`
/// refusal and is DIFFERENT, so a caller can tell an old server that does not know the flag (exit 2) from a refusal.
pub const EXIT_OK: i32 = 0;
pub const EXIT_FAILURE: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_REFUSED: i32 = 3;

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Version,
    Serve { config: PathBuf },
    Migrate { config: PathBuf },
    Bootstrap(BootstrapArgs),
}

#[derive(Debug, PartialEq, Eq)]
pub struct BootstrapArgs {
    pub config: PathBuf,
    pub project: String,
    pub lead: String,
    pub token_file: PathBuf,
    pub require_existing: bool,
}

/// A usage error: the message goes to stderr and the exit code is [`EXIT_USAGE`].
#[derive(Debug, PartialEq, Eq)]
pub struct UsageError(pub String);

/// Parse the arguments after the program name.
pub fn parse(args: &[String]) -> Result<Command, UsageError> {
    let Some(first) = args.first() else {
        return Err(usage(
            "no command: expected --version, serve, migrate or bootstrap",
        ));
    };
    let rest = &args[1..];
    match first.as_str() {
        "--version" => {
            if rest.is_empty() {
                Ok(Command::Version)
            } else {
                Err(usage("--version takes no other argument"))
            }
        }
        "serve" => Ok(Command::Serve {
            config: config_only(rest)?,
        }),
        "migrate" => Ok(Command::Migrate {
            config: config_only(rest)?,
        }),
        "bootstrap" => bootstrap(rest),
        _ => Err(usage(
            "unknown command: expected --version, serve, migrate or bootstrap",
        )),
    }
}

fn usage(m: &str) -> UsageError {
    UsageError(m.to_owned())
}

/// What a command line held: its valued flags and its boolean flags. An operator-supplied token is NEVER echoed in a message
/// (names and paths are operator input; a message names the FLAG, not the value).
struct Collected {
    vals: Vec<(&'static str, String)>,
    bools: Vec<&'static str>,
}

fn collect(
    args: &[String],
    valued: &[&'static str],
    boolean: &[&'static str],
) -> Result<Collected, UsageError> {
    let mut c = Collected {
        vals: Vec::new(),
        bools: Vec::new(),
    };
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if let Some(&flag) = valued.iter().find(|f| **f == a) {
            if c.vals.iter().any(|(f, _)| *f == flag) {
                return Err(UsageError(format!("{flag} was given twice")));
            }
            let Some(v) = args.get(i + 1) else {
                return Err(UsageError(format!("{flag} needs a value")));
            };
            c.vals.push((flag, v.clone()));
            i += 2;
        } else if let Some(&flag) = boolean.iter().find(|f| **f == a) {
            if c.bools.contains(&flag) {
                return Err(UsageError(format!("{flag} was given twice")));
            }
            c.bools.push(flag);
            i += 1;
        } else {
            return Err(usage("an argument is not valid for this command"));
        }
    }
    Ok(c)
}

fn config_only(args: &[String]) -> Result<PathBuf, UsageError> {
    let c = collect(args, &["--config"], &[])?;
    Ok(c.vals
        .into_iter()
        .next()
        .map_or_else(|| PathBuf::from(DEFAULT_CONFIG), |(_, v)| PathBuf::from(v)))
}

fn bootstrap(args: &[String]) -> Result<Command, UsageError> {
    let c = collect(
        args,
        &["--config", "--project", "--lead", "--token-file"],
        &["--require-existing"],
    )?;
    let get = |flag: &str| {
        c.vals
            .iter()
            .find(|(f, _)| *f == flag)
            .map(|(_, v)| v.clone())
    };
    let need = |flag: &'static str| -> Result<String, UsageError> {
        match get(flag) {
            Some(v) if !v.is_empty() => Ok(v),
            Some(_) => Err(UsageError(format!("{flag} must not be empty"))),
            None => Err(UsageError(format!("{flag} is required"))),
        }
    };
    Ok(Command::Bootstrap(BootstrapArgs {
        config: get("--config").map_or_else(|| PathBuf::from(DEFAULT_CONFIG), PathBuf::from),
        project: need("--project")?,
        lead: need("--lead")?,
        token_file: PathBuf::from(need("--token-file")?),
        require_existing: c.bools.contains(&"--require-existing"),
    }))
}
