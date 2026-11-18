"""S5 static checker (plan S5-PLAN.md section 4). Text only: it reads files and runs `sh -n` / `bash -n`; it starts no container.

    python3 -I scripts/s5_check.py <deploy-dir>            all rules over a deploy directory
    python3 -I scripts/s5_check.py --never-list <dir>      only S5-15 over every shell script under <dir>

Exit 0: every rule passes. Exit 1: at least one finding, each printed as `FAIL S5-nn <file>: <why>`.
Exit 2: the checker itself could not run, which is never a pass. A form the checker does not understand is a finding (fail closed).

Rules
  S5-01 a published port binds 127.0.0.1 only (no IPv6, no all-interface publish), in the short form
  S5-02 no host port is one of v1's (8501, 5499, 5500, 8765, 8766), and every host port resolves to a number
  S5-03 every name we create (project, container, network, volume, secret, built image) starts cv2-s5-
  S5-04 no secret value in `environment:` or in the config example (only `*_FILE` / `*_file` references)
  S5-05 the Containerfile copies no config or secret file and has no secret ENV or ARG
  S5-06 every image is digest-pinned and none is on `latest`; our own image is never pulled (pull_policy: never). In the RELEASE layout (no
        service builds) our image is exactly ${CORTEX_SERVER_IMAGE:?} on the server and the one-shot: the digest pin is then stack.env's, and the
        engine refuses a value that is not name@sha256:<64 hex> (stack_env.rs, verify_release_dir); this checker cannot see it
  S5-07 services of ours (built here, or running our image) run non-root, read-only, cap_drop ALL, no-new-privileges
  S5-08 every service has a real health check, except a one-shot of ours (see S5-16)
  S5-09 every service carries the label cortexv2=dev
  S5-10 every shell script has a shebang, `set -e` and `set -u`, and passes the shell's syntax check
        (except a source that a line of install.sh includes with the exact trailing marker `# @include`: it has no shebang and must hold only function definitions).
        S5-10 is a LINT with a dynamic oracle: it sources an included file in a fixed, confined environment (bash --posix -r, xtrace, sentinel, namespace) and refuses it if anything
        runs, prints or exits there; it also checks syntax (sh -n), the closed ending and the control-character rule. It is NOT a proof that the file consists of function definitions
        only. The rule fails when bash is missing. Restricted mode is not a security boundary (S5-15 is the static backstop); the oracle is for a file that is wrong, not one that is hostile
  S5-11 every secret is declared external (a Podman secret) and every use is declared
  S5-12 no privileged container, host namespace, extra capability, device or switched-off seccomp or label
  S5-13 the server config is bind-mounted read-only at /etc/cortex/cortex.toml
  S5-14 the required files exist and parse
  S5-18 a postgres service's EFFECTIVE initdb auth is local=peer and host=scram-sha-256 (the last value wins; every auth token must be peer or scram-sha-256; every initdb argument must be on a closed allow-list),
        POSTGRES_HOST_AUTH_METHOD is absent or scram-sha-256, and the server is never told to read another hba_file
  S5-17 every long-running service restarts by policy (unless-stopped or always): after a VM stop and start the stack comes back
  S5-16 the long-running server never mounts the owner or superuser secret: a one-shot migrate does (it carries the label
        cortexv2.role: oneshot, restart "no" and the command ["migrate"], all three; restart "no" is compose's default and proves
        nothing), and every long-running service of ours waits for it with condition service_completed_successfully
  S5-21 the user units in systemd/ (piece 12, bob's files; install.sh fills two placeholders with sed): every @TOKEN@ is @CORTEX_HOME@ or @DB_CONTAINER@, every
        Exec* line and every Documentation entry (after an optional file:) starts with @CORTEX_HOME@/, and there is no % and no $ anywhere in a unit (a systemd
        specifier or a variable would change what the fill-in wrote; their absence is why the --home character class is enough)
  S5-22 db-roles.sh carries a SERVER-SIDE budget (cox 19:41Z on mike's R6): one `budget=<n>` line with n from 1 to 30 (shorter than the engine's run_roles deadline, which
        is at least 45 s), and transaction_timeout is set nowhere but PGOPTIONS on the podman exec. W1-55 (mike's M-4: transaction_timeout bounds how long the transaction RUNS,
        not when it STARTS, and a script that read its own start could be scheduled late): the CALLER passes its absolute deadline E (`deadline=$2`; the real adapter: launch +
        timeout, install.sh: now + 60); the script reads NO clock of its own (`date` is refused); the relation is written once: `idle=5`, `latency=2`, `slack=2` pinned exactly,
        `lead=$((idle + latency + slack))` = 9, `guard_by=$((deadline - lead))`, all before the exec; the session gets exactly `-c transaction_timeout=${budget}s -c
        idle_in_transaction_session_timeout=${idle}s -c db_roles.guard_by=${guard_by}`; and the roles SQL ENDS with a fixed guard (clock_timestamp(), not now(); current_setting
        without missing_ok, so a missing setting fails closed) that raises when the database clock is past guard_by. The latest completed commit is therefore E - slack.
        Hale's guards on the guards (5n): the exec line is EXACTLY the one line (psql -v ON_ERROR_STOP=1 -1 -U postgres -d cortex -q: one transaction on cortex), no code line ends or
        splits the transaction (COMMIT, END;, ROLLBACK, ABORT, SAVEPOINT, START TRANSACTION), ON_ERROR_STOP appears once, and guard_by is named only on the exec line and the guard line.
  S5-23 the release feature set (cox, release-layout gap): deploy/server-features is the ONE source of the cortex-server features a release is built with. It is one line,
        the optional-piece features of crates/cortex-server/Cargo.toml (every key of [features] but test-faults, found at <tree>/../crates/cortex-server/Cargo.toml) as
        lower-case names separated by single commas, each once, ended by one LF; it never names test-faults, and the manifest declares no `default` feature (a default would
        make the set invisible). The Containerfile consumes it and nothing else decides: it COPYs deploy/server-features into the build stage before the cargo build, and every
        `cargo build` passes exactly one `--features "$(cat <the copied path>)"` (no --all-features, which builds test-faults in, no literal list, no second --features or any
        word starting -F, joined or not); and Containerfile.dockerignore (the build context drops deploy/) lets that one file through (the last matching pattern decides, as in Docker).
        WHAT IT READS (a lint of TEXT, hale's S523-9; the limits are in the record, not hidden): each RUN line is read as shell words and cut at && || ; | & ( ), and every
        command whose first word is cargo (after VAR=value, exec, env, time, command, nice, nohup) is judged ON ITS OWN: cargo build and cargo b must carry the set, any other
        subcommand that is not harmless (fetch, vendor, clean, metadata, tree, update, generate-lockfile, locate-project, pkgid, verify-project, version, help, fmt) is refused.
        FAIL CLOSED (hale, the follow-up): the word cargo anywhere but as the first word of a command the reader understood (a wrapper such as timeout, sudo or setsid, if/then,
        braces, a negation, a for loop, a backtick, the exec form of RUN, an argument such as an echo text) is REFUSED as 'a position this rule cannot judge', never skipped; the
        flags of the RUN instruction itself (BuildKit --mount=..., --network=...) are not part of the command but are VALIDATED, not discarded (the mount policy below), and a # starts a comment only at the start of a word.
        THE RUN MOUNT POLICY (sam S523-SAM01; an ALLOWLIST, so a mount that can put other bytes under the path the judged build reads is refused): every RUN option is read before the command
        is judged. ACCEPTED: --mount=type=cache whose keys are only type, target (or dst, destination), id, sharing, mode, uid, gid, whose type is written out, and whose target is a
        literal absolute path that does NOT OVERLAP (equal, above or below, component by component, after . and .. are resolved) any COPY or ADD destination of the same stage (relative
        destinations are resolved against the WORKDIR in force, which is tracked here), the build directory (the WORKDIR in force) or a default PATH directory; --network=none|default;
        --security=sandbox. REFUSED, each for its reason: --network=host and --security=insecure (entitlements that ask the builder for the host network or extra privileges and so weaken the build's isolation; the real Containerfile and the good fixture use no RUN option at all, so the allowlist is kept tight), type=bind (a file or directory bind mount can cover the COPY'd feature path or its parent with other bytes while the
        text of the build line stays the same: the finding itself), a mount with NO type (the default type is bind), type=tmpfs, secret and ssh (not an accepted form: nothing else is),
        the keys from, source and src (they seed a cache from another stage or the build context: bytes this rule does not see), every other key and any duplicate key (not in the allowlist),
        a bare word such as ro, a missing, relative or variable target (it cannot be compared), a target that overlaps (it can hide what was copied in), a target on a default PATH directory
        (a cache there could replace cat or cargo; PATH itself is not tracked, so the list is a default), any other RUN option, a value with a quote (it cannot be read), and ANY mount at
        all (sam S523-SAM02: fail closed on context this rule does not see) in a stage that is not built FROM an external image (a FROM that names an earlier stage or is a variable inherits
        that stage's files and WORKDIR), before an explicit absolute literal WORKDIR is set EARLIER in the same stage (an external base image may set one of its own, so the build directory
        is not known), or in a stage whose WORKDIR in force or a COPY or ADD destination is not a literal path (the overlap cannot be judged). The policy covers every RUN of the Containerfile.
        WHAT IT CANNOT SEE (S523-5 to S523-8): cargo started through sh -c (its text is one word, so it is refused only if the word cargo is in it), xargs or a script; abbreviated long flags; later FROM stages, and WORKDIR for the feature file's own COPY and cat (the mount policy alone tracks WORKDIR; the COPY and the
        cat are matched by path text, so the real build guard test "$(wc -l < server-features)" -eq 1 is not REQUIRED by the rule); the ignore file is read with fnmatch, not
        Docker's own matcher (./deploy and **/deploy are not recognised as excluding, a pattern such as *s is a false alarm); only the keys of [features] are read (an optional
        dependency without dep: is an implicit feature the rule never sees; a feature name with _ or a capital can be neither listed nor ignored); the fixture manifest is a COPY of
        the real one and nothing compares them; the release layout cells place the manifest beside the layout, so they cannot see whether release.sh does (X-1 stays open).
  S5-15 cox's NEVER list over every shell script: pkill and killall, kill -9, group signals, podman rm -a / --all, prune
        and reset, compose down -v, git push, v1's names, and printing a token or secret value. A line may be allowed with
        `# s5-never: ok <reason>`; each such line is listed as a NOTE. A lint, not a proof: it reads lines, not intent.
"""
import fnmatch
import json
import os
import posixpath
import re
import select
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import tomllib

try:
    import yaml
except ImportError:
    print("s5-check: PyYAML is required (python3 -I -c 'import yaml')", file=sys.stderr)
    sys.exit(2)

V1_PORTS = {8501, 5499, 5500, 8765, 8766}
LOOPBACK = {"127.0.0.1"}   # IPv4 loopback only: no IPv6 publish, no all-interface publish (vera)
REQUIRED = ["compose.yml", "Containerfile", "install.sh", "initdb/00-cortex-roles.sh", "cortex.toml.example", "server-features"]
CONFIG_MOUNT = "/etc/cortex/cortex.toml"
DUMPS_MOUNT = "/var/lib/cortex-dumps"
SECRET_WORDS = re.compile(r"(password|passwd|secret|token|api_?key|private_?key|credential)", re.I)
CONFIG_PATTERNS = [
    re.compile(p, re.I) for p in (
        r"^cortex[^/]*\.toml(\..*)?$", r"^config[^/]*\.(toml|ya?ml|json|ini|conf)$", r"(^|\.)env$", r"^\.env",
        r"\.(token|key|pem|p12|pfx)$", r"secret", r"password", r"credential",
    )
]
DIGEST = re.compile(r"@sha256:[0-9a-f]{64}$")

findings = []


def fail(rule, where, msg):
    findings.append((rule, where, msg))


def resolve_vars(text):
    """Replace ${VAR:-default} and ${VAR-default} by the default. Returns (text, unresolved); unresolved is True
    when a ${VAR} has no default, because then the value depends on the caller's environment."""
    unresolved = False

    def sub(m):
        nonlocal unresolved
        if m.group(2) is None:
            unresolved = True
            return ""
        return m.group(2)

    return re.sub(r"\$\{([A-Za-z_][A-Za-z0-9_]*)(?::?-([^}]*))?\}", sub, text), unresolved


def read(tree, rel):
    with open(os.path.join(tree, rel), encoding="utf-8") as f:
        return f.read()


# ---- compose ---------------------------------------------------------------------------------------------------------

def name_ok(n):
    return isinstance(n, str) and (n == "cv2-s5" or n.startswith("cv2-s5-"))


def check_ports(sname, ports):
    where = f"compose.yml service {sname}"
    for p in ports or []:
        if not isinstance(p, str):
            fail("S5-01", where, 'long port syntax is not accepted; write "127.0.0.1:<host>:<container>"')
            continue
        text, unresolved = resolve_vars(p)
        text = re.sub(r"/(tcp|udp)$", "", text)
        parts = text.split(":")
        if len(parts) < 3:
            fail("S5-01", where, f"port {p!r} publishes without a loopback address")
        else:
            ip = ":".join(parts[:-2])
            if ip not in LOOPBACK:
                fail("S5-01", where, f"port {p!r} binds {ip!r}; only 127.0.0.1 is allowed")
        if len(parts) >= 2:
            host = parts[-2]
            if unresolved:
                fail("S5-02", where, f"port {p!r} has a host port with no default; v1's ports cannot be ruled out")
                continue
            m = re.fullmatch(r"(\d+)(?:-(\d+))?", host)
            if not m:
                fail("S5-02", where, f"port {p!r} has a host port that is not a number")
                continue
            lo, hi = int(m.group(1)), int(m.group(2) or m.group(1))
            clash = sorted(v for v in V1_PORTS if lo <= v <= hi)
            if clash:
                fail("S5-02", where, f"host port {host} is v1's ({clash})")


def check_names(c):
    if not name_ok(c.get("name")):
        fail("S5-03", "compose.yml", f"project name {c.get('name')!r} must be cv2-s5 or start with cv2-s5-")
    for kind in ("networks", "volumes", "secrets"):
        for key, spec in (c.get(kind) or {}).items():
            if not name_ok(key):
                fail("S5-03", "compose.yml", f"{kind[:-1]} {key!r} must start with cv2-s5-")
            nm = (spec or {}).get("name") if isinstance(spec, dict) else None
            if nm is not None and not name_ok(nm):
                fail("S5-03", "compose.yml", f"{kind[:-1]} name {nm!r} must start with cv2-s5-")
    for sname, svc in (c.get("services") or {}).items():
        cn = svc.get("container_name")
        if cn is not None and not name_ok(cn):
            fail("S5-03", f"compose.yml service {sname}", f"container_name {cn!r} must start with cv2-s5-")
        if "build" in svc:
            img = svc.get("image", "")
            if not str(img).startswith("localhost/cv2-s5-"):
                fail("S5-03", f"compose.yml service {sname}", f"built image {img!r} must start with localhost/cv2-s5-")


def env_items(env):
    if isinstance(env, dict):
        return list(env.items())
    out = []
    for e in env or []:
        k, _, v = str(e).partition("=")
        out.append((k, v))
    return out


# The release layout (5h): release.sh drops `build:` and names our image with exactly this variable; stack.env supplies a digest-pinned value.
# Optional message after `:?`, and nothing that could close or nest the expansion. Anything else is another image and gets no say here.
RELEASE_IMAGE = re.compile(r"^\$\{CORTEX_SERVER_IMAGE:\?[^}$]*\}\Z")   # \Z, not $: a trailing newline must not pass (hale 5h N-1)


def is_release_image(svc):
    return isinstance(svc.get("image"), str) and RELEASE_IMAGE.match(svc["image"]) is not None


def is_ours(svc):
    """A service of ours is one we build here, or one that runs our image (so a one-shot migrate is held to the same rules). In the release
    layout our image is the variable above: classifying it as ours only ADDS rules (S5-07, S5-13, the one-shot exemptions), never removes one."""
    return "build" in svc or str(svc.get("image", "")).startswith("localhost/cv2-s5-") or is_release_image(svc)


ONESHOT_COMMANDS = [["migrate"]]


def role(svc):
    """The value of the label cortexv2.role, or None."""
    labels = svc.get("labels")
    if isinstance(labels, dict):
        v = labels.get("cortexv2.role")
        return None if v is None else str(v)
    for x in labels or []:
        if str(x).startswith("cortexv2.role="):
            return str(x).split("=", 1)[1]
    return None


def is_oneshot(svc):
    """A one-shot of ours is a ROLE: the label, an explicit restart "no" and a known one-shot command, all three.
    restart "no" alone is compose's default, so it cannot be what makes a service a one-shot (nix N11)."""
    return (is_ours(svc) and role(svc) == "oneshot" and svc.get("restart") in ("no", False)
            and svc.get("command") in ONESHOT_COMMANDS)


def is_shipped_server(svc):
    """The server we build (dev: `build:`) or ship (release: the image variable on a service that is not a one-shot)."""
    return "build" in svc or (is_release_image(svc) and not is_oneshot(svc))


def initdb_auth(args):
    """Walk the initdb arguments in order the way initdb does (the LAST value wins) and return (local, host, weak), where weak lists every
    auth token whose value is neither peer nor scram-sha-256, even one that a later token overrides (nix N25)."""
    toks = str(args or "").split()
    local = host = None
    weak = []
    i = 0
    while i < len(toks):
        t = toks[i]
        opt = val = None
        if t in ("-A", "--auth", "--auth-local", "--auth-host") and i + 1 < len(toks):
            opt, val = t, toks[i + 1]
            i += 1
        elif t.startswith("-A") and len(t) > 2 and not t.startswith("--"):
            opt, val = "-A", t[2:]
        else:
            for name in ("--auth-local", "--auth-host", "--auth"):
                if t.startswith(name + "="):
                    opt, val = name, t.split("=", 1)[1]
                    break
        if opt is not None:
            if val not in ("peer", "scram-sha-256"):
                weak.append(f"{opt} {val}")
            if opt in ("-A", "--auth"):
                local = host = val
            elif opt == "--auth-local":
                local = val
            else:
                host = val
        i += 1
    return local, host, weak


# nix N29: do not parse the initdb options, ALLOW-LIST them. getopt_long accepts an unambiguous abbreviation of a long option, short options combine
# (-kA), and the postgres entrypoint evals the string, so a shell removes quotes and backslashes and runs $(...) and backticks. A parser cannot be
# complete against that; a closed list of exact tokens and a closed set of characters can.
INITDB_FIXED = ("--data-checksums", "--locale-provider=builtin", "--auth-local=peer", "--auth-host=scram-sha-256")
INITDB_VALUE = re.compile(r"^[A-Za-z0-9_][A-Za-z0-9._@-]*$")


def initdb_unknown(args):
    """Return the findings for POSTGRES_INITDB_ARGS that are not on the closed list: every character must be a letter, digit or one of . _ @ = - and
    a space or a tab (NOT a newline or a carriage return: the entrypoint evals the string and a newline ends a command; nix N33);
    every token must be one of INITDB_FIXED, -E with an encoding name after it, --locale=<name> or --builtin-locale=<name>."""
    s = str(args or "")
    bad = []
    stray = sorted({c for c in s if not (c.isascii() and (c.isalnum() or c in "._@=- \t"))})
    if stray:
        bad.append("initdb arguments contain characters outside the closed set (letters, digits and . _ @ = - and spaces or tabs): "
                   + " ".join(repr(c) for c in stray) + " (the entrypoint evals this string)")
    toks = s.split()
    i = 0
    while i < len(toks):
        t = toks[i]
        if t == "-E":
            if i + 1 < len(toks) and INITDB_VALUE.match(toks[i + 1]):
                i += 2
                continue
            bad.append("initdb argument '-E' needs an encoding name after it")
        elif t in INITDB_FIXED:
            pass
        elif t.startswith("--locale=") and INITDB_VALUE.match(t[len("--locale="):]):
            pass
        elif t.startswith("--builtin-locale=") and INITDB_VALUE.match(t[len("--builtin-locale="):]):
            pass
        else:
            bad.append(f"initdb argument {t!r} is not on the allow-list (-E <name>, --locale-provider=builtin, --locale=<name>, --builtin-locale=<name>, --data-checksums, --auth-local=peer, --auth-host=scram-sha-256)")
        i += 1
    return bad


def check_service(sname, svc):
    where = f"compose.yml service {sname}"
    ours = is_ours(svc)
    check_ports(sname, svc.get("ports"))

    for k, v in env_items(svc.get("environment")):
        if SECRET_WORDS.search(k) and not k.upper().endswith("_FILE") and v not in (None, ""):
            fail("S5-04", where, f"environment {k} holds a value; pass a *_FILE path to a Podman secret instead")

    # S5-18 a postgres service: no trust rule, ever (vera). initdb writes the pg_hba rules, so its flags decide them.
    envd = dict(env_items(svc.get("environment")))
    if "POSTGRES_PASSWORD_FILE" in envd or "POSTGRES_DB" in envd:
        local, host, weak = initdb_auth(envd.get("POSTGRES_INITDB_ARGS"))
        if local != "peer" or host != "scram-sha-256":
            fail("S5-18", where, f"the EFFECTIVE initdb auth must be local=peer and host=scram-sha-256 (initdb takes the last value); it is local={local} host={host}")
        for w in weak:
            fail("S5-18", where, f"initdb auth token {w!r} is neither peer nor scram-sha-256 (even if a later token overrides it)")
        for u in initdb_unknown(envd.get("POSTGRES_INITDB_ARGS")):
            fail("S5-18", where, u)
        if envd.get("POSTGRES_HOST_AUTH_METHOD") not in (None, "scram-sha-256"):
            fail("S5-18", where, "POSTGRES_HOST_AUTH_METHOD may be absent or scram-sha-256, nothing else")
        cmd = svc.get("command")
        if "hba_file" in (" ".join(str(c) for c in cmd) if isinstance(cmd, list) else str(cmd or "")):
            fail("S5-18", where, "the server must not be told to read another hba_file")

    # S5-06 images
    img = str(svc.get("image", ""))
    if ours and is_release_image(svc):
        # the release image: never together with a build (the dev layout names its tag), never pulled. The pin is stack.env's, not visible here.
        if "build" in svc:
            fail("S5-06", where, f"image {img!r} (the release image) is not allowed together with build: (a build names its own tag)")
        if svc.get("pull_policy") != "never":
            fail("S5-06", where, "our own image must set pull_policy: never (it is loaded from the release archive; a pull would look for a registry)")
    elif ours:
        tag = img.rsplit(":", 1)[1] if ":" in img.rsplit("/", 1)[-1] else ""
        if not tag or tag == "latest":
            fail("S5-06", where, f"built image {img!r} needs an explicit tag other than latest")
        if svc.get("pull_policy") != "never":
            fail("S5-06", where, "our own image must set pull_policy: never (it is built here; a pull would look for a registry)")
    else:
        if not DIGEST.search(img):
            fail("S5-06", where, f"image {img!r} is not pinned by @sha256 digest")
        if re.search(r":latest(@|$)", img):
            fail("S5-06", where, f"image {img!r} is on latest")

    # S5-07 hardening of what we build
    if ours:
        if svc.get("read_only") is not True:
            fail("S5-07", where, "read_only must be true")
        if "ALL" not in [str(x).upper() for x in (svc.get("cap_drop") or [])]:
            fail("S5-07", where, "cap_drop must include ALL")
        sec = [str(x).replace("=", ":").lower() for x in (svc.get("security_opt") or [])]
        if "no-new-privileges:true" not in sec:
            fail("S5-07", where, "security_opt must include no-new-privileges:true")
        user = str(svc.get("user", "")).split(":")[0]
        if user in ("", "0", "root"):
            fail("S5-07", where, f"user {svc.get('user')!r} must be set and not root")

    # S5-08 health
    hc = svc.get("healthcheck")
    test = hc.get("test") if isinstance(hc, dict) else None
    if is_oneshot(svc):
        pass  # it runs once and exits; the services that need it wait for it (S5-16)
    elif (not isinstance(hc, dict) or hc.get("disable") or not test
            or (isinstance(test, list) and str(test[0]).upper() == "NONE")):
        fail("S5-08", where, "needs a healthcheck with a real test")

    # S5-17 restart policy for everything that is not a one-shot
    if not is_oneshot(svc) and svc.get("restart") not in ("unless-stopped", "always"):
        fail("S5-17", where, 'needs restart: unless-stopped (or always); a service without a policy stays down after a VM stop and start')

    # S5-09 label
    labels = svc.get("labels")
    if isinstance(labels, dict):
        has = str(labels.get("cortexv2")) == "dev"
    else:
        has = "cortexv2=dev" in [str(x) for x in (labels or [])]
    if not has:
        fail("S5-09", where, "label cortexv2=dev is missing")

    # S5-12 isolation
    if svc.get("privileged"):
        fail("S5-12", where, "privileged is not allowed")
    for key in ("network_mode", "pid", "ipc", "userns_mode"):
        if str(svc.get(key, "")).lower().startswith("host"):
            fail("S5-12", where, f"{key}: host is not allowed")
    if svc.get("cap_add"):
        fail("S5-12", where, f"cap_add {svc.get('cap_add')!r} is not allowed")
    if svc.get("devices"):
        fail("S5-12", where, "devices are not allowed")
    for o in svc.get("security_opt") or []:
        low = str(o).lower()
        if "unconfined" in low or low.replace("=", ":") == "label:disable":
            fail("S5-12", where, f"security_opt {o!r} switches protection off")

    check_dumps_and_sha(sname, svc)

    # S5-13 config mount
    if ours:
        ok = False
        for v in svc.get("volumes") or []:
            if isinstance(v, str):
                parts = re.sub(r"\$\{[^}]*\}", "VAR", v).split(":")
                if len(parts) >= 2 and parts[1] == CONFIG_MOUNT:
                    ok = len(parts) >= 3 and "ro" in parts[2].split(",")
            elif isinstance(v, dict) and v.get("target") == CONFIG_MOUNT:
                ok = v.get("type") == "bind" and v.get("read_only") is True
        if not ok:
            fail("S5-13", where, f"{CONFIG_MOUNT} must be bind-mounted read-only (:ro)")


def mounts_of(svc):
    """The volumes of a service as (source, target, read_only, raw, options): read_only is True only when `ro` is stated and `rw` is not;
    options is the set of mount options (short form: as written; long form: ro when read_only is true, z or Z from bind.selinux)."""
    out = []
    for v in svc.get("volumes") or []:
        if isinstance(v, dict):
            bind = v.get("bind") if isinstance(v.get("bind"), dict) else {}
            options = ({"ro"} if v.get("read_only") is True else set()) | ({str(bind["selinux"])} if bind.get("selinux") else set())
            out.append((str(v.get("source", "")), str(v.get("target", "")), v.get("read_only") is True, str(v), options))
        elif isinstance(v, str):
            m = re.match(r"^(\$\{[^}]*\}|[^:]*):([^:]*)(?::(.*))?$", v)
            if m:
                options = set((m.group(3) or "").split(","))
                out.append((m.group(1), m.group(2), "ro" in options and "rw" not in options, v, options))
    return out


DUMP_STATUS_SOURCE = "${CORTEX_DUMP_STATUS:?set by install.sh}"
GIT_SHA_ARG = "${CORTEX_GIT_SHA:-unknown}"
# every host path a service may bind-mount, exactly; anything else must be a named volume (a parent of the install home would reach the private dump tree)
BIND_SOURCES = {"${CORTEX_CONFIG:?set by install.sh}", DUMP_STATUS_SOURCE, "./initdb"}


def check_dumps_and_sha(sname, svc):
    """S5-19 (A100, vera option C) the server reads a status-only directory and never the private dump tree. A mount IS the status mount when its
    TARGET is /var/lib/cortex-dumps (normalised) or its SOURCE names CORTEX_DUMP_STATUS, wherever it is mounted. It must be in the server we
    build, once, at that target, from exactly ${CORTEX_DUMP_STATUS:?set by install.sh}, with the options `ro` or `ro,z` and nothing else (no rw, no
    private relabel Z). No service mounts the private tree: a source that names CORTEX_DUMPS or is a path ending in /dumps is refused.
    The directory is <home>/dump-status, 0755, and holds LAST_GOOD alone (0644): install.sh makes the directory, cortex-dump publishes the file.
    S5-20 the server's build passes the git sha EXACTLY as ${CORTEX_GIT_SHA:-unknown} (`cortex-server --version`)."""
    where = f"compose.yml service {sname}"
    mounts = mounts_of(svc)
    for source, _t, _r, raw, _o in mounts:
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*", source or "") and source not in BIND_SOURCES:
            fail("S5-19", where, f"a bind-mount source is one of {sorted(BIND_SOURCES)} or a named volume, not {raw!r} (a parent of the private dump tree reaches it)")
    if svc.get("volumes_from"):
        fail("S5-19", where, "volumes_from inherits another service's mounts; list the mounts in the service itself")
    for source, _target, _ro, raw, _opts in mounts:
        if "CORTEX_DUMPS" in source or re.search(r"(^|/)dumps/*$", source):
            fail("S5-19", where, f"the private dump tree is never mounted (a status-only directory is), not {raw!r}")
    status = [m for m in mounts if posixpath.normpath(m[1] or ".") == DUMPS_MOUNT or "CORTEX_DUMP_STATUS" in m[0]]
    built = is_shipped_server(svc)
    if built and not status:
        fail("S5-19", where, f"{DUMPS_MOUNT} must be bind-mounted read-only from CORTEX_DUMP_STATUS")
    if status and not built:
        fail("S5-19", where, f"only the server we build or ship may mount the status directory (CORTEX_DUMP_STATUS or {DUMPS_MOUNT})")
    if len(status) > 1:
        fail("S5-19", where, "the status directory is mounted exactly once")
    for source, target, read_only, raw, options in status:
        if posixpath.normpath(target or ".") != DUMPS_MOUNT:
            fail("S5-19", where, f"the status directory is mounted at {DUMPS_MOUNT} only, not {target!r}")
        if source != DUMP_STATUS_SOURCE:
            fail("S5-19", where, f"the source of {DUMPS_MOUNT} must be exactly {DUMP_STATUS_SOURCE}, not {source!r}")
        if not read_only:
            fail("S5-19", where, f"{DUMPS_MOUNT} must be mounted read-only (:ro, and not rw), not {raw!r}")
        elif options not in ({"ro"}, {"ro", "z"}):
            fail("S5-19", where, f"the options of {DUMPS_MOUNT} must be ro or ro,z (no rw, no private relabel Z, nothing else), not {raw!r}")
    if "build" in svc:   # a release has no build: its sha is baked by release.sh's own build
        build = svc.get("build")
        args = build.get("args") if isinstance(build, dict) else None
        value = None
        if isinstance(args, dict):
            value = args.get("CORTEX_GIT_SHA")
        elif isinstance(args, list):
            for a in args:
                if str(a).startswith("CORTEX_GIT_SHA="):
                    value = str(a).split("=", 1)[1]
        if value is None or str(value) != GIT_SHA_ARG:
            fail("S5-20", where, f"the build must pass CORTEX_GIT_SHA exactly as {GIT_SHA_ARG} (a baked default would be a false sha)")


def check_git_sha_build(text):
    """S5-20 the Containerfile: before the first `cargo build` there is `ARG CORTEX_GIT_SHA=unknown` and `ENV CORTEX_GIT_SHA=${CORTEX_GIT_SHA}`."""
    arg_seen = env_seen = False
    for op, arg in instructions(text):
        if op == "ARG" and arg.split("=", 1)[0].strip() == "CORTEX_GIT_SHA":
            if arg.strip() != "CORTEX_GIT_SHA=unknown":
                fail("S5-20", "Containerfile", f"ARG {arg.strip()!r}: the default must be unknown (a sha is passed at build time, never baked in)")
            else:
                arg_seen = True
        elif op == "ENV" and arg.split("=", 1)[0].strip() == "CORTEX_GIT_SHA":
            if arg.strip() not in ("CORTEX_GIT_SHA=${CORTEX_GIT_SHA}", "CORTEX_GIT_SHA=$CORTEX_GIT_SHA"):
                fail("S5-20", "Containerfile", f"ENV {arg.strip()!r}: it must be the value of the ARG")
            elif arg_seen:
                env_seen = True
        elif op == "RUN" and "cargo build" in arg:
            if not (arg_seen and env_seen):
                fail("S5-20", "Containerfile", "the build stage must set ARG CORTEX_GIT_SHA=unknown and ENV CORTEX_GIT_SHA=${CORTEX_GIT_SHA} before cargo build")
            return


def check_secrets(c):
    top = c.get("secrets") or {}
    for key, spec in top.items():
        if not isinstance(spec, dict) or spec.get("external") is not True:
            fail("S5-11", "compose.yml", f"secret {key!r} must be external: true (a Podman secret)")
        elif "file" in spec or "environment" in spec:
            fail("S5-11", "compose.yml", f"secret {key!r} must not read a file or variable")
    for sname, svc in (c.get("services") or {}).items():
        for s in svc.get("secrets") or []:
            src = s.get("source") if isinstance(s, dict) else s
            if src not in top:
                fail("S5-11", f"compose.yml service {sname}", f"uses secret {src!r} that is not declared")


def check_image_builder(c):
    """S5-06 (5h) an image of ours that nothing in this file builds is never pulled (pull_policy: never), so it would have to exist already: that is
    the signature of a release layout rewritten halfway (the server on the release image, the one-shot still on the dev tag)."""
    services = c["services"]
    if any("build" in (sv or {}) for sv in services.values()):
        return
    for n, sv in services.items():
        img = str((sv or {}).get("image", ""))
        if img.startswith("localhost/cv2-s5-"):
            fail("S5-06", f"compose.yml service {n}", f"image {img!r} is ours but nothing builds it in this file (a release layout names it ${{CORTEX_SERVER_IMAGE:?}})")


def check_stack_env(tree, services):
    """S5-06 (hale 5h N-2) a release layout names our image by a VARIABLE, so a pass here does not mean "digest-pinned": the pin is stack.env's. Say so, once per
    release image; and when a stack.env is in the directory, check the SHAPE of the two image values (name@sha256:<64 lowercase hex>). The engine's stack_env.rs holds the rest."""
    release = [n for n, sv in services.items() if is_release_image(sv or {})]
    if not release:
        return
    path = os.path.join(tree, "stack.env")
    if not os.path.isfile(path):
        for n in release:
            infos.append(("S5-06", f"compose.yml service {n}", "release image ${CORTEX_SERVER_IMAGE:?}: its digest pin is stack.env's and was NOT checked here (no stack.env in this directory)"))
        return
    values = {}
    for ln in read(tree, "stack.env").splitlines():
        ln = ln.strip()
        if ln and not ln.startswith("#") and "=" in ln:
            k, _, v = ln.partition("=")
            values[k.strip()] = v.strip()
    shaped = True
    for key in ("CORTEX_SERVER_IMAGE", "CORTEX_DB_IMAGE"):
        v = values.get(key)
        if v is None:
            fail("S5-06", "stack.env", f"stack.env does not name {key}")
            shaped = False
        elif not STACK_ENV_SHAPE.match(v):
            fail("S5-06", "stack.env", f"stack.env {key} is not name@sha256:<64 lowercase hex>")
            shaped = False
    if shaped:
        for n in release:
            infos.append(("S5-06", f"compose.yml service {n}", "release image ${CORTEX_SERVER_IMAGE:?}: the pin in stack.env was checked for SHAPE only (name@sha256:<64 hex>); the engine's stack_env.rs holds the rest"))


def check_owner_secret(c):
    services = c["services"]
    ours = {n: sv or {} for n, sv in services.items() if is_ours(sv or {})}
    oneshots = {n for n, sv in ours.items() if is_oneshot(sv)}
    for n, sv in ours.items():
        if role(sv) == "oneshot" and n not in oneshots:
            fail("S5-16", f"compose.yml service {n}", 'labelled cortexv2.role: oneshot but not an allowed one-shot (needs restart "no" and the command ["migrate"])')
    for n, sv in ours.items():
        if n in oneshots:
            continue
        for sec in sv.get("secrets") or []:
            src = sec.get("source") if isinstance(sec, dict) else sec
            if "owner" in str(src) or "superuser" in str(src):
                fail("S5-16", f"compose.yml service {n}", f"the long-running service mounts {src!r}; only a one-shot migrate may")
        deps = sv.get("depends_on")
        for o in sorted(oneshots):
            cond = deps.get(o, {}).get("condition") if isinstance(deps, dict) and isinstance(deps.get(o), dict) else None
            if cond != "service_completed_successfully":
                fail("S5-16", f"compose.yml service {n}", f"must depend on the one-shot {o} with condition service_completed_successfully")


# ---- S5-23 the release feature set ---------------------------------------------------------------------------------------

FEATURES_FILE = "server-features"
FEATURES_SRC = "deploy/server-features"
FEATURES_LINE = re.compile(r"[a-z][a-z0-9-]*(,[a-z][a-z0-9-]*)*\n")
IGNORE_FILE = "Containerfile.dockerignore"


def server_features_text(tree):
    """The raw text of deploy/server-features (no newline translation: a CR must show), or None when the file is not there (S5-14 says so)."""
    path = os.path.join(tree, FEATURES_FILE)
    if not os.path.isfile(path):
        return None
    with open(path, "rb") as f:
        return f.read().decode("utf-8", errors="replace")


def manifest_features(tree):
    """The keys of [features] in crates/cortex-server/Cargo.toml, which sits beside deploy/. Returns (keys, error text)."""
    path = os.path.join(tree, "..", "crates", "cortex-server", "Cargo.toml")
    try:
        with open(path, "rb") as f:
            feats = tomllib.loads(f.read().decode("utf-8")).get("features")
    except (OSError, UnicodeDecodeError, tomllib.TOMLDecodeError) as e:
        return None, f"cannot read crates/cortex-server/Cargo.toml (the set cannot be compared): {type(e).__name__}"
    if not isinstance(feats, dict):
        return None, "crates/cortex-server/Cargo.toml has no [features] table (the set cannot be compared)"
    return set(feats), None


def ignore_excludes(text, path):
    """Docker's rule for an ignore file: patterns in order, the LAST one that matches the path (or a directory above it) decides; `!` re-includes."""
    parts = path.split("/")
    cands = ["/".join(parts[:i]) for i in range(1, len(parts) + 1)]
    excluded = False
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        neg = line.startswith("!")
        pat = (line[1:] if neg else line).strip().strip("/")
        if any(fnmatch.fnmatchcase(c, pat) for c in cands):   # fnmatch's * crosses slashes, so **/name matches the name at any depth below deploy/
            excluded = not neg
    return excluded


def norm_path(p):
    p = str(p)
    return p[2:] if p.startswith("./") else p


# ---- S5-23: the RUN mount policy (sam S523-SAM01) --------------------------------------------------------------------------

MOUNT_TARGET_KEYS = ("target", "dst", "destination")
MOUNT_CACHE_KEYS = ("type", "id", "sharing", "mode", "uid", "gid") + MOUNT_TARGET_KEYS
MOUNT_SEED_KEYS = ("from", "source", "src")
RUN_NETWORK_VALUES = ("none", "default")
RUN_SECURITY_VALUES = ("sandbox",)
PATH_DIRS = ("/usr/local/sbin", "/usr/local/bin", "/usr/sbin", "/usr/bin", "/sbin", "/bin", "/usr/local/cargo/bin", "/root/.cargo/bin", "/opt/cargo/bin")


def clean_path(path):
    """The absolute path with . and .. and repeated slashes resolved, as a list of components; None when it is not absolute or holds a variable, a backtick or a quote."""
    if not path or not path.startswith("/") or any(c in path for c in "$`\"'"):
        return None
    parts = []
    for c in path.split("/"):
        if c in ("", "."):
            continue
        if c == "..":
            if parts:
                parts.pop()
            continue
        parts.append(c)
    return parts


def resolve_in(workdir, path):
    """The components of `path` as the build sees it, against the WORKDIR in force (a list of components); None when it cannot be known (a variable, a quote, an unknown WORKDIR)."""
    if path is None or any(c in path for c in "$`\"'"):
        return None
    if path.startswith("/"):
        return clean_path(path)
    if workdir is None:
        return None
    return clean_path("/" + "/".join(workdir) + "/" + path)


def paths_overlap(a, b):
    """True when one path is the other or lies above or below it, component by component."""
    n = min(len(a), len(b))
    return a[:n] == b[:n]


def stage_facts(ins):
    """Per instruction index of a RUN: (stage, the WORKDIR in force as a component list, or None when no explicit absolute WORKDIR was set earlier in the stage or the one in force is
    not a literal absolute path, whether any WORKDIR instruction was seen in the stage); per stage: the COPY and ADD destinations as (components or None, text); per stage: why it
    inherits context this rule does not see (None when the stage is built FROM an external image). A stage that starts FROM an earlier stage, from a variable or from nothing
    inherits that stage's files and WORKDIR, and an external base image may set a WORKDIR of its own: none of that is assumed empty (sam S523-SAM02)."""
    stage, workdir, seen = 0, None, False
    aliases = set()
    at, dests, inherits = {}, {}, {0: "a RUN that comes before any FROM"}
    for idx, (op, arg) in enumerate(ins):
        if op == "FROM":
            stage += 1
            workdir, seen = None, False
            toks = [t for t in arg.split() if not t.startswith("--")]
            image = toks[0] if toks else ""
            if not image:
                inherits[stage] = "the FROM names no image"
            elif any(c in image for c in "$`"):
                inherits[stage] = f"the FROM {image!r} is a variable"
            elif image.lower() in aliases:
                inherits[stage] = f"the stage is built FROM the earlier stage {image!r}"
            else:
                inherits[stage] = None
            if len(toks) >= 3 and toks[1].upper() == "AS":
                aliases.add(toks[2].lower())
        elif op == "WORKDIR":
            seen = True
            try:
                words = shlex.split(arg)
            except ValueError:
                words = []
            workdir = resolve_in(workdir, words[0]) if len(words) == 1 else None
        elif op in ("COPY", "ADD"):
            try:
                args = json.loads(arg) if arg.lstrip().startswith("[") else shlex.split(arg)
            except (ValueError, json.JSONDecodeError):
                args = []
            args = [str(a) for a in args if not str(a).startswith("--")]
            dests.setdefault(stage, []).append((resolve_in(workdir, args[-1]) if args else None, args[-1] if args else arg))
        elif op == "RUN":
            at[idx] = (stage, workdir, seen)
    return at, dests, inherits


def split_run_options(arg):
    """(the options as (name, value or None, text), the rest of the line, reasons). The options are the leading words that start with two hyphens."""
    opts, errors, rest = [], [], arg
    while True:
        m = re.match(r"\s*(--\S+)(?=\s|$)", rest)
        if not m:
            break
        tok = m.group(1)
        rest = rest[m.end():]
        name, eq, value = tok[2:].partition("=")
        if any(c in tok for c in "\"'") or not re.fullmatch(r"[A-Za-z][A-Za-z0-9-]*", name):
            errors.append(f"the RUN option {tok!r} cannot be read (a quote, or a name this rule does not know how to split)")
        opts.append((name, value if eq else None, tok))
    return opts, rest, errors


def mount_problem(value, protected, unjudgeable):
    """Why a --mount value is refused, or None. `protected` is a list of (components, what it is, how): how is "either" (the target may be neither above, equal nor below it) or
    "covers" (the target may be neither equal to it nor above it: a mount below the build directory hides nothing that was copied in by itself); `unjudgeable` is a reason text
    when a path of the stage cannot be resolved."""
    if not value:
        return "--mount has no value (only --mount=type=cache,target=<path> is accepted)"
    pairs = [(part.partition("=")[0].strip(), part.partition("=")[2] if "=" in part else None) for part in value.split(",")]
    bare = [k for k, v in pairs if v is None]
    if bare:
        return f"the mount word {bare[0]!r} has no value; a bare word such as ro is not in the allowlist"
    keys = [("target" if k in MOUNT_TARGET_KEYS else k) for k, _ in pairs]
    dup = next((k for k in keys if keys.count(k) > 1), None)
    if dup:
        return f"the mount key {dup!r} is given twice (the allowlist reads one value of each)"
    kv = dict(pairs)
    if "type" not in kv:
        return "the mount names no type, so it is a BIND mount (the default type), which can cover a path with other bytes; only type=cache is accepted"
    if kv["type"] != "cache":
        return f"type={kv['type']} is refused: only type=cache is accepted, and no other mount form (bind, tmpfs, secret and ssh included)"
    seed = next((k for k in MOUNT_SEED_KEYS if k in kv), None)
    if seed:
        return f"the key {seed!r} seeds the cache from another stage or from the build context: bytes this rule does not see"
    unknown = next((k for k in kv if k not in MOUNT_CACHE_KEYS), None)
    if unknown:
        return f"the mount key {unknown!r} is not in the allowlist ({', '.join(MOUNT_CACHE_KEYS[:-2])}, target)"
    target = next((kv[k] for k in MOUNT_TARGET_KEYS if k in kv), None)
    if target is None:
        return "the cache mount has no target"
    if not target.startswith("/"):
        return f"the cache target {target!r} is relative: it cannot be compared with the paths the build reads (write an absolute path)"
    comps = clean_path(target)
    if comps is None:
        return f"the cache target {target!r} holds a variable or a quote: it cannot be compared with the paths the build reads"
    if unjudgeable:
        return unjudgeable
    for other, what, how in protected:
        if paths_overlap(comps, other) if how == "either" else other[:len(comps)] == comps:
            return f"the cache target {target!r} overlaps {what} (/{'/'.join(other)}): it can hide what the Containerfile copied in or reads"
    return None


def check_run_options(ins):
    """Every RUN option of the Containerfile is validated (sam S523-SAM01): see the module docstring, THE RUN MOUNT POLICY."""
    at, dests, inherits = stage_facts(ins)
    for idx, (op, arg) in enumerate(ins):
        if op != "RUN":
            continue
        opts, _rest, errors = split_run_options(arg)
        for e in errors:
            fail("S5-23", "Containerfile", e)
        stage, workdir, seen = at[idx]
        for name, value, tok in opts:
            if name == "mount":
                unjudgeable = None
                if inherits.get(stage) is not None:
                    unjudgeable = f"inherited or unknown context is not judged: {inherits[stage]}; a stage that is not built FROM an external image inherits files and a WORKDIR this rule does not see, so a cache mount cannot be judged there (fail closed)"
                elif workdir is None:
                    unjudgeable = ("inherited or unknown context is not judged: no explicit absolute WORKDIR is set earlier in this stage, so the build directory is not known and a cache mount cannot be judged (fail closed)" if not seen else
                                   "inherited or unknown context is not judged: the WORKDIR in force is not a literal absolute path, so the build directory is not known and a cache mount cannot be judged (fail closed)")
                elif any(d is None for d, _ in dests.get(stage, [])):
                    unjudgeable = "a COPY or ADD destination of this stage is not a literal path, so the cache target cannot be compared with it (fail closed)"
                protected = ([(workdir, "the build directory (the WORKDIR in force)", "covers")] if workdir is not None else []) + \
                            [(d, f"the destination {raw!r} of a COPY or ADD of this stage", "either") for d, raw in dests.get(stage, []) if d is not None] + \
                            [(clean_path(d), "a default PATH directory", "either") for d in PATH_DIRS]
                problem = mount_problem(value, protected, unjudgeable)
                if problem:
                    fail("S5-23", "Containerfile", f"RUN {tok}: {problem}")
            elif name == "network" and value in RUN_NETWORK_VALUES:
                continue
            elif name == "security" and value in RUN_SECURITY_VALUES:
                continue
            elif name == "network" and value == "host":
                fail("S5-23", "Containerfile", f"RUN {tok}: --network=host asks the builder to run the step on the host network, an entitlement that weakens the build's isolation; a release Containerfile has no reason to ask for it")
            elif name == "security" and value == "insecure":
                fail("S5-23", "Containerfile", f"RUN {tok}: --security=insecure asks the builder to run the step with extra privileges, an entitlement that weakens the build's isolation; a release Containerfile has no reason to ask for it")
            else:
                fail("S5-23", "Containerfile", f"RUN {tok}: this RUN option is not accepted (only --mount=type=cache, --network={'|'.join(RUN_NETWORK_VALUES)} and --security={'|'.join(RUN_SECURITY_VALUES)}), so the line cannot be judged")


CARGO_HARMLESS = frozenset({"fetch", "vendor", "clean", "metadata", "tree", "update", "generate-lockfile", "locate-project", "pkgid", "verify-project", "version", "help", "fmt"})
SHELL_CUTS = frozenset("();|&")
SHELL_PREFIX_WORDS = frozenset({"exec", "env", "time", "command", "nice", "nohup"})
SHELL_ASSIGN = re.compile(r"[A-Za-z_][A-Za-z0-9_]*=")
CARGO_WORD = re.compile(r"(?<![A-Za-z0-9_-])cargo(?![A-Za-z0-9_-])")


def run_commands(arg):
    """The simple commands of a RUN line, each a list of words: shell quoting is honoured and the line is cut at && || ; | & ( ). None when the line cannot be read as
    shell words (a quote that never closes): the rule then says so and does not guess."""
    try:
        lex = shlex.shlex(arg, posix=True, punctuation_chars="();|&")
        lex.whitespace_split = True
        lex.commenters = ""   # a shell starts a comment only at the start of a word; shlex would end the line at a # inside a word (hale)
        words = list(lex)
    except ValueError:
        return None
    cmds, cur = [], []
    for w in words:
        if w and set(w) <= SHELL_CUTS:
            cmds.append(cur)
            cur = []
        else:
            cur.append(w)
    cmds.append(cur)
    return [c for c in cmds if c]


def cargo_invocation(cmd):
    """The words after `cargo` when the command IS a cargo invocation (a leading VAR=value, exec, env, time, command, nice or nohup is skipped), else None."""
    i = 0
    while i < len(cmd) and (SHELL_ASSIGN.match(cmd[i]) or cmd[i] in SHELL_PREFIX_WORDS):
        i += 1
    if i < len(cmd) and os.path.basename(cmd[i]) == "cargo":
        return cmd[i + 1:]
    return None


def check_features_build(text):
    """The Containerfile half of S5-23: the file is COPYed before each cargo build, and each `cargo build` (judged one invocation at a time, on the words of the shell
    line) passes exactly one --features reading it. Any other cargo subcommand that is not harmless is refused: it could build the server with another set."""
    copies, builds = [], []
    check_run_options(instructions(text))
    for idx, (op, arg) in enumerate(instructions(text)):
        if op in ("COPY", "ADD") and "--from=" not in arg:
            try:
                args = json.loads(arg) if arg.lstrip().startswith("[") else shlex.split(arg)
            except (ValueError, json.JSONDecodeError):
                continue
            args = [str(a) for a in args if not str(a).startswith("--")]
            if len(args) == 2 and norm_path(args[0]) == FEATURES_SRC:
                dest = norm_path(args[1])
                copies.append((idx, dest + FEATURES_FILE if dest.endswith("/") else dest))
        elif op == "RUN":
            arg = split_run_options(arg)[1]   # BuildKit flags of the RUN instruction are not part of the command (they were validated by check_run_options above)
            cmds = run_commands(arg)
            if cmds is None:
                if re.search(r"\bcargo\b", arg):
                    fail("S5-23", "Containerfile", "a RUN line that names cargo cannot be read as shell words, so the rule cannot say what it builds")
                continue
            understood = 0
            for cmd in cmds:
                words = cargo_invocation(cmd)
                if words is None:
                    continue
                understood += len(CARGO_WORD.findall(next(w for w in cmd if os.path.basename(w) == "cargo")))
                sub = next((w for w in words if not w.startswith(("-", "+"))), None)
                if sub is None or sub in CARGO_HARMLESS:
                    continue
                if sub in ("build", "b"):
                    builds.append((idx, words))
                else:
                    fail("S5-23", "Containerfile", f"the Containerfile runs `cargo {sub}`: only `cargo build` with the file's set may compile the server (another subcommand can build it with any features)")
            if len(CARGO_WORD.findall(arg)) != understood:   # fail CLOSED (hale): cargo anywhere but as the first word of a command this reader understood cannot be judged
                fail("S5-23", "Containerfile", "the word cargo appears in a position this rule cannot judge (not as the first word of a command it reads: a wrapper such as timeout or sudo, if/then, braces, a backtick, the exec form, an argument): refused")
    if not builds:
        fail("S5-23", "Containerfile", "there is no cargo build to carry the feature set")
    for idx, words in builds:
        before = [d for i, d in copies if i < idx]
        if not before:
            fail("S5-23", "Containerfile", f"the Containerfile {'does not COPY' if not copies else 'COPYs'} {FEATURES_SRC} {'into the build stage' if not copies else 'only after the cargo build'}; the build must read the file's set")
            continue
        flags = [w for w in words if w in ("--all-features", "--features") or w.startswith(("--features=", "-F"))]
        if "--all-features" in flags:
            fail("S5-23", "Containerfile", "the cargo build uses --all-features, which builds test-faults in; the release set is the file's")
        elif len(flags) > 1:
            fail("S5-23", "Containerfile", "the cargo build names --features more than once; the file's set is the only one")
        elif not any(w == "--features" and k + 1 < len(words) and words[k + 1] == f"$(cat {d})" for k, w in enumerate(words) for d in before):
            fail("S5-23", "Containerfile", f'the cargo build must pass --features "$(cat <the path {FEATURES_SRC} was copied to>)" exactly, e.g. --features "$(cat {before[-1]})"')


def check_server_features(tree):
    """S5-23, see the module docstring."""
    text = server_features_text(tree)
    if text is None:
        return
    items = text[:-1].split(",") if FEATURES_LINE.fullmatch(text) else None
    if items is None or len(set(items)) != len(items):
        fail("S5-23", FEATURES_FILE, "must be ONE line: the optional-piece features of crates/cortex-server separated by single commas (lower-case letters, digits and hyphen, each once) and ended by one LF")
    else:
        pieces, err = manifest_features(tree)
        if err:
            fail("S5-23", FEATURES_FILE, err)
        else:
            if "default" in pieces:
                fail("S5-23", FEATURES_FILE, "crates/cortex-server declares a default feature; the set is named only by server-features, so a default would hide it")
            optional = pieces - {"test-faults", "default"}
            if "test-faults" in items:
                fail("S5-23", FEATURES_FILE, "test-faults is never in a release set (it is the test-only fault switch)")
            for piece in sorted(optional - set(items)):
                fail("S5-23", FEATURES_FILE, f"missing from server-features: {piece} (an optional piece of crates/cortex-server the release would be built without)")
            for piece in sorted(set(items) - optional - {"test-faults"}):
                fail("S5-23", FEATURES_FILE, f"in server-features but not an optional piece of crates/cortex-server: {piece}")
    if os.path.isfile(os.path.join(tree, "Containerfile")):
        check_features_build(read(tree, "Containerfile"))
        infos.append(("S5-23", "Containerfile", "read as TEXT (hale S523-5 to S523-8): cargo started through sh -c, xargs or a script, WORKDIR (for the feature file's COPY and cat; the RUN mount policy does track it) and later FROM stages, Docker's own ignore matcher and the manifest's implicit features are NOT seen; see the rule's docstring"))
        if os.path.isfile(os.path.join(tree, IGNORE_FILE)) and ignore_excludes(read(tree, IGNORE_FILE), FEATURES_SRC):
            fail("S5-23", IGNORE_FILE, f"excludes {FEATURES_SRC} from the build context, so the Containerfile cannot COPY it (add `!{FEATURES_SRC}` after the line that drops deploy)")


# ---- Containerfile ---------------------------------------------------------------------------------------------------

def instructions(text):
    out, buf = [], ""
    for raw in text.splitlines():
        s = raw.rstrip()
        if not s.strip() or s.lstrip().startswith("#"):   # Docker drops comment and empty lines inside a continued instruction too (hale N23): the instruction goes on
            continue
        if s.endswith("\\"):
            buf += s[:-1] + " "
            continue
        buf += s
        word = buf.strip().split(None, 1)
        out.append((word[0].upper(), word[1] if len(word) > 1 else ""))
        buf = ""
    if buf.strip():
        word = buf.strip().split(None, 1)
        out.append((word[0].upper(), word[1] if len(word) > 1 else ""))
    return out


def check_containerfile(text):
    check_git_sha_build(text)
    stages = set()
    for op, arg in instructions(text):
        if op == "FROM":
            toks = [t for t in arg.split() if not t.startswith("--")]
            image = toks[0] if toks else ""
            if len(toks) >= 3 and toks[1].upper() == "AS":
                alias = toks[2]
            else:
                alias = None
            if image != "scratch" and image not in stages:
                if "$" in image:
                    fail("S5-06", "Containerfile", f"FROM {image!r} must be a literal image, not a variable")
                elif not DIGEST.search(image):
                    fail("S5-06", "Containerfile", f"FROM {image!r} is not pinned by @sha256 digest")
                if re.search(r":latest(@|$)", image):
                    fail("S5-06", "Containerfile", f"FROM {image!r} is on latest")
            if alias:
                stages.add(alias)
        elif op in ("COPY", "ADD"):
            if "--from=" in arg:
                continue
            try:
                args = json.loads(arg) if arg.lstrip().startswith("[") else shlex.split(arg)
            except (ValueError, json.JSONDecodeError):
                fail("S5-05", "Containerfile", f"{op} line cannot be read: {arg!r}")
                continue
            args = [a for a in args if not str(a).startswith("--")]
            for src in args[:-1]:
                base = os.path.basename(str(src).rstrip("/"))
                if any(p.search(base) for p in CONFIG_PATTERNS) or "secret" in str(src).lower():
                    fail("S5-05", "Containerfile", f"{op} of {src!r} bakes config or a secret into the image")
        elif op in ("ENV", "ARG"):
            try:
                toks = shlex.split(arg)
            except ValueError:
                fail("S5-05", "Containerfile", f"{op} line cannot be read: {arg!r}")
                continue
            keys = [t.split("=", 1)[0] for t in toks] if toks and "=" in toks[0] else toks[:1]
            for k in keys:
                if SECRET_WORDS.search(k) and not k.upper().endswith("_FILE"):
                    fail("S5-05", "Containerfile", f"{op} {k} looks like a secret; it would stay in the image history")


# ---- shell scripts and the config example ----------------------------------------------------------------------------

# S5-10 (cox's ruling on the install lane): a file that a line of install.sh names with the exact trailing marker `# @include` is JOINED into the one install.sh
# asset by the release builder. The source must have no shebang and hold only function definitions; the lint observes one fixed environment and does not prove that requirement.
# Every other .sh keeps the shebang, set -eu and syntax rules. The dev form is `[ ! -f "$here/<name>" ] || . "$here/<name>"   # @include` (or without the guard).
INCLUDE_LINE = re.compile(r'^\s*(?:\[ ! -f "\$here/[A-Za-z0-9._-]+" \] \|\| )?\. "\$here/([A-Za-z0-9._-]+)"\s+# @include\s*$')


def included_sources(tree):
    """The names of the files that install.sh includes (the lines that end with the exact marker `# @include`)."""
    path = os.path.join(tree, "install.sh")
    if not os.path.isfile(path):
        return set()
    with open(path, encoding="utf-8") as f:
        return {m.group(1) for m in (INCLUDE_LINE.match(ln) for ln in f.read().splitlines()) if m}


# S5-10 is a LINT with a dynamic oracle: it sources an included file in a fixed, confined environment (bash --posix -r, xtrace, sentinel, namespace) and refuses it if anything
# runs, prints or exits there; it also checks syntax (sh -n), the closed ending and the control-character rule. It is NOT a proof that the file consists of function definitions only.
# The bytes go to bash untouched (no text decoding, no splitlines).
# KILL RULE OF THIS VM (kai, cox W1-70): the oracle runs INSIDE `unshare --user --pid --fork --mount-proc`, so even if every other layer failed a signal reaches only that namespace;
# when unshare (or a bash >= 4.4) is not there the checker FAILS LOUDLY (exit 2, "cannot run") and NEVER runs the file unconfined. Further layers: a clean environment, PATH = an
# empty directory (nothing external can start), cwd = an empty directory, restricted mode (no redirect out, no cd, no exec, no `.` or command -p), the builtin kill disabled, no new
# process (ulimit -u 1), a memory, CPU and file-size limit, a new session, and the first byte of output ends the run (SIGKILL to that session's group, the exact group this call
# created). Restricted mode is not a security boundary; S5-15 stays the static backstop. NO CELL OR PROBE carries a destructive payload: the sandbox is tested by QUERY (type, $-, $$,
# ulimit) and by harmless canaries, never by calling the dangerous command.
ORACLE_PRELUDE = b"enable -n kill suspend\nset -x\n"
# What is appended after the file. Only POSIX SPECIAL builtins (a file cannot shadow them in --posix mode): `case $-` finds out that xtrace is still on (a file that switched it off with
# `set +x` is refused: exit 99), then the state is printed (options, exported and read-only variables, traps) and the shell ends with the fixed status 73. A clean file makes the
# output EXACTLY that of an empty file and the status 73, so a refusal that is only a restriction message, a syntax error or a swallowed epilogue can never pass as an accept.
ORACLE_EPILOGUE = b"\ncase $- in *x*) ;; *) exit 99 ;; esac\nset +x\nset +o\nexport -p\nreadonly -p\ntrap -p\nexit 73\n"
ORACLE_SENTINEL_STATUS = 73
ORACLE_SECONDS = 20
MOUNT_PATHS = ("/usr/bin/mount", "/bin/mount")
UNSHARE_PATHS = ("/usr/bin/unshare", "/bin/unshare")
ORACLE_ENV = {"PATH": "/oracle/path", "HOME": "/oracle/home", "LC_ALL": "C"}   # fixed, and neither directory exists: the state printed after the file is comparable and nothing external can start
ORACLE_TOOLS = None
ORACLE_VERSION = None
ORACLE_BASELINE = None


class OracleUnavailable(Exception):
    """The oracle cannot run here: the checker exits 2 (it could not run), never 0."""


def _wrapper(mount):
    """The first stage inside the namespace (bash builtins and one absolute mount only): a private, EMPTY, tmpfs /tmp as the working directory, then the limits, then the restricted bash."""
    return (f"{shlex.quote(mount)} -t tmpfs -o size=1m,mode=0755,nosuid,nodev,noexec tmpfs /tmp && cd /tmp && "
            'ulimit -t 20 -v 2097152 -f 0 -c 0 -u 1 && exec "$1" --norc --noprofile --posix -r -s')


def oracle_tools():
    """(unshare, bash, mount) after a pre-flight; raises OracleUnavailable with the reason. The namespace: user (the caller is root of it only: it maps to no privilege outside, and mount needs it), pid, mount (a private /proc and an empty tmpfs /tmp) and NO network."""
    global ORACLE_TOOLS, ORACLE_VERSION
    if ORACLE_TOOLS is not None:
        return ORACLE_TOOLS
    if os.geteuid() == 0:   # never root (kai W1-71): `--map-root-user` would map the caller to the real root, whose file permissions the user namespace does not take away
        raise OracleUnavailable("S5-10 does not run the oracle as root (the user namespace would map the caller to the real root); run the checker as an ordinary user")
    bash = shutil.which("bash")
    if not bash:
        raise OracleUnavailable("bash was not found: S5-10 reads an included source with bash (4.4 or newer)")
    unshare = next((c for c in UNSHARE_PATHS if os.access(c, os.X_OK)), None) or shutil.which("unshare")
    if not unshare:
        raise OracleUnavailable("unshare was not found: S5-10 runs the shell inside `unshare --user --map-root-user --pid --fork --mount-proc --net` and never unconfined (the kill rule of this VM)")
    mount = next((c for c in MOUNT_PATHS if os.access(c, os.X_OK)), None)
    if not mount:
        raise OracleUnavailable("mount was not found: the oracle's private /tmp is a tmpfs mounted inside the namespace")
    try:
        p = subprocess.run([unshare, "--user", "--map-root-user", "--pid", "--fork", "--mount-proc", "--net", bash, "--norc", "--noprofile", "-c", 'echo "$BASH_VERSINFO ${BASH_VERSINFO[1]} $$"'],
                           capture_output=True, text=True, timeout=30, check=False)
    except (OSError, subprocess.SubprocessError) as e:
        raise OracleUnavailable(f"the pre-flight of the namespace failed: {e}")
    parts = p.stdout.split()
    if p.returncode != 0 or len(parts) != 3 or not parts[0].isdigit() or not parts[1].isdigit() or parts[2] != "1":
        raise OracleUnavailable(f"the namespace for the oracle cannot be created here (unshare --user --map-root-user --pid --fork --mount-proc --net; rc {p.returncode}): {(p.stderr or p.stdout).strip()[:200]}")
    if (int(parts[0]), int(parts[1])) < (4, 4):
        raise OracleUnavailable(f"bash {parts[0]}.{parts[1]} is older than 4.4")
    ORACLE_TOOLS = (unshare, bash, mount)
    ORACLE_VERSION = f"{parts[0]}.{parts[1]}"
    return ORACLE_TOOLS


def _oracle_run(data, expect):
    """Run `data` as the script of the restricted bash inside the namespace. With `expect` (bytes) the run ends at the first byte that is not the next byte of `expect`; without it
    the whole output is read. Returns (output bytes, exit status or None when it was ended or timed out, note)."""
    unshare, bash, mount = oracle_tools()
    with tempfile.TemporaryDirectory(prefix="s5-oracle-") as d:
        os.chmod(d, 0o755)
        script = os.path.join(d, "script.sh")
        with open(script, "wb") as f:
            f.write(data)
        os.chmod(script, 0o644)
        with open(script, "rb") as stdin:
            try:
                p = subprocess.Popen([unshare, "--user", "--map-root-user", "--pid", "--fork", "--mount-proc", "--net", bash, "--norc", "--noprofile", "-c", _wrapper(mount), "oracle", bash],
                                     stdin=stdin, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, cwd="/", env=ORACLE_ENV, start_new_session=True)
            except (OSError, subprocess.SubprocessError) as e:
                raise OracleUnavailable(f"the oracle could not be started: {e}")
        out = b""
        note = ""
        ended = False
        try:
            while True:
                if not select.select([p.stdout], [], [], ORACLE_SECONDS)[0]:
                    note = f"bash did not finish reading the file in {ORACLE_SECONDS} s"
                    ended = True
                    break
                chunk = os.read(p.stdout.fileno(), 4096)
                if not chunk:
                    break
                out += chunk
                if (expect is not None and not expect.startswith(out)) or len(out) > (1 << 20):
                    ended = True
                    break
        finally:
            try:
                os.killpg(p.pid, signal.SIGKILL)
            except (ProcessLookupError, PermissionError):
                pass
            rc = p.wait()
            p.stdout.close()
        return out, (None if ended else rc), note


def oracle_baseline():
    """What the oracle prints for an EMPTY file, twice and equal (else the oracle is not trustworthy here)."""
    global ORACLE_BASELINE
    if ORACLE_BASELINE is None:
        script = ORACLE_PRELUDE + ORACLE_EPILOGUE
        first, rc1, note = _oracle_run(script, None)
        second, rc2, _ = _oracle_run(script, None)
        if rc1 != ORACLE_SENTINEL_STATUS or not first.startswith(b"+ case $- in\n+ set +x\n") or second != first or rc2 != rc1:
            raise OracleUnavailable(f"the oracle's own baseline run of an empty file did not end as designed (status {rc1}, repeatable {second == first}): {first.decode('utf-8', 'replace')[:200]} {note}")
        ORACLE_BASELINE = first
    return ORACLE_BASELINE


def source_trace(path, whole=False):
    """The oracle's answer for the included source at `path`: "" when it ACCEPTS (the output is exactly an empty file's and the status is 73), else a short text with what bash printed or
    how it ended. `whole` (the self-test's queries) returns everything bash printed for the file, followed by the epilogue's state dump. Raises OracleUnavailable when it cannot run."""
    with open(path, "rb") as f:
        data = f.read()
    script = ORACLE_PRELUDE + data + ORACLE_EPILOGUE
    if whole:
        out, rc, note = _oracle_run(script, None)
        return out.decode("utf-8", "replace") + note
    base = oracle_baseline()
    out, rc, note = _oracle_run(script, base)
    if rc == ORACLE_SENTINEL_STATUS and out == base:
        return ""
    if out and out != base[:len(out)]:
        words = " | ".join(ln.strip() for ln in out.decode("utf-8", "replace").splitlines() if ln.strip())[:300]
        return words or "bash printed something that an empty file does not"
    if note:
        return note
    return f"bash ended with status {rc} where an empty file ends with {ORACLE_SENTINEL_STATUS}" + (" (99: xtrace was switched off)" if rc == 99 else "")


FORBIDDEN_CHARS = {0x0b: "VT", 0x0c: "FF", 0x0d: "CR", 0x1c: "FS", 0x1d: "GS", 0x1e: "RS", 0x7f: "DEL", 0x85: "NEL", 0x2028: "U+2028", 0x2029: "U+2029"}


def included_findings(path):
    """Every reason S5-10 refuses the included source at `path` (a list of texts; empty = accepted). Raises OracleUnavailable. This is also the fuzz entry point: source_verdict."""
    with open(path, "rb") as f:
        data = f.read()
    found = []
    if data[:2] == b"#!":
        found.append("an included source must not have a shebang (the release builder joins it into install.sh)")
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        text = None
        found.append("an included source must be UTF-8 text")
    if text is not None:
        # a line break for some readers (a text-mode read, splitlines) and an ordinary character for the shell (hale F-1): refused at the read; only TAB and LF are allowed among the controls
        bad = sorted({ord(c) for c in text if (ord(c) < 32 and c not in "\t\n") or ord(c) in FORBIDDEN_CHARS})
        if bad:
            found.append("an included source holds a control character the shell reads as an ordinary character and a line reader as a line break (" + ", ".join(FORBIDDEN_CHARS.get(c, f"U+{c:04X}") for c in bad) + "): only TAB and LF are allowed")
        # the file is joined into install.sh (cox W1-70): it ends with a newline and its last line is exactly one column-0 closer of a function, so nothing can run on into what follows it
        if not data.endswith(b"\n") or data.split(b"\n")[-2:-1] not in ([b"}"], [b")"]):
            found.append("an included source ends with a newline and its last line is exactly one closer, } or ) (the last function of a subshell form), at column 0")
    out = source_trace(path)
    if out:
        found.append(f"an included source holds functions only: sourcing it must run nothing, and bash (--posix -r, xtrace, in a namespace) says it does not: {out}")
    r = subprocess.run(["sh", "-n", path], capture_output=True, text=True, timeout=30, check=False)
    if r.returncode != 0:
        found.append(f"sh -n: {(r.stderr.strip().splitlines() or ['syntax error'])[0]}")
    return found


def source_verdict(path):
    """None when S5-10 accepts the included source at `path`, else the first refusal as text. The entry point for a differential fuzz (hale): one call, one verdict."""
    found = included_findings(path)
    return found[0] if found else None


def check_included(tree, rel):
    for text in included_findings(os.path.join(tree, rel)):
        fail("S5-10", rel, text)


def check_shell(tree, rel):
    path = os.path.join(tree, rel)
    with open(path, encoding="utf-8") as f:
        lines = f.read().splitlines()
    m = re.match(r"#!\s*(?:/usr/bin/env\s+)?(?:/bin/)?(sh|bash)\b", lines[0]) if lines else None
    if not m:
        fail("S5-10", rel, "first line must be a #!/bin/sh or bash shebang")
        return
    flags = "".join(
        re.match(r"\s*set\s+(.*)$", ln).group(1).replace("-", "").replace("o pipefail", "")
        for ln in lines[:20] if re.match(r"\s*set\s+-", ln)
    )
    if "e" not in flags or "u" not in flags:
        fail("S5-10", rel, "needs `set -e` and `set -u` (for example `set -eu`) in its first 20 lines")
    r = subprocess.run([m.group(1), "-n", path], capture_output=True, text=True, timeout=30, check=False)
    if r.returncode != 0:
        fail("S5-10", rel, f"{m.group(1)} -n: {(r.stderr.strip().splitlines() or ['syntax error'])[0]}")


def walk_toml(node, rel):
    for k, v in node.items():
        if isinstance(v, dict):
            walk_toml(v, rel)
        elif SECRET_WORDS.search(k) and not k.lower().endswith("_file") and v not in ("", None):
            fail("S5-04", rel, f"key {k!r} holds a value; use a *_file path to a Podman secret")


# ---- S5-15: the NEVER list ---------------------------------------------------------------------------------------------

SHELL_SHEBANG = re.compile(r"^#!\s*(?:/usr/bin/env\s+)?(?:/bin/)?(sh|bash)\b")
ALLOW = re.compile(r"#\s*s5-never:\s*ok\s+(\S.*)$")
NEVER = [
    ("pkill/killall", re.compile(r"\b(pkill|killall)\b"), "matches processes by name; stop a process by its exact pid"),
    ("kill -9", re.compile(r"\bkill\s+(-9|-KILL|-SIGKILL|-s\s*(9|KILL|SIGKILL))\b"), "a forced kill; stop a process by its exact pid, politely"),
    ("group signal", re.compile(r"\bkill\b[^\n;|&]*\s--\s+-\d|\bkill\s+(-[A-Za-z0-9]+\s+)?(0|-1)(\s|$)|\bkill\s+(?:\S+\s+)+[\"']?-[\"']?[\d$]"), "a signal to a process group (a negative pid, literal or variable) or to everything"),
    ("podman rm/kill/stop all", re.compile(r"\bpodman\s+(container\s+)?(rm|kill|stop|restart|pause|unpause)\b[^\n;|&]*(\s-[A-Za-z]*a[A-Za-z]*(\s|$)|\s--all\b)"), "acts on every container; name the container"),
    ("kill by pattern", re.compile(r"\bpgrep\b[^\n]*\|\s*xargs\b|\bxargs\b[^\n]*\bkill\b|\bkill\b[^\n]*(\$\(|`)\s*(pgrep|pidof|ps)\b"), "kills what a pattern finds; stop a process by its exact pid"),
    ("podman over every container", re.compile(r"\bpodman\s+(container\s+)?(rm|stop|kill)\b[^\n]*(\$\(|`)\s*podman\s+(ps|container\s+ls)\b|\bxargs\b[^\n]*\bpodman\s+(container\s+)?(rm|stop|kill)\b"), "acts on a list of containers it did not name"),
    ("podman prune/reset", re.compile(r"\bpodman\s+(system|volume|image|container|pod|network)\s+(prune|reset)\b"), "removes things that are not ours"),
    ("compose down -v", re.compile(r"\bcompose\b[^\n;|&]*\bdown\b[^\n;|&]*(\s-v\b|\s--volumes\b)"), "deletes data volumes"),
    ("git push", re.compile(r"\bgit\s+push\b"), "no push to a remote"),
    ("v1 names", re.compile(r"\bcortex-(pg|api|embed-worker|graph-worker|pdf-worker|audio-worker|vision-worker)\b|\bspike-s0-|\bharness-appdb\b|\bplatform_agent_memory\b"), "names the live v1 stack"),
]
# Printing a value. Skipped on a line that redirects or pipes (the value is going somewhere private) and in *-test.sh scripts (fake tokens only).
PRINT_CONTENT = [
    ("prints a token/secret file", re.compile(r"\b(cat|head|tail|less|more)\b[^\n]*(token|secret|password|passwd|\.pem|\.key|\$\{?(new|dest|part)\b)", re.I), "shows a secret on the terminal"),
    ("prints a secret variable", re.compile(r"\b(echo|printf)\b[^\n]*\$\{?(val|[A-Za-z_]*(secret|password|passwd|bearer|apikey|api_key|tokenval|token_value)[A-Za-z_]*)\b", re.I), "shows a secret on the terminal"),
]
notes = []
infos = []   # (rule, where, message): what a PASS did not check (hale 5h N-2); printed as NOTE lines, never a finding
STACK_ENV_SHAPE = re.compile(r"^[A-Za-z0-9._:/-]{1,300}@sha256:[0-9a-f]{64}\Z")


def shell_files(root):
    for dirpath, _dirs, files in os.walk(root):
        for fn in sorted(files):
            path = os.path.join(dirpath, fn)
            if fn.endswith(".sh"):
                yield path
                continue
            try:
                with open(path, "rb") as f:
                    first = f.readline(200)
            except OSError:
                continue
            if SHELL_SHEBANG.match(first.decode("utf-8", "replace")):
                yield path


SYSTEMD_TOKENS = {"@CORTEX_HOME@", "@DB_CONTAINER@"}


def check_systemd(tree):
    """S5-21 the units in <tree>/systemd/: only the two placeholders install.sh fills, every Exec* and Documentation path under @CORTEX_HOME@/, no % and no $."""
    d = os.path.join(tree, "systemd")
    if not os.path.isdir(d):
        return
    installed = set(os.listdir(d))
    for fn in sorted(installed):
        rel = f"systemd/{fn}"
        text = read(tree, rel)
        for n, raw in enumerate(text.splitlines(), 1):
            where = f"{rel}:{n}"
            line = raw.lstrip()   # systemd strips leading whitespace
            if raw.rstrip().endswith("\\"):
                fail("S5-21", where, "a line continuation (trailing backslash) is not allowed in a unit template")
            for tok in re.findall(r"@[A-Za-z0-9_]*@", line):
                if tok not in SYSTEMD_TOKENS:
                    fail("S5-21", where, f"{tok} is not a placeholder install.sh fills (only {', '.join(sorted(SYSTEMD_TOKENS))})")
            if "%" in line or "$" in line:
                fail("S5-21", where, "a % (systemd specifier) or a $ (variable) is not allowed in a unit template")
            if line.startswith("Exec"):
                value = line.split("=", 1)[1].strip() if "=" in line else ""
                if re.search(r"(^|\s);(\s|$)", value):
                    fail("S5-21", where, "a second command (; separator) on an Exec line is not allowed")
                if value.startswith("-"):   # systemd's "ignore a failure" prefix: only on ExecStartPost (the status unit's clear), never on a command whose failure must be seen
                    if line.split("=", 1)[0].strip() != "ExecStartPost":
                        fail("S5-21", where, f"the - prefix hides a failure; it is allowed on ExecStartPost only, not {line.split('=', 1)[0].strip()}")
                    value = value[1:]
                if not value.startswith("@CORTEX_HOME@/"):
                    fail("S5-21", where, f"{line.split('=', 1)[0]} must start with @CORTEX_HOME@/, not {value[:40]!r}")
            elif line.startswith(("Unit=", "OnFailure=")):
                # hale N-3: a timer's Unit= and a service's OnFailure= must name a unit that is IN this directory, or the backup (or its alarm) never runs, silently
                key = line.split("=", 1)[0]
                for target in line.split("=", 1)[1].split():
                    if target not in installed:
                        fail("S5-21", where, f"{key}= names {target[:50]!r}, which is not a unit in this directory")
            elif line.startswith("Documentation="):
                for entry in line.split("=", 1)[1].split():
                    path = entry[len("file:"):] if entry.startswith("file:") else entry
                    if not path.startswith("@CORTEX_HOME@/"):
                        fail("S5-21", where, f"Documentation must point under @CORTEX_HOME@/, not {entry[:40]!r}")


HANDLED_SERVICES = {"cortex-dump.service": "cortex-dump-alert.service", "cortex-dump-status.service": "cortex-dump-alert.service", "cortex-dump-alert.service": None}


def check_unit_handlers(tree):
    """S5-21 (hale 5k): the failure handler of each backup service is named, exactly once, in [Unit], and a timer triggers its own service from [Timer]. A service with no row in HANDLED_SERVICES is refused."""
    d = os.path.join(tree, "systemd")
    if not os.path.isdir(d):
        return
    for fn in sorted(os.listdir(d)):
        rel = f"systemd/{fn}"
        keys = []
        section = ""
        for n, raw in enumerate(read(tree, rel).splitlines(), 1):
            ln = raw.strip()
            sec = re.match(r"^\[([^\]]+)\]$", ln)
            if sec:
                section = sec.group(1)
            elif ln and not ln.startswith("#") and "=" in ln:
                k, v = ln.split("=", 1)
                keys.append((n, section, k.strip(), v.strip()))
        if fn.endswith(".service") and fn not in HANDLED_SERVICES:
            fail("S5-21", rel, "a service unit has no row in HANDLED_SERVICES (the table that names its failure handler): add the row and a cell, so that a new backup unit cannot be silent")
        if fn in HANDLED_SERVICES:
            vals = [v for _n, _s, k, v in keys if k == "OnFailure"]
            want = [HANDLED_SERVICES[fn]] if HANDLED_SERVICES[fn] else []
            if vals != want:
                fail("S5-21", rel, f"OnFailure= must be {'exactly one line naming ' + want[0] if want else 'absent'} (a backup that fails must never be silent, BK-04), not {vals!r}")
        if fn.endswith(".timer"):
            svc = fn[: -len(".timer")] + ".service"
            vals = [v for _n, _s, k, v in keys if k == "Unit"]
            if vals not in ([], [""], [svc]):
                fail("S5-21", rel, f"Unit= must be absent or {svc}, not {vals!r}")
        # the section rule comes AFTER the two above, which stay section-blind: their sentences are the ones the older cells (m226 to m235) expect
        for n, sec_name, k, _v in keys:
            if k == "OnFailure" and sec_name != "Unit":
                fail("S5-21", f"{rel}:{n}", f"OnFailure= belongs in [Unit] (systemd ignores it in [{sec_name}])")
            if k == "Unit" and fn.endswith(".timer") and sec_name != "Timer":
                fail("S5-21", f"{rel}:{n}", f"Unit= belongs in [Timer] (systemd ignores it in [{sec_name}])")

ROLES_BUDGET_MAX = 30
# THE RELATION (cox W1-55, mike's M-4), written ONCE: the guard of db-roles.sh refuses when the database clock is past E - LEAD, E being the CALLER's absolute deadline
# (the real adapter: its launch + the timeout; install.sh: now + 60). LEAD is the sum of three terms, each pinned exactly in the script:
ROLES_IDLE = 5            # a client that stalls between the guard and COMMIT is ended by idle_in_transaction_session_timeout
ROLES_COMMIT_LATENCY = 2  # the time COMMIT itself may take
ROLES_SLACK = 2           # what is left before E
ROLES_LEAD = 9
assert ROLES_LEAD == ROLES_IDLE + ROLES_COMMIT_LATENCY + ROLES_SLACK
# so the latest COMPLETED commit is E - ROLES_SLACK, whatever delayed the launch, the lock or the client. The engine's cell en_09 mirrors these numbers.
ROLES_TERMS = (("idle", ROLES_IDLE), ("latency", ROLES_COMMIT_LATENCY), ("slack", ROLES_SLACK))
ROLES_OPTION = '--env "PGOPTIONS=-c transaction_timeout=${budget}s -c idle_in_transaction_session_timeout=${idle}s -c db_roles.guard_by=${guard_by}"'
# the exec line, EXACTLY (hale 5i F-2: matched as a substring, the options could sit in a trailing comment; -1 or ON_ERROR_STOP or the database could change): ONE psql transaction on cortex
ROLES_EXEC_LINE = '} | podman exec -i --user postgres ' + ROLES_OPTION + ' "$db" psql -v ON_ERROR_STOP=1 -1 -U postgres -d cortex -q'
# a statement that ends or splits the transaction would let the guard run outside it (hale 5i F-1)
ROLES_TXN_WORDS = re.compile(r"\b(COMMIT|ROLLBACK|ABORT|SAVEPOINT)\b|\bEND\s*;|\bSTART\s+TRANSACTION\b", re.I)
ROLES_GUARD_LINE = "  IF extract(epoch FROM clock_timestamp()) > current_setting('db_roles.guard_by')::float8 THEN"
ROLES_LEAD_LINE = "lead=$((idle + latency + slack))"
ROLES_GUARD_BY_LINE = "guard_by=$((deadline - lead))"
ROLES_DEADLINE_LINE = "deadline=$2"
# the LAST statement of the roles SQL: the database refuses to commit past E - LEAD (now() would be the transaction's START; missing_ok would pass a missing setting)
ROLES_GUARD = (
    "  cat <<'SQL'\n"
    "DO $$ BEGIN\n"
    "  IF extract(epoch FROM clock_timestamp()) > current_setting('db_roles.guard_by')::float8 THEN\n"
    "    RAISE EXCEPTION 'db-roles: past its deadline; nothing was committed';\n"
    "  END IF;\n"
    "END $$;\n"
    "SQL\n"
)


def check_roles_budget(tree):
    """S5-22 the role script: the server-side budget, and the caller's deadline with the relation lead = idle + latency + slack (see the rule list above). Only when the tree has a db-roles.sh."""
    rel = "db-roles.sh"
    if not os.path.isfile(os.path.join(tree, rel)):
        return
    lines = read(tree, rel).splitlines()
    code = [(n, ln) for n, ln in enumerate(lines, 1) if not ln.lstrip().startswith("#")]
    budgets = [(n, re.fullmatch(r"budget=([0-9]+)(\s+#.*)?", ln)) for n, ln in code if ln.startswith("budget=")]
    if len(budgets) != 1 or budgets[0][1] is None:
        fail("S5-22", rel, "there must be exactly one line `budget=<seconds>` (a plain number)")
    elif not 1 <= int(budgets[0][1].group(1)) <= ROLES_BUDGET_MAX:
        fail("S5-22", f"{rel}:{budgets[0][0]}", f"the budget must be 1 to {ROLES_BUDGET_MAX} seconds (the engine's deadline is at least 45 s)")
    exec_lines = [(n, ln) for n, ln in code if "podman exec" in ln and "psql" in ln]
    if len(exec_lines) != 1 or exec_lines[0][1].count(ROLES_OPTION) != 1 or exec_lines[0][1].count("PGOPTIONS") != 1:
        fail("S5-22", rel, f"the psql session must be given exactly these options, once: {ROLES_OPTION}")
    if len(exec_lines) == 1 and exec_lines[0][1] != ROLES_EXEC_LINE:
        fail("S5-22", f"{rel}:{exec_lines[0][0]}", f"the exec line must be exactly: {ROLES_EXEC_LINE}")
    for n, ln in code:
        for opt in ("transaction_timeout", "idle_in_transaction_session_timeout", "db_roles.guard_by"):
            if opt in ln and (n, ln) not in exec_lines and not (opt == "db_roles.guard_by" and ln == ROLES_GUARD_LINE):
                fail("S5-22", f"{rel}:{n}", f"{opt} is set only through PGOPTIONS on the podman exec, never inside the SQL (the guard line is the only other place that names guard_by)")
        if ROLES_TXN_WORDS.search(ln):
            fail("S5-22", f"{rel}:{n}", "no statement may end or split the transaction (COMMIT, END, ROLLBACK, ABORT, SAVEPOINT, START TRANSACTION): the deadline guard is only the last statement of ONE transaction")
    if sum(ln.count("ON_ERROR_STOP") for _n, ln in code) != 1:
        fail("S5-22", rel, "ON_ERROR_STOP appears once, on the exec line (a psql meta-command that turns it off would let a failing guard end with exit status 0)")
    # the relation, ONCE: each term pinned exactly (a different number is a different relation, and the engine's en_09 would then be wrong)
    for name, value in ROLES_TERMS:
        found = [(n, ln) for n, ln in code if ln.startswith(name + "=")]
        if len(found) != 1 or found[0][1] != f"{name}={value}":
            fail("S5-22", rel, f"there must be exactly one line `{name}={value}`: {name} must be exactly {value} seconds (the relation lead = idle {ROLES_IDLE} + commit latency {ROLES_COMMIT_LATENCY} + slack {ROLES_SLACK} = {ROLES_LEAD} lives here, in the script's comment and in the engine's en_09)")
    for what, line in (("lead", ROLES_LEAD_LINE), ("guard_by", ROLES_GUARD_BY_LINE), ("deadline", ROLES_DEADLINE_LINE)):
        if [ln for _, ln in code].count(line) != 1:
            fail("S5-22", rel, f"there must be exactly one line `{line}` ({what}: the caller's deadline is the 2nd argument; the guard's limit is the deadline minus the lead)")
    # the script reads NO clock of its own for a deadline (mike's M-4: a start it read for itself could be later than the engine's launch)
    for n, ln in code:
        if re.search(r"\bdate\b", ln):
            fail("S5-22", f"{rel}:{n}", "db-roles.sh must not read the host clock: the CALLER passes its absolute deadline (--deadline)")
    # the order: the terms, then lead, then guard_by, all before the exec that hands guard_by to the session
    pos = {}
    for n, ln in code:
        for key, line in (("lead", ROLES_LEAD_LINE), ("guard_by", ROLES_GUARD_BY_LINE)):
            if ln == line:
                pos[key] = n
    if len(exec_lines) == 1 and "lead" in pos and "guard_by" in pos and not (pos["lead"] < pos["guard_by"] < exec_lines[0][0]):
        fail("S5-22", rel, "`lead=` then `guard_by=` must come before the podman exec (guard_by is handed to the session on that line)")
    # the guard is the LAST statement before the exec line (COMMIT follows it)
    text = "\n".join(lines) + "\n"
    at = text.find("\n} | podman exec")
    if at < 0 or not text[: at + 1].endswith(ROLES_GUARD):
        fail("S5-22", rel, "the roles SQL must END with the deadline guard, exactly (clock_timestamp(), current_setting without missing_ok, RAISE EXCEPTION): it is the last statement before COMMIT")


def check_never(root):
    for path in shell_files(root):
        rel = os.path.relpath(path, root)
        is_test = os.path.basename(path).endswith("-test.sh")
        with open(path, encoding="utf-8", errors="replace") as f:
            lines = f.read().splitlines()
        for n, line in enumerate(lines, 1):
            if line.lstrip().startswith("#"):
                continue
            m = ALLOW.search(line)
            if m:
                notes.append((rel, n, m.group(1).strip()))
                continue
            code = re.sub(r"\s+#(\s.*)?$", "", line)
            for name, rx, why in NEVER:
                if rx.search(code):
                    fail("S5-15", f"{rel}:{n}", f"{name}: {why}")
            # psql's `\set name `cat file`` reads a secret into a psql variable and prints nothing: the safe way, kept narrow.
            psql_assign = re.match(r"\s*\\set\s+\w+\s+`", code) is not None
            private = re.sub(r"\d*>&\d+", "", code)   # >&2 and 2>&1 send to the terminal: still a print
            if not is_test and not psql_assign and not re.search(r"[>|]", private):
                for name, rx, why in PRINT_CONTENT:
                    if rx.search(code):
                        fail("S5-15", f"{rel}:{n}", f"{name}: {why}")


# ---- main ------------------------------------------------------------------------------------------------------------

def report(label):
    for rel, n, why in notes:
        print(f"NOTE S5-15 {rel}:{n}: allowed: {why}")
    for rule, where, msg in infos:
        print(f"NOTE {rule} {where}: {msg}")
    for rule, where, msg in sorted(findings):
        print(f"FAIL {rule} {where}: {msg}")
    if findings:
        print(f"s5-check: {len(findings)} finding(s) in {label}")
        return 1
    return 0


def main(argv):
    if len(argv) == 3 and argv[1] == "--never-list":
        if not os.path.isdir(argv[2]):
            fail("S5-15", argv[2], "the directory does not exist")
        else:
            check_never(argv[2])
        rc = report(argv[2])
        if rc == 0:
            print(f"s5-check: OK (S5-15, {len(notes)} allowed line(s)) {argv[2]}")
        return rc
    if len(argv) != 2:
        print("usage: s5_check.py <deploy-dir> | --never-list <dir>", file=sys.stderr)
        return 2
    tree = argv[1]
    if not os.path.isdir(tree):
        fail("S5-14", tree, "the directory does not exist")
    else:
        for rel in REQUIRED:
            if not os.path.isfile(os.path.join(tree, rel)):
                fail("S5-14", rel, "required file is missing")

        if os.path.isfile(os.path.join(tree, "compose.yml")):
            try:
                c = yaml.safe_load(read(tree, "compose.yml"))
            except yaml.YAMLError as e:
                fail("S5-14", "compose.yml", f"does not parse: {str(e).splitlines()[0]}")
                c = None
            if c is not None:
                if not isinstance(c, dict) or not isinstance(c.get("services"), dict) or not c["services"]:
                    fail("S5-14", "compose.yml", "has no services")
                else:
                    check_names(c)
                    check_secrets(c)
                    for kind in ("configs", "secrets"):
                        for name, v in (c.get(kind) or {}).items():
                            if isinstance(v, dict) and (v.get("file") or v.get("environment")):
                                fail("S5-19", f"compose.yml {kind} {name}", "a top-level config or secret reads a host file or the environment; the mounts of a service are listed in the service")
                    check_owner_secret(c)
                    check_image_builder(c)
                    check_stack_env(tree, c["services"])
                    for sname, svc in c["services"].items():
                        check_service(sname, svc or {})

        if os.path.isfile(os.path.join(tree, "Containerfile")):
            check_containerfile(read(tree, "Containerfile"))
        check_server_features(tree)

        included = included_sources(tree)
        if included:
            print("NOTE S5-10: an oracle run in a fixed environment, not a proof of definitions only; F5 is open (vera 5W-REVIEW); the real includes are read by hand at their final hashes and bound by hash (G12).")
        read_includes = set()
        for dirpath, _dirs, files in os.walk(tree):
            for fn in sorted(files):
                if fn.endswith(".sh"):
                    rel = os.path.relpath(os.path.join(dirpath, fn), tree)
                    if rel in included:
                        check_included(tree, rel)
                        read_includes.add(rel)
                    else:
                        check_shell(tree, rel)

        if included:   # which shells ran (cox W1-70 point 5): the packet and every run say it
            sh_path = shutil.which("sh")
            if read_includes:
                print(f"NOTE S5-10: {len(read_includes)} included source(s) read by bash {ORACLE_VERSION} --posix -r in a namespace (xtrace oracle), and by `sh -n` = {os.path.realpath(sh_path) if sh_path else 'none'}; "
                      "dash is not covered (0.2.001 is Rocky Linux 10 only, where sh is bash)")
            else:
                print("NOTE S5-10: 0 included source(s) read; no oracle ran.")
            absent = sorted(rel for rel in included if not os.path.lexists(os.path.join(tree, rel)))
            unread = sorted(included - read_includes - set(absent))
            print("NOTE S5-10: named but absent: " + (", ".join(absent) if absent else "none"))
            if unread:
                print("NOTE S5-10: named but not read: " + ", ".join(unread))

        check_never(tree)
        check_systemd(tree)
        check_unit_handlers(tree)
        check_roles_budget(tree)

        if os.path.isfile(os.path.join(tree, "cortex.toml.example")):
            try:
                walk_toml(tomllib.loads(read(tree, "cortex.toml.example")), "cortex.toml.example")
            except tomllib.TOMLDecodeError as e:
                fail("S5-14", "cortex.toml.example", f"does not parse: {e}")

    rc = report(tree)
    if rc == 0:
        print(f"s5-check: OK (23 rules, {len(notes)} allowed line(s)) {tree}")
    return rc


def run_main(argv):
    """main() with its failures turned into exit codes: 2 = the checker could not run (never a pass), whatever the reason."""
    try:
        return main(argv)
    except SystemExit:
        raise
    except OracleUnavailable as e:  # S5-10 cannot read an included source here: the checker did not run
        print(f"s5-check: cannot run: {e}", file=sys.stderr)
        return 2
    except Exception as e:  # the checker failing is never a pass
        print(f"s5-check: internal error: {e!r}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(run_main(sys.argv))
