#!/usr/bin/python3 -I
"""Cells GR-nn of the 6f gate runner's own tests (increment R1).  Run as:  cv2-admit python3 -I test_gate_runner.py
Every run uses fakes first on PATH (ssh, hostname, ip, getent): no network, no real host.  Scratch lives under
GR_TMP (default: the data disk), never /tmp.  GR_RUNNER names another runner file (the mutant runner uses it).
Output: 'ok GR-nn ...' or 'FAIL GR-nn: why'; exit 1 when any cell failed."""
import hashlib, importlib.util, json, os, re, shutil, signal, subprocess, sys, tempfile, time

HERE = os.path.dirname(os.path.abspath(__file__))
RUNNER = os.environ.get("GR_RUNNER", os.path.join(HERE, "gate_runner.py"))
FAKES = os.path.join(HERE, "fakes")
TMP = os.environ.get("GR_TMP", os.path.expanduser("~/.cache/cortex-install-gate-tests"))
if TMP.startswith("/tmp"):
    sys.exit("refusing test scratch under /tmp")
os.makedirs(TMP, exist_ok=True)

# how the fake ssh recognises the three directory probes of the runner (the commands are in gate_runner.py: G1_DIRS, G1_UNITS, TRIAL_STATE)
DIRS_RULE = r'for r in "\.local/share/containers"'
UNITS_RULE = r'^w\(\) \{.*; w "\.config/systemd/user"; case '
STATE_RULE = r'^\{ w\(\) \{.*w "gate-trial-home"; case '

LOCAL_ID = "0123456789abcdef0123456789abcdef"
REMOTE_ID = "fedcba9876543210fedcba9876543210"
CHECKSUM = "a1" * 32
COMPOSE = "services:\n  db:\n    container_name: cortex-gate-db\n  migrate:\n    image: x\n    container_name: cortex-gate-migrate\n  server:\n    container_name: cortex-gate-server\n"
SERVER_IMAGE = "localhost/cortex-server@sha256:" + "c3" * 32
STACK_ENV = "CORTEX_VERSION=0.2.1\nCORTEX_SERVER_IMAGE=" + SERVER_IMAGE + "\nCORTEX_DB_IMAGE=docker.io/pgvector/pgvector@sha256:" + "d4" * 32 + "\n"
ORDER = ["G1", "G10", "G7", "G2", "G9", "G3", "G4", "G5", "G11", "G8", "G12", "G6"]
TS = re.compile(rb"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ\n$")


SANDBOXES = []


def sandbox():
    d = tempfile.mkdtemp(prefix="gr.", dir=TMP)
    SANDBOXES.append(d)
    os.makedirs(os.path.join(d, "art"))
    open(os.path.join(d, "machine-id"), "w").write(LOCAL_ID + "\n")
    return d


def rule(match, out="", rc=0, **kw):
    r = {"match": match, "out": out, "rc": rc}
    r.update(kw)
    return r


def clean_rules(podman=False):
    """What a clean Rocky 10.2 machine answers to the commands of G1 and G3."""
    r = [
        rule(r"^cat /etc/machine-id$", REMOTE_ID + "\n"),
        rule(r"^hostname$", "gate-vm\n"),
        rule(r"^cat /etc/os-release$", 'NAME="Rocky Linux"\nVERSION_ID="10.2"\n'),
        rule(r"^rpm -qa --qf", "bash 5.2-1.el10.x86_64\nrocky-release 10.2-1.el10.noarch\n"),
        rule(r"^ss -ltnH$", "LISTEN 0 128 0.0.0.0:22 0.0.0.0:*\n"),
        rule(r"^command -v podman$", "/usr/bin/podman\n" if podman else "", 0 if podman else 1),
        rule(r"^command -v cargo$", "", 1),
        rule(r"^command -v rustc$", "", 1),
        rule(DIRS_RULE, "absent /home/gate/.local/share/containers\nabsent /home/gate/.config/containers\n"),
        rule(UNITS_RULE, "@@user_units absent\n"),
        rule(r"^sha256sum /etc/pki/rpm-gpg/RPM-GPG-KEY-Rocky-", "b2" * 32 + "  /etc/pki/rpm-gpg/RPM-GPG-KEY-Rocky-10\n"),
        rule(r"^rpm -q rocky-release rocky-gpg-keys", "rocky-release-10.2-1.el10.noarch\nrocky-gpg-keys-10.2-1.el10.noarch\n"),
        rule(r"^rpm -qa 'gpg-pubkey\*'", "gpg-pubkey-6d745a60-5f7e5d1e Rocky Enterprise Software Foundation\n"),
        rule(r"^curl -fsS --max-time 10 http://127.0.0.1:\d+/v2/health/ready$", '{"ready": true}\n'),
        rule(r'^cat "\$HOME/cortex-v2/current/compose.yml"$', COMPOSE),
        rule(r'^cat "\$HOME/cortex-v2/current/stack.env"$', STACK_ENV),
        rule(r"^systemctl --user is-enabled ", "enabled\nenabled\n"),
        rule(r"^systemctl --user is-active ", "active\nactive\n"),
        rule(r"^loginctl show-user ", "Linger=yes\nState=lingering\n"),
        rule(r"^systemctl --user list-timers ", "Sun 2026-11-08 11:00:00 UTC 9h left cortex-dump.timer cortex-dump.service\nSun 2026-11-08 02:00:00 UTC 1h left cortex-dump-status.timer cortex-dump-status.service\n"),
        rule(r"^loginctl list-sessions ", "12 1000 gate seat0\n"),
        rule(r"^podman logs cortex-gate-(db|migrate|server)$", "LOG: ready (image sha256:" + "d4" * 32 + ")\n"),
        rule(r"^podman inspect --format ", "0 " + SERVER_IMAGE + "\n"),
    ]
    return r


def base_local():
    return {"hostname": "build-host-1", "addrs": ["10.0.0.4", "fe80::1"],
            "resolve": {"gate-vm": ["10.9.9.9"], "10.9.9.9": ["10.9.9.9"], "alias-of-local": ["10.0.0.4"],
                        "127.0.0.1": ["127.0.0.1"], "::1": ["::1"], "localhost": ["127.0.0.1"],
                        "build-host-1": ["10.0.0.4"], "build-host-2": ["10.0.0.5"]}}


def base_args(sb, target="gate@gate-vm", extra=None, only="G1,G3"):
    a = ["--target", target, "--evidence-dir", os.path.join(sb, "ev"), "--artefacts", os.path.join(sb, "art"),
         "--project", "gate", "--lead", "gate-lead", "--image-name", "Rocky-10-GenericCloud-Base",
         "--image-version", "10.2-20261001.0", "--image-checksum", CHECKSUM,
         "--image-source", "https://example.invalid/rocky/images", "--local-machine-id", os.path.join(sb, "machine-id")]
    if only:
        a += ["--only", only]
    return a + ["--install-timeout", "20"] + (extra or [])


def run(sb, args, rules=None, local=None, env_extra=None, python_flags=("-I",), timeout=60, runner=None):
    json.dump({"rules": rules if rules is not None else clean_rules()}, open(os.path.join(sb, "scenario.json"), "w"))
    json.dump(local if local is not None else base_local(), open(os.path.join(sb, "local.json"), "w"))
    env = {"PATH": FAKES + ":/usr/bin:/bin", "FAKE_LOG": os.path.join(sb, "fake.log"),
           "FAKE_SCENARIO": os.path.join(sb, "scenario.json"), "FAKE_LOCAL": os.path.join(sb, "local.json"),
           "FAKE_CONTAINERS": os.path.join(sb, "containers.json"), "CV2_ADMITTED": "1", "HOME": sb}
    if not os.path.exists(os.path.join(sb, "containers.json")):
        json.dump(["gate-rocky10"], open(os.path.join(sb, "containers.json"), "w"))
    env.update(env_extra or {})
    return subprocess.run(["/usr/bin/python3", *python_flags, runner or RUNNER, *args], env=env,
                          capture_output=True, timeout=timeout)


def fake_log(sb):
    p = os.path.join(sb, "fake.log")
    if not os.path.exists(p):
        return []
    return [json.loads(l) for l in open(p) if l.strip()]


def ssh_commands(sb):
    return [e["argv"][-1] for e in fake_log(sb) if e.get("tool") == "ssh" and "argv" in e]


def tsv(sb):
    rows = {}
    for line in open(os.path.join(sb, "ev", "verdict.tsv")):
        f = line.rstrip("\n").split("\t")
        rows[f[0]] = f
    return rows


def sha(path):
    return hashlib.sha256(open(path, "rb").read()).hexdigest()


def check(cond, msg):
    if not cond:
        raise AssertionError(msg)


def load_module():
    spec = importlib.util.spec_from_file_location("gate_runner_under_test", RUNNER)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


CELLS = []


def cell(name):
    def deco(fn):
        CELLS.append((name, fn))
        return fn
    return deco


@cell("GR-01 a fake clean machine: G1 and G3 pass, the other ten are NOT RUN, overall INCOMPLETE (exit 3), table in G order")
def gr01():
    sb = sandbox()
    p = run(sb, base_args(sb))
    check(p.returncode == 3, "exit %s, wanted 3 (INCOMPLETE): %s" % (p.returncode, p.stderr[-300:]))
    rows = tsv(sb)
    check(list(rows) == ["G%d" % i for i in range(1, 13)], "table rows %s" % list(rows))
    check(rows["G1"][1] == "PASS" and rows["G3"][1] == "PASS", "G1 %s G3 %s (%s)" % (rows["G1"][1:3], rows["G3"][1:3], p.stdout[-400:]))
    check(all(rows[g][1] == "NOT RUN" for g in rows if g not in ("G1", "G3")), "an unbuilt or unselected check is not NOT RUN: %s" % {g: rows[g][1] for g in rows})
    out = p.stdout.decode()
    check("OVERALL: INCOMPLETE" in out, "no OVERALL line: %s" % out[-300:])
    check(os.path.join(sb, "ev") in out, "the evidence directory is not printed")
    check(re.search(r"^G1\s+PASS", out, re.M) is not None, "no table line for G1")
    insp = [c for c in ssh_commands(sb) if c.startswith("podman inspect")]
    check(insp and insp[0].endswith(" cortex-gate-migrate"), "the migrate container was not taken from the release's compose: %s" % insp)
    check(not any("cv2-s5" in c for c in ssh_commands(sb)), "a hard-coded cv2-s5 name was sent")


def refused(sb, target, code, calls, rules=None, local=None, why=""):
    p = run(sb, base_args(sb, target=target), rules=rules, local=local)
    check(p.returncode == code, "%s: exit %s, wanted %s: %s" % (why or target, p.returncode, code, p.stderr[-200:]))
    n = len(ssh_commands(sb))
    check(n == calls, "%s: %d ssh calls, wanted %d (%s)" % (why or target, n, calls, ssh_commands(sb)))
    ev = os.path.join(sb, "ev")
    if calls == 0:
        check(not os.path.exists(ev), "%s: an evidence directory was created for a target refused before any connection" % (why or target))
    else:
        names = os.listdir(ev)
        check("REFUSED.txt" in names and "SHA256SUMS" in names, "%s: a refusal after the identity probe keeps REFUSED.txt and SHA256SUMS: %s" % (why or target, names))
        check(not any(re.match(r"\d\d-G\d", n) for n in names), "%s: a check ran after the refusal: %s" % (why or target, names))


@cell("GR-02a the target NAME is this host (build-host-1, build-host-2, localhost, its own short name): exit 10, no ssh call")
def gr02a():
    for t in ("gate@build-host-1", "build-host-2", "gate@localhost", "gate@BUILD-HOST-1.example.internal"):
        refused(sandbox(), t, 10, 0, why=t)


@cell("GR-02b the target ADDRESS is one of this host's own (a second name, loopback, ::1): exit 11, no ssh call")
def gr02b():
    for t in ("gate@alias-of-local", "gate@127.0.0.1", "gate@::1"):
        refused(sandbox(), t, 11, 0, why=t)


@cell("GR-02c the machine-id read over ssh equals this host's: exit 12 after exactly ONE ssh call")
def gr02c():
    r = clean_rules()
    r[0] = rule(r"^cat /etc/machine-id$", LOCAL_ID + "\n")
    refused(sandbox(), "gate@gate-vm", 12, 1, rules=r, why="machine-id")


@cell("GR-02d the hostname read over ssh equals this host's: exit 13 after exactly TWO ssh calls")
def gr02d():
    r = clean_rules()
    r[1] = rule(r"^hostname$", "build-host-1\n")
    refused(sandbox(), "gate@gate-vm", 13, 2, rules=r, why="hostname")


@cell("GR-02e the target name equals this host's own name when that name is not a protected one: exit 10, no ssh call")
def gr02e():
    loc = base_local()
    loc["hostname"] = "build-box-7"
    loc["resolve"]["build-box-7"] = ["10.9.9.9"]
    sb = sandbox()
    p = run(sb, base_args(sb, target="gate@build-box-7.example.internal"), local=loc)
    check(p.returncode == 10 and not ssh_commands(sb), "exit %s, ssh calls %s" % (p.returncode, ssh_commands(sb)))


@cell("GR-02f a machine-id or hostname the target does not give is a refusal (fail closed): exit 12 after 1 call, exit 13 after 2 calls")
def gr02f():
    r = clean_rules()
    r[0] = rule(r"^cat /etc/machine-id$", "", 1)
    refused(sandbox(), "gate@gate-vm", 12, 1, rules=r, why="no machine-id")
    r = clean_rules()
    r[1] = rule(r"^hostname$", "", 1)
    refused(sandbox(), "gate@gate-vm", 13, 2, rules=r, why="no hostname")


@cell("GR-02g this host's address list cannot be read or is empty: exit 11, no ssh call (fail closed)")
def gr02g():
    loc = base_local()
    loc["addrs"] = []
    refused(sandbox(), "gate@gate-vm", 11, 0, local=loc, why="no local addresses")


@cell("GR-03 a failed check does not hide the next: podman present fails G1, every G1 step still ran, G3 still passes, exit 1")
def gr03():
    sb = sandbox()
    p = run(sb, base_args(sb), rules=clean_rules(podman=True))
    check(p.returncode == 1, "exit %s, wanted 1" % p.returncode)
    rows = tsv(sb)
    check(rows["G1"][1] == "FAIL" and "podman" in rows["G1"][2], "G1 row %s" % rows["G1"])
    check(rows["G3"][1] == "PASS", "G3 was hidden or failed after G1 failed: %s" % rows["G3"])
    check("command -v rustc" in ssh_commands(sb), "a G1 step after the failing one did not run: %s" % ssh_commands(sb))


@cell("GR-04 an exception inside a check is a FAIL with the traceback kept as a receipt; the next check still runs")
def gr04():
    mod = load_module()
    sb = sandbox()
    rec = mod.Recorder(os.path.join(sb, "ev"))
    def boom(ctx):
        raise RuntimeError("planted failure")
    def fine(ctx):
        return ("PASS", "fine", [])
    results = mod.execute([("G1", boom), ("G3", fine)], ["G1", "G3"], rec, ctx=None)
    check(results["G1"][0] == "FAIL", "exception became %s" % (results["G1"],))
    check(results["G3"][0] == "PASS", "the check after the exception did not run")
    tb = [f for f in os.listdir(os.path.join(sb, "ev")) if f.endswith(".traceback")]
    check(tb and b"planted failure" in open(os.path.join(sb, "ev", tb[0]), "rb").read(), "no traceback receipt: %s" % os.listdir(os.path.join(sb, "ev")))


@cell("GR-05 SIGTERM mid-run: exit 130, the table so far with INTERRUPTED, the rest NOT RUN, SHA256SUMS verifies, the ssh child is gone")
def gr05():
    sb = sandbox()
    r = clean_rules()
    pidfile = os.path.join(sb, "ssh.pid")
    for i, x in enumerate(r):
        if x["match"] == r"^ss -ltnH$":
            r[i] = rule(r"^ss -ltnH$", "x\n", sleep=30, pidfile=pidfile)
    json.dump({"rules": r}, open(os.path.join(sb, "scenario.json"), "w"))
    json.dump(base_local(), open(os.path.join(sb, "local.json"), "w"))
    env = {"PATH": FAKES + ":/usr/bin:/bin", "FAKE_LOG": os.path.join(sb, "fake.log"), "FAKE_SCENARIO": os.path.join(sb, "scenario.json"),
           "FAKE_LOCAL": os.path.join(sb, "local.json"), "HOME": sb}
    proc = subprocess.Popen(["/usr/bin/python3", "-I", RUNNER, *base_args(sb)], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    end = time.time() + 20
    while not os.path.exists(pidfile) and time.time() < end:
        time.sleep(0.05)
    check(os.path.exists(pidfile), "the fake ssh never reached the sleeping step")
    child = int(open(pidfile).read())
    proc.send_signal(signal.SIGTERM)
    out, err = proc.communicate(timeout=30)
    check(proc.returncode == 130, "exit %s, wanted 130: %s" % (proc.returncode, err[-300:]))
    rows = tsv(sb)
    check("INTERRUPTED" in open(os.path.join(sb, "ev", "verdict.txt")).read(), "verdict.txt does not say INTERRUPTED")
    check(rows["G12"][1] == "NOT RUN", "an unreached check is %s" % rows["G12"][1])
    check(rows["G1"][1] in ("FAIL", "NOT RUN"), "the interrupted check is %s, it must not pass" % rows["G1"][1])
    v = subprocess.run(["sha256sum", "-c", "SHA256SUMS"], cwd=os.path.join(sb, "ev"), capture_output=True)
    check(v.returncode == 0, "SHA256SUMS does not verify after an interrupt: %s" % v.stdout[-200:])
    time.sleep(0.3)
    gone = True
    try:
        os.kill(child, 0)
        gone = False
    except OSError:
        pass
    check(gone and os.path.exists(pidfile + ".term"), "the ssh child %d was not terminated by the runner" % child)


@cell("GR-07 receipts are byte for byte (non-UTF-8, CRLF, trailing spaces), timed in UTC, and SHA256SUMS lists them all and verifies")
def gr07():
    sb = sandbox()
    raw = b'NAME="Rocky"\r\n\xff\xfe  \n'
    r = [x for x in clean_rules() if x["match"] != r"^cat /etc/os-release$"]
    r.append({"match": r"^cat /etc/os-release$", "out_hex": raw.hex(), "rc": 0})
    p = run(sb, base_args(sb), rules=r)
    ev = os.path.join(sb, "ev")
    outs = sorted(f for f in os.listdir(ev) if f.endswith("-os_release.out") or "os_release" in f and f.endswith(".out"))
    check(outs, "no receipt for os_release: %s" % sorted(os.listdir(ev)))
    base = os.path.join(ev, outs[0][:-4])
    check(open(base + ".out", "rb").read() == raw, "the .out receipt is not the exact bytes: %r" % open(base + ".out", "rb").read())
    check(open(base + ".cmd", "rb").read().rstrip(b"\n") == b"cat /etc/os-release", "the .cmd is %r" % open(base + ".cmd", "rb").read())
    check(open(base + ".rc", "rb").read() == b"0\n", ".rc is %r" % open(base + ".rc", "rb").read())
    for ext in (".t0", ".t1"):
        check(TS.match(open(base + ext, "rb").read()), "%s is %r" % (ext, open(base + ext, "rb").read()))
    listed = [l.split("  ", 1)[1] for l in open(os.path.join(ev, "SHA256SUMS")).read().splitlines()]
    files = sorted(os.path.relpath(os.path.join(d, f), ev) for d, _, fs in os.walk(ev) for f in fs if f != "SHA256SUMS")
    check(sorted(listed) == files, "SHA256SUMS lists %d files, the directory holds %d: missing %s" % (len(listed), len(files), sorted(set(files) - set(listed))[:3]))
    v = subprocess.run(["sha256sum", "-c", "SHA256SUMS"], cwd=ev, capture_output=True)
    check(v.returncode == 0, "sha256sum -c failed: %s" % v.stdout[-200:])
    newest = max(os.stat(os.path.join(ev, f)).st_mtime_ns for f in files)
    check(os.stat(os.path.join(ev, "SHA256SUMS")).st_mtime_ns >= newest, "SHA256SUMS was not written last")
    with open(base + ".out", "ab") as fh:
        fh.write(b"x")
    v = subprocess.run(["sha256sum", "-c", "SHA256SUMS"], cwd=ev, capture_output=True)
    check(v.returncode != 0, "a changed receipt still verifies")


@cell("GR-08 the runner never records its own environment or any secret-looking argument: a planted variable is not under the evidence")
def gr08():
    sb = sandbox()
    p = run(sb, base_args(sb), env_extra={"GATE_TEST_TOKEN": "ip54-secret-token-marker", "SSH_AUTH_SOCK": "/nonexistent/ip54-secret-socket"})
    check(p.returncode == 3, "exit %s" % p.returncode)
    hit = subprocess.run(["grep", "-r", "-l", "-e", "ip54-secret-token-marker", "-e", "ip54-secret-socket", os.path.join(sb, "ev")], capture_output=True)
    check(hit.returncode == 1, "the environment reached the evidence: %s" % hit.stdout)
    check(b"ip54-secret" not in p.stdout + p.stderr, "the environment reached the terminal")


@cell("GR-16 a container target must be ONE exact name: patterns, -a, live names and spaces are refused with exit 14; a plain name is not refused")
def gr16():
    for name in ("cortex-*", "-a", "cortex-db", "cv2-s5-db", "spike-s0-x", "harness-appdb", "a b", "", "gate*", "gate/x", "$(id)"):
        sb = sandbox()
        a = base_args(sb, only=None)
        i = a.index("--target")
        del a[i:i + 2]
        p = run(sb, a + ["--container=" + name])
        check(p.returncode == 14, "container %r: exit %s, wanted 14: %s" % (name, p.returncode, p.stderr[-200:]))
        check(not ssh_commands(sb), "container %r reached ssh" % name)
    sb = sandbox()
    a = base_args(sb, only=None)
    i = a.index("--target")
    del a[i:i + 2]
    p = run(sb, a + ["--container", "-a"])
    check(p.returncode in (2, 14) and not ssh_commands(sb), "the two-argument form '--container -a': exit %s" % p.returncode)
    sb = sandbox()
    a = base_args(sb, only=None)
    i = a.index("--target")
    del a[i:i + 2]
    p = run(sb, a + ["--container", "gate-rocky10"])
    check(p.returncode != 14, "an exact plain name was refused as a pattern")


@cell("GR-17 every command sent over ssh is on the runner's allow-list; ssh never prompts (BatchMode); the fake saw no unmatched command")
def gr17():
    mod = load_module()
    sb = sandbox()
    run(sb, base_args(sb))
    cmds = ssh_commands(sb)
    check(cmds, "no ssh command was sent")
    bad = [c for c in cmds if not any(p.match(c) for p in mod.ALLOWED)]
    check(not bad, "commands not on the allow-list: %s" % bad)
    check(not any("UNMATCHED" in e for e in fake_log(sb)), "the fake had no rule for: %s" % [e for e in fake_log(sb) if "UNMATCHED" in e])
    for e in fake_log(sb):
        if "argv" in e:
            check("BatchMode=yes" in e["argv"], "ssh without BatchMode=yes: %s" % e["argv"])
    check(mod.command_allowed("rm -rf /") is False and mod.command_allowed("cat /etc/machine-id") is True, "command_allowed() is wrong")


@cell("GR-17b a command that is not on the allow-list is refused BEFORE ssh is called; a listed one reaches ssh")
def gr17b():
    mod = load_module()
    calls = []
    class FakeSsh:
        def run(self, cmd, timeout=120, stdin_path=None):
            calls.append(cmd)
            return 0, b"", b""
    sb = sandbox()
    rec = mod.Recorder(os.path.join(sb, "ev"))
    ctx = mod.Ctx(FakeSsh(), rec, None)
    mod.COMMANDS["evil"] = ("rm -rf /", r"echo harmless")
    try:
        ctx.step("G1", "evil", "evil")
        raised = False
    except mod.Refused:
        raised = True
    check(raised and not calls, "an unlisted command reached ssh: raised=%s calls=%s" % (raised, calls))
    ctx.step("G1", "ok", "g1_ports")
    check(calls == ["ss -ltnH"], "a listed command did not reach ssh: %s" % calls)


@cell("GR-22 G3 fails on ready false, broken JSON, a failed request, a non-zero migrate exit, and a compose with no usable migrate name; G1 is unaffected")
def gr22():
    cases = {"ready false": ("ready", '{"ready": false}\n', 0), "broken json": ("ready", "<html>502</html>\n", 0),
             "request failed": ("ready", "", 22), "migrate 1": ("migrate", "1 localhost/cortex-server@sha256:" + "c3" * 32 + "\n", 0),
             "migrate rc": ("migrate", "", 1),
             "compose without migrate": ("compose", "services:\n  db:\n    container_name: x\n", 0),
             "compose unsafe name": ("compose", "services:\n  migrate:\n    container_name: 'bad name'\n", 0),
             "compose shell characters": ("compose", "services:\n  migrate:\n    container_name: x;touch\n", 0),
             "compose partial output": ("compose", COMPOSE, 1),
             "compose unreadable": ("compose", "", 1)}
    for name, (which, out, rc) in cases.items():
        sb = sandbox()
        r = clean_rules()
        for i, x in enumerate(r):
            if (which == "ready" and "health/ready" in x["match"]) or (which == "migrate" and "podman inspect" in x["match"]) or (which == "compose" and "compose.yml" in x["match"]):
                r[i] = rule(x["match"], out, rc)
        p = run(sb, base_args(sb), rules=r)
        rows = tsv(sb)
        check(p.returncode == 1 and rows["G3"][1] == "FAIL", "%s: exit %s, G3 %s" % (name, p.returncode, rows["G3"]))
        check(rows["G1"][1] == "PASS", "%s: G1 was affected" % name)
        if which == "compose":
            check(not any(c.startswith("podman inspect") for c in ssh_commands(sb)), "%s: a container was inspected without a usable name" % name)
            check("no usable migrate container" in rows["G3"][2], "%s: the verdict does not name the cause itself (a later guard caught it?): %s" % (name, rows["G3"][2]))


@cell("GR-18a python3 -I: a planted json.py beside the runner is not imported; GR-18b the runner refuses to run without -I (exit 2)")
def gr18():
    sb = sandbox()
    d = os.path.join(sb, "copy")
    os.makedirs(d)
    shutil.copy(RUNNER, os.path.join(d, "gate_runner.py"))
    marker = os.path.join(sb, "json-imported")
    open(os.path.join(d, "json.py"), "w").write("open(%r, 'w').write('x')\nraise ImportError('planted')\n" % marker)
    p = run(sb, base_args(sb), runner=os.path.join(d, "gate_runner.py"))
    check(not os.path.exists(marker), "a planted json.py beside the runner was imported")
    check(p.returncode == 3, "the copy did not run normally: exit %s %s" % (p.returncode, p.stderr[-200:]))
    q = run(sandbox(), [], python_flags=())
    check(q.returncode == 2 and b"-I" in q.stderr, "without -I: exit %s, stderr %r" % (q.returncode, q.stderr[-200:]))


@cell("GR-19 evidence directory: required, must not exist, not under /tmp; created mode 0700; bad usage is exit 2 and runs nothing")
def gr19():
    sb = sandbox()
    a = base_args(sb)
    i = a.index("--evidence-dir")
    p = run(sb, a[:i] + a[i + 2:])
    check(p.returncode == 2, "no --evidence-dir: exit %s" % p.returncode)
    os.makedirs(os.path.join(sb, "ev"))
    p = run(sb, base_args(sb))
    check(p.returncode == 2 and not ssh_commands(sb), "an existing evidence directory: exit %s, ssh calls %s" % (p.returncode, ssh_commands(sb)))
    under_tmp = "/tmp/gr-never-created-" + os.path.basename(sb)
    try:
        p = run(sb, base_args(sb)[:i + 1] + [under_tmp] + base_args(sb)[i + 2:])
        check(p.returncode == 2 and not os.path.exists(under_tmp), "an evidence directory under /tmp: exit %s, created=%s" % (p.returncode, os.path.exists(under_tmp)))
    finally:
        if os.path.isdir(under_tmp):
            shutil.rmtree(under_tmp)
    sb2 = sandbox()
    run(sb2, base_args(sb2))
    check(oct(os.stat(os.path.join(sb2, "ev")).st_mode & 0o777) == "0o700", "evidence directory mode %s" % oct(os.stat(os.path.join(sb2, "ev")).st_mode & 0o777))
    p = run(sandbox(), ["--unknown-flag"])
    check(p.returncode == 2, "an unknown flag: exit %s" % p.returncode)


@cell("GR-21 G1 records the image identity and FAILS on a missing or malformed field; the next check still runs")
def gr21():
    sb = sandbox()
    a = base_args(sb)
    i = a.index("--image-checksum")
    a[i + 1] = "not-a-sha256"
    p = run(sb, a)
    rows = tsv(sb)
    check(p.returncode == 1 and rows["G1"][1] == "FAIL" and "checksum" in rows["G1"][2].lower(), "G1 row %s exit %s" % (rows["G1"], p.returncode))
    check(rows["G3"][1] == "PASS", "G3 hidden after G1 failed")
    sb = sandbox()
    p = run(sb, base_args(sb))
    ident = [f for f in os.listdir(os.path.join(sb, "ev")) if "identity" in f and f.endswith(".out")]
    data = open(os.path.join(sb, "ev", ident[0])).read() if ident else ""
    for want in ("Rocky-10-GenericCloud-Base", "10.2-20261001.0", CHECKSUM, "https://example.invalid/rocky/images"):
        check(want in data, "the identity record lacks %r (files %s)" % (want, ident))



# ---- increment R2: G2 (upload, install in a PTY) and G9 (questions) -----------------------------------------------------------
ART_FILES = {"install.sh": b"#!/bin/sh\necho installer\n", "cortex-0.2.1-x86_64-linux.tar.gz": b"\x1f\x8b tar bytes",
             "manifest.json": b"{}\n", "SHA256SUMS": b"x\n", "SHA256SUMS.sig": b"sig\n"}
INSTALL_CMD = 'sh "$HOME/gate-artefacts/install.sh" --from "$HOME/gate-artefacts" --project gate --lead gate-lead'
CONSENT = "Proceed with these changes? [y/N] "


def g2_sandbox(files=None, wrong_hash=None, install=None, rc=0):
    sb = sandbox()
    files = ART_FILES if files is None else files
    art = os.path.join(sb, "art")
    for n, data in files.items():
        open(os.path.join(art, n), "wb").write(data)
    os.makedirs(os.path.join(sb, "remote"))
    r = [x for x in clean_rules()]
    sums = ""
    for n, data in files.items():
        h = hashlib.sha256(data).hexdigest()
        if n == wrong_hash:
            h = "0" * 64
        sums += "%s  /home/gate/gate-artefacts/%s\n" % (h, n)
    extra = [rule("^" + re.escape('mkdir -m 0700 "$HOME/gate-artefacts"') + "$", "")]
    for n in files:
        if re.fullmatch(r"[A-Za-z0-9._-]{1,128}", n):
            extra.append(rule("^" + re.escape('cat > "$HOME/gate-artefacts/%s"' % n) + "$", "", save_stdin=os.path.join(sb, "remote", n)))
    extra.append(rule("^" + re.escape('sha256sum "$HOME"/gate-artefacts/*') + "$", sums))
    steps = install if install is not None else [{"print": "install: looking at the machine\r\n"}, {"print": CONSENT, "read": True}, {"print": "install: done\r\n"}]
    extra.append(rule("^" + re.escape(INSTALL_CMD) + "$", "", rc, pty_steps=steps))
    return sb, r + extra


def answers(sb):
    return [e["answer"] for e in fake_log(sb) if e.get("tool") == "ssh" and "answer" in e]


@cell("GR-23 G2 uploads and verifies the artefacts (sha256 on both sides), runs the install in a PTY and answers the ONE consent question; G9 counts 1 and passes")
def gr23():
    sb, r = g2_sandbox()
    p = run(sb, base_args(sb, only="G1,G2,G9,G3"), rules=r)
    rows = tsv(sb)
    check(rows["G2"][1] == "PASS", "G2 %s (%s)" % (rows["G2"], p.stdout[-300:]))
    check(rows["G9"][1] == "PASS" and "1 question" in rows["G9"][2], "G9 %s" % (rows["G9"],))
    for n, data in ART_FILES.items():
        check(open(os.path.join(sb, "remote", n), "rb").read() == data, "the upload of %s differs" % n)
    check(answers(sb) == ["y\n"], "the answers typed were %r" % answers(sb))
    cmds = ssh_commands(sb)
    check(cmds.index(INSTALL_CMD) > max(i for i, c in enumerate(cmds) if c.startswith("sha256sum")), "the install ran before the upload was verified")
    ev = os.path.join(sb, "ev")
    out = [f for f in os.listdir(ev) if f.endswith("-install.out")]
    check(out and b"Proceed with these changes?" in open(os.path.join(ev, out[0]), "rb").read(), "the raw transcript was not kept")
    ans = [f for f in os.listdir(ev) if f.endswith("-install.answers")]
    ev_json = json.load(open(os.path.join(ev, ans[0]))) if ans else []
    check(len(ev_json) == 1 and ev_json[0]["answer"] == "y" and ev_json[0]["kind"] == "consent", "the answers receipt is %s" % ev_json)


@cell("GR-24 two questions: G9 FAILS with the count stated, G2 still passes (the install exited 0)")
def gr24():
    sb, r = g2_sandbox(install=[{"print": CONSENT, "read": True}, {"print": "Really? [y/N] ", "read": True}, {"print": "done\r\n"}])
    run(sb, base_args(sb, only="G1,G2,G9,G3"), rules=r)
    rows = tsv(sb)
    check(rows["G2"][1] == "PASS" and rows["G9"][1] == "FAIL" and "2 questions" in rows["G9"][2], "G2 %s G9 %s" % (rows["G2"][1:3], rows["G9"][1:3]))


@cell("GR-25 a password prompt: the runner sends end of input, never text; G2 and G9 FAIL; the later checks still run")
def gr25():
    sb, r = g2_sandbox(install=[{"print": "[sudo] password for gate: ", "read": True, "eof_rc": 1}, {"print": "never\r\n"}], rc=1)
    run(sb, base_args(sb, only="G1,G2,G9,G3"), rules=r)
    rows = tsv(sb)
    check(rows["G2"][1] == "FAIL" and "password" in rows["G2"][2].lower(), "G2 %s" % (rows["G2"],))
    check(rows["G9"][1] == "FAIL" and "password" in rows["G9"][2].lower(), "G9 %s" % (rows["G9"],))
    check(answers(sb) == [""], "the runner typed %r at a password prompt (wanted end of input only)" % answers(sb))
    check(rows["G3"][1] == "PASS", "G3 was hidden")


@cell("GR-26 no question at all: G9 passes with 0; GR-27 an install that exits 1: G2 FAILS, the transcript is kept, G9 is still counted, G3 still runs")
def gr26():
    sb, r = g2_sandbox(install=[{"print": "install: done\r\n"}])
    run(sb, base_args(sb, only="G1,G2,G9,G3"), rules=r)
    rows = tsv(sb)
    check(rows["G9"][1] == "PASS" and "0 questions" in rows["G9"][2], "G9 %s" % (rows["G9"],))
    sb, r = g2_sandbox(install=[{"print": CONSENT, "read": True}, {"print": "problem: x\r\nfix: y\r\n"}], rc=3)
    p = run(sb, base_args(sb, only="G1,G2,G9,G3"), rules=r)
    rows = tsv(sb)
    check(p.returncode == 1 and rows["G2"][1] == "FAIL" and "3" in rows["G2"][2], "G2 %s exit %s" % (rows["G2"], p.returncode))
    check(rows["G9"][1] == "PASS" and "1 question" in rows["G9"][2], "G9 %s" % (rows["G9"],))
    check(rows["G3"][1] == "PASS", "G3 was hidden after a failed install")
    out = [f for f in os.listdir(os.path.join(sb, "ev")) if f.endswith("-install.out")]
    check(out and b"problem: x" in open(os.path.join(sb, "ev", out[0]), "rb").read(), "the failed transcript was not kept")


@cell("GR-28 an upload whose remote sha256 differs: G2 FAILS and the install command is NEVER sent; G9 is NOT RUN")
def gr28():
    sb, r = g2_sandbox(wrong_hash="manifest.json")
    run(sb, base_args(sb, only="G1,G2,G9,G3"), rules=r)
    rows = tsv(sb)
    check(rows["G2"][1] == "FAIL" and "manifest.json" in rows["G2"][2], "G2 %s" % (rows["G2"],))
    check(INSTALL_CMD not in ssh_commands(sb), "the install ran on an unverified upload")
    check(rows["G9"][1] == "NOT RUN", "G9 %s" % (rows["G9"],))


@cell("GR-29 command hygiene: no variable set by hand, no --yes (it would hide the questions), ssh sends no environment (SendEnv=-*), and an unsafe artefact name is refused before any upload")
def gr29():
    sb, r = g2_sandbox()
    run(sb, base_args(sb, only="G2"), rules=r)
    check(INSTALL_CMD in ssh_commands(sb), "the install command was not the plain one: %s" % ssh_commands(sb)[-2:])
    check(not re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", INSTALL_CMD) and "--yes" not in INSTALL_CMD, "test constant is wrong")
    for e in fake_log(sb):
        if "argv" in e:
            check("SendEnv=-*" in e["argv"], "ssh may send the environment: %s" % e["argv"])
    sb, r = g2_sandbox(files={"install.sh": b"x\n", "a b": b"y\n"})
    run(sb, base_args(sb, only="G2"), rules=r)
    check(tsv(sb)["G2"][1] == "FAIL" and not [c for c in ssh_commands(sb) if c.startswith("cat >")], "an unsafe name reached an upload: %s" % ssh_commands(sb))
    check("unsafe" in tsv(sb)["G2"][2], "the refusal does not name the unsafe entry itself: %s" % (tsv(sb)["G2"],))
    sb, r = g2_sandbox(files={"x": b"y\n"})
    run(sb, base_args(sb, only="G2"), rules=r)
    check(tsv(sb)["G2"][1] == "FAIL" and "install.sh" in tsv(sb)["G2"][2], "a release without install.sh passed: %s" % (tsv(sb)["G2"],))


@cell("GR-30 the transcript is raw (ANSI, CRLF) and a prompt wrapped in ANSI codes is still found")
def gr30():
    sb, r = g2_sandbox(install=[{"print": "\x1b[1minstall\x1b[0m: ok\r\n"}, {"print": "\x1b[33mProceed with these changes? [y/N]\x1b[0m ", "read": True}, {"print": "done\r\n"}])
    run(sb, base_args(sb, only="G1,G2,G9"), rules=r)
    rows = tsv(sb)
    check(rows["G9"][1] == "PASS" and "1 question" in rows["G9"][2], "G9 %s" % (rows["G9"],))
    out = [f for f in os.listdir(os.path.join(sb, "ev")) if f.endswith("-install.out")]
    data = open(os.path.join(sb, "ev", out[0]), "rb").read()
    check(b"\x1b[1minstall\x1b[0m: ok" in data and b"\x1b[33mProceed" in data and b"\r\n" in data, "the transcript was cleaned: %r" % data[:80])


@cell("GR-31 an install that hangs: --install-timeout stops it by the exact child, G2 FAILS with 'timed out', the child is gone, G3 still runs")
def gr31():
    sb = None
    pidfile = None
    sb, r = g2_sandbox(install=[{"print": "working\r\n"}, {"sleep": 30, "pidfile": None}])
    pidfile = os.path.join(sb, "hang.pid")
    for x in r:
        if x.get("pty_steps"):
            x["pty_steps"][1]["pidfile"] = pidfile
    p = run(sb, base_args(sb, only="G1,G2,G9,G3", extra=["--install-timeout", "2"]), rules=r, timeout=60)
    rows = tsv(sb)
    check(rows["G2"][1] == "FAIL" and "timed out" in rows["G2"][2], "G2 %s" % (rows["G2"],))
    check(rows["G3"][1] == "PASS", "G3 was hidden by the timeout")
    time.sleep(0.3)
    gone = True
    try:
        os.kill(int(open(pidfile).read()), 0)
        gone = False
    except OSError:
        pass
    check(gone and os.path.exists(pidfile + ".term"), "the hung ssh child was not terminated")


@cell("GR-32 SIGTERM during the install: the PTY child is terminated, exit 130, the transcript so far is a receipt")
def gr32():
    sb, r = g2_sandbox(install=[{"print": "working\r\n"}, {"sleep": 30, "pidfile": None}])
    pidfile = os.path.join(sb, "pty.pid")
    for x in r:
        if x.get("pty_steps"):
            x["pty_steps"][1]["pidfile"] = pidfile
    json.dump({"rules": r}, open(os.path.join(sb, "scenario.json"), "w"))
    json.dump(base_local(), open(os.path.join(sb, "local.json"), "w"))
    env = {"PATH": FAKES + ":/usr/bin:/bin", "FAKE_LOG": os.path.join(sb, "fake.log"), "FAKE_SCENARIO": os.path.join(sb, "scenario.json"),
           "FAKE_LOCAL": os.path.join(sb, "local.json"), "HOME": sb}
    proc = subprocess.Popen(["/usr/bin/python3", "-I", RUNNER, *base_args(sb, only="G1,G2,G9")], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    end = time.time() + 20
    while not os.path.exists(pidfile) and time.time() < end:
        time.sleep(0.05)
    check(os.path.exists(pidfile), "the install never reached its sleeping step")
    child = int(open(pidfile).read())
    proc.send_signal(signal.SIGTERM)
    proc.communicate(timeout=30)
    check(proc.returncode == 130, "exit %s" % proc.returncode)
    time.sleep(0.3)
    gone = True
    try:
        os.kill(child, 0)
        gone = False
    except OSError:
        pass
    check(gone and os.path.exists(pidfile + ".term"), "the PTY child was not terminated")
    out = [f for f in os.listdir(os.path.join(sb, "ev")) if "-install" in f and f.endswith(".out")]
    check(out, "no .out receipt of the interrupted install: %s" % os.listdir(os.path.join(sb, "ev")))
    check(b"working" in open(os.path.join(sb, "ev", out[0]), "rb").read(), "the interrupted transcript lacks what the install had printed before the signal")
    check(any(f.endswith(".answers") for f in os.listdir(os.path.join(sb, "ev")) if "-install" in f), "the answers receipt of the interrupted install is missing")
    check(open(os.path.join(sb, "ev", out[0][:-4] + ".rc"), "rb").read().strip() == b"interrupted", "the interrupted install's rc receipt does not say interrupted")
    v = subprocess.run(["sha256sum", "-c", "SHA256SUMS"], cwd=os.path.join(sb, "ev"), capture_output=True)
    check(v.returncode == 0, "SHA256SUMS does not verify")


@cell("GR-33 the artefact directory already exists on the target (not clean): G2 FAILS, nothing is uploaded or run; GR-34 G9 is NOT RUN when the install never ran")
def gr33():
    sb, r = g2_sandbox()
    for i, x in enumerate(r):
        if x["match"] == "^" + re.escape('mkdir -m 0700 "$HOME/gate-artefacts"') + "$":
            r[i] = rule(x["match"], "", 1, err="mkdir: cannot create directory: File exists\n")
    run(sb, base_args(sb, only="G1,G2,G9"), rules=r)
    rows = tsv(sb)
    check(rows["G2"][1] == "FAIL" and "not clean" in rows["G2"][2], "G2 %s" % (rows["G2"],))
    check(not [c for c in ssh_commands(sb) if c.startswith("cat >") or c == INSTALL_CMD], "something was uploaded or run: %s" % ssh_commands(sb)[-3:])
    check(rows["G9"][1] == "NOT RUN", "G9 %s" % (rows["G9"],))


# ---- increment R3: G3 image identity, G4 first worker, G5 kept not redone --------------------------------------------------------
WORKER_TOKEN = '"$HOME/cortex-v2/gate-worker.token"'
FP_QUERY_HEAD = 'podman exec --user postgres cortex-gate-db psql -U postgres -d cortex -tAc "select table_schema'


def fp_rules(after=None, rerun=None):
    """Rules for the five fingerprint components; the BEFORE answer first, the AFTER answer second (a rule that has a sequence)."""
    base = {"packages": "bash 5.2-1.el10.x86_64\npodman 5.8.2-1.el10.x86_64\n", "containers": "a1 cortex-gate-db 2026-11-08 db-image\nb2 cortex-gate-server 2026-11-08 srv-image\n",
            "files": "e5" * 32 + "  /home/gate/cortex-v2/cortex.toml\n" + "f6" * 32 + "  /home/gate/cortex-v2/gate-lead.token\n",
            "timers": "cortex-dump.timer enabled\ncortex-dump-status.timer enabled\n", "rows": "public.projects|1\npublic.workers|2\n"}
    after = after or {}
    keys = {"packages": r"^rpm -qa --qf", "containers": r"^podman ps -a --format", "files": r"^find .* -exec sha256sum", "timers": r"^systemctl --user list-unit-files",
            "rows": "^" + re.escape(FP_QUERY_HEAD)}
    out = []
    for name, match in keys.items():
        seq = [{"out": base[name], "rc": 0}, {"out": after.get(name, base[name]), "rc": 0}]
        if name == "packages":   # G1 runs the same rpm command first: its answer is the first of the sequence
            seq = [{"out": base[name], "rc": 0}] + seq
        out.append({"match": match, "seq": seq})
    return out


def g5_sandbox(after=None, install=None, rc=0):
    sb, r = g2_sandbox(install=install if install is not None else [{"print": CONSENT, "read": True}, {"print": "install: done\r\n"}], rc=rc)
    # the first install asks one question; the second run (G5) is a second call of the same command: no question
    for x in r:
        if x.get("pty_steps"):
            first = x["pty_steps"]
            x["pty_steps"] = None
            x["seq"] = [{"pty_steps": first, "rc": rc}, {"pty_steps": [{"print": "install: nothing to do\r\n"}], "rc": 0}]
    r = [x for x in r if not x["match"].startswith("^rpm -qa --qf")]   # replaced by the sequence rule below
    return sb, fp_rules(after) + r


@cell("GR-40 G3 compares the migrate container's image with the release's stack.env: a different image, a missing or unusable stack.env FAIL")
def gr40():
    sb = sandbox()
    r = clean_rules()
    for i, x in enumerate(r):
        if "podman inspect" in x["match"]:
            r[i] = rule(x["match"], "0 localhost/cortex-server:dev\n")
    run(sb, base_args(sb), rules=r)
    check(tsv(sb)["G3"][1] == "FAIL" and "image" in tsv(sb)["G3"][2], "a different image: %s" % (tsv(sb)["G3"],))
    for name, out, rc in (("unreadable", "", 1), ("no server image", "CORTEX_VERSION=0.2.1\n", 0)):
        sb = sandbox()
        r = clean_rules()
        for i, x in enumerate(r):
            if "stack.env" in x["match"]:
                r[i] = rule(x["match"], out, rc)
        run(sb, base_args(sb), rules=r)
        check(tsv(sb)["G3"][1] == "FAIL" and "stack.env" in tsv(sb)["G3"][2], "%s stack.env: %s" % (name, tsv(sb)["G3"]))


UUID_W = "11111111-2222-4333-8444-555555555555"
LEAD_TOKEN = "LEADTOKEN-canary-0123456789abcdef"
ISSUED_TOKEN = "ISSUEDTOKEN-canary-fedcba9876543210"


def whoami_json(**over):
    d = {"installation": "i-1", "project": "gate", "worker": UUID_W, "name": "gate-lead", "kind": "ai", "role": "lead", "team": "t", "job_roles": ["lead"],
         "token_id": "tok-new", "expires_at": "2026-12-01T00:00:00Z"}
    d.update(over)
    for k in [k for k, v in d.items() if v is None]:
        del d[k]
    return json.dumps(d)


def g4_rules(base_rules, lead_mode="600\n", lead_code="200", lead_fields=None, issue_code="201", issued_ids="tok-new True 2026-12-01T00:00:00Z\n", who_body=None, who_code="200", hdr_rc=0):
    r = list(base_rules)
    r.append(rule(r"^stat -c '%a' .*gate-lead\.token", lead_mode))
    r.append(rule(r"^umask 077; printf 'Authorization: Bearer %s", "", hdr_rc))
    r.append(rule(r"^curl -sS -H @.*gate-lead\.hdr.* -o .*gate-lead-whoami\.json", lead_code))
    r.append(rule(r"^python3 -c .*gate-lead-whoami\.json", lead_fields if lead_fields is not None else UUID_W + " tok-lead\n"))
    r.append(rule(r"^cat .*gate-lead\.token", LEAD_TOKEN))
    r.append(rule(r"^umask 077; curl -sS -X POST ", issue_code))
    r.append(rule(r'^python3 -c .*"token" in d.*gate-worker-issued\.json', issued_ids))
    r.append(rule(r"^python3 -c .*sys\.stdout\.write", ISSUED_TOKEN))
    r.append(rule(r"^umask 077; python3 -c .*> .*gate-worker\.hdr", ""))
    r.append(rule(r"^curl -sS -H @.*gate-worker\.hdr", (who_body if who_body is not None else whoami_json()) + "\n" + who_code))
    r.append(rule(r'^rm -f "\$HOME/gate-lead\.hdr"', ""))
    return r


def g4_sandbox(**kw):
    sb, r = g2_sandbox()
    return sb, g4_rules(r, **kw)


def g4_args(sb, only="G1,G2,G9,G4"):
    return base_args(sb, only=only)


@cell("GR-41 G4 on the HTTP routes: the lead's token file is 0600, a second token is issued for the lead's own worker (201, a fresh Idempotency-Key), whoami with it (200) names that worker and the NEW token_id; no token is printed")
def gr41():
    sb, r = g4_sandbox()
    p = run(sb, g4_args(sb), rules=r)
    rows = tsv(sb)
    check(rows["G4"][1] == "PASS", "G4 %s (%s)" % (rows["G4"], p.stdout[-300:]))
    cmds = ssh_commands(sb)
    issue = [c for c in cmds if "-X POST" in c]
    check(len(issue) == 1 and "/v2/workers/%s/tokens" % UUID_W in issue[0] and re.search(r'Idempotency-Key: [0-9a-f-]{36}"', issue[0]), "the issue request: %s" % issue)
    check('rm -f "$HOME/gate-lead.hdr" "$HOME/gate-worker.hdr" "$HOME/gate-lead-whoami.json" "$HOME/gate-worker-issued.json"' in cmds, "the header and response files were not removed")
    check(not any("Bearer " + LEAD_TOKEN in c or ISSUED_TOKEN in c for c in cmds), "a token value is in a command line sent to the target")
    sb2, r2 = g4_sandbox()
    run(sb2, g4_args(sb2), rules=r2)
    keys = [re.search(r"Idempotency-Key: ([0-9a-f-]{36})", c).group(1) for c in ssh_commands(sb2) + cmds if "-X POST" in c]
    check(len(keys) == 2 and keys[0] != keys[1], "the Idempotency-Key is not fresh: %s" % keys)


@cell("GR-41b G4 FAILS on: a lead token file that is not 0600, a lead whoami that is not 200 or gives no worker UUID (no token is issued), an issue answer other than 201, a response without token_id, a whoami that is not 200, lacks a field, names another worker or another token; the files are removed every time")
def gr41b():
    cases = {"lead mode 644": (dict(lead_mode="644\n"), "mode 600"), "lead whoami 401": (dict(lead_code="401"), "lead's token answered 401"),
             "no worker uuid": (dict(lead_fields="not-a-uuid tok-lead\n"), "no worker UUID"), "issue 428": (dict(issue_code="428"), "issuing a worker token answered 428"),
             "no token_id": (dict(issued_ids=" True 2026-12-01T00:00:00Z\n"), "lacks token"), "whoami 403": (dict(who_code="403"), "answered 403"),
             "field missing": (dict(who_body=whoami_json(job_roles=None)), "job_roles"), "other worker": (dict(who_body=whoami_json(worker="99999999-2222-4333-8444-555555555555")), "another worker"),
             "other token": (dict(who_body=whoami_json(token_id="tok-lead")), "token_id"), "not json": (dict(who_body="<html>502</html>"), "JSON")}
    for name, (kw, word) in cases.items():
        sb, r = g4_sandbox(**kw)
        run(sb, g4_args(sb), rules=r)
        rows = tsv(sb)
        check(rows["G4"][1] == "FAIL" and word.lower() in rows["G4"][2].lower(), "%s: %s" % (name, rows["G4"]))
        check('rm -f "$HOME/gate-lead.hdr" "$HOME/gate-worker.hdr" "$HOME/gate-lead-whoami.json" "$HOME/gate-worker-issued.json"' in ssh_commands(sb), "%s: the files were not removed" % name)
        if name in ("lead mode 644", "lead whoami 401", "no worker uuid"):
            check(not any("-X POST" in c for c in ssh_commands(sb)), "%s: a token was issued anyway" % name)


@cell("GR-41c G4 holds the token VALUES in memory only: they are in no receipt, no verdict file, no run.json and not on the terminal, the secret steps say 'withheld'; G8 finds an issued token that the installer printed, and passes when none leaked")
def gr41c():
    sb, r = g4_sandbox()
    p = run(sb, base_args(sb, only="G1,G2,G9,G4,G8"), rules=r)
    rows = tsv(sb)
    check(rows["G4"][1] == "PASS" and rows["G8"][1] == "PASS", "G4 %s G8 %s" % (rows["G4"][1:3], rows["G8"][1:3]))
    check("2 held secret value(s)" in rows["G8"][2], "G8 does not say it held the two token values: %s" % rows["G8"][2])
    ev = os.path.join(sb, "ev")
    for v in (LEAD_TOKEN, ISSUED_TOKEN):
        hit = subprocess.run(["grep", "-r", "-l", "-F", v, ev], capture_output=True)
        check(hit.returncode == 1, "the value %s... is under the evidence: %s" % (v[:10], hit.stdout))
        check(v.encode() not in p.stdout + p.stderr, "a token value reached the terminal")
    withheld = [f for f in os.listdir(ev) if "-secret_" in f and f.endswith(".out")]
    check(len(withheld) == 2 and all(b"withheld" in open(os.path.join(ev, f), "rb").read() for f in withheld), "the secret steps do not say withheld: %s" % withheld)
    leak = [{"print": "debug: got " + ISSUED_TOKEN + "\r\n"}, {"print": CONSENT, "read": True}, {"print": "done\r\n"}]
    sb, r = g2_sandbox(install=leak)
    r = g4_rules(r)
    run(sb, base_args(sb, only="G1,G2,G9,G4,G8"), rules=r)
    rows = tsv(sb)
    check(rows["G8"][1] == "FAIL" and "known_secret" in rows["G8"][2], "G8 missed the issued token in the install transcript: %s" % (rows["G8"],))
    check(ISSUED_TOKEN not in rows["G8"][2], "G8 printed the value")


@cell("GR-41d G4 is NOT RUN when the install did not run (no stack to sign in to)")
def gr41d():
    sb = sandbox()
    run(sb, base_args(sb, only="G1,G4"))
    check(tsv(sb)["G4"][1] == "NOT RUN", "G4 %s" % (tsv(sb)["G4"],))


@cell("GR-42 G5: equal fingerprints and a quiet second run PASS; a changed package list, container, file, timer or row count each FAIL naming the component; so do a question or a non-zero exit on the second run; NOT RUN when G2 did not install")
def gr42():
    sb, r = g5_sandbox()
    p = run(sb, base_args(sb, only="G1,G2,G9,G5"), rules=r)
    check(tsv(sb)["G5"][1] == "PASS", "G5 %s (%s)" % (tsv(sb)["G5"], p.stdout[-300:]))
    changes = {"packages": "bash 5.2-1.el10.x86_64\npodman 5.8.3-1.el10.x86_64\n", "containers": "a1 cortex-gate-db 2026-11-08 db-image\nb9 cortex-gate-server 2026-11-08 srv-image\n",
               "files": "e5" * 32 + "  /home/gate/cortex-v2/cortex.toml\n" + "00" * 32 + "  /home/gate/cortex-v2/gate-lead.token\n",
               "timers": "cortex-dump.timer enabled\n", "rows": "public.projects|1\npublic.workers|3\n"}
    for comp, out in changes.items():
        sb, r = g5_sandbox(after={comp: out})
        run(sb, base_args(sb, only="G1,G2,G9,G5"), rules=r)
        check(tsv(sb)["G5"][1] == "FAIL" and comp in tsv(sb)["G5"][2], "%s changed: %s" % (comp, tsv(sb)["G5"]))
    # the same lines in another order are the same state
    sb, r = g5_sandbox(after={"files": "f6" * 32 + "  /home/gate/cortex-v2/gate-lead.token\n" + "e5" * 32 + "  /home/gate/cortex-v2/cortex.toml\n"})
    run(sb, base_args(sb, only="G1,G2,G9,G5"), rules=r)
    check(tsv(sb)["G5"][1] == "PASS", "a reordered file list was seen as a change: %s" % (tsv(sb)["G5"],))
    sb, r = g5_sandbox()
    for x in r:
        if x.get("seq") and x["seq"][0].get("pty_steps"):
            x["seq"][1] = {"pty_steps": [{"print": CONSENT, "read": True}, {"print": "done\r\n"}], "rc": 0}
    run(sb, base_args(sb, only="G1,G2,G9,G5"), rules=r)
    check(tsv(sb)["G5"][1] == "FAIL" and "question" in tsv(sb)["G5"][2], "a question on the second run: %s" % (tsv(sb)["G5"],))
    sb, r = g5_sandbox()
    for x in r:
        if x.get("seq") and x["seq"][0].get("pty_steps"):
            x["seq"][1] = {"pty_steps": [{"print": "problem: x\r\n"}], "rc": 2}
    run(sb, base_args(sb, only="G1,G2,G9,G5"), rules=r)
    check(tsv(sb)["G5"][1] == "FAIL" and "exited 2" in tsv(sb)["G5"][2], "a failing second run: %s" % (tsv(sb)["G5"],))
    sb, r = g5_sandbox()
    run(sb, base_args(sb, only="G1,G5"), rules=r)
    check(tsv(sb)["G5"][1] == "NOT RUN", "G5 without G2: %s" % (tsv(sb)["G5"],))


@cell("GR-43 the row counts are exact SQL through podman exec in the db container NAMED BY THE RELEASE's compose; every R3 command is allowed")
def gr43():
    mod = load_module()
    sb, r = g5_sandbox()
    sb2, r2 = g5_sandbox()
    run(sb2, base_args(sb2, only="G1,G2,G9,G5"), rules=r2)
    cmds = ssh_commands(sb2)
    sql = [c for c in cmds if c.startswith("podman exec")]
    check(sql and all("cortex-gate-db" in c and "query_to_xml" in c for c in sql), "the row counts did not use the compose's db container: %s" % sql[:1])
    bad = [c for c in cmds if not mod.command_allowed(c)]
    check(not bad, "commands not on the allow-list: %s" % bad)
    check(not any("cv2-s5" in c for c in cmds), "a hard-coded cv2-s5 name was sent")


# ---- increment R5: G8, the leak scan (canaries and patterns, no real secret is read) ------------------------------------------------
CANARIES = {"credential": "token=CANARYtoken1234567890", "hex64": "1f" * 32, "private_key": "-----BEGIN CANARY PRIVATE KEY-----",
            "bearer": "Authorization: Bearer CANARYbearer0123456789abcdef", "stderr": "password=CANARYstderrpass123"}


def g8_args(sb):
    return base_args(sb, only="G1,G2,G9,G8")


def leak_sandbox(logs=None, install=None, ss_err=""):
    sb, r = g2_sandbox(install=install)
    r2 = []
    for x in r:
        if x["match"] == r"^ss -ltnH$" and ss_err:
            x = dict(x, err=ss_err)
        r2.append(x)
    for name, text in (logs or {}).items():
        r2.insert(0, rule("^" + re.escape("podman logs " + name) + "$", text))
    return sb, r2


@cell("GR-50 G8 on a clean run: PASS; the scanner's own control finds all four classes; sha256 digests in the allowed contexts are not flagged; the container logs come from the release's compose names")
def gr50():
    sb, r = leak_sandbox()
    p = run(sb, g8_args(sb), rules=r)
    rows = tsv(sb)
    check(rows["G8"][1] == "PASS", "G8 %s (%s)" % (rows["G8"], p.stdout[-300:]))
    check("control 5/5" in rows["G8"][2], "the control is not stated: %s" % rows["G8"][2])
    logs = [c for c in ssh_commands(sb) if c.startswith("podman logs")]
    check(sorted(logs) == ["podman logs cortex-gate-db", "podman logs cortex-gate-migrate", "podman logs cortex-gate-server"], "container logs read: %s" % logs)


@cell("GR-51 G8 finds a planted canary of each class in an installer transcript, in container logs and in stderr; the value is never printed or written by the runner itself; LEAK-QUARANTINE.txt names the receipts")
def gr51():
    sb, r = leak_sandbox(
        logs={"cortex-gate-server": CANARIES["hex64"] + "\n", "cortex-gate-migrate": CANARIES["private_key"] + "\n", "cortex-gate-db": CANARIES["bearer"] + "\n"},
        install=[{"print": "debug " + CANARIES["credential"] + "\r\n"}, {"print": CONSENT, "read": True}, {"print": "done\r\n"}], ss_err=CANARIES["stderr"] + "\n")
    p = run(sb, g8_args(sb), rules=r)
    rows = tsv(sb)
    check(rows["G8"][1] == "FAIL", "G8 %s" % (rows["G8"],))
    for cls in ("hex64", "private_key", "bearer", "credential"):
        check(cls in rows["G8"][2], "class %s not named: %s" % (cls, rows["G8"][2]))
    check("5 hit" in rows["G8"][2], "the hit count is not stated: %s" % rows["G8"][2])
    ev = os.path.join(sb, "ev")
    check(os.path.exists(os.path.join(ev, "LEAK-QUARANTINE.txt")), "no LEAK-QUARANTINE.txt")
    q = open(os.path.join(ev, "LEAK-QUARANTINE.txt")).read()
    expected = ("install", "logs_cortex-gate-server", "logs_cortex-gate-migrate", "logs_cortex-gate-db", "ports")
    for name in expected:
        check(name in q, "the quarantine note does not name the receipt %s: %s" % (name, q[:300]))
    sb2, r2 = leak_sandbox(install=[{"print": "debug " + CANARIES["credential"] + "\r\n"}, {"print": CONSENT, "read": True}, {"print": "done\r\n"}])
    run(sb2, g8_args(sb2), rules=r2)
    q2 = open(os.path.join(sb2, "ev", "LEAK-QUARANTINE.txt")).read()
    check("install" in q2 and not any(name in q2 for name in expected[1:]), "the note names a receipt that did not leak (negative control): %s" % q2[:300])
    for f in ("verdict.tsv", "verdict.txt", "run.json", "LEAK-QUARANTINE.txt"):
        data = open(os.path.join(ev, f)).read()
        for v in CANARIES.values():
            check(v not in data and v.split("=")[-1] not in data, "the value %r reached %s" % (v, f))
    check(all(v.split("=")[-1].encode() not in p.stdout + p.stderr for v in CANARIES.values()), "a value reached the terminal")


@cell("GR-52 the allow-list is narrow: a sha256: digest and the hash-list receipts pass, the same 64 hex digits bare in a transcript are flagged")
def gr52():
    sb, r = leak_sandbox(install=[{"print": "build id " + "2e" * 32 + "\r\n"}, {"print": CONSENT, "read": True}, {"print": "done\r\n"}])
    run(sb, g8_args(sb), rules=r)
    rows = tsv(sb)
    check(rows["G8"][1] == "FAIL" and "hex64" in rows["G8"][2] and "1 hit" in rows["G8"][2], "G8 %s" % (rows["G8"],))


@cell("GR-54 G8 FAILS when no container log could be read, and is NOT RUN when G2 did not run the install")
def gr54():
    sb, r = leak_sandbox()
    r = [rule(r"^podman logs ", "", 1, err="Error: no such container\n")] + r
    run(sb, g8_args(sb), rules=r)
    check(tsv(sb)["G8"][1] == "FAIL" and "no container log" in tsv(sb)["G8"][2], "G8 %s" % (tsv(sb)["G8"],))
    sb = sandbox()
    run(sb, base_args(sb, only="G1,G8"))
    check(tsv(sb)["G8"][1] == "NOT RUN", "G8 without G2: %s" % (tsv(sb)["G8"],))


@cell("GR-53 G8 after G5: the fingerprint receipts (file hashes) are hash lists, not leaks")
def gr53():
    sb, r = g5_sandbox()
    p = run(sb, base_args(sb, only="G1,G2,G9,G5,G8"), rules=r)
    rows = tsv(sb)
    check(rows["G5"][1] == "PASS" and rows["G8"][1] == "PASS", "G5 %s G8 %s (%s)" % (rows["G5"][1:2], rows["G8"], p.stdout[-200:]))


# ---- increment R6: G11, backup timers scheduled and surviving a logout -----------------------------------------------------------
def g11_args(sb, wait="2"):
    return base_args(sb, only="G1,G2,G9,G11", extra=["--logout-wait", wait])


def g11_rules(after=None):
    """The AFTER-the-logout answers replace the second answer of a rule that gives a sequence."""
    sb, r = g2_sandbox()
    base = {"enabled": ("^systemctl --user is-enabled ", "enabled\nenabled\n", 0), "active": ("^systemctl --user is-active ", "active\nactive\n", 0),
            "linger": ("^loginctl show-user ", "Linger=yes\nState=lingering\n", 0), "list": ("^systemctl --user list-timers ", "cortex-dump.timer\ncortex-dump-status.timer\n", 0),
            "sessions": ("^loginctl list-sessions ", "12 1000 gate seat0\n", 0)}
    after = after or {}
    r = [x for x in r if not any(x["match"] == b[0] for b in base.values())]
    for name, (match, out, rc) in base.items():
        a_out, a_rc = after.get(name, (out, rc))
        r.insert(0, {"match": match, "seq": [{"out": out, "rc": rc}, {"out": a_out, "rc": a_rc}]})
    return sb, r


@cell("GR-60 G11 PASS: timers enabled and active and linger yes before AND after a real wait with no ssh session open; the second look is a second login")
def gr60():
    sb, r = g11_rules()
    p = run(sb, g11_args(sb), rules=r)
    rows = tsv(sb)
    check(rows["G11"][1] == "PASS", "G11 %s (%s)" % (rows["G11"], p.stdout[-300:]))
    ev = os.path.join(sb, "ev")
    def stamp(name):
        return time.mktime(time.strptime(open(os.path.join(ev, name)).read().strip(), "%Y-%m-%dT%H:%M:%SZ"))
    names = sorted(f[:-3] for f in os.listdir(ev) if f.endswith(".t1") and "-G11-" in f)
    first_b = [n for n in names if n.endswith("_after")]
    last_a = [n for n in names if n.endswith("_before") or n.endswith("sessions")]
    check(first_b and last_a, "receipts of both phases missing: %s" % names)
    gap = stamp(sorted(first_b)[0] + ".t0") - stamp(sorted(last_a)[-1] + ".t1")
    check(gap >= 1.5, "only %.1f s between the two looks: the logout was not waited for" % gap)


@cell("GR-61 G11 FAILS: a timer disabled, inactive after the logout, linger no, the user manager gone after the logout, another login session open (the logout would be vacuous)")
def gr61():
    cases = {"disabled": ({"enabled": ("disabled\nenabled\n", 1)}, "enabled"), "inactive after logout": ({"active": ("inactive\nactive\n", 3)}, "after the logout"),
             "linger no": ({"linger": ("Linger=no\nState=active\n", 0)}, "linger"), "manager gone": ({"linger": ("Linger=yes\nState=closing\n", 0)}, "state")}
    for name, (after, word) in cases.items():
        sb, r = g11_rules(after=after)
        run(sb, g11_args(sb, "1"), rules=r)
        rows = tsv(sb)
        check(rows["G11"][1] == "FAIL" and word in rows["G11"][2].lower(), "%s: %s" % (name, rows["G11"]))
    sb, r = g11_rules()
    for x in r:
        if x.get("seq") and x["seq"][0]["out"].startswith("12 1000"):
            x["seq"] = [{"out": "12 1000 gate seat0\n13 1000 gate seat0\n", "rc": 0}] * 2
    run(sb, g11_args(sb, "1"), rules=r)
    check(tsv(sb)["G11"][1] == "FAIL" and "session" in tsv(sb)["G11"][2].lower(), "another session: %s" % (tsv(sb)["G11"],))


@cell("GR-62 G11 is NOT RUN when G2 did not install; an unsafe --timers value is a usage error")
def gr62():
    sb = sandbox()
    run(sb, base_args(sb, only="G1,G11"))
    check(tsv(sb)["G11"][1] == "NOT RUN", "G11 %s" % (tsv(sb)["G11"],))
    p = run(sb, base_args(sandbox(), only="G11", extra=["--timers", "a;b.timer"]))
    check(p.returncode == 2, "an unsafe timers value: exit %s" % p.returncode)


# ---- increment R4: G10 (trust) and G7 (plain failures), on the clean machine, before G2 --------------------------------------------
def problem_fix(cause, fix):
    return [{"print": "problem: %s\r\nfix: %s\r\n" % (cause, fix)}]


def inj_defaults():
    return {"g10_checksum": (problem_fix("a release file does not match its checksum", "obtain a complete release"), 4),
            "g10_signature": (problem_fix("the signature could not be verified", "obtain the signed release files"), 4),
            "no_network": (problem_fix("the packages could not be downloaded", "check the network"), 3),
            "port": (problem_fix("port 8604 is in use", "stop what listens there or choose --port"), 2),
            "low_disk": (problem_fix("not enough free disk", "free 20 GiB"), 2),
            "old_podman": (problem_fix("podman is older than 5", "update podman"), 2),
            "no_linger": (problem_fix("lingering is off for this user", "run loginctl enable-linger"), 2),
            "bad_os": (problem_fix("this installer supports Rocky Linux 10 only", "use Rocky Linux 10"), 2)}


def state_text(extra="", rcs=None, home_extra=""):
    """The answer of the G7/G10 state probe on the trial machine, in the runner's sentinel format: each part ends with '@@name ANSWER', ANSWER an exit code or
    absent | unreadable | notdir (the directory parts)."""
    ans = dict({"trial_home": "0" if extra else "absent", "user_units": "absent", "podman_ps": "0", "ports": "0", "home": "0"}, **{k: str(v) for k, v in (rcs or {}).items()})
    parts = [("trial_home", extra), ("user_units", ""), ("podman_ps", ""), ("ports", "LISTEN 0 128 0.0.0.0:22 0.0.0.0:*\n"), ("home", ".bash_profile\n" + home_extra)]
    return "".join(text + "@@%s %s\n" % (name, ans[name]) for name, text in parts)


def inj_sandbox(over=None, state_after=None, ss_listening=True, static_install=None, pkg_after=None, linger="Linger=no\n", base=None):
    over = over or {}
    d = inj_defaults()
    d.update(over)
    sb, r = base if base else g2_sandbox()
    if static_install is not None:
        open(os.path.join(sb, "art", "install.sh"), "wb").write(static_install)
        r = [x for x in r if "sha256sum" not in x["match"]] + [rule("^" + re.escape('sha256sum "$HOME"/gate-artefacts/*') + "$", "".join(
            "%s  /home/gate/gate-artefacts/%s\n" % (hashlib.sha256(open(os.path.join(sb, "art", n), "rb").read()).hexdigest(), n) for n in os.listdir(os.path.join(sb, "art"))))]
    def run_rule(match, key):
        steps, rc = d[key]
        return rule(match, "", rc, pty_steps=steps)
    quiet = lambda m: rule(m, "")
    state_seq = [{"out": state_text(), "rc": 0}]
    state_seq.append({"out": state_text(**state_after), "rc": 0} if state_after else {"out": state_text(home_extra="gate-tamper-checksum\ngate-artefacts\n"), "rc": 0})
    ss_seq = [{"out": "LISTEN 0 128 0.0.0.0:22 0.0.0.0:*\n", "rc": 0}] + ([{"out": "LISTEN 0 128 127.0.0.1:8604 0.0.0.0:*\n", "rc": 0}] if ss_listening else [])
    extra = [
        quiet(r'^cp -a "\$HOME/gate-artefacts" "\$HOME/gate-tamper-(checksum|signature)"$'), quiet(r'^f="\$HOME/gate-tamper-(checksum|signature)/'),
        quiet(r'^rm -rf "\$HOME/gate-tamper-checksum" "\$HOME/gate-tamper-signature"$'),
        {"match": STATE_RULE, "seq": state_seq},
        run_rule(r'^sh "\$HOME/gate-tamper-checksum/install\.sh" ', "g10_checksum"), run_rule(r'^sh "\$HOME/gate-tamper-signature/install\.sh" ', "g10_signature"),
        run_rule(r"^unshare --user --map-current-user --net sh ", "no_network"),
        quiet(r"^nohup python3 -c "), quiet(r'^kill "\$\(cat "\$HOME/gate-listener\.pid"\)"; rm -f "\$HOME/gate-listener\.pid"$'),
        run_rule(r'^sh "\$HOME/gate-artefacts/install\.sh" .* --port 8604$', "port"), run_rule(r"^CORTEX_MIN_FREE_GIB=100000 sh ", "low_disk"),
        quiet(r'^mkdir -p "\$HOME/gate-shim"'), run_rule(r'^PATH="\$HOME/gate-shim:\$PATH" sh ', "old_podman"), quiet(r'^rm -rf "\$HOME/gate-shim"$'),
        rule(r'^loginctl show-user "\$\(id -un\)" -p Linger$', "Linger=no\n"), quiet(r'^loginctl disable-linger '), quiet(r'^loginctl enable-linger '),
        run_rule(r'^sh "\$HOME/gate-artefacts/install\.sh" .* --port 8607$', "no_linger"),
        quiet(r'^mkdir -p "\$HOME/gate-fakeroot/etc"'), run_rule(r'^PREREQ_ROOT="\$HOME/gate-fakeroot" sh ', "bad_os"), quiet(r'^rm -rf "\$HOME/gate-fakeroot"$'),
    ]
    r = [x for x in r if x["match"] != r"^ss -ltnH$"] + [{"match": r"^ss -ltnH$", "seq": ss_seq}]
    if pkg_after:
        base_pk = "bash 5.2-1.el10.x86_64\nrocky-release 10.2-1.el10.noarch\n"
        r = [x for x in r if not x["match"].startswith("^rpm -qa --qf")] + [{"match": r"^rpm -qa --qf", "seq": [{"out": base_pk, "rc": 0}, {"out": base_pk, "rc": 0}, {"out": pkg_after, "rc": 0}]}]
    extra = [x if x["match"] != r'^loginctl show-user "\$\(id -un\)" -p Linger$' else rule(x["match"], linger) for x in extra]
    return sb, extra + r


def g10_args(sb):
    return base_args(sb, only="G1,G10")


@cell("GR-70 G10 PASS on the clean machine: both tampered copies (checksum, signature) exit 4 with a plain cause and fix, the consent is declined (n), the state is unchanged, the copies are removed")
def gr70():
    sb, r = inj_sandbox()
    p = run(sb, g10_args(sb), rules=r)
    rows = tsv(sb)
    check(rows["G10"][1] == "PASS", "G10 %s (%s)" % (rows["G10"], p.stdout[-300:]))
    check(answers(sb) == [] or all(a != "y\n" for a in answers(sb)), "the consent was answered yes: %s" % answers(sb))
    cmds = ssh_commands(sb)
    check(cmds.count('rm -rf "$HOME/gate-tamper-checksum" "$HOME/gate-tamper-signature"') == 1, "the tampered copies were not removed once")
    check(sum(1 for c in cmds if c.startswith('f="$HOME/gate-tamper-')) == 2, "both tamper commands did not run: %s" % [c[:50] for c in cmds])
    check(not any(re.search(r"\b(pkill|killall|kill -9)\b", c) for c in cmds), "a pattern kill was sent")


@cell("GR-71 G10 FAILS: the tampered release was installed (exit 0), the installer stopped at the prerequisites (exit 2: the trust step was not reached), exit 4 without a fix line, a changed state, a wrong exit code; the copies are removed in every case")
def gr71():
    cases = {"accepted": ({"g10_checksum": (problem_fix("x", "y"), 0)}, None, "went through"),
             "prerequisites first": ({"g10_signature": ([{"print": CONSENT, "read": True}, {"print": "problem: declined\r\nfix: run again\r\n"}], 2)}, None, "not reached"),
             "no fix line": ({"g10_checksum": ([{"print": "problem: bad checksum\r\n"}], 4)}, None, "fix"),
             "wrong code": ({"g10_signature": (problem_fix("x", "y"), 5)}, None, "exit 5"),
             "stage left behind": ({}, {"home_extra": ".cortex-fetch.aB3dE9\n"}, "state"),
             "no cause": ({"g10_checksum": ([{"print": "fix: obtain a complete release\r\n"}], 4)}, None, "cause"),
             "state changed": ({}, {"extra": "gate-trial-home\n"}, "state")}
    cases["packages changed"] = ({}, None, "packages")
    for name, (over, state_after, word) in cases.items():
        sb, r = inj_sandbox(over=over, state_after=state_after, pkg_after="bash 5.2-1.el10.x86_64\npodman 5.8.2-1.el10.x86_64\n" if name == "packages changed" else None)
        run(sb, g10_args(sb), rules=r)
        rows = tsv(sb)
        check(rows["G10"][1] == "FAIL" and word in rows["G10"][2].lower(), "%s: %s" % (name, rows["G10"]))
        check('rm -rf "$HOME/gate-tamper-checksum" "$HOME/gate-tamper-signature"' in ssh_commands(sb), "%s: the tampered copies were not removed" % name)
        if name == "stage left behind":
            check(any('ls -A "$HOME"; echo "@@home $?"' in c for c in ssh_commands(sb)), "the top level of $HOME is not read for a stage left behind")
        if name == "prerequisites first":
            check(answers(sb) == ["n\n"], "%s: the consent was not declined: %s" % (name, answers(sb)))


@cell("GR-72 G7 PASS: six injections (no network, port in use, low disk, podman too old, no linger, unsupported OS), each exits with its code, says problem and fix, leaves the state unchanged; every setup is undone by exact name or pid")
def gr72():
    asks = ([{"print": CONSENT, "read": True}, {"print": "problem: lingering is off for this user\r\nfix: run loginctl enable-linger\r\n"}], 2)
    sb, r = inj_sandbox(over={"no_linger": asks})
    p = run(sb, base_args(sb, only="G1,G7"), rules=r)
    rows = tsv(sb)
    check(rows["G7"][1] == "PASS", "G7 %s (%s)" % (rows["G7"], p.stdout[-300:]))
    check(answers(sb) == ["n\n"], "the consent asked during an injection was answered %s (wanted one n)" % answers(sb))
    cmds = ssh_commands(sb)
    for want in ('kill "$(cat "$HOME/gate-listener.pid")"; rm -f "$HOME/gate-listener.pid"', 'rm -rf "$HOME/gate-shim"', 'rm -rf "$HOME/gate-fakeroot"'):
        check(want in cmds, "teardown missing: %s" % want)
    check(not any(re.search(r"\b(pkill|killall|kill -9|kill -KILL)\b", c) for c in cmds), "a pattern kill was sent")
    check(any(c.startswith("unshare --user --map-current-user --net") for c in cmds), "no-network was not injected with an unprivileged namespace")


@cell("GR-73 G7 FAILS: an injection that exits 0, one with no fix line, a changed state, a listener that did not take; every setup is still undone")
def gr73():
    cases = {"exit 0": ({"low_disk": (problem_fix("x", "y"), 0)}, None, True, "low_disk"),
             "no fix": ({"bad_os": ([{"print": "problem: wrong os\r\n"}], 2)}, None, True, "unsupported_os"),
             "state changed": ({}, {"extra": "gate-trial-home\n"}, True, "state"),
             "listener did not take": ({}, None, False, "did not take")}
    for name, (over, state_after, listening, word) in cases.items():
        sb, r = inj_sandbox(over=over, state_after=state_after, ss_listening=listening)
        run(sb, base_args(sb, only="G1,G7"), rules=r)
        rows = tsv(sb)
        check(rows["G7"][1] == "FAIL" and word in rows["G7"][2].lower(), "%s: %s" % (name, rows["G7"]))
        cmds = ssh_commands(sb)
        check('rm -rf "$HOME/gate-shim"' in cmds and 'rm -rf "$HOME/gate-fakeroot"' in cmds, "%s: a setup was not undone" % name)


@cell("GR-74 G7 static rule: every die or fail message of the shipped install.sh has a cause and a fix (a msg id on the line, or a fix cue in the words); a bare die is listed and FAILS")
def gr74():
    good = b'#!/bin/sh\nmsg podman_missing || :; die "$PREFLIGHT_WHY"\ndie "run it as a normal user, not as root"\nfail 5 "check the release location and try again"\ndie "$reason; no token file was written"\n'
    bad = good + b'die "something broke"\n'
    for name, body, want in (("good", good, "PASS"), ("bad", bad, "FAIL")):
        sb, r = inj_sandbox(static_install=body)
        run(sb, base_args(sb, only="G1,G7"), rules=r)
        rows = tsv(sb)
        check(rows["G7"][1] == want, "%s: G7 %s" % (name, rows["G7"]))
        if want == "FAIL":
            check("something broke" in rows["G7"][2] or "line 5" in rows["G7"][2], "the bare die is not listed: %s" % rows["G7"][2])


@cell("GR-75 G10 and G7 run BEFORE G2 on the clean machine, the artefacts are uploaded ONCE and shared, and the table still reads G1..G12")
def gr75():
    sb, r = inj_sandbox()
    run(sb, base_args(sb, only="G1,G10,G7,G2,G9"), rules=r)
    cmds = ssh_commands(sb)
    check(cmds.count('mkdir -m 0700 "$HOME/gate-artefacts"') == 1, "the artefacts were uploaded %d times" % cmds.count('mkdir -m 0700 "$HOME/gate-artefacts"'))
    first_tamper = min(i for i, c in enumerate(cmds) if "gate-tamper-checksum/install.sh" in c)
    g2_run = cmds.index(INSTALL_CMD)
    first_g7 = min(i for i, c in enumerate(cmds) if c.startswith("unshare --user"))
    check(first_tamper < first_g7 < g2_run, "order is not G10, G7, G2: %d %d %d" % (first_tamper, first_g7, g2_run))
    rows = tsv(sb)
    check(list(rows) == ["G%d" % i for i in range(1, 13)], "table order %s" % list(rows))
    check(rows["G2"][1] == "PASS" and rows["G10"][1] == "PASS" and rows["G7"][1] == "PASS", "verdicts G10 %s G7 %s G2 %s" % (rows["G10"][1], rows["G7"][1], rows["G2"][1]))


@cell("GR-76 G7 restores lingering only when it was on before the no-linger injection")
def gr76():
    sb, r = inj_sandbox(linger="Linger=yes\n")
    run(sb, base_args(sb, only="G1,G7"), rules=r)
    cmds = ssh_commands(sb)
    check('loginctl disable-linger "$(id -un)"' in cmds and 'loginctl enable-linger "$(id -un)"' in cmds, "linger was not restored: %s" % [c for c in cmds if "linger" in c])
    sb, r = inj_sandbox(linger="Linger=no\n")
    run(sb, base_args(sb, only="G1,G7"), rules=r)
    check('loginctl enable-linger "$(id -un)"' not in ssh_commands(sb), "linger was switched on although it was off before")


@cell("GR-77 an upload that does not match fails G10, G7 and G2 together, and no tamper, injection or install command is sent")
def gr77():
    sb, r = g2_sandbox(wrong_hash="manifest.json")
    run(sb, base_args(sb, only="G1,G10,G7,G2"), rules=r)
    rows = tsv(sb)
    check(all(rows[g][1] == "FAIL" and "manifest.json" in rows[g][2] for g in ("G10", "G7", "G2")), "rows %s" % {g: rows[g][1:3] for g in ("G10", "G7", "G2")})
    cmds = ssh_commands(sb)
    check(not [c for c in cmds if c.startswith(("cp -a", "f=", "unshare", "nohup", "sh "))], "a later command was sent: %s" % [c[:40] for c in cmds if c.startswith(("cp -a", "f=", "unshare", "nohup", "sh "))])


# ---- increment R7: G12, the joined installer is sound (controller side: files, git and hue's checker; no target) -----------------------
NOTE_LINE = ("NOTE S5-10: an oracle run in a fixed environment, not a proof of definitions only; F5 is open (vera 5W-REVIEW); "
             "the real includes are read by hand at their final hashes and bound by hash (G12).")
DEV_INSTALL = ('#!/bin/sh\nset -eu\numask 077\nhere=x\n[ ! -f "$here/install-prereq.sh" ] || . "$here/install-prereq.sh"   # @include\n'
               '[ ! -f "$here/install-fetch.sh" ] || . "$here/install-fetch.sh"   # @include\necho done\n')
INC_PREREQ = b"prereq_msg() {\n  echo prereq\n}\n"
INC_FETCH = b"rel_fetch() (\n  echo fetch\n)\n"


def git(d, *a):
    return subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t", "-C", d, *a], capture_output=True, check=True).stdout.decode().strip()


def join_text(dev_install, includes):
    out = b""
    for line in dev_install.splitlines(keepends=True):
        m = re.search(rb'"\$here/([A-Za-z0-9._-]+)"\s+# @include\s*$', line)
        out += includes[m.group(1).decode()] if m else line
    return out


def g12_sandbox(dev_install=None, includes=None, oracle_out=None, oracle_rc=0, shipped=None, record=None, dirty=False, drift=False, symlinks=(), drift_record=False, drift_cmd=""):
    sb = sandbox()
    dev = os.path.join(sb, "dev")
    os.makedirs(os.path.join(dev, "deploy"))
    os.makedirs(os.path.join(dev, "scripts"))
    dev_text = (DEV_INSTALL if dev_install is None else dev_install).encode()
    includes = includes or {"install-prereq.sh": INC_PREREQ, "install-fetch.sh": INC_FETCH}
    open(os.path.join(dev, "deploy", "install.sh"), "wb").write(dev_text)
    for n, data in includes.items():
        if n in symlinks:   # the include is a symlink to a real file elsewhere in the tree
            open(os.path.join(dev, "real-" + n), "wb").write(data)
            os.symlink(os.path.join("..", "real-" + n), os.path.join(dev, "deploy", n))
        else:
            open(os.path.join(dev, "deploy", n), "wb").write(data)
    out = NOTE_LINE + "\n" if oracle_out is None else oracle_out
    open(os.path.join(dev, "scripts", "s5-check.sh"), "w").write("#!/bin/sh\ncat <<'EOF'\n" + out + "EOF\n" + ("echo '# drift' >> deploy/install-fetch.sh\n" if drift else "") + ("echo ' ' >> ../binding.json\n" if drift_record else "") + drift_cmd + "exit %d\n" % oracle_rc)
    git(dev, "init", "-q")
    git(dev, "add", "-A")
    git(dev, "commit", "-q", "-m", "x")
    joined = shipped if shipped is not None else join_text(dev_text, includes)
    open(os.path.join(sb, "art", "install.sh"), "wb").write(joined)
    rec = {"schema": 1, "object_format": "sha1", "commit": git(dev, "rev-parse", "HEAD"), "tree": git(dev, "rev-parse", "HEAD^{tree}"),
           "includes": [{"name": n, "sha256": hashlib.sha256(d).hexdigest()} for n, d in includes.items()],
           "splice": SPLICE_SENTENCE, "joined_sha256": hashlib.sha256(joined).hexdigest()}
    rec.update(record or {})
    json.dump(rec, open(os.path.join(sb, "binding.json"), "w"))
    if dirty:
        open(os.path.join(dev, "deploy", "install-fetch.sh"), "ab").write(b"# drift\n")
    return sb


def g12_args(sb):
    return g12m_args(sb)


def g12_row(sb):
    p = run(sb, g12_args(sb))
    return tsv(sb)["G12"], p


@cell("GR-80 G12 PASS: sh -n and bash --posix -n on the shipped file, the splice rebuilt by a second method equals it byte for byte, the binding record matches, each include ends with a newline and one closing brace or parenthesis, the checker prints the NOTE line")
def gr80():
    sb = g12_sandbox()
    row, p = g12_row(sb)
    check(row[1] == "PASS", "G12 %s (%s)" % (row, p.stdout[-300:]))
    ev = os.path.join(sb, "ev")
    binding = [f for f in os.listdir(ev) if "-G12-binding" in f and f.endswith(".out")]
    check(binding, "no binding receipt: %s" % sorted(os.listdir(ev)))
    text = open(os.path.join(ev, binding[0])).read()
    for want in (json.load(open(os.path.join(sb, "binding.json")))["joined_sha256"], hashlib.sha256(INC_PREREQ).hexdigest(), hashlib.sha256(INC_FETCH).hexdigest()):
        check(want in text, "the binding receipt lacks %s" % want[:12])
    check(any("-G12-oracle" in f for f in os.listdir(ev)), "the oracle's raw output is not a receipt")


@cell("GR-81 G12 FAILS on each fault: a shipped file that differs by one byte, a syntax error, an include without a final newline or with a last line that is not one closing brace or parenthesis, a marker with two names")
def gr81():
    inc_no_nl = {"install-prereq.sh": b"prereq_msg() {\n  echo prereq\n}", "install-fetch.sh": INC_FETCH}
    inc_fi = {"install-prereq.sh": b"prereq_msg() {\n  if true; then echo x; fi\n}\nfi\n", "install-fetch.sh": INC_FETCH}
    good = join_text(DEV_INSTALL.encode(), {"install-prereq.sh": INC_PREREQ, "install-fetch.sh": INC_FETCH})
    two_names = DEV_INSTALL.replace('. "$here/install-fetch.sh"', '. "$here/install-fetch.sh" "$here/install-prereq.sh"')
    cases = {"one byte differs": (dict(shipped=good.replace(b"echo done", b"echo dine")), "differs"),
             "syntax error": (dict(dev_install=DEV_INSTALL + "if then\n"), "syntax"),
             "no final newline": (dict(includes=inc_no_nl), "newline"),
             "last line not a brace": (dict(includes=inc_fi), "closing"),
             "two names on a marker": (dict(dev_install=two_names), "marker")}
    for name, (kw, word) in cases.items():
        sb = g12_sandbox(**kw)
        row, p = g12_row(sb)
        check(row[1] == "FAIL" and word in row[2].lower(), "%s: %s" % (name, row))


@cell("GR-82 G12 FAILS on the binding record: a wrong joined hash, a wrong include hash, include order changed, a wrong commit or tree, a dirty development tree")
def gr82():
    h1, h2 = hashlib.sha256(INC_PREREQ).hexdigest(), hashlib.sha256(INC_FETCH).hexdigest()
    cases = {"joined hash": (dict(record={"joined_sha256": "0" * 64}), "joined"),
             "include hash": (dict(record={"includes": [{"name": "install-prereq.sh", "sha256": "1" * 64}, {"name": "install-fetch.sh", "sha256": h2}]}), "install-prereq.sh"),
             "order": (dict(record={"includes": [{"name": "install-fetch.sh", "sha256": h2}, {"name": "install-prereq.sh", "sha256": h1}]}), "order"),
             "commit": (dict(record={"commit": "2" * 40}), "commit"), "tree": (dict(record={"tree": "3" * 40}), "tree"), "dirty": (dict(dirty=True), "uncommitted")}
    for name, (kw, word) in cases.items():
        sb = g12_sandbox(**kw)
        row, p = g12_row(sb)
        check(row[1] == "FAIL" and word in row[2].lower(), "%s: %s" % (name, row))


@cell("GR-83 G12 and the checker: a non-zero exit, a missing NOTE line, a second claim that the checker proves definitions only; NOT RUN without --dev-tree and --binding-record")
def gr83():
    cases = {"exit 1": (dict(oracle_rc=1), "exited 1"), "no NOTE line": (dict(oracle_out="S5 OK\n"), "note"),
             "a proof claim": (dict(oracle_out=NOTE_LINE + "\nS5-10 proves functions only\n"), "claim"),
             "the NOTE line twice": (dict(oracle_out=NOTE_LINE + "\n" + NOTE_LINE + "\n"), "note")}
    for name, (kw, word) in cases.items():
        sb = g12_sandbox(**kw)
        row, p = g12_row(sb)
        check(row[1] == "FAIL" and word in row[2].lower(), "%s: %s" % (name, row))
    sb = sandbox()
    run(sb, base_args(sb, only="G1,G12"))
    check(tsv(sb)["G12"][1] == "NOT RUN", "without the inputs: %s" % (tsv(sb)["G12"],))


@cell("GR-84 G12: a byte that changes while the check runs (the checker touches an include) invalidates it; GR-85 a local command that is not on the allow-list is refused before it runs")
def gr84():
    sb = g12_sandbox(drift=True)
    row, p = g12_row(sb)
    check(row[1] == "FAIL" and "changed while" in row[2].lower(), "drift: %s" % (row,))
    mod = load_module()
    sb2 = sandbox()
    rec = mod.Recorder(os.path.join(sb2, "ev"))
    ctx = mod.Ctx(None, rec, None)
    marker = os.path.join(sb2, "must-not-exist")
    try:
        ctx.local_step("G12", "evil", ["touch", marker])
        raised = False
    except mod.Refused:
        raised = True
    check(raised and not os.path.exists(marker), "an unlisted local command ran: raised=%s exists=%s" % (raised, os.path.exists(marker)))


# ---- increment R8: G12 (e), the finite input matrix, layer L1 (source time), through hue's oracle ----------------------------------------
FAKE_ORACLE = '''import subprocess
import os as _os
if _os.environ.get("GR_ORACLE_IMPORTED"):   # the oracle's TOP LEVEL: it runs when, and only when, the module is imported (pin A repair: a refused oracle must never be)
    open(_os.environ["GR_ORACLE_IMPORTED"], "a").write("imported\\n")
ORACLE_PRELUDE = b"enable -n kill suspend\\nset -x\\n"
ORACLE_EPILOGUE = b"\\ncase $- in *x*) ;; *) exit 99 ;; esac\\nset +x\\nset +o\\nexport -p\\nreadonly -p\\ntrap -p\\nexit 73\\n"
ORACLE_SENTINEL_STATUS = 73
ORACLE_ENV = {"PATH": "/oracle/path", "HOME": "/oracle/home", "LC_ALL": "C"}
def oracle_tools():
    return ("fake",)
def oracle_baseline():
    return b""
def _oracle_run(data, expect):
    if "MAKE_LEAF_FAIL" in ORACLE_ENV:
        return b"restricted: the leaf cannot start with this context\\n", 1, ""
    p = subprocess.run(["/usr/bin/bash", "--norc", "--noprofile", "--posix", "-r", "-s"], input=data, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                       env=ORACLE_ENV, cwd="/", timeout=20)
    return p.stdout, p.returncode, ""
'''
SMALL_INV = {"schema": 1, "variables": [
    {"name": "CORTEX_INSTALL_POLL", "kind": "numeric", "valid": ["5"], "hostile": ["$(echo CANARY)"], "read_at": "x"},
    {"name": "CORTEX_LOG", "kind": "path", "valid": ["/home/gate/install.log"], "hostile": [], "read_at": "x"},
    {"name": "PATH", "kind": "shell", "valid": ["/oracle/other:/oracle/path"], "hostile": [], "read_at": "x"}],
    "interacting": [{"why": "poll and log together", "set": {"CORTEX_INSTALL_POLL": "1", "CORTEX_LOG": "/home/gate/install.log"}}],
    "argv": [[], ["--check"]], "profiles": ["clean", "all_valid"]}
WITNESS = b"w() {\n  :\n}\nfor p in ${CORTEX_INSTALL_POLL:-}; do echo CANARY; done\nv() {\n  :\n}\n"


def matrix_fixtures(sb, inv=None, approved=True, bind=True):
    os.makedirs(os.path.join(sb, "oracle", "scripts"), exist_ok=True)
    open(os.path.join(sb, "oracle", "scripts", "s5_check.py"), "w").write(FAKE_ORACLE)
    dev_checker, oracle_checker = os.path.join(sb, "dev", "scripts", "s5-check.sh"), os.path.join(sb, "oracle", "scripts", "s5-check.sh")
    if os.path.exists(dev_checker) and not os.path.exists(oracle_checker):
        # way C: with --oracle-tree the checker OF THE ORACLE TREE is the one that runs; for the older cells it is the candidate's own fake, byte for byte, so they still test what they tested
        shutil.copy(dev_checker, oracle_checker)
    if bind and not os.path.isdir(os.path.join(sb, "oracle", ".git")):
        # way C (nix's specification answer of 12 Nov): an oracle tree given to --oracle-tree must have an IDENTITY (a commit and a tree id) or G12 fails: the cells' oracle is a git tree, unless bind=False
        orc = os.path.join(sb, "oracle")
        git(orc, "init", "-q")
        git(orc, "add", "-A")
        git(orc, "commit", "-q", "-m", "oracle")
    path = os.path.join(sb, "inventory.json")
    json.dump(inv or SMALL_INV, open(path, "w"))
    return ["--oracle-tree", os.path.join(sb, "oracle"), "--inventory", path,
            "--inventory-approved-sha256", sha(path) if approved else "f" * 64]


def g12m_args(sb, **kw):
    a = base_args(sb, only="G1,G12", extra=["--dev-tree", os.path.join(sb, "dev"), "--binding-record", os.path.join(sb, "binding.json")] + matrix_fixtures(sb, **kw))
    return a


def g12m(sb, **kw):
    p = run(sb, g12m_args(sb, **kw))
    return tsv(sb)["G12"], p


def matrix_receipt(sb):
    ev = os.path.join(sb, "ev")
    names = [f for f in os.listdir(ev) if "-G12-matrix" in f and f.endswith(".out")]
    return open(os.path.join(ev, names[0])).read() if names else ""


@cell("GR-86 G12 matrix on good includes: every cell of every include is ACCEPTED, a clean control comes first, the table and BOTH raw observations of every run are receipts, the launch values are observed")
def gr86():
    sb = g12_sandbox()
    row, p = g12m(sb)
    check(row[1] == "PASS" and "matrix" in row[2], "G12 %s (%s)" % (row, p.stdout[-300:]))
    check("untestable (none)" in row[2], "the untestable list is not stated: %s" % row[2])
    text = matrix_receipt(sb)
    check("M000" in text and "clean control" in text, "no clean control row in the matrix table")
    raw = [f for f in os.listdir(os.path.join(sb, "ev")) if "-G12-matrix_raw" in f and f.endswith(".out")]
    check(raw and "--- empty-file run" in open(os.path.join(sb, "ev", raw[0])).read() and "OBS CORTEX_INSTALL_POLL=5" in open(os.path.join(sb, "ev", raw[0])).read(), "the raw observations are not kept or do not show the launch value")
    text = matrix_receipt(sb)
    check("inventory_sha256=" in text and "CORTEX_INSTALL_POLL unset\tACCEPT" in text.replace("\t", "\t"), "the matrix receipt: %s" % text[:300])
    check("REFUSE" not in text, "a good include was refused: %s" % [l for l in text.splitlines() if "REFUSE" in l][:2])
    for kind in ("CORTEX_INSTALL_POLL empty", "CORTEX_INSTALL_POLL valid", "CORTEX_INSTALL_POLL hostile", "poll and log together", "argv"):
        check(any(kind in l for l in text.splitlines()), "no %r row in the matrix table" % kind)
    table = [l.split("\t") for l in text.splitlines() if l.startswith("M") and l.count("\t") >= 5]
    # the expectation is written BY HAND from SMALL_INV (control; POLL unset, empty, valid, hostile; LOG unset, empty, valid; PATH unset, empty, valid;
    # one interacting case; two argument vectors x two profiles), not read back from the table (vera VR13-04)
    expected = [("M000", "control"), ("M001", "unset"), ("M002", "empty"), ("M003", "valid"), ("M004", "hostile"), ("M005", "unset"), ("M006", "empty"), ("M007", "valid"),
                ("M008", "unset"), ("M009", "empty"), ("M010", "valid"), ("M011", "interacting"), ("M012", "argv"), ("M013", "argv"), ("M014", "argv"), ("M015", "argv")]
    for name in ("install-fetch.sh", "install-prereq.sh"):
        got = [(r[0], r[2]) for r in table if r[1] == name]
        check(got == expected, "%s does not run exactly the expected cells: %s" % (name, got))
    check(len(table) == 2 * len(expected), "the table has %d rows, wanted %d" % (len(table), 2 * len(expected)))
    rawtext = open(os.path.join(sb, "ev", raw[0])).read()
    blocks = {}
    for blk in rawtext.split("=== ")[1:]:
        head = blk.split("\n", 1)[0].split()
        blocks[(head[0], head[1])] = blk
    for cid, kind in expected:
        for name in ("install-fetch.sh", "install-prereq.sh"):
            blk = blocks.get((cid, name))
            check(blk is not None and "--- empty-file run\n" in blk and "--- file run\n" in blk, "the raw observations lack both runs of %s in %s" % (cid, name))


@cell("GR-87 G12 matrix finds vera's witness class: a top-level for-loop over an unset variable is accepted when it is UNSET and refused when the variable has a value")
def gr87():
    includes = {"install-prereq.sh": WITNESS, "install-fetch.sh": INC_FETCH}
    sb = g12_sandbox(includes=includes)
    row, p = g12m(sb)
    check(row[1] == "FAIL" and "CORTEX_INSTALL_POLL" in row[2], "G12 %s" % (row,))
    lines = matrix_receipt(sb).splitlines()
    unset = [l for l in lines if "CORTEX_INSTALL_POLL unset" in l and "install-prereq.sh" in l]
    valid = [l for l in lines if "CORTEX_INSTALL_POLL valid" in l and "install-prereq.sh" in l]
    check(unset and unset[0].split("\t")[4] == "ACCEPT", "the unset cell did not accept (the plain oracle's blind spot): %s" % unset)
    check(valid and valid[0].split("\t")[4] == "REFUSE", "the valid cell did not refuse: %s" % valid)
    sb = g12_sandbox(includes={"install-prereq.sh": INC_PREREQ, "install-fetch.sh": WITNESS})   # the same witness in the OTHER include (vera D-F2)
    row, p = g12m(sb)
    lines = matrix_receipt(sb).splitlines()
    valid = [l for l in lines if "CORTEX_INSTALL_POLL valid" in l and "install-fetch.sh" in l]
    check(row[1] == "FAIL" and valid and valid[0].split("\t")[4] == "REFUSE", "the witness in install-fetch.sh was not refused: %s %s" % (row, valid[:1]))


@cell("GR-88 G12 matrix never runs unapproved or unconfined: an inventory whose hash is not the approved one FAILS, so does a development tree with no oracle; neither is a PASS or a skip")
def gr88():
    sb = g12_sandbox()
    row, p = g12m(sb, approved=False)
    check(row[1] == "FAIL" and "not owner-approved" in row[2], "unapproved: %s" % (row,))
    check(matrix_receipt(sb) == "", "the matrix ran on an unapproved inventory")
    sb = g12_sandbox()
    a = g12m_args(sb)
    i = a.index("--oracle-tree")
    a[i + 1] = os.path.join(sb, "no-such-tree")
    p = run(sb, a)
    row = tsv(sb)["G12"]
    check(row[1] == "FAIL" and ("matrix could not run" in row[2] or "the matrix was NOT run: the oracle is not bound" in row[2]), "no oracle: %s" % (row,))   # pin A repair (F4): an oracle tree that is not there is unbound, so the matrix is not even started
    check(matrix_receipt(sb) == "", "no oracle: the matrix ran")
    sb = g12_sandbox()   # an oracle file that exists but has no oracle in it: the generic handler, not the missing-file one
    a = g12m_args(sb)
    open(os.path.join(sb, "oracle", "scripts", "s5_check.py"), "w").write("x = 1\n")
    git(os.path.join(sb, "oracle"), "add", "-A")   # committed: the tool tree is bound and clean, so the matrix starts and its generic handler is the one tested
    git(os.path.join(sb, "oracle"), "commit", "-q", "-m", "no oracle in it")
    run(sb, a)
    row = tsv(sb)["G12"]
    check(row[1] == "FAIL" and "matrix could not run" in row[2] and "has no oracle" in row[2], "a checker without the oracle: %s" % (row,))


@cell("GR-89 the include context counts: a top-level use of an unset variable (set -u) and of the installer's argument vector are refused in the cells that make them matter, never accepted")
def gr89():
    unset_use = {"install-prereq.sh": b"w() {\n  :\n}\nfor p in $CORTEX_NO_SUCH_VARIABLE; do :; done\nv() {\n  :\n}\n", "install-fetch.sh": INC_FETCH}
    sb = g12_sandbox(includes=unset_use)
    row, p = g12m(sb)
    check(row[1] == "FAIL" and "refused" in row[2], "set -u: %s" % (row,))
    argv_use = {"install-prereq.sh": b"w() {\n  :\n}\nfor a; do echo \"$a\"; done\nv() {\n  :\n}\n", "install-fetch.sh": INC_FETCH}
    sb = g12_sandbox(includes=argv_use)
    row, p = g12m(sb)
    lines = [l.split("\t") for l in matrix_receipt(sb).splitlines() if "\t" in l and "install-prereq.sh" in l]
    clean_rows = [l for l in lines if l[2] == "argv" and "profile clean" in l[3] and "[]" in l[3]]
    check_rows = [l for l in lines if l[2] == "argv" and "--check" in l[3]]
    check(clean_rows and clean_rows[0][4] == "ACCEPT", "an empty argument vector must accept: %s" % clean_rows[:1])
    check(check_rows and all(l[4] == "REFUSE" for l in check_rows), "the --check argument vector must be refused: %s" % check_rows[:1])


def real_oracle_tree():
    for cand in (os.environ.get("GR_ORACLE_TREE", ""), os.path.expanduser("~/workspace/helix/cortex-v2-lanes/s5-5t"), os.path.join(HERE, "..", "..")):
        path = os.path.join(cand, "scripts", "s5_check.py") if cand else ""
        if path and os.path.isfile(path) and b"def _oracle_run" in open(path, "rb").read():
            return os.path.abspath(cand)
    return None


@cell("GR-90 G12 matrix through hue's REAL oracle (the user and PID namespace): good includes accepted, the witness refused where the variable has a value; skipped (and said so) when no tree with the oracle is available")
def gr90():
    tree = real_oracle_tree()
    if tree is None:
        print("SKIP GR-90: no development tree with hue's oracle (set GR_ORACLE_TREE)")
        return
    spec = importlib.util.spec_from_file_location("gm_real", os.path.join(HERE, "gate_matrix.py"))
    gm = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(gm)
    oracle = gm.load_oracle(tree)
    prefix = gm.prefix_from_install(DEV_INSTALL.encode())
    rows, counts, untestable, raw = gm.run_matrix(oracle, {"good.sh": INC_PREREQ}, SMALL_INV, prefix)
    check(counts["REFUSE"] == 0 and counts["ACCEPT"] > 5, "good include through the real oracle: %s %s" % (counts, untestable))
    rows, counts, untestable, raw = gm.run_matrix(oracle, {"witness.sh": WITNESS}, SMALL_INV, prefix)
    unset = [r for r in rows if r[3] == "CORTEX_INSTALL_POLL unset"]
    valid = [r for r in rows if r[3] == "CORTEX_INSTALL_POLL valid"]
    check(unset[0][4] == "ACCEPT" and valid[0][4] == "REFUSE", "the witness through the real oracle: unset %s valid %s" % (unset[0][4:], valid[0][4:]))


# ---- increment R9: the container transport and the PARTIAL dry run (G1, G2, G9, G5, G7, G8 only) -----------------------------------------
def podman_calls(sb):
    return [e["argv"] for e in fake_log(sb) if e.get("tool") == "podman" and e.get("argv", [None])[0] == "exec"]


def cont_args(sb, only=None, extra=None):
    a = base_args(sb, only=only, extra=extra)
    i = a.index("--target")
    del a[i:i + 2]
    return a + ["--container=gate-rocky10"]


def cont_sandbox():
    sb, r = g5_sandbox()
    return inj_sandbox(base=(sb, r))


@cell("GR-100 the partial dry run in a container: G1, G2, G9, G5, G7 (without the lingering injection) and G8 run through podman exec on ONE exact container name; G3, G4, G6, G10, G11 are NOT RUN with the reason; PARTIAL-DRY-RUN.txt says so; the overall is never PASS")
def gr100():
    mod = load_module()
    sb, r = cont_sandbox()
    p = run(sb, cont_args(sb), rules=r)
    rows = tsv(sb)
    check(p.returncode == 3, "exit %s, wanted 3 (INCOMPLETE): %s" % (p.returncode, p.stderr[-300:]))
    for g in ("G1", "G2", "G9", "G5", "G7", "G8"):
        check(rows[g][1] == "PASS", "%s %s" % (g, rows[g]))
    for g in ("G3", "G4", "G6", "G10", "G11"):
        check(rows[g][1] == "NOT RUN" and "container" in rows[g][2].lower(), "%s %s" % (g, rows[g]))
    check("no_linger not run in a container" in rows["G7"][2], "G7 does not say the lingering injection was not run: %s" % rows["G7"][2])
    check('loginctl disable-linger "$(id -un)"' not in [c[-1] for c in podman_calls(sb)], "lingering was switched off inside a container run")
    check(not [e for e in fake_log(sb) if e.get("tool") == "ssh"], "ssh was used in a container run")
    calls = podman_calls(sb)
    check(calls and all("gate-rocky10" in c and "-a" not in c and "--all" not in c for c in calls), "a podman exec without the exact name or with -a: %s" % calls[:2])
    check(all(mod.command_allowed(c[-1]) for c in calls), "a command outside the allow-list: %s" % [c[-1][:60] for c in calls if not mod.command_allowed(c[-1])][:2])
    check(all("-i" in c or "-it" in c for c in calls if c[-1].startswith("cat > ")), "an upload without podman exec -i")
    check(all("-it" in c for c in calls if c[-1].startswith("sh \"$HOME/gate-artefacts/install.sh\"")), "the install without a terminal (-it)")
    ev = os.path.join(sb, "ev")
    note = open(os.path.join(ev, "PARTIAL-DRY-RUN.txt")).read()
    check("G3, G4, G6, G10, G11" in note and "not the gate" in note and "no_linger" in note, "the partial note: %s" % note[:300])
    check("PARTIAL DRY RUN" in open(os.path.join(ev, "verdict.txt")).read().splitlines()[1], "verdict.txt does not announce the partial run")


@cell("GR-101 a container run is refused: not started through cv2-admit (exit 2), no such container (exit 2), its machine-id is this host's (exit 12) or its hostname is (exit 13)")
def gr101():
    sb, r = cont_sandbox()
    p = run(sb, cont_args(sb), rules=r, env_extra={"CV2_ADMITTED": ""})
    check(p.returncode == 2 and b"cv2-admit" in p.stderr and not podman_calls(sb), "not admitted: exit %s %s" % (p.returncode, p.stderr[-200:]))
    sb, r = cont_sandbox()
    json.dump(["another-container"], open(os.path.join(sb, "containers.json"), "w"))
    p = run(sb, cont_args(sb), rules=r)
    check(p.returncode == 2 and not podman_calls(sb), "no such container: exit %s" % p.returncode)
    sb, r = cont_sandbox()
    r2 = [x for x in r if x["match"] != r"^cat /etc/machine-id$" and not x["match"].startswith("^cat /etc/machine-id")]
    r2.insert(0, rule(r"^cat /etc/machine-id$", LOCAL_ID + "\n"))
    p = run(sb, cont_args(sb), rules=r2)
    check(p.returncode == 12 and len(podman_calls(sb)) == 1, "machine-id equal: exit %s, exec calls %d" % (p.returncode, len(podman_calls(sb))))
    sb, r = cont_sandbox()
    r2 = [rule(r"^hostname$", "build-host-1\n")] + r
    p = run(sb, cont_args(sb), rules=r2)
    check(p.returncode == 13 and len(podman_calls(sb)) == 2, "hostname equal: exit %s, exec calls %d" % (p.returncode, len(podman_calls(sb))))


@cell("GR-102 the container's own identity is a receipt, and a local podman command that is not on the allow-list is refused before it runs")
def gr102():
    mod = load_module()
    sb, r = cont_sandbox()
    run(sb, cont_args(sb, only="G1"), rules=r)
    ev = os.path.join(sb, "ev")
    names = [f for f in os.listdir(ev) if "container_inspect" in f and f.endswith(".out")]
    check(names and b"rocky-systemd" in open(os.path.join(ev, names[0]), "rb").read(), "no container identity receipt: %s" % sorted(os.listdir(ev))[:6])
    sb2 = sandbox()
    rec = mod.Recorder(os.path.join(sb2, "ev"))
    ctx = mod.Ctx(None, rec, None)
    try:
        ctx.local_step("G1", "evil", ["podman", "rm", "-f", "gate-rocky10"])
        raised = False
    except mod.Refused:
        raised = True
    check(raised, "podman rm -f was allowed as a local command")


# the include context is the REAL prefix of install.sh, so a caller variable that the installer overwrites is overwritten in the cell too (vera's REWORK, point 2)
OVERWRITE_INSTALL = ('#!/bin/sh\nset -eu\numask 077\nCORTEX_YES=0\nhere=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)\n'
                     '[ ! -f "$here/install-prereq.sh" ] || . "$here/install-prereq.sh"   # @include\n[ ! -f "$here/install-fetch.sh" ] || . "$here/install-fetch.sh"   # @include\necho done\n')
READS_YES = b"w() {\n  :\n}\nfor p in ${CORTEX_YES#0}; do echo CANARY; done\nv() {\n  :\n}\n"
YES_INV = {"schema": 2, "variables": [{"name": "CORTEX_YES", "kind": "text", "valid": ["1"], "hostile": [], "read_at": "install.sh 26"}], "interacting": [], "argv": [[]], "profiles": ["clean"]}


@cell("GR-91 the cells replay install.sh's own prefix: a caller CORTEX_YES=1 is overwritten before the include (line 26), so an include that reacts to CORTEX_YES=1 is ACCEPTED there; the context is a receipt line (its hash); a development install.sh with no include marker FAILS")
def gr91():
    sb = g12_sandbox(dev_install=OVERWRITE_INSTALL, includes={"install-prereq.sh": READS_YES, "install-fetch.sh": INC_FETCH})
    row, p = g12m(sb, inv=YES_INV)
    check(row[1] == "PASS", "the overwritten caller variable was not replayed: %s" % (row,))
    check("include_context_sha256=" in matrix_receipt(sb), "the include context is not a receipt line")
    rows91 = [l.split("\t") for l in matrix_receipt(sb).splitlines() if "\t" in l and "install-prereq.sh" in l]
    yes = [l for l in rows91 if "CORTEX_YES valid" in l[3]]
    check(yes and all(l[4] == "ACCEPT" for l in yes), "the CORTEX_YES cells were not ACCEPTED (untestable would mean the launch value never reached the leaf): %s" % yes[:2])
    raw91 = [f for f in os.listdir(os.path.join(sb, "ev")) if "-G12-matrix_raw" in f and f.endswith(".out")]
    check(raw91 and "OBS CORTEX_YES=1" in open(os.path.join(sb, "ev", raw91[0])).read(), "the raw observations do not show CORTEX_YES=1 in the leaf")
    sb = g12_sandbox(dev_install="#!/bin/sh\necho no marker here\n")
    row, p = g12m(sb)
    check(row[1] == "FAIL" and "marker" in row[2].lower(), "no marker: %s" % (row,))


@cell("GR-92 a launch value the leaf shell does not take (SHELLOPTS=bogus) is UNTESTABLE with what was observed, listed in the verdict, never an accept; a value that spans lines is observed whole; a taken value is accepted")
def gr92():
    inv = {"schema": 2, "variables": [{"name": "SHELLOPTS", "kind": "shell", "valid": ["bogus"], "hostile": [], "read_at": "the shell"},
                                      {"name": "CORTEX_LOG", "kind": "path", "valid": ["/home/gate/x.log"], "hostile": ["x\necho CANARY"], "read_at": "install.sh 26"}],
           "interacting": [], "argv": [[]], "profiles": ["clean"]}
    sb = g12_sandbox()
    row, p = g12m(sb, inv=inv)
    check(row[1] == "PASS" and "untestable (SHELLOPTS)" in row[2], "a value the leaf did not take must be listed: %s" % (row,))
    lines = [l.split("\t") for l in matrix_receipt(sb).splitlines() if "\t" in l]
    bogus = [l for l in lines if "SHELLOPTS valid" in l[3]]
    check(bogus and all(l[4] == "UNTESTABLE" and "did not take SHELLOPTS" in l[5] for l in bogus), "SHELLOPTS=bogus: %s" % bogus[:1])
    check(any("CORTEX_LOG valid" in l[3] and l[4] == "ACCEPT" for l in lines), "a taken value was not accepted")
    check(any("CORTEX_LOG hostile" in l[3] and l[4] == "ACCEPT" for l in lines), "a value that spans two lines was not observed whole")


@cell("GR-93 a vacuous matrix is a FAIL: a clean control that is not accepted (the replayed context cannot run), more than half of the cells untestable, and an UNTESTABLE variable is listed and kept out of the profiles")
def gr93():
    sb = g12_sandbox(dev_install=DEV_INSTALL.replace("here=x\n", "cd /\nhere=x\n"))
    row, p = g12m(sb)
    check(row[1] == "FAIL" and "control" in row[2].lower(), "a control that cannot run: %s" % (row,))
    broken = {"schema": 2, "variables": [{"name": "MAKE_LEAF_FAIL", "kind": "text", "valid": ["1", "2", "3", "4"], "hostile": ["a", "b"], "read_at": "x"}], "interacting": [], "argv": [[]], "profiles": ["clean", "all_valid"]}
    sb = g12_sandbox()
    row, p = g12m(sb, inv=broken)
    check(row[1] == "FAIL" and "mostly untestable" in row[2], "a mostly untestable matrix: %s" % (row,))
    one = {"schema": 2, "variables": [{"name": "MAKE_LEAF_FAIL", "kind": "text", "valid": ["1"], "hostile": [], "read_at": "x"},
                                      {"name": "CORTEX_LOG", "kind": "path", "valid": ["/home/gate/x.log"], "hostile": [], "read_at": "x"},
                                      {"name": "CORTEX_PORT", "kind": "numeric", "valid": ["8601"], "hostile": [], "read_at": "x"},
                                      {"name": "HOME", "kind": "path", "valid": ["/home/gate"], "hostile": [], "read_at": "x"}], "interacting": [], "argv": [[]], "profiles": ["clean", "all_valid"]}
    sb = g12_sandbox()
    row, p = g12m(sb, inv=one)
    check(row[1] == "PASS" and "untestable (MAKE_LEAF_FAIL)" in row[2], "one untestable variable must be listed: %s" % (row,))
    lines = [l.split("\t") for l in matrix_receipt(sb).splitlines() if "\t" in l]
    profile = [l for l in lines if l[2] == "argv" and "all_valid" in l[3]]
    check(profile and all(l[4] == "ACCEPT" for l in profile), "the untestable variable was not kept out of the profile: %s" % profile[:1])


@cell("GR-94 the marker lines must be in the builder's EXACT forms (plain or guarded, three spaces before # @include, LF): both forms pass; two spaces, a tab, CRLF, text after the marker or two names are refused as a marker the builder would not accept")
def gr94():
    plain = ('#!/bin/sh\nset -eu\numask 077\nhere=x\n. "$here/install-prereq.sh"   # @include\n. "$here/install-fetch.sh"   # @include\necho done\n')
    sb = g12_sandbox(dev_install=plain)
    row, p = g12m(sb)
    check(row[1] == "PASS", "the plain form: %s" % (row,))
    sb = g12_sandbox()
    row, p = g12m(sb)
    check(row[1] == "PASS", "the guarded form: %s" % (row,))
    bad = {"two spaces": plain.replace('"   # @include', '"  # @include', 1), "a tab": plain.replace('"   # @include', '"\t# @include', 1),
           "CRLF": plain.replace('"   # @include\n', '"   # @include\r\n', 1), "text after": plain.replace('# @include\n', '# @include now\n', 1),
           "no newline at the end": plain.replace('echo done\n', '. "$here/install-fetch.sh"   # @include')}
    for name, text in bad.items():
        sb = g12_sandbox(dev_install=text)
        row, p = g12m(sb)
        check(row[1] == "FAIL" and "would not accept" in row[2], "%s: %s" % (name, row))


# ---- increment R13: vera's review of 165b0ee (nine medium findings), one RED cell each ----------------------------------------------
def driver(sb, rules, body, timeout=30):
    """Run `body` in a fresh python -I that has the runner loaded as `m`, with the fakes first on PATH. Returns (completed process or None on timeout, seconds)."""
    json.dump({"rules": rules}, open(os.path.join(sb, "scenario.json"), "w"))
    env = {"PATH": FAKES + ":/usr/bin:/bin", "FAKE_LOG": os.path.join(sb, "fake.log"), "FAKE_SCENARIO": os.path.join(sb, "scenario.json"), "HOME": sb}
    code = ("import importlib.util, json, sys, time\nspec = importlib.util.spec_from_file_location('g', %r)\nm = importlib.util.module_from_spec(spec)\n"
            "spec.loader.exec_module(m)\n" % RUNNER) + body
    t0 = time.monotonic()
    try:
        p = subprocess.run(["/usr/bin/python3", "-I", "-c", code], env=env, capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return None, time.monotonic() - t0
    return p, time.monotonic() - t0


def replace_rule(rules, match, new):
    out, hit = [], False
    for x in rules:
        if x["match"] == match:
            out.append(new)
            hit = True
        else:
            out.append(x)
    check(hit, "no rule with the match %r to replace" % match)
    return out


@cell("GR-103 vera A-F1: a PTY command that keeps writing is still stopped at the deadline (it is checked on every pass, not only when the terminal is quiet) and its transcript is bounded")
def gr103():
    sb = sandbox()
    r = [rule(r"^flood$", "", 0, pty_steps=[{"print": "start\r\n"}, {"flood": 20}])]
    p, took = driver(sb, r, "s = m.Ssh('t')\nrc, tr, ev, to = s.run_pty('flood', 1.5)\nprint(json.dumps([rc, len(tr), to]))\n", timeout=15)
    check(p is not None and took < 8, "a command that keeps writing was not stopped at its deadline (%.1f s, driver %s)" % (took, "timed out" if p is None else "ended"))
    rc, size, timed_out = json.loads(p.stdout)
    check(timed_out is True and rc == 124 and size > 0, "rc %s, timed_out %s, %s bytes" % (rc, timed_out, size))
    body = "m.PTY_TRANSCRIPT_CAP = 20000\ns = m.Ssh('t')\nrc, tr, ev, to = s.run_pty('flood', 30)\nprint(json.dumps([rc, len(tr), to, b'transcript limit' in tr]))\n"
    p, took = driver(sb, r, body, timeout=20)
    check(p is not None and took < 10, "the transcript was not bounded: the flood ran %.1f s" % took)
    rc, size, timed_out, marked = json.loads(p.stdout)
    check(timed_out is True and rc == 124 and size <= 20000 + 4096 + 200 and marked, "cap: rc %s timed_out %s size %s marked %s" % (rc, timed_out, size, marked))


@cell("GR-104 vera A-F2: a command that ignores SIGTERM, or whose detached child keeps the pipes open, still returns rc 124 within the escalation bound, with the output read so far")
def gr104():
    sb = sandbox()
    pidfile = os.path.join(sb, "hang.pid")
    for name, rl in (("ignores SIGTERM", rule(r"^hang$", "", 0, early_out="partial-a\n", sleep=25, ignore_term=True, pid_to=pidfile)),
                     ("a detached child holds the pipes", rule(r"^hang$", "", 0, early_out="partial-b\n", orphan=10, sleep=25))):
        body = "m.Ssh.GRACE = 0.5\ns = m.Ssh('t')\nrc, out, err = s.run('hang', timeout=1)\nprint(json.dumps([rc, out.decode(), err.decode()]))\n"
        p, took = driver(sb, [rl], body, timeout=14)
        check(p is not None and took < 6, "%s: not bounded (%.1f s, driver %s)" % (name, took, "timed out" if p is None else "ended"))
        rc, out, err = json.loads(p.stdout)
        check(rc == 124 and "timed out" in err and out.startswith("partial-"), "%s: rc %s out %r err %r" % (name, rc, out, err))
        if name == "ignores SIGTERM":
            time.sleep(0.5)
            alive = True
            try:
                os.kill(int(open(pidfile).read()), 0)
            except OSError:
                alive = False
            check(not alive, "the command that ignores SIGTERM was left running (no kill after the grace period)")


def with_rules(rules, *pairs):
    for match, new in pairs:
        rules = replace_rule(rules, match, new)
    return rules


@cell("GR-105 vera A-F3: G1 rejects an abnormal probe failure (ssh 255, timeout 124, command not found 127) instead of reading it as absence, for every probe; a normal 'not found' (rc 1, or ls rc 2) still passes")
def gr105():
    probes = {"cargo": r"^command -v cargo$", "podman": r"^command -v podman$", "rustc": r"^command -v rustc$", "ports": r"^ss -ltnH$", "os_release": r"^cat /etc/os-release$",
              "rocky_pkgs": r"^rpm -q rocky-release rocky-gpg-keys", "gpg_pubkeys": r"^rpm -qa 'gpg-pubkey\*'", "podman_dirs": DIRS_RULE,
              "user_units": UNITS_RULE, "rocky_keys": r"^sha256sum /etc/pki/rpm-gpg/RPM-GPG-KEY-Rocky-"}
    sb = sandbox()
    run(sb, base_args(sb))
    check(tsv(sb)["G1"][1] == "PASS", "the clean machine does not pass G1: %s" % (tsv(sb)["G1"],))
    for name, match in probes.items():
        for rc in (255, 127):
            sb = sandbox()
            run(sb, base_args(sb), rules=with_rules(clean_rules(), (match, rule(match, "", rc, err="ssh: connect to host gate-vm: Connection refused\n"))))
            row = tsv(sb)["G1"]
            check(row[1] == "FAIL" and "abnormal" in row[2] and name in row[2], "%s rc %d: %s" % (name, rc, row))


@cell("GR-106 vera A-F4: G4 FAILS when a token it must hold cannot be collected (rc not 0 or empty), and G8 then cannot PASS: the known-secret inventory must be complete")
def gr106():
    lead_match, issued_match = r"^cat .*gate-lead\.token", r"^python3 -c .*sys\.stdout\.write"
    for name, match, word in (("lead token", lead_match, "lead's token"), ("issued token", issued_match, "issued token")):
        for rl in (rule(match, "", 255, err="ssh: connection lost\n"), rule(match, "", 0)):
            sb, r = g4_sandbox()
            r = [rl] + r
            run(sb, base_args(sb, only="G1,G2,G9,G4,G8"), rules=r)
            rows = tsv(sb)
            check(rows["G4"][1] == "FAIL" and "could not be collected" in rows["G4"][2] and word in rows["G4"][2], "%s rc %d: G4 %s" % (name, rl["rc"], rows["G4"][1:3]))
            check(rows["G8"][1] == "FAIL" and "known-secret inventory" in rows["G8"][2], "%s rc %d: G8 %s" % (name, rl["rc"], rows["G8"][1:3]))


@cell("GR-107 vera B-F1: a development installer with the same marker twice is a FAIL (duplicate), even when the shipped file, its hash and the record all agree; the independent splice is never silently skipped")
def gr107():
    first = '[ ! -f "$here/install-prereq.sh" ] || . "$here/install-prereq.sh"   # @include\n'
    dup = DEV_INSTALL.replace(first, first + first)
    incs = {"install-prereq.sh": INC_PREREQ, "install-fetch.sh": INC_FETCH}
    h1, h2 = hashlib.sha256(INC_PREREQ).hexdigest(), hashlib.sha256(INC_FETCH).hexdigest()
    record = {"includes": [{"name": "install-prereq.sh", "sha256": h1}, {"name": "install-prereq.sh", "sha256": h1}, {"name": "install-fetch.sh", "sha256": h2}]}
    for name, shipped in (("shipped equals the splice", join_text(dup.encode(), incs)), ("shipped differs from the splice", join_text(dup.encode(), incs).replace(b"echo done", b"echo dine"))):
        sb = g12_sandbox(dev_install=dup, shipped=shipped, record=record)
        row, p = g12_row(sb)
        check(row[1] == "FAIL" and "duplicate" in row[2].lower(), "%s: %s" % (name, row))
    sb = g12_sandbox(dev_install=dup)   # a VALID record (distinct names): only the development installer repeats a marker
    row, p = g12_row(sb)
    check(row[1] == "FAIL" and "duplicate include marker" in row[2], "a duplicated marker with a valid record: %s" % (row,))
    bad_marker = DEV_INSTALL.replace('. "$here/install-fetch.sh"', '. "$here/install-fetch.sh" "$here/install-prereq.sh"')
    row, p = g12_row(g12_sandbox(dev_install=bad_marker))
    check(row[1] == "FAIL" and "could not be rebuilt" in row[2], "a marker the builder would not accept: the second method must say it could not rebuild: %s" % (row,))


@cell("GR-108 vera B-F2: G12 requires its three git reads to succeed: a development tree that is not a repository (rc 128, empty output) with an empty commit and tree in the record is a FAIL naming git, never a PASS")
def gr108():
    sb = g12_sandbox(record={"commit": "0" * 40, "tree": "0" * 40})   # valid-shaped ids, so the record itself is accepted and the failed git reads are what is judged
    shutil.rmtree(os.path.join(sb, "dev", ".git"))
    row, p = g12_row(sb)
    check(row[1] == "FAIL" and all(w in row[2] for w in ("the git commit read failed (rc", "the git tree read failed (rc", "the git cleanliness read failed (rc")), "not a repository: %s" % (row,))
    sb = g12_sandbox(record={"commit": "abc", "tree": "def"})
    row, p = g12_row(sb)
    check(row[1] == "FAIL" and "commit" in row[2].lower(), "a short commit in the record: %s" % (row,))


@cell("GR-109 vera B-F3: G7 and G10 FAIL when the before or after state could not be read, even when both reads fail the same way (equal failed observations are not 'nothing changed')")
def gr109():
    bad_state = [{"out": state_text(rcs={"podman_ps": 125}), "rc": 0}]
    base_pk = "bash 5.2-1.el10.x86_64\nrocky-release 10.2-1.el10.noarch\n"
    cases = (("podman ps failed both times", dict(state=bad_state), "state"), ("the state probe printed nothing both times", dict(state=[{"out": "", "rc": 255}]), "state"), ("package list failed both times", dict(pk=[{"out": base_pk, "rc": 0}, {"out": "", "rc": 255}]), "package"))
    for cid, only in (("G10", "G1,G10"), ("G7", "G1,G7")):
        for name, kw, word in cases:
            sb, r = inj_sandbox(over={"no_linger": ([{"print": CONSENT, "read": True}, {"print": "problem: lingering is off for this user\r\nfix: run loginctl enable-linger\r\n"}], 2)})
            if "state" in kw:
                r = [x if not x["match"].startswith(STATE_RULE) else {"match": x["match"], "seq": kw["state"]} for x in r]
            if "pk" in kw:
                r = [x for x in r if not x["match"].startswith("^rpm -qa --qf")] + [{"match": r"^rpm -qa --qf", "seq": kw["pk"]}]
            run(sb, base_args(sb, only=only), rules=r)
            row = tsv(sb)[cid]
            check(row[1] == "FAIL" and "could not be read" in row[2] and word in row[2], "%s, %s: %s" % (cid, name, row))
    # control: podman is ABSENT on the clean machine (rc 127 every time): a stable, normal state, not an unreadable one
    sb, r = inj_sandbox()
    r = [x if not x["match"].startswith(STATE_RULE) else {"match": x["match"], "seq": [{"out": state_text(rcs={"podman_ps": 127}), "rc": 0}]} for x in r]
    run(sb, base_args(sb, only="G1,G10"), rules=r)
    check(tsv(sb)["G10"][1] == "PASS", "podman absent (rc 127) was read as an unreadable state: %s" % (tsv(sb)["G10"],))


@cell("GR-110 vera B-F4: G8 FAILS when only some of the release's container logs could be read, naming the container that was not read; it never scans a subset and says PASS")
def gr110():
    sb, r = leak_sandbox()
    r = [rule(r"^podman logs cortex-gate-migrate$", "", 1, err="Error: no such container\n")] + r
    run(sb, g8_args(sb), rules=r)
    row = tsv(sb)["G8"]
    check(row[1] == "FAIL" and "cortex-gate-migrate" in row[2] and "incomplete" in row[2], "G8 %s" % (row,))


@cell("GR-111 vera B-Q2: the G12 matrix receipt records the oracle's identity (tree, s5_check.py sha256, git commit and tree ids or 'none (git rc N)', and whether it is the development tree); another tree than the development tree is said so in the verdict")
def gr111():
    sb = g12_sandbox()
    row, p = g12m(sb)
    check(row[1] == "PASS", "G12 %s" % (row,))
    text = matrix_receipt(sb)
    oracle_file = os.path.join(sb, "oracle", "scripts", "s5_check.py")
    for want in ("oracle_tree=" + os.path.join(sb, "oracle"), "oracle_file_sha256=" + sha(oracle_file), "oracle_commit=" + git(os.path.join(sb, "oracle"), "rev-parse", "HEAD"), "oracle_tree_id=" + git(os.path.join(sb, "oracle"), "rev-parse", "HEAD^{tree}"), "oracle_is_development_tree=NO"):
        check(want in text, "the matrix receipt lacks %r: %s" % (want, text[:500]))
    check("sha256 %s" % sha(oracle_file)[:12] in row[2] and "ANOTHER tree" in row[2], "the verdict does not name the oracle: %s" % row[2])


@cell("GR-112 vera B-Q3: while the run-time matrix (layer L2) is MISSING, G12's PASS says 'L1 ONLY', the matrix receipt says layer_l2=MISSING, the table says so, and no overall PASS is possible even when all twelve rows pass")
def gr112():
    sb = g12_sandbox()
    row, p = g12m(sb)
    check(row[1] == "PASS" and row[2].startswith("L1 ONLY, layer L2"), "the G12 note: %s" % row[2][:120])
    check("layer_l2=MISSING" in matrix_receipt(sb) and "layer_l1=run" in matrix_receipt(sb), "the matrix receipt does not state the layers")
    table = open(os.path.join(sb, "ev", "verdict.txt")).read()
    check("G12 layer L2 (the run-time matrix) is MISSING" in table, "the table does not say L2 is missing: %s" % table[-400:])
    m = load_module()
    check(m.overall_of({g: ("PASS", "", []) for g in m.TABLE_ORDER}, False) == ("INCOMPLETE", m.EX_INCOMPLETE), "twelve PASS rows gave an overall PASS with L2 missing")
    check(m.overall_of({**{g: ("PASS", "", []) for g in m.TABLE_ORDER}, "G3": ("FAIL", "x", [])}, False) == ("FAIL", m.EX_FAIL), "a FAIL row did not give an overall FAIL")


# ---- increment R14: vera's review of 2696be2 (VR13-01 to VR13-04), one RED cell each -------------------------------------------------
def pid_alive(path):
    try:
        os.kill(int(open(path).read()), 0)
        return True
    except (OSError, ValueError):
        return False


@cell("GR-113 vera VR13-01: after a timeout the whole process group is killed even when the leader already exited on SIGTERM; a same-group descendant that ignores SIGTERM does not survive")
def gr113():
    sb = sandbox()
    pidfile = os.path.join(sb, "child.pid")
    r = [rule(r"^hang$", "", 0, early_out="partial-g\n", sleep=25, same_group_child={"pid_to": pidfile, "sleep": 25})]
    body = "m.Ssh.GRACE = 0.5\ns = m.Ssh('t')\nrc, out, err = s.run('hang', timeout=1)\nprint(json.dumps([rc, out.decode()]))\n"
    p, took = driver(sb, r, body, timeout=14)
    check(p is not None and took < 6, "the stop was not bounded (%.1f s, driver %s)" % (took, "timed out" if p is None else "ended"))
    rc, out = json.loads(p.stdout)
    check(rc == 124 and out.startswith("partial-g"), "rc %s out %r" % (rc, out))
    time.sleep(0.6)
    check(os.path.exists(pidfile), "the descendant never started")
    check(not pid_alive(pidfile), "a same-group descendant that ignores SIGTERM survived after its leader exited")


@cell("GR-114 vera VR13-02: a descendant that keeps writing to the terminal after the command has exited cannot hold the runner: the drain is bounded, the output so far is kept, the timeout is reported, the writer is stopped")
def gr114():
    sb = sandbox()
    pidfile = os.path.join(sb, "flooder.pid")
    r = [rule(r"^leaderexit$", "", 0, pty_steps=[{"print": "start\r\n"}, {"flood_child": 20, "flood_pid_to": pidfile}])]
    body = "m.Ssh.GRACE = 0.5\ns = m.Ssh('t')\nrc, tr, ev, to = s.run_pty('leaderexit', 1.5)\nprint(json.dumps([rc, len(tr), to, b'start' in tr, b'after the command ended' in tr]))\n"
    p, took = driver(sb, r, body, timeout=15)
    check(p is not None and took < 6, "the inherited writer held the runner (%.1f s, driver %s)" % (took, "timed out" if p is None else "ended"))
    rc, size, timed_out, kept, marked = json.loads(p.stdout)
    check(timed_out is True and rc == 124 and size > 0 and kept and marked, "rc %s timed_out %s size %s kept %s marked %s" % (rc, timed_out, size, kept, marked))
    time.sleep(0.6)
    check(os.path.exists(pidfile) and not pid_alive(pidfile), "the inherited writer was not stopped")


@cell("GR-115 vera VR13-03: G1, G7 and G10 tell VERIFIED ABSENCE from a read error: an unreadable or not-a-directory answer is a FAIL in all three even when it is the same before and after; genuine absence still passes")
def gr115():
    m1 = DIRS_RULE
    pre = "absent /home/gate/.local/share/containers\n"
    for name, out, word in (("unreadable", pre + "unreadable /home/gate/.config/containers\n", "could not read"), ("present", pre + "present /home/gate/.config/containers\n", "already exists"),
                            ("no answer", "", "no verified answer"), ("one line only", pre, "no verified answer"), ("garbage", pre + "maybe /home/gate/.config/containers\n", "no verified answer")):
        sb = sandbox()
        run(sb, base_args(sb), rules=with_rules(clean_rules(), (m1, rule(m1, out))))
        row = tsv(sb)["G1"]
        check(row[1] == "FAIL" and word in row[2], "G1 containers %s: %s" % (name, row))
    m2 = UNITS_RULE
    for name, out, word in (("unreadable", "@@user_units unreadable\n", "no verified answer"), ("not a directory", "@@user_units notdir\n", "no verified answer"), ("no sentinel", "", "no sentinel"),
                            ("ls failed", "@@user_units 2\n", "no verified answer"), ("a cortex unit", "cortex-server.service\n@@user_units 0\n", "cortex user unit")):
        sb = sandbox()
        run(sb, base_args(sb), rules=with_rules(clean_rules(), (m2, rule(m2, out))))
        row = tsv(sb)["G1"]
        check(row[1] == "FAIL" and word in row[2], "G1 user units %s: %s" % (name, row))
    asks = ([{"print": CONSENT, "read": True}, {"print": "problem: lingering is off for this user\r\nfix: run loginctl enable-linger\r\n"}], 2)
    for cid, only in (("G10", "G1,G10"), ("G7", "G1,G7")):
        for part, token in (("trial_home", "unreadable"), ("trial_home", "notdir"), ("user_units", "unreadable"), ("user_units", "2")):
            sb, r = inj_sandbox(over={"no_linger": asks})
            bad = [{"out": state_text(rcs={part: token}), "rc": 0}]
            r = [x if not x["match"].startswith(STATE_RULE) else {"match": x["match"], "seq": bad} for x in r]
            run(sb, base_args(sb, only=only), rules=r)
            row = tsv(sb)[cid]
            check(row[1] == "FAIL" and "could not be read" in row[2] and part in row[2], "%s %s %s: %s" % (cid, part, token, row))


@cell("GR-116 vera VR13-03: the directory probes, run in a REAL shell on a real tree, answer absent only for verified absence (parent missing, or entry missing in a searchable parent), present for an existing entry, unreadable under a parent that cannot be searched, notdir for a file")
def gr116():
    m = load_module()
    sb = sandbox()
    def sh(cmd, home):
        return subprocess.run(["/bin/sh", "-c", cmd], env={"PATH": "/usr/bin:/bin", "HOME": home}, capture_output=True, timeout=20).stdout.decode()
    def tree(name):
        h = os.path.join(sb, name)
        os.makedirs(h)
        return h
    c1, c2 = ".local/share/containers", ".config/containers"
    h = tree("h-none")
    check(sh(m.G1_DIRS, h) == "absent %s/%s\nabsent %s/%s\n" % (h, c1, h, c2), "no directories at all: %r" % sh(m.G1_DIRS, h))
    h = tree("h-present")
    os.makedirs(os.path.join(h, c1))
    os.makedirs(os.path.join(h, ".config"))
    check(sh(m.G1_DIRS, h) == "present %s/%s\nabsent %s/%s\n" % (h, c1, h, c2), "one present, one missing in a searchable parent: %r" % sh(m.G1_DIRS, h))
    h = tree("h-file")
    os.makedirs(os.path.join(h, ".config"))
    open(os.path.join(h, c2), "w").write("x")
    check("present %s/%s" % (h, c2) in sh(m.G1_DIRS, h), "an existing entry that is a file is present")
    h = tree("h-dangling")
    os.makedirs(os.path.join(h, ".config"))
    os.symlink(os.path.join(sb, "no-such-target"), os.path.join(h, c2))
    check("present %s/%s" % (h, c2) in sh(m.G1_DIRS, h), "a dangling symlink is an existing entry (present), not an absent one: %r" % sh(m.G1_DIRS, h))
    units = ".config/systemd/user"
    h = tree("u-none")
    check(sh(m.G1_UNITS, h) == "@@user_units absent\n", "no units directory: %r" % sh(m.G1_UNITS, h))
    h = tree("u-list")
    os.makedirs(os.path.join(h, units))
    open(os.path.join(h, units, "a.service"), "w").write("")
    open(os.path.join(h, units, "b.timer"), "w").write("")
    check(sh(m.G1_UNITS, h) == "a.service\nb.timer\n@@user_units 0\n", "a listing: %r" % sh(m.G1_UNITS, h))
    h = tree("u-file")
    os.makedirs(os.path.join(h, ".config", "systemd"))
    open(os.path.join(h, units), "w").write("x")
    check(sh(m.G1_UNITS, h) == "@@user_units notdir\n", "a file where the directory should be: %r" % sh(m.G1_UNITS, h))
    parts = m.parse_state(sh(m.TRIAL_STATE, tree("t-none")))
    check(sorted(parts) == sorted(m.STATE_PARTS) and parts["trial_home"][0] == "absent" and parts["user_units"][0] == "absent", "the trial-state probe on an empty home: %s" % parts)
    require_nonroot()   # root reads through mode 000: the unreadable cases need an ordinary user, and a root run must FAIL here, not skip (vera, euid provenance)
    h = tree("u-denied")
    os.makedirs(os.path.join(h, ".config", "systemd", "user"))
    os.chmod(os.path.join(h, ".config", "systemd"), 0)
    try:
        check(sh(m.G1_UNITS, h) == "@@user_units unreadable\n", "a parent that cannot be searched: %r" % sh(m.G1_UNITS, h))
    finally:
        os.chmod(os.path.join(h, ".config", "systemd"), 0o700)
    h = tree("d-denied")
    os.makedirs(os.path.join(h, ".config", "containers"))
    os.makedirs(os.path.join(h, ".local", "share"))
    os.chmod(os.path.join(h, ".config"), 0)
    try:
        check(sh(m.G1_DIRS, h) == "absent %s/%s\nunreadable %s/%s\n" % (h, c1, h, c2), "a container directory under a parent that cannot be searched: %r" % sh(m.G1_DIRS, h))
    finally:
        os.chmod(os.path.join(h, ".config"), 0o700)
    h = tree("t-denied")
    os.makedirs(os.path.join(h, "gate-trial-home"))
    open(os.path.join(h, "gate-trial-home", "f"), "w").write("")
    os.chmod(os.path.join(h, "gate-trial-home"), 0)
    try:
        check(m.parse_state(sh(m.TRIAL_STATE, h))["trial_home"][0] == "2", "an unreadable trial home must answer ls's own status 2, never absent")
    finally:
        os.chmod(os.path.join(h, "gate-trial-home"), 0o700)


# ---- increment R15: vera's review of 181ffd0 (VR14-01, VR14-02), one RED cell each -----------------------------------------------------
def require_nonroot():
    check(os.geteuid() != 0, "this cell needs a user that cannot read through mode 000 (euid %d): run it as an ordinary user" % os.geteuid())


@cell("GR-117 vera VR14-01: the directory probes walk the WHOLE ancestor chain from HOME: an inaccessible EARLIER ancestor (above the immediate parent, or HOME's own ancestor) is 'unreadable', never 'absent'; genuine absence still passes")
def gr117():
    require_nonroot()
    m = load_module()
    sb = sandbox()
    def sh(cmd, home):
        return subprocess.run(["/bin/sh", "-c", cmd], env={"PATH": "/usr/bin:/bin", "HOME": home}, capture_output=True, timeout=20).stdout.decode()
    def locked(path, fn):
        os.chmod(path, 0)
        try:
            return fn()
        finally:
            os.chmod(path, 0o700)
    h = os.path.join(sb, "h-local")
    os.makedirs(os.path.join(h, ".local", "share", "containers"))
    out = locked(os.path.join(h, ".local"), lambda: sh(m.G1_DIRS, h))
    check(out == "unreadable %s/.local/share/containers\nabsent %s/.config/containers\n" % (h, h), ".local cannot be searched, so .local/share/containers is unobserved: %r" % out)
    h = os.path.join(sb, "h-config")
    os.makedirs(os.path.join(h, ".config", "systemd", "user"))
    out = locked(os.path.join(h, ".config"), lambda: sh(m.G1_UNITS, h))
    check(out == "@@user_units unreadable\n", ".config cannot be searched, so the units directory two levels down is unobserved: %r" % out)
    out = locked(os.path.join(h, ".config"), lambda: sh(m.G1_DIRS, h))
    check(out.endswith("unreadable %s/.config/containers\n" % h), "the container directory under an unsearchable .config: %r" % out)
    h = os.path.join(sb, "ha", "b")
    os.makedirs(h)
    out = locked(os.path.join(sb, "ha"), lambda: sh(m.G1_UNITS, h))
    check(out == "@@user_units unreadable\n", "HOME's own ancestor cannot be searched: %r" % out)
    parts = locked(os.path.join(sb, "ha"), lambda: m.parse_state(sh(m.TRIAL_STATE, h)))
    check(parts["trial_home"][0] == "unreadable" and parts["user_units"][0] == "unreadable", "the trial-state probe when HOME's ancestor cannot be searched: %s" % {k: v[0] for k, v in parts.items()})
    # genuine absence stays absence: nothing exists, and a searchable HOME with the directories missing
    h = os.path.join(sb, "h-nothing")
    os.makedirs(h)
    check(sh(m.G1_UNITS, h) == "@@user_units absent\n" and sh(m.G1_DIRS, h) == "absent %s/.local/share/containers\nabsent %s/.config/containers\n" % (h, h), "genuine absence must stay absent")


@cell("GR-118 vera VR14-02: the before and after fingerprint keeps each directory's presence: absent and present-empty differ, in both directions, and a gate-trial-home that appears in the home listing is a change; unchanged absence and unchanged present-empty still pass")
def gr118():
    asks = ([{"print": CONSENT, "read": True}, {"print": "problem: lingering is off for this user\r\nfix: run loginctl enable-linger\r\n"}], 2)
    absent, empty = state_text(), state_text(rcs={"trial_home": "0"})
    cases = (("absent -> present-empty", absent, empty, True), ("present-empty -> absent", empty, absent, True),
             ("gate-trial-home appears in the home listing", absent, state_text(home_extra="gate-trial-home\n"), True),
             ("absent twice", absent, absent, False), ("present-empty twice", empty, empty, False))
    for cid, only in (("G10", "G1,G10"), ("G7", "G1,G7")):
        for name, before, after, changes in cases:
            sb, r = inj_sandbox(over={"no_linger": asks})
            seq = [{"out": before, "rc": 0}, {"out": after, "rc": 0}]
            r = [x if not x["match"].startswith(STATE_RULE) else {"match": x["match"], "seq": seq} for x in r]
            run(sb, base_args(sb, only=only), rules=r)
            row = tsv(sb)[cid]
            if changes:
                check(row[1] == "FAIL" and "state changed" in row[2], "%s, %s: %s" % (cid, name, row))
            else:
                check(row[1] == "PASS", "%s, %s was read as a change: %s" % (cid, name, row))


@cell("GR-123 vera VR15-01 (sam GR15-SAM01): G7 and G10 read the OUTER state probe's own status: a probe that failed (255) or timed out (124) FAILS the row even when its output is complete and equals the other observation, whether it is the before read, the after read or both; the status 0 control passes")
def gr123():
    asks = ([{"print": CONSENT, "read": True}, {"print": "problem: lingering is off for this user\r\nfix: run loginctl enable-linger\r\n"}], 2)
    good = state_text()
    for cid, only in (("G10", "G1,G10"), ("G7", "G1,G7")):
        for rc in (255, 124):
            for where in ("before", "after", "both"):
                seq = [{"out": good, "rc": rc if where in ("before", "both") else 0}, {"out": good, "rc": rc if where in ("after", "both") else 0}]
                sb, r = inj_sandbox(over={"no_linger": asks})
                r = [x if not x["match"].startswith(STATE_RULE) else {"match": x["match"], "seq": seq} for x in r]
                run(sb, base_args(sb, only=only), rules=r)
                row = tsv(sb)[cid]
                check(row[1] == "FAIL" and "could not be read" in row[2] and ("exited %d" % rc) in row[2], "%s, outer status %d on the %s read: %s" % (cid, rc, where, row))
        sb, r = inj_sandbox(over={"no_linger": asks})
        r = [x if not x["match"].startswith(STATE_RULE) else {"match": x["match"], "seq": [{"out": good, "rc": 0}]} for x in r]
        run(sb, base_args(sb, only=only), rules=r)
        check(tsv(sb)[cid][1] == "PASS", "%s, the status 0 control: %s" % (cid, tsv(sb)[cid]))


# ---- increment R16: the strict binding-record validator (schema r3 section 2), rules 1 to 9, test first ------------------------------
SPLICE_SENTENCE = "each '# @include' marker line, newline included, is replaced by the exact bytes of its include; nothing is added"
SAMPLE_RECORD = os.path.join(HERE, "fixtures", "g12-sample-record-from-release-fixes-fixture.json")
SAMPLE_SHA256 = "7d9ac2139c6b729d5f706f96a31c975cac2ed7560af926ce73207a50710a9e5e"   # sophi's emitter, a FIXTURE (ids all c and d), not a release


def good_record(**over):
    d = {"schema": 1, "object_format": "sha1", "commit": "c" * 40, "tree": "d" * 40,
         "includes": [{"name": "install-prereq.sh", "sha256": "a" * 64}, {"name": "install-fetch.sh", "sha256": "b" * 64}],
         "splice": SPLICE_SENTENCE, "joined_sha256": "e" * 64}
    d.update(over)
    for k in [k for k, v in d.items() if v is KEY_ABSENT]:
        del d[k]
    return d


KEY_ABSENT = object()


@cell("GR-119 schema r3 rules 1 to 5: validate_record accepts the exact seven-key record and rejects, each with its own reason, a duplicate JSON key, a missing key, an unknown key, schema as a boolean or 2 or a string, a bad object_format, ids of the wrong width or case, empty or malformed includes, a repeated or unknown include name, a wrong splice sentence and a short joined hash")
def gr119():
    m = load_module()
    rec, fails = m.validate_record(json.dumps(good_record()).encode())
    check(rec is not None and not fails, "the good record was refused: %s" % fails)
    sha256_rec = good_record(object_format="sha256", commit="c" * 64, tree="d" * 64)
    rec, fails = m.validate_record(json.dumps(sha256_rec).encode())
    check(rec is not None and not fails, "a sha256-format record with 64-digit ids was refused: %s" % fails)
    dup = json.dumps(good_record()).replace('"schema": 1,', '"schema": 1, "schema": 1,', 1)
    bad = {"duplicate key": (dup.encode(), "duplicate"),
           "not JSON": (b"{not json", "JSON"), "not UTF-8": (b"\xff\xfe", "UTF-8"), "a list": (b"[1]", "top level"),
           "missing key": (json.dumps(good_record(splice=KEY_ABSENT)).encode(), "missing"),
           "unknown key": (json.dumps(good_record(extra="x")).encode(), "unknown"),
           "schema true": (json.dumps(good_record(schema=True)).encode(), "schema"), "schema 2 without its anchor key": (json.dumps(good_record(schema=2)).encode(), "anchor"), "schema 3": (json.dumps(good_record(schema=3)).encode(), "schema"),
           "schema string": (json.dumps(good_record(schema="1")).encode(), "schema"), "schema float 1.0": (json.dumps(good_record(schema=1.0)).encode(), "schema"),
           "object_format md5": (json.dumps(good_record(object_format="md5")).encode(), "object_format"),
           "object_format null": (json.dumps(good_record(object_format=None)).encode(), "object_format"),
           "commit upper case": (json.dumps(good_record(commit="C" * 40)).encode(), "commit"), "commit short": (json.dumps(good_record(commit="c" * 39)).encode(), "commit"),
           "commit 64 digits in a sha1 record": (json.dumps(good_record(commit="c" * 64)).encode(), "commit"),
           "commit 40 digits in a sha256 record": (json.dumps(good_record(object_format="sha256", tree="d" * 64)).encode(), "commit"),
           "tree number": (json.dumps(good_record(tree=7)).encode(), "tree"),
           "includes empty": (json.dumps(good_record(includes=[])).encode(), "includes"), "includes not a list": (json.dumps(good_record(includes="x")).encode(), "includes"),
           "include element extra key": (json.dumps(good_record(includes=[{"name": "install-prereq.sh", "sha256": "a" * 64, "x": 1}])).encode(), "includes"),
           "include element missing sha": (json.dumps(good_record(includes=[{"name": "install-prereq.sh"}])).encode(), "includes"),
           "include sha upper case": (json.dumps(good_record(includes=[{"name": "install-prereq.sh", "sha256": "A" * 64}])).encode(), "includes"),
           "include sha short": (json.dumps(good_record(includes=[{"name": "install-prereq.sh", "sha256": "a" * 63}])).encode(), "includes"),
           "include name repeated": (json.dumps(good_record(includes=[{"name": "install-prereq.sh", "sha256": "a" * 64}, {"name": "install-prereq.sh", "sha256": "b" * 64}])).encode(), "duplicate"),
           "include name unknown": (json.dumps(good_record(includes=[{"name": "install-evil.sh", "sha256": "a" * 64}])).encode(), "unknown include"),
           "splice another sentence": (json.dumps(good_record(splice="each marker is replaced by its include")).encode(), "splice"),
           "splice with a trailing space": (json.dumps(good_record(splice=SPLICE_SENTENCE + " ")).encode(), "splice"),
           "joined short": (json.dumps(good_record(joined_sha256="e" * 63)).encode(), "joined_sha256")}
    for name, (raw, word) in bad.items():
        rec, fails = m.validate_record(raw)
        check(rec is None and fails and any(word in f for f in fails), "%s: %s %s" % (name, rec is None, fails))


@cell("GR-120 the REAL emitter's sample record (sophi, a fixture: ids all c and d, sha256 pinned here) passes validate_record, and G12 then refuses it only for what is true of a fixture (its commit, tree and hashes are not this tree's), never as an invalid record")
def gr120():
    raw = open(SAMPLE_RECORD, "rb").read()
    check(hashlib.sha256(raw).hexdigest() == SAMPLE_SHA256, "the sample file is not the pinned bytes of sophi's fixture record")
    m = load_module()
    rec, fails = m.validate_record(raw)
    check(rec is not None and not fails, "the real emitter's record fails the strict validator: %s" % fails)
    sb = g12_sandbox()
    open(os.path.join(sb, "binding.json"), "wb").write(raw)
    row, p = g12_row(sb)
    check(row[1] == "FAIL" and "commit" in row[2].lower() and "binding record is not valid" not in row[2], "G12 on the sample: %s" % (row,))


@cell("GR-121 G12 refuses an invalid binding record BEFORE it trusts anything in it: a duplicate key, an unknown key, a boolean schema, a wrong splice sentence, a name the builder does not know; the note says the record is not valid")
def gr121():
    good = good_record(commit="0" * 40, tree="0" * 40)
    for name, raw, word in (("duplicate key", json.dumps(good).replace('"schema": 1,', '"schema": 1, "schema": 1,', 1), "duplicate"),
                            ("unknown key", json.dumps(dict(good, note="x")), "unknown"), ("boolean schema", json.dumps(dict(good, schema=True)), "schema"),
                            ("another splice sentence", json.dumps(dict(good, splice="x")), "splice"), ("an include name nobody knows", json.dumps(dict(good, includes=[{"name": "install-evil.sh", "sha256": "a" * 64}])), "unknown include")):
        sb = g12_sandbox()
        open(os.path.join(sb, "binding.json"), "w").write(raw)
        row, p = g12_row(sb)
        check(row[1] == "FAIL" and "binding record is not valid" in row[2] and word in row[2], "%s: %s" % (name, row))


@cell("GR-122 the builder's include rules and the rest of schema r3: an include with a CR, a shebang, the marker text, no function, or that is a symlink is a FAIL; a record whose object_format is not the tree's is a FAIL; a record that changes while the checker runs is a FAIL; a PASS says the binding schema is UNFROZEN")
def gr122():
    base = {"install-fetch.sh": INC_FETCH}
    cases = {"a CR": ({"install-prereq.sh": b"prereq_msg() {\r\n  echo prereq\r\n}\n", **base}, "carriage return"),
             "a shebang": ({"install-prereq.sh": b"#!/bin/sh\nprereq_msg() {\n  echo prereq\n}\n", **base}, "shebang"),
             "the marker text": ({"install-prereq.sh": b"prereq_msg() {\n  : # @include\n}\n", **base}, ("marker text", "still holds an include marker")),
             "no function": ({"install-prereq.sh": b"(\n  :\n)\n", **base}, "function")}
    for name, (incs, word) in cases.items():
        sb = g12_sandbox(includes=incs)
        row, p = g12_row(sb)
        words = word if isinstance(word, tuple) else (word,)
        check(row[1] == "FAIL" and all(w in row[2].lower() for w in words), "%s: %s" % (name, row))
    sb = g12_sandbox()   # the development install.sh itself a symlink to a real file
    real = os.path.join(sb, "dev", "install.real")
    os.replace(os.path.join(sb, "dev", "deploy", "install.sh"), real)
    os.symlink(real, os.path.join(sb, "dev", "deploy", "install.sh"))
    row, p = g12_row(sb)
    check(row[1] == "FAIL" and "development install.sh is not a regular file" in row[2], "a symlink development installer: %s" % (row,))
    sb = g12_sandbox(symlinks=("install-fetch.sh",))
    row, p = g12_row(sb)
    check(row[1] == "FAIL" and "regular file" in row[2], "a symlink include: %s" % (row,))
    sb = g12_sandbox(record={"object_format": "sha256", "commit": "0" * 64, "tree": "0" * 64})
    row, p = g12_row(sb)
    check(row[1] == "FAIL" and "object format" in row[2], "the record says sha256, the tree is sha1: %s" % (row,))
    sb = g12_sandbox(drift_record=True)
    row, p = g12_row(sb)
    check(row[1] == "FAIL" and "bytes changed" in row[2], "the record changed while the checker ran: %s" % (row,))
    sb = g12_sandbox()
    row, p = g12_row(sb)
    check(row[1] == "PASS" and "binding schema UNFROZEN" in row[2], "a PASS must say the schema is unfrozen: %s" % (row,))
    check("binding_schema=UNFROZEN" in open(os.path.join(sb, "ev", [f for f in os.listdir(os.path.join(sb, "ev")) if "-G12-binding" in f and f.endswith(".out")][0])).read(), "the binding receipt does not state the schema status")


# ---- increment R18: sam's review of R17 d8de84d (GR17-SAM01, GR17-SAM02), one RED cell each ----------------------------------------------
@cell("GR-124 sam GR17-SAM01: a non-regular input is refused BEFORE it is opened: a FIFO with no writer (it would block an open or a read) as the development installer, as an include, as the shipped installer or as the binding record, and a symlink to a FIFO as the development installer or an include, each end in a FAIL naming the type, promptly, and no reader is ever reached; the regular control PASSES")
def gr124():
    def fifo(path):
        os.remove(path)
        os.mkfifo(path)
    def to_fifo_link(path):
        os.remove(path)
        target = path + ".fifo"
        os.mkfifo(target)
        os.symlink(target, path)
    cases = (("the development installer is a FIFO", lambda sb: fifo(os.path.join(sb, "dev", "deploy", "install.sh")), "development install.sh"),
             ("an include is a FIFO", lambda sb: fifo(os.path.join(sb, "dev", "deploy", "install-fetch.sh")), "install-fetch.sh"),
             ("the shipped installer is a FIFO", lambda sb: fifo(os.path.join(sb, "art", "install.sh")), "shipped install.sh"),
             ("the binding record is a FIFO", lambda sb: fifo(os.path.join(sb, "binding.json")), "binding record"),
             ("the development installer is a symlink to a FIFO", lambda sb: to_fifo_link(os.path.join(sb, "dev", "deploy", "install.sh")), "development install.sh"),
             ("an include is a symlink to a FIFO", lambda sb: to_fifo_link(os.path.join(sb, "dev", "deploy", "install-prereq.sh")), "install-prereq.sh"))
    for name, mutate, word in cases:
        sb = g12_sandbox()
        mutate(sb)
        t0 = time.monotonic()
        run(sb, g12m_args(sb), timeout=40)   # a reader that reaches the FIFO blocks here and the call raises TimeoutExpired: a cell error, never a pass
        took = time.monotonic() - t0
        row = tsv(sb)["G12"]
        check(row[1] == "FAIL" and "not a regular file" in row[2] and word in row[2] and took < 30, "%s: %s (%.1f s)" % (name, row, took))
    sb = g12_sandbox()
    row, p = g12_row(sb)
    check(row[1] == "PASS", "the regular control: %s" % (row,))


@cell("GR-125 sam GR17-SAM02: G12 requires BOTH builder-known markers exactly once, independently of the record: a development installer that omits one known include (with the record and the shipped bytes coherent with the omission, and ordinary syntax, git, checker and matrix results) FAILS with the missing marker named, for each include; a marker for an include the builder does not know FAILS; the complete two-marker control PASSES")
def gr125():
    prereq_line = '[ ! -f "$here/install-prereq.sh" ] || . "$here/install-prereq.sh"   # @include\n'
    fetch_line = '[ ! -f "$here/install-fetch.sh" ] || . "$here/install-fetch.sh"   # @include\n'
    for omitted, kept_line, kept_name, kept in (("install-fetch.sh", None, "install-prereq.sh", INC_PREREQ), ("install-prereq.sh", None, "install-fetch.sh", INC_FETCH)):
        line = fetch_line if omitted == "install-fetch.sh" else prereq_line
        dev = DEV_INSTALL.replace(line, "")
        check(dev != DEV_INSTALL, "the fixture line was not found")
        sb = g12_sandbox(dev_install=dev, includes={kept_name: kept})
        row, p = g12_row(sb)
        check(row[1] == "FAIL" and "include marker missing: %s" % omitted in row[2], "omitting %s coherently: %s" % (omitted, row))
    evil = DEV_INSTALL.replace(fetch_line, fetch_line + '. "$here/install-evil.sh"   # @include\n')
    shipped = join_text(evil.encode(), {"install-prereq.sh": INC_PREREQ, "install-fetch.sh": INC_FETCH, "install-evil.sh": b"evil_fn() {\n  :\n}\n"})
    sb = g12_sandbox(dev_install=evil, shipped=shipped)   # the record names only the two known includes: the development installer alone names a third
    row, p = g12_row(sb)
    check(row[1] == "FAIL" and "the builder does not know: install-evil.sh" in row[2], "an include the builder does not know: %s" % (row,))
    row, p = g12_row(g12_sandbox())
    check(row[1] == "PASS", "the complete control: %s" % (row,))


# ---- STAGE B: schema 2, the approved key file and the anchor (r4, accepted for implementation) -------------------------------------------------
import base64

KEYGEN = "/usr/bin/ssh-keygen"
PRINCIPAL = "release@example.invalid"
SPLICE2 = ("each '# @include' marker line, newline included, is replaced by the exact bytes of its include; then each of the three empty anchor statements "
           "CORTEX_ANCHOR_PRINCIPAL='', CORTEX_ANCHOR_KEY='', CORTEX_ANCHOR_FPR='' is replaced by the same statement holding its recorded value in single quotes; nothing else is added or changed")
ANCHOR_NAMES = ("CORTEX_ANCHOR_PRINCIPAL", "CORTEX_ANCHOR_KEY", "CORTEX_ANCHOR_FPR")
EMPTY3 = "CORTEX_ANCHOR_PRINCIPAL=''\nCORTEX_ANCHOR_KEY=''\nCORTEX_ANCHOR_FPR=''\n"
DEV_ANCHOR = ('#!/bin/sh\n# the development installer\n\n' + EMPTY3 + 'set -eu\numask 077\nhere=x\n'
              '[ ! -f "$here/install-prereq.sh" ] || . "$here/install-prereq.sh"   # @include\n[ ! -f "$here/install-fetch.sh" ] || . "$here/install-fetch.sh"   # @include\necho done\n')
_KEYS = {}


def throwaway(index=0):
    """A THROWAWAY ed25519 public key blob (68 characters), made once per index in a private directory on the data disk; the private half is never read by any cell."""
    if index not in _KEYS:
        d = tempfile.mkdtemp(prefix="gr-key.", dir=TMP)
        SANDBOXES.append(d)
        subprocess.run([KEYGEN, "-q", "-t", "ed25519", "-N", "", "-C", "gr-throwaway", "-f", os.path.join(d, "k")], check=True, capture_output=True)
        _KEYS[index] = open(os.path.join(d, "k.pub")).read().split()[1]
    return _KEYS[index]


def independent_fpr(key):
    """The fingerprint computed by the CELL itself (sha256 of the decoded blob, base64 without padding), not by ssh-keygen."""
    return "SHA256:" + base64.b64encode(hashlib.sha256(base64.b64decode(key)).digest()).decode().rstrip("=")


def anchor_values(index=0, principal=PRINCIPAL):
    key = throwaway(index)
    return {"principal": principal, "key": key, "fpr": independent_fpr(key)}


def anchor_final(dev_text, includes, v):
    """T1 then T2, written independently of the runner."""
    out = join_text(dev_text.encode(), includes)
    for name, field in (("CORTEX_ANCHOR_PRINCIPAL", "principal"), ("CORTEX_ANCHOR_KEY", "key"), ("CORTEX_ANCHOR_FPR", "fpr")):
        out = out.replace(("%s=''\n" % name).encode(), ("%s='%s'\n" % (name, v[field])).encode(), 1)
    return out


def anchor_sandbox(dev=None, includes=None, shipped_fn=None, key_buffer=None, record=None, anchor=None, shipped_values=None, drift_cmd=""):
    """A G12 sandbox whose development installer carries the three empty anchor statements, whose shipped file is the independently built FINAL bytes, whose record is a correct
    schema-2 record, and whose approved key file is sb/approved.pub. Each parameter plants ONE fault."""
    dev = DEV_ANCHOR if dev is None else dev
    incs = includes or {"install-prereq.sh": INC_PREREQ, "install-fetch.sh": INC_FETCH}
    v = anchor_values()
    buf = ("%s ssh-ed25519 %s\n" % (v["principal"], v["key"])).encode() if key_buffer is None else key_buffer
    shipped = anchor_final(dev, incs, shipped_values or v)
    if shipped_fn:
        shipped = shipped_fn(shipped)
    rec = {"schema": 2, "splice": SPLICE2, "anchor": dict({"principal": v["principal"], "key": v["key"], "fpr": v["fpr"], "key_input_sha256": hashlib.sha256(
        ("%s ssh-ed25519 %s\n" % (v["principal"], v["key"])).encode()).hexdigest()}, **(anchor or {}))}
    rec.update(record or {})
    sb = g12_sandbox(dev_install=dev, includes=includes, shipped=shipped, record=rec, drift_cmd=drift_cmd)
    open(os.path.join(sb, "approved.pub"), "wb").write(buf)
    return sb


def anchor_args(sb, extra=()):
    return g12m_args(sb) + ["--approved-key-file", os.path.join(sb, "approved.pub")] + list(extra)


def anchor_row(sb, extra=(), timeout=60):
    p = run(sb, anchor_args(sb, extra), timeout=timeout)
    return tsv(sb)["G12"], p


def script(sb, name, body):
    path = os.path.join(sb, name)
    open(path, "w").write("#!/bin/sh\n" + body)
    os.chmod(path, 0o755)
    return path


def scratch_dirs(sb):
    return [d for d in os.listdir(sb) if d.startswith(".anchor-")]


@cell("GR-126 r4 section 4 (1), the approved key file is ONE bounded buffer: empty, a second line, two trailing LFs, another key type, a private-key header, a bad, over-long or hyphen-led principal, a short or wrongly prefixed key, a CR, a NUL, non-ASCII, over 1024 bytes, two fields, four fields, a symlink, a directory, a FIFO and a missing path are each REFUSED before anything else; one trailing LF and none are accepted")
def gr126():
    key = throwaway()
    good = "%s ssh-ed25519 %s" % (PRINCIPAL, key)
    bad = {"empty": (b"", "is empty"), "a second line": ((good + "\nsecond\n").encode(), "more than one line"), "two trailing LFs": ((good + "\n\n").encode(), "more than one line"),
           "another key type": (("%s ssh-rsa %s\n" % (PRINCIPAL, key)).encode(), "key type"), "a private-key header": (b"-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----\n", "private-key header"),
           "a bad principal": (("rel$ease ssh-ed25519 %s\n" % key).encode(), "principal"), "a principal over 64": (("p" * 65 + " ssh-ed25519 %s\n" % key).encode(), "principal"),
           "a hyphen-led principal": (("-release ssh-ed25519 %s\n" % key).encode(), "principal"), "a short key": (("%s ssh-ed25519 %s\n" % (PRINCIPAL, key[:-1])).encode(), "68 characters"),
           "a wrongly prefixed key": (("%s ssh-ed25519 BBBBC3%s\n" % (PRINCIPAL, key[6:])).encode(), "68 characters"), "a CR": ((good + "\r\n").encode(), "CR"),
           "a NUL": ((good + "\n").encode().replace(b"example", b"exa\x00mple"), "NUL"), "non-ASCII": ((good + "\n").replace("example", "exämple").encode("utf-8"), "not ASCII"),
           "over 1024 bytes": ((good + " " + "x" * 1100 + "\n").encode(), "over 1024"), "two fields": (("ssh-ed25519 %s\n" % key).encode(), "three space-separated fields"),
           "four fields": ((good + " comment\n").encode(), "three space-separated fields")}
    for name, (buf, word) in bad.items():
        sb = anchor_sandbox(key_buffer=buf)
        row, p = anchor_row(sb)
        check(row[1] == "FAIL" and "the approved key file is not valid" in row[2] and word in row[2], "%s: %s" % (name, row))
    sb = anchor_sandbox()
    link = os.path.join(sb, "approved.pub")
    os.rename(link, link + ".real")
    os.symlink(link + ".real", link)
    row, p = anchor_row(sb)
    check(row[1] == "FAIL" and "approved key file is not a regular file" in row[2], "a symlink: %s" % (row,))
    sb = anchor_sandbox()
    os.remove(os.path.join(sb, "approved.pub"))
    os.mkdir(os.path.join(sb, "approved.pub"))
    row, p = anchor_row(sb)
    check(row[1] == "FAIL" and "approved key file is not a regular file" in row[2], "a directory: %s" % (row,))
    sb = anchor_sandbox()
    os.remove(os.path.join(sb, "approved.pub"))
    os.mkfifo(os.path.join(sb, "approved.pub"))
    t0 = time.monotonic()
    row, p = anchor_row(sb, timeout=40)
    check(row[1] == "FAIL" and "approved key file is not a regular file" in row[2] and time.monotonic() - t0 < 30, "a FIFO: %s" % (row,))
    sb = anchor_sandbox()
    os.remove(os.path.join(sb, "approved.pub"))
    row, p = anchor_row(sb)
    check(row[1] == "FAIL" and "approved key file is not valid" in row[2] and "cannot be read" in row[2], "a missing path: %s" % (row,))
    for name, buf in (("one trailing LF", ("%s ssh-ed25519 %s\n" % (PRINCIPAL, key)).encode()), ("no trailing LF", ("%s ssh-ed25519 %s" % (PRINCIPAL, key)).encode())):
        sb = anchor_sandbox(key_buffer=buf, anchor={"key_input_sha256": hashlib.sha256(buf).hexdigest()})
        row, p = anchor_row(sb)
        check(row[1] == "PASS", "%s must be accepted: %s" % (name, row))


@cell("GR-127 r4 section 4 (2), the fingerprint tool: a non-zero status, a TIMEOUT, MALFORMED output, MULTILINE output, empty output and a wrong TOOL KIND (RSA, ECDSA, ED25519-SK) are each refused; the scratch is a 0700 directory with a 0600 file (read DURING the call); the scratch is gone after every outcome and after SIGTERM; the real tool and the cell's own independent fingerprint agree")
def gr127():
    fp43 = independent_fpr(throwaway())[7:]
    cases = {"a non-zero status": ("exit 2\n", "exited 2"), "a timeout": ("exec sleep 60\n", "timed out"),
             "malformed output": ("echo 'not a fingerprint'\n", "malformed"), "multiline output": ("echo '256 SHA256:%s no comment (ED25519)'\necho '256 SHA256:%s second (ED25519)'\n" % (fp43, fp43), "not exactly one line"),
             "empty output": ("exit 0\n", "not exactly one line"), "an RSA tool kind": ("echo '3072 SHA256:%s no comment (RSA)'\n" % fp43, "malformed"),
             "an ECDSA tool kind": ("echo '256 SHA256:%s no comment (ECDSA)'\n" % fp43, "ECDSA"), "a security-key tool kind": ("echo '256 SHA256:%s no comment (ED25519-SK)'\n" % fp43, "ED25519-SK")}
    for name, (body, word) in cases.items():
        sb = anchor_sandbox()
        tool = script(sb, "kg", body)
        row, p = anchor_row(sb, ["--ssh-keygen", tool, "--keygen-timeout", "1"], timeout=40)
        check(row[1] == "FAIL" and "could not be derived" in row[2] and word in row[2], "%s: %s" % (name, row))
        check(not scratch_dirs(sb), "%s: the scratch directory was left behind: %s" % (name, scratch_dirs(sb)))
    sb = anchor_sandbox()
    log = os.path.join(sb, "modes.log")
    tool = script(sb, "kg", 'stat -c "%%a %%F" "$(dirname "$3")" "$3" > %s\nexec %s "$@"\n' % (log, KEYGEN))
    row, p = anchor_row(sb, ["--ssh-keygen", tool])
    check(row[1] == "PASS" and "ssh_keygen_sha256=" in open(os.path.join(sb, "ev", [f for f in os.listdir(os.path.join(sb, "ev")) if "-G12-binding" in f and f.endswith(".out")][0])).read(), "the real tool through a recording wrapper: %s" % (row,))
    modes = open(log).read().split("\n")
    check(modes[0].startswith("700 directory") and modes[1].startswith("600 regular file"), "the scratch modes during the call: %s" % modes)
    check(not scratch_dirs(sb), "the scratch directory was left behind after a success")
    sb = anchor_sandbox()
    pidfile = os.path.join(sb, "kg.pid")
    tool = script(sb, "kg", "echo $$ > %s\nexec sleep 60\n" % pidfile)
    json.dump({"rules": clean_rules()}, open(os.path.join(sb, "scenario.json"), "w"))
    json.dump(base_local(), open(os.path.join(sb, "local.json"), "w"))
    env = {"PATH": FAKES + ":/usr/bin:/bin", "FAKE_LOG": os.path.join(sb, "fake.log"), "FAKE_SCENARIO": os.path.join(sb, "scenario.json"), "FAKE_LOCAL": os.path.join(sb, "local.json"), "HOME": sb}
    proc = subprocess.Popen(["/usr/bin/python3", "-I", RUNNER, *anchor_args(sb, ["--ssh-keygen", tool])], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    end = time.time() + 30
    while not os.path.exists(pidfile) and time.time() < end:
        time.sleep(0.05)
    check(os.path.exists(pidfile), "the tool never started")
    child = int(open(pidfile).read())
    check(any(d.startswith(".anchor-") for d in os.listdir(sb)), "the scratch directory did not exist while the tool ran")
    proc.send_signal(signal.SIGTERM)
    proc.communicate(timeout=30)
    time.sleep(0.3)
    check(proc.returncode == 130, "exit %s after SIGTERM" % proc.returncode)
    check(not scratch_dirs(sb), "the scratch directory was left behind after SIGTERM")
    try:
        os.kill(child, 0)
        alive = True
    except OSError:
        alive = False
    check(not alive, "the tool survived SIGTERM")


@cell("GR-128 r4 section 3, T1 then T2: each of the three empty statements missing, duplicated, carried by an include, already filled, out of order, or not at the top is REFUSED naming it; a header of comments and blank lines is allowed; the final bytes differ from T1 by exactly the three lines; one byte wrong elsewhere in the shipped file is a FAIL")
def gr128():
    lines = EMPTY3.splitlines(keepends=True)
    def dev_without(i):
        return DEV_ANCHOR.replace(lines[i], "", 1)
    cases = {"the principal statement missing": (dev_without(0), "CORTEX_ANCHOR_PRINCIPAL is missing", "the shipped installer has 0 CORTEX_ANCHOR_PRINCIPAL statement(s), wanted exactly one"),
             "the key statement missing": (dev_without(1), "CORTEX_ANCHOR_KEY is missing", "the shipped installer has 0 CORTEX_ANCHOR_KEY statement(s), wanted exactly one"),
             "the fpr statement missing": (dev_without(2), "CORTEX_ANCHOR_FPR is missing", "the shipped installer has 0 CORTEX_ANCHOR_FPR statement(s), wanted exactly one"),
             "a statement duplicated": (DEV_ANCHOR.replace(lines[1], lines[1] * 2, 1), "CORTEX_ANCHOR_KEY occurs 2 times", "the shipped installer has 2 CORTEX_ANCHOR_KEY statement(s), wanted exactly one"),
             "a statement already filled": (DEV_ANCHOR.replace(lines[2], "CORTEX_ANCHOR_FPR='SHA256:%s'\n" % ("A" * 43), 1), "CORTEX_ANCHOR_FPR is already filled"),
             "two statements out of order": (DEV_ANCHOR.replace(lines[0] + lines[1], lines[1] + lines[0], 1), "first three statements after the header, in order"),
             "a command before the statements": (DEV_ANCHOR.replace(EMPTY3, "set -eu\n" + EMPTY3, 1), "first three statements after the header, in order")}
    for name, case in cases.items():
        dev, word = case[0], case[1]
        sb = anchor_sandbox(dev=dev)
        row, p = anchor_row(sb)
        check(row[1] == "FAIL" and word in row[2] and all(w in row[2] for w in case[2:]), "%s: %s" % (name, row))
    carried = {"install-prereq.sh": b"prereq_msg() {\n  :\nCORTEX_ANCHOR_KEY=''\n}\n", "install-fetch.sh": INC_FETCH}
    sb = anchor_sandbox(includes=carried)
    row, p = anchor_row(sb)
    check(row[1] == "FAIL" and "CORTEX_ANCHOR_KEY occurs 2 times" in row[2], "a statement carried by an include: %s" % (row,))
    header = '#!/bin/sh\n# one\n\n# two\n\n'
    sb = anchor_sandbox(dev=DEV_ANCHOR.replace('#!/bin/sh\n# the development installer\n\n', header, 1))
    row, p = anchor_row(sb)
    check(row[1] == "PASS", "a header of comments and blank lines: %s" % (row,))
    v = anchor_values()
    t1 = join_text(DEV_ANCHOR.encode(), {"install-prereq.sh": INC_PREREQ, "install-fetch.sh": INC_FETCH}).splitlines(keepends=True)
    final = anchor_final(DEV_ANCHOR, {"install-prereq.sh": INC_PREREQ, "install-fetch.sh": INC_FETCH}, v).splitlines(keepends=True)
    check(len(t1) == len(final) and [i for i in range(len(t1)) if t1[i] != final[i]] == [3, 4, 5], "the fixture itself: final differs from T1 by exactly three lines")
    sb = anchor_sandbox(shipped_fn=lambda b: b.replace(b"echo done", b"echo dine"))
    row, p = anchor_row(sb)
    check(row[1] == "FAIL" and "differs from the development install.sh with the includes spliced" in row[2], "one byte wrong elsewhere: %s" % (row,))


@cell("GR-129 r4 section 7 and the schema-2 record: schema 1 with a key supplied FAILS, schema 2 without a key FAILS, an anchor with a missing, extra or duplicate key is invalid, each anchor field's shape is checked, the sentence of the other schema is refused; with no key, schema 1 and EMPTY anchor statements PASS with a NO ANCHOR note, filled statements FAIL")
def gr129():
    sb = g12_sandbox(dev_install=DEV_ANCHOR, shipped=join_text(DEV_ANCHOR.encode(), {"install-prereq.sh": INC_PREREQ, "install-fetch.sh": INC_FETCH}))
    open(os.path.join(sb, "approved.pub"), "wb").write(("%s ssh-ed25519 %s\n" % (PRINCIPAL, throwaway())).encode())
    row, p = anchor_row(sb)
    check(row[1] == "FAIL" and "record is schema 1" in row[2], "schema 1 with a key supplied: %s" % (row,))
    sb = anchor_sandbox()
    row, p = g12_row(sb)
    check(row[1] == "FAIL" and "no --approved-key-file was supplied" in row[2], "schema 2 without a key: %s" % (row,))
    v = anchor_values()
    good = {"principal": v["principal"], "key": v["key"], "fpr": v["fpr"], "key_input_sha256": hashlib.sha256(("%s ssh-ed25519 %s\n" % (v["principal"], v["key"])).encode()).hexdigest()}
    def with_anchor(anchor_text):
        rec = json.dumps(good_record(schema=2, splice=SPLICE2))
        return rec[:-1] + ', "anchor": ' + anchor_text + "}"
    raw = {"an anchor key missing": json.dumps({k: x for k, x in good.items() if k != "fpr"}), "an extra anchor key": json.dumps(dict(good, extra="x")),
           "a duplicate anchor key": json.dumps(good).replace('"key":', '"key": "x", "key":', 1), "an anchor that is not an object": '"x"'}
    for name, anchor_text in raw.items():
        text = with_anchor(anchor_text)
        sb = anchor_sandbox()
        open(os.path.join(sb, "binding.json"), "w").write(text)
        row, p = anchor_row(sb)
        check(row[1] == "FAIL" and "binding record is not valid" in row[2], "%s: %s" % (name, row))
    shape = {"principal": "re lease", "key": "AAAA", "fpr": "SHA256:short", "key_input_sha256": "A" * 64}
    for field, value in shape.items():
        sb = anchor_sandbox(anchor={field: value})
        row, p = anchor_row(sb)
        check(row[1] == "FAIL" and "anchor.%s does not match its grammar" % field in row[2], "the shape of anchor.%s: %s" % (field, row))
    sb = anchor_sandbox(record={"splice": SPLICE_SENTENCE})
    row, p = anchor_row(sb)
    check(row[1] == "FAIL" and "splice is not the exact versioned sentence of schema 2" in row[2], "the schema-1 sentence in a schema-2 record: %s" % (row,))
    sb = g12_sandbox(dev_install=DEV_ANCHOR, shipped=join_text(DEV_ANCHOR.encode(), {"install-prereq.sh": INC_PREREQ, "install-fetch.sh": INC_FETCH}), record={"splice": SPLICE2})
    row, p = g12_row(sb)
    check(row[1] == "FAIL" and "splice is not the exact versioned sentence of schema 1" in row[2], "the schema-2 sentence in a schema-1 record: %s" % (row,))
    sb = g12_sandbox(dev_install=DEV_ANCHOR, shipped=join_text(DEV_ANCHOR.encode(), {"install-prereq.sh": INC_PREREQ, "install-fetch.sh": INC_FETCH}))
    row, p = g12_row(sb)
    check(row[1] == "PASS" and "NO ANCHOR" in row[2], "no key, schema 1, empty statements: %s" % (row,))
    filled = DEV_ANCHOR.replace("CORTEX_ANCHOR_KEY=''", "CORTEX_ANCHOR_KEY='%s'" % v["key"], 1)
    sb = g12_sandbox(dev_install=filled, shipped=join_text(filled.encode(), {"install-prereq.sh": INC_PREREQ, "install-fetch.sh": INC_FETCH}))
    row, p = g12_row(sb)
    check(row[1] == "FAIL" and "the development installer's CORTEX_ANCHOR_KEY is filled but no approved key file was supplied" in row[2], "filled statements without a key (the development installer is named): %s" % (row,))



@cell("GR-130 r4 sections 4 (3) and 5, the expectations come from the APPROVED FILE: each of the four recorded anchor fields wrong, and each of the three installer literals wrong EVEN WHEN THE RECORD AGREES WITH THE INSTALLER (agreement between record and installer is insufficient), name the field; a consistent record, installer and file PASS, with the public fields and the tool's hash in the binding receipt")
def gr130():
    other = anchor_values(1)
    for field in ("principal", "key", "fpr", "key_input_sha256"):
        value = hashlib.sha256(b"another file").hexdigest() if field == "key_input_sha256" else (other[field] if field != "principal" else "someone@example.invalid")
        sb = anchor_sandbox(anchor={field: value})
        row, p = anchor_row(sb)
        check(row[1] == "FAIL" and "the record's anchor %s differs from the one derived from the approved key file" % field in row[2], "the record's %s wrong: %s" % (field, row))
    for statement, field in (("CORTEX_ANCHOR_PRINCIPAL", "principal"), ("CORTEX_ANCHOR_KEY", "key"), ("CORTEX_ANCHOR_FPR", "fpr")):
        values = dict(anchor_values(), **{field: other[field] if field != "principal" else "someone@example.invalid"})
        sb = anchor_sandbox(shipped_values=values, anchor={field: values[field]})   # the installer AND the record carry the same wrong value
        row, p = anchor_row(sb)
        check(row[1] == "FAIL" and "the shipped installer's %s literal differs from the one derived from the approved key file" % statement in row[2]
              and "the record's anchor %s differs" % field in row[2], "the installer's %s wrong with the record in agreement: %s" % (statement, row))
    sb = anchor_sandbox(record={"joined_sha256": "0" * 64})
    row, p = anchor_row(sb)
    check(row[1] == "FAIL" and "joined sha256" in row[2], "the identity chain, joined_sha256: %s" % (row,))
    sb = anchor_sandbox()
    row, p = anchor_row(sb)
    check(row[1] == "PASS" and "approved key file" in row[2] and "the approval is the CTO's" in row[2], "the consistent case: %s" % (row,))
    v = anchor_values()
    receipt = open(os.path.join(sb, "ev", [f for f in os.listdir(os.path.join(sb, "ev")) if "-G12-binding" in f and f.endswith(".out")][0])).read()
    for want in ("anchor_mode=approved-file", "anchor_principal=" + v["principal"], "anchor_key=" + v["key"], "anchor_fpr=" + v["fpr"], "ssh_keygen=" + KEYGEN, "ssh_keygen_sha256=" + sha(KEYGEN)):
        check(want in receipt, "the binding receipt lacks %r" % want)
    check(not scratch_dirs(sb), "the scratch directory was left behind")


@cell("GR-131 r4 rule 9 for the approved key file: the approved key file is hashed at the start and read AGAIN at the end; a file that changes while the checker runs is a FAIL")
def gr131():
    sb = anchor_sandbox(drift_cmd="printf ' ' >> ../approved.pub\n")
    row, p = anchor_row(sb)
    check(row[1] == "FAIL" and "bytes changed while the check ran" in row[2], "the approved key file changed during the run: %s" % (row,))



HDR = '#!/bin/sh\n# the development installer\n\n'
M_PREREQ = '[ ! -f "$here/install-prereq.sh" ] || . "$here/install-prereq.sh"   # @include\n'
M_FETCH = '[ ! -f "$here/install-fetch.sh" ] || . "$here/install-fetch.sh"   # @include\n'
BODY = 'set -eu\numask 077\nhere=x\n'


@cell("GR-132 sam GR19-SAM02, the ORIGIN of the three empty anchor statements: with a key, the RAW development installer must itself carry them (inventory, order, top) BEFORE T1; placeholders supplied ONLY by an include (all three, or one of them completing a pair) FAIL naming the development installer, once each, even when the record, the shipped bytes and the splice are coherent; a duplicate carried by an include still FAILS after T1; the development installer's own statements PASS")
def gr132():
    word = "development installer: "
    only_in_include = HDR + M_PREREQ + BODY + M_FETCH + "echo done\n"
    incs = {"install-prereq.sh": EMPTY3.encode() + INC_PREREQ, "install-fetch.sh": INC_FETCH}
    sb = anchor_sandbox(dev=only_in_include, includes=incs)
    row, p = anchor_row(sb)
    check(row[1] == "FAIL" and word in row[2] and all(("%s is missing" % n) in row[2] for n in ANCHOR_NAMES), "all three supplied only by an include: %s" % (row,))
    check(all(row[2].count("%s is missing" % n) == 1 for n in ANCHOR_NAMES), "each missing statement is named once: %s" % (row,))
    pair = HDR + "CORTEX_ANCHOR_PRINCIPAL=''\nCORTEX_ANCHOR_KEY=''\n" + M_PREREQ + BODY + M_FETCH + "echo done\n"
    incs = {"install-prereq.sh": b"CORTEX_ANCHOR_FPR=''\n" + INC_PREREQ, "install-fetch.sh": INC_FETCH}
    sb = anchor_sandbox(dev=pair, includes=incs)
    row, p = anchor_row(sb)
    check(row[1] == "FAIL" and word in row[2] and "CORTEX_ANCHOR_FPR is missing" in row[2] and "CORTEX_ANCHOR_PRINCIPAL is missing" not in row[2], "the third supplied by an include: %s" % (row,))
    sb = anchor_sandbox(dev=DEV_ANCHOR.replace("CORTEX_ANCHOR_PRINCIPAL=''\n", "", 1))
    row, p = anchor_row(sb)
    check(row[1] == "FAIL" and row[2].count("CORTEX_ANCHOR_PRINCIPAL is missing") == 1, "a statement missing and supplied by nothing is named once: %s" % (row,))
    both = {"install-prereq.sh": EMPTY3.encode() + INC_PREREQ, "install-fetch.sh": INC_FETCH}
    sb = anchor_sandbox(includes=both)
    row, p = anchor_row(sb)
    check(row[1] == "FAIL" and "occurs 2 times" in row[2] and word not in row[2], "all three in the development installer AND an include (the post-T1 check): %s" % (row,))
    sb = anchor_sandbox()
    row, p = anchor_row(sb)
    check(row[1] == "PASS", "the development installer's own three statements: %s" % (row,))
    m = load_module()
    t1 = join_text(only_in_include.encode(), {"install-prereq.sh": EMPTY3.encode() + INC_PREREQ, "install-fetch.sh": INC_FETCH})
    check(m.anchor_problems(t1) == [], "the T1 result of the include-only case satisfies the inventory (so a check after T1 alone is satisfied): %s" % (m.anchor_problems(t1),))
    raw = m.anchor_problems(only_in_include.encode())
    check(len(raw) == 3 and all(("%s is missing" % n) in " ".join(raw) for n in ANCHOR_NAMES), "the RAW development installer of the include-only case is refused three times: %s" % (raw,))


@cell("GR-133 sam GR19-SAM01, without a key every statement of every input of the final bytes is EMPTY OR ABSENT: a FILLED statement carried by an include FAILS naming the include and the shipped installer, for each of the three names, when the record, the shipped bytes and the splice are coherent and the development installer is clean (statements empty or absent); an EMPTY statement in an include, and no statement anywhere, PASS with a NO ANCHOR note")
def gr133():
    def keyless(dev, includes):
        return g12_sandbox(dev_install=dev, includes=includes, shipped=join_text(dev.encode(), includes))
    for name in ANCHOR_NAMES:
        filled = {"install-prereq.sh": ("%s='x'\n" % name).encode() + INC_PREREQ, "install-fetch.sh": INC_FETCH}
        for label, dev in (("empty statements in the development installer", DEV_ANCHOR), ("no statements in the development installer", DEV_INSTALL)):
            sb = keyless(dev, filled)
            row, p = g12_row(sb)
            check(row[1] == "FAIL" and ("the include install-prereq.sh's %s is filled but no approved key file was supplied" % name) in row[2]
                  and ("the shipped installer's %s is filled but no approved key file was supplied" % name) in row[2], "%s filled in an include, %s: %s" % (name, label, row))
    for label, dev, incs in (("an EMPTY statement inside an include's function body (the oracle never runs it)", DEV_INSTALL, {"install-prereq.sh": b"prereq_msg() {\nCORTEX_ANCHOR_KEY=''\n  echo prereq\n}\n", "install-fetch.sh": INC_FETCH}),
                             ("no statement anywhere", DEV_INSTALL, {"install-prereq.sh": INC_PREREQ, "install-fetch.sh": INC_FETCH}),
                             ("the empty statements of the development installer", DEV_ANCHOR, {"install-prereq.sh": INC_PREREQ, "install-fetch.sh": INC_FETCH})):
        sb = keyless(dev, incs)
        row, p = g12_row(sb)
        check(row[1] == "PASS" and "NO ANCHOR" in row[2], "%s: %s" % (label, row))
    sb = keyless(DEV_ANCHOR.replace("CORTEX_ANCHOR_KEY=''", "CORTEX_ANCHOR_KEY='x'", 1), {"install-prereq.sh": INC_PREREQ, "install-fetch.sh": INC_FETCH})
    row, p = g12_row(sb)
    check(row[1] == "FAIL" and "the development installer's CORTEX_ANCHOR_KEY is filled but no approved key file was supplied" in row[2], "filled in the development installer (the earlier rule): %s" % (row,))


@cell("GR-134 sam GR19-SAM01, the LONE witness: a FILLED anchor statement at column 0 inside an include's FUNCTION BODY (the oracle never runs it, so the matrix is ordinary and the development installer is clean) is the ONLY failure when no key was supplied, for each of the three names")
def gr134():
    def keyless(dev, includes):
        return g12_sandbox(dev_install=dev, includes=includes, shipped=join_text(dev.encode(), includes))
    for name in ANCHOR_NAMES:   # inert to the oracle (inside a function body, at column 0), so the matrix is ordinary and the contract rule is the ONLY failure
        inert = {"install-prereq.sh": ("prereq_msg() {\n%s='x'\n  echo prereq\n}\n" % name).encode(), "install-fetch.sh": INC_FETCH}
        sb = keyless(DEV_ANCHOR, inert)
        row, p = g12_row(sb)
        check(row[1] == "FAIL" and ("the include install-prereq.sh's %s is filled but no approved key file was supplied" % name) in row[2] and "clean control" not in row[2],
              "%s filled inside an include's function body (the matrix is ordinary): %s" % (name, row))


READ_ALL_SH = "touch ../checker-ran\nfor f in deploy/*.sh; do timeout 5 cat \"$f\" >/dev/null 2>&1 || true; done\n"


@cell("GR-135 vera VR18-01, a refused include stops the run BEFORE any delegated reader: with a checker that touches a sentinel and then reads every deploy/*.sh (bounded by timeout 5, as the production checker walks them), a FIFO include and a symlink to a FIFO include each end in a FAIL naming the include, promptly, and the sentinel is ABSENT (the checker never ran); the regular control PASSES with the sentinel PRESENT")
def gr135():
    def fifo(path):
        os.remove(path)
        os.mkfifo(path)
    def fifo_link(path):
        os.remove(path)
        target = path + ".fifo"
        os.mkfifo(target)
        os.symlink(target, path)
    for name, mutate, word in (("an include is a FIFO", lambda sb: fifo(os.path.join(sb, "dev", "deploy", "install-fetch.sh")), "install-fetch.sh"),
                               ("an include is a symlink to a FIFO", lambda sb: fifo_link(os.path.join(sb, "dev", "deploy", "install-prereq.sh")), "install-prereq.sh")):
        sb = g12_sandbox(drift_cmd=READ_ALL_SH)
        mutate(sb)
        t0 = time.monotonic()
        run(sb, g12m_args(sb), timeout=60)
        took = time.monotonic() - t0
        row = tsv(sb)["G12"]
        check(row[1] == "FAIL" and "not a regular file" in row[2] and word in row[2] and took < 30, "%s: %s (%.1f s)" % (name, row, took))
        check(not os.path.exists(os.path.join(sb, "checker-ran")), "%s: the delegated checker was invoked after the include was refused" % name)
    sb = g12_sandbox(drift_cmd=READ_ALL_SH)
    row, p = g12_row(sb)
    check(row[1] == "PASS" and os.path.exists(os.path.join(sb, "checker-ran")), "the regular control (the checker runs): %s" % (row,))


@cell("GR-136 vera VR18-02, the FINAL reread is guarded: an input that becomes a FIFO DURING the check (swapped from inside the checker, after the first read) ends in the drift FAIL 'bytes changed while the check ran', promptly, and never blocks: the binding record, the shipped installer, the development installer, an include, and with a key the approved key file; a regular input that is rewritten still gives the same FAIL")
def gr136():
    cases = (("the binding record", "rm -f ../binding.json\nmkfifo ../binding.json\n", False),
             ("the shipped installer", "rm -f ../art/install.sh\nmkfifo ../art/install.sh\n", False),
             ("the development installer", "rm -f deploy/install.sh\nmkfifo deploy/install.sh\n", False),
             ("an include", "rm -f deploy/install-fetch.sh\nmkfifo deploy/install-fetch.sh\n", False),
             ("the approved key file", "rm -f ../approved.pub\nmkfifo ../approved.pub\n", True))
    for name, swap, keyed in cases:
        t0 = time.monotonic()
        if keyed:
            sb = anchor_sandbox(drift_cmd=swap)
            row, p = anchor_row(sb, timeout=30)
        else:
            sb = g12_sandbox(drift_cmd=swap)
            run(sb, g12m_args(sb), timeout=30)
            row = tsv(sb)["G12"]
        took = time.monotonic() - t0
        check(row[1] == "FAIL" and "bytes changed while the check ran" in row[2] and took < 25, "%s became a FIFO during the check: %s (%.1f s)" % (name, row, took))
    sb = g12_sandbox(drift_cmd="echo '# drift' >> deploy/install-fetch.sh\n")
    row, p = g12_row(sb)
    check(row[1] == "FAIL" and "bytes changed while the check ran" in row[2], "a regular include rewritten during the check: %s" % (row,))


PRODUCER = os.environ.get("GR_PRODUCER", os.path.join(HERE, "fixtures", "producer", "release_manifest.py"))
PRODUCER_PIN = "440189e3cb34aa922ecb474baedad8f3562d78cc"
PRODUCER_SHA256 = "691789da02b027616f134309ba395be6b5c614a4424715b9ceb795b86a201341"
PRODUCER_RECORD_KEYS = ("schema", "object_format", "commit", "tree", "includes", "splice", "joined_sha256", "anchor")


def conform(producer, keyed, tamper=None):
    """Runs the PRODUCER's join-installer (sophi's release_manifest.py) on the sandbox's development tree, with the sandbox's approved key file when keyed, and judges what it WROTE
    against expectations built independently of it. Returns (problems, the sandbox): an empty list is conformance."""
    sb = anchor_sandbox() if keyed else g12_sandbox(dev_install=DEV_ANCHOR)
    dev = os.path.join(sb, "dev")
    commit, tree = git(dev, "rev-parse", "HEAD"), git(dev, "rev-parse", "HEAD^{tree}")
    shipped, record = os.path.join(sb, "art", "install.sh"), os.path.join(sb, "binding.json")
    os.remove(shipped)
    os.remove(record)   # nothing of mine is left behind: the producer writes both the installer and the record
    args = [sys.executable, "-I", producer, "join-installer", "--deploy", os.path.join(dev, "deploy"), "--output", shipped, "--record", record,
            "--commit", commit, "--tree", tree, "--object-format", "sha1"]
    if keyed:
        args += ["--trust-key-file", os.path.join(sb, "approved.pub")]
    p = subprocess.run(args, capture_output=True, text=True, timeout=60)
    if p.returncode != 0 or not os.path.exists(shipped) or not os.path.exists(record):
        return ["the producer's join failed (exit %s): %s" % (p.returncode, (p.stdout + p.stderr).strip()[-200:])], sb
    if tamper:
        tamper(shipped)   # a change made AFTER the producer wrote its output and before anything is judged
    incs = {"install-prereq.sh": INC_PREREQ, "install-fetch.sh": INC_FETCH}
    v = anchor_values()
    want = anchor_final(DEV_ANCHOR, incs, v) if keyed else join_text(DEV_ANCHOR.encode(), incs)
    problems = []
    got = open(shipped, "rb").read()
    if got != want:
        problems.append("the producer's joined installer differs from the bytes my independent T1%s builds" % (" then T2" if keyed else ""))
    try:
        rec = json.loads(open(record, "rb").read())
    except ValueError as e:
        return problems + ["the producer's record is not JSON: %s" % e], sb
    keys = PRODUCER_RECORD_KEYS if keyed else PRODUCER_RECORD_KEYS[:-1]
    if set(rec) != set(keys):
        problems.append("the producer's record keys are %s, not %s" % (sorted(rec), sorted(keys)))
    if rec.get("schema") != (2 if keyed else 1) or type(rec.get("schema")) is not int:
        problems.append("the producer's record schema is %r, not %d" % (rec.get("schema"), 2 if keyed else 1))
    if rec.get("splice") != (SPLICE2 if keyed else SPLICE_SENTENCE):
        problems.append("the producer's splice sentence is not the contract's")
    if rec.get("joined_sha256") != hashlib.sha256(want).hexdigest():
        problems.append("the producer's joined_sha256 is not the hash of the bytes my independent build gives")
    if rec.get("commit") != commit or rec.get("tree") != tree or rec.get("object_format") != "sha1":
        problems.append("the producer's commit, tree or object_format differs from the development tree's own")
    if rec.get("includes") != [{"name": n, "sha256": hashlib.sha256(incs[n]).hexdigest()} for n in ("install-prereq.sh", "install-fetch.sh")]:
        problems.append("the producer's include list differs from the includes' own names and sha256 in marker order")
    if keyed:
        buf = ("%s ssh-ed25519 %s\n" % (v["principal"], v["key"])).encode()
        expect = {"principal": v["principal"], "key": v["key"], "fpr": v["fpr"], "key_input_sha256": hashlib.sha256(buf).hexdigest()}
        anchor = rec.get("anchor")
        if not isinstance(anchor, dict) or set(anchor) != set(expect):
            problems.append("the producer's anchor object does not have exactly the four keys")
        else:
            for k in expect:
                if anchor[k] != expect[k]:
                    problems.append("the producer's anchor.%s differs from the value my independent derivation gives" % k)
    row = anchor_row(sb)[0] if keyed else g12_row(sb)[0]
    if row[1] != "PASS":
        problems.append("G12 does not PASS the producer's own output: %s" % (row[2][:240],))
    elif ("anchor: the approved key file" not in row[2]) if keyed else ("NO ANCHOR" not in row[2]):
        problems.append("the G12 PASS does not carry its anchor note: %s" % (row[2][:240],))
    return problems, sb


@cell("GR-137 conformance of the PRODUCER (cox W1-114 condition 4): the joined installer and the schema-2 record that sophi's REAL join (release_manifest.py join-installer at her corrected pin 440189e, a COPY pinned by sha256) writes from a development tree with the three empty statements and a throwaway approved key are byte-identical to my independent T1 then T2, carry the eight keys and the four anchor fields my own derivation gives, and PASS G12 with the approved key file; without a key the same join writes a schema-1 record and an installer with the statements empty, and G12 PASSES it with a NO ANCHOR note; one byte changed in the producer's installer FAILS")
def gr137():
    check(hashlib.sha256(open(PRODUCER, "rb").read()).hexdigest() == PRODUCER_SHA256 or os.environ.get("GR_PRODUCER"), "the pinned producer copy is not the file INDEX.txt names (sha256 %s)" % PRODUCER_SHA256)
    problems, sb = conform(PRODUCER, True)
    check(problems == [], "the producer's keyed output does not conform: %s" % (problems,))
    problems, sb2 = conform(PRODUCER, False)
    check(problems == [], "the producer's keyless output does not conform: %s" % (problems,))
    # one byte changed AFTER the producer wrote it (a fresh sandbox: the evidence directory of a sandbox serves one run): G12 FAILS and the independent comparison says why
    def flip(path):
        data = open(path, "rb").read()
        assert b"echo done" in data, "the producer's installer is not the expected shape"
        open(path, "wb").write(data.replace(b"echo done", b"echo dine"))
    problems, sb = conform(PRODUCER, True, tamper=flip)
    check(any("G12 does not PASS" in x for x in problems) and any("differs from the bytes" in x for x in problems), "one byte changed in the producer's installer: %s" % (problems,))


@cell("GR-138 the conformance check CAN FAIL: six faults planted one at a time in COPIES of sophi's producer (T2 fills the key into the fpr statement, the record's fpr is the key, the record's key_input_sha256 is wrong, the record keeps the schema-1 sentence, T2 is skipped, the record says schema 1) are each reported by the same check that passes the real producer")
def gr138():
    src = open(PRODUCER, encoding="utf-8").read()
    faults = (("T2 fills the key into the fpr statement", '(anchor["principal"], anchor["key"], anchor["fpr"])', '(anchor["principal"], anchor["key"], anchor["key"])', "differs from the bytes"),
              ("the record's fpr is the key", '"fpr": anchor["fpr"], "key_input_sha256"', '"fpr": anchor["key"], "key_input_sha256"', "anchor.fpr"),
              ("the record's key_input_sha256 is wrong", '"key_input_sha256": anchor["key_input_sha256"]}', '"key_input_sha256": "0" * 64}', "anchor.key_input_sha256"),
              ("the record keeps the schema-1 splice sentence", '"splice": ANCHOR_SPLICE if anchor else SPLICE', '"splice": SPLICE', "splice sentence"),
              ("T2 is skipped", "        template = apply_anchor(template, anchor)", "        pass", "differs from the bytes"),
              ("the record says schema 1", '"schema": 2 if anchor else 1', '"schema": 1', "schema"))
    for name, old, new, phrase in faults:
        check(src.count(old) == 1, "the planted fault %r does not anchor exactly once in the pinned producer" % name)
        d = tempfile.mkdtemp(prefix="gr-producer.", dir=TMP)
        SANDBOXES.append(d)
        faulty = os.path.join(d, "release_manifest.py")
        open(faulty, "w", encoding="utf-8").write(src.replace(old, new, 1))
        problems, _sb = conform(faulty, True)
        check(any(phrase in x for x in problems), "the planted fault %r is not reported (wanted %r): %s" % (name, phrase, problems))

# ---- way C (cox W1-177 and W1-178; nix's spec e4d859a3, kai's row 187e1c07): G12 (g) runs the checker OF THE ORACLE TREE on the deploy directory OF THE DEVELOPMENT TREE ----
def wayc_oracle(sb, out=None, rc=0, git_repo=True, judge_only="*/dev/deploy", after=""):
    """The oracle tree of the way-C cells: a fake scripts/s5-check.sh that records its argv and working directory in $HOME/oracle-argv.txt and prints `out` (default: the NOTE line once)
    only when asked to judge a deploy directory matching `judge_only` (default: the DEVELOPMENT tree's), the fake oracle scripts/s5_check.py, a deploy/ of its own (the TOOL tree's: judging
    it is the fault), and a git repository unless git_repo is False."""
    orc = os.path.join(sb, "oracle")
    os.makedirs(os.path.join(orc, "scripts"), exist_ok=True)
    os.makedirs(os.path.join(orc, "deploy"), exist_ok=True)
    open(os.path.join(orc, "deploy", "install.sh"), "w").write("# the tool tree's own deploy directory: not the candidate\n")
    open(os.path.join(orc, "scripts", "s5_check.py"), "w").write(FAKE_ORACLE)
    body = (NOTE_LINE + "\n") if out is None else out
    open(os.path.join(orc, "scripts", "s5-check.sh"), "w").write(
        "#!/bin/sh\nprintf 'arg1=%s argc=%s cwd=%s\\n' \"$1\" \"$#\" \"$(pwd)\" > \"$HOME/oracle-argv.txt\"\ncase \"$1\" in\n" + judge_only + ")\ncat <<'EOF'\n" + body + "EOF\n" + after + "exit " + str(rc)
        + "\n;;\n*) echo \"judged the wrong deploy: $1\"; exit 3 ;;\nesac\n")
    if git_repo:
        git(orc, "init", "-q")
        git(orc, "add", "-A")
        git(orc, "commit", "-q", "-m", "oracle")
    return orc


def wayc_sandbox(dev_out="S5 OK: the candidate's own checker prints no NOTE (the older wording)\n", **oracle_kw):
    """A G12 sandbox whose development tree has its OWN checker (it prints dev_out and leaves ../dev-checker-ran behind when it runs) and whose oracle tree is wayc_oracle(...)."""
    sb = g12_sandbox(oracle_out=dev_out, drift_cmd="echo ran > ../dev-checker-ran\n")
    wayc_oracle(sb, **oracle_kw)
    return sb


def wayc_identity_receipt(sb):
    ev = os.path.join(sb, "ev")
    names = [f for f in os.listdir(ev) if "-G12-oracle_identity" in f and f.endswith(".out")]
    return open(os.path.join(ev, names[0])).read() if names else ""


@cell("GR-139 way C, G12 (g): with --oracle-tree the checker OF THE ORACLE TREE judges the deploy directory OF THE DEVELOPMENT TREE (the candidate's own checker, which prints no NOTE, is not run); the oracle's identity, the candidate's inputs and the actual command are receipted")
def gr139():
    sb = wayc_sandbox()
    p = run(sb, g12_args(sb))
    row = tsv(sb)["G12"]
    check(row[1] == "PASS", "G12 %s (%s)" % (row, p.stdout[-300:]))
    dev, orc = os.path.join(sb, "dev"), os.path.join(sb, "oracle")
    argv = open(os.path.join(sb, "oracle-argv.txt")).read() if os.path.exists(os.path.join(sb, "oracle-argv.txt")) else "(the oracle's checker did not run)"
    check(argv.startswith("arg1=" + os.path.join(dev, "deploy") + " argc=1 cwd=" + dev + "\n"), "the oracle's checker was not asked to judge the development tree's deploy directory, with the development tree as its working directory: %s" % argv)
    check(not os.path.exists(os.path.join(sb, "dev-checker-ran")), "the candidate's own checker ran: with --oracle-tree the oracle tree's checker runs")
    for name in ("-G12-oracle_g_commit", "-G12-oracle_g_tree_id", "-G12-oracle_identity", "-G12-oracle"):   # the verdict row names the receipts a reader follows (the identity receipt among them)
        check(name in row[3], "the verdict row does not name the receipt %s: %s" % (name, row[3]))
    text = wayc_identity_receipt(sb)
    for want in ("oracle_tree=" + orc, "oracle_file_sha256=" + sha(os.path.join(orc, "scripts", "s5_check.py")), "oracle_checker_sha256=" + sha(os.path.join(orc, "scripts", "s5-check.sh")),
                 "oracle_commit=" + git(orc, "rev-parse", "HEAD"), "oracle_tree_id=" + git(orc, "rev-parse", "HEAD^{tree}"), "oracle_is_development_tree=NO",
                 "candidate_deploy=" + os.path.join(dev, "deploy"), "candidate_commit=" + git(dev, "rev-parse", "HEAD"), "candidate_tree=" + git(dev, "rev-parse", "HEAD^{tree}"),
                 "command=sh " + os.path.join(orc, "scripts", "s5-check.sh") + " " + os.path.join(dev, "deploy")):
        check(want in text, "the identity receipt lacks %r: %s" % (want, text[:700]))
    keys = ("oracle_tree=", "oracle_file_sha256=", "oracle_commit=", "oracle_tree_id=", "oracle_is_development_tree=")   # exactly as the matrix receipt records the oracle
    m_lines, i_lines = [l for l in matrix_receipt(sb).splitlines() if l.startswith(keys)], [l for l in text.splitlines() if l.startswith(keys)]
    check(len(m_lines) == 5 and m_lines == i_lines, "the oracle's identity lines differ between the matrix receipt and the way-C receipt: matrix %s / way C %s" % (m_lines, i_lines))


@cell("GR-140 way C: the NOTE check is decided by the ORACLE's checker: a non-zero exit, no NOTE line, the NOTE line twice and a claim that it proves definitions only FAIL even when the candidate's own checker is clean")
def gr140():
    cases = {"exit 1": (dict(rc=1), "exited 1"), "no NOTE line": (dict(out="S5 OK\n"), "note"),
             "a proof claim": (dict(out=NOTE_LINE + "\nS5-10 proves functions only\n"), "claim"),
             "the NOTE line twice": (dict(out=NOTE_LINE + "\n" + NOTE_LINE + "\n"), "note")}
    for name, (kw, word) in cases.items():
        sb = wayc_sandbox(dev_out=NOTE_LINE + "\n", **kw)   # the candidate's own checker is CLEAN
        run(sb, g12_args(sb))
        row = tsv(sb)["G12"]
        check(row[1] == "FAIL" and word in row[2].lower(), "%s: %s" % (name, row))


@cell("GR-141 way C: the oracle's checker is asked to judge the DEVELOPMENT tree's deploy directory, never the tool tree's: a checker that prints the NOTE only for the tool tree's own deploy does not pass")
def gr141():
    sb = wayc_sandbox(dev_out=NOTE_LINE + "\n", judge_only="*/oracle/deploy")
    run(sb, g12_args(sb))
    row = tsv(sb)["G12"]
    check(row[1] == "FAIL" and "note" in row[2].lower(), "G12 %s" % (row,))
    argv = open(os.path.join(sb, "oracle-argv.txt")).read() if os.path.exists(os.path.join(sb, "oracle-argv.txt")) else "(the oracle's checker did not run)"
    check(argv.startswith("arg1=" + os.path.join(sb, "dev", "deploy") + " "), "the oracle's checker was not given the development tree's deploy directory: %s" % argv)


@cell("GR-142 way C: the oracle's identity is receipted as the matrix receipts it: git commit and tree ids, or 'none (git rc N)' when the oracle tree is not a git tree (recorded, not hidden); oracle_is_development_tree is yes only when the oracle tree IS the development tree")
def gr142():
    sb = wayc_sandbox(git_repo=False)
    run(sb, g12m_args(sb, bind=False))   # an oracle tree that is not a git tree: recorded as none (git rc N); that G12 then FAILS is GR-145's question
    text = wayc_identity_receipt(sb)
    check("oracle_commit=none (git rc " in text and "oracle_tree_id=none (git rc " in text and "oracle_is_development_tree=NO" in text, "a non-git oracle tree is not recorded as 'none (git rc N)': %s" % text[:500])
    sb = g12_sandbox(oracle_out=NOTE_LINE + "\n")   # the oracle tree IS the development tree: its checker and its fake oracle are the candidate's own
    dev = os.path.join(sb, "dev")
    open(os.path.join(dev, "scripts", "s5_check.py"), "w").write(FAKE_ORACLE)   # untracked: the tree stays clean for the binding's status read
    args = g12_args(sb)
    args[args.index("--oracle-tree") + 1] = dev
    run(sb, args)
    text = wayc_identity_receipt(sb)
    for want in ("oracle_tree=" + dev, "oracle_commit=" + git(dev, "rev-parse", "HEAD"), "oracle_is_development_tree=yes"):
        check(want in text, "the identity receipt of the oracle tree that is the development tree lacks %r: %s" % (want, text[:500]))


@cell("GR-143 way C leaves the default alone: without --oracle-tree the candidate's own checker runs as before (sh scripts/s5-check.sh deploy, working directory the development tree) and no oracle identity is receipted")
def gr143():
    sb = wayc_sandbox(dev_out=NOTE_LINE + "\n")
    args = g12_args(sb)
    i = args.index("--oracle-tree")
    del args[i:i + 2]
    run(sb, args)
    check(os.path.exists(os.path.join(sb, "dev-checker-ran")), "the candidate's own checker did not run without --oracle-tree")
    cmds = [f for f in os.listdir(os.path.join(sb, "ev")) if f.endswith("-G12-oracle.cmd")]
    check(cmds, "no receipt of the checker step: %s" % sorted(os.listdir(os.path.join(sb, "ev")))[:20])
    cmd = open(os.path.join(sb, "ev", cmds[0])).read()
    check(cmd.startswith("sh scripts/s5-check.sh deploy  (cwd " + os.path.join(sb, "dev") + ")"), "the default command changed: %s" % cmd[:200])
    check(wayc_identity_receipt(sb) == "", "an oracle identity was receipted without --oracle-tree")
    check(not os.path.exists(os.path.join(sb, "oracle-argv.txt")), "the oracle's checker ran without --oracle-tree")


@cell("GR-144 way C keeps the confinement (pin A, F2: vera's return on eec6febc): the GENERIC local allow-list takes no way-C form at all, and the way-C step takes exactly ONE command: the one built from the two trees GIVEN (sh <oracle>/scripts/s5-check.sh <dev>/deploy); a second tool tree, another deploy directory, another script name, a third argument, a path with a space or a semicolon, and a call without --oracle-tree are refused before anything runs, even when every path is of the safe alphabet")
def gr144():
    mod = load_module()
    sb = sandbox()
    rec = mod.Recorder(os.path.join(sb, "ev"))
    marker = os.path.join(sb, "must-not-exist")
    d = os.path.join(sb, "d")
    os.makedirs(os.path.join(d, "deploy"))

    def tool(name):
        t = os.path.join(sb, name)
        os.makedirs(os.path.join(t, "scripts"))
        open(os.path.join(t, "scripts", "s5-check.sh"), "w").write("#!/bin/sh\necho ok\ntouch \"$0.ran\"\n")
        return t, os.path.join(t, "scripts", "s5-check.sh")
    o, script = tool("o")
    o2, script2 = tool("o2")
    ns = type("Args", (), {"oracle_tree": o, "dev_tree": d})()
    ctx = mod.Ctx(None, rec, ns)
    exact = ["sh", script, os.path.join(d, "deploy")]
    step = ctx.way_c_step(exact, cwd=d)
    check(step.rc == 0 and step.out.strip() == b"ok" and os.path.exists(script + ".ran"), "the one allowed command was refused or did not run: rc %s %r" % (step.rc, step.out))
    os.remove(script + ".ran")
    try:
        ctx.local_step("G12", "generic", exact)   # the generic local allow-list takes no way-C form: only the way-C step runs this command
        raised = False
    except mod.Refused:
        raised = True
    check(raised and not os.path.exists(script + ".ran"), "the generic local step ran the way-C command (raised=%s)" % raised)
    spaced = os.path.join(sb, "o x")
    os.makedirs(os.path.join(spaced, "scripts"))
    for label, argv, args_ns, cwd in (("a second tool tree of the safe alphabet", ["sh", script2, os.path.join(d, "deploy")], ns, d),
                                 ("another deploy directory", ["sh", script, os.path.join(d, "other")], ns, d),
                                 ("the tool tree's own deploy directory", ["sh", script, os.path.join(o, "deploy")], ns, d),
                                 ("another script name", ["sh", os.path.join(o, "scripts", "other.sh"), os.path.join(d, "deploy")], ns, d),
                                 ("a path with a semicolon", ["sh", script, os.path.join(sb, "d;touch " + marker, "deploy")], ns, d),
                                 ("a path with a space", ["sh", os.path.join(spaced, "scripts", "s5-check.sh"), os.path.join(d, "deploy")], ns, d),
                                 ("a third argument", ["sh", script, os.path.join(d, "deploy"), "extra"], ns, d),
                                 ("the exact command in the tool tree's working directory", exact, ns, o),
                                 ("the exact command without --oracle-tree", exact, type("Args", (), {"oracle_tree": None, "dev_tree": d})(), d)):
        ctx2 = mod.Ctx(None, rec, args_ns)
        try:
            ctx2.way_c_step(argv, cwd=cwd)
            raised = False
        except mod.Refused:
            raised = True
        check(raised and not os.path.exists(marker) and not os.path.exists(script + ".ran") and not os.path.exists(script2 + ".ran"), "%s was not refused (raised=%s)" % (label, raised))


@cell("GR-145 way C (nix's specification answer of 12 Nov): with --oracle-tree an UNBOUND oracle (its commit or tree identity cannot be read) FAILS G12: the numeric git status is in the verdict, the none-valued identity is still receipted, the candidate's own checker is NOT used as a silent fallback, and the oracle checker's clean output does not rescue it")
def gr145():
    sb = wayc_sandbox(git_repo=False, dev_out=NOTE_LINE + "\n")   # the candidate's own checker is clean, and the oracle's checker prints the NOTE once
    run(sb, g12m_args(sb, bind=False))
    row = tsv(sb)["G12"]
    check(row[1] == "FAIL", "G12 did not fail with an unbound oracle: %s" % (row,))
    check(re.search(r"identity is not bound: its commit cannot be read \(git rc [0-9]+\)", row[2]) is not None, "the verdict does not carry the numeric git status of the unreadable commit: %s" % row[2])
    check(re.search(r"identity is not bound: its tree id cannot be read \(git rc [0-9]+\)", row[2]) is not None, "the verdict does not carry the numeric git status of the unreadable tree id: %s" % row[2])
    text = wayc_identity_receipt(sb)
    check("oracle_commit=none (git rc " in text and "oracle_tree_id=none (git rc " in text, "the none-valued identity is not receipted: %s" % text[:500])
    check(not matrix_receipt(sb), "the matrix ran with an unbound oracle (F4: it is never loaded): %s" % matrix_receipt(sb)[:200])   # was: the matrix receipt records the none-valued identity; now no matrix runs
    check(not os.path.exists(os.path.join(sb, "dev-checker-ran")), "the candidate's own checker ran as a silent fallback")


@cell("GR-146 way C: an oracle whose checker hash is UNAVAILABLE fails G12 and says which file: scripts/s5-check.sh or scripts/s5_check.py missing, even with a readable git identity")
def gr146():
    for name in ("s5-check.sh", "s5_check.py"):
        sb = wayc_sandbox(dev_out=NOTE_LINE + "\n")
        args = g12_args(sb)
        os.remove(os.path.join(sb, "oracle", "scripts", name))   # after the oracle's commit and after the builder: the identity reads, the file does not
        run(sb, args)
        row = tsv(sb)["G12"]
        check(row[1] == "FAIL" and "hash is unavailable" in row[2] and name in row[2], "%s missing: %s" % (name, row))
        check("oracle_commit=" + git(os.path.join(sb, "oracle"), "rev-parse", "HEAD") in wayc_identity_receipt(sb), "%s missing: the identity receipt lost the commit" % name)


@cell("GR-147 way C, the POSITIVE cell: a bound tool identity DISTINCT from the candidate's: G12 passes with an oracle tree that is a git tree of its own; the receipt shows the tool's commit, tree id and checker hash different from the candidate's, and oracle_is_development_tree=NO")
def gr147():
    sb = wayc_sandbox()
    run(sb, g12_args(sb))
    row = tsv(sb)["G12"]
    check(row[1] == "PASS", "G12 %s" % (row,))
    dev, orc = os.path.join(sb, "dev"), os.path.join(sb, "oracle")
    o_commit, o_tree, c_commit, c_tree = git(orc, "rev-parse", "HEAD"), git(orc, "rev-parse", "HEAD^{tree}"), git(dev, "rev-parse", "HEAD"), git(dev, "rev-parse", "HEAD^{tree}")
    check(re.fullmatch(r"[0-9a-f]{40}", o_commit) and re.fullmatch(r"[0-9a-f]{40}", o_tree), "the tool tree has no real git identity: %s %s" % (o_commit, o_tree))
    check(o_commit != c_commit and o_tree != c_tree, "the tool's identity equals the candidate's: %s %s" % (o_commit, c_commit))
    o_sha, c_sha = sha(os.path.join(orc, "scripts", "s5-check.sh")), sha(os.path.join(dev, "scripts", "s5-check.sh"))
    check(o_sha != c_sha, "the fixture's two checkers are the same file")
    text = wayc_identity_receipt(sb)
    for want in ("oracle_commit=" + o_commit, "candidate_commit=" + c_commit, "oracle_tree_id=" + o_tree, "candidate_tree=" + c_tree, "oracle_checker_sha256=" + o_sha, "oracle_is_development_tree=NO"):
        check(want in text, "the identity receipt lacks %r: %s" % (want, text[:700]))
    check("oracle_checker_sha256=" + c_sha not in text, "the receipt names the CANDIDATE's checker hash as the tool's")


# ---- way C, pin A completed (cox's one list, 5350b273, items F1 and F2): the tool tree must be KNOWN and CLEAN, and the receipt binds the executed wrapper, the checker and the running runner ----
def wayc_run_pass(**kw):
    sb = wayc_sandbox(**kw)
    args = g12_args(sb)
    return sb, args


@cell("GR-148 way C F1 (cox's list; kai badf8ed5; nix c3f5493b): with --oracle-tree the tool tree must be CLEAN in scripts/: a modified file and a staged change in the oracle's scripts/ each FAIL G12 naming scripts/ and 'uncommitted', the status read is a receipt the verdict row names, and the checker's clean output does not rescue it; a change OUTSIDE scripts/ and a clean oracle PASS with oracle_scripts_clean=yes")
def gr148():
    for label, staged in (("a modified file", False), ("a staged change", True)):
        sb, args = wayc_run_pass(dev_out=NOTE_LINE + "\n")
        orc = os.path.join(sb, "oracle")
        open(os.path.join(orc, "scripts", "s5_check.py"), "a").write("# an edit after the commit\n")
        if staged:
            git(orc, "add", "scripts/s5_check.py")
        run(sb, args)
        row = tsv(sb)["G12"]
        check(row[1] == "FAIL" and "uncommitted" in row[2] and "scripts/" in row[2], "%s: %s" % (label, row))
        check("-G12-oracle_g_status" in row[3], "%s: the verdict row does not name the status receipt: %s" % (label, row[3]))
        check("oracle_scripts_clean=NO" in wayc_identity_receipt(sb), "%s: the identity receipt does not say the scripts are not clean: %s" % (label, wayc_identity_receipt(sb)[:600]))
    sb, args = wayc_run_pass()
    open(os.path.join(sb, "oracle", "deploy", "install.sh"), "a").write("# an edit outside scripts/\n")
    run(sb, args)
    row = tsv(sb)["G12"]
    check(row[1] == "PASS" and "oracle_scripts_clean=yes" in wayc_identity_receipt(sb), "a change outside scripts/ must not fail: %s" % (row,))
    sb, args = wayc_run_pass()
    run(sb, args)
    check(tsv(sb)["G12"][1] == "PASS" and "oracle_scripts_clean=yes" in wayc_identity_receipt(sb), "a clean oracle: %s" % (tsv(sb)["G12"],))


@cell("GR-149 way C F1: the cleanliness READ that fails proves nothing: with an oracle that is not a git tree the verdict carries the status read's numeric exit beside the commit and tree ones, and the receipt says so")
def gr149():
    sb = wayc_sandbox(git_repo=False, dev_out=NOTE_LINE + "\n")
    run(sb, g12m_args(sb, bind=False))
    row = tsv(sb)["G12"]
    check(row[1] == "FAIL" and re.search(r"cleanliness read failed \(rc [0-9]+\)", row[2]) is not None, "the verdict lacks the numeric status of the cleanliness read: %s" % (row,))
    check("oracle_scripts_clean=unknown (git rc " in wayc_identity_receipt(sb), "the receipt does not record the unreadable status: %s" % wayc_identity_receipt(sb)[:600])
    check(re.search(r"git top level cannot be read \(git rc [0-9]+\)", row[2]) is not None, "the verdict lacks the numeric status of the top-level read: %s" % row[2])
    check("oracle_git_toplevel=none (git rc " in wayc_identity_receipt(sb), "the receipt does not record the unreadable top level: %s" % wayc_identity_receipt(sb)[:700])


@cell("GR-150 way C F2 (vera's return on eec6febc): the receipt binds the EXECUTED WRAPPER, the checker and the RUNNING runner: oracle_checker_sha256 (s5-check.sh), oracle_file_sha256 (s5_check.py) and gate_runner_sha256 with gate_runner_path are the hashes of the files themselves, and the exact command stays in the receipt")
def gr150():
    sb = wayc_sandbox()
    run(sb, g12_args(sb))
    check(tsv(sb)["G12"][1] == "PASS", "G12 %s" % (tsv(sb)["G12"],))
    text = wayc_identity_receipt(sb)
    orc, dev = os.path.join(sb, "oracle"), os.path.join(sb, "dev")
    for want in ("oracle_checker_sha256=" + sha(os.path.join(orc, "scripts", "s5-check.sh")), "oracle_file_sha256=" + sha(os.path.join(orc, "scripts", "s5_check.py")),
                 "gate_runner_sha256=" + sha(RUNNER), "gate_runner_path=" + os.path.abspath(RUNNER), "gate_matrix_sha256=" + sha(os.path.join(os.path.dirname(os.path.abspath(RUNNER)), "gate_matrix.py")),
                 "oracle_tree_real=" + os.path.realpath(orc), "candidate_deploy_real=" + os.path.join(os.path.realpath(dev), "deploy"),
                 "command=sh " + os.path.join(orc, "scripts", "s5-check.sh") + " " + os.path.join(dev, "deploy")):
        check(want in text.splitlines(), "the receipt lacks the line %r: %s" % (want, text[:900]))
    check(sha(RUNNER) not in (sha(os.path.join(orc, "scripts", "s5-check.sh")), sha(os.path.join(orc, "scripts", "s5_check.py"))), "the fixture's files are the runner")


@cell("GR-151 way C F2: a wrapper or a checker that is a SYMBOLIC LINK, and a scripts/ directory that is one, are REFUSED: G12 FAILS naming the link, the checker is NOT run (no sentinel), and the identity is still receipted; the regular control passes (GR-147)")
def gr151():
    for what in ("s5-check.sh", "s5_check.py", "scripts"):
        sb = wayc_sandbox(dev_out=NOTE_LINE + "\n")
        args = g12_args(sb)
        orc = os.path.join(sb, "oracle")
        elsewhere = os.path.join(sb, "elsewhere")
        os.makedirs(elsewhere)
        if what == "scripts":
            shutil.move(os.path.join(orc, "scripts"), os.path.join(elsewhere, "scripts"))
            os.symlink(os.path.join(elsewhere, "scripts"), os.path.join(orc, "scripts"))
        else:
            shutil.move(os.path.join(orc, "scripts", what), os.path.join(elsewhere, what))
            os.symlink(os.path.join(elsewhere, what), os.path.join(orc, "scripts", what))
        git(orc, "add", "-A")
        git(orc, "commit", "-q", "-m", "link")   # committed: the tool tree is CLEAN, so the refusal is the link's and nothing else's
        run(sb, args)
        row = tsv(sb)["G12"]
        check(row[1] == "FAIL" and "symbolic link" in row[2] and what in row[2], "%s as a link: %s" % (what, row))
        check(not os.path.exists(os.path.join(sb, "oracle-argv.txt")), "%s as a link: the checker RAN" % what)
        check("oracle_commit=" + git(orc, "rev-parse", "HEAD") in wayc_identity_receipt(sb), "%s as a link: the identity was not receipted" % what)


@cell("GR-152 way C F2: the checker must judge the CANDIDATE's bytes: when the candidate's deploy directory resolves OUTSIDE the development tree (a symbolic link to a copy elsewhere, or to the tool tree's own deploy directory) G12 FAILS naming it, and the checker is NOT run; an oracle tree that IS the development tree is not refused for it")
def gr152():
    for label in ("a copy elsewhere", "the tool tree's own deploy directory"):
        sb = wayc_sandbox(dev_out=NOTE_LINE + "\n")
        args = g12_args(sb)
        dev, orc = os.path.join(sb, "dev"), os.path.join(sb, "oracle")
        if label == "a copy elsewhere":
            shutil.move(os.path.join(dev, "deploy"), os.path.join(sb, "deploy-elsewhere"))
            os.symlink(os.path.join(sb, "deploy-elsewhere"), os.path.join(dev, "deploy"))
        else:
            shutil.rmtree(os.path.join(dev, "deploy"))
            os.symlink(os.path.join(orc, "deploy"), os.path.join(dev, "deploy"))
        run(sb, args)
        row = tsv(sb)["G12"]
        check(row[1] == "FAIL" and "deploy directory" in row[2] and "outside the development tree" in row[2], "%s: %s" % (label, row))
        check(not os.path.exists(os.path.join(sb, "oracle-argv.txt")), "%s: the checker RAN on a deploy directory that is not the candidate's" % label)
    sb = g12_sandbox(oracle_out=NOTE_LINE + "\n")
    dev = os.path.join(sb, "dev")
    open(os.path.join(dev, "scripts", "s5_check.py"), "w").write(FAKE_ORACLE)
    args = g12_args(sb)
    args[args.index("--oracle-tree") + 1] = dev
    run(sb, args)
    check("outside the development tree" not in tsv(sb)["G12"][2], "an oracle tree that IS the development tree was refused: %s" % (tsv(sb)["G12"],))
    sb = wayc_sandbox(dev_out=NOTE_LINE + "\n")   # the development tree itself reached through a symbolic link: its deploy directory still resolves inside it
    args = g12_args(sb)
    alias = os.path.join(sb, "dev-alias")
    os.symlink(os.path.join(sb, "dev"), alias)
    args[args.index("--dev-tree") + 1] = alias
    run(sb, args)
    check("outside the development tree" not in tsv(sb)["G12"][2], "a development tree reached through a symbolic link was refused: %s" % (tsv(sb)["G12"],))


@cell("GR-153 way C (vera's row 152f59bc; ellis: the identity is read twice with no cross-check): the identity the MATRIX reads later must be the one step (g) bound: a commit that changes, a tree that changes, a git identity that becomes unreadable (the numeric status in the verdict) and a checker file that changes between the two reads each FAIL G12; a later none or mismatch cannot leave it PASS")
def gr153():
    commit = 'git -C "$HOME/oracle" -c user.name=t -c user.email=t@t commit -q '
    cases = (("the oracle's commit changes after step (g)", commit + '--allow-empty -m later\n', r"commit changed between step \(g\) and the matrix"),
             ("the oracle's tree changes after step (g)", 'echo x > "$HOME/oracle/deploy/later.txt"\ngit -C "$HOME/oracle" add -A\n' + commit + '-m later\n', r"tree id changed between step \(g\) and the matrix"),
             ("the oracle's git identity becomes unreadable", 'rm -rf "$HOME/oracle/.git"\n', r"not bound at the matrix: its commit cannot be read \(git rc [0-9]+\).*not bound at the matrix: its tree id cannot be read \(git rc [0-9]+\)"),
             ("the oracle's checker file changes", 'echo "# later" >> "$HOME/oracle/scripts/s5_check.py"\n', r"s5_check\.py changed between step \(g\) and the matrix"))
    for label, after, pattern in cases:
        sb = wayc_sandbox(dev_out=NOTE_LINE + "\n", after=after)
        run(sb, g12_args(sb))
        row = tsv(sb)["G12"]
        check(row[1] == "FAIL" and re.search(pattern, row[2]) is not None, "%s: %s" % (label, row))
    sb = wayc_sandbox(dev_out=NOTE_LINE + "\n", after=":\n")   # the control: the wrapper changes nothing
    run(sb, g12_args(sb))
    check(tsv(sb)["G12"][1] == "PASS", "the unchanged control: %s" % (tsv(sb)["G12"],))


@cell("GR-154 way C (ellis: git -C inside an enclosing repository returns the ENCLOSING repository's ids): the oracle tree must be its OWN git top level: a plain directory inside another repository FAILS G12 naming the enclosing repository, with its commit and tree ids otherwise readable and its scripts/ clean, and the receipt records the top level; a tree of its own passes (GR-147)")
def gr154():
    sb = wayc_sandbox(dev_out=NOTE_LINE + "\n")
    args = g12_args(sb)
    encl = os.path.join(sb, "encl")
    shutil.copytree(os.path.join(sb, "oracle"), os.path.join(encl, "tool"), ignore=shutil.ignore_patterns(".git"))
    git(encl, "init", "-q")
    git(encl, "add", "-A")
    git(encl, "commit", "-q", "-m", "enclosing")
    args[args.index("--oracle-tree") + 1] = os.path.join(encl, "tool")
    run(sb, args)
    row = tsv(sb)["G12"]
    check(row[1] == "FAIL" and "own git top level" in row[2] and os.path.realpath(encl) in row[2], "a tool tree inside another repository: %s" % (row,))
    check("oracle_git_toplevel=" + os.path.realpath(encl) in wayc_identity_receipt(sb), "the receipt does not record the top level: %s" % wayc_identity_receipt(sb)[:700])
    check("uncommitted" not in row[2] and "cannot be read" not in row[2], "the toplevel refusal is not the only one: %s" % row[2])


@cell("GR-155 way C (ellis F-1 and F-4): the identity receipt is written BEFORE the checker runs (its sequence number is lower than the checker step's), and an oracle tree reached through a SYMBOLIC LINK to the development tree is still the development tree (oracle_is_development_tree=yes: the comparison is by real path), both real paths receipted")
def gr155():
    sb = wayc_sandbox(dev_out=NOTE_LINE + "\n")
    run(sb, g12_args(sb))
    ev = os.path.join(sb, "ev")
    ident = [int(f.split("-")[0]) for f in os.listdir(ev) if f.endswith("-G12-oracle_identity.out")]
    step = [int(f.split("-")[0]) for f in os.listdir(ev) if f.endswith("-G12-oracle.cmd")]
    check(len(ident) == 1 and len(step) == 1 and ident[0] < step[0], "the identity receipt is not written before the checker runs: identity %s checker %s" % (ident, step))
    sb = g12_sandbox(oracle_out=NOTE_LINE + "\n")
    dev = os.path.join(sb, "dev")
    open(os.path.join(dev, "scripts", "s5_check.py"), "w").write(FAKE_ORACLE)
    args = g12_args(sb)
    alias = os.path.join(sb, "dev-alias")
    os.symlink(dev, alias)
    args[args.index("--oracle-tree") + 1] = alias
    run(sb, args)
    text = wayc_identity_receipt(sb)
    for want in ("oracle_tree=" + alias, "oracle_is_development_tree=yes", "oracle_tree_real=" + os.path.realpath(dev)):
        check(want in text.splitlines(), "an alias of the development tree: the receipt lacks %r: %s" % (want, text[:700]))
    check("own git top level" not in tsv(sb)["G12"][2], "an oracle tree reached through a symbolic link was refused as not its own top level: %s" % (tsv(sb)["G12"],))


# ---- pin A repair (vera's row 76ed0a98; ellis r2: F4 and F5 of cox's list): a refused or unbound oracle is NEVER loaded by a later step; each of four hashes is read ONCE and an unreadable one FAILS ----
def imported_marker(sb):
    return os.path.join(sb, "oracle-imported.txt")


def run_marked(sb, args):
    return run(sb, args, env_extra={"GR_ORACLE_IMPORTED": imported_marker(sb)})


def load_gm():
    spec = importlib.util.spec_from_file_location("gm_pin_a_repair", os.path.join(os.path.dirname(os.path.abspath(RUNNER)), "gate_matrix.py"))
    gm = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(gm)
    return gm


def refused_cases():
    """(label, a function that builds the sandbox and the argument list for ONE case of a refused or unbound oracle)."""
    def links(what):
        def build():
            sb = wayc_sandbox(dev_out=NOTE_LINE + "\n")
            args = g12_args(sb)
            orc, elsewhere = os.path.join(sb, "oracle"), os.path.join(sb, "elsewhere")
            os.makedirs(elsewhere)
            if what == "scripts":
                shutil.move(os.path.join(orc, "scripts"), os.path.join(elsewhere, "scripts"))
                os.symlink(os.path.join(elsewhere, "scripts"), os.path.join(orc, "scripts"))
            else:
                shutil.move(os.path.join(orc, "scripts", what), os.path.join(elsewhere, what))
                os.symlink(os.path.join(elsewhere, what), os.path.join(orc, "scripts", what))
            git(orc, "add", "-A")
            git(orc, "commit", "-q", "-m", "link")
            return sb, args
        return build

    def unbound():
        sb = wayc_sandbox(git_repo=False, dev_out=NOTE_LINE + "\n")
        return sb, g12m_args(sb, bind=False)

    def dirty():
        sb = wayc_sandbox(dev_out=NOTE_LINE + "\n")
        args = g12_args(sb)
        open(os.path.join(sb, "oracle", "scripts", "s5_check.py"), "a").write("# an edit after the commit\n")
        return sb, args

    def foreign_top():
        sb = wayc_sandbox(dev_out=NOTE_LINE + "\n")
        args = g12_args(sb)
        encl = os.path.join(sb, "encl")
        shutil.copytree(os.path.join(sb, "oracle"), os.path.join(encl, "tool"), ignore=shutil.ignore_patterns(".git"))
        git(encl, "init", "-q")
        git(encl, "add", "-A")
        git(encl, "commit", "-q", "-m", "enclosing")
        args[args.index("--oracle-tree") + 1] = os.path.join(encl, "tool")
        return sb, args

    def no_checker_file():
        sb = wayc_sandbox(dev_out=NOTE_LINE + "\n")
        args = g12_args(sb)
        os.remove(os.path.join(sb, "oracle", "scripts", "s5_check.py"))
        return sb, args

    def deploy_outside():
        sb = wayc_sandbox(dev_out=NOTE_LINE + "\n")
        args = g12_args(sb)
        shutil.move(os.path.join(sb, "dev", "deploy"), os.path.join(sb, "deploy-elsewhere"))
        os.symlink(os.path.join(sb, "deploy-elsewhere"), os.path.join(sb, "dev", "deploy"))
        return sb, args
    return (("a linked scripts/ directory", links("scripts")), ("a linked wrapper", links("s5-check.sh")), ("a linked checker", links("s5_check.py")),
            ("an oracle that is not a git tree", unbound), ("an oracle whose scripts/ has uncommitted changes", dirty), ("an oracle inside another repository", foreign_top),
            ("an oracle whose checker file is missing", no_checker_file), ("a candidate deploy directory outside the development tree", deploy_outside))


@cell("GR-173 F4 (cox's list; ellis r2 P2; vera 76ed0a98): a REFUSED or UNBOUND oracle is not loaded or executed by ANY later step: for a linked scripts/, wrapper or checker, an oracle that is not a git tree, one with uncommitted changes, one inside another repository, one whose checker file is missing and a candidate deploy outside the development tree, the oracle's top level (a sentinel written when its module is imported) NEVER runs, no matrix receipt exists and G12 FAILS saying the matrix was NOT run; the bound control imports it (so the sentinel works) and passes")
def gr173():
    sb = wayc_sandbox(dev_out=NOTE_LINE + "\n")
    run_marked(sb, g12_args(sb))
    check(tsv(sb)["G12"][1] == "PASS" and os.path.exists(imported_marker(sb)), "the bound control: G12 %s, the oracle was imported: %s" % (tsv(sb)["G12"][1], os.path.exists(imported_marker(sb))))
    for label, build in refused_cases():
        sb, args = build()
        run_marked(sb, args)
        row = tsv(sb)["G12"]
        check(not os.path.exists(imported_marker(sb)), "%s: the oracle's top level RAN (it was imported by a later step)" % label)
        check(row[1] == "FAIL" and "the matrix was NOT run: the oracle is not bound" in row[2], "%s: %s" % (label, row))
        check(not matrix_receipt(sb), "%s: a matrix receipt exists: %s" % (label, matrix_receipt(sb)[:120]))


@cell("GR-174 F4, in depth: load_oracle itself refuses a symbolic link, the checker or the scripts/ directory it would execute: it raises OracleMissing naming the link and the module's top level does not run; a regular file is loaded (the sentinel shows it)")
def gr174():
    gm = load_gm()
    sb = sandbox()
    marker = imported_marker(sb)
    old = os.environ.get("GR_ORACLE_IMPORTED")
    os.environ["GR_ORACLE_IMPORTED"] = marker
    try:
        def tree(name):
            t = os.path.join(sb, name)
            os.makedirs(os.path.join(t, "scripts"))
            return t
        real = os.path.join(sb, "real-s5_check.py")
        open(real, "w").write(FAKE_ORACLE)
        t1 = tree("t1")
        os.symlink(real, os.path.join(t1, "scripts", "s5_check.py"))
        t2 = os.path.join(sb, "t2")
        os.makedirs(os.path.join(sb, "real-scripts"))
        open(os.path.join(sb, "real-scripts", "s5_check.py"), "w").write(FAKE_ORACLE)
        os.makedirs(t2)
        os.symlink(os.path.join(sb, "real-scripts"), os.path.join(t2, "scripts"))
        for label, t in (("a linked checker", t1), ("a linked scripts/ directory", t2)):
            try:
                gm.load_oracle(t)
                raised = ""
            except gm.OracleMissing as exc:
                raised = str(exc)
            check("symbolic link" in raised, "%s: not refused (raised %r)" % (label, raised))
            check(not os.path.exists(marker), "%s: the module's top level ran" % label)
        t3 = tree("t3")
        open(os.path.join(t3, "scripts", "s5_check.py"), "w").write(FAKE_ORACLE)
        gm.load_oracle(t3)
        check(os.path.exists(marker), "a regular file was not imported (the sentinel does not work)")
    finally:
        if old is None:
            os.environ.pop("GR_ORACLE_IMPORTED", None)
        else:
            os.environ["GR_ORACLE_IMPORTED"] = old


@cell("GR-175 F5 (cox's list; ellis r2 F-7 and F-8): the four hashes of way_c_identity (the wrapper, the checker, the running runner, the matrix module) are each read ONCE, the same value in the receipt and in the check, and an unreadable one is a PROBLEM that names the file: the wrapper unreadable on its FIRST read only (a second read would succeed), the runner, the matrix module and the checker unreadable each fail; with nothing patched there is no such problem")
def gr175():
    mod = load_module()
    sb = wayc_sandbox()
    g12_args(sb)
    ns = type("Args", (), {"oracle_tree": os.path.join(sb, "oracle"), "dev_tree": os.path.join(sb, "dev")})()
    here = os.path.dirname(os.path.abspath(mod.__file__))
    paths = {"wrapper": os.path.join(ns.oracle_tree, "scripts", "s5-check.sh"), "checker": os.path.join(ns.oracle_tree, "scripts", "s5_check.py"),
             "runner": os.path.abspath(mod.__file__), "matrix": os.path.join(here, "gate_matrix.py")}
    real = mod.sha256_file

    def attempt(fail_on=None, first_only=False):
        calls = {}
        def patched(path):
            calls[path] = calls.get(path, 0) + 1
            if fail_on and path == paths[fail_on] and (calls[path] == 1 or not first_only):
                raise OSError("planted")
            return real(path)
        mod.sha256_file = patched
        try:
            rec = mod.Recorder(os.path.join(sb, "ev-%s-%s" % (fail_on, first_only)))
            ctx = mod.Ctx(None, rec, ns)
            receipts, problems, refused, seen = mod.way_c_identity(ctx, ns, "c" * 40, "d" * 40, mod.way_c_command(ns))
            text = open(os.path.join(rec.path, receipts[-1] + ".out")).read()
        finally:
            mod.sha256_file = real
        return calls, problems, text
    calls, problems, text = attempt()
    check(not [x for x in problems if "hash" in x or "cannot be hashed" in x], "a problem with nothing patched: %s" % problems)
    check(all(calls.get(path, 0) == 1 for path in paths.values()), "a file was hashed more or less than once in way_c_identity: %s" % {k: calls.get(v, 0) for k, v in paths.items()})
    for label, key, first_only, word in (("the wrapper, first read only", "wrapper", True, "s5-check.sh"), ("the wrapper, every read", "wrapper", False, "s5-check.sh"),
                                         ("the checker", "checker", False, "s5_check.py"), ("the running runner", "runner", False, "gate_runner.py"), ("the matrix module", "matrix", False, "gate_matrix.py")):
        calls, problems, text = attempt(key, first_only)
        check(any(word in x and ("unavailable" in x or "cannot be" in x) for x in problems), "%s unreadable: no problem names %s: %s" % (label, word, problems))
        check("=unreadable" in text, "%s unreadable: the receipt does not say so: %s" % (label, text[:300]))


@cell("GR-178 F4 for the LATER read (cox's observation on the repaired pin A, row 5651bc45): an oracle whose commit, tree id or s5_check.py CHANGED between step (g) and the matrix, or whose git identity became unreadable, is reported AND NOT LOADED: the oracle's top level (a sentinel written when its module is imported) never runs, no matrix receipt exists, and G12 FAILS saying the matrix was NOT run; the unchanged control imports it and passes")
def gr178():
    commit = 'git -C "$HOME/oracle" -c user.name=t -c user.email=t@t commit -q '
    cases = (("the oracle's commit changes after step (g)", commit + '--allow-empty -m later\n', "commit changed between step"),
             ("the oracle's tree changes after step (g)", 'echo x > "$HOME/oracle/deploy/later.txt"\ngit -C "$HOME/oracle" add -A\n' + commit + '-m later\n', "tree id changed between step"),
             ("the oracle's git identity becomes unreadable", 'rm -rf "$HOME/oracle/.git"\n', "not bound at the matrix"),
             ("the oracle's checker file changes", 'echo "# later" >> "$HOME/oracle/scripts/s5_check.py"\n', "s5_check.py changed between step"))
    for label, after, word in cases:
        sb = wayc_sandbox(dev_out=NOTE_LINE + "\n", after=after)
        run_marked(sb, g12_args(sb))
        row = tsv(sb)["G12"]
        check(row[1] == "FAIL" and word in row[2] and "the matrix was NOT run: the oracle is not the one step (g) bound" in row[2], "%s: %s" % (label, row))
        check(not os.path.exists(imported_marker(sb)), "%s: the oracle's top level RAN: it was loaded although it changed after step (g)" % label)
        check(not matrix_receipt(sb), "%s: a matrix receipt exists: %s" % (label, matrix_receipt(sb)[:120]))
    sb = wayc_sandbox(dev_out=NOTE_LINE + "\n", after=":\n")
    run_marked(sb, g12_args(sb))
    check(tsv(sb)["G12"][1] == "PASS" and os.path.exists(imported_marker(sb)), "the unchanged control: G12 %s, imported %s" % (tsv(sb)["G12"][1], os.path.exists(imported_marker(sb))))


@cell("GR-184 the DEFAULT path with the matrix (no --oracle-tree): the development tree is its own oracle, its checker and its s5_check.py run, the matrix runs and G12 PASSES, and no oracle identity of way C is receipted (cox/ellis: nothing covered the default path through the matrix)")
def gr184():
    sb = g12_sandbox(oracle_out=NOTE_LINE + "\n")
    dev = os.path.join(sb, "dev")
    open(os.path.join(dev, "scripts", "s5_check.py"), "w").write(FAKE_ORACLE)   # untracked: the tree stays clean for the status read of the binding
    args = g12_args(sb)
    i = args.index("--oracle-tree")
    del args[i:i + 2]
    run(sb, args)
    row = tsv(sb)["G12"]
    check(row[1] == "PASS" and "from the development tree" in row[2], "the default path through the matrix: %s" % (row,))
    check(wayc_identity_receipt(sb) == "" and matrix_receipt(sb) != "", "the way-C identity was receipted, or the matrix did not run")


def main():
    only = sys.argv[1:]
    failed = 0
    for name, fn in CELLS:
        if only and not any(name.startswith(o) for o in only):
            continue
        try:
            fn()
            print("ok " + name)
        except AssertionError as e:
            failed += 1
            print("FAIL %s: %s" % (name.split(" ", 1)[0], e))
        except Exception as e:
            failed += 1
            print("FAIL %s: cell error %s: %s" % (name.split(" ", 1)[0], type(e).__name__, e))
    if os.environ.get("GR_KEEP") != "1":
        for d in SANDBOXES:
            shutil.rmtree(d, ignore_errors=True)
    print("%d cells, %d failed" % (len([1 for n, _ in CELLS if not only or any(n.startswith(o) for o in only)]), failed))
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
