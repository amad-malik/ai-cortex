//! Piece 14, design §4b, UP-29: the `db` service must not change in an upgrade. A strict reader of one block (cox,
//! no YAML crate): the shapes it accepts, every refusal class on both sides, equal and different pairs, the top-level
//! entries the `db` service names, and that no message ever holds a value.

use cortex_upgrade::compose::{ComposeError, Refusal, Side, compare_db_service};
use cortex_upgrade::stack_env::{StackEnv, parse_stack_env};

const D: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

const COMPOSE: &str = r#"# Cortex v2 skeleton: a comment that is not part of any service
name: cv2-s5

services:
  db:
    # the pinned image comes from stack.env
    image: ${CORTEX_DB_IMAGE}
    container_name: cv2-s5-db
    restart: unless-stopped
    init: true
    shm_size: 256m
    mem_limit: 2g
    cpus: 1.0
    labels:
      cortexv2: dev
    command:
      - postgres
      - -c
      - shared_buffers=256MB
      - -c
      - effective_cache_size=1GB
    environment:
      POSTGRES_DB: cortex
      POSTGRES_INITDB_ARGS: --auth-local=peer --auth-host=scram-sha-256 -E UTF8 --data-checksums
      POSTGRES_PASSWORD_FILE: /run/secrets/cv2-s5-db-superuser
    secrets:
      - cv2-s5-db-superuser
      - cv2-s5-owner-password
      - cv2-s5-app-password
    volumes:
      - cv2-s5-pg-data:/var/lib/postgresql
      - ./initdb:/docker-entrypoint-initdb.d:ro,z
    networks:
      - cv2-s5-net
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -q -U postgres -h 127.0.0.1 -d cortex >/dev/null 2>&1"]
      interval: 15s
      timeout: 20s
      retries: 20
      start_period: 10s

  migrate:
    image: localhost/cv2-s5-server:${CORTEX_VERSION}
    command: ["migrate"]
    restart: "no"

  server:
    image: localhost/cv2-s5-server:${CORTEX_VERSION}
    ports:
      - "127.0.0.1:${CORTEX_PORT:-8601}:8601"

networks:
  cv2-s5-net:
    name: cv2-s5-net

volumes:
  cv2-s5-pg-data:
    name: cv2-s5-pg-data
  cv2-s5-state:
    name: cv2-s5-state

# Podman secrets, created by install.sh.
secrets:
  cv2-s5-db-superuser:
    external: true
  cv2-s5-owner-password:
    external: true
  cv2-s5-app-password:
    external: true
"#;

fn env(db_image: &str, version: &str) -> StackEnv {
    parse_stack_env(&format!(
        "CORTEX_VERSION={version}\nCORTEX_SERVER_IMAGE=localhost/cortex-server@{D}\nCORTEX_DB_IMAGE={db_image}\n"
    ))
    .unwrap()
}

fn db_a() -> String {
    format!("docker.io/pgvector/pgvector:0.8.2-pg18@{D}")
}

/// One occurrence, replaced; the test fails if the fixture text is not there exactly once.
fn edit(text: &str, from: &str, to: &str) -> String {
    assert_eq!(text.matches(from).count(), 1, "fixture text {from:?}");
    text.replacen(from, to, 1)
}

fn cmp(current: &str, candidate: &str) -> Result<(), ComposeError> {
    compare_db_service(
        (current, &env(&db_a(), "0.2.1-rc.1")),
        (candidate, &env(&db_a(), "0.2.1-rc.2")),
    )
}

fn assert_safe(e: &ComposeError) {
    let m = e.to_string();
    assert!(
        !m.is_empty() && m.len() < 300 && !m.chars().any(char::is_control),
        "{m:?}"
    );
    assert!(
        !m.contains("SYNTHETIC") && !m.contains("hunter2") && !m.contains("sha256"),
        "a value is in the message: {m}"
    );
}

fn differs_at(r: Result<(), ComposeError>, path: &str) {
    match r {
        Err(e @ ComposeError::Differs { .. }) => {
            assert_eq!(e, ComposeError::Differs { path: path.into() });
            assert_safe(&e);
        }
        other => panic!("expected a difference at {path}, got {other:?}"),
    }
}

fn cannot(r: Result<(), ComposeError>, side: Side, class: Refusal, path: &str) {
    match r {
        Err(e @ ComposeError::Cannot { .. }) => {
            assert_eq!(
                e,
                ComposeError::Cannot {
                    side,
                    class,
                    path: path.into()
                }
            );
            assert_safe(&e);
        }
        other => panic!("expected a refusal ({class:?} at {path}), got {other:?}"),
    }
}

// ---------------------------------------------------------------- equal

#[test]
fn cm_01_identical_files_are_equal_and_a_version_bump_alone_is_no_difference() {
    assert_eq!(cmp(COMPOSE, COMPOSE), Ok(()));
    // CORTEX_VERSION and the server image differ between the two stack.env files; the db service does not use them
    let a = env(&db_a(), "0.2.1-rc.1");
    let b = parse_stack_env(&format!(
        "CORTEX_SERVER_IMAGE=localhost/other@{D}\nCORTEX_VERSION=9.9.9\nCORTEX_DB_IMAGE={}\n",
        db_a()
    ))
    .unwrap();
    assert_eq!(compare_db_service((COMPOSE, &a), (COMPOSE, &b)), Ok(()));
}

#[test]
fn cm_02_key_order_comments_blank_lines_and_indentation_width_do_not_matter() {
    // environment keys reordered
    let env_block = "      POSTGRES_DB: cortex\n      POSTGRES_INITDB_ARGS: --auth-local=peer --auth-host=scram-sha-256 -E UTF8 --data-checksums\n      POSTGRES_PASSWORD_FILE: /run/secrets/cv2-s5-db-superuser\n";
    let reordered = "      POSTGRES_PASSWORD_FILE: /run/secrets/cv2-s5-db-superuser\n      POSTGRES_DB: cortex\n      POSTGRES_INITDB_ARGS: --auth-local=peer --auth-host=scram-sha-256 -E UTF8 --data-checksums\n";
    assert_eq!(cmp(COMPOSE, &edit(COMPOSE, env_block, reordered)), Ok(()));
    // the whole healthcheck block moved above the image
    let hc = "    healthcheck:\n      test: [\"CMD-SHELL\", \"pg_isready -q -U postgres -h 127.0.0.1 -d cortex >/dev/null 2>&1\"]\n      interval: 15s\n      timeout: 20s\n      retries: 20\n      start_period: 10s\n";
    let moved = edit(
        &edit(COMPOSE, hc, ""),
        "    # the pinned image comes from stack.env\n",
        &format!("{hc}    # the pinned image comes from stack.env\n"),
    );
    assert_eq!(cmp(COMPOSE, &moved), Ok(()));
    // comments: changed, added, removed; blank lines added
    let commented = edit(
        &edit(
            COMPOSE,
            "    # the pinned image comes from stack.env\n",
            "\n    # a different comment\n      # an odd one\n\n",
        ),
        "    init: true\n",
        "    # new\n\n    init: true\n",
    );
    assert_eq!(cmp(COMPOSE, &commented), Ok(()));
    // every indentation doubled (still consistent)
    let wide: String = COMPOSE
        .lines()
        .map(|l| {
            let n = l.len() - l.trim_start().len();
            format!("{}{}\n", " ".repeat(n * 2), l.trim_start())
        })
        .collect();
    assert_eq!(cmp(COMPOSE, &wide), Ok(()));
}

#[test]
fn cm_03_the_other_services_are_not_compared_and_may_hold_anything() {
    let other = edit(
        &edit(
            COMPOSE,
            "    command: [\"migrate\"]\n    restart: \"no\"\n",
            "    command: |\n      migrate it\n    restart: &r always\n    x: *r\n",
        ),
        "      - \"127.0.0.1:${CORTEX_PORT:-8601}:8601\"\n",
        "      - 8601\n    environment: {A: B}\n",
    );
    assert_eq!(cmp(COMPOSE, &other), Ok(()));
}

// ---------------------------------------------------------------- different

#[test]
fn cm_04_a_changed_value_anywhere_in_the_db_service_is_named_by_its_path() {
    for (from, to, path) in [
        (
            "--data-checksums",
            "SYNTHETIC-VALUE-B",
            "services.db.environment.POSTGRES_INITDB_ARGS",
        ),
        (
            "      cortexv2: dev",
            "      cortexv2: SYNTHETIC-VALUE-B",
            "services.db.labels.cortexv2",
        ),
        (
            "      interval: 15s",
            "      interval: 16s",
            "services.db.healthcheck.interval",
        ),
        (
            "-d cortex >/dev/null",
            "-d SYNTHETIC-VALUE-B >/dev/null",
            "services.db.healthcheck.test[1]",
        ),
        (
            "    restart: unless-stopped",
            "    restart: always",
            "services.db.restart",
        ),
        (
            "    mem_limit: 2g",
            "    mem_limit: 3g",
            "services.db.mem_limit",
        ),
        (
            "    container_name: cv2-s5-db",
            "    container_name: other",
            "services.db.container_name",
        ),
        (
            "      - shared_buffers=256MB",
            "      - shared_buffers=512MB",
            "services.db.command[2]",
        ),
        (
            "      - ./initdb:/docker-entrypoint-initdb.d:ro,z",
            "      - ./initdb:/docker-entrypoint-initdb.d:ro",
            "services.db.volumes[1]",
        ),
    ] {
        differs_at(cmp(COMPOSE, &edit(COMPOSE, from, to)), path);
        differs_at(cmp(&edit(COMPOSE, from, to), COMPOSE), path); // and the other way round
    }
}

#[test]
fn cm_05_an_added_or_removed_key_or_item_is_a_difference_and_list_order_matters() {
    differs_at(
        cmp(
            COMPOSE,
            &edit(
                COMPOSE,
                "    init: true\n",
                "    init: true\n    privileged: true\n",
            ),
        ),
        "services.db.privileged",
    );
    differs_at(
        cmp(COMPOSE, &edit(COMPOSE, "    init: true\n", "")),
        "services.db.init",
    );
    differs_at(
        cmp(
            COMPOSE,
            &edit(
                COMPOSE,
                "      POSTGRES_DB: cortex\n",
                "      POSTGRES_DB: cortex\n      EXTRA: SYNTHETIC-VALUE-B\n",
            ),
        ),
        "services.db.environment.EXTRA",
    );
    differs_at(
        cmp(
            COMPOSE,
            &edit(
                COMPOSE,
                "      - cv2-s5-app-password\n    volumes:",
                "    volumes:",
            ),
        ),
        "services.db.secrets[2]",
    );
    differs_at(
        cmp(
            COMPOSE,
            &edit(
                COMPOSE,
                "      - cv2-s5-db-superuser\n      - cv2-s5-owner-password\n",
                "      - cv2-s5-owner-password\n      - cv2-s5-db-superuser\n",
            ),
        ),
        "services.db.secrets[0]",
    );
    differs_at(
        cmp(
            COMPOSE,
            &edit(
                COMPOSE,
                "      - -c\n      - shared_buffers=256MB\n      - -c\n",
                "      - shared_buffers=256MB\n      - -c\n      - -c\n",
            ),
        ),
        "services.db.command[1]",
    );
}

#[test]
fn cm_06_the_image_comes_from_stack_env_and_quoting_is_part_of_the_value() {
    let a = env(&db_a(), "0.2.1-rc.1");
    let b = env(
        &format!("docker.io/pgvector/pgvector:0.8.3-pg18@{D}"),
        "0.2.1-rc.1",
    );
    differs_at(
        compare_db_service((COMPOSE, &a), (COMPOSE, &b)),
        "services.db.image",
    );
    differs_at(
        cmp(
            COMPOSE,
            &edit(
                COMPOSE,
                "    restart: unless-stopped",
                "    restart: \"unless-stopped\"",
            ),
        ),
        "services.db.restart",
    );
    // a default and a required form with a variable that IS in stack.env are replaced
    let with_default = edit(
        COMPOSE,
        "    container_name: cv2-s5-db",
        "    container_name: x-${CORTEX_VERSION:-none}",
    );
    let with_required = edit(
        COMPOSE,
        "    container_name: cv2-s5-db",
        "    container_name: x-${CORTEX_VERSION:?missing}",
    );
    // the same version in both stack.env files: `${V:-d}` and `${V:?m}` are the same value
    let same = env(&db_a(), "0.2.1-rc.1");
    assert_eq!(
        compare_db_service((&with_default, &same), (&with_required, &same)),
        Ok(()),
        "a variable that is set gives the same value in both forms"
    );
    // a variable the db service USES and that differs between the two releases is a difference
    differs_at(
        cmp(&with_default, &with_default),
        "services.db.container_name",
    );
}

#[test]
fn cm_06b_labels_as_a_list_instead_of_a_mapping_is_a_difference_not_a_refusal() {
    // both forms are legal compose; they are different shapes, so they differ
    differs_at(
        cmp(
            COMPOSE,
            &edit(
                COMPOSE,
                "    labels:\n      cortexv2: dev",
                "    labels:\n      - cortexv2=dev",
            ),
        ),
        "services.db.labels",
    );
}

// ---------------------------------------------------------------- refusals

struct Case {
    what: &'static str,
    from: &'static str,
    to: &'static str,
    class: Refusal,
    path: &'static str,
}

fn refusal_cases() -> Vec<Case> {
    let c = |what, from, to, class, path| Case {
        what,
        from,
        to,
        class,
        path,
    };
    vec![
        c(
            "an anchor",
            "    restart: unless-stopped",
            "    restart: &r unless-stopped",
            Refusal::Anchor,
            "services.db.restart",
        ),
        c(
            "an alias",
            "    restart: unless-stopped",
            "    restart: *r",
            Refusal::Alias,
            "services.db.restart",
        ),
        c(
            "a merge key",
            "    init: true\n",
            "    <<: *base\n    init: true\n",
            Refusal::MergeKey,
            "services.db",
        ),
        c(
            "a tag",
            "    init: true",
            "    init: !!bool true",
            Refusal::Tag,
            "services.db.init",
        ),
        c(
            "a literal block scalar",
            "    shm_size: 256m",
            "    shm_size: |\n      256m",
            Refusal::BlockScalar,
            "services.db.shm_size",
        ),
        c(
            "a folded block scalar",
            "    shm_size: 256m",
            "    shm_size: >-\n      256m",
            Refusal::BlockScalar,
            "services.db.shm_size",
        ),
        c(
            "a flow mapping",
            "    labels:\n      cortexv2: dev",
            "    labels: {cortexv2: dev}",
            Refusal::FlowMapping,
            "services.db.labels",
        ),
        c(
            "a multi-line plain value",
            "    mem_limit: 2g\n",
            "    mem_limit: 2g\n      and more\n",
            Refusal::MultiLine,
            "services.db.mem_limit",
        ),
        c(
            "a multi-line quoted value",
            "\"pg_isready -q -U postgres -h 127.0.0.1 -d cortex >/dev/null 2>&1\"]",
            "\"pg_isready",
            Refusal::MultiLine,
            "services.db.healthcheck.test",
        ),
        c(
            "a tab",
            "      cortexv2: dev",
            "\tcortexv2: dev",
            Refusal::Tab,
            "services.db",
        ),
        c(
            "a repeated key",
            "    init: true\n",
            "    init: true\n    init: false\n",
            Refusal::DuplicateKey,
            "services.db.init",
        ),
        c(
            "a bare variable",
            "    shm_size: 256m",
            "    shm_size: $HOME",
            Refusal::UnknownInterpolation,
            "services.db.shm_size",
        ),
        c(
            "a ${..:+..} form",
            "    shm_size: 256m",
            "    shm_size: ${CORTEX_DB_IMAGE:+x}",
            Refusal::UnknownInterpolation,
            "services.db.shm_size",
        ),
        c(
            "a $$ escape",
            "    shm_size: 256m",
            "    shm_size: $${X}",
            Refusal::UnknownInterpolation,
            "services.db.shm_size",
        ),
        c(
            "a nested default",
            "    shm_size: 256m",
            "    shm_size: ${CORTEX_VERSION:-${CORTEX_DB_IMAGE}}",
            Refusal::UnknownInterpolation,
            "services.db.shm_size",
        ),
        c(
            "an unclosed ${",
            "    shm_size: 256m",
            "    shm_size: ${CORTEX_VERSION",
            Refusal::UnknownInterpolation,
            "services.db.shm_size",
        ),
        c(
            "a variable that is not in stack.env",
            "    shm_size: 256m",
            "    shm_size: ${HOME}",
            Refusal::UnknownVariable,
            "services.db.shm_size",
        ),
        c(
            "a required variable that is not in stack.env",
            "    shm_size: 256m",
            "    shm_size: ${CORTEX_CONFIG:?set by install.sh}",
            Refusal::UnknownVariable,
            "services.db.shm_size",
        ),
        c(
            "a default for a variable that is not in stack.env",
            "    shm_size: 256m",
            "    shm_size: ${CORTEX_PORT:-8601}",
            Refusal::UnknownVariable,
            "services.db.shm_size",
        ),
        c(
            "a comment after a value",
            "    cpus: 1.0",
            "    cpus: 1.0 # one cpu",
            Refusal::InlineComment,
            "services.db.cpus",
        ),
        c(
            "a quoted key",
            "    init: true",
            "    \"init\": true",
            Refusal::BadKey,
            "services.db",
        ),
        c(
            "an empty key",
            "    init: true",
            "    : true",
            Refusal::BadKey,
            "services.db",
        ),
        c(
            "a key with a space",
            "    init: true",
            "    in it: true",
            Refusal::BadKey,
            "services.db",
        ),
        c(
            "a key with a variable",
            "    init: true",
            "    ${X}: true",
            Refusal::BadKey,
            "services.db",
        ),
        c(
            "uneven indentation",
            "      interval: 15s",
            "     interval: 15s",
            Refusal::BadIndent,
            "services.db.healthcheck",
        ),
        c(
            "a list item that is a mapping",
            "      - cv2-s5-net",
            "      - name: cv2-s5-net",
            Refusal::UnsupportedShape,
            "services.db.networks[0]",
        ),
        c(
            "a mapping on one line",
            "    cpus: 1.0",
            "    cpus: a: b",
            Refusal::UnsupportedShape,
            "services.db.cpus",
        ),
        c(
            "a list on a key line",
            "    cpus: 1.0",
            "    cpus: - 1.0",
            Refusal::UnsupportedShape,
            "services.db.cpus",
        ),
        c(
            "a nested flow list",
            "[\"CMD-SHELL\", \"pg_isready",
            "[\"CMD-SHELL\", [\"x\"], \"pg_isready",
            Refusal::UnsupportedShape,
            "services.db.healthcheck.test",
        ),
        c(
            "an escape in a quoted value",
            "\"CMD-SHELL\"",
            "\"CMD\\nSHELL\"",
            Refusal::Escape,
            "services.db.healthcheck.test[0]",
        ),
        c(
            "a complex key",
            "    init: true",
            "    ? init\n    : true",
            Refusal::BadKey,
            "services.db",
        ),
    ]
}

#[test]
fn cm_07_every_refusal_class_is_refused_on_both_sides_with_a_path_and_no_value() {
    assert_eq!(cmp(COMPOSE, COMPOSE), Ok(()), "control");
    for case in refusal_cases() {
        let bad = edit(COMPOSE, case.from, case.to);
        cannot(cmp(COMPOSE, &bad), Side::Candidate, case.class, case.path);
        cannot(cmp(&bad, COMPOSE), Side::Current, case.class, case.path);
        // refused even when both sides are the same bad file
        assert!(
            matches!(
                cmp(&bad, &bad),
                Err(ComposeError::Cannot {
                    side: Side::Current,
                    ..
                })
            ),
            "{}",
            case.what
        );
    }
}

#[test]
fn cm_08_whole_file_refusals() {
    for (what, text, class) in [
        (
            "include",
            format!("include:\n  - other.yml\n{COMPOSE}"),
            Refusal::Include,
        ),
        (
            "extends",
            edit(
                COMPOSE,
                "  server:\n",
                "  server:\n    extends:\n      service: db\n",
            ),
            Refusal::Extends,
        ),
        (
            "an extension field",
            format!("x-common: &c\n  a: b\n{COMPOSE}"),
            Refusal::TopLevelKey,
        ),
        (
            "a configs section",
            format!("{COMPOSE}configs:\n  c:\n    file: ./c\n"),
            Refusal::TopLevelKey,
        ),
        (
            "a second document",
            format!("{COMPOSE}---\nname: other\n"),
            Refusal::MultiDocument,
        ),
        (
            "a document marker first",
            format!("---\n{COMPOSE}"),
            Refusal::MultiDocument,
        ),
        (
            "no services",
            edit(COMPOSE, "services:\n", "servicez:\n"),
            Refusal::TopLevelKey,
        ),
    ] {
        cannot(cmp(COMPOSE, &text), Side::Candidate, class, "compose.yml");
        let _ = what;
    }
    cannot(
        cmp(COMPOSE, &edit(COMPOSE, "  db:\n", "  database:\n")),
        Side::Candidate,
        Refusal::Missing,
        "services.db",
    );
    let twice = edit(
        COMPOSE,
        "  migrate:\n",
        "  db:\n    image: x\n\n  migrate:\n",
    );
    cannot(
        cmp(COMPOSE, &twice),
        Side::Candidate,
        Refusal::DuplicateKey,
        "services.db",
    );
}

#[test]
fn cm_09_size_limits() {
    let long_line = edit(
        COMPOSE,
        "    cpus: 1.0",
        &format!("    cpus: {}", "1".repeat(1100)),
    );
    cannot(
        cmp(COMPOSE, &long_line),
        Side::Candidate,
        Refusal::TooLarge,
        "services.db",
    );
    let many: String = (0..450).map(|i| format!("      K{i}: v\n")).collect();
    let big_block = edit(
        COMPOSE,
        "    labels:\n      cortexv2: dev\n",
        &format!("    labels:\n      cortexv2: dev\n{many}"),
    );
    cannot(
        cmp(COMPOSE, &big_block),
        Side::Candidate,
        Refusal::TooLarge,
        "services.db",
    );
    let huge = format!("{COMPOSE}{}", "# padding\n".repeat(30_000));
    cannot(
        cmp(COMPOSE, &huge),
        Side::Candidate,
        Refusal::TooLarge,
        "compose.yml",
    );
}

// ---------------------------------------------------------------- the top-level entries the db service names

#[test]
fn cm_10_a_change_to_a_volume_network_or_secret_the_db_service_names_is_a_difference() {
    differs_at(
        cmp(
            COMPOSE,
            &edit(
                COMPOSE,
                "    name: cv2-s5-pg-data\n",
                "    name: cv2-s5-pg-data\n    driver: local\n",
            ),
        ),
        "volumes.cv2-s5-pg-data.driver",
    );
    differs_at(
        cmp(
            COMPOSE,
            &edit(
                COMPOSE,
                "    name: cv2-s5-pg-data\n",
                "    name: other-data\n",
            ),
        ),
        "volumes.cv2-s5-pg-data.name",
    );
    differs_at(
        cmp(
            COMPOSE,
            &edit(COMPOSE, "    name: cv2-s5-net\n", "    name: other-net\n"),
        ),
        "networks.cv2-s5-net.name",
    );
    differs_at(
        cmp(
            COMPOSE,
            &edit(
                COMPOSE,
                "  cv2-s5-owner-password:\n    external: true\n",
                "  cv2-s5-owner-password:\n    external: false\n",
            ),
        ),
        "secrets.cv2-s5-owner-password.external",
    );
    // an entry that is missing on one side
    differs_at(
        cmp(
            COMPOSE,
            &edit(COMPOSE, "  cv2-s5-pg-data:\n    name: cv2-s5-pg-data\n", ""),
        ),
        "volumes.cv2-s5-pg-data",
    );
}

#[test]
fn cm_11_entries_the_db_service_does_not_name_are_not_compared_and_a_bind_mount_is_not_a_volume() {
    // cv2-s5-state is used by another service only: any change, any shape, is no difference and no refusal
    assert_eq!(
        cmp(
            COMPOSE,
            &edit(
                COMPOSE,
                "    name: cv2-s5-state\n",
                "    name: elsewhere\n    driver_opts: {a: b}\n    x: &anchor y\n"
            )
        ),
        Ok(())
    );
    // ./initdb is a bind mount, not a named volume: there is nothing to look up
    assert_eq!(
        cmp(
            COMPOSE,
            &edit(
                COMPOSE,
                "\nvolumes:\n  cv2-s5-pg-data:",
                "\nvolumes:\n  cv2-s5-pg-data:"
            )
        ),
        Ok(()),
        "control"
    );
}

#[test]
fn cm_12_a_named_entry_in_a_shape_outside_the_subset_is_a_refusal() {
    for (from, to, class, path) in [
        (
            "    name: cv2-s5-pg-data\n",
            "    name: &x cv2-s5-pg-data\n",
            Refusal::Anchor,
            "volumes.cv2-s5-pg-data.name",
        ),
        (
            "    name: cv2-s5-net\n",
            "    name: {a: b}\n",
            Refusal::FlowMapping,
            "networks.cv2-s5-net.name",
        ),
        (
            "  cv2-s5-owner-password:\n    external: true\n",
            "  cv2-s5-owner-password:\n    external: |\n      true\n",
            Refusal::BlockScalar,
            "secrets.cv2-s5-owner-password.external",
        ),
    ] {
        let bad = edit(COMPOSE, from, to);
        cannot(cmp(COMPOSE, &bad), Side::Candidate, class, path);
        cannot(cmp(&bad, COMPOSE), Side::Current, class, path);
    }
}

// ---------------------------------------------------------------- mike, packet 4

const DB_NETWORKS: &str = "    networks:\n      - cv2-s5-net\n    healthcheck:";

/// The `db` service names no network, so Compose attaches it to the implicit `default` network.
fn without_db_networks(text: &str) -> String {
    edit(text, DB_NETWORKS, "    healthcheck:")
}

fn with_default_network(text: &str, body: &str) -> String {
    edit(
        text,
        "networks:\n  cv2-s5-net:\n",
        &format!("networks:\n  default:\n{body}  cv2-s5-net:\n"),
    )
}

#[test]
fn cm_14_a_db_service_that_names_no_network_is_on_the_implicit_default_network_and_that_is_compared()
 {
    let base = without_db_networks(COMPOSE);
    let one = with_default_network(&base, "    name: one\n");
    let two = with_default_network(&base, "    name: two\n");
    // controls: nothing about `default` anywhere is equal; the same `default` on both sides is equal
    assert_eq!(cmp(&base, &base), Ok(()), "control");
    assert_eq!(cmp(&one, &one), Ok(()), "control");
    // the settings of `default` changed, the db service text did not
    differs_at(cmp(&one, &two), "networks.default.name");
    differs_at(cmp(&two, &one), "networks.default.name");
    // `default` configured on one side only
    differs_at(cmp(&base, &one), "networks.default");
    differs_at(cmp(&one, &base), "networks.default");
    // a different driver, not only a different name
    let driver = with_default_network(&base, "    name: one\n    driver: bridge\n");
    differs_at(cmp(&one, &driver), "networks.default.driver");
    // the refusal rules apply to the entry like any other
    let bad = with_default_network(&base, "    name: &x one\n");
    cannot(
        cmp(&one, &bad),
        Side::Candidate,
        Refusal::Anchor,
        "networks.default.name",
    );
    // `networks:` with nothing under it names no network either
    let blank = |t: &str| edit(t, DB_NETWORKS, "    networks:\n    healthcheck:");
    differs_at(
        cmp(
            &blank(&with_default_network(COMPOSE, "    name: one\n")),
            &blank(&with_default_network(COMPOSE, "    name: two\n")),
        ),
        "networks.default.name",
    );
    // an empty `default:` entry and none at all are different files, so they are a difference
    // (the comparator does not guess what Compose makes of it)
    let empty = edit(
        &base,
        "networks:\n  cv2-s5-net:\n",
        "networks:\n  default:\n  cv2-s5-net:\n",
    );
    differs_at(cmp(&base, &empty), "networks.default");
}

#[test]
fn cm_14b_when_the_db_service_names_its_networks_the_default_one_is_not_its_business() {
    // positive controls: it is on cv2-s5-net, so a change to `default` alone is no difference ...
    let one = with_default_network(COMPOSE, "    name: one\n");
    let two = with_default_network(COMPOSE, "    name: two\n");
    assert_eq!(cmp(&one, &two), Ok(()));
    assert_eq!(cmp(COMPOSE, &one), Ok(()));
    // ... and the network it does name is still compared
    differs_at(
        cmp(
            &one,
            &edit(&two, "    name: cv2-s5-net\n", "    name: other-net\n"),
        ),
        "networks.cv2-s5-net.name",
    );
    // naming `default` itself is the explicit form of the same thing
    let named = |t: &str| {
        edit(
            t,
            DB_NETWORKS,
            "    networks:\n      - default\n    healthcheck:",
        )
    };
    differs_at(cmp(&named(&one), &named(&two)), "networks.default.name");
}

#[test]
fn cm_14c_a_mapping_of_networks_names_them_by_key() {
    let map_form = |t: &str| {
        edit(
            t,
            DB_NETWORKS,
            "    networks:\n      default:\n        ipv4_address: 10.0.0.2\n    healthcheck:",
        )
    };
    let one = map_form(&with_default_network(COMPOSE, "    name: one\n"));
    let two = map_form(&with_default_network(COMPOSE, "    name: two\n"));
    differs_at(cmp(&one, &two), "networks.default.name");
}

#[test]
fn cm_15_an_env_file_in_the_db_service_is_refused_on_both_sides_and_only_there() {
    // its contents are not part of the file, so equal text proves nothing
    for (what, to) in [
        ("a scalar", "    env_file: ./db.env\n    init: true\n"),
        (
            "a list",
            "    env_file:\n      - ./db.env\n    init: true\n",
        ),
        (
            "a list of mappings",
            "    env_file:\n      - path: ./db.env\n        required: false\n    init: true\n",
        ),
    ] {
        let bad = edit(COMPOSE, "    init: true\n", to);
        let (a, b) = (cmp(COMPOSE, &bad), cmp(&bad, COMPOSE));
        match (&a, &b) {
            (
                Err(ComposeError::Cannot {
                    side: Side::Candidate,
                    ..
                }),
                Err(ComposeError::Cannot {
                    side: Side::Current,
                    ..
                }),
            ) => {}
            other => panic!("{what}: expected a refusal on the side that holds it: {other:?}"),
        }
        assert!(
            matches!(
                cmp(&bad, &bad),
                Err(ComposeError::Cannot {
                    side: Side::Current,
                    ..
                })
            ),
            "{what}: equal text with an env_file is still refused"
        );
        // the name of the key is the path: no file name, no value
        if what == "a scalar" {
            cannot(a, Side::Candidate, Refusal::EnvFile, "services.db.env_file");
            cannot(b, Side::Current, Refusal::EnvFile, "services.db.env_file");
        }
    }
    // another service may use an env_file (it is not compared), and so may a key that only ends in it
    let other = edit(
        COMPOSE,
        "    command: [\"migrate\"]\n",
        "    command: [\"migrate\"]\n    env_file: ./migrate.env\n",
    );
    assert_eq!(cmp(COMPOSE, &other), Ok(()), "control: not the db service");
    let near = edit(
        COMPOSE,
        "    init: true\n",
        "    init: true\n    x_env_file: a\n",
    );
    assert_eq!(
        cmp(&near, &near),
        Ok(()),
        "control: a different key is not env_file"
    );
}

// ---------------------------------------------------------------- the real S5 file

/// `tests/fixtures/s5-compose.yml` is a byte copy of `deploy/compose.yml` on branch `wave1/s5` at e010d1a (sha256 5f04f8b36fc70f50bdd0de90d9c59a5d57e701a6e08d3a20a3de637dcc148f90).
const REAL: &str = include_str!("fixtures/s5-compose.yml");

#[test]
fn cm_13_the_real_s5_compose_file_is_read_and_its_release_form_is_equal_to_it() {
    assert_eq!(
        cmp(REAL, REAL),
        Ok(()),
        "the real file is inside the accepted shapes"
    );
    // the release layout has `image: ${CORTEX_DB_IMAGE}` where the dev file has the pinned reference
    let pinned = REAL
        .lines()
        .find(|l| l.trim_start().starts_with("image: docker.io/pgvector"))
        .expect("the pinned db image line");
    let image = pinned
        .trim_start()
        .trim_start_matches("image: ")
        .to_string();
    let release = REAL.replacen(pinned, "    image: ${CORTEX_DB_IMAGE}", 1);
    let e = parse_stack_env(&format!("CORTEX_VERSION=0.2.1-rc.1\nCORTEX_SERVER_IMAGE=localhost/cortex-server@{D}\nCORTEX_DB_IMAGE={image}\n")).unwrap();
    assert_eq!(
        compare_db_service((REAL, &e), (&release, &e)),
        Ok(()),
        "the dev file and its release form describe the same db service"
    );
    // and a different pinned image is named by path
    let other = parse_stack_env(&format!("CORTEX_VERSION=0.2.1-rc.1\nCORTEX_SERVER_IMAGE=localhost/cortex-server@{D}\nCORTEX_DB_IMAGE=docker.io/pgvector/pgvector:0.8.3-pg18@{D}\n")).unwrap();
    differs_at(
        compare_db_service((REAL, &e), (&release, &other)),
        "services.db.image",
    );
}
