# Known limits of the CLI output boundary (0.2.001)

Owner: sophi. Source: vera's disposition of CLI batch 1 (E-2, N-3, F-4; `BATCH1-REVIEW.md`, 7 Nov 2026) and cox's design
decision of 18:01:59Z the same day: ACCEPTED AS STATED for 0.2.001, with NO claim of a typed boundary. quill includes this file as the
"known limits" of the CLI. A typed output boundary is a follow-up for 0.2.002, not a condition of this release.

What the CLI does, and where it stops. The ERROR DIAGNOSTICS the CLI prints (title, detail, fix) go through
`cortex_client::display::safe_text` (md) or `json_escape_unsafe` (json lines), and the Health strings of `cortex doctor` in MD output
go through `safe_text`; the token is redacted in those. The records of `cortex audit list` (a success body) go through the same two
helpers, but the token is NOT redacted in them (item 8). One list of unsafe characters decides: every control character, U+2028,
U+2029, U+200E, U+200F, U+061C, U+202A..U+202E, U+2066..U+2069 and the tag block U+E0000..U+E007F. ONE PATH IS EXPLICITLY EXCLUDED:
`cortex doctor --format json` prints the response body as received, so that raw body is neither passed through those helpers nor
token-redacted by them (item 5); this note does not claim universal redaction.

The limits, each stated as it is:

1. **No typed boundary.** The guard `n3_no_group_file_names_problem_or_reads_a_servers_text_fields` (`tests/registry.rs`) is a LEXICAL
   TRIPWIRE: it scans `src/groups/*.rs` for the name `Problem` and for `.detail`, `.fix`, `.title`, with a naive split of comments. It
   does not see a server string reached by another path (`Busy.last`, `Problem.scope`, an index into a JSON value, a success body), and it
   can block an unrelated field that has one of those names. A group that prints a server string raw passes the type checker.
2. **The mutation proof is partial.** Mutant M-41 selects the client display unit tests only. There is no mutant of the BINARY for the LF
   and TAB cases (N-2); the binary's assertions exist (`cli_process.rs`) but no mutation run proves they would fail.
3. **64 is a display policy, not a schema limit.** The Health schema has no `maxLength`. `cortex doctor` in md cuts every Health string
   (state, version, piece names, `last_good_dump_at`) to 64 characters (`BUDGET_IDENT`) with the visible marker; a valid longer value is
   clipped in md. The structured data is not changed.
4. **The number of pieces in `doctor` is unbounded.** Each value is cut; the total printed is not.
5. **Raw doctor JSON (F-4).** `cortex doctor --format json` prints the body as received. A guard cell keeps it so; it is excluded from
   this batch.
6. **The list of unsafe characters is a list.** Other invisible characters (zero-width space, U+2060, U+FEFF, the variation selectors
   supplement) pass by design and are pinned as staying in `display.rs`. No terminal was tested: what a given terminal does with a
   character outside the list, or with one inside it, is not claimed.
7. **Raw fields stay raw in the structured output.** The server's fields in json output are kept as the server sent them, escaped
   for the line only (cox decision 1 of 6 Nov: public by design).
8. **`cortex audit list` prints what the server sends, escaped, not redacted.** Its records are a success body, so the tripwire of
   item 1 does not look at them: the proof is the cells AU-05 and AU-09 and the mutants MA-5 and MA-6 (`tests/audit.rs`), not the type
   system. The token is not redacted in them (the contract keeps secrets out of audit details; that the CLIENT never prints the token it
   sends is cell AU-18). In md a value with spaces blurs the columns, and the number of `target` and `details` pairs is not bounded; in
   json the key order is sorted, not the server's.
9. **`cortex audit list --all` stops at 20 pages and exits 8, incomplete.** What was fetched is printed (md) or is the json object whose
   `next` is the continuation cursor, and stderr says `incomplete: 20 pages were printed and more records exist; continue with --after
   <cursor>`; exit 8 is in the exit table (cox W1-90), so a script is told. A capped audit listing is not a complete export. The log is
   a PLAIN log in 0.2.001: no chain, no tamper evidence (`audit verify` waits for the hash-chain release).
