#!/usr/bin/python3 -I
"""6f gate runner (nix), increments R1 and R2: arguments, refusal of this host, receipts, SHA256SUMS, verdict states, interruption,
and the two checks G1 (clean start) and G3 (healthy).  The other ten checks report NOT RUN until their increments exist.

An EVIDENCE COLLECTOR and a RUNBOOK, not a verdict machine: each check's pass rule is plain code below.  Run it only as
    python3 -I gate_runner.py --target USER@HOST --evidence-dir NEWDIR ...
Standard library only.  Every command sent to the target is on the ALLOWED list; nothing else is ever sent.  The runner never
writes its own environment, a token or a secret."""
import argparse, hashlib, importlib.util, ipaddress, json, os, pty, re, select, shlex, shutil, signal, stat, subprocess, sys, tempfile, time, traceback, uuid

EX_PASS, EX_FAIL, EX_USAGE, EX_INCOMPLETE = 0, 1, 2, 3
EX_REFUSED_NAME, EX_REFUSED_ADDR, EX_REFUSED_MACHINE_ID, EX_REFUSED_HOSTNAME, EX_REFUSED_CONTAINER = 10, 11, 12, 13, 14
EX_INTERRUPTED = 130

STATE_ORDER = ["G1", "G10", "G7", "G2", "G9", "G3", "G4", "G5", "G11", "G8", "G12", "G6"]
TABLE_ORDER = ["G%d" % i for i in range(1, 13)]
LIVE_HOSTS = ("build-host-1", "build-host-2")
LIVE_CONTAINER_PREFIXES = ("cortex-", "cv2-", "spike-s0-", "harness-appdb")
CONTAINER_NAME = re.compile(r"[a-z][a-z0-9_.-]{0,62}")
HOME_ARG = re.compile(r"(\$HOME|/[A-Za-z0-9._-]+)(/[A-Za-z0-9._-]+)*")
UTC = "%Y-%m-%dT%H:%M:%SZ"
NAME_ARG = re.compile(r"[a-z][a-z0-9-]{0,31}")
ARTEFACT_NAME = re.compile(r"[A-Za-z0-9._-]{1,128}")
ANSI = re.compile(rb"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[@-Z\\-_]")
PROMPT = re.compile(r"(\[y/N\]|\[Y/n\]|\(y/n\)\??|[Pp]assword[^\n]*:|continue\?|Proceed[^\n]*\?)\s*$")

# vera VR14-01: absence is believed only after a WALK down the whole ancestor chain from $HOME: every component that exists is stepped into; the first one that does not
# exist is 'absent' only when the directory we stand in is searchable (x), else 'unreadable'. The walk sets st to exists, absent or unreadable.
WALK = ('w() { cur="$HOME"; rest="$1"; st=exists; while [ -n "$rest" ]; do c=${rest%%/*}; case "$rest" in */*) rest=${rest#*/};; *) rest=;; esac; '
        'nxt="$cur/$c"; if [ -e "$nxt" ] || [ -L "$nxt" ]; then cur="$nxt"; elif [ -d "$cur" ] && [ -x "$cur" ]; then st=absent; break; else st=unreadable; break; fi; done; }')


def dir_probe(rel, name):
    """Shell text (after WALK) that lists $HOME/rel and tells VERIFIED ABSENCE from a read error: '@@name RC' after a listing (RC is ls's own status), '@@name absent'
    only after the ancestor walk found the entry missing in a searchable directory, else '@@name unreadable' or '@@name notdir' (vera VR13-03, VR14-01)."""
    return ('w "%s"; case "$st" in absent) echo "@@%s absent";; unreadable) echo "@@%s unreadable";; *) d="$HOME/%s"; '
            'if [ -d "$d" ]; then ls -A "$d"; echo "@@%s $?"; else echo "@@%s notdir"; fi;; esac') % (rel, name, name, rel, name, name)


G1_DIRS = (WALK + '; for r in ".local/share/containers" ".config/containers"; do w "$r"; case "$st" in exists) echo "present $HOME/$r";; absent) echo "absent $HOME/$r";; '
           '*) echo "unreadable $HOME/$r";; esac; done')
G1_UNITS = WALK + "; " + dir_probe(".config/systemd/user", "user_units")
TRIAL_STATE = ("{ " + WALK + "; " + dir_probe("gate-trial-home", "trial_home") + "; " + dir_probe(".config/systemd/user", "user_units") +
               "; podman ps -a --format '{{.ID}} {{.Names}}'; echo \"@@podman_ps $?\"; ss -ltnH; echo \"@@ports $?\"; ls -A \"$HOME\"; echo \"@@home $?\"; } 2>&1")
# the names THIS RUNNER puts into $HOME while G7 and G10 run; anything else that appears there, gate-trial-home included, is a change (vera VR14-02)
RUNNER_HOME_NAMES = re.compile(r"gate-(artefacts|tamper-checksum|tamper-signature|shim|fakeroot|listener\.pid|lead\.hdr|worker\.hdr|lead-whoami\.json|worker-issued\.json)")
# key -> (template, pattern): the whole allow-list.  A template is filled only with validated parameters.
COMMANDS = {
    "identity_machine_id": ("cat /etc/machine-id", r"cat /etc/machine-id"),
    "identity_hostname": ("hostname", r"hostname"),
    "g1_os_release": ("cat /etc/os-release", r"cat /etc/os-release"),
    "g1_rpm_qa": ("rpm -qa --qf '%{NAME} %{VERSION}-%{RELEASE}.%{ARCH}\\n'", r"rpm -qa --qf '%\{NAME\} %\{VERSION\}-%\{RELEASE\}\.%\{ARCH\}\\n'"),
    "g1_ports": ("ss -ltnH", r"ss -ltnH"),
    "g1_cmd_podman": ("command -v podman", r"command -v (podman|cargo|rustc)"),
    "g1_cmd_cargo": ("command -v cargo", r"command -v (podman|cargo|rustc)"),
    "g1_cmd_rustc": ("command -v rustc", r"command -v (podman|cargo|rustc)"),
    "g1_podman_dirs": (G1_DIRS, re.escape(G1_DIRS)),
    "g1_user_units": (G1_UNITS, re.escape(G1_UNITS)),
    "g1_rocky_keys": ("sha256sum /etc/pki/rpm-gpg/RPM-GPG-KEY-Rocky-*", r"sha256sum /etc/pki/rpm-gpg/RPM-GPG-KEY-Rocky-\*"),
    "g1_rocky_pkgs": ("rpm -q rocky-release rocky-gpg-keys", r"rpm -q rocky-release rocky-gpg-keys"),
    "g1_gpg_pubkeys": ("rpm -qa 'gpg-pubkey*' --qf '%{NAME}-%{VERSION}-%{RELEASE} %{SUMMARY}\\n'",
                       r"rpm -qa 'gpg-pubkey\*' --qf '%\{NAME\}-%\{VERSION\}-%\{RELEASE\} %\{SUMMARY\}\\n'"),
    "g2_mkdir": ('mkdir -m 0700 "$HOME/gate-artefacts"', r'mkdir -m 0700 "\$HOME/gate-artefacts"'),
    "g2_put": ('cat > "$HOME/gate-artefacts/{name}"', r'cat > "\$HOME/gate-artefacts/[A-Za-z0-9._-]{1,128}"'),
    "g2_sums": ('sha256sum "$HOME"/gate-artefacts/*', r'sha256sum "\$HOME"/gate-artefacts/\*'),
    "g2_install": ('sh "$HOME/gate-artefacts/install.sh" --from "$HOME/gate-artefacts" --project {project} --lead {lead}',
                   r'sh "\$HOME/gate-artefacts/install\.sh" --from "\$HOME/gate-artefacts" --project [a-z][a-z0-9-]{0,31} --lead [a-z][a-z0-9-]{0,31}'),
    "g3_health": ("curl -fsS --max-time 10 http://127.0.0.1:{port}/v2/health/ready", r"curl -fsS --max-time 10 http://127\.0\.0\.1:[0-9]{1,5}/v2/health/ready"),
    "g3_compose": ('cat "{home}/current/compose.yml"', r'cat "(\$HOME|/)[A-Za-z0-9._/-]*/current/compose\.yml"'),
    "g3_stack_env": ('cat "{home}/current/stack.env"', r'cat "(\$HOME|/)[A-Za-z0-9._/-]*/current/stack\.env"'),
    "g4_mode": ("stat -c '%a' \"{home}/{name}.token\"", r"stat -c '%a' \"(\$HOME|/)[A-Za-z0-9._/-]*/[a-z][a-z0-9-]{0,31}\.token\""),
    "g5_containers": ("podman ps -a --format '{{.ID}} {{.Names}} {{.CreatedAt}} {{.Image}}'", r"podman ps -a --format '\{\{\.ID\}\} \{\{\.Names\}\} \{\{\.CreatedAt\}\} \{\{\.Image\}\}'"),
    "g5_files": ("find \"{home}\" -type f ! -name '*.log' -exec sha256sum {{}} +", r"find \"(\$HOME|/)[A-Za-z0-9._/-]*\" -type f ! -name '\*\.log' -exec sha256sum \{\} \+"),
    "g5_timers": ("systemctl --user list-unit-files --state=enabled --no-pager --no-legend", r"systemctl --user list-unit-files --state=enabled --no-pager --no-legend"),
    "g5_rows": ("podman exec --user postgres {container} psql -U postgres -d cortex -tAc \"select table_schema || '.' || table_name, (xpath('/row/c/text()', query_to_xml(format('select count(*) as c from %I.%I', table_schema, table_name), false, true, '')))[1]::text from information_schema.tables where table_schema not in ('pg_catalog','information_schema') and table_type='BASE TABLE' order by 1\"", None),
    "g11_enabled": ("systemctl --user is-enabled {timers}", r"systemctl --user is-enabled [a-z0-9@._-]+\.timer( [a-z0-9@._-]+\.timer)*"),
    "g11_active": ("systemctl --user is-active {timers}", r"systemctl --user is-active [a-z0-9@._-]+\.timer( [a-z0-9@._-]+\.timer)*"),
    "g11_linger": ('loginctl show-user "$(id -un)" -p Linger -p State', r'loginctl show-user "\$\(id -un\)" -p Linger -p State'),
    "g11_list": ("systemctl --user list-timers --all --no-pager --no-legend", r"systemctl --user list-timers --all --no-pager --no-legend"),
    "g11_sessions": ("loginctl list-sessions --no-legend", r"loginctl list-sessions --no-legend"),
    "g8_logs": ("podman logs {container}", r"podman logs [a-z][a-z0-9_.-]{0,62}"),
    "g3_migrate": ("podman inspect --format '{{{{.State.ExitCode}}}} {{{{.ImageName}}}}' {container}",
                   r"podman inspect --format '\{\{\.State\.ExitCode\}\} \{\{\.ImageName\}\}' [a-z][a-z0-9_.-]{0,62}"),
}
COMMANDS["g5_rows"] = (COMMANDS["g5_rows"][0], re.escape(COMMANDS["g5_rows"][0]).replace(re.escape("{container}"), "[a-z][a-z0-9_.-]{0,62}"))
def template_pattern(template, params, formatted=False):
    """The allow-list pattern of a fixed template: every character literal, each {parameter} a stated alphabet. A formatted template writes a real brace twice."""
    if formatted:
        template = template.replace("{{", "{").replace("}}", "}")
    pattern = re.escape(template)
    for key, alphabet in params.items():
        pattern = pattern.replace(re.escape("{%s}" % key), alphabet)
    return pattern


ART = "[A-Za-z0-9._-]{1,128}"
NAMES = "[a-z][a-z0-9-]{0,31}"
TRIAL = '--home "$HOME/gate-trial-home"'
RUN = 'sh "$HOME/gate-artefacts/install.sh" --from "$HOME/gate-artefacts" --project {project} --lead {lead} ' + TRIAL
FLIP = 'f="$HOME/gate-tamper-%s/{name}"; b=$(dd if="$f" bs=1 skip=%d count=1 2>/dev/null | od -An -tu1 | tr -d \' \'); printf "\\\\$(printf \'%%03o\' $(( (b + 1) %% 256 )))" | dd of="$f" bs=1 seek=%d conv=notrunc 2>/dev/null'
INJECT = {
    "g10_copy_checksum": ('cp -a "$HOME/gate-artefacts" "$HOME/gate-tamper-checksum"', {}),
    "g10_copy_signature": ('cp -a "$HOME/gate-artefacts" "$HOME/gate-tamper-signature"', {}),
    "g10_flip_checksum": (FLIP % ("checksum", 100, 100), {"name": ART}),
    "g10_flip_signature": (FLIP % ("signature", 10, 10), {"name": ART}),
    "g10_run_checksum": ('sh "$HOME/gate-tamper-checksum/install.sh" --from "$HOME/gate-tamper-checksum" --project {project} --lead {lead} ' + TRIAL + " --port 8602", {"project": NAMES, "lead": NAMES}),
    "g10_run_signature": ('sh "$HOME/gate-tamper-signature/install.sh" --from "$HOME/gate-tamper-signature" --project {project} --lead {lead} ' + TRIAL + " --port 8602", {"project": NAMES, "lead": NAMES}),
    "g10_cleanup": ('rm -rf "$HOME/gate-tamper-checksum" "$HOME/gate-tamper-signature"', {}),
    "trial_state": (TRIAL_STATE, {}),
    "g7_run_no_network": ("unshare --user --map-current-user --net " + RUN + " --port 8603", {"project": NAMES, "lead": NAMES}),
    "g7_listen": ("nohup python3 -c 'import os,socket,time; s=socket.socket(); s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1); s.bind((\"127.0.0.1\", 8604)); s.listen(1); open(os.path.expanduser(\"~/gate-listener.pid\"), \"w\").write(str(os.getpid())); time.sleep(300)' > /dev/null 2>&1 &", {}),
    "g7_run_port": (RUN + " --port 8604", {"project": NAMES, "lead": NAMES}),
    "g7_unlisten": ('kill "$(cat "$HOME/gate-listener.pid")"; rm -f "$HOME/gate-listener.pid"', {}),
    "g7_run_low_disk": ("CORTEX_MIN_FREE_GIB=100000 " + RUN + " --port 8605", {"project": NAMES, "lead": NAMES}),
    "g7_shim": ("mkdir -p \"$HOME/gate-shim\" && printf '#!/bin/sh\\necho \"podman version 4.9.0\"\\n' > \"$HOME/gate-shim/podman\" && chmod 755 \"$HOME/gate-shim/podman\"", {}),
    "g7_run_old_podman": ('PATH="$HOME/gate-shim:$PATH" ' + RUN + " --port 8606", {"project": NAMES, "lead": NAMES}),
    "g7_unshim": ('rm -rf "$HOME/gate-shim"', {}),
    "g7_linger_get": ('loginctl show-user "$(id -un)" -p Linger', {}),
    "g7_linger_off": ('loginctl disable-linger "$(id -un)"', {}),
    "g7_linger_on": ('loginctl enable-linger "$(id -un)"', {}),
    "g7_run_no_linger": (RUN + " --port 8607", {"project": NAMES, "lead": NAMES}),
    "g7_fakeroot": ("mkdir -p \"$HOME/gate-fakeroot/etc\" && printf 'ID=ubuntu\\nVERSION_ID=\"24.04\"\\n' > \"$HOME/gate-fakeroot/etc/os-release\"", {}),
    "g7_run_bad_os": ('PREREQ_ROOT="$HOME/gate-fakeroot" ' + RUN + " --port 8608", {"project": NAMES, "lead": NAMES}),
    "g7_unfakeroot": ('rm -rf "$HOME/gate-fakeroot"', {}),
}
for _key, (_template, _params) in INJECT.items():
    COMMANDS[_key] = (_template, template_pattern(_template, _params))

HOME_RX = r"(?:\$HOME|/)[A-Za-z0-9._/-]*"
UUID_RX = "[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}"
PORT_RX = "[0-9]{1,5}"
G4_TEMPLATES = {
    "g4_hdr_lead": ("umask 077; printf 'Authorization: Bearer %s\\n' \"$(cat \"{home}/{lead}.token\")\" > \"$HOME/gate-lead.hdr\"", {"home": HOME_RX, "lead": NAMES}),
    "g4_lead_whoami": ("curl -sS -H @\"$HOME/gate-lead.hdr\" -o \"$HOME/gate-lead-whoami.json\" -w '%{{http_code}}' http://127.0.0.1:{port}/v2/whoami", {"port": PORT_RX}),
    "g4_lead_fields": ("python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(d.get(\"worker\",\"\"), d.get(\"token_id\",\"\"))' \"$HOME/gate-lead-whoami.json\"", {}),
    "g4_issue": ("umask 077; curl -sS -X POST -H @\"$HOME/gate-lead.hdr\" -H \"Idempotency-Key: {idem}\" -o \"$HOME/gate-worker-issued.json\" -w '%{{http_code}}' http://127.0.0.1:{port}/v2/workers/{worker_id}/tokens",
                 {"idem": UUID_RX, "port": PORT_RX, "worker_id": UUID_RX}),
    "g4_hdr_worker": ("umask 077; python3 -c 'import json,sys; print(\"Authorization: Bearer \" + json.load(open(sys.argv[1]))[\"token\"])' \"$HOME/gate-worker-issued.json\" > \"$HOME/gate-worker.hdr\"", {}),
    "g4_worker_whoami": ("curl -sS -H @\"$HOME/gate-worker.hdr\" -w '\\n%{{http_code}}' http://127.0.0.1:{port}/v2/whoami", {"port": PORT_RX}),
    "g4_issued_ids": ("python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(d.get(\"token_id\",\"\"), \"token\" in d, d.get(\"expires_at\",\"\"))' \"$HOME/gate-worker-issued.json\"", {}),
    "g4_secret_issued": ("python3 -c 'import json,sys; sys.stdout.write(json.load(open(sys.argv[1]))[\"token\"])' \"$HOME/gate-worker-issued.json\"", {}),
    "g4_secret_lead": ("cat \"{home}/{lead}.token\"", {"home": HOME_RX, "lead": NAMES}),
    "g4_cleanup": ("rm -f \"$HOME/gate-lead.hdr\" \"$HOME/gate-worker.hdr\" \"$HOME/gate-lead-whoami.json\" \"$HOME/gate-worker-issued.json\"", {}),
}
for _key, (_template, _params) in G4_TEMPLATES.items():
    COMMANDS[_key] = (_template, template_pattern(_template, _params, formatted=bool(_params)))


ALLOWED = [re.compile(p) for _, p in COMMANDS.values()]


def command_allowed(cmd):
    return any(p.fullmatch(cmd) for p in ALLOWED)


SAFE_PATH = r"/[A-Za-z0-9._/-]+"
LOCAL_ALLOWED = [re.compile(p) for p in (
    r"sh -n " + SAFE_PATH, r"/usr/bin/bash --posix -n " + SAFE_PATH, r"sh scripts/s5-check\.sh deploy",
    r"git -C " + SAFE_PATH + r" rev-parse HEAD", r"git -C " + SAFE_PATH + r" rev-parse 'HEAD\^\{tree\}'", r"git -C " + SAFE_PATH + r" rev-parse --show-object-format",
    r"git -C " + SAFE_PATH + r" status --porcelain --untracked-files=no -- deploy scripts",
    r"git -C " + SAFE_PATH + r" status --porcelain --untracked-files=no -- scripts", r"git -C " + SAFE_PATH + r" rev-parse --show-toplevel",   # way C, pin A (F1): the tool tree's own cleanliness and top level
    r"podman inspect --format '\{\{\.State\.Running\}\} \{\{\.ImageName\}\}' [a-z][a-z0-9_.-]{0,62}")]
CONTAINER_NOT_PROVED = ("G3", "G4", "G6", "G10", "G11")
L2_STATUS = "MISSING"   # the run-time layer of the G12 matrix (the consumer-reaching one): until it exists, G12 is layer L1 only and no overall PASS is possible (vera B-Q3)
L1_ONLY = "L1 ONLY, layer L2 (the run-time matrix) is MISSING: not a release pass. "
NOTE_LINE = ("NOTE S5-10: an oracle run in a fixed environment, not a proof of definitions only; F5 is open (vera 5W-REVIEW); "
             "the real includes are read by hand at their final hashes and bound by hash (G12).")
PROOF_CLAIM = re.compile(r"(?i)\b(proves?|proven|guarantees?d?|ensures?|verif(?:y|ies|ied))\b.*\b(functions|definitions)\b.{0,12}\bonly\b")


class Interrupted(BaseException):
    pass


class MatrixNotRun(Exception):
    """The G12 matrix is not started because the oracle is not the one that step (g) bound: its code is not loaded."""


class Refused(Exception):
    def __init__(self, code, why):
        Exception.__init__(self, why)
        self.code, self.why = code, why


def now():
    return time.strftime(UTC, time.gmtime())


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for block in iter(lambda: fh.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


class Step:
    def __init__(self, base, rc, out, err):
        self.base, self.rc, self.out, self.err = base, rc, out, err


class Recorder:
    """The evidence directory: raw receipts, written as received; the verdict files; SHA256SUMS last."""

    def __init__(self, path):
        os.mkdir(path, 0o700)
        self.path = path
        self.seq = 0
        self.interrupted = False
        self.leaks = []

    def _put(self, name, data):
        with open(os.path.join(self.path, name), "xb") as fh:
            fh.write(data)
        os.chmod(os.path.join(self.path, name), 0o600)

    def next_base(self, check, step):
        self.seq += 1
        return "%02d-%s-%s" % (self.seq, check, step)

    def receipt(self, base, cmd, rc, out, err, t0, t1):
        self._put(base + ".cmd", cmd.encode() + b"\n")
        self._put(base + ".out", out)
        self._put(base + ".err", err)
        self._put(base + ".rc", (str(rc) + "\n").encode())
        self._put(base + ".t0", (t0 + "\n").encode())
        self._put(base + ".t1", (t1 + "\n").encode())

    def data_receipt(self, check, step, text):
        base = self.next_base(check, step)
        t = now()
        self.receipt(base, "(data supplied by the operator, not a command)", 0, text.encode(), b"", t, t)
        return base

    def traceback_receipt(self, check, text):
        self.seq += 1
        name = "%02d-%s.traceback" % (self.seq, check)
        self._put(name, text.encode())
        return name

    def finalize(self, files):
        for name, data in files.items():
            self._put(name, data)
        lines = []
        for d, _, fs in os.walk(self.path):
            for f in fs:
                rel = os.path.relpath(os.path.join(d, f), self.path)
                if rel != "SHA256SUMS":
                    lines.append((rel, sha256_file(os.path.join(d, f))))
        self._put("SHA256SUMS", "".join("%s  %s\n" % (h, rel) for rel, h in sorted(lines)).encode())


PTY_TRANSCRIPT_CAP = 32 * 1024 * 1024   # a finite bound on one terminal transcript (vera A-F1)


def leader_exited(pid):
    """True once the child has exited, WITHOUT reaping it: its pid and its process group stay reserved until proc.wait() (vera VR13-01)."""
    try:
        return os.waitid(os.P_PID, pid, os.WEXITED | os.WNOHANG | os.WNOWAIT) is not None
    except ChildProcessError:
        return True


class Ssh:
    GRACE = 5   # seconds each escalation step (terminate, then kill) and the final output drain may take (vera A-F2)
    OPTIONS = ["-o", "BatchMode=yes", "-o", "ConnectTimeout=15", "-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=4", "-o", "SendEnv=-*"]

    def __init__(self, target):
        self.target = target

    def argv(self, cmd, tty=False, stdin=False):
        return ["ssh"] + (["-tt"] if tty else []) + self.OPTIONS + [self.target, cmd]

    @classmethod
    def stop(cls, proc):
        """Terminate, then kill, the child (its whole session when it has one); never waits longer than two GRACE periods."""
        def send(sig):
            try:
                if getattr(proc, "gate_group", False):
                    os.killpg(proc.pid, sig)
                else:
                    proc.send_signal(sig)
            except (ProcessLookupError, PermissionError):
                pass
        send(signal.SIGTERM)
        end = time.monotonic() + cls.GRACE
        while not leader_exited(proc.pid) and time.monotonic() < end:
            time.sleep(0.02)
        if getattr(proc, "gate_group", False) or not leader_exited(proc.pid):
            send(signal.SIGKILL)   # the leader's exit says nothing about the other members of its session (vera VR13-01): the group is killed anyway
        try:
            proc.wait(cls.GRACE)
        except subprocess.TimeoutExpired:
            pass

    def run(self, cmd, timeout=120, stdin_path=None):
        stdin = open(stdin_path, "rb") if stdin_path else subprocess.DEVNULL
        proc = subprocess.Popen(self.argv(cmd, stdin=bool(stdin_path)), stdin=stdin, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        proc.gate_group = True
        try:
            out, err = proc.communicate(timeout=timeout)
        except subprocess.TimeoutExpired as exc:
            out, err = exc.stdout or b"", exc.stderr or b""
            self.stop(proc)
            try:
                out, err = proc.communicate(timeout=self.GRACE)
            except subprocess.TimeoutExpired as late:
                out, err = late.stdout or out, late.stderr or err   # a descendant still holds the pipes: keep what was read, stop waiting
                for pipe in (proc.stdout, proc.stderr):
                    try:
                        pipe.close()
                    except OSError:
                        pass
            return 124, out, err + b"gate_runner: timed out\n"
        except BaseException:
            self.stop(proc)
            raise
        finally:
            if stdin_path:
                stdin.close()
        return proc.returncode, out, err

    def run_pty(self, cmd, timeout, idle=0.4, consent=b"y\n"):
        """Run cmd on a pseudo-terminal, count and answer its questions. Returns (rc, transcript, events, timed_out)."""
        master, slave = pty.openpty()
        proc = subprocess.Popen(self.argv(cmd, tty=True), stdin=slave, stdout=slave, stderr=slave, close_fds=True, start_new_session=True)
        proc.gate_group = True
        os.close(slave)
        transcript, events, scan_from, timed_out = bytearray(), [], 0, False
        last_data = time.monotonic()
        deadline = last_data + timeout
        exited_at = None
        try:
            while True:
                tick = time.monotonic()
                if exited_at is None and leader_exited(proc.pid):
                    exited_at = tick
                if exited_at is None and tick > deadline:
                    timed_out = True
                    self.stop(proc)
                    break
                if exited_at is not None and tick - exited_at > self.GRACE:   # vera VR13-02: a descendant still writing after the command ended
                    transcript += b"\r\ngate_runner: output still arriving after the command ended; the drain was stopped\r\n"
                    timed_out = True
                    self.stop(proc)
                    break
                ready, _, _ = select.select([master], [], [], 0.1)
                if ready:
                    try:
                        data = os.read(master, 4096)
                    except OSError:
                        data = b""
                    if data:
                        transcript += data
                        last_data = time.monotonic()
                        if len(transcript) > PTY_TRANSCRIPT_CAP:
                            transcript += b"\r\ngate_runner: transcript limit reached\r\n"
                            timed_out = True
                            self.stop(proc)
                            break
                        continue
                    break
                if exited_at is not None:
                    break
                if tick - last_data >= idle:
                    text = ANSI.sub(b"", bytes(transcript[scan_from:])).decode("utf-8", "replace")
                    m = PROMPT.search(text[-300:])
                    if m:
                        password = "assword" in m.group(0)
                        os.write(master, b"\x04" if password else consent)
                        lines = [l for l in text.splitlines() if l.strip()]
                        events.append({"kind": "password" if password else "consent", "prompt": (lines[-1] if lines else "")[:120].strip(),
                                       "answer": "end of input" if password else consent.decode().strip(), "offset": len(transcript)})
                        scan_from = len(transcript)
                        last_data = time.monotonic()
            rc = proc.wait(timeout=10) if not timed_out else 124
        except BaseException as exc:
            exc.gate_partial = (bytes(transcript), list(events))   # the transcript so far survives an interruption (vera C-F1)
            self.stop(proc)
            raise
        finally:
            os.close(master)
        return rc, bytes(transcript), events, timed_out


class PodmanExec(Ssh):
    """The container transport: `podman exec` into ONE exact container name (never a pattern, never -a), the same allow-listed commands through sh -c."""

    def __init__(self, name):
        self.target = name

    def argv(self, cmd, tty=False, stdin=False):
        return ["podman", "exec"] + (["-it"] if tty else (["-i"] if stdin else [])) + [self.target, "sh", "-c", cmd]


class Ctx:
    def __init__(self, ssh, rec, args):
        self.ssh, self.rec, self.args = ssh, rec, args
        self.state = {}
        self.secrets = []   # exact values, in MEMORY ONLY: searched for by G8, never written, printed or recorded
        self.secrets_expected = 0    # how many secret steps were asked for (vera A-F4) ...
        self.secrets_collected = 0   # ... and how many returned a value: G8 cannot pass while these differ

    def step(self, check, step, key, stdin_path=None, **params):
        template = COMMANDS[key][0]
        cmd = template.format(**params) if params else template
        if not command_allowed(cmd):
            raise Refused(EX_USAGE, "command not on the allow-list: %r" % cmd)
        base = self.rec.next_base(check, step)
        t0 = now()
        try:
            rc, out, err = self.ssh.run(cmd, stdin_path=stdin_path)
        except BaseException:
            self.rec._put(base + ".cmd", cmd.encode() + b"\n")
            self.rec._put(base + ".rc", b"interrupted\n")
            self.rec._put(base + ".t0", (t0 + "\n").encode())
            raise
        shown = cmd + ("\n# stdin: %s sha256=%s" % (os.path.basename(stdin_path), sha256_file(stdin_path)) if stdin_path else "")
        self.rec.receipt(base, shown, rc, out, err, t0, now())
        return Step(base, rc, out, err)

    def secret_step(self, check, step, key, **params):
        """A step whose stdout IS a secret: the receipt keeps the command, the exit code, the size and a hash prefix, never the value; the value stays in memory for G8."""
        template = COMMANDS[key][0]
        cmd = template.format(**params) if params else template
        if not command_allowed(cmd):
            raise Refused(EX_USAGE, "command not on the allow-list: %r" % cmd)
        base = self.rec.next_base(check, step)
        t0 = now()
        self.secrets_expected += 1
        try:
            rc, out, err = self.ssh.run(cmd)
        except BaseException:
            self.rec._put(base + ".cmd", cmd.encode() + b"\n")
            self.rec._put(base + ".rc", b"interrupted\n")
            self.rec._put(base + ".t0", (t0 + "\n").encode())
            raise
        withheld = ("(output withheld: a secret value, %d bytes, sha256 prefix %s)\n" % (len(out), hashlib.sha256(out).hexdigest()[:12])).encode()
        self.rec.receipt(base, cmd + "\n# secret step: the value is held in memory only", rc, withheld, err, t0, now())
        if rc == 0 and out.strip():
            self.secrets.append(out.strip())
            self.secrets_collected += 1
        return Step(base, rc, out, err)

    def local_step(self, check, step, argv, cwd=None, timeout=900):
        """A command run on THIS host, never on the target: parse checks, git reads, hue's checker. The same allow-list rule as for the target."""
        cmd = shlex.join(argv)
        keygen = getattr(self.args, "ssh_keygen", None)
        if not (any(p.fullmatch(cmd) for p in LOCAL_ALLOWED) or (keygen and re.fullmatch(re.escape(keygen) + " -l -f " + SAFE_PATH, cmd))):
            raise Refused(EX_USAGE, "local command not on the allow-list: %r" % cmd)
        return self._run_local(check, step, argv, cmd, cwd, timeout)

    def way_c_step(self, argv, cwd, timeout=900):
        """Way C (pin A, F2: vera's return on eec6febc): the ONE command step (g) may run with --oracle-tree: exactly the command built from the two trees GIVEN (way_c_command), in the development
        tree. The generic allow-list takes no way-C form, and a path of the safe alphabet is not enough: another tool tree, another deploy directory, another script or another argument is refused before anything runs."""
        a = self.args
        if not (getattr(a, "oracle_tree", None) and getattr(a, "dev_tree", None)) or argv != way_c_command(a) or cwd != a.dev_tree:
            raise Refused(EX_USAGE, "local command not on the allow-list: %r" % shlex.join(argv))
        return self._run_local("G12", "oracle", argv, shlex.join(argv), cwd, timeout)

    def _run_local(self, check, step, argv, cmd, cwd, timeout):
        base = self.rec.next_base(check, step)
        t0 = now()
        env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "LC_ALL": "C", "HOME": os.environ.get("HOME", "/nonexistent")}
        proc = subprocess.Popen(argv, cwd=cwd, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)
        try:
            out, err = proc.communicate(timeout=timeout)
            rc = proc.returncode
        except subprocess.TimeoutExpired:
            Ssh.stop(proc)
            out, err, rc = b"", b"gate_runner: timed out\n", 124
        except BaseException:
            Ssh.stop(proc)
            self.rec._put(base + ".cmd", cmd.encode() + b"\n")
            self.rec._put(base + ".rc", b"interrupted\n")
            self.rec._put(base + ".t0", (t0 + "\n").encode())
            raise
        self.rec.receipt(base, cmd + ("  (cwd %s)" % cwd if cwd else "") + "\n# local command on this host", rc, out, err, t0, now())
        return Step(base, rc, out, err)

    def pty_step(self, check, step, key, consent=b"y\n", injection=False, **params):
        """A command run on a pseudo-terminal: the raw transcript is the .out receipt, the answers typed are the .answers receipt."""
        cmd = COMMANDS[key][0].format(**params) if params else COMMANDS[key][0]
        if not command_allowed(cmd) or (re.match(r"[A-Za-z_][A-Za-z0-9_]*=", cmd) and not injection) or "--yes" in cmd.split():
            raise Refused(EX_USAGE, "command not allowed on a terminal: %r" % cmd)
        base = self.rec.next_base(check, step)
        t0 = now()
        try:
            rc, transcript, events, timed_out = self.ssh.run_pty(cmd, self.args.install_timeout, consent=consent)
        except BaseException as exc:
            self.rec._put(base + ".cmd", cmd.encode() + b"\n")
            self.rec._put(base + ".rc", b"interrupted\n")
            self.rec._put(base + ".t0", (t0 + "\n").encode())
            partial = getattr(exc, "gate_partial", None)
            if partial is not None:
                self.rec._put(base + ".out", partial[0])
                self.rec._put(base + ".answers", (json.dumps(partial[1], indent=1) + "\n").encode())
            raise
        self.rec.receipt(base, cmd, rc, transcript, b"", t0, now())
        self.rec._put(base + ".answers", (json.dumps(events, indent=1) + "\n").encode())
        step_result = Step(base, rc, transcript, b"")
        step_result.events, step_result.timed_out = events, timed_out
        return step_result


def execute(checks, selected, rec, ctx, skip=None):
    """Run the checks in the given order.  A failure or an exception never hides the next check. `skip` maps a check id to the reason it cannot run here."""
    results = {}
    interrupted = False
    for cid, fn in checks:
        if cid not in selected:
            results[cid] = ("NOT RUN", "not selected (--only)", [])
            continue
        if skip and cid in skip:
            results[cid] = ("NOT RUN", skip[cid], [])
            continue
        if interrupted:
            results[cid] = ("NOT RUN", "interrupted before it started", [])
            continue
        try:
            verdict, note, receipts = fn(ctx)
            results[cid] = (verdict, note, list(receipts))
        except Interrupted:
            interrupted = True
            rec.interrupted = True
            results[cid] = ("NOT RUN", "interrupted while it ran", [])
        except Exception as exc:
            name = rec.traceback_receipt(cid, traceback.format_exc())
            results[cid] = ("FAIL", "exception %s: %s (%s)" % (type(exc).__name__, exc, name), [name])
    return results


# ---- the checks --------------------------------------------------------------------------------------------------------------

def check_g1(ctx):
    a = ctx.args
    s = lambda name, key: ctx.step("G1", name, key)
    osr, pk, ports = s("os_release", "g1_os_release"), s("rpm_qa", "g1_rpm_qa"), s("ports", "g1_ports")
    podman, cargo, rustc = s("cmd_podman", "g1_cmd_podman"), s("cmd_cargo", "g1_cmd_cargo"), s("cmd_rustc", "g1_cmd_rustc")
    dirs, units = s("podman_dirs", "g1_podman_dirs"), s("user_units", "g1_user_units")
    keys, rel, gpg = s("rocky_keys", "g1_rocky_keys"), s("rocky_pkgs", "g1_rocky_pkgs"), s("gpg_pubkeys", "g1_gpg_pubkeys")
    ident = ctx.rec.data_receipt("G1", "identity", "image_name=%s\nimage_version=%s\nimage_checksum=%s\nimage_source=%s\n" % (
        a.image_name, a.image_version, a.image_checksum, a.image_source))
    fails = []
    partial = a.scenario == "partial"
    # vera A-F3: absence is only believed from a probe's NORMAL answer; ssh 255, a timeout, command-not-found are not evidence of anything
    for step_name, step_, normal in (("os_release", osr, (0,)), ("rpm_qa", pk, (0,)), ("ports", ports, (0,)), ("cmd_podman", podman, (0,) if partial else (0, 1)),
                                     ("cmd_cargo", cargo, (0, 1)), ("cmd_rustc", rustc, (0, 1)), ("podman_dirs", dirs, (0,)), ("user_units", units, (0,)),
                                     ("rocky_keys", keys, (0,)), ("rocky_pkgs", rel, (0,)), ("gpg_pubkeys", gpg, (0,))):
        if step_.rc not in normal:
            fails.append("%s probe failed abnormally (rc %d, a normal answer is %s): that is not evidence either way" % (step_name, step_.rc, " or ".join(str(n) for n in normal)))
    if not pk.out.strip():
        fails.append("the package list is empty")
    if partial and podman.rc != 0:
        fails.append("scenario partial: podman is not present")
    if not partial and (podman.rc == 0 or podman.out.strip()):
        fails.append("podman is present")
    if cargo.rc == 0 or cargo.out.strip():
        fails.append("cargo is present")
    if rustc.rc == 0 or rustc.out.strip():
        fails.append("rustc is present")
    dir_lines = dirs.out.decode("utf-8", "replace").splitlines()   # vera VR13-03: 'present P', 'absent P' (verified) or 'unreadable P', one line per path
    if len(dir_lines) != 2 or not all(re.fullmatch(r"(present|absent|unreadable) /\S+", l) for l in dir_lines):
        fails.append("podman_dirs probe gave no verified answer for both container directories (%d line(s))" % len(dir_lines))
    else:
        for l in dir_lines:
            if l.startswith("unreadable"):
                fails.append("podman_dirs probe could not read %s: that is not evidence of absence" % l.split(" ", 1)[1])
            elif l.startswith("present") and not partial:
                fails.append("container storage or configuration already exists (%s)" % l.split(" ", 1)[1])
    unit_state = parse_state(units.out.decode("utf-8", "replace")).get("user_units")
    if unit_state is None or unit_state[0] not in ("0", "absent"):
        fails.append("user_units probe gave no verified answer (%s): that is not evidence of absence" % ("no sentinel" if unit_state is None else unit_state[0]))
    elif any("cortex" in l for l in unit_state[1]):
        fails.append("a cortex user unit already exists")
    if not re.search(rb"^[0-9a-f]{64}  /etc/pki/rpm-gpg/RPM-GPG-KEY-Rocky-", keys.out, re.M):
        fails.append("no Rocky certificate file recorded")
    if not a.image_name.strip():
        fails.append("the image name is missing")
    if not a.image_version.strip():
        fails.append("the image version is missing")
    if not re.fullmatch(r"[0-9a-f]{64}", a.image_checksum):
        fails.append("the image checksum is not a 64-digit sha256")
    if not a.image_source.strip():
        fails.append("the image source is missing")
    receipts = [x.base for x in (osr, pk, ports, podman, cargo, rustc, dirs, units, keys, rel, gpg)] + [ident]
    if fails:
        return "FAIL", "; ".join(fails), receipts
    return "PASS", "clean start recorded: %d packages, identity and Rocky certificate files saved" % len(pk.out.splitlines()), receipts


def compose_scan(text):
    """{service: container_name} of a compose file, by a plain scan (no YAML library). Names come from the release, never from this file."""
    found, in_services, svc_indent, current = {}, False, None, None
    for line in text.splitlines():
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        indent = len(line) - len(line.lstrip(" "))
        stripped = line.strip()
        if indent == 0:
            in_services, current, svc_indent = stripped == "services:", None, None
            continue
        if not in_services:
            continue
        if svc_indent is None:
            svc_indent = indent
        if indent == svc_indent and stripped.endswith(":") and " " not in stripped[:-1]:
            current = stripped[:-1]
            found.setdefault(current, None)
            continue
        if current and indent > svc_indent and found.get(current) is None:
            m = re.fullmatch(r"container_name:\s*(\S+)", stripped)
            if m:
                found[current] = m.group(1).strip("'\"")
    return found


def compose_container(text, service):
    return compose_scan(text).get(service)


def check_g3(ctx):
    a = ctx.args
    h = ctx.step("G3", "health", "g3_health", port=a.port)
    c = ctx.step("G3", "compose", "g3_compose", home=a.home)
    e = ctx.step("G3", "stack_env", "g3_stack_env", home=a.home)
    fails, receipts = [], [h.base, c.base, e.base]
    try:
        ready = json.loads(h.out.decode("utf-8")).get("ready") is True
    except (ValueError, AttributeError):
        ready = False
    if h.rc != 0 or not ready:
        fails.append("health is not ready (rc %d)" % h.rc)
    server_image = None
    if e.rc == 0:
        m = re.search(r"^CORTEX_SERVER_IMAGE=(\S+)$", e.out.decode("utf-8", "replace"), re.M)
        server_image = m.group(1) if m else None
    if server_image is None:
        fails.append("stack.env is unreadable or names no CORTEX_SERVER_IMAGE")
    name = compose_container(c.out.decode("utf-8", "replace"), "migrate") if c.rc == 0 else None
    if name is None or not CONTAINER_NAME.fullmatch(name):
        fails.append("the release's compose file names no usable migrate container")
    else:
        m = ctx.step("G3", "migrate", "g3_migrate", container=name)
        receipts.append(m.base)
        parts = m.out.split()
        if m.rc != 0 or parts[:1] != [b"0"]:
            fails.append("the migrate container did not exit 0")
        elif server_image is not None and (len(parts) < 2 or parts[1].decode("utf-8", "replace") != server_image):
            fails.append("the migrate container ran from a different image than the release's stack.env names")
    if fails:
        return "FAIL", "; ".join(fails), receipts
    return "PASS", "ready true; migrate exited 0 from the release image", receipts


def list_artefacts(path):
    """The release files to upload: a flat directory of plain files with safe names, install.sh among them."""
    if not os.path.isdir(path):
        return None, "the artefact directory %s does not exist" % path
    names = sorted(os.listdir(path))
    bad = [n for n in names if not ARTEFACT_NAME.fullmatch(n) or os.path.islink(os.path.join(path, n)) or not os.path.isfile(os.path.join(path, n))]
    if bad:
        return None, "unsafe or unexpected entries in the artefact directory: %s" % ", ".join(repr(b) for b in bad[:5])
    if "install.sh" not in names:
        return None, "install.sh is not among the release artefacts"
    return names, ""


def ensure_upload(ctx, check):
    """Upload and verify the release artefacts ONCE (G10, G7 and G2 share them). Returns (ok, why, receipts)."""
    if "upload" in ctx.state:
        return ctx.state["upload"]
    a = ctx.args
    names, why = list_artefacts(a.artefacts)
    if names is None:
        ctx.state["upload"] = (False, why, [])
        return ctx.state["upload"]
    local = {n: sha256_file(os.path.join(a.artefacts, n)) for n in names}
    receipts = []
    mk = ctx.step(check, "mkdir", "g2_mkdir")
    receipts.append(mk.base)
    if mk.rc != 0:
        ctx.state["upload"] = (False, "the artefact directory could not be created on the target (it exists: the machine is not clean?)", receipts)
        return ctx.state["upload"]
    for n in names:
        put = ctx.step(check, "put_" + n, "g2_put", stdin_path=os.path.join(a.artefacts, n), name=n)
        receipts.append(put.base)
        if put.rc != 0:
            ctx.state["upload"] = (False, "the upload of %s failed (rc %d)" % (n, put.rc), receipts)
            return ctx.state["upload"]
    sums = ctx.step(check, "verify", "g2_sums")
    receipts.append(sums.base)
    remote = {}
    for line in sums.out.decode("utf-8", "replace").splitlines():
        m = re.fullmatch(r"([0-9a-f]{64})  (.+)", line)
        if m:
            remote[os.path.basename(m.group(2))] = m.group(1)
    wrong = sorted(n for n in names if remote.get(n) != local[n]) + sorted(n for n in remote if n not in local)
    if sums.rc != 0 or wrong:
        ctx.state["upload"] = (False, "the upload does not match the local files: %s (the install was not run)" % ", ".join(wrong or ["verify failed"]), receipts)
        return ctx.state["upload"]
    ctx.state["upload"] = (True, "", receipts)
    return ctx.state["upload"]


def check_g2(ctx):
    a = ctx.args
    ok, why, receipts = ensure_upload(ctx, "G2")
    if not ok:
        return "FAIL", why, receipts
    inst = ctx.pty_step("G2", "install", "g2_install", project=a.project, lead=a.lead)
    receipts = receipts + [inst.base]
    ctx.state["g2_events"] = inst.events
    if inst.timed_out:
        return "FAIL", "the install timed out after %d s (a prompt the runner does not know?)" % a.install_timeout, receipts
    if any(e["kind"] == "password" for e in inst.events):
        return "FAIL", "the install asked for a password (sudo needs one): the runner answered with end of input", receipts
    if inst.rc != 0:
        return "FAIL", "the install exited %d" % inst.rc, receipts
    return "PASS", "install exited 0 after %d question(s)" % len(inst.events), receipts


def check_g9(ctx):
    events = ctx.state.get("g2_events")
    if events is None:
        return "NOT RUN", "G2 did not run the install", []
    n = len(events)
    quoted = "; ".join(e["prompt"] for e in events)
    word = "question" if n == 1 else "questions"
    if any(e["kind"] == "password" for e in events):
        return "FAIL", "%d %s, one of them a password prompt: %s" % (n, word, quoted), []
    if n > 1:
        return "FAIL", "%d %s (target: at most one): %s" % (n, word, quoted), []
    return "PASS", "%d %s%s" % (n, word, (": " + quoted) if quoted else ""), []


COMPONENTS = ("packages", "containers", "files", "timers", "rows")


WHOAMI_FIELDS = ("installation", "project", "worker", "name", "kind", "role", "team", "job_roles", "token_id", "expires_at")


def check_g4(ctx):
    """G4 on the HTTP routes (kai c524dc80, cox c3181bb5): the lead's token file, a second token issued for the lead's own worker, whoami with it. No token is printed:
    headers go through files (mode 0600) and the issued token is read into memory only, for G8."""
    a = ctx.args
    if ctx.state.get("g2_events") is None:
        return "NOT RUN", "G2 did not run the install; there is no stack to sign in to", []
    receipts = []
    try:
        lm = ctx.step("G4", "lead_token_mode", "g4_mode", home=a.home, name=a.lead)
        receipts.append(lm.base)
        if lm.out.strip() != b"600":
            return "FAIL", "the lead's token file is not mode 600", receipts
        hl = ctx.step("G4", "header_lead", "g4_hdr_lead", home=a.home, lead=a.lead)
        lw = ctx.step("G4", "lead_whoami", "g4_lead_whoami", port=a.port)
        lf = ctx.step("G4", "lead_fields", "g4_lead_fields")
        receipts += [hl.base, lw.base, lf.base]
        if hl.rc != 0 or lw.out.strip() != b"200":
            return "FAIL", "whoami with the lead's token answered %s (rc %d)" % (lw.out.strip()[:20].decode("utf-8", "replace") or "nothing", lw.rc), receipts
        fields = lf.out.decode("utf-8", "replace").split()
        if len(fields) != 2 or not re.fullmatch(UUID_RX, fields[0]):
            return "FAIL", "whoami with the lead's token gave no worker UUID, so no token can be issued", receipts
        worker_id, lead_token_id = fields
        lead_secret = ctx.secret_step("G4", "secret_lead", "g4_secret_lead", home=a.home, lead=a.lead)
        receipts.append(lead_secret.base)
        if lead_secret.rc != 0 or not lead_secret.out.strip():
            return "FAIL", "the lead's token could not be collected (rc %d): the leak scan would not cover its value" % lead_secret.rc, receipts
        iss = ctx.step("G4", "issue", "g4_issue", idem=str(uuid.uuid4()), port=a.port, worker_id=worker_id)
        receipts.append(iss.base)
        if iss.out.strip() != b"201":
            return "FAIL", "issuing a worker token answered %s (rc %d)" % (iss.out.strip()[:20].decode("utf-8", "replace") or "nothing", iss.rc), receipts
        ids = ctx.step("G4", "issued_ids", "g4_issued_ids")
        receipts.append(ids.base)
        parts = ids.out.decode("utf-8", "replace").split()
        if len(parts) != 3 or parts[1] != "True" or not parts[0]:
            return "FAIL", "the issue response lacks token, token_id or expires_at", receipts
        issued_token_id = parts[0]
        issued_secret = ctx.secret_step("G4", "secret_issued", "g4_secret_issued")
        if issued_secret.rc != 0 or not issued_secret.out.strip():
            receipts.append(issued_secret.base)
            return "FAIL", "the issued token could not be collected (rc %d): the leak scan would not cover its value" % issued_secret.rc, receipts
        hw = ctx.step("G4", "header_worker", "g4_hdr_worker")
        ww = ctx.step("G4", "worker_whoami", "g4_worker_whoami", port=a.port)
        receipts += [issued_secret.base, hw.base, ww.base]
        if hw.rc != 0:
            return "FAIL", "the worker's header file could not be made (rc %d)" % hw.rc, receipts
        body, _, code = ww.out.decode("utf-8", "replace").rpartition("\n")
        if code.strip() != "200":
            return "FAIL", "whoami with the issued token answered %s" % (code.strip()[:20] or "nothing"), receipts
        try:
            who = json.loads(body)
        except ValueError:
            return "FAIL", "whoami with the issued token did not return JSON", receipts
        missing = [f for f in WHOAMI_FIELDS if f not in who]
        if missing:
            return "FAIL", "whoami lacks the field(s) %s" % ", ".join(missing), receipts
        if who["worker"] != worker_id:
            return "FAIL", "whoami names another worker than the one the token was issued for", receipts
        if who["token_id"] != issued_token_id or who["token_id"] == lead_token_id:
            return "FAIL", "whoami's token_id is not the issued token's (the lead's token may have been used)", receipts
        return "PASS", "a second token was issued for the lead's worker (HTTP 201) and whoami with it names that worker and that token_id (200); no token was printed", receipts
    finally:
        cl = ctx.step("G4", "cleanup", "g4_cleanup")
        receipts.append(cl.base)


def fingerprint(ctx, tag, db):
    home = ctx.args.home
    steps = {"packages": ctx.step("G5", "fp_%s_packages" % tag, "g1_rpm_qa"),
             "containers": ctx.step("G5", "fp_%s_containers" % tag, "g5_containers"),
             "files": ctx.step("G5", "fp_%s_files" % tag, "g5_files", home=home),
             "timers": ctx.step("G5", "fp_%s_timers" % tag, "g5_timers"),
             "rows": ctx.step("G5", "fp_%s_rows" % tag, "g5_rows", container=db)}
    texts = {k: sorted(v.out.decode("utf-8", "replace").splitlines()) for k, v in steps.items()}
    return texts, [v.base for v in steps.values()], {k: v.rc for k, v in steps.items() if v.rc != 0}


def check_g5(ctx):
    a = ctx.args
    if ctx.state.get("g2_events") is None:
        return "NOT RUN", "G2 did not run the install; there is nothing to run twice", []
    c = ctx.step("G5", "compose", "g3_compose", home=a.home)
    receipts = [c.base]
    db = compose_container(c.out.decode("utf-8", "replace"), "db") if c.rc == 0 else None
    if db is None or not CONTAINER_NAME.fullmatch(db):
        return "FAIL", "the release's compose file names no usable db container", receipts
    before, r1, bad1 = fingerprint(ctx, "before", db)
    receipts += r1
    rerun = ctx.pty_step("G5", "rerun", "g2_install", project=a.project, lead=a.lead)
    receipts.append(rerun.base)
    after, r2, bad2 = fingerprint(ctx, "after", db)
    receipts += r2
    fails = []
    if bad1 or bad2:
        fails.append("a fingerprint command failed: %s" % ", ".join(sorted(set(bad1) | set(bad2))))
    if rerun.timed_out:
        fails.append("the second run timed out")
    elif rerun.rc != 0:
        fails.append("the second run exited %d" % rerun.rc)
    if rerun.events:
        fails.append("the second run asked %d question(s)" % len(rerun.events))
    for comp in COMPONENTS:
        if before[comp] != after[comp]:
            diff = sorted(set(before[comp]) ^ set(after[comp]))
            example = diff[0].split("  ", 1)[-1][:80] if diff else ""
            fails.append("%s changed (%s)" % (comp, example))
    if fails:
        return "FAIL", "; ".join(fails), receipts
    return "PASS", "second run exited 0, asked 0 questions, the state fingerprint is equal (%d components)" % len(COMPONENTS), receipts


SECRET_CLASSES = (("private_key", re.compile(rb"-----BEGIN [A-Z ]*PRIVATE KEY-----")),
                  ("bearer", re.compile(rb"(?i)\bbearer\s+[A-Za-z0-9._~+/=-]{16,}")),
                  ("credential", re.compile(rb"(?i)\b(?:password|passwd|secret|token|api[_-]?key)\b\s*[=:]\s*[^\s'\"<>]{8,}")),
                  ("hex64", re.compile(rb"(?<![0-9A-Fa-f])[0-9a-f]{64}(?![0-9A-Fa-f])")))
CANARY_CONTROL = {"private_key": b"-----BEGIN CANARY PRIVATE KEY-----", "bearer": b"Authorization: Bearer CANARY0123456789abcdef",
                  "credential": b"password=CANARYpassword123", "hex64": b"ca" * 32}
CANARY_SECRET = b"CANARY-KNOWN-SECRET-0123456789"
HASH_STEPS = ("verify", "rocky_keys", "identity")


def scan_bytes(data, hashes_allowed=False):
    """[(class, line number)] of secret-shaped strings. A 64-digit hex string right after 'sha256:' is a digest, not a secret."""
    hits = []
    for number, line in enumerate(data.splitlines(), 1):
        line = ANSI.sub(b"", line)
        for cls, pattern in SECRET_CLASSES:
            for m in pattern.finditer(line):
                if cls == "hex64" and (hashes_allowed or line[max(0, m.start() - 7):m.start()] == b"sha256:"):
                    continue
                hits.append((cls, number))
    return hits


def scan_known(data, secrets):
    """[(class, line number)] where the exact value of a held secret appears."""
    hits = []
    for number, line in enumerate(data.splitlines(), 1):
        if any(sec and sec in line for sec in secrets):
            hits.append(("known_secret", number))
    return hits


def check_g8(ctx):
    a = ctx.args
    if ctx.state.get("g2_events") is None:
        return "NOT RUN", "G2 did not run the install; there is no installer transcript to scan", []
    found = {cls for sample in CANARY_CONTROL.values() for cls, _ in scan_bytes(sample)}
    missing = sorted(set(CANARY_CONTROL) - found)
    if not scan_known(b"leak " + CANARY_SECRET + b" here", [CANARY_SECRET]):
        missing.append("known_secret")
    if missing:
        return "FAIL", "the scanner's own control did not find the canary class(es): %s" % ", ".join(missing), []
    if ctx.secrets_collected < ctx.secrets_expected:
        return "FAIL", "the known-secret inventory is incomplete: %d of %d required token value(s) were collected, so the exact-value scan cannot be trusted" % (ctx.secrets_collected, ctx.secrets_expected), []
    c = ctx.step("G8", "compose", "g3_compose", home=a.home)
    receipts = [c.base]
    names = [n for n in compose_scan(c.out.decode("utf-8", "replace")).values() if n] if c.rc == 0 else []
    names = [n for n in names if CONTAINER_NAME.fullmatch(n)]
    if not names:
        return "FAIL", "the release's compose file names no usable container to read logs from", receipts
    read, unread = 0, []
    for n in names:
        lg = ctx.step("G8", "logs_" + n, "g8_logs", container=n)
        receipts.append(lg.base)
        if lg.rc == 0:
            read += 1
        else:
            unread.append(n)
    if read == 0:
        return "FAIL", "no container log was read (podman logs failed for every container)", receipts
    if unread:
        return "FAIL", "the leak scan is incomplete: %d of %d container log(s) could not be read (%s)" % (len(unread), len(names), ", ".join(unread)), receipts
    hits, scanned, size = [], 0, 0
    for fname in sorted(os.listdir(ctx.rec.path)):
        if not fname.endswith((".out", ".err")):
            continue
        step = fname.split("-", 2)[2].rsplit(".", 1)[0]
        with open(os.path.join(ctx.rec.path, fname), "rb") as fh:
            data = fh.read()
        scanned += 1
        size += len(data)
        for cls, number in scan_bytes(data, hashes_allowed=step in HASH_STEPS or step.startswith("fp_")) + scan_known(data, ctx.secrets):
            hits.append((fname, cls, number))
    ctx.rec.leaks = hits
    if hits:
        shown = "; ".join("%s %s:%d" % (cls, fname, number) for fname, cls, number in hits[:8])
        return "FAIL", "%d hit(s), values not shown: %s" % (len(hits), shown), receipts
    return "PASS", "no secret-shaped string in %d receipts (%d bytes) and %d of %d container logs and %d held secret value(s)%s; control 5/5" % (scanned, size, read, len(names), len(ctx.secrets),
                                                                                    "" if ctx.secrets else " (no token value was collected in this run: the exact-value scan covered nothing)"), receipts


def g11_phase(ctx, tag):
    a = ctx.args
    timers = a.timers.split(",")
    joined = " ".join(timers)
    en = ctx.step("G11", "enabled_" + tag, "g11_enabled", timers=joined)
    ac = ctx.step("G11", "active_" + tag, "g11_active", timers=joined)
    ln = ctx.step("G11", "linger_" + tag, "g11_linger")
    ls = ctx.step("G11", "timers_" + tag, "g11_list")
    fails = []
    if en.rc != 0 or en.out.decode("utf-8", "replace").split() != ["enabled"] * len(timers):
        fails.append("not every timer is enabled")
    if ac.rc != 0 or ac.out.decode("utf-8", "replace").split() != ["active"] * len(timers):
        fails.append("not every timer is active")
    props = dict(l.split("=", 1) for l in ln.out.decode("utf-8", "replace").splitlines() if "=" in l)
    if props.get("Linger") != "yes":
        fails.append("linger is not yes")
    if props.get("State") not in ("active", "lingering"):
        fails.append("the user manager state is %r" % props.get("State"))
    text = ls.out.decode("utf-8", "replace")
    for t in timers:
        if t not in text:
            fails.append("%s is not in list-timers" % t)
    return fails, [x.base for x in (en, ac, ln, ls)]


def check_g11(ctx):
    a = ctx.args
    if ctx.state.get("g2_events") is None:
        return "NOT RUN", "G2 did not run the install; there are no backup timers to look at", []
    before, r1 = g11_phase(ctx, "before")
    sess = ctx.step("G11", "sessions", "g11_sessions")
    receipts = r1 + [sess.base]
    if before:
        return "FAIL", "before the logout: " + "; ".join(before), receipts
    others = len([l for l in sess.out.decode("utf-8", "replace").splitlines() if l.strip()]) - 1
    if others > 0:
        return "FAIL", "%d other login session(s) are open on the target: the logout would not be tested" % others, receipts
    time.sleep(a.logout_wait)
    after, r2 = g11_phase(ctx, "after")
    receipts += r2
    if after:
        return "FAIL", "after the logout: " + "; ".join(after), receipts
    return "PASS", "timers enabled and active, linger yes, before and after a %d s logout" % a.logout_wait, receipts


STATE_KEYS = ("packages", "state")


STATE_PARTS = {"trial_home": ("0", "absent"), "user_units": ("0", "absent"), "podman_ps": ("0", "127"), "ports": ("0",), "home": ("0",)}   # part -> its normal answers: an exit code, or VERIFIED absence (podman is ABSENT, 127, on the clean machine)
STATE_SENTINEL = re.compile(r"@@([a-z_]+) (\d+|absent|unreadable|notdir)")


def parse_state(text):
    """{part: (answer, [lines])} of a probe answer that ends every part with a sentinel line '@@part rc' or '@@part absent|unreadable|notdir'."""
    parts, current = {}, []
    for line in text.splitlines():
        m = STATE_SENTINEL.fullmatch(line)
        if m:
            parts[m.group(1)] = (m.group(2), current)
            current = []
        else:
            current.append(line)
    return parts


def trial_state(ctx, check, tag):
    """The light state fingerprint of a machine that has no release yet: the packages, the trial home, the user units, the containers, the ports.
    Returns (fingerprint with an 'errors' list, receipts). A part that could not be read is an error: two equal failed reads are not 'nothing changed' (vera B-F3)."""
    pk = ctx.step(check, "fp_%s_packages" % tag, "g1_rpm_qa")
    st = ctx.step(check, "fp_%s_state" % tag, "trial_state")
    errors = []
    if pk.rc != 0 or not pk.out.strip():
        errors.append("the package list could not be read (rc %d)" % pk.rc)
    if st.rc != 0:   # vera VR15-01: Ssh.run keeps stdout when it returns a timeout (124) or a lost transport (255): complete output does not make the probe succeed
        errors.append("the state could not be read: the probe exited %d" % st.rc)
    parts = parse_state(st.out.decode("utf-8", "replace"))
    lines = []
    for name, normal in STATE_PARTS.items():
        if name not in parts:
            errors.append("the state could not be read: the %s part is missing" % name)
            continue
        rc, body = parts[name]
        if rc not in normal:
            errors.append("the state could not be read: %s answered %s" % (name, rc))
        lines.append("%s:@@%s" % (name, rc))   # the answer itself is state: absent and present-empty differ (vera VR14-02)
        lines += ["%s:%s" % (name, l) for l in body if not (name == "home" and RUNNER_HOME_NAMES.fullmatch(l))]
    return {"packages": sorted(pk.out.decode("utf-8", "replace").splitlines()), "state": sorted(lines), "errors": errors}, [pk.base, st.base]


def state_changes(before, after):
    out = []
    for key in STATE_KEYS:
        if before[key] != after[key]:
            diff = sorted(set(before[key]) ^ set(after[key]))
            out.append("%s changed (%s)" % (key, diff[0][:60] if diff else ""))
    return out


def judge_run(run, codes, label):
    """The failures of one declined installer run: the exit code, the plain cause and fix."""
    text = ANSI.sub(b"", run.out).decode("utf-8", "replace")
    fails = []
    if run.timed_out:
        fails.append("%s timed out" % label)
    elif run.rc == 0:
        fails.append("%s: the installer exited 0 (it went through)" % label)
    elif run.rc not in codes:
        if run.rc == 2 and run.events and 4 in codes:
            fails.append("%s: the trust step was not reached: the prerequisites come first (exit 2)" % label)
        else:
            fails.append("%s: exit %d, wanted %s" % (label, run.rc, " or ".join(str(c) for c in sorted(codes))))
    if run.rc != 0 and "problem:" not in text:
        fails.append("%s: no plain cause (problem: line)" % label)
    if run.rc != 0 and "fix:" not in text:
        fails.append("%s: no fix line" % label)
    return fails


def check_g10(ctx):
    a = ctx.args
    ok, why, receipts = ensure_upload(ctx, "G10")
    if not ok:
        return "FAIL", why, receipts
    names = sorted(os.listdir(a.artefacts))
    tarball = next((n for n in names if re.fullmatch(r"cortex-.*\.tar\.gz", n)), None)
    if tarball is None or "SHA256SUMS.sig" not in names:
        return "FAIL", "the artefacts have no release tarball or no SHA256SUMS.sig to tamper with", receipts
    fails = []
    try:
        for kind, flipped in (("checksum", tarball), ("signature", "SHA256SUMS.sig")):
            cp = ctx.step("G10", "copy_" + kind, "g10_copy_" + kind)
            fl = ctx.step("G10", "flip_" + kind, "g10_flip_" + kind, name=flipped)
            receipts += [cp.base, fl.base]
            if cp.rc != 0 or fl.rc != 0:
                fails.append("%s: the tampered copy could not be made (rc %d, %d)" % (kind, cp.rc, fl.rc))
                continue
            before, r1 = trial_state(ctx, "G10", "before_" + kind)
            run = ctx.pty_step("G10", "run_" + kind, "g10_run_" + kind, consent=b"n\n", injection=True, project=a.project, lead=a.lead)
            after, r2 = trial_state(ctx, "G10", "after_" + kind)
            receipts += r1 + [run.base] + r2
            fails += judge_run(run, {4}, "tampered " + kind)
            fails += ["tampered %s: %s" % (kind, c) for c in state_changes(before, after)]
            fails += ["tampered %s: %s" % (kind, e) for e in before["errors"] + after["errors"]]
    finally:
        cl = ctx.step("G10", "cleanup", "g10_cleanup")
        receipts.append(cl.base)
    if fails:
        return "FAIL", "; ".join(fails), receipts
    return "PASS", "a tampered checksum and a tampered signature each exit 4 with a cause and a fix, and change nothing", receipts


INJECTIONS = (
    ("no_network", (), "g7_run_no_network", (), {3, 5}),
    ("port_in_use", ("g7_listen",), "g7_run_port", ("g7_unlisten",), {2}),
    ("low_disk", (), "g7_run_low_disk", (), {2}),
    ("podman_too_old", ("g7_shim",), "g7_run_old_podman", ("g7_unshim",), {2}),
    ("no_linger", ("g7_linger_off",), "g7_run_no_linger", (), {2, 3}),
    ("unsupported_os", ("g7_fakeroot",), "g7_run_bad_os", ("g7_unfakeroot",), {2}),
)
DIE_CALL = re.compile(r"""(?:^|[;&|{(]\s*|\bthen\s+|\|\|\s*)(?:die|fail\s+[^\s"']+)\s+(?P<q>["'])(?P<msg>.*?)(?P=q)""")
FIX_CUE = re.compile(r"\b(run|use|install|update|check|see|try|fix|enable|set|choose|pass|fetch|obtain|open|remove|start|stop|free|retry|again)\b", re.I)


def g7_static(path):
    """Every die or fail message of the shipped install.sh: a catalogue id on the line (msg ...) or a fix cue in the words, else it is listed."""
    try:
        text = open(path, encoding="utf-8", errors="replace").read()
    except OSError:
        return None, ["install.sh is not readable at %s" % path]
    unclassified = []
    for number, line in enumerate(text.splitlines(), 1):
        stripped = line.strip()
        if stripped.startswith("#"):
            continue
        for m in DIE_CALL.finditer(line):
            msg = m.group("msg")
            if re.search(r"(^|[;&|{(]\s*)msg\s+\S+", line[:m.start()] + " ") or "$" in msg or FIX_CUE.search(msg):
                continue
            unclassified.append("line %d: %s" % (number, msg[:60]))
    return True, unclassified


def check_g7(ctx):
    a = ctx.args
    ok, why, receipts = ensure_upload(ctx, "G7")
    if not ok:
        return "FAIL", why, receipts
    fails, results = [], []
    in_container = a.container is not None
    linger_was_on = False
    if not in_container:
        got = ctx.step("G7", "linger_before", "g7_linger_get")
        receipts.append(got.base)
        linger_was_on = b"Linger=yes" in got.out
    for name, setup, run_key, teardown, codes in INJECTIONS:
        if in_container and name == "no_linger":
            results.append("no_linger not run in a container (no lingering without a real user manager)")
            continue
        done = []
        try:
            for key in setup:
                st = ctx.step("G7", "%s_%s" % (name, key), key)
                receipts.append(st.base)
                done.append(key)
            if name == "port_in_use":
                time.sleep(1)
                ports = ctx.step("G7", "port_in_use_listening", "g1_ports")
                receipts.append(ports.base)
                if b":8604" not in ports.out:
                    fails.append("port_in_use: the injection did not take (nothing listens on 8604)")
                    continue
            before, r1 = trial_state(ctx, "G7", "before_" + name)
            run = ctx.pty_step("G7", "run_" + name, run_key, consent=b"n\n", injection=True, project=a.project, lead=a.lead)
            after, r2 = trial_state(ctx, "G7", "after_" + name)
            receipts += r1 + [run.base] + r2
            problems = judge_run(run, codes, name) + ["%s: %s" % (name, c) for c in state_changes(before, after)] + ["%s: %s" % (name, e) for e in before["errors"] + after["errors"]]
            if problems:
                fails += problems
            else:
                results.append("%s exit %d" % (name, run.rc))
        finally:
            for key in teardown:
                td = ctx.step("G7", "%s_%s" % (name, key), key)
                receipts.append(td.base)
            if name == "no_linger" and linger_was_on:
                td = ctx.step("G7", "linger_restore", "g7_linger_on")
                receipts.append(td.base)
    ok_static, listed = g7_static(os.path.join(a.artefacts, "install.sh"))
    if ok_static is None:
        fails.append(listed[0])
    elif listed:
        fails.append("%d die or fail message(s) with no cause or fix: %s" % (len(listed), "; ".join(listed[:5])))
    if fails:
        return "FAIL", "; ".join(fails), receipts
    return "PASS", "%d injections (%s), each with a cause and a fix and no change; the static rule found no bare message" % (len([r for r in results if " exit " in r]), ", ".join(results)), receipts


# The two marker forms marv's builder (release_manifest.py join_installer) accepts, exactly: three spaces before # @include, LF at the end of the line.
MARKER_FORMS = (re.compile(rb'\. "\$here/([A-Za-z0-9._-]+)"   # @include\n'),
                re.compile(rb'\[ ! -f "\$here/([A-Za-z0-9._-]+)" \] \|\| \. "\$here/\1"   # @include\n'))


# ---- the binding record: schema r3 (mvp/G12-BINDING-RECORD-SCHEMA-nix-r3.md), UNFROZEN until a REAL record has been emitted (cox 08:09Z, vera) ----------
BINDING_SCHEMA_STATUS = "UNFROZEN"
RECORD_KEYS = ("schema", "object_format", "commit", "tree", "includes", "splice", "joined_sha256")
KNOWN_INCLUDES = ("install-prereq.sh", "install-fetch.sh")   # the include names marv's builder knows
SPLICE_SENTENCE = "each '# @include' marker line, newline included, is replaced by the exact bytes of its include; nothing is added"
# schema 2 (G12-BINDING-RECORD-SCHEMA-nix-r4.md, accepted for implementation by vera and cox, UNFROZEN): r3's seven keys plus anchor, and the final bytes are T1 then T2
SPLICE_SENTENCE_2 = ("each '# @include' marker line, newline included, is replaced by the exact bytes of its include; then each of the three empty anchor statements "
                     "CORTEX_ANCHOR_PRINCIPAL='', CORTEX_ANCHOR_KEY='', CORTEX_ANCHOR_FPR='' is replaced by the same statement holding its recorded value in single quotes; "
                     "nothing else is added or changed")
RECORD_KEYS_2 = RECORD_KEYS + ("anchor",)
ANCHOR_KEYS = ("principal", "key", "fpr", "key_input_sha256")
ANCHOR_STATEMENTS = ("CORTEX_ANCHOR_PRINCIPAL", "CORTEX_ANCHOR_KEY", "CORTEX_ANCHOR_FPR")
PRINCIPAL_RX = re.compile(r"[A-Za-z0-9._@][A-Za-z0-9._@-]{0,63}")   # 1 to 64 characters, no leading hyphen (rel_trust's grammar)
KEY_RX = re.compile(r"AAAAC3NzaC1lZDI1NTE5AAAAI[A-Za-z0-9+/]{43}")   # an ed25519 public key blob: 68 characters
FPR_RX = re.compile(r"SHA256:[A-Za-z0-9+/]{43}")
KEY_FILE_MAX = 1024     # the approved key file is ONE bounded buffer
KEYGEN_TIMEOUT = 10     # seconds
DEFAULT_SSH_KEYGEN = "/usr/bin/ssh-keygen"
OBJECT_WIDTH = {"sha1": 40, "sha256": 64}
HEX64 = re.compile(r"[0-9a-f]{64}")
# marv's builder's function test (release_manifest.py join_installer, lanes r4a-r6 c8c3c84 and release-fixes 79b4abe): LEXICAL, it does not prove a file holds only functions
FUNCTION_PATTERN = re.compile(rb'(?m)^([a-zA-Z_][a-zA-Z0-9_]*)[ \t]*\([ \t]*\)[ \t\n]*[\{(]')


class DuplicateKey(ValueError):
    pass


def _no_duplicate_keys(pairs):
    out = {}
    for key, value in pairs:
        if key in out:
            raise DuplicateKey("duplicate JSON key %r" % key)
        out[key] = value
    return out


def validate_record(raw):
    """(record, failures) of the raw bytes of a binding record under schema r3: exactly the seven keys, no duplicate key (last-key-wins is refused), schema the integer 1
    and not a boolean, lowercase hex of the declared object format's width, distinct known include names with 64-digit hashes, the exact splice sentence."""
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError:
        return None, ["the record is not UTF-8"]
    try:
        rec = json.loads(text, object_pairs_hook=_no_duplicate_keys)
    except DuplicateKey as exc:
        return None, [str(exc)]
    except ValueError as exc:
        return None, ["the record is not valid JSON: %s" % exc]
    if not isinstance(rec, dict):
        return None, ["the top level of the record is not an object"]
    schema = rec.get("schema")
    if type(schema) is not int or schema not in (1, 2):
        return None, ["schema must be the integer 1 or 2 (a boolean, a string or another number is not)"]
    keys = RECORD_KEYS_2 if schema == 2 else RECORD_KEYS
    missing = [k for k in keys if k not in rec]
    unknown = sorted(str(k) for k in rec if k not in keys)
    fails = []
    if missing:
        fails.append("missing key(s): %s" % ", ".join(missing))
    if unknown:
        fails.append("unknown key(s): %s" % ", ".join(unknown))
    if fails:
        return None, fails
    fmt = rec["object_format"]
    width = OBJECT_WIDTH.get(fmt) if isinstance(fmt, str) else None
    if width is None:
        fails.append("object_format must be sha1 or sha256")
    for key in ("commit", "tree"):
        value = rec[key]
        if not (isinstance(value, str) and width and re.fullmatch("[0-9a-f]{%d}" % width, value)):
            fails.append("%s must be %s lowercase hex digits (the declared object format's width)" % (key, width or "N"))
    incs = rec["includes"]
    if not isinstance(incs, list) or not incs:
        fails.append("includes must be a non-empty list")
    else:
        seen = []
        for element in incs:
            if not (isinstance(element, dict) and sorted(element) == ["name", "sha256"]):
                fails.append("each includes element must be an object with exactly the keys name and sha256")
                continue
            name, digest_ = element["name"], element["sha256"]
            if not isinstance(name, str) or name not in KNOWN_INCLUDES:
                fails.append("unknown include name %r (the builder knows %s)" % (name, ", ".join(KNOWN_INCLUDES)))
            elif name in seen:
                fails.append("includes: duplicate name %s" % name)
            else:
                seen.append(name)
            if not (isinstance(digest_, str) and HEX64.fullmatch(digest_)):
                fails.append("includes: the sha256 of %s is not 64 lowercase hex digits" % (name if isinstance(name, str) else "an include"))
    if rec["splice"] != (SPLICE_SENTENCE_2 if schema == 2 else SPLICE_SENTENCE):
        fails.append("splice is not the exact versioned sentence of schema %d" % schema)
    if schema == 2:
        anchor = rec["anchor"]
        if not (isinstance(anchor, dict) and sorted(anchor) == sorted(ANCHOR_KEYS)):
            fails.append("anchor must be an object with exactly the keys principal, key, fpr and key_input_sha256")
        else:
            for name, rx in (("principal", PRINCIPAL_RX), ("key", KEY_RX), ("fpr", FPR_RX), ("key_input_sha256", HEX64)):
                if not (isinstance(anchor[name], str) and rx.fullmatch(anchor[name])):
                    fails.append("anchor.%s does not match its grammar" % name)
    if not (isinstance(rec["joined_sha256"], str) and HEX64.fullmatch(rec["joined_sha256"])):
        fails.append("joined_sha256 is not 64 lowercase hex digits")
    return (rec if not fails else None), fails


def input_kind_problem(label, path):
    """None when `path` is a regular file, or does not exist (the read then fails by itself and says so); else the refusal text. The kind is decided with lstat BEFORE
    any open, so a FIFO or a device cannot block the gate collector and a symlink is never followed (sam GR17-SAM01; marv's builder checks the type before read_bytes)."""
    try:
        mode = os.lstat(path).st_mode
    except OSError:
        return None
    if stat.S_ISREG(mode):
        return None
    return "the %s is not a regular file (a symlink, a FIFO or another kind of file): refused before it was opened" % label


def read_approved_key(path):
    """(info, failures) from ONE bounded buffer of the approved public key file (the runner's own independent input; the CTO's custody, C-57). The kind is judged with lstat
    before any open; at most KEY_FILE_MAX bytes are read once; the buffer is hashed and parsed from those same bytes: ASCII, no NUL, no CR, no private-key header, at most one
    trailing LF, exactly ONE line of exactly three single-space-separated fields <principal> ssh-ed25519 <key>."""
    problem = input_kind_problem("approved key file", path)
    if problem:
        return None, [problem]
    try:
        with open(path, "rb") as fh:
            buf = fh.read(KEY_FILE_MAX + 1)
    except OSError as exc:
        return None, ["the approved key file cannot be read: %s" % exc]
    if len(buf) > KEY_FILE_MAX:
        return None, ["the approved key file is over %d bytes" % KEY_FILE_MAX]
    if not buf:
        return None, ["the approved key file is empty"]
    fails = []
    if b"\x00" in buf:
        fails.append("the approved key file holds a NUL")
    if b"\r" in buf:
        fails.append("the approved key file holds a CR")
    if b"PRIVATE KEY" in buf:
        fails.append("the approved key file holds a private-key header")
    try:
        text = buf.decode("ascii")
    except UnicodeDecodeError:
        return None, fails + ["the approved key file is not ASCII"]
    body = text[:-1] if text.endswith("\n") else text
    if "\n" in body:
        fails.append("the approved key file has more than one line")
    fields = body.split(" ")
    principal = key = None
    if len(fields) != 3:
        fails.append("the approved key file must be exactly three space-separated fields: <principal> ssh-ed25519 <key>")
    else:
        principal, kind, key = fields
        if kind != "ssh-ed25519":
            fails.append("the approved key file's key type is not ssh-ed25519")
        if not PRINCIPAL_RX.fullmatch(principal):
            fails.append("the approved key file's principal is not 1 to 64 of [A-Za-z0-9._@-] without a leading hyphen")
        if not KEY_RX.fullmatch(key):
            fails.append("the approved key file's key is not an ed25519 public key blob of 68 characters")
    if fails:
        return None, fails
    return {"principal": principal, "key": key, "sha256": hashlib.sha256(buf).hexdigest()}, []


def derive_fpr(ctx, key, scratch_parent, receipts):
    """(fingerprint or None, failures): `ssh-keygen -l -f` on `ssh-ed25519 <key>` held in a 0600 file inside a fresh private 0700 directory (vera VA-J2); the tool's exit status,
    a timeout, malformed or multiline output and every kind other than ED25519 are refused; the scratch is removed on every outcome."""
    scratch = tempfile.mkdtemp(prefix=".anchor-", dir=scratch_parent)
    try:
        os.chmod(scratch, 0o700)
        keyfile = os.path.join(scratch, "k.pub")
        fd = os.open(keyfile, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "wb") as fh:
            fh.write(b"ssh-ed25519 " + key.encode("ascii") + b"\n")
        os.chmod(keyfile, 0o600)   # explicit, whatever the umask: a 0700 DIRECTORY holding a 0600 FILE (vera VA-J2)
        step = ctx.local_step("G12", "anchor_fpr", [ctx.args.ssh_keygen, "-l", "-f", keyfile], timeout=ctx.args.keygen_timeout)
        receipts.append(step.base)
        if step.rc == 124:
            return None, ["the fingerprint tool timed out after %s seconds" % ctx.args.keygen_timeout]
        if step.rc != 0:
            return None, ["the fingerprint tool exited %d" % step.rc]
        lines = step.out.decode("utf-8", "replace").split("\n")
        if len(lines) != 2 or lines[1] != "":
            return None, ["the fingerprint tool's output is not exactly one line"]
        m = re.fullmatch(r"256 (SHA256:[A-Za-z0-9+/]{43}) .* \(([A-Z0-9-]+)\)", lines[0])
        if not m:
            return None, ["the fingerprint tool's output is malformed"]
        if m.group(2) != "ED25519":
            return None, ["the fingerprint tool reports the key kind %s, not ED25519" % m.group(2)]
        return m.group(1), []
    finally:
        shutil.rmtree(scratch, ignore_errors=True)


def anchor_problems(text):
    """The failures of the INVENTORY of the three empty anchor statements in a text: the raw development installer (before T1, sam GR19-SAM02) or its T1 result (before T2). After the
    header (leading lines that are blank or begin with #) the next three lines are exactly the three EMPTY statements in order, and each occurs exactly once, none already filled."""
    lines = text.splitlines(keepends=True)
    empties = [("%s=''\n" % n).encode() for n in ANCHOR_STATEMENTS]
    fails = []
    for n, e in zip(ANCHOR_STATEMENTS, empties):
        count = sum(1 for l in lines if l == e)
        filled = [l for l in lines if l.startswith(n.encode() + b"=") and l != e]
        if count == 0 and filled:
            fails.append("the anchor statement %s is already filled" % n)
        elif count == 0:
            fails.append("the anchor statement %s is missing" % n)
        elif count > 1:
            fails.append("the anchor statement %s occurs %d times" % (n, count))
    top = 0
    while top < len(lines) and (not lines[top].strip() or lines[top].startswith(b"#")):
        top += 1
    if not fails and lines[top:top + 3] != empties:
        fails.append("the three anchor statements are not the first three statements after the header, in order")
    return fails


def apply_anchor(text, values):
    """(final bytes, failures): T2 of schema 2. The text must pass anchor_problems; then each of the three empty statements is replaced by the same statement holding its value in
    single quotes; every other byte stays."""
    fails = anchor_problems(text)
    if fails:
        return None, fails
    lines = text.splitlines(keepends=True)
    empties = [("%s=''\n" % n).encode() for n in ANCHOR_STATEMENTS]
    out = []
    for l in lines:
        if l in empties:
            n = ANCHOR_STATEMENTS[empties.index(l)]
            l = ("%s='%s'\n" % (n, values[n])).encode()
        out.append(l)
    return b"".join(out), []


def marker_name(line):
    """The include named by a marker line in one of the builder's two exact forms, else None."""
    for form in MARKER_FORMS:
        m = form.fullmatch(line)
        if m:
            return m.group(1).decode()
    return None


def splice(dev_text, includes):
    """The shipped text as I rebuild it (a second method, not marv's joiner): each marker line, newline included, is replaced by the exact bytes of its include."""
    out = b""
    for line in dev_text.splitlines(keepends=True):
        if re.search(rb"@include", line):
            name = marker_name(line)
            if name is None:
                raise ValueError("a marker line the builder would not accept")
            out += includes[name]
        else:
            out += line
    return out


def way_c_command(a):
    """Way C: the ONE command step (g) runs when --oracle-tree is given: the checker OF THE ORACLE TREE on the deploy directory OF THE DEVELOPMENT TREE. The allow-list takes exactly this list (Ctx.way_c_step)."""
    return ["sh", os.path.join(a.oracle_tree, "scripts", "s5-check.sh"), os.path.join(a.dev_tree, "deploy")]


def way_c_identity(ctx, a, candidate_commit, candidate_tree, argv):
    """Way C (cox W1-177 and W1-178; nix's spec e4d859a3; cox's one list 5350b273, pin A): when --oracle-tree is given, step (g) runs the checker OF THE ORACLE TREE on the deploy directory OF THE DEVELOPMENT TREE.
    Before it runs, the identity of that checker, the candidate's inputs and the ACTUAL command are receipted: the five lines the matrix receipt uses for the oracle (oracle_tree, oracle_file_sha256, oracle_commit,
    oracle_tree_id, oracle_is_development_tree: git ids, or 'none (git rc N)' when the tree is not a git tree: recorded, not hidden) and more: the executed wrapper's sha256, the candidate's inputs, the command, whether the
    oracle's scripts/ is clean, the oracle's git top level, both real paths, and the running runner's own path and sha256.
    Returns (the receipt names, the problems, the refusals, what step (g) saw). The problems are added to the failures of G12 (nix's specification answer of 12 Nov: an oracle whose commit or tree id cannot be read, or whose
    checker files cannot be hashed, is NOT BOUND to a tool head; cox's F1: nor is one whose scripts/ has uncommitted changes or that is not its own git top level; every git read that fails proves nothing: its numeric status is in the
    verdict; no silent fallback). The refusals (F2: vera's return on eec6febc) are the problems after which the checker is NOT run: a wrapper, a checker or a scripts/ directory that is a symbolic link, and a candidate's deploy
    directory that resolves outside the development tree."""
    root = a.oracle_tree
    gc = ctx.local_step("G12", "oracle_g_commit", ["git", "-C", root, "rev-parse", "HEAD"])
    gt = ctx.local_step("G12", "oracle_g_tree_id", ["git", "-C", root, "rev-parse", "HEAD^{tree}"])
    gs = ctx.local_step("G12", "oracle_g_status", ["git", "-C", root, "status", "--porcelain", "--untracked-files=no", "--", "scripts"])
    gl = ctx.local_step("G12", "oracle_g_toplevel", ["git", "-C", root, "rev-parse", "--show-toplevel"])

    def sha_of(path):
        try:
            return sha256_file(path)
        except OSError:
            return "unreadable"
    same = os.path.realpath(root) == os.path.realpath(a.dev_tree)
    commit_id = gc.out.decode().strip() if gc.rc == 0 else None
    tree_id = gt.out.decode().strip() if gt.rc == 0 else None
    top = (os.path.realpath(gl.out.decode().strip()) if gl.out.strip() else "none (empty)") if gl.rc == 0 else None
    clean = ("yes" if not gs.out.strip() else "NO") if gs.rc == 0 else "unknown (git rc %d)" % gs.rc
    deploy = os.path.join(a.dev_tree, "deploy")
    real_deploy = os.path.realpath(deploy)
    file_sha = sha_of(os.path.join(root, "scripts", "s5_check.py"))
    wrapper_sha = sha_of(os.path.join(root, "scripts", "s5-check.sh"))   # F5 (cox's list; ellis r2 F-7, F-8): each of the four hashes is read ONCE and the same value is receipted and checked
    runner_path = os.path.abspath(__file__)
    runner_sha = sha_of(runner_path)
    matrix_path = os.path.join(os.path.dirname(runner_path), "gate_matrix.py")
    matrix_sha = sha_of(matrix_path)
    text = ("oracle_tree=%s\noracle_file_sha256=%s\noracle_commit=%s\noracle_tree_id=%s\noracle_is_development_tree=%s\n"
            "oracle_checker_sha256=%s\ncandidate_deploy=%s\ncandidate_commit=%s\ncandidate_tree=%s\ncommand=%s\n"
            "oracle_scripts_clean=%s\noracle_git_toplevel=%s\noracle_tree_real=%s\ncandidate_deploy_real=%s\ngate_runner_path=%s\ngate_runner_sha256=%s\ngate_matrix_sha256=%s\n") % (
        root, file_sha, commit_id or "none (git rc %d)" % gc.rc, tree_id or "none (git rc %d)" % gt.rc, "yes" if same else "NO",
        wrapper_sha, deploy, candidate_commit, candidate_tree, shlex.join(argv),
        clean, top or "none (git rc %d)" % gl.rc, os.path.realpath(root), real_deploy, runner_path, runner_sha,
        matrix_sha)   # the matrix module the runner loads is part of the running tool (ellis)
    problems, refused = [], []
    if gc.rc != 0:
        problems.append("the oracle's identity is not bound: its commit cannot be read (git rc %d)" % gc.rc)
    if gt.rc != 0:
        problems.append("the oracle's identity is not bound: its tree id cannot be read (git rc %d)" % gt.rc)
    for fname, value in (("s5-check.sh", wrapper_sha), ("s5_check.py", file_sha)):
        if value == "unreadable":
            problems.append("the oracle's checker hash is unavailable: scripts/%s cannot be read" % fname)
    if runner_sha == "unreadable":
        problems.append("the running gate_runner.py cannot be hashed (%s): the receipt cannot name the tool that ran" % runner_path)
    if matrix_sha == "unreadable":
        problems.append("gate_matrix.py cannot be hashed (%s): the receipt cannot name the matrix module that ran" % matrix_path)
    if gs.rc != 0:
        problems.append("the oracle's git cleanliness read failed (rc %d): the tool tree is not bound to its commit" % gs.rc)
    elif gs.out.strip():
        problems.append("the oracle's scripts/ has uncommitted changes: the tool head is not the tree that ran")
    if gl.rc != 0:
        problems.append("the oracle's git top level cannot be read (git rc %d)" % gl.rc)
    elif top != os.path.realpath(root):
        problems.append("the oracle tree is not its own git top level (it lies inside the repository %s): its commit and tree ids are that repository's, not the tool tree's" % top)
    for label, path in (("scripts directory", os.path.join(root, "scripts")), ("wrapper scripts/s5-check.sh", os.path.join(root, "scripts", "s5-check.sh")), ("checker scripts/s5_check.py", os.path.join(root, "scripts", "s5_check.py"))):
        if os.path.islink(path):
            refused.append("the oracle's %s is a symbolic link: refused, the bytes that run must be in the tree that was read" % label)
    if real_deploy != os.path.join(os.path.realpath(a.dev_tree), "deploy"):
        refused.append("the candidate's deploy directory %s resolves outside the development tree (to %s): the checker would not judge the candidate's bytes" % (deploy, real_deploy))
    problems += refused
    if refused:
        problems.append("the oracle's checker was NOT run")
    seen = {"commit": commit_id, "tree": tree_id, "file": file_sha}
    return [gc.base, gt.base, gs.base, gl.base, ctx.rec.data_receipt("G12", "oracle_identity", text)], problems, refused, seen


def way_c_consistency(seen, oc, ot, oracle_file):
    """Vera's row 152f59bc (ellis: the oracle's identity is read twice with no cross-check): the identity the matrix reads LATER, after the checker ran, must be the one step (g) bound. A read that fails, or a commit,
    tree id or s5_check.py that is not the one step (g) saw, is a problem for the caller to add to the failures of G12: a later none or mismatch cannot leave it PASS."""
    problems = []
    for what, st in (("commit", oc), ("tree id", ot)):
        if st.rc != 0:
            problems.append("the oracle's identity is not bound at the matrix: its %s cannot be read (git rc %d)" % (what, st.rc))
    for what, st, before in (("commit", oc, seen["commit"]), ("tree id", ot, seen["tree"])):
        now_id = st.out.decode().strip() if st.rc == 0 else None
        if now_id and before and now_id != before:
            problems.append("the oracle's %s changed between step (g) and the matrix: %s then %s" % (what, before, now_id))
    try:
        later = sha256_file(oracle_file)
    except OSError:
        later = "unreadable"
    if later != seen["file"]:
        problems.append("the oracle's s5_check.py changed between step (g) and the matrix: sha256 %s then %s" % (seen["file"], later))
    return problems


def check_g12(ctx):
    a = ctx.args
    if not a.dev_tree or not a.binding_record:
        return "NOT RUN", "G12 needs --dev-tree (the development tree at the candidate commit) and --binding-record (marv's record made at the cut)", []
    shipped_path = a.shipped or os.path.join(a.artefacts, "install.sh")
    dev_install = os.path.join(a.dev_tree, "deploy", "install.sh")
    receipts, fails = [], []
    for label, path in (("binding record", a.binding_record),
                        ("shipped install.sh", shipped_path),
                        ("development install.sh", dev_install)):
        problem = input_kind_problem(label, path)
        if problem:
            return "FAIL", problem, receipts
    try:
        record_bytes = open(a.binding_record, "rb").read()
        shipped = open(shipped_path, "rb").read()
        dev_text = open(dev_install, "rb").read()
    except OSError as exc:
        return "FAIL", "an input of G12 cannot be read: %s" % exc, receipts
    record, record_fails = validate_record(record_bytes)
    if record is None:
        return "FAIL", "the binding record is not valid: %s" % "; ".join(record_fails), receipts
    if a.approved_key_file and record["schema"] != 2:
        return "FAIL", "an approved key file was supplied but the record is schema 1: it has no anchor to check", receipts
    if not a.approved_key_file and record["schema"] == 2:
        return "FAIL", "the record is schema 2 (it has an anchor) but no --approved-key-file was supplied: the anchor cannot be checked", receipts
    expected, approved = None, None
    if a.approved_key_file:
        approved, approved_fails = read_approved_key(a.approved_key_file)
        if approved is None:
            return "FAIL", "the approved key file is not valid: %s" % "; ".join(approved_fails), receipts
        fpr, fpr_fails = derive_fpr(ctx, approved["key"], os.path.dirname(os.path.abspath(ctx.rec.path)), receipts)
        if fpr is None:
            return "FAIL", "the expected fingerprint could not be derived: %s" % "; ".join(fpr_fails), receipts
        expected = {"principal": approved["principal"], "key": approved["key"], "fpr": fpr, "key_input_sha256": approved["sha256"]}
    markers = []
    for line in dev_text.splitlines(keepends=True):
        if re.search(rb"@include", line):
            name = marker_name(line)
            if name is None:
                fails.append("a marker line the builder would not accept (exact plain or guarded form, three spaces before # @include, LF): %r" % line.strip()[:70])
            else:
                markers.append(name)
    for dup in sorted({n for n in markers if markers.count(n) > 1}):
        fails.append("duplicate include marker: %s is named %d times in the development installer" % (dup, markers.count(dup)))
    for known in KNOWN_INCLUDES:   # sam GR17-SAM02: the builder requires EACH known include exactly once, whatever the record says
        if markers.count(known) == 0:
            fails.append("include marker missing: %s is not named in the development installer (the builder requires each of %s exactly once)" % (known, ", ".join(KNOWN_INCLUDES)))
    for unknown in sorted({n for n in markers if n not in KNOWN_INCLUDES}):
        fails.append("the development installer names an include the builder does not know: %s" % unknown)
    anchor_raw_failed = False
    if expected:   # sam GR19-SAM02: the RAW development installer must itself carry the three empty statements (inventory, order, top); an include must not be able to supply them
        raw_problems = anchor_problems(dev_text)
        for problem in raw_problems:
            fails.append("development installer: " + problem)
        anchor_raw_failed = bool(raw_problems)
    includes = {}
    kind_failed = False
    for n in markers:
        ipath = os.path.join(a.dev_tree, "deploy", n)
        problem = input_kind_problem("include " + n, ipath)
        if problem:
            fails.append(problem)
            kind_failed = True
            continue
        try:
            includes[n] = open(ipath, "rb").read()
        except OSError:
            fails.append("include %s is missing from the development tree" % n)
    if kind_failed:   # vera VR18-01: a refused include must stop the run BEFORE any delegated reader (the S5 checker walks deploy/*.sh and would open it)
        return "FAIL", "; ".join(fails), receipts
    if not expected:   # sam GR19-SAM01: without a key every statement of every input of the final bytes is empty or absent, and the failure names the origin
        for origin, data in [("development installer", dev_text)] + [("include " + n, d) for n, d in includes.items()] + [("shipped installer", shipped)]:
            for n in ANCHOR_STATEMENTS:
                empty = ("%s=''\n" % n).encode()
                if any(l.startswith(n.encode() + b"=") and l != empty for l in data.splitlines(keepends=True)):
                    fails.append("the %s's %s is filled but no approved key file was supplied" % (origin, n))
    digest = lambda b: hashlib.sha256(b).hexdigest()
    before = {"shipped": digest(shipped), "dev": digest(dev_text), "record": digest(record_bytes), **{n: digest(d) for n, d in includes.items()}}
    if approved:
        before["approved"] = approved["sha256"]
        for k in ANCHOR_KEYS:   # vera VA-J3: every recorded field against the expectation derived from the APPROVED file, never against the installer alone
            if record["anchor"][k] != expected[k]:
                fails.append("the record's anchor %s differs from the one derived from the approved key file" % k)
        shipped_lines = shipped.splitlines(keepends=True)
        for n, field in zip(ANCHOR_STATEMENTS, ("principal", "key", "fpr")):
            found = [l for l in shipped_lines if l.startswith(n.encode() + b"=")]
            if len(found) != 1:
                fails.append("the shipped installer has %d %s statement(s), wanted exactly one" % (len(found), n))
            elif found[0] != ("%s='%s'\n" % (n, expected[field])).encode():
                fails.append("the shipped installer's %s literal differs from the one derived from the approved key file" % n)
    # (a) syntax, in the shells named: bash --posix is what sh is on Rocky Linux 10
    for step, argv, word in (("sh_n", ["sh", "-n", shipped_path], "sh -n"), ("bash_posix_n", ["/usr/bin/bash", "--posix", "-n", shipped_path], "bash --posix -n")):
        r = ctx.local_step("G12", step, argv)
        receipts.append(r.base)
        if r.rc != 0:
            fails.append("syntax: %s failed on the shipped install.sh" % word)
    # (b) the splice, rebuilt by a second method
    if not fails or all(n in includes for n in markers):
        try:
            rebuilt = splice(dev_text, includes)
        except (KeyError, StopIteration, ValueError):
            rebuilt = None
        t2_failed = anchor_raw_failed
        if rebuilt is not None and expected and not anchor_raw_failed:
            final, t2_fails = apply_anchor(rebuilt, {"CORTEX_ANCHOR_PRINCIPAL": expected["principal"], "CORTEX_ANCHOR_KEY": expected["key"], "CORTEX_ANCHOR_FPR": expected["fpr"]})
            if final is None:
                fails.extend(t2_fails)
                t2_failed = True
            else:
                rebuilt = final
        if rebuilt is None:
            fails.append("the splice could not be rebuilt by the second method: the shipped file is not independently checked")   # vera B-F1: never a silent skip
        elif t2_failed:
            pass
        elif b"# @include" in rebuilt:
            fails.append("the rebuilt text still holds an include marker")
        elif rebuilt != shipped:
            at = next((i for i in range(min(len(rebuilt), len(shipped))) if rebuilt[i] != shipped[i]), min(len(rebuilt), len(shipped)))
            fails.append("the shipped file differs from the development install.sh with the includes spliced (first difference at byte %d)" % at)
    # (d) the shape of each include
    for n, data in includes.items():
        if not data.endswith(b"\n"):
            fails.append("include %s does not end with a newline" % n)
        elif data[:-1].split(b"\n")[-1] not in (b"}", b")"):
            fails.append("include %s: its last line is not exactly one closing brace or parenthesis at column 0" % n)
        if b"\r" in data:
            fails.append("include %s holds a carriage return" % n)
        if data.startswith(b"#!"):
            fails.append("include %s starts with a shebang" % n)
        if b"# @include" in data:
            fails.append("include %s holds the marker text" % n)
        if not FUNCTION_PATTERN.search(data):
            fails.append("include %s defines no function (marv's lexical test)" % n)
    # (c) the binding record, and the development tree it names
    commit = ctx.local_step("G12", "git_commit", ["git", "-C", a.dev_tree, "rev-parse", "HEAD"])
    tree = ctx.local_step("G12", "git_tree", ["git", "-C", a.dev_tree, "rev-parse", "HEAD^{tree}"])
    dirty = ctx.local_step("G12", "git_status", ["git", "-C", a.dev_tree, "status", "--porcelain", "--untracked-files=no", "--", "deploy", "scripts"])
    fmt = ctx.local_step("G12", "git_object_format", ["git", "-C", a.dev_tree, "rev-parse", "--show-object-format"])
    receipts += [commit.base, tree.base, dirty.base, fmt.base]
    if fmt.rc != 0 or fmt.out.decode("utf-8", "replace").strip() != record["object_format"]:
        fails.append("the record's object format %s is not the development tree's (git said %r, rc %d)" % (record["object_format"], fmt.out.decode("utf-8", "replace").strip(), fmt.rc))
    for what, step_ in (("commit", commit), ("tree", tree), ("cleanliness", dirty)):   # vera B-F2: a git read that failed proves nothing
        if step_.rc != 0:
            fails.append("the git %s read failed (rc %d): the candidate is not bound to a commit" % (what, step_.rc))
    if record.get("joined_sha256") != before["shipped"]:
        fails.append("the shipped file's sha256 differs from the joined sha256 in the binding record")
    if [i.get("name") for i in record.get("includes", [])] != markers:
        fails.append("the include order differs from the binding record")
    for i in record.get("includes", []):
        if i.get("name") in includes and i.get("sha256") != before[i["name"]]:
            fails.append("%s: sha256 differs from the binding record" % i["name"])
    if record.get("commit") != commit.out.decode().strip():
        fails.append("the commit differs from the binding record")
    if record.get("tree") != tree.out.decode().strip():
        fails.append("the tree differs from the binding record")
    if dirty.out.strip():
        fails.append("the development tree has uncommitted changes in deploy/ or scripts/")
    if expected:
        try:
            keygen_sha = digest(open(a.ssh_keygen, "rb").read())
        except OSError:
            keygen_sha = "unreadable"
        anchor_lines = ("anchor_mode=approved-file\nanchor_principal=%s\nanchor_key=%s\nanchor_fpr=%s\nanchor_key_input_sha256=%s\nssh_keygen=%s\nssh_keygen_sha256=%s\n"
                        % (expected["principal"], expected["key"], expected["fpr"], expected["key_input_sha256"], a.ssh_keygen, keygen_sha))
        anchor_note = "anchor: the approved key file (sha256 %s, an INPUT the runner was given: the approval is the CTO's, C-57) matches the record, the installer's literals and the rebuilt final bytes" % expected["key_input_sha256"][:12]
    else:
        anchor_lines = "anchor_mode=none\n"
        anchor_note = "NO ANCHOR (schema 1, no approved key file: a test-only path, not a candidate)"
    receipts.append(ctx.rec.data_receipt("G12", "binding", "shipped_sha256=%s\ndev_install_sha256=%s\n%scommit=%s\ntree=%s\nobject_format=%s\nbinding_record_sha256=%s\nsplice=%s\nbinding_schema=%s\n%s" % (
        before["shipped"], before["dev"], "".join("include %s sha256=%s\n" % (n, before[n]) for n in includes), commit.out.decode().strip(), tree.out.decode().strip(),
        record["object_format"], before["record"], record["splice"], BINDING_SCHEMA_STATUS, anchor_lines)))
    # (g) the checker, and the sentence it prints
    way_c_seen = None
    way_c_problems = []
    if a.oracle_tree:
        # way C: the checker OF THE ORACLE TREE judges the deploy directory OF THE DEVELOPMENT TREE (the tool head's wording of the NOTE, the candidate's bytes), never the tool tree's own deploy directory
        way_c_argv = way_c_command(a)
        way_c_receipts, way_c_problems, way_c_refused, way_c_seen = way_c_identity(ctx, a, commit.out.decode().strip(), tree.out.decode().strip(), way_c_argv)
        receipts += way_c_receipts
        fails += way_c_problems
        if way_c_refused:   # refused (F2), not run: the receipt says why; rc 0 and no output, so the checks below add only what is true: the NOTE line was not printed
            orc = Step(ctx.rec.data_receipt("G12", "oracle_not_run", "the oracle's checker was NOT run: " + "; ".join(way_c_refused) + "\n"), 0, b"", b"")
        else:
            orc = ctx.way_c_step(way_c_argv, a.dev_tree)
    else:
        orc = ctx.local_step("G12", "oracle", ["sh", "scripts/s5-check.sh", "deploy"], cwd=a.dev_tree)
    receipts.append(orc.base)
    lines = (orc.out + orc.err).decode("utf-8", "replace").splitlines()
    if orc.rc != 0:
        fails.append("the S5 checker exited %d" % orc.rc)
    if lines.count(NOTE_LINE) != 1:
        fails.append("the checker did not print the NOTE line verbatim exactly once")
    claims = [l for l in lines if l != NOTE_LINE and PROOF_CLAIM.search(l)]
    if claims:
        fails.append("a claim that the checker proves definitions only: %r" % claims[0][:80])
    # (e) the finite input matrix, layer L1, on every include, in hue's oracle namespace
    here = os.path.dirname(os.path.abspath(__file__))
    inventory_path = a.inventory or os.path.join(here, "matrix", "inventory.json")
    matrix_summary = None
    try:
        spec = importlib.util.spec_from_file_location("gate_matrix_loaded", os.path.join(here, "gate_matrix.py"))
        gm = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(gm)
        inventory_sha = gm.sha256_file(inventory_path)
        inv = json.load(open(inventory_path, encoding="utf-8"))
        if a.inventory_approved_sha256 != inventory_sha:
            fails.append("the matrix inventory is not owner-approved: its sha256 %s is not the approved one" % inventory_sha[:16])
        elif way_c_problems:   # F4 (cox's list; ellis r2 P2): an oracle that step (g) refused or could not bind is NOT loaded or executed by any later step: the matrix imports the oracle's code into this very process
            fails.append("the matrix was NOT run: the oracle is not bound (%d problem(s) of step (g) above): its code is not loaded" % len(way_c_problems))
        else:
            oracle_tree = a.oracle_tree or a.dev_tree
            oracle_file = os.path.join(oracle_tree, "scripts", "s5_check.py")
            oc = ctx.local_step("G12", "oracle_commit", ["git", "-C", oracle_tree, "rev-parse", "HEAD"])
            ot = ctx.local_step("G12", "oracle_tree_id", ["git", "-C", oracle_tree, "rev-parse", "HEAD^{tree}"])
            receipts += [oc.base, ot.base]
            same_tree = os.path.realpath(oracle_tree) == os.path.realpath(a.dev_tree)
            identity = "oracle_tree=%s\noracle_file_sha256=%s\noracle_commit=%s\noracle_tree_id=%s\noracle_is_development_tree=%s\n" % (
                oracle_tree, digest(open(oracle_file, "rb").read()), oc.out.decode().strip() if oc.rc == 0 else "none (git rc %d)" % oc.rc,
                ot.out.decode().strip() if ot.rc == 0 else "none (git rc %d)" % ot.rc, "yes" if same_tree else "NO")
            if a.oracle_tree:
                later = way_c_consistency(way_c_seen, oc, ot, oracle_file)
                fails += later
                if later:   # cox's read of the repaired pin A (row 5651bc45): an oracle that changed between step (g) and the matrix is reported AND NOT LOADED
                    raise MatrixNotRun("the matrix was NOT run: the oracle is not the one step (g) bound (%d problem(s) above): its code is not loaded" % len(later))
            oracle = gm.load_oracle(oracle_tree)
            prefix = gm.prefix_from_install(dev_text)
            rows, counts, untestable, raw = gm.run_matrix(oracle, includes, inv, prefix)
            receipts.append(ctx.rec.data_receipt("G12", "matrix", "inventory_sha256=%s\ninclude_context_sha256=%s\n%slayer_l1=run\nlayer_l2=%s\ncells=%d includes=%d\n%s" % (
                inventory_sha, digest(prefix), identity, L2_STATUS, len(gm.generate_cells(inv, untestable)), len(includes), gm.table(rows))))
            receipts.append(ctx.rec.data_receipt("G12", "matrix_raw", gm.raw_text(raw)))
            bad_control = [r for r in rows if r[0] == "M000" and r[4] != "ACCEPT"]
            if bad_control:
                fails.append("the clean control cell is not accepted in %s (%s): the matrix proves nothing" % (bad_control[0][1], bad_control[0][4]))
            if counts["UNTESTABLE"] * 2 > len(rows):
                fails.append("the matrix is mostly untestable (%d of %d cells): it proves too little" % (counts["UNTESTABLE"], len(rows)))
            refused = [(r[0], r[1], r[3], r[5]) for r in rows if r[4] == "REFUSE"]
            matrix_summary = "matrix %d cells x %d includes: %d accepted, %d untestable (%s); oracle s5_check.py sha256 %s from %s" % (
                len(rows) // max(len(includes), 1), len(includes), counts["ACCEPT"], counts["UNTESTABLE"], ", ".join(sorted(untestable)) or "none",
                identity.split("oracle_file_sha256=")[1].split("\n")[0][:12], "the development tree" if same_tree else "ANOTHER tree than the development tree")
            if refused:
                fails.append("the matrix refused %d cell(s), first: %s in %s (%s): %s" % (len(refused), refused[0][0], refused[0][1], refused[0][2], refused[0][3]))
    except MatrixNotRun as exc:
        fails.append(str(exc))
    except (OSError, ValueError, KeyError) as exc:
        fails.append("the matrix could not run: %s" % exc)
    except Exception as exc:
        fails.append("the matrix could not run: %s: %s" % (type(exc).__name__, exc))
    # any byte drift while this ran invalidates it
    try:
        reread = [shipped_path, dev_install, a.binding_record] + [os.path.join(a.dev_tree, "deploy", n) for n in includes] + ([a.approved_key_file] if approved else [])
        if any(input_kind_problem("input", path) for path in reread):
            after = None
        else:
            after = {"shipped": digest(open(shipped_path, "rb").read()), "dev": digest(open(dev_install, "rb").read()), "record": digest(open(a.binding_record, "rb").read()),
                     **{n: digest(open(os.path.join(a.dev_tree, "deploy", n), "rb").read()) for n in includes}}
            if approved:
                after["approved"] = digest(open(a.approved_key_file, "rb").read(KEY_FILE_MAX + 1))
    except OSError:
        after = None
    if after != before:
        fails.append("bytes changed while the check ran")
    if fails:
        return "FAIL", "; ".join(fails), receipts
    return "PASS", L1_ONLY + "shipped install.sh bound by sha256 %s to commit %s: syntax ok, splice rebuilt equal, %d includes shaped, checker NOTE line printed; %s; binding schema %s (r3 freezes only when a REAL record has been emitted: this run used a fixture or an unfrozen record); %s" % (
        before["shipped"][:12], commit.out.decode().strip()[:10], len(includes), matrix_summary, BINDING_SCHEMA_STATUS, anchor_note), receipts


def not_built(cid):
    return lambda ctx: ("NOT RUN", "not built in increment R7", [])


CHECKS = {"G1": check_g1, "G2": check_g2, "G9": check_g9, "G3": check_g3, "G4": check_g4, "G5": check_g5, "G8": check_g8, "G11": check_g11, "G10": check_g10, "G7": check_g7, "G12": check_g12}


# ---- refusal of this host -----------------------------------------------------------------------------------------------------

def local_out(argv):
    p = subprocess.run(argv, stdin=subprocess.DEVNULL, capture_output=True)
    return p.returncode, p.stdout.decode("utf-8", "replace")


def refuse_name_and_address(target):
    host = target.rsplit("@", 1)[-1].strip("[]").split("%")[0].lower()
    first = host.split(".")[0]
    if host in ("localhost", "localhost.localdomain") or host.endswith(".localhost") or first in LIVE_HOSTS:
        raise Refused(EX_REFUSED_NAME, "the target %r is this host or a protected host (build-host-1, build-host-2, localhost)" % host)
    rc, name = local_out(["hostname"])
    name = name.strip().lower()
    if rc != 0 or not name:
        raise Refused(EX_REFUSED_NAME, "cannot read this host's name; refusing")
    if host == name or first == name.split(".")[0]:
        raise Refused(EX_REFUSED_NAME, "the target name %r is this host's name" % host)
    rc, text = local_out(["ip", "-o", "addr", "show"])
    local = set()
    for m in re.finditer(r"\binet6? ([0-9a-fA-F:.]+)/", text):
        local.add(ipaddress.ip_address(m.group(1)))
    if rc != 0 or not local:
        raise Refused(EX_REFUSED_ADDR, "cannot list this host's addresses; refusing")
    try:
        addrs = [ipaddress.ip_address(host)]
    except ValueError:
        rc, text = local_out(["getent", "ahosts", host])
        addrs = []
        for line in text.splitlines():
            try:
                addrs.append(ipaddress.ip_address(line.split()[0].split("%")[0]))
            except (ValueError, IndexError):
                pass
        if rc != 0 or not addrs:
            raise Refused(EX_USAGE, "cannot resolve the target %r" % host)
    for addr in addrs:
        if addr.is_loopback or addr in local:
            raise Refused(EX_REFUSED_ADDR, "the target %r resolves to %s, an address of this host" % (host, addr))


def identity_probe(ctx, local_id_path):
    try:
        local_id = open(local_id_path).read().strip().lower()
    except OSError:
        raise Refused(EX_USAGE, "cannot read this host's machine-id (%s); pass --local-machine-id" % local_id_path)
    r = ctx.step("00", "machine_id", "identity_machine_id")
    remote_id = r.out.decode("utf-8", "replace").strip().lower()
    if r.rc != 0 or not re.fullmatch(r"[0-9a-f]{32}", remote_id):
        raise Refused(EX_REFUSED_MACHINE_ID, "cannot read a machine-id from the target; refusing (it could be this host)")
    if remote_id == local_id:
        raise Refused(EX_REFUSED_MACHINE_ID, "the target's machine-id equals this host's: it IS this host")
    h = ctx.step("00", "hostname", "identity_hostname")
    remote_host = h.out.decode("utf-8", "replace").strip().lower()
    _, mine = local_out(["hostname"])
    mine = mine.strip().lower()
    if h.rc != 0 or not remote_host:
        raise Refused(EX_REFUSED_HOSTNAME, "cannot read the target's hostname; refusing")
    if remote_host.split(".")[0] == mine.split(".")[0] or remote_host.split(".")[0] in LIVE_HOSTS:
        raise Refused(EX_REFUSED_HOSTNAME, "the target's hostname %r is this host's or a protected host's" % remote_host)


# ---- the table and the run -----------------------------------------------------------------------------------------------------

def overall_of(results, interrupted):
    if interrupted:
        return "INTERRUPTED", EX_INTERRUPTED
    verdicts = [results[g][0] for g in TABLE_ORDER]
    if "FAIL" in verdicts:
        return "FAIL", EX_FAIL
    if "NOT RUN" in verdicts or L2_STATUS != "PRESENT":
        return "INCOMPLETE", EX_INCOMPLETE
    return "PASS", EX_PASS


def clean(text):
    return re.sub(r"[\t\r\n]+", " ", text)


def table_text(results, overall, evidence, target, partial=False):
    lines = ["gate run %s target=%s" % (now(), target)]
    if partial:
        lines.append("PARTIAL DRY RUN IN A CONTAINER, NOT THE GATE: G1, G2, G9, G5, G7 (without lingering) and G8 only; the rest needs the clean Rocky 10.2 VM")
    for g in TABLE_ORDER:
        v, note, receipts = results[g]
        lines.append("%-4s %-8s %s" % (g, v, clean(note)))
    if L2_STATUS != "PRESENT":
        lines.append("G12 layer L2 (the run-time matrix) is %s: this table can never be an overall PASS until it exists" % L2_STATUS)
    lines.append("OVERALL: " + overall)
    lines.append("evidence: " + evidence)
    return "\n".join(lines) + "\n"


def parse_args(argv):
    p = argparse.ArgumentParser(prog="gate_runner.py", description="6f gate runner (increment R1)")
    g = p.add_mutually_exclusive_group(required=True)
    g.add_argument("--target")
    g.add_argument("--container")
    p.add_argument("--evidence-dir", required=True)
    p.add_argument("--artefacts", required=True)
    p.add_argument("--project", required=True)
    p.add_argument("--lead", required=True)
    p.add_argument("--image-name", required=True)
    p.add_argument("--image-version", required=True)
    p.add_argument("--image-checksum", required=True)
    p.add_argument("--image-source", required=True)
    p.add_argument("--scenario", choices=("clean", "partial"), default="clean")
    p.add_argument("--only", default="")
    p.add_argument("--port", default="8601")
    p.add_argument("--home", default="$HOME/cortex-v2")
    p.add_argument("--install-timeout", type=int, default=1800)
    p.add_argument("--dev-tree", default="")
    p.add_argument("--oracle-tree", default="")
    p.add_argument("--inventory", default="")
    p.add_argument("--inventory-approved-sha256", default="")
    p.add_argument("--binding-record", default="")
    p.add_argument("--approved-key-file", default="")   # the runner's own independent input for the trust anchor (r4): the CTO's approved PUBLIC key; never read from the record
    p.add_argument("--keygen-timeout", type=float, default=KEYGEN_TIMEOUT)   # seconds; a test seam like --ssh-keygen
    p.add_argument("--ssh-keygen", default=DEFAULT_SSH_KEYGEN)   # the fingerprint tool; a test seam, recorded in the receipt with its sha256
    p.add_argument("--shipped", default="")
    p.add_argument("--timers", default="cortex-dump.timer,cortex-dump-status.timer")
    p.add_argument("--logout-wait", type=int, default=20)
    p.add_argument("--local-machine-id", default="/etc/machine-id")
    return p.parse_args(argv)


def container_name_ok(name):
    return bool(CONTAINER_NAME.fullmatch(name)) and not name.startswith(LIVE_CONTAINER_PREFIXES)


def main(argv):
    if sys.flags.isolated != 1:
        sys.stderr.write("gate_runner: run it as: python3 -I gate_runner.py ... (a file beside it must not be imported)\n")
        return EX_USAGE
    args = parse_args(argv)
    if args.container is not None and not container_name_ok(args.container):
        sys.stderr.write("gate_runner: refused container %r: one exact plain name that is not a live container\n" % args.container)
        return EX_REFUSED_CONTAINER
    container = args.container is not None
    if container:
        if os.environ.get("CV2_ADMITTED") != "1":
            sys.stderr.write("gate_runner: a container run must be started through cv2-admit (CV2_ADMITTED is not 1)\n")
            return EX_USAGE
        if local_out(["podman", "container", "exists", args.container])[0] != 0:
            sys.stderr.write("gate_runner: no container named %r\n" % args.container)
            return EX_USAGE
    ev = os.path.abspath(args.evidence_dir)
    if ev.startswith(("/tmp/", "/var/tmp/")) or os.path.exists(ev) or not os.path.isdir(os.path.dirname(ev)):
        sys.stderr.write("gate_runner: the evidence directory must be NEW, in an existing parent, and not under /tmp\n")
        return EX_USAGE
    if (not re.fullmatch(r"[0-9]{1,5}", args.port) or not HOME_ARG.fullmatch(args.home) or not NAME_ARG.fullmatch(args.project)
            or not NAME_ARG.fullmatch(args.lead) or args.install_timeout < 1
            or not re.fullmatch(r"[a-z0-9@._-]+\.timer(,[a-z0-9@._-]+\.timer)*", args.timers) or args.logout_wait < 0
            or any(x and not re.fullmatch(SAFE_PATH, x) for x in (args.dev_tree, args.binding_record, args.shipped, args.oracle_tree, args.inventory, args.approved_key_file, args.ssh_keygen))
            or (args.inventory_approved_sha256 and not re.fullmatch(r"[0-9a-f]{64}", args.inventory_approved_sha256))):
        sys.stderr.write("gate_runner: bad --port, --home, --project, --lead or --install-timeout\n")
        return EX_USAGE
    selected = set(TABLE_ORDER)
    if args.only:
        selected = set(x.strip() for x in args.only.split(","))
        if not selected <= set(TABLE_ORDER):
            sys.stderr.write("gate_runner: --only names an unknown check\n")
            return EX_USAGE
    if not container:
        try:
            refuse_name_and_address(args.target)
        except Refused as r:
            sys.stderr.write("gate_runner: REFUSED: %s\n" % r.why)
            return r.code
    label = ("container " + args.container) if container else args.target
    started = now()
    rec = Recorder(ev)
    ctx = Ctx(PodmanExec(args.container) if container else Ssh(args.target), rec, args)
    for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        signal.signal(sig, lambda signum, frame: (_ for _ in ()).throw(Interrupted()))
    try:
        identity_probe(ctx, args.local_machine_id)
    except Refused as r:
        rec.finalize({"REFUSED.txt": ("REFUSED: %s\n" % r.why).encode()})
        sys.stderr.write("gate_runner: REFUSED: %s\n" % r.why)
        return r.code
    except Interrupted:
        rec.finalize({"INTERRUPTED.txt": b"interrupted during the identity probe\n"})
        return EX_INTERRUPTED
    if container:
        try:
            ci = ctx.local_step("00", "container_inspect", ["podman", "inspect", "--format", "{{.State.Running}} {{.ImageName}}", args.container])
        except Interrupted:
            rec.finalize({"INTERRUPTED.txt": b"interrupted while the container was read\n"})
            return EX_INTERRUPTED
        if ci.rc != 0:
            rec.finalize({"REFUSED.txt": b"REFUSED: the container cannot be inspected\n"})
            return EX_USAGE
    checks = [(g, CHECKS.get(g, not_built(g))) for g in STATE_ORDER]
    skip = {g: "not proved in a container: it needs the clean Rocky 10.2 VM" for g in CONTAINER_NOT_PROVED} if container else None
    results = execute(checks, selected, rec, ctx, skip)
    overall, code = overall_of(results, rec.interrupted)
    tsv = "".join("%s\t%s\t%s\t%s\n" % (g, results[g][0], clean(results[g][1]), ",".join(results[g][2])) for g in TABLE_ORDER)
    text = table_text(results, overall, ev, label, partial=container)
    meta = {"runner_sha256": sha256_file(os.path.abspath(sys.argv[0])), "python": sys.version.split()[0], "started": started, "finished": now(),
            "target": label, "scenario": args.scenario, "only": sorted(selected), "order_run": STATE_ORDER, "overall": overall,
            "image": {"name": args.image_name, "version": args.image_version, "checksum": args.image_checksum, "source": args.image_source},
            "project": args.project, "lead": args.lead, "port": args.port}
    extra = {}
    if container:
        extra["PARTIAL-DRY-RUN.txt"] = ("This evidence comes from a PARTIAL DRY RUN in the container %s on this host. It is not the gate.\n"
                                        "Run: G1, G2, G9, G5, G7 (without no_linger: lingering has no meaning without a real user manager) and G8.\n"
                                        "Not run: G3, G4, G6, G10, G11 (they need the clean Rocky 10.2 VM). G12 reads files, not the target.\n"
                                        "Even the checks that ran are only partly proved here: no real kernel, no real lingering, no real user manager.\n"
                                        "The real run waits for the clean VM (C-57).\n" % args.container).encode()
    if rec.leaks:
        extra["LEAK-QUARANTINE.txt"] = ("G8 found secret-shaped strings in these receipts (class, file, line; the values are NOT repeated here).\n"
                                        "Treat this evidence directory as sensitive: do not copy or attach it before the receipts are reviewed.\n"
                                        + "".join("%s %s:%d\n" % (cls, f, n) for f, cls, n in rec.leaks)).encode()
    rec.finalize({**extra, "verdict.tsv": tsv.encode(), "verdict.txt": text.encode(), "run.json": (json.dumps(meta, indent=1, sort_keys=True) + "\n").encode()})
    sys.stdout.write(text)
    return code


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
