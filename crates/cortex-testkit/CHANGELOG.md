# cortex-testkit changelog

Changes to the reviewed supervisor (`supervise.sh`, `group_runner.py`) and the fixtures, for the
next review pass. Newest first. The W1-305 ruling explicitly updates `controls/`.

## 2026-11-15, W1-305

- Give each supervised PostgreSQL run a named, owner-labelled volume and mount it at
  `/var/lib/postgresql`. After the container is verified absent, remove only that run's
  volume and independently verify its absence; failure or uncertainty is not success.
- The earlier `--volumes` change did not prevent leaks when `--rm` removed the container
  first. The W1-305 fake-Podman cells cover the lifecycle without starting a database.
- W1-305a extends the controls' forced-removal follow-up to verify the recorded container's
  owner, remove its exact matching volume after container absence, and verify volume absence.
- W1-305b adds a general per-case follow-up for fail-closed and other paths: it checks the
  recorded run identity and volume owner, refuses referenced volumes, and verifies removal.

## 2026-11-05, Wave 0 (branch `wave0/socket`)

- **`afb7d9c`, `3201c6e` `pg.rs`: `empty_db(tag)`.** It creates a database of its own with the
  roles and NO fixture schema, so a piece's migrations are proved on a truly empty database (cox's
  freeze bar, P-03).
  - `create_database` became `create_database_with(name, fixture)`. `isolated_db` and `test_db`
    pass `true`, so their behaviour is unchanged.
  - New test `tests/w0_empty_db.rs`; its control shows `isolated_db` still has `test_a`,
    `test_audit` and `test_b`.
- **`e55c55b` `supervise.sh`: `podman rm --force --volumes "$NAME"`** at its three removals of the
  exact container name (cleanup, the cleanup retry, and the start-retry remnant).
  - **Why:** the Postgres image declares `VOLUME /var/lib/postgresql`, so every run gets one
    anonymous volume. `--rm` removes it with the container, but cleanup's own `rm` without
    `--volumes` can win that race and leave the volume behind.
  - **Who:** cox's ruling 421ba505, adopting finn's proposal in e3ba6c5f.
  - **Change:** blob `ed04f3b2` to `0f978c5d`; sha256 `b9a72ab7…` to `297a8c7c…`. Nothing else
    changed.
  - **Checked:** `bash -n` passes. The fake `podman` in `controls/` keys on the subcommand only;
    the controls were not re-run.
  - **Observed** in four supervised runs on 5 Nov, 21:31Z to 21:35Z: each container verified
    absent, and the count of dangling volumes was unchanged (76, all from before 5 Nov).
- **`d0a9243` `maintenance` feature:** `src/maint.rs` (the F1-31 fixtures) is built only with this
  crate's `maintenance` feature, which turns on `cortex-db/maintenance` and `sha2`. Banked for
  v0.2.001.
