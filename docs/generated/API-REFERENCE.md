# Cortex API reference (generated)

Generated from `cortex-v2-skeleton.openapi.r2.7.json` (sha256 `1d2c4b2f3f4d61cddffa3574d13a5b2efdae00a8cd6b5ea3087a895ec9d165d5`) by `tools/render_reference.py`. Do not edit: this page is a build product. Correct the contract and run `tools/build-docs.sh --write`.

Cortex v2 (v0.2.001 walking skeleton) 0.2.1-skeleton-draft.2.7: 33 operations on 30 paths.

## Route rules

### R-1 bytes

Every length limit is in UTF-8 BYTES. x-max-bytes on a string is the limit; maxLength carries the same number only as a looser bound on characters. The handler checks bytes BEFORE the database: over the limit is 422 invalid_request (or the field's own code), never a 500. (bob 3, nemo 3)

### R-2 text

A NUL character (U+0000) in any string is 422 invalid_request. Invalid UTF-8 or a lone surrogate escape is a JSON syntax error: 400 invalid_request. (bob 3; mira M3)

### R-3 decoding

Strict decoding (chain step 8, serde): bad JSON syntax is 400 invalid_request; a body that is valid JSON but not an OBJECT (an array, a string, a number, null) is 422 (all 15 request bodies are objects; W1-35 B); an unknown field, a missing schema-required field, a wrong type, a non-string value for an enum field (null, a number, true) or a value outside an enum is 422 invalid_request, never 400. BOUNDS (maxLength, x-max-bytes, minLength, maxItems, uniqueItems, pattern, minimum, maximum) are checked by the HANDLER: 422 with the property's x-code when it has one, else 422 invalid_request. A body that is not application/json is 415 invalid_request. A known path with the wrong method is 405 invalid_request with an Allow header. An unknown path is 404 not_found. (mira M3, M4, M7)

### R-4 query

Query parameters are strict too: an unknown query parameter, or a bad value (a limit outside its range, a bad cursor, a value outside an enum, a malformed id or date-time) is 422 invalid_request. A well-formed id in a FILTER that matches nothing gives an empty page, not 404. (sophi A6b, bob 6, tate M2) BUILT at 8a76e92 (socket delta 2, closed by vera r5): StrictQuery answers 422, and a DEFAULT-DENY marker means a route takes no query string unless it declares one. Until the serde_ignored decode lands (Wave 1, finn), every query struct MUST derive #[serde(deny_unknown_fields)] (vera r5).

### R-5 ids

An id in the PATH or the BODY that does not name a resource of the token's project (unknown, or another project's) is 404 not_found, with one body for both cases (S1:239, :358). A malformed id in the PATH is the same 404 (sophi A6a); a malformed id in the BODY fails decoding first (422 invalid_request). detail may name the field the client sent; it never says which case applied. Consequence: the r1 codes affects_unknown and lineage_target_unknown are retired (404 instead); unknown_addressee stays only for a job role that no active worker of the project holds. BUILT at 8a76e92 (socket delta 2, vera r5): path ids reach read AND write handlers through Ctx, and a malformed one is 404.

### R-6 handler-required

A field whose absence has its OWN minted code is not schema-required (else strict decoding would answer invalid_request first and the code could never fire). x-handler-required lists such fields; the handler refuses them absent, empty or whitespace-only with that code: reason_required, notes_required, evidence_required, risks_required, packet_incomplete. (sophi A5, nemo 1/6)

### R-7 bodies

A route that declares a request body needs a JSON object; the CLI and cortex-client always send one, {} when no field is set. (bob 5)

### R-8 lists

Every list answers {items, next}; limit default 50, maximum 200 (search has its own: 1-50, default 10); after = the next value of the previous page; keyset paging with no gaps or repeats; the order is the operation's x-order and is stable. A cursor is a POSITION, never authority: it cannot select rows outside the token's project (RLS). A cursor that does not decode, or was issued for another list, is 422 invalid_request. (sophi B2, bob 6, sam)

### R-9 platform answers

Every token route may also answer: 400/405/415/422 invalid_request (R-2, R-3, R-4); 401 (Unauthenticated); 403 missing_scope; 429 busy with Retry-After: 1, in two cases: (a) a database lock conflict or deadlock, at begin, in the handler or at commit: retry with the SAME Idempotency-Key; (b) a route's OWN concurrency cap is full (search: 2 at once): the handler refuses AT ONCE, before any database statement, and does not queue (v1 incident 912073e2: queueing under overload turned a slow query into an outage); search takes no Idempotency-Key, so a plain retry after Retry-After. cortex-http's INTERIM global cap queues and never refuses (until the cortex-http limiter release); it is separate from a route's own cap (nix); 500 internal_error; 503 unavailable (the database is down or timed out, or the token verifier cannot answer). 429 and 503 are declared on every token route; 400, 405 and 415 fall under default. The three health routes are OPEN: they ignore any Authorization header, valid, bad or absent, and answer as if none were sent (sam). (mira M1-M4, nix 1) BUILT at 8a76e92 (socket delta 2, vera r5): open routes sit outside the token check, inside the request-id and normaliser layers.

### R-10 headers

Every response: Cortex-Request-Id, a new UUIDv7 per response; a client-sent value is ignored (H-04). Every refusal: Cache-Control: no-store. Every 401: WWW-Authenticate: Bearer. Every 429: Retry-After: 1. A replayed success: Cortex-Idempotent-Replay: true. Every response to a token-authenticated request after authentication succeeded, SUCCESS OR REFUSAL (403 target, 404 malformed path id, 409, 422, 413, 428, 429 busy, a handler panic 500, a timeout 503; a read handler or a write handler alike): Cortex-Token-Expires (the PRESENTED token's expiry, never a newly issued token's) and, inside the renewal window, Cortex-Token-Renew: due. NEITHER is on a 401, nor on any refusal BEFORE authentication (a 429 or 503 before the token check), nor on the three open health routes. (mira M5, M6, W1-25; finn 7)

### R-11 idempotency

Only a committed success is recorded and replayed; no record for any refusal; a handler's own 4xx rolls back with the reservation (piece 11 s5). Authentication, scope AND the current target rule run BEFORE a replay (H-25, R-19), so a retry whose token is no longer valid, or whose caller no longer passes the target rule, is refused, never answered with the stored reply.

### R-12 services

No service principal exists in 0.2.001: worker_create accepts kind worker only (sophi ID-12) and nothing attests (kos-attest arrives with S5). The scope rows that name services take effect when services arrive. self:read and self:revoke are held by principals of kind worker only, so a service fixture is 403 missing_scope at step 7 (tate M6). More generally, a principal of kind service holds NO scope in 0.2.001, so every route's missing_scope negative can use a service fixture where no worker role lacks the scope (sam's deny actors).

### R-13 problem

Problem members are the built ones (cortex-http problem.rs at 0b13f10b): type, title, status, detail, fix, request_id, and scope on a scoped 503. No extension members in 0.2.001: numbers a code needs (boot_budget_too_small: needed and budget) are in detail. fix is fixed text per code.

### R-14 non-active targets

Revoking a token that is not active (revoked, superseded or expired) answers 200 {ok: true} with no state change and no second audit row (finn 4).

### R-15 job roles

A job_role addressee matches WorkerView.job_roles EXACTLY and case-sensitively, with no aliases and no folding (bob Q-9), on the job-role grammar of x-label-grammar (a lowercase letter, then up to 31 of a-z, 0-9 and hyphen: S1, S2 identity.workers). A job_role ref outside that grammar can match no one, so it is 422 invalid_request at step 0 (R-16), before any read.

### R-16 refusal order

When several refusals apply: the chain's own steps first (401, 403 missing_scope, 413, 400/422 decoding, the key-header errors 400 and 428). Then, in the handler, STEP 0: every check that needs NO database read answers 422 at once: bytes and bounds, patterns and the label grammar (R-18), NUL, uniqueness inside the body, conditional body rules (a source on a non-knowledge record), and the UNCONDITIONAL handler-required fields (sophi Q-10: no round trip, and nothing leaks, because a stateless 422 does not depend on any row). Inside step 0 the PER-FIELD checks (length, pattern, enum, range, NUL, a field own x-code) come BEFORE the CROSS-FIELD rules (oneOf and anyOf, the conditional source and orientation rules, the aggregate limit); the first refusal wins, in schema order within a group (cox, CHANGES section 17). Then the checks that need a read, in this order: 404 not_found (the path resource, then body references; R-5); 403 (the target rule FIRST, then self_review_forbidden, so a caller who is neither the creator nor the lead gets target_not_allowed even as the author: bob); THE IDEMPOTENCY LOOKUP (R-19: a stored success is replayed, a key used on another path or body is 422 idempotency_key_reused, otherwise proceed); 409 (invalid_transition before not_holder, then other conflicts); and last the 422s that need a read (unknown_addressee; release's reason_required when the caller is the lead and not the holder) (bob Q-12). So a retry of a COMMITTED operation by an authorized caller returns the stored success, never 409. TWO EXCEPTIONS: worker_retire answers 409 lead_permanent BEFORE 403, for any caller holding worker:retire, else that code could never fire (sophi Q-5); search checks its bounds (422) and THEN its own cap (429 busy, R-9 case b), both before its first database statement, so an invalid request takes no slot (nix MS-15b).

### R-17 targets

x-target on each operation names the mod-policy TargetRule variant its handler passes to ScopePolicy::target (sophi 3.2): None, SelfWorker, SelfOrProjectLead, NotProjectLead, RetireTarget, TokenTarget, Lineage, TaskAddressee. A rule that needs a Target field and finds None fails closed. BUILT at 3e69d030 (delta 3, closed by vera r6): S0 has the 8 variants; the truth table is mod-policy's (sophi).

### R-18 labels

Identifier-like strings (project, team and worker names; job roles; revoke codes; audit actions) take their grammar from ONE table, x-label-grammar, generated from S2's CHECKs and S1's grammar. S3 never accepts a label that S2 would refuse (mike 5fc9890a; kai's sweep, 6 Nov 02:5xZ). A label outside its grammar is 422 with the field's x-code (name_invalid, job_roles_invalid) or invalid_request, at step 0. PATTERN DIALECT (sophi F-11): patterns are ECMA-262, so ^ and $ anchor the WHOLE string and a trailing newline does NOT match; Postgres ~ and Rust regex agree. Python's re.search lets $ match before a final newline and would accept the name plus a newline: every Python checker uses re.fullmatch.

### R-19 replay

A replay NEVER bypasses authorization (kai 2ddfafe0, security). (1) Authentication, scope AND the CURRENT target rule are evaluated before any stored reply is returned; if the target check fails the caller gets exactly the refusal a first call would get (403 target_not_allowed, or 404 not_found for another project's id) and NO stored body. (2) The key is bound to the project, the principal, the METHOD, the CONCRETE path with every path id, and the canonical body hash. The same key with a different path id or body is 422 idempotency_key_reused, evaluated AFTER the target check of the CURRENT request, so a caller without access learns nothing about the key. (3) Reading the target row for the check, before the lookup and in the same transaction, is allowed. (4) The lookup sits where R-16 puts it. (5) Defence in depth: a mutating route's success reply carries only id, ok, state, round, created_at and section_revision (tokens.issue is never stored; a retry is 409 already_issued), so a replay never reveals more than the first answer did. BUILT CODE: MET at e1ff0b6 (Wave 1 base; socket deltas 4 and 4b, vera ACCEPT): a write route takes an authorize closure and an execute closure; an Authorized proof comes only from Ctx::authorize and is bound to the chain's original authority; the chain's one idempotency wrapper looks up AFTER authorize. STATED LIMITS: a 405 comes from the method router outside every guard, so its refusal log line names no principal and the path (unmatched); only the standard methods are named in that line, any other method token logs (other).

### R-20 generations

The generation CHECK is ENFORCED in 0.2.001 (kai 69b235ad: a restore must not un-revoke a token). A token is stamped with the CURRENT install and project generations when it is issued; at verify the check is EQUALITY, and a stored generation different from the current one answers 401 token_revoked (S0 AuthError::Revoked; the distinct code generation_stale is deferred). A MISSING generation row means generation 0, at issue and at verify; a NULL is NEVER "unknown, so pass". The restore tooling (cortex-restore) raises the install generation in EVERY restore, with no flag and no skip (nix 87937d77). A PLAIN pg_restore does not raise it: that is DV-4, accepted as warning-only for 0.2.001, with the restore fence (0.2.011) as its closer.

## Label grammars

| Grammar | Pattern | At most (characters) | Source | Applies to |
|---|---|---|---|---|
| `name` | `^[A-Za-z][A-Za-z0-9-]{0,31}$` | 32 | S2 identity.names: name_key ^[a-z][a-z0-9-]{0,31}$ and lower(label) = name_key (rev4); names_label_ascii ^[A-Za-z][A-Za-z0-9-]{0,31}$ (rev5 F-10); S1 3.3. Unique per project and kind by name_key (case-insensitive) | `identity.workers_list` parameter `name`, `Project.name`, `Team.name`, `TeamCreate.name`, `Worker.name`, `WorkerCreate.name`, `Whoami.name` |
| `job_role` | `^[a-z][a-z0-9-]{0,31}$` | 32 | S2 identity.workers job_roles CHECK ^[a-z][a-z0-9-]{0,31}(,...)*$, at most 8 (rev4 and rev5); S1 label grammar | `Worker.job_roles[]`, `WorkerCreate.job_roles[]`, `TeamAssign.job_roles[]`, `Whoami.job_roles[]` |
| `revoke_code` | `^[a-z][a-z_]{0,31}$` | 32 | S2 tokens.revoke_reason CHECK (tokens.sql:24) | `TokenRevoke.reason` |
| `audit_action` | `^[a-z][a-z0-9_.:-]{0,63}$` | 64 | S2 audit.records.action CHECK (audit.sql:19) | `AuditRecord.action` |

## Notes

### datetime-rule

W1-39 (hale, piece 8 and piece 6): EVERY date-time input of a body, a query or a cursor is RFC 3339 YYYY-MM-DDTHH:MM:SS[.f{1,9}](Z|+HH:MM|-HH:MM) with upper-case T and Z, a real calendar date, year 1970 to 9999, hour 0-23, minute 0-59, second 0-59 (no leap second), offset minutes 0-59 and the offset within -12:00 .. +14:00; NUL or any other character is rejected. A reject is 422 invalid_request, never a database error, never 500. Shared vectors: mvp/api/rfc3339-vectors.json (14 accept, 48 reject). Not normalised to UTC.

### max-body

x-max-body is declared per operation (bytes) and is the chain's step-6 limit; an operation that declares none would get the default of 1 MiB, which counts as DRIFT for the Routes table (mira, item F). In this contract: 15 operations at 16384, 1 operations at 262144, 1 operations at 2097152, 1 operations at 9437184; operations with none: 15 (the open health routes and the plain reads). POST /v2/search is a READ with a body (x-mutates false, no Idempotency-Key) and lists 413 and 422; GET /v2/sections/{name} lists 422 for a section name that is not reserved; the open GET /v2/health/ready lists 503 capability_unavailable (mira F).

## Operations

### `server.health`: GET `/v2/health`

**Task:** Server state: version, readiness, pieces, last good dump, backup state.

> Server state: version, readiness, pieces, last good dump, backup state. Open by design (see Health)

**Who may call:** scope `none` (class open); auth: no token (security: []); target rule `None`.

**Request:**

No parameters.

**Response:**

- `200`: `application/json` `Health`; headers `Cortex-Request-Id`

**Failures:**

- This route's own codes: none
- Responses: `default` Problem

**Idempotency:** not a write: safe to retry.

**CLI:** `cortex doctor`

### `server.live`: GET `/v2/health/live`

**Task:** Liveness

> Liveness: 200 while the process runs

**Who may call:** scope `none` (class open); auth: no token (security: []); target rule `None`.

**Request:**

No parameters.

**Response:**

- `200`: `application/json` `Ok`; headers `Cortex-Request-Id`

**Failures:**

- This route's own codes: none
- Responses: `default` Problem

**Idempotency:** not a write: safe to retry.

**CLI:** none

### `server.ready`: GET `/v2/health/ready`

**Task:** Readiness

> Readiness: 200 only when the database is reachable, migrations are current and every enabled piece is ready, else 503 capability_unavailable

**Who may call:** scope `none` (class open); auth: no token (security: []); target rule `None`.

**Request:**

No parameters.

**Response:**

- `200`: `application/json` `Ok`; headers `Cortex-Request-Id`

**Failures:**

- This route's own codes: `503 capability_unavailable`
- Responses: `default` Problem, `503`

**Idempotency:** not a write: safe to retry.

**CLI:** none

### `identity.project_get`: GET `/v2/projects/{id}`

**Task:** One project

> One project: the token's own; any other id is 404

**Who may call:** scope `roster:read` (class work): project_lead, team_lead, member; auth: a worker token; target rule `None`.

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `id` | path | yes | string, uuid | malformed, unknown or another project's: 404 not_found (R-5) |

**Response:**

- `200`: `application/json` `Project`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`

**Failures:**

- This route's own codes: `404 not_found`
- Responses: `401` Unauthenticated, `403` Forbidden, `404` NotFound, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** not a write: safe to retry.

**CLI:** `cortex projects`

### `identity.teams_list`: GET `/v2/teams`

**Task:** Teams of the token's project

**Who may call:** scope `roster:read` (class work): project_lead, team_lead, member; auth: a worker token; target rule `None`.

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `limit` | query | no | integer, 1 to 200, default `50` |  |
| `after` | query | no | string | the next value of the previous page (R-8) |

**Response:**

- `200`: `application/json` `TeamPage`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`
- Order: name, then id

**Failures:**

- This route's own codes: none
- Responses: `401` Unauthenticated, `403` Forbidden, `422` Unprocessable, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** not a write: safe to retry.

**CLI:** `cortex roster`

### `identity.team_create`: POST `/v2/teams`

**Task:** Create a team (project lead)

**Who may call:** scope `team:create` (class admin): project_lead; auth: a worker token; target rule `None`.

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `Idempotency-Key` | header | yes | string, pattern `^[\x20-\x7E]+$` | Required on every mutating route (x-mutates: true). 8 to 128 printable ASCII characters, 0x20 to 0x7E (S2:549; cortex-db idempotency.rs:56). The CLI and cortex-client generate a UUIDv7 per logical operation and reuse it on retry. Bound to (project, principal, METHOD + CONCRETE PATH with every path id, body hash: RFC 8785 canonical JSON). Same key, path and body: the stored status and body are replayed (after the target check, R-19) with Cortex-Idempotent-Replay: true and nothing happens twice. Same key with a different path id or body: 422 idempotency_key_reused. Missing: 428 idempotency_key_required. Two Idempotency-Key headers, or a value outside 8-128 or not printable ASCII: 400 invalid_request (chain step 9; mira M3). Replays are kept 24 hours. Authentication runs before a replay (R-11). |

Body (at most 16384 bytes): `application/json` `TeamCreate`

**Response:**

- `201`: `application/json` `Id`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`, `Cortex-Idempotent-Replay`

**Failures:**

- This route's own codes: `422 name_invalid`, `409 name_taken`
- Responses: `401` Unauthenticated, `403` Forbidden, `422` Unprocessable, `428` IdempotencyKeyRequired, `413` BodyTooLarge, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** a write: send `Idempotency-Key`; the same key with the same body replays the first answer for 24 hours, and the same key with another body is refused (R-11, R-19).

**CLI:** `cortex team add <name>`

### `identity.workers_list`: GET `/v2/workers`

**Task:** THE ROSTER: active workers of the token's project.

> THE ROSTER: active workers of the token's project. Retired workers are not listed in 0.2.001 (Q-2; include_retired cut, sophi A7)

**Who may call:** scope `roster:read` (class work): project_lead, team_lead, member; auth: a worker token; target rule `None`.

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `name` | query | no | string, pattern `^[A-Za-z][A-Za-z0-9-]{0,31}$`, at most 32 bytes | compared by name_key; a value outside the name grammar (33 characters, a space, a leading digit, a trailing newline) is 422 invalid_request, not an empty page (sophi Q-11, R-4, R-18) |
| `limit` | query | no | integer, 1 to 200, default `50` |  |
| `after` | query | no | string | the next value of the previous page (R-8) |

**Response:**

- `200`: `application/json` `WorkerPage`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`
- Order: name, then id

**Failures:**

- This route's own codes: none
- Responses: `401` Unauthenticated, `403` Forbidden, `422` Unprocessable, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** not a write: safe to retry.

**CLI:** `cortex roster`

### `identity.worker_create`: POST `/v2/workers`

**Task:** Add a worker (project lead).

> Add a worker (project lead). kind worker only in 0.2.001; a team that is not this project's is 404

**Who may call:** scope `worker:create` (class admin): project_lead; auth: a worker token; target rule `None`.

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `Idempotency-Key` | header | yes | string, pattern `^[\x20-\x7E]+$` | Required on every mutating route (x-mutates: true). 8 to 128 printable ASCII characters, 0x20 to 0x7E (S2:549; cortex-db idempotency.rs:56). The CLI and cortex-client generate a UUIDv7 per logical operation and reuse it on retry. Bound to (project, principal, METHOD + CONCRETE PATH with every path id, body hash: RFC 8785 canonical JSON). Same key, path and body: the stored status and body are replayed (after the target check, R-19) with Cortex-Idempotent-Replay: true and nothing happens twice. Same key with a different path id or body: 422 idempotency_key_reused. Missing: 428 idempotency_key_required. Two Idempotency-Key headers, or a value outside 8-128 or not printable ASCII: 400 invalid_request (chain step 9; mira M3). Replays are kept 24 hours. Authentication runs before a replay (R-11). |

Body (at most 16384 bytes): `application/json` `WorkerCreate`

**Response:**

- `201`: `application/json` `Id`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`, `Cortex-Idempotent-Replay`

**Failures:**

- This route's own codes: `422 name_invalid`, `409 name_taken`, `422 job_roles_invalid`, `404 not_found`
- Responses: `401` Unauthenticated, `403` Forbidden, `404` NotFound, `422` Unprocessable, `428` IdempotencyKeyRequired, `413` BodyTooLarge, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** a write: send `Idempotency-Key`; the same key with the same body replays the first answer for 24 hours, and the same key with another body is refused (R-11, R-19).

**CLI:** `cortex worker add <name> --role R --mode M`

### `identity.worker_retire`: POST `/v2/workers/{id}/retire`

**Task:** Retire a worker (one-way): its tokens then answer 401 worker_retired.

> Retire a worker (one-way): its tokens then answer 401 worker_retired. The reason is stored in identity.workers.retired_reason (sophi F-9; nemo, S2 revision 5). Project lead (not self), or a team lead for its own team (not self); S1 5.3

**Who may call:** scope `worker:retire` (class admin): project_lead (not self); team_lead for its own team (not self); S1 5.3 (sophi A1); auth: a worker token; target rule `RetireTarget`.

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `id` | path | yes | string, uuid | malformed, unknown or another project's: 404 not_found (R-5) |
| `Idempotency-Key` | header | yes | string, pattern `^[\x20-\x7E]+$` | Required on every mutating route (x-mutates: true). 8 to 128 printable ASCII characters, 0x20 to 0x7E (S2:549; cortex-db idempotency.rs:56). The CLI and cortex-client generate a UUIDv7 per logical operation and reuse it on retry. Bound to (project, principal, METHOD + CONCRETE PATH with every path id, body hash: RFC 8785 canonical JSON). Same key, path and body: the stored status and body are replayed (after the target check, R-19) with Cortex-Idempotent-Replay: true and nothing happens twice. Same key with a different path id or body: 422 idempotency_key_reused. Missing: 428 idempotency_key_required. Two Idempotency-Key headers, or a value outside 8-128 or not printable ASCII: 400 invalid_request (chain step 9; mira M3). Replays are kept 24 hours. Authentication runs before a replay (R-11). |

Body (at most 16384 bytes): `application/json` `Reason`

**Response:**

- `200`: `application/json` `Ok`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`, `Cortex-Idempotent-Replay`

**Failures:**

- This route's own codes: `404 not_found`, `409 lead_permanent`, `409 target_retired`, `403 target_not_allowed`, `422 reason_required`
- Responses: `401` Unauthenticated, `403` Forbidden, `404` NotFound, `422` Unprocessable, `428` IdempotencyKeyRequired, `413` BodyTooLarge, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** a write: send `Idempotency-Key`; the same key with the same body replays the first answer for 24 hours, and the same key with another body is refused (R-11, R-19).

**CLI:** `cortex worker retire <id> --reason <text>`

### `identity.team_assign`: PUT `/v2/workers/{id}/team`

**Task:** Move, promote or demote a worker within teams (one team lead per team).

> Move, promote or demote a worker within teams (one team lead per team). Repeating the same (team, role) with job_roles absent or equal to the stored array, element for element, is 200 with no change and no audit row; the same (team, role) with a DIFFERENT job_roles list replaces the array (perm_gen does not rise) and leaves ONE audit record worker.team_assigned with job_roles_changed true (CHANGES section 24)

**Who may call:** scope `team:assign` (class admin): project_lead; auth: a worker token; target rule `NotProjectLead`.

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `id` | path | yes | string, uuid | malformed, unknown or another project's: 404 not_found (R-5) |
| `Idempotency-Key` | header | yes | string, pattern `^[\x20-\x7E]+$` | Required on every mutating route (x-mutates: true). 8 to 128 printable ASCII characters, 0x20 to 0x7E (S2:549; cortex-db idempotency.rs:56). The CLI and cortex-client generate a UUIDv7 per logical operation and reuse it on retry. Bound to (project, principal, METHOD + CONCRETE PATH with every path id, body hash: RFC 8785 canonical JSON). Same key, path and body: the stored status and body are replayed (after the target check, R-19) with Cortex-Idempotent-Replay: true and nothing happens twice. Same key with a different path id or body: 422 idempotency_key_reused. Missing: 428 idempotency_key_required. Two Idempotency-Key headers, or a value outside 8-128 or not printable ASCII: 400 invalid_request (chain step 9; mira M3). Replays are kept 24 hours. Authentication runs before a replay (R-11). |

Body (at most 16384 bytes): `application/json` `TeamAssign`

**Response:**

- `200`: `application/json` `Ok`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`, `Cortex-Idempotent-Replay`

**Failures:**

- This route's own codes: `404 not_found`, `409 target_retired`, `409 team_lead_taken`, `403 target_not_allowed`, `422 job_roles_invalid`
- Responses: `401` Unauthenticated, `403` Forbidden, `404` NotFound, `422` Unprocessable, `428` IdempotencyKeyRequired, `413` BodyTooLarge, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** a write: send `Idempotency-Key`; the same key with the same body replays the first answer for 24 hours, and the same key with another body is refused (R-11, R-19).

**CLI:** `cortex worker move|promote|demote`

### `tokens.whoami`: GET `/v2/whoami`

**Task:** Who the token is

> Who the token is: installation, project, worker, kind, role, team, job_roles, token id and expiry

**Who may call:** scope `self:read` (class self): every principal of kind worker (R-12); auth: a worker token; target rule `None`.

**Request:**

No parameters.

**Response:**

- `200`: `application/json` `Whoami`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`

**Failures:**

- This route's own codes: none
- Responses: `401` Unauthenticated, `403` Forbidden, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** not a write: safe to retry.

**CLI:** `cortex whoami`

### `tokens.revoke_self`: POST `/v2/self/token/revoke`

**Task:** Revoke the presented token.

> Revoke the presented token. A retry with the same key after a success is 401 token_revoked (authentication runs before a replay, R-11); the revoke happened once (finn 5)

**Who may call:** scope `self:revoke` (class self): every principal of kind worker (R-12); auth: a worker token; target rule `None`.

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `Idempotency-Key` | header | yes | string, pattern `^[\x20-\x7E]+$` | Required on every mutating route (x-mutates: true). 8 to 128 printable ASCII characters, 0x20 to 0x7E (S2:549; cortex-db idempotency.rs:56). The CLI and cortex-client generate a UUIDv7 per logical operation and reuse it on retry. Bound to (project, principal, METHOD + CONCRETE PATH with every path id, body hash: RFC 8785 canonical JSON). Same key, path and body: the stored status and body are replayed (after the target check, R-19) with Cortex-Idempotent-Replay: true and nothing happens twice. Same key with a different path id or body: 422 idempotency_key_reused. Missing: 428 idempotency_key_required. Two Idempotency-Key headers, or a value outside 8-128 or not printable ASCII: 400 invalid_request (chain step 9; mira M3). Replays are kept 24 hours. Authentication runs before a replay (R-11). |

**Response:**

- `200`: `application/json` `Ok`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`, `Cortex-Idempotent-Replay`

**Failures:**

- This route's own codes: none
- Responses: `401` Unauthenticated, `403` Forbidden, `422` Unprocessable, `428` IdempotencyKeyRequired, `413` BodyTooLarge, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** a write: send `Idempotency-Key`; the same key with the same body replays the first answer for 24 hours, and the same key with another body is refused (R-11, R-19).

**CLI:** `cortex token revoke-self`

### `tokens.issue`: POST `/v2/workers/{id}/tokens`

**Task:** Issue (or reissue) a worker's token.

> Issue (or reissue) a worker's token. Issuing SUPERSEDES the worker's other active tokens (card, S1 4.3): in 0.2.001 they then answer 401 token_revoked and read state superseded. The SECRET is in this response ONCE and is never stored or replayed: a retry with the same key is 409 already_issued naming the token id only. Cortex-Token-Expires is the CALLER's token; the body's expires_at is the new token's (finn 2, 7)

**Who may call:** scope `token:issue` (class admin): project_lead; team_lead for its own team (not services); auth: a worker token; target rule `TokenTarget`.

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `id` | path | yes | string, uuid | malformed, unknown or another project's: 404 not_found (R-5) |
| `Idempotency-Key` | header | yes | string, pattern `^[\x20-\x7E]+$` | Required on every mutating route (x-mutates: true). 8 to 128 printable ASCII characters, 0x20 to 0x7E (S2:549; cortex-db idempotency.rs:56). The CLI and cortex-client generate a UUIDv7 per logical operation and reuse it on retry. Bound to (project, principal, METHOD + CONCRETE PATH with every path id, body hash: RFC 8785 canonical JSON). Same key, path and body: the stored status and body are replayed (after the target check, R-19) with Cortex-Idempotent-Replay: true and nothing happens twice. Same key with a different path id or body: 422 idempotency_key_reused. Missing: 428 idempotency_key_required. Two Idempotency-Key headers, or a value outside 8-128 or not printable ASCII: 400 invalid_request (chain step 9; mira M3). Replays are kept 24 hours. Authentication runs before a replay (R-11). |

**Response:**

- `201`: `application/json` `IssuedToken`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`, `Cortex-Idempotent-Replay`

**Failures:**

- This route's own codes: `404 not_found`, `409 already_issued`, `409 target_retired`, `403 target_not_allowed`
- Responses: `401` Unauthenticated, `403` Forbidden, `404` NotFound, `422` Unprocessable, `428` IdempotencyKeyRequired, `413` BodyTooLarge, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** a write: send `Idempotency-Key`; the same key with the same body replays the first answer for 24 hours, and the same key with another body is refused (R-11, R-19).

**CLI:** `cortex token issue|reissue --worker <id>`

### `tokens.list`: GET `/v2/tokens`

**Task:** Token metadata (never a digest or a secret)

**Who may call:** scope `token:list` (class admin): project_lead; team_lead own team; auth: a worker token; target rule `None`.
Target note: a list, not one target: rule None, and the SQL filters a team lead's rows to its own team (sophi Q-8; finn)

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `limit` | query | no | integer, 1 to 200, default `50` |  |
| `after` | query | no | string | the next value of the previous page (R-8) |

**Response:**

- `200`: `application/json` `TokenPage`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`
- Order: issued_at, then token_id

**Failures:**

- This route's own codes: none
- Responses: `401` Unauthenticated, `403` Forbidden, `422` Unprocessable, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** not a write: safe to retry.

**CLI:** `cortex token list`

### `tokens.inspect`: GET `/v2/tokens/{id}`

**Task:** One token's metadata

**Who may call:** scope `token:list` (class admin): project_lead; team_lead own team; auth: a worker token; target rule `TokenTarget`.

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `id` | path | yes | string, uuid | malformed, unknown or another project's: 404 not_found (R-5) |

**Response:**

- `200`: `application/json` `TokenMeta`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`

**Failures:**

- This route's own codes: `404 not_found`, `403 target_not_allowed`
- Responses: `401` Unauthenticated, `403` Forbidden, `404` NotFound, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** not a write: safe to retry.

**CLI:** `cortex token inspect <id>`

### `tokens.revoke`: POST `/v2/tokens/{id}/revoke`

**Task:** Revoke a token, with a reason CODE.

> Revoke a token, with a reason CODE. A token that is not active answers 200 with no change and no second audit row (R-14, finn 4)

**Who may call:** scope `token:revoke` (class admin): project_lead; team_lead own team; auth: a worker token; target rule `TokenTarget`.

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `id` | path | yes | string, uuid | malformed, unknown or another project's: 404 not_found (R-5) |
| `Idempotency-Key` | header | yes | string, pattern `^[\x20-\x7E]+$` | Required on every mutating route (x-mutates: true). 8 to 128 printable ASCII characters, 0x20 to 0x7E (S2:549; cortex-db idempotency.rs:56). The CLI and cortex-client generate a UUIDv7 per logical operation and reuse it on retry. Bound to (project, principal, METHOD + CONCRETE PATH with every path id, body hash: RFC 8785 canonical JSON). Same key, path and body: the stored status and body are replayed (after the target check, R-19) with Cortex-Idempotent-Replay: true and nothing happens twice. Same key with a different path id or body: 422 idempotency_key_reused. Missing: 428 idempotency_key_required. Two Idempotency-Key headers, or a value outside 8-128 or not printable ASCII: 400 invalid_request (chain step 9; mira M3). Replays are kept 24 hours. Authentication runs before a replay (R-11). |

Body (at most 16384 bytes): `application/json` `TokenRevoke`

**Response:**

- `200`: `application/json` `Ok`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`, `Cortex-Idempotent-Replay`

**Failures:**

- This route's own codes: `404 not_found`, `403 target_not_allowed`, `422 reason_required`
- Responses: `401` Unauthenticated, `403` Forbidden, `404` NotFound, `422` Unprocessable, `428` IdempotencyKeyRequired, `413` BodyTooLarge, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** a write: send `Idempotency-Key`; the same key with the same body replays the first answer for 24 hours, and the same key with another body is refused (R-11, R-19).

**CLI:** `cortex token revoke <id> --reason <code>`

### `audit.list`: GET `/v2/audit`

**Task:** The token's project's audit records, project lead only (audit:read).

> The token's project's audit records, project lead only (audit:read). PLAIN LOG in 0.2.001: no chain id, no seq, no hash, so NO completeness or tamper evidence; that arrives with the mod-audit hash chain release, and with it cortex audit verify (marv)

**Who may call:** scope `audit:read` (class admin): project_lead only, own project; auth: a worker token; target rule `None`.

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `limit` | query | no | integer, 1 to 200, default `50` |  |
| `after` | query | no | string | the next value of the previous page (R-8) |

**Response:**

- `200`: `application/json` `AuditPage`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`
- Order: (at, id) ascending

**Failures:**

- This route's own codes: none
- Responses: `401` Unauthenticated, `403` Forbidden, `422` Unprocessable, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** not a write: safe to retry.

**CLI:** `cortex audit list`

### `records.log`: POST `/v2/records`

**Task:** Write a record (S2 3.1 types; see RecordCreate).

> Write a record (S2 3.1 types; see RecordCreate). Writing a reserved section is a target rule: rules-project, the project lead; rules-worker:<id>, that worker or the project lead; orientation, the project lead (audited as section.write_reserved)

**Who may call:** scope `records:write` (class work): project_lead, team_lead, member; auth: a worker token; target rule `None`.
Target note: reserved sections are the handler's own check (403 section_reserved): the rules sections for their owners, orientation for the project lead

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `Idempotency-Key` | header | yes | string, pattern `^[\x20-\x7E]+$` | Required on every mutating route (x-mutates: true). 8 to 128 printable ASCII characters, 0x20 to 0x7E (S2:549; cortex-db idempotency.rs:56). The CLI and cortex-client generate a UUIDv7 per logical operation and reuse it on retry. Bound to (project, principal, METHOD + CONCRETE PATH with every path id, body hash: RFC 8785 canonical JSON). Same key, path and body: the stored status and body are replayed (after the target check, R-19) with Cortex-Idempotent-Replay: true and nothing happens twice. Same key with a different path id or body: 422 idempotency_key_reused. Missing: 428 idempotency_key_required. Two Idempotency-Key headers, or a value outside 8-128 or not printable ASCII: 400 invalid_request (chain step 9; mira M3). Replays are kept 24 hours. Authentication runs before a replay (R-11). |

Body (at most 9437184 bytes): `application/json` `RecordCreate`

**Response:**

- `201`: `application/json` `RecordRef`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`, `Cortex-Idempotent-Replay`

**Failures:**

- This route's own codes: `422 type_reserved`, `422 source_required`, `422 source_not_allowed`, `422 locator_too_long`, `422 invalid_request`, `422 why_required`, `422 files_required`, `422 section_invalid`, `403 section_reserved`, `404 not_found`
- Responses: `401` Unauthenticated, `403` Forbidden, `404` NotFound, `422` Unprocessable, `428` IdempotencyKeyRequired, `413` BodyTooLarge, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** a write: send `Idempotency-Key`; the same key with the same body replays the first answer for 24 hours, and the same key with another body is refused (R-11, R-19).

**CLI:** `cortex log <type> ... | cortex rules set`

### `records.show`: GET `/v2/records/{id}`

**Task:** One record with its state and lineage (always returned, even when superseded or invalidated)

**Who may call:** scope `records:read` (class work): project_lead, team_lead, member; auth: a worker token; target rule `None`.

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `id` | path | yes | string, uuid | malformed, unknown or another project's: 404 not_found (R-5) |

**Response:**

- `200`: `application/json` `Record`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`

**Failures:**

- This route's own codes: `404 not_found`
- Responses: `401` Unauthenticated, `403` Forbidden, `404` NotFound, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** not a write: safe to retry.

**CLI:** `cortex record show <id>`

### `records.supersede`: POST `/v2/records/{id}/supersede`

**Task:** Supersede a record by another (target rule: the author, the author's team lead or the project lead; a non-worker author: the project lead only).

> Supersede a record by another (target rule: the author, the author's team lead or the project lead; a non-worker author: the project lead only). The path record or by naming nothing in this project: 404. The replacement `by` must be ACTIVE (not superseded, not invalidated): otherwise 409 lineage_state_conflict, checked in the same atomic statement as the state change, which also makes a cycle impossible; a record that is a SECTION REVISION cannot be superseded at all (409 lineage_state_conflict: a section changes by writing a new revision and is withdrawn by invalidation). Audited. Of concurrent supersedes of one record exactly one wins; the others are 409 lineage_state_conflict (CHANGES sections 20 and 26)

**Who may call:** scope `records:lineage` (class work): project_lead, team_lead, member (target rule: author, author's team lead, project lead); auth: a worker token; target rule `Lineage`.

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `id` | path | yes | string, uuid | malformed, unknown or another project's: 404 not_found (R-5) |
| `Idempotency-Key` | header | yes | string, pattern `^[\x20-\x7E]+$` | Required on every mutating route (x-mutates: true). 8 to 128 printable ASCII characters, 0x20 to 0x7E (S2:549; cortex-db idempotency.rs:56). The CLI and cortex-client generate a UUIDv7 per logical operation and reuse it on retry. Bound to (project, principal, METHOD + CONCRETE PATH with every path id, body hash: RFC 8785 canonical JSON). Same key, path and body: the stored status and body are replayed (after the target check, R-19) with Cortex-Idempotent-Replay: true and nothing happens twice. Same key with a different path id or body: 422 idempotency_key_reused. Missing: 428 idempotency_key_required. Two Idempotency-Key headers, or a value outside 8-128 or not printable ASCII: 400 invalid_request (chain step 9; mira M3). Replays are kept 24 hours. Authentication runs before a replay (R-11). |

Body (at most 16384 bytes): `application/json` `Supersede`

**Response:**

- `200`: `application/json` `LineageState`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`, `Cortex-Idempotent-Replay`

**Failures:**

- This route's own codes: `404 not_found`, `422 self_supersede`, `409 lineage_state_conflict`, `403 target_not_allowed`, `422 reason_required`
- Responses: `401` Unauthenticated, `403` Forbidden, `404` NotFound, `422` Unprocessable, `428` IdempotencyKeyRequired, `413` BodyTooLarge, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** a write: send `Idempotency-Key`; the same key with the same body replays the first answer for 24 hours, and the same key with another body is refused (R-11, R-19).

**CLI:** `cortex record supersede <id> --by <id> --reason <text>`

### `records.invalidate`: POST `/v2/records/{id}/invalidate`

**Task:** Invalidate a record, or undo an invalidation.

> Invalidate a record, or undo an invalidation. A reason is required for both. Undo is allowed only for an invalidated record; for a section revision it is 409 lineage_state_conflict if a higher revision exists in any state (CHANGES section 20). Invalidating a record that is already SUPERSEDED is 409 lineage_state_conflict (it would change no visibility; the body of a record in any state is readable by id). Audited

**Who may call:** scope `records:lineage` (class work): project_lead, team_lead, member (target rule: author, author's team lead, project lead); auth: a worker token; target rule `Lineage`.

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `id` | path | yes | string, uuid | malformed, unknown or another project's: 404 not_found (R-5) |
| `Idempotency-Key` | header | yes | string, pattern `^[\x20-\x7E]+$` | Required on every mutating route (x-mutates: true). 8 to 128 printable ASCII characters, 0x20 to 0x7E (S2:549; cortex-db idempotency.rs:56). The CLI and cortex-client generate a UUIDv7 per logical operation and reuse it on retry. Bound to (project, principal, METHOD + CONCRETE PATH with every path id, body hash: RFC 8785 canonical JSON). Same key, path and body: the stored status and body are replayed (after the target check, R-19) with Cortex-Idempotent-Replay: true and nothing happens twice. Same key with a different path id or body: 422 idempotency_key_reused. Missing: 428 idempotency_key_required. Two Idempotency-Key headers, or a value outside 8-128 or not printable ASCII: 400 invalid_request (chain step 9; mira M3). Replays are kept 24 hours. Authentication runs before a replay (R-11). |

Body (at most 16384 bytes): `application/json` `Invalidate`

**Response:**

- `200`: `application/json` `LineageState`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`, `Cortex-Idempotent-Replay`

**Failures:**

- This route's own codes: `404 not_found`, `409 lineage_state_conflict`, `403 target_not_allowed`, `422 reason_required`
- Responses: `401` Unauthenticated, `403` Forbidden, `404` NotFound, `422` Unprocessable, `428` IdempotencyKeyRequired, `413` BodyTooLarge, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** a write: send `Idempotency-Key`; the same key with the same body replays the first answer for 24 hours, and the same key with another body is refused (R-11, R-19).

**CLI:** `cortex record invalidate <id> --reason <text> [--undo]   (alias: cortex invalidate)`

### `records.section_current`: GET `/v2/sections/{name}`

**Task:** The CURRENT revision of a reserved section (rules-project, rules-worker:<id>, orientation)

> The CURRENT revision of a reserved section (rules-project, rules-worker:<id>, orientation): a pointer; 404 when none, and 404 after the current revision is invalidated until a new revision is written (no fallback to the previous one: CHANGES section 20)

**Who may call:** scope `records:read` (class work): project_lead, team_lead, member; auth: a worker token; target rule `None`.

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `name` | path | yes | string |  |

**Response:**

- `200`: `application/json` `SectionPointer`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`

**Failures:**

- This route's own codes: `404 not_found`, `422 section_invalid`
- Responses: `401` Unauthenticated, `403` Forbidden, `404` NotFound, `422` Unprocessable, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** not a write: safe to retry.

**CLI:** `cortex rules`

### `search.query`: POST `/v2/search`

**Task:** Keyword search over the token's project (Postgres full text).

> Keyword search over the token's project (Postgres full text). Read-only POST: no Idempotency-Key. Superseded and invalidated records are left out unless include_history. 429 busy when search's own cap (2 at once) is full: refused at once, before any database statement (R-9 case b); 503 unavailable on a statement timeout or an unreadable records dependency (nix 1)

**Who may call:** scope `records:read` (class work): project_lead, team_lead, member; auth: a worker token; target rule `None`.

**Request:**

No parameters.

Body (at most 16384 bytes): `application/json` `SearchRequest`

**Response:**

- `200`: `application/json` `SearchResult`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`
- Order: rank descending, then created_at descending, then id

**Failures:**

- This route's own codes: `422 query_too_long`, `422 invalid_request`, `429 busy`, `503 unavailable`
- Responses: `401` Unauthenticated, `403` Forbidden, `422` Unprocessable, `413` BodyTooLarge, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** not a write: safe to retry.

**CLI:** `cortex search <q> [--type T]... [--limit N] [--since T] [--until T] [--history]   (alias: cortex-search; the v1-only flags --room --hall --graph --no-rerank are accepted, NOT sent, and noted in one stderr line: nix 2)`

### `handoffs.create`: POST `/v2/handoffs`

**Task:** Create a handoff.

> Create a handoff. ONE scope; the target rule decides by kind: a task needs the project lead (any addressee) or a team lead (a WORKER addressee in its own team; the team comes from Role in socket r5); a consult is open to any worker. The packet includes acceptance AND verify

**Who may call:** scope `handoff:create` (class work): every worker; target rule by kind: task = project_lead (any addressee) or team_lead (a worker addressee in its own team); consult = any worker; auth: a worker token; target rule `TaskAddressee`.
Target note: declared TaskAddressee; a consult is open to any worker: the handler passes Target{consult: true} for kind consult (decoded from the TOP-LEVEL kind field, never Addressee.kind) and mod-policy answers true for a consult for any principal that passed the scope check; for kind task the real addressee Target is checked by table 3.2 (cox W1-9)

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `Idempotency-Key` | header | yes | string, pattern `^[\x20-\x7E]+$` | Required on every mutating route (x-mutates: true). 8 to 128 printable ASCII characters, 0x20 to 0x7E (S2:549; cortex-db idempotency.rs:56). The CLI and cortex-client generate a UUIDv7 per logical operation and reuse it on retry. Bound to (project, principal, METHOD + CONCRETE PATH with every path id, body hash: RFC 8785 canonical JSON). Same key, path and body: the stored status and body are replayed (after the target check, R-19) with Cortex-Idempotent-Replay: true and nothing happens twice. Same key with a different path id or body: 422 idempotency_key_reused. Missing: 428 idempotency_key_required. Two Idempotency-Key headers, or a value outside 8-128 or not printable ASCII: 400 invalid_request (chain step 9; mira M3). Replays are kept 24 hours. Authentication runs before a replay (R-11). |

Body (at most 262144 bytes): `application/json` `HandoffCreate`

**Response:**

- `201`: `application/json` `HandoffRef`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`, `Cortex-Idempotent-Replay`

**Failures:**

- This route's own codes: `422 packet_incomplete`, `422 unknown_addressee`, `404 not_found`, `409 target_retired`, `403 target_not_allowed`
- Responses: `401` Unauthenticated, `403` Forbidden, `404` NotFound, `422` Unprocessable, `428` IdempotencyKeyRequired, `413` BodyTooLarge, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** a write: send `Idempotency-Key`; the same key with the same body replays the first answer for 24 hours, and the same key with another body is refused (R-11, R-19).

**CLI:** `cortex handoff create ...`

### `handoffs.list`: GET `/v2/handoffs`

**Task:** Handoffs of the token's project, filtered.

> Handoffs of the token's project, filtered. mine=true gives: the ones I hold (claimed); the pending ones addressed to me or to a job role I hold; and the RETURNED ones I may review: all returned for the project lead, the ones it created for a creator, never one it returned itself unless it is the only possible reviewer (self-review); AND the ones I CREATED that SOMEONE ELSE cancelled, for 7 days after closed_at (state cancelled, with closed_by and close_reason; one I cancelled myself is not listed: kai 32f893de). state absent = every state (bob 6)

**Who may call:** scope `handoff:read` (class work): project_lead, team_lead, member; auth: a worker token; target rule `None`.

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `mine` | query | no | boolean |  |
| `state` | query | no | string, one of `pending`, `claimed`, `returned`, `accepted`, `cancelled`, `all`, default `"all"` |  |
| `holder` | query | no | string, uuid |  |
| `created_by` | query | no | string, uuid |  |
| `limit` | query | no | integer, 1 to 200, default `50` |  |
| `after` | query | no | string | the next value of the previous page (R-8) |

**Response:**

- `200`: `application/json` `HandoffPage`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`
- Order: priority descending, then created_at ascending, then id

**Failures:**

- This route's own codes: none
- Responses: `401` Unauthenticated, `403` Forbidden, `422` Unprocessable, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** not a write: safe to retry.

**CLI:** `cortex handoff mine | cortex handoff list`

### `handoffs.show`: GET `/v2/handoffs/{id}`

**Task:** Every packet field exactly as created, INCLUDING acceptance and verify, plus state, holder, round, the latest RETURN (summary, risks, tests, evidence refs

> Every packet field exactly as created, INCLUDING acceptance and verify, plus state, holder, round, the latest RETURN (summary, risks, tests, evidence refs: bob 1) and the latest review (with self_review)

**Who may call:** scope `handoff:read` (class work): project_lead, team_lead, member; auth: a worker token; target rule `None`.

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `id` | path | yes | string, uuid | malformed, unknown or another project's: 404 not_found (R-5) |

**Response:**

- `200`: `application/json` `Handoff`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`

**Failures:**

- This route's own codes: `404 not_found`
- Responses: `401` Unauthenticated, `403` Forbidden, `404` NotFound, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** not a write: safe to retry.

**CLI:** `cortex handoff show <id>`

### `handoffs.claim`: POST `/v2/handoffs/{id}/claim`

**Task:** Claim a pending handoff (the addressee; one winner).

> Claim a pending handoff (the addressee; one winner). Answers {id, state, round}; the client calls show for the packet (bob 9)

**Who may call:** scope `handoff:claim` (class work): the addressee (target rule); auth: a worker token; target rule `SelfWorker`.
Target note: a WORKER addressee; a job-role addressee is checked in the handler with IdentityApi::worker (the Principal has no job roles)

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `id` | path | yes | string, uuid | malformed, unknown or another project's: 404 not_found (R-5) |
| `Idempotency-Key` | header | yes | string, pattern `^[\x20-\x7E]+$` | Required on every mutating route (x-mutates: true). 8 to 128 printable ASCII characters, 0x20 to 0x7E (S2:549; cortex-db idempotency.rs:56). The CLI and cortex-client generate a UUIDv7 per logical operation and reuse it on retry. Bound to (project, principal, METHOD + CONCRETE PATH with every path id, body hash: RFC 8785 canonical JSON). Same key, path and body: the stored status and body are replayed (after the target check, R-19) with Cortex-Idempotent-Replay: true and nothing happens twice. Same key with a different path id or body: 422 idempotency_key_reused. Missing: 428 idempotency_key_required. Two Idempotency-Key headers, or a value outside 8-128 or not printable ASCII: 400 invalid_request (chain step 9; mira M3). Replays are kept 24 hours. Authentication runs before a replay (R-11). |

**Response:**

- `200`: `application/json` `HandoffState`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`, `Cortex-Idempotent-Replay`

**Failures:**

- This route's own codes: `404 not_found`, `409 already_claimed`, `409 invalid_transition`, `403 target_not_allowed`
- Responses: `401` Unauthenticated, `403` Forbidden, `404` NotFound, `422` Unprocessable, `428` IdempotencyKeyRequired, `413` BodyTooLarge, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** a write: send `Idempotency-Key`; the same key with the same body replays the first answer for 24 hours, and the same key with another body is refused (R-11, R-19).

**CLI:** `cortex handoff claim <id>`

### `handoffs.release`: POST `/v2/handoffs/{id}/release`

**Task:** Release a claimed handoff to pending: the holder (reason optional), or the project lead (reason required).

> Release a claimed handoff to pending: the holder (reason optional), or the project lead (reason required). A release also CLEARS the rework lock (rework_holder), so the original addressing applies again (bob Q-17)

**Who may call:** scope `handoff:hold` (class work): the holder; release also the project lead (with a reason); auth: a worker token; target rule `None`.
Target note: None: the holder is mutable STATE, so the holder and lead checks are the handler's own: 409 not_holder; the project lead may release with a reason (cox, on C17: else not_holder could never fire under R-16)

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `id` | path | yes | string, uuid | malformed, unknown or another project's: 404 not_found (R-5) |
| `Idempotency-Key` | header | yes | string, pattern `^[\x20-\x7E]+$` | Required on every mutating route (x-mutates: true). 8 to 128 printable ASCII characters, 0x20 to 0x7E (S2:549; cortex-db idempotency.rs:56). The CLI and cortex-client generate a UUIDv7 per logical operation and reuse it on retry. Bound to (project, principal, METHOD + CONCRETE PATH with every path id, body hash: RFC 8785 canonical JSON). Same key, path and body: the stored status and body are replayed (after the target check, R-19) with Cortex-Idempotent-Replay: true and nothing happens twice. Same key with a different path id or body: 422 idempotency_key_reused. Missing: 428 idempotency_key_required. Two Idempotency-Key headers, or a value outside 8-128 or not printable ASCII: 400 invalid_request (chain step 9; mira M3). Replays are kept 24 hours. Authentication runs before a replay (R-11). |

Body (at most 16384 bytes): `application/json` `Release`

**Response:**

- `200`: `application/json` `HandoffState`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`, `Cortex-Idempotent-Replay`

**Failures:**

- This route's own codes: `404 not_found`, `409 not_holder`, `409 invalid_transition`, `422 reason_required`
- Responses: `401` Unauthenticated, `403` Forbidden, `404` NotFound, `422` Unprocessable, `428` IdempotencyKeyRequired, `413` BodyTooLarge, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** a write: send `Idempotency-Key`; the same key with the same body replays the first answer for 24 hours, and the same key with another body is refused (R-11, R-19).

**CLI:** `cortex handoff release <id> [--reason <text>]`

### `handoffs.return`: POST `/v2/handoffs/{id}/return`

**Task:** Return with a report (the holder)

> Return with a report (the holder): summary, evidence (at least one), risks, tests

**Who may call:** scope `handoff:hold` (class work): the holder; release also the project lead (with a reason); auth: a worker token; target rule `None`.
Target note: None: the holder check is the handler's own (409 not_holder), as for release

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `id` | path | yes | string, uuid | malformed, unknown or another project's: 404 not_found (R-5) |
| `Idempotency-Key` | header | yes | string, pattern `^[\x20-\x7E]+$` | Required on every mutating route (x-mutates: true). 8 to 128 printable ASCII characters, 0x20 to 0x7E (S2:549; cortex-db idempotency.rs:56). The CLI and cortex-client generate a UUIDv7 per logical operation and reuse it on retry. Bound to (project, principal, METHOD + CONCRETE PATH with every path id, body hash: RFC 8785 canonical JSON). Same key, path and body: the stored status and body are replayed (after the target check, R-19) with Cortex-Idempotent-Replay: true and nothing happens twice. Same key with a different path id or body: 422 idempotency_key_reused. Missing: 428 idempotency_key_required. Two Idempotency-Key headers, or a value outside 8-128 or not printable ASCII: 400 invalid_request (chain step 9; mira M3). Replays are kept 24 hours. Authentication runs before a replay (R-11). |

Body (at most 2097152 bytes): `application/json` `HandoffReturn`

**Response:**

- `200`: `application/json` `HandoffState`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`, `Cortex-Idempotent-Replay`

**Failures:**

- This route's own codes: `404 not_found`, `409 not_holder`, `409 invalid_transition`, `422 evidence_required`, `422 risks_required`, `422 evidence_hash_mismatch`, `422 evidence_source_invalid`
- Responses: `401` Unauthenticated, `403` Forbidden, `404` NotFound, `422` Unprocessable, `428` IdempotencyKeyRequired, `413` BodyTooLarge, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** a write: send `Idempotency-Key`; the same key with the same body replays the first answer for 24 hours, and the same key with another body is refused (R-11, R-19).

**CLI:** `cortex handoff return <id> ...`

### `handoffs.accept`: POST `/v2/handoffs/{id}/accept`

**Task:** Accept a returned handoff.

> Accept a returned handoff. The reviewer is the creator or the project lead (else 403 target_not_allowed), never the author while someone else can review (403 self_review_forbidden). The target rule comes FIRST, so a plain-worker author gets target_not_allowed (bob; R-16); the creator who is also the project lead and the only possible reviewer may self-review, marked self_review=true

**Who may call:** scope `handoff:review` (class work): the creator or the project lead, never the author (self-review only as the creator who is the project lead and the only possible reviewer); auth: a worker token; target rule `SelfOrProjectLead`.
Target note: T.worker = the creator; "never the author" is the handler's own check (self_review_forbidden)

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `id` | path | yes | string, uuid | malformed, unknown or another project's: 404 not_found (R-5) |
| `Idempotency-Key` | header | yes | string, pattern `^[\x20-\x7E]+$` | Required on every mutating route (x-mutates: true). 8 to 128 printable ASCII characters, 0x20 to 0x7E (S2:549; cortex-db idempotency.rs:56). The CLI and cortex-client generate a UUIDv7 per logical operation and reuse it on retry. Bound to (project, principal, METHOD + CONCRETE PATH with every path id, body hash: RFC 8785 canonical JSON). Same key, path and body: the stored status and body are replayed (after the target check, R-19) with Cortex-Idempotent-Replay: true and nothing happens twice. Same key with a different path id or body: 422 idempotency_key_reused. Missing: 428 idempotency_key_required. Two Idempotency-Key headers, or a value outside 8-128 or not printable ASCII: 400 invalid_request (chain step 9; mira M3). Replays are kept 24 hours. Authentication runs before a replay (R-11). |

Body (at most 16384 bytes): `application/json` `ReviewNotes`

**Response:**

- `200`: `application/json` `HandoffState`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`, `Cortex-Idempotent-Replay`

**Failures:**

- This route's own codes: `404 not_found`, `409 invalid_transition`, `403 self_review_forbidden`, `403 target_not_allowed`
- Responses: `401` Unauthenticated, `403` Forbidden, `404` NotFound, `422` Unprocessable, `428` IdempotencyKeyRequired, `413` BodyTooLarge, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** a write: send `Idempotency-Key`; the same key with the same body replays the first answer for 24 hours, and the same key with another body is refused (R-11, R-19).

**CLI:** `cortex handoff accept <id>   (v1 form kept as alias: cortex-handoff --return <id> --decision accept; D-3, client owner marv)`

### `handoffs.rework`: POST `/v2/handoffs/{id}/rework`

**Task:** Send a returned handoff back (notes required; same worker; round + 1).

> Send a returned handoff back (notes required; same worker; round + 1). Reviewer rules as accept

**Who may call:** scope `handoff:review` (class work): the creator or the project lead, never the author (self-review only as the creator who is the project lead and the only possible reviewer); auth: a worker token; target rule `SelfOrProjectLead`.
Target note: T.worker = the creator; "never the author" is the handler's own check (self_review_forbidden)

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `id` | path | yes | string, uuid | malformed, unknown or another project's: 404 not_found (R-5) |
| `Idempotency-Key` | header | yes | string, pattern `^[\x20-\x7E]+$` | Required on every mutating route (x-mutates: true). 8 to 128 printable ASCII characters, 0x20 to 0x7E (S2:549; cortex-db idempotency.rs:56). The CLI and cortex-client generate a UUIDv7 per logical operation and reuse it on retry. Bound to (project, principal, METHOD + CONCRETE PATH with every path id, body hash: RFC 8785 canonical JSON). Same key, path and body: the stored status and body are replayed (after the target check, R-19) with Cortex-Idempotent-Replay: true and nothing happens twice. Same key with a different path id or body: 422 idempotency_key_reused. Missing: 428 idempotency_key_required. Two Idempotency-Key headers, or a value outside 8-128 or not printable ASCII: 400 invalid_request (chain step 9; mira M3). Replays are kept 24 hours. Authentication runs before a replay (R-11). |

Body (at most 16384 bytes): `application/json` `ReviewNotesRequired`

**Response:**

- `200`: `application/json` `HandoffState`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`, `Cortex-Idempotent-Replay`

**Failures:**

- This route's own codes: `404 not_found`, `409 invalid_transition`, `422 notes_required`, `403 self_review_forbidden`, `403 target_not_allowed`
- Responses: `401` Unauthenticated, `403` Forbidden, `404` NotFound, `422` Unprocessable, `428` IdempotencyKeyRequired, `413` BodyTooLarge, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** a write: send `Idempotency-Key`; the same key with the same body replays the first answer for 24 hours, and the same key with another body is refused (R-11, R-19).

**CLI:** `cortex handoff rework <id> --notes <text>   (alias: cortex-handoff --return <id> --decision rework)`

### `handoffs.cancel`: POST `/v2/handoffs/{id}/cancel`

**Task:** Cancel an open handoff (pending, claimed or returned)

> Cancel an open handoff (pending, claimed or returned): its creator, the project lead, or the team lead of the WORKER ADDRESSEE's team at cancel time (a job_role addressee has no team); reason required (kai 120d490d)

**Who may call:** scope `handoff:manage` (class work): the creator, the project lead, or the team lead of the worker addressee's team (cancel); auth: a worker token; target rule `Lineage`.
Target note: Lineage with Target{worker: the creator, kind: worker, team: the worker addressee's team at cancel time (None for a job_role addressee), lead}: the creator, the team lead of that team, or the project lead (kai 120d490d; sophi keeps the name, Q-12; a team lead who is the addressee may cancel, Q-13)

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `id` | path | yes | string, uuid | malformed, unknown or another project's: 404 not_found (R-5) |
| `Idempotency-Key` | header | yes | string, pattern `^[\x20-\x7E]+$` | Required on every mutating route (x-mutates: true). 8 to 128 printable ASCII characters, 0x20 to 0x7E (S2:549; cortex-db idempotency.rs:56). The CLI and cortex-client generate a UUIDv7 per logical operation and reuse it on retry. Bound to (project, principal, METHOD + CONCRETE PATH with every path id, body hash: RFC 8785 canonical JSON). Same key, path and body: the stored status and body are replayed (after the target check, R-19) with Cortex-Idempotent-Replay: true and nothing happens twice. Same key with a different path id or body: 422 idempotency_key_reused. Missing: 428 idempotency_key_required. Two Idempotency-Key headers, or a value outside 8-128 or not printable ASCII: 400 invalid_request (chain step 9; mira M3). Replays are kept 24 hours. Authentication runs before a replay (R-11). |

Body (at most 16384 bytes): `application/json` `Reason`

**Response:**

- `200`: `application/json` `HandoffState`; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`, `Cortex-Idempotent-Replay`

**Failures:**

- This route's own codes: `404 not_found`, `409 invalid_transition`, `422 reason_required`, `403 target_not_allowed`
- Responses: `401` Unauthenticated, `403` Forbidden, `404` NotFound, `422` Unprocessable, `428` IdempotencyKeyRequired, `413` BodyTooLarge, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** a write: send `Idempotency-Key`; the same key with the same body replays the first answer for 24 hours, and the same key with another body is refused (R-11, R-19).

**CLI:** `cortex handoff cancel <id> --reason <text>`

### `boot.get`: GET `/v2/boot`

**Task:** The briefing: identity, orientation, the rules in force as POINTERS (section, revision, sha256, source path), claimed handoffs, pending handoffs addressed to me, with per-section budget accounting.

> The briefing: identity, orientation, the rules in force as POINTERS (section, revision, sha256, source path), claimed handoffs, pending handoffs addressed to me, with per-section budget accounting. A budget below what the floor needs (4096 bytes) is 422 boot_budget_too_small with needed and budget in detail (R-13)

**Who may call:** scope `self:read` (class self): every principal of kind worker (R-12); auth: a worker token; target rule `None`.

**Request:**

| Name | In | Required | Schema | Description |
|---|---|---|---|---|
| `budget` | query | no | integer, at most 262144, default `12288` | bytes; over 262144 is 422 invalid_request; below the floor is 422 boot_budget_too_small (no schema minimum: only boot can compute needed) |
| `format` | query | no | string, one of `md`, `json`, default `"md"` |  |

**Response:**

- `200`: `application/json` `Boot`; `text/markdown; charset=utf-8` string; headers `Cortex-Request-Id`, `Cortex-Token-Expires`, `Cortex-Token-Renew`

**Failures:**

- This route's own codes: `422 boot_budget_too_small`
- Responses: `401` Unauthenticated, `403` Forbidden, `422` Unprocessable, `429` Busy, `503` Unavailable, `default` Problem

**Idempotency:** not a write: safe to retry.

**CLI:** `cortex boot [--budget N] [--format md|json]`

## Errors

type = the bare code; every refusal carries fix; a code is minted once; a duplicate or a collision with a platform code is refused at startup (CONTRACTS r7 :434). invalid_request is the one code with several statuses (400, 405, 415, 422: R-2, R-3, R-4). busy is 429 with Retry-After: 1 for a database lock conflict (0b13f10b) or a route's own full concurrency cap (R-9).

### Platform and chain codes

| Code | Status |
|---|---|
| `token_missing` | 401 |
| `token_invalid` | 401 |
| `token_expired` | 401 |
| `token_revoked` | 401 |
| `worker_retired` | 401 |
| `missing_scope` | 403 |
| `target_not_allowed` | 403 |
| `not_found` | 404 |
| `restore_in_progress` | 503 |
| `maintenance` | 503 |
| `capability_unavailable` | 503 |
| `idempotency_key_required` | 428 |
| `idempotency_key_reused` | 422 |
| `already_issued` | 409 |
| `body_too_large` | 413 |
| `invalid_request` | 400, 405, 415, 422 |
| `internal_error` | 500 |
| `unavailable` | 503 |
| `busy` | 429 |
| `conflict` | 409 |

### Piece codes

| Code | Status |
|---|---|
| `already_claimed` | 409 |
| `boot_budget_too_small` | 422 |
| `evidence_hash_mismatch` | 422 |
| `evidence_required` | 422 |
| `evidence_source_invalid` | 422 |
| `files_required` | 422 |
| `invalid_transition` | 409 |
| `job_roles_invalid` | 422 |
| `lead_permanent` | 409 |
| `lineage_state_conflict` | 409 |
| `locator_too_long` | 422 |
| `name_invalid` | 422 |
| `name_taken` | 409 |
| `not_holder` | 409 |
| `notes_required` | 422 |
| `packet_incomplete` | 422 |
| `query_too_long` | 422 |
| `reason_required` | 422 |
| `risks_required` | 422 |
| `section_invalid` | 422 |
| `section_reserved` | 403 |
| `self_review_forbidden` | 403 |
| `self_supersede` | 422 |
| `source_not_allowed` | 422 |
| `source_required` | 422 |
| `target_retired` | 409 |
| `team_lead_taken` | 409 |
| `type_reserved` | 422 |
| `unknown_addressee` | 422 |
| `why_required` | 422 |

### Deferred codes

| Code | Status |
|---|---|
| `token_superseded` | 401 |
| `generation_stale` | 401 |
| `why` | S0 AuthError (C5) has no Superseded or Stale variant; adding them is an S0 change with a delta review (mira M2). In 0.2.001 a superseded token AND a stale generation both answer token_revoked; the generation CHECK is ENFORCED (kai 69b235ad: a restore must not un-revoke a token); only the distinct CODES are deferred. |

### Retired in r2

| Code | Status |
|---|---|
| `affects_unknown` | 404 not_found (R-5) |
| `lineage_target_unknown` | 404 not_found (R-5) |
| `body_not_utf8` | cannot fire through JSON: invalid UTF-8 is a syntax error, 400 invalid_request (R-2) |

### Codes shared by several pieces

W1-34: a problem code declared by more than one PIECE has exactly one status, title and fix; the registry refuses a difference (W1-31). Platform and chain codes (invalid_request, not_found, target_not_allowed, ...) are minted once by the chain and are not listed here.

| Code | Status | Title | Fix |
|---|---|---|---|
| `reason_required` | 422 | Reason required | Send the reason this operation requires, in the form its documentation gives. |
| `target_retired` | 409 | Worker retired | Use an active worker. |

## Host commands (not HTTP)

| Command | What it does |
|---|---|
| `cortex-server bootstrap --project P --lead L --token-file PATH` | host only (podman exec), never HTTP. First run: project + lead + lead token; re-run: reissues the lead token (old one superseded) into a new 0600 file; a new project with its own lead is allowed; a different lead for an existing project is 409. Token never printed. (piece 11 contract s1) |
| `cortex-server migrate` | one-shot; the only place the owner secret is mounted; forward-only; non-zero exit naming the failing migration. |
| `cortex-server --version` | SemVer machine version and git sha; no database. |
