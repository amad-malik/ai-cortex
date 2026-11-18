---
id: cortex-conventions
title: Cortex v2 — conventions every caller shares
kind: reference
status: draft
owner: quill
updated: 2026-11-06
product: cortex
audience: developer
maturity: planned
---

# Conventions every caller shares

Read this once, before the route you came for. Everything here applies to **every** route in Cortex v2, so it is not
repeated per route; the routes themselves are in the generated reference, and where the two disagree, the reference (and
behind it the contract) wins.

**What this page is built from.** The S3 contract, draft **r2.7**, pinned read-only at
`mvp/wave1-pins-20261106-S3r27/S3/cortex-v2-skeleton.openapi.r2.7-8a4c0cb1.yaml` (sha256 `8a4c0cb1…`; JSON twin
`1d2c4b2f…`, `SHA256SUMS` `cd981c79…`), and the data contract **S2 revision 8**
(`mvp/wave1-pins-20261106-S2r8`). The pin is not decoration: every convention below is a fact about *that* pin, and when
the pin moves this page is re-read — as of **r2.6 it was re-read for r2.7**, which is why section 8 carries the
date-time rule the contract now states in full.

## 1. Every refusal is the same shape

A refusal is `application/problem+json`, never a bare error string, and every member has a job:

| Member | What it is for |
|---|---|
| `type` | the stable machine identifier of the refusal, e.g. `missing_scope` |
| `title` | a short human phrase for the same thing |
| `status` | the HTTP status, repeated in the body so a log line is readable alone |
| `detail` | dynamic text: **may name the field the client sent**, and for `boot_budget_too_small` the **needed and budget byte counts** (R-13); it **never** carries a body, a secret or another project's data |
| `fix` | fixed text per code: what the caller should do |
| `request_id` | the same UUIDv7 as the `Cortex-Request-Id` header |
| `scope` | present only on a `503` admission refusal: **which admission refused**, an enum of `install` or `project` — it is not an authorisation scope, so a caller should never look for a value like `records:write` here |

Two rules that follow from that table. **A client branches on `type`, never on `title` or `detail`** — the first is
stable, the other two are written for a person. And **`fix` is not a suggestion you can rely on for logic**: it is the
same sentence for every caller who hits that code.

## 2. Lengths are bytes, not characters

Every limit in the contract is **UTF-8 bytes**: `x-max-bytes` is the real limit, and `maxLength` carries the same number
only as a looser character bound. A 200-character string of Greek is 400 bytes. The handler checks bytes **before** the
database is touched, so an over-long field is a stateless `422` and never a `500` and never a half-written row.

`NUL` (U+0000) anywhere in a string is `422 invalid_request`. Invalid UTF-8, or a lone surrogate escape, is a JSON
**syntax** error: `400`.

## 3. Aggregate limits are measured on the JSONB text

Some fields are bounded *together*, and the arithmetic is the data contract's, not the API's: the limit is the octet
length of the **JSONB text** of each array. So `["x"]` counts the value **plus its quotes and brackets**, and a
comma-space between two elements counts 2. Where the API page and S2 disagree about an aggregate, **S2 wins**.

## 4. Labels have four grammars, and they are checked on items too

Four kinds of identifier-like string — names, job roles, revoke codes, audit actions — take their pattern from one
table. `name`: `^[A-Za-z][A-Za-z0-9-]{0,31}$` · `job_role`: `^[a-z][a-z0-9-]{0,31}$` · `revoke_code`:
`^[a-z][a-z_]{0,31}$` · `audit_action`: `^[a-z][a-z0-9_.:-]{0,63}$`.

Three things a checker gets wrong:

- **The grammar applies to array items**, not only to a scalar property. `job_roles` is a list; each element is checked.
- **The pattern dialect is ECMA-262**, so `^` and `$` anchor the whole string and a trailing newline does **not** match.
  Python's `re.search` would accept `name\n` — use `re.fullmatch`.
- **A label outside its grammar is `422`**, with the field's own code when it has one (`name_invalid`,
  `job_roles_invalid`), else `invalid_request`. It is never silently truncated or normalised.

## 5. Idempotency: a key, a binding, and a replay that never leaks

Every mutating route requires `Idempotency-Key`: 8 to 128 printable ASCII characters. The key is bound to the
**project, the principal, the method, the concrete path (with every path id) and the canonical body hash**, and a
committed success is replayed for **24 hours**.

What a replay is *not*: a way around authorisation. Authentication, the scope check and the **current** target rule run
before any stored reply is returned, so a retry by a caller who has since lost access gets exactly the refusal a first
call would have got. The same key with a different path or body is `422 idempotency_key_reused`, and no refusal is ever
stored — only a committed success is.

## 6. The order in which refusals are decided

When several refusals could apply, only one wins, and the order is fixed. It is worth knowing because it explains why
your second guess was the answer:

1. **The chain**: request id → install admission (a no-op in 0.2.001) → authentication (`401`) → project admission (a
   no-op) → limiter → body size (`413`) → **scope only** (`403 missing_scope`) → strict decode (`400` syntax, `422`
   shape) → the idempotency **header** (`400`, `428`). *The contract numbers these steps 1 to 12; this page groups them
   into nine arrows, so a reader who counts will count differently and both are right.*
2. **"Step 0"** — the contract's own name for this phase — **and no database read happens yet**: every per-field check — length, pattern, enum, range, NUL, a field's own
   code — **before** every cross-field rule (`oneOf`/`anyOf`, the conditional `source` and `orientation` rules, an
   aggregate limit). The first refusal wins, in schema order within each group.
3. **Then the checks that need a read**: `404` (the path resource, then body references) → `403` (the **target rule
   first**, then `self_review_forbidden`) → the **idempotency lookup** → `409` (`invalid_transition` before
   `not_holder`) → and last the `422`s that need a read.
4. **Two exceptions, both deliberate**: retiring a worker answers `409 lead_permanent` *before* `403`, or that code could
   never fire; and search checks its bounds (`422`) and then its own cap (`429 busy`) both before its first statement,
   so an invalid request takes no slot.

## 7. Words that are in the contract and refused anyway

A contract can contain a word and still refuse it. In 0.2.001: the record types `human-message`, `drill-canary`,
`drill` and `conversation` are `422 type_reserved`; the section `project-brief` is `422 section_invalid`; and the four
revoke reason codes `superseded`, `reissued`, `retired`, `replaced_unused` are **the server's own** — a client sending
one is `422 invalid_request` at step 0.

## 8. Time, ids and paging

- **Dates** follow the contract's own **`x-notes.datetime-rule`** (ruling W1-39), and every date-time input of a
  body, a query or a cursor obeys it: `YYYY-MM-DDTHH:MM:SS[.f{1,9}](Z|+HH:MM|-HH:MM)` with **upper-case `T` and `Z`**, a
  real calendar date, **year 1970 to 9999**, hour 0–23, minute 0–59, second 0–59 (no leap second), offset minutes 0–59
  and the **offset within -12:00 to +14:00**. NUL or any other character is rejected; a reject is `422
  invalid_request`, never a database error and never `500`, and the value is **not normalised to UTC**. The shared
  vectors are `rfc3339-vectors.json` beside the pin (sha256 `4125a4b4…`): 14 accepted, 48 rejected — a client that
  passes those 62 is done arguing about dates.
- **Ids** are UUIDv7. An id in the **path** that names nothing in your project is `404 not_found` — the same answer for
  "unknown" and "another project's", because the difference is not yours to learn. A malformed id in the **body** fails
  decoding first (`422`).
- **Lists** answer `{items, next}`; `limit` defaults to 50 and caps at 200 (search has its own 1–50). A cursor is a
  **position, never authority**: it cannot select rows outside your project.

> **The generated page is beside this one now.** `docs/generated/API-REFERENCE.md` is produced by piece 13's renderer from
> the pinned contract and is committed here, so the link below resolves. Two things about it worth knowing: it is a **build
> product** — correct the contract and re-run the renderer, never edit it — and it carries the contract's own version in its
> first lines, so a reader can always tell which pin they are looking at. Where this page and that one disagree, the contract
> decides and one of the two is stale.

## 9. What this page deliberately does not repeat

- **The routes, the codes and the limits**: the [generated reference](../generated/API-REFERENCE.md) owns them, and it
  is built from the pin. A second copy here would drift from the first.
- **What an operator does when a call fails**: the install guide owns the steps and the dogfood guide owns the loop.
- **The data contract's own rules** — table shapes, row-level security, retention: `S2` owns them and this page only says
  where the API's arithmetic comes from.

## 10. If you are checking this page

The two questions worth asking of any claim here: **does the pin still hash to `ef180824…`**, and **does the reference
say the same**. If the first fails, this page is stale and belongs on the redraw queue, not in a bug report; if the
second fails, one of the two is wrong and the contract decides which.
