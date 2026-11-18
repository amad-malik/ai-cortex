---
id: cortex-reference-overview
title: Cortex v2 — the API in one page
kind: reference
status: draft
owner: quill
updated: 2026-11-06
product: cortex
audience: developer
maturity: planned
---

# The API in one page

Cortex has **one surface and two doors in this release**: HTTP and the `cortex` CLI. A worker reaches the same verbs over
both; neither has a capability the other lacks, and neither is a second implementation. **MCP is a planned third door and
is not in 0.2.001** — the contract does not contain the word, and the release train owns the release that adds it. This page is the map: what
the surface is, how it is grouped, which door to use for what, and how to read the reference that lists every route.

**Built from** the S3 contract, draft **r2.7**, pinned at `mvp/wave1-pins-20261106-S3r27/` (YAML `8a4c0cb1…`,
JSON twin `1d2c4b2f…`) against **S2 revision 8**. Counts here are facts about that pin and are checked against it; the
per-route detail lives in the [generated reference](../generated/API-REFERENCE.md), and the shared rules in
[conventions](conventions.md).

## 1. The shape

**33 operations across 30 paths, in nine groups** — the contract's tag list holds **ten**, and the tenth is `admin`:
host-only commands that are never on this HTTP listener. So a reader who recounts tags gets ten and is not wrong. Three are **open** — the health routes, which ignore any
`Authorization` header, valid or not, and answer as if none were sent — and every other route needs a worker token and
exactly one scope.

| Group | Operations | Scopes it uses | What it is for |
|---|---|---|---|
| **health** | 3 | — (open) | is the process alive, is it ready, what is this build |
| **identity** | 7 | `roster:read`, `worker:create`, `worker:retire`, `team:create`, `team:assign` | projects, teams, workers: who exists and what they may be |
| **tokens** | 6 | `token:issue`, `token:list`, `token:revoke`, `self:read`, `self:revoke` | issuing, listing and revoking credentials; who am I |
| **audit** | 1 | `audit:read` | the project's audit rows (a plain log in 0.2.001) |
| **records** | 5 | `records:read`, `records:write`, `records:lineage` | write, read, supersede, invalidate; the reserved sections |
| **search** | 1 | `records:read` | keyword search over the project's records |
| **handoffs** | 9 | `handoff:create`, `handoff:read`, `handoff:claim`, `handoff:hold`, `handoff:review`, `handoff:manage` | the queue: create, claim, return, accept, rework, cancel, release |
| **boot** | 1 | `self:read` | the briefing a worker starts from |
| **policy** | 0 | — | no routes of its own: it owns the scope table and the target rules that every other group is checked against |

**Twenty scopes** exist in the frozen table, and each route declares exactly one. A scope says *may this principal*; a
**target rule** says *may it do this to this row* — two different questions, and section 6 of
[conventions](conventions.md) fixes which refusal wins when both fail.

## 2. Which door for what

- **HTTP** is the contract. Everything else is a client of it.
- **The CLI** (`cortex …`) is for a human or a script on the host: `cortex boot`, `cortex handoff claim`, `cortex log`,
  `cortex record show`, `cortex rules`, `cortex token revoke`, `cortex doctor`. Every route in the reference names its CLI command **or says it has none** — the two plain health checks carry
  `x-cli: "-"`, which the contract defines as "a route with no CLI command". And a command the contract **names** that the
  binary does not answer is a **build failure**: the reference is generated from the contract, so the parity is checked
  rather than promised.
- **MCP** (planned, not in 0.2.001) will expose the same verbs to a worker that would rather call a tool than build an
  HTTP request: same scopes, same mistakes, same refusals.

**Why one surface:** a second write path would need its own rules, and then two workers on two harnesses would get
different answers to the same question. The board **renders** memory; it never writes it.

## 3. What a request carries

| Thing | Required on | Note |
|---|---|---|
| `Authorization: Bearer <token>` | every route but the three health routes | a token is a **file** on the worker's host, `0600`, never printed |
| exactly one scope | every token route | a route declares one; a principal with several still passes one check |
| `Idempotency-Key` | every mutating route | 8–128 printable ASCII; bound to project, principal, method, path and body hash |
| `application/json` body | every route that declares one | a known path with the wrong method is `405` with `Allow`, not `404` |

Every response carries `Cortex-Request-Id` (a new UUIDv7; a client-sent value is ignored), every refusal carries
`Cache-Control: no-store`, every `401` carries `WWW-Authenticate: Bearer`, every `429` carries `Retry-After: 1`, and a
replayed success carries `Cortex-Idempotent-Replay: true`.

## 4. What comes back

- A **list** answers `{items, next}` and pages by keyset; `limit` defaults to 50, caps at 200 (search: 1–50, default 10).
- A **record** answers the row plus its state and lineage; a superseded or invalidated record is still readable by id,
  because history is the point.
- A **refusal** answers `application/problem+json` with a typed `type`, and a client branches on that, never on the
  human text.
- A **mutating success** answers only ids and state — `id`, `ok`, `state`, `round`, `created_at`, `section_revision` —
  **except `tokens.issue`**, which answers the token secret once and is never replayed: a retry with the same key is
  `409 already_issued`. The narrow reply is what keeps a replay from ever revealing more than the first answer did.

> **The generated page is beside this one now.** `docs/generated/API-REFERENCE.md` is produced by piece 13's renderer from
> the pinned contract and is committed here, so the link at the top of this page resolves. Two things about it worth knowing: it is a **build
> product** — correct the contract and re-run the renderer, never edit it — and it carries the contract's own version in its
> first lines, so a reader can always tell which pin they are looking at. Where this page and that one disagree, the contract
> decides and one of the two is stale.

## 5. How to read the reference

The generated page is organised **by route, in the order the doc-pack spec fixes**, and each route entry answers the
same five questions in the same order: *the task it performs · who may call it (scope and class) · the target rule it
enforces · what it answers and how it can fail · the fields and the bytes each one takes*. Read it as a lookup, not as a
story: this page is the story.

Two things it will not tell you, deliberately: what to **do** when a call fails (that is the operator's guide) and why a
route exists at all (that is the design pages). A reference that explains itself becomes a second design document, and
then the two disagree.

## 6. Where 0.2.001 ends

The surface is deliberately small, and the edges are stated rather than implied. Absent from this release: **MCP**, event
streaming and the doorbell, approvals, steering, skills distribution, the document and media workers, vector and hybrid
search, import from v1, and the host admin commands (which live on a Unix socket, never on this listener). **`conversation`
records, the `project-brief` section and the denial log are named in the contract and refused** — they are next releases'
verbs, present so that a client author can see the shape early and absent so that nobody builds against a promise.

## 7. Known limits of the CLI's output boundary

The note's own paragraph, copied rather than paraphrased because the last two versions of this section each lost a qualifier in the retelling:

> What the CLI does, and where it stops. The ERROR DIAGNOSTICS the CLI prints (title, detail, fix) go through `cortex_client::display::safe_text` (md) or `json_escape_unsafe` (json lines), and the Health strings of `cortex doctor` in MD output go through `safe_text`; the token is redacted in those. One list of unsafe characters decides: every control character, U+2028, U+2029, U+200E, U+200F, U+061C, U+202A..U+202E, U+2066..U+2069 and the tag block U+E0000..U+E007F. ONE PATH IS EXPLICITLY EXCLUDED: `cortex doctor --format json` prints the response body as received, so that raw body is neither passed through those helpers nor token-redacted by them (item 5); this note does not claim universal redaction.

That layer has **seven known limits**, accepted as stated for 0.2.001 with **no claim of a typed boundary** (cox's decision of
2026-11-07; vera's E-2, N-3, F-4, and her D1 on exactly the overclaim this paragraph used to carry):

1. **No typed boundary.** The guard `n3_no_group_file_names_problem_or_reads_a_servers_text_fields` is a *lexical tripwire*: it
   scans `src/groups/*.rs` for the names `Problem`, `.detail`, `.fix`, `.title` with a naive split of comments. A server string
   reached another way — through `Busy.last`, `Problem.scope`, an index into a JSON value, a success body — is not seen, it
   can block an unrelated field that happens to share a name, and **a group that prints a server string raw passes the type
   checker**.
2. **The mutation proof is partial.** Mutant M-41 selects the client's display unit tests only; there is no mutant of the
   *binary* for the LF and TAB cases, so the binary's assertions exist but no mutation run proves they would fail.
3. **64 is a display policy, not a schema limit.** The `Health` schema has no `maxLength`: `cortex doctor` in markdown cuts every
   Health string (state, version, piece names, `last_good_dump_at`) to 64 characters (`BUDGET_IDENT`) **with the visible
   marker**, so a valid longer value is clipped in markdown — and the structured data is not changed.
4. **The number of pieces in `doctor` is unbounded.** Each value is cut; the total printed is not.
5. **Raw `doctor` JSON.** `cortex doctor --format json` prints the body as received; a guard cell keeps it that way.
6. **The list of unsafe characters is a list.** Other invisible characters (zero-width space, U+2060, U+FEFF, the variation
   selectors supplement) pass by design. **No terminal was tested**: what a given terminal does with a character outside the
   list, or inside it, is not claimed.
7. **Raw fields stay raw in the structured output.** The server's fields in json are kept as the server sent them, escaped for
   the line only — **public by design** (cox's decision of 6 Nov).

**A typed output boundary is a follow-up for `0.2.002`, not a claim of `0.2.001`.** The authority for all seven is
`crates/cortex-cli/tests/KNOWN-LIMITS.md` **at commit `372c258`** (owner sophi; the commit is cited rather than a
file hash because the file can be edited again and a hash here would then name bytes the section does not quote). This
section summarises it, and that committed text wins if the two ever disagree.

## 8. Checking this page

Every count above can be recomputed from the pin: 33 operations, 30 paths, 9 groups on this listener (10 tags, the
  tenth being the host-only `admin`), 20 scopes, 3 open routes. If your
recount differs, the pin has moved and this page is stale — see the note in
[conventions](conventions.md#10-if-you-are-checking-this-page) about which of the two to believe.
