//! `cortex audit list`: GET /v2/audit and print the records (AU-01 to AU-20, `mvp/tests/cortex-cli-audit.md`). The first
//! command that sends a token (W1-43 Q-1: `--token-file`, then `CORTEX_TOKEN`, then `CORTEX_TOKEN_FILE`).
//!
//! The server stays the authority on the cursor (W1-46); the CLI checks only what it can say without the server: `--limit` is
//! 1 to 200 and `--after` is 1 to 256 printable ASCII characters (usage errors, nothing is sent). Every server string that
//! reaches stdout or stderr goes through the ONE display boundary (`safe_text`, `json_escape_unsafe`). Nothing is printed
//! unless the whole command succeeded.

use clap::{Arg, ArgAction, ArgMatches, Command};
use cortex_client::display::{BUDGET_IDENT, json_escape_unsafe, safe_text};
use cortex_client::env::{resolve_server, resolve_token};
use cortex_client::{Client, ClientConfig, ClientError, Method, Mutates, Request, Response, exit};
use serde_json::{Map, Value};

use crate::registry::{Ctx, Format, Group, Output};

/// `--all` stops after this many pages and says so on stderr (cox 7 Nov 12:49Z).
const MAX_PAGES: usize = 20;
/// The most characters a cursor may have, to send and to take back (D1).
const CURSOR_MAX: usize = 256;
/// A target or details value is cut to this many characters on a terminal; every other field to `BUDGET_IDENT`.
const VALUE_BUDGET: usize = 128;
/// The members every record has (S3 `AuditRecord`).
const REQUIRED: [&str; 5] = ["id", "at", "action", "actor_kind", "outcome"];

pub fn group() -> Group {
    Group {
        name: "audit",
        owner: "sophi@helix",
        implements: &["cortex audit list"],
        commands,
        run,
    }
}

fn commands() -> Vec<Command> {
    vec![
        Command::new("audit")
            .about("Read the audit log of the token's project")
            .subcommand_required(true)
            .arg_required_else_help(true)
            .subcommand(
                Command::new("list")
                    .about(
                        "List the audit records of the token's project, oldest first (project lead only). The log is PLAIN in \
                         0.2.001: no chain, no tamper evidence",
                    )
                    .arg(
                        Arg::new("limit")
                            .long("limit")
                            .value_name("N")
                            .value_parser(parse_limit)
                            .help("Records per page, 1 to 200 [default: the server's 50]"),
                    )
                    .arg(
                        Arg::new("after")
                            .long("after")
                            .value_name("CURSOR")
                            .value_parser(parse_cursor)
                            .help(
                                "Continue after this cursor (the next value of a page): 1 to 256 printable ASCII characters",
                            ),
                    )
                    .arg(
                        Arg::new("all")
                            .long("all")
                            .action(ArgAction::SetTrue)
                            .help("Follow the pages to the end, at most 20 pages; if more records exist it stops, says 'incomplete' on stderr and exits 8"),
                    ),
            ),
    ]
}

/// 1 to 200, digits only (no sign, no space, nothing but ASCII digits).
fn parse_limit(text: &str) -> Result<u16, String> {
    let digits = !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    match text.parse::<u16>() {
        Ok(n) if digits && (1..=200).contains(&n) => Ok(n),
        _ => Err("a whole number from 1 to 200".to_owned()),
    }
}

/// 1 to 256 characters of printable ASCII (0x20 to 0x7E).
fn cursor_ok(text: &str) -> bool {
    (1..=CURSOR_MAX).contains(&text.len()) && text.bytes().all(|b| (0x20..=0x7e).contains(&b))
}

fn parse_cursor(text: &str) -> Result<String, String> {
    if cursor_ok(text) {
        Ok(text.to_owned())
    } else {
        Err("1 to 256 printable ASCII characters".to_owned())
    }
}

fn run(ctx: &Ctx<'_>, _command: &str, matches: &ArgMatches) -> Result<Output, ClientError> {
    let Some(("list", sub)) = matches.subcommand() else {
        return Err(ClientError::BadRequest(
            "cortex audit takes the subcommand list".to_owned(),
        ));
    };
    let limit = sub.get_one::<u16>("limit").copied();
    let all = sub.get_flag("all");
    let mut cursor = sub.get_one::<String>("after").cloned();

    let endpoint = resolve_server(ctx.server.as_deref(), ctx.env)?;
    let token = resolve_token(ctx.token_file.as_deref(), ctx.env)?;
    let client = Client::new(ClientConfig::new(endpoint, Some(token)));

    let mut pages: Vec<Page> = Vec::new();
    let mut more: Option<String> = None;
    loop {
        let mut request = Request::new(Method::Get, "/v2/audit", &[], Mutates::No)?;
        if let Some(n) = limit {
            request = request.query("limit", &n.to_string());
        }
        if let Some(c) = &cursor {
            request = request.query("after", c);
        }
        let response = client.send(&request)?;
        let page = Page::decode(&response)?;
        let next = page.next.clone();
        pages.push(page);
        let Some(next) = next.filter(|_| all) else {
            break;
        };
        check_cursor(&next, cursor.as_deref(), &response)?;
        if pages.len() == MAX_PAGES {
            more = Some(next);
            break;
        }
        cursor = Some(next);
    }

    let mut out = Output::default();
    match ctx.format {
        Format::Json => out.stdout = render_json(&pages, all, more.as_deref()),
        Format::Md => {
            for page in &pages {
                for record in page.records() {
                    out.stdout.push_str(&render_line(record));
                    out.stdout.push('\n');
                }
            }
        }
    }
    if all {
        if let Some(last) = &more {
            // W1-90: a capped listing is INCOMPLETE, and the exit status says so (stdout still holds what was fetched).
            out.exit = exit::INCOMPLETE;
            out.stderr = format!(
                "incomplete: {MAX_PAGES} pages were printed and more records exist; continue with --after {}\n",
                safe_text(last, CURSOR_MAX)
            );
        }
    } else if ctx.format == Format::Md
        && let Some(next) = pages.first().and_then(|p| p.next.as_deref())
    {
        out.stderr = format!("next: {}\n", safe_text(next, CURSOR_MAX));
    }
    Ok(out)
}

/// One page of the answer: the members as received, and `next`.
struct Page {
    members: Map<String, Value>,
    next: Option<String>,
}

impl Page {
    fn decode(response: &Response) -> Result<Page, ClientError> {
        let bad = |what: &str| ClientError::Decode {
            what: format!("the audit page is not the one the contract describes: {what}"),
            request_id: response.request_id.clone(),
        };
        let Value::Object(members) = response.json()? else {
            return Err(bad("page is not an object"));
        };
        let items = members
            .get("items")
            .and_then(Value::as_array)
            .ok_or_else(|| bad("items is missing or not an array"))?;
        let next = match members.get("next") {
            Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            _ => return Err(bad("next is missing or neither text nor null")),
        };
        for (i, item) in items.iter().enumerate() {
            let record = item
                .as_object()
                .ok_or_else(|| bad(&format!("items[{i}] is not an object")))?;
            for key in REQUIRED {
                if !record.get(key).is_some_and(Value::is_string) {
                    return Err(bad(&format!("items[{i}].{key} is missing or not text")));
                }
            }
            for key in ["actor_id", "request_id"] {
                if !record.get(key).is_none_or(|v| v.is_null() || v.is_string()) {
                    return Err(bad(&format!("items[{i}].{key} is neither text nor null")));
                }
            }
            for key in ["target", "details"] {
                if !record.get(key).is_none_or(Value::is_object) {
                    return Err(bad(&format!("items[{i}].{key} is not an object")));
                }
            }
        }
        Ok(Page { members, next })
    }

    /// The records, already checked to be objects.
    fn records(&self) -> impl Iterator<Item = &Map<String, Value>> {
        self.members
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_object)
    }
}

/// The cursor a page gives in `--all` is sent back, so it must be one this command would take from the user (D1), and it
/// must move on from the one just sent.
fn check_cursor(next: &str, sent: Option<&str>, response: &Response) -> Result<(), ClientError> {
    let bad = |what: &str| ClientError::Decode {
        what: format!("the audit page is not the one the contract describes: {what}"),
        request_id: response.request_id.clone(),
    };
    if !cursor_ok(next) {
        return Err(bad(
            "next is not a cursor this command can send back (1 to 256 printable ASCII characters)",
        ));
    }
    if sent == Some(next) {
        return Err(bad("next did not move on from the cursor just sent"));
    }
    Ok(())
}

/// json: the one page as received (same members and values, on one line), or with `--all` one object with every record.
fn render_json(pages: &[Page], all: bool, more: Option<&str>) -> String {
    let value = if all {
        let items: Vec<Value> = pages
            .iter()
            .flat_map(|p| p.records().cloned().map(Value::Object))
            .collect();
        let mut o = Map::new();
        o.insert("items".to_owned(), Value::Array(items));
        o.insert(
            "next".to_owned(),
            more.map_or(Value::Null, |c| Value::String(c.to_owned())),
        );
        Value::Object(o)
    } else {
        pages
            .first()
            .map_or(Value::Null, |p| Value::Object(p.members.clone()))
    };
    // serde_json writes the C0 controls as escapes; DEL, C1 and the format characters would be written raw. Escape them too
    // (lossless: the line parses to the same strings).
    format!("{}\n", json_escape_unsafe(&value.to_string()))
}

/// A field of the md line: a server string, safe and cut to `budget`.
fn field(record: &Map<String, Value>, key: &str, budget: usize) -> String {
    match record.get(key) {
        Some(Value::String(s)) => safe_text(s, budget),
        _ => "-".to_owned(),
    }
}

/// `target` or `details`: the pairs as `key=value` sorted by key and joined by `,`, or `-`.
fn pairs(record: &Map<String, Value>, key: &str) -> String {
    let Some(Value::Object(members)) = record.get(key) else {
        return "-".to_owned();
    };
    if members.is_empty() {
        return "-".to_owned();
    }
    members
        .iter()
        .map(|(k, v)| {
            let shown = match v {
                Value::String(s) => safe_text(s, VALUE_BUDGET),
                other => safe_text(&other.to_string(), VALUE_BUDGET),
            };
            format!("{}={shown}", safe_text(k, BUDGET_IDENT))
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// `at action actor_kind:actor_id outcome target request_id details`: seven fields, a missing one is `-`.
fn render_line(record: &Map<String, Value>) -> String {
    format!(
        "{} {} {}:{} {} {} {} {}",
        field(record, "at", BUDGET_IDENT),
        field(record, "action", BUDGET_IDENT),
        field(record, "actor_kind", BUDGET_IDENT),
        field(record, "actor_id", BUDGET_IDENT),
        field(record, "outcome", BUDGET_IDENT),
        pairs(record, "target"),
        field(record, "request_id", BUDGET_IDENT),
        pairs(record, "details"),
    )
}
