//! `cortex doctor`: GET /v2/health and print it (DR-01 to DR-05). Reads no token: the health routes are open.

use clap::{ArgMatches, Command};
use cortex_client::display::{BUDGET_IDENT, safe_text};
use cortex_client::env::resolve_server;
use cortex_client::{Client, ClientConfig, ClientError, Method, Mutates, Request, Response, exit};
use serde_json::Value;

use crate::registry::{Ctx, Format, Group, Output};

pub fn group() -> Group {
    Group {
        name: "doctor",
        owner: "sophi@helix",
        implements: &["cortex doctor"],
        commands,
        run,
    }
}

fn commands() -> Vec<Command> {
    vec![
        Command::new("doctor")
            .about("Check that the Cortex server answers (GET /v2/health); reads no token"),
    ]
}

fn run(ctx: &Ctx<'_>, _command: &str, _matches: &ArgMatches) -> Result<Output, ClientError> {
    let endpoint = resolve_server(ctx.server.as_deref(), ctx.env)?;
    let client = Client::new(ClientConfig::new(endpoint, None));
    let response =
        client.send(&Request::new(Method::Get, "/v2/health", &[], Mutates::No)?.open())?;
    let health = Health::decode(&response)?;

    let mut out = Output {
        exit: if health.ready { exit::OK } else { exit::BUSY },
        ..Output::default()
    };
    match ctx.format {
        Format::Json => {
            out.stdout = format!("{}\n", String::from_utf8_lossy(&response.body).trim_end());
        }
        Format::Md => out.stdout = health.render(),
    }
    if health.backup != "ok" {
        out.stderr = format!("backup: {}\n", health.backup);
    }
    Ok(out)
}

/// The `Health` document of the S3 twin: state, version, ready, pieces, backup (required); last_good_dump_at (null or text).
struct Health {
    state: String,
    version: String,
    ready: bool,
    backup: String,
    last_good_dump_at: Option<String>,
    pieces: Vec<(String, bool, bool)>,
}

/// Text from the server shown on a terminal: the ONE list of unsafe characters and a budget (ellis R-1; cox 7 Nov 12:24Z).
fn plain(text: &str) -> String {
    safe_text(text, BUDGET_IDENT)
}

impl Health {
    fn decode(response: &Response) -> Result<Health, ClientError> {
        let bad = |what: &str| ClientError::Decode {
            what: format!("the health document is not the one the contract describes: {what}"),
            request_id: response.request_id.clone(),
        };
        let value = response.json()?;
        let o = value.as_object().ok_or_else(|| bad("not an object"))?;
        let text = |k: &str| {
            o.get(k)
                .and_then(Value::as_str)
                .map(plain)
                .ok_or_else(|| bad(k))
        };
        let ready = o
            .get("ready")
            .and_then(Value::as_bool)
            .ok_or_else(|| bad("ready"))?;
        let backup = text("backup")?;
        if !["ok", "stale", "none"].contains(&backup.as_str()) {
            return Err(bad("backup"));
        }
        let last_good_dump_at = match o.get("last_good_dump_at") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(plain(s)),
            Some(_) => return Err(bad("last_good_dump_at")),
        };
        let mut pieces = Vec::new();
        for p in o
            .get("pieces")
            .and_then(Value::as_array)
            .ok_or_else(|| bad("pieces"))?
        {
            let p = p.as_object().ok_or_else(|| bad("pieces"))?;
            pieces.push((
                p.get("name")
                    .and_then(Value::as_str)
                    .map(plain)
                    .ok_or_else(|| bad("pieces.name"))?,
                p.get("enabled")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| bad("pieces.enabled"))?,
                p.get("ready")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| bad("pieces.ready"))?,
            ));
        }
        Ok(Health {
            state: text("state")?,
            version: text("version")?,
            ready,
            backup,
            last_good_dump_at,
            pieces,
        })
    }

    fn render(&self) -> String {
        let mut s = format!(
            "state: {}\nversion: {}\nready: {}\nbackup: {}\nlast_good_dump_at: {}\npieces:\n",
            self.state,
            self.version,
            self.ready,
            self.backup,
            self.last_good_dump_at.as_deref().unwrap_or("none"),
        );
        for (name, enabled, ready) in &self.pieces {
            s.push_str(&format!("  {name}: enabled={enabled} ready={ready}\n"));
        }
        s
    }
}
