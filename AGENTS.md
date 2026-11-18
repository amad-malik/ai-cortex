# cortex-v2: build and test

Plain commands for every harness (D47). The repository is local, with no remote. Do not push.

## While S0 holds the VM (until 2026-11-05 20:19Z, or cox's "S0 VM released")

`.cargo/config.toml` is the admission template. It is committed byte for byte.
- Every `rustc` call and every test binary goes through `cv2-admit`, at nice 19 and under the shared lock.
- Builds share one target directory, `/path/to/workspace/cortex-v2-target`, with at most 2 jobs.

Start the whole command under admission too:

```
A=/path/to/workspace/spikes/cortex-v2-s0/admission/cv2-admit
CV2_ADMIT_LOG=1 $A cargo test --workspace --no-fail-fast
```

Never start a detached or long-lived container through `cv2-admit` (START §8.1, T9). Containers are named `cv2-a-…` and labelled `cortexv2=dev`.

## Checks (LAYOUT §6)

```
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo deny check
cargo audit
cargo run --locked -p xtask -- fitness   # the crate-graph, source and pin rules (F1-01; D94 pins)
cargo run -p xtask -- licences       # this workspace against deny.toml (F1-02)
cargo test --locked --workspace
```

**`--locked` (Wave 1; kai d2a88cf7):** builds, tests and CI use `--locked`, so cargo refuses a `Cargo.lock` that
would change. A pinned third-party crate (`xtask::PINNED_CRATES`: `serde_ignored` `=0.1.14` and its lock
checksum) is also checked by `xtask fitness`, which fails if the lock or the manifest drifts. A new dependency
is added in the SAME commit as its lock entry and its `INTERIM-REGISTER.md` row (D94).

All of the above in one go: `scripts/ci.sh` (`--text-only` runs only what starts no build). The licence checks are report-only (D94). The deploy files are checked by `scripts/s5-check.sh deploy`.

`cargo xtask …` needs an alias in `.cargo/config.toml`. That file must stay the admission template, so use `cargo run -p xtask --`.

## Who writes what

Each path has one writing lane, given in `OWNERS.toml` (LAYOUT §4). A change to another lane's path goes to that lane's owner as a diff.
