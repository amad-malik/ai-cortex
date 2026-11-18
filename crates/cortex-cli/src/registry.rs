//! The command registry: each owner adds his own `Group` in his own file under `src/groups/` and one line in
//! `src/groups/mod.rs`. A group is a name, an owner, the `x-cli` strings of the S3 twin it implements, the top-level
//! commands it contributes and the function that runs them. The registry refuses two groups of one name and two commands
//! of one name at startup (CL-01), and sorts everything it prints (CL-06).

use std::collections::BTreeSet;
use std::path::PathBuf;

use clap::{Arg, ArgMatches, Command};
use cortex_client::ClientError;
use cortex_client::env::Lookup;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Md,
    Json,
}

/// The global flags and the environment lookup, handed to every command.
pub struct Ctx<'a> {
    pub server: Option<String>,
    pub token_file: Option<PathBuf>,
    pub format: Format,
    pub env: Lookup<'a>,
}

/// What a command produced. `exit` is the process exit code.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub exit: u8,
}

pub struct Group {
    pub name: &'static str,
    pub owner: &'static str,
    /// The `x-cli` strings of the S3 twin this group implements, exactly as the twin gives them.
    pub implements: &'static [&'static str],
    /// The top-level clap commands this group contributes.
    pub commands: fn() -> Vec<Command>,
    /// Run one of them: the top-level command name and its matches.
    pub run: fn(&Ctx<'_>, &str, &ArgMatches) -> Result<Output, ClientError>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    #[error("two groups are named {0}")]
    DuplicateGroup(String),
    #[error("the command {0} is registered twice")]
    DuplicateCommand(String),
}

pub struct Registry {
    groups: Vec<Group>,
}

impl Registry {
    /// Refuses two groups of one name and two commands of one name; keeps the groups sorted by name.
    pub fn new(mut groups: Vec<Group>) -> Result<Registry, RegistryError> {
        let mut group_names = BTreeSet::new();
        let mut command_names = BTreeSet::new();
        for g in &groups {
            if !group_names.insert(g.name) {
                return Err(RegistryError::DuplicateGroup(g.name.to_owned()));
            }
            for c in (g.commands)() {
                if !command_names.insert(c.get_name().to_owned()) {
                    return Err(RegistryError::DuplicateCommand(c.get_name().to_owned()));
                }
            }
        }
        groups.sort_by(|a, b| a.name.cmp(b.name));
        Ok(Registry { groups })
    }

    /// The root clap command: the global flags and every group's commands, sorted by name.
    pub fn command(&self) -> Command {
        let mut commands: Vec<Command> = self.groups.iter().flat_map(|g| (g.commands)()).collect();
        commands.sort_by(|a, b| a.get_name().cmp(b.get_name()));
        let mut root = Command::new("cortex")
            .version(crate::version::line())
            .about("The Cortex command line")
            .arg_required_else_help(true)
            .subcommand_required(true)
            .args(global_args());
        for c in commands {
            root = root.subcommand(c);
        }
        root
    }

    pub fn group_of_command(&self, command: &str) -> Option<&Group> {
        self.groups
            .iter()
            .find(|g| (g.commands)().iter().any(|c| c.get_name() == command))
    }

    /// Every `x-cli` string some group implements.
    pub fn registered_cli(&self) -> BTreeSet<&'static str> {
        self.groups
            .iter()
            .flat_map(|g| g.implements.iter().copied())
            .collect()
    }

    pub fn groups(&self) -> &[Group] {
        &self.groups
    }
}

/// The three global flags (CL-03): `--server`, `--token-file`, `--format`.
pub fn global_args() -> Vec<Arg> {
    vec![
        Arg::new("server")
            .long("server")
            .value_name("URL")
            .global(true)
            .help("Server address, exactly http://HOST:PORT on a loopback host [default: $CORTEX_SERVER, else http://127.0.0.1:8601]"),
        Arg::new("token-file")
            .long("token-file")
            .value_name("PATH")
            .value_parser(clap::value_parser!(PathBuf))
            .global(true)
            .help("File that holds the token, mode 0600 [default: $CORTEX_TOKEN, else $CORTEX_TOKEN_FILE]"),
        Arg::new("format")
            .long("format")
            .value_name("FORMAT")
            .value_parser(clap::builder::PossibleValuesParser::new(["md", "json"]))
            .default_value("md")
            .global(true)
            .help("Output format"),
    ]
}
