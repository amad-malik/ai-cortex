//! The `cortex` command line (Wave 1 piece 10; the skeleton): the registry, the global flags, `--version` and `doctor`.
//! Test definitions: `mvp/tests/cortex-client-and-cli.md`. The exit-code table is `cortex_client::exit` (W1-43 Q-2).

pub mod coverage;
pub mod groups;
pub mod registry;
pub mod version;

use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;

use clap::error::ErrorKind;
use cortex_client::display::{
    BUDGET_DETAIL, BUDGET_FIX, BUDGET_IDENT, BUDGET_TITLE, bounded, json_escape_unsafe,
};
use cortex_client::env::Lookup;
use cortex_client::{ClientError, exit};
use serde_json::{Map, Value, json};

use registry::{Ctx, Format, Registry};

/// Run `cortex` with `args` (the first is the program name) and return the exit code. Results go to `out`, one diagnostic
/// to `err`; a failure writes nothing to `out`.
pub fn run<I, T>(args: I, env: Lookup<'_>, out: &mut dyn Write, err: &mut dyn Write) -> u8
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let registry = match Registry::new(groups::all()) {
        Ok(r) => r,
        Err(e) => {
            let _ = writeln!(err, "cortex: {e}");
            return exit::OTHER;
        }
    };
    let matches = match registry.command().try_get_matches_from(args) {
        Ok(m) => m,
        Err(e) => {
            return match e.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                    let _ = write!(out, "{}", e.render());
                    exit::OK
                }
                _ => {
                    let _ = write!(err, "{}", e.render());
                    exit::USAGE
                }
            };
        }
    };
    let Some((name, sub)) = matches.subcommand() else {
        let _ = writeln!(err, "cortex: no command given");
        return exit::USAGE;
    };
    let Some(group) = registry.group_of_command(name) else {
        let _ = writeln!(err, "cortex: no group provides {name}");
        return exit::OTHER;
    };
    let format = match sub.get_one::<String>("format").map(String::as_str) {
        Some("json") => Format::Json,
        _ => Format::Md,
    };
    let ctx = Ctx {
        server: sub.get_one::<String>("server").cloned(),
        token_file: sub.get_one::<PathBuf>("token-file").cloned(),
        format,
        env,
    };
    match (group.run)(&ctx, name, sub) {
        Ok(output) => {
            let _ = out.write_all(output.stdout.as_bytes());
            let _ = err.write_all(output.stderr.as_bytes());
            output.exit
        }
        Err(e) => {
            report(&e, format, err);
            exit::exit_code(&e)
        }
    }
}

/// What a failure shows, whatever its type: the members of the JSON line (EX-04). A Problem's free text is cut to the display
/// budgets here (`bounded`: not mapped, the json escape below keeps the characters); nothing else is changed.
#[derive(Default)]
struct Facts {
    kind: &'static str,
    title: String,
    status: Option<u16>,
    detail: Option<String>,
    fix: Option<String>,
    request_id: Option<String>,
}

fn facts(error: &ClientError) -> Facts {
    let plain = |kind| Facts {
        kind,
        title: error.to_string(),
        ..Facts::default()
    };
    match error {
        ClientError::Problem(p) => Facts {
            kind: "problem",
            title: bounded(&p.title, BUDGET_TITLE),
            status: Some(p.status),
            detail: p.detail.as_ref().map(|d| bounded(d, BUDGET_DETAIL)),
            fix: Some(bounded(&p.fix, BUDGET_FIX)),
            request_id: Some(bounded(&p.request_id, BUDGET_IDENT)),
        },
        ClientError::Http {
            status, request_id, ..
        } => Facts {
            status: Some(*status),
            request_id: request_id.clone(),
            ..plain("http")
        },
        ClientError::Busy { .. } => Facts {
            status: Some(429),
            ..plain("busy")
        },
        ClientError::UnexpectedRedirect { status } => Facts {
            status: Some(*status),
            ..plain("redirect")
        },
        ClientError::Decode { request_id, .. } => Facts {
            request_id: request_id.clone(),
            ..plain("decode")
        },
        ClientError::Transport { .. } => plain("transport"),
        ClientError::Token(_) => plain("token"),
        ClientError::Endpoint(_) => plain("endpoint"),
        ClientError::Key(_) => plain("key"),
        ClientError::BadRequest(_) => plain("bad_request"),
        ClientError::RequestTooLarge { .. } => plain("request_too_large"),
        ClientError::ResponseTooLarge { .. } => plain("response_too_large"),
    }
}

/// One diagnostic on `err`: the typed error's text, or with `--format json` one JSON line (EX-03, EX-04).
fn report(error: &ClientError, format: Format, err: &mut dyn Write) {
    if format == Format::Md {
        let _ = writeln!(err, "{error}");
        return;
    }
    let f = facts(error);
    let mut e = Map::new();
    // a Problem's own code is the `type` member; every other failure has a short kind
    let code = match error {
        ClientError::Problem(p) => bounded(&p.code, BUDGET_IDENT),
        _ => f.kind.to_owned(),
    };
    e.insert("code".into(), json!(code));
    e.insert("title".into(), json!(f.title));
    if let Some(s) = f.status {
        e.insert("status".into(), json!(s));
    }
    if let Some(d) = f.detail {
        e.insert("detail".into(), json!(d));
    }
    if let Some(x) = f.fix {
        e.insert("fix".into(), json!(x));
    }
    if let Some(r) = f.request_id {
        e.insert("request_id".into(), json!(r));
    }
    e.insert("exit".into(), json!(exit::exit_code(error)));
    let line = Value::Object(Map::from_iter([("error".to_owned(), Value::Object(e))])).to_string();
    // serde_json escapes the C0 controls; DEL, C1 and the format characters of the list would be written raw. Escape them too
    // (lossless: the line parses to the same strings), so no unsafe character reaches the terminal (ellis F-1).
    let _ = writeln!(err, "{}", json_escape_unsafe(&line));
}
