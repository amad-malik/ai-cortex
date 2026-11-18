#!/bin/sh
# docker-compose-test.sh: the tests of M-1b, Compose for Docker Desktop on macOS as an OVERRIDE file (bob; cox rows e6aed96f and 5f3cd001,
# kai row 64dbf491). Plan: mvp/pieces/Mac-M1b-compose-docker-PLAN-r1.md.
#
#   scripts/docker-compose-test.sh [--shell sh|posix] [DC-nn ...]
#
#   --shell sh      run the shell tools under `sh` (default; /bin/sh is bash in POSIX mode on this VM)
#   --shell posix   run the shell tools under `bash --posix`
#   no ids          run every cell; with ids, only those
#
# NOTHING here is real: there is no Docker on this VM and none is ever called. A FAKE `docker` first in PATH records its argument vector and prints a
# hand-written rendered configuration (scripts/docker-compose-fixtures/rendered-good.json, the author's reading of the JSON shape of
# `docker compose config --format json`, NOT a recording). The "merged" configuration the lint checks in merge mode is the author's IMPLEMENTATION of
# the merge rules on docs.docker.com/reference/compose-file/merge/, NOT Compose's own merge. The real Docker Desktop and the real macOS are NOT RUN
# until M-6. Run it through cv2-admit: `cv2-admit scripts/docker-compose-test.sh`. Each cell builds its own directory under target/dc-test; the run
# removes those directories, and only those, by their exact names at the end.
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
lint=$here/docker-compose-lint
render=$here/docker-compose-render
fx=$here/docker-compose-fixtures
cbase=$here/../deploy/compose.yml
ovr=$here/../deploy/docker/compose.docker.yml
base=${CORTEX_DC_TMP:-"$here/../target/dc-test"}

sut=sh
want=""
while [ $# -gt 0 ]; do
  case $1 in
    --shell)
      shift
      case ${1:-} in
        sh) sut=sh ;;
        posix) sut="bash --posix" ;;
        *) echo "docker-compose-test: --shell takes sh or posix" >&2; exit 2 ;;
      esac
      ;;
    DC-*) want="$want $1" ;;
    -h | --help) sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "docker-compose-test: unknown argument $1" >&2; exit 2 ;;
  esac
  shift
done
unset CORTEX_SECRETS_DIR CORTEX_CONFIG CORTEX_DUMP_STATUS CORTEX_SERVER_IMAGE CORTEX_PORT
LC_ALL=C
export LC_ALL
mkdir -p "$base"
base=$(CDPATH= cd -P -- "$base" && pwd -P)
made=""
sdir=/Users/test/cortex/secrets
img=registry.example/cortex-server@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa

# ---- helpers ---------------------------------------------------------------------------------------------

need() { # need <message> <test args...>: a failed check names itself and fails the cell
  _m=$1
  shift
  if "$@"; then return 0; fi
  printf '    CHECK FAILED: %s\n' "$_m" >&2
  return 1
}
run() { # run <name> <command...>: status in $rc, output in $T/<name>.out and .err
  _n=$1
  shift
  rc=0
  "$@" > "$T/$_n.out" 2> "$T/$_n.err" || rc=$?
}
new_test() {
  T=$(mktemp -d "$base/t.XXXXXX")
  made="$made $T"
  mkdir -p "$T/bin"
  : > "$T/docker.log"
  fake_docker
}
cleanup_made() { # remove the directories THIS run made, by their exact names, shape-checked
  for _d in $made; do
    case $_d in "$base"/t.??????) rm -rf -- "$_d" ;; esac # s5-never: ok the exact directories this run made, shape-checked
  done
}
pj() { # pj <json file> <python expression over d>: one value of a JSON file, printed as Python prints it
  python3 -I -c 'import json,sys; d=json.load(open(sys.argv[1])); print(eval(sys.argv[2]))' "$1" "$2"
}
lintrun() { # lintrun <name> <lint args...>
  _n=$1
  shift
  run "$_n" python3 -I "$lint" "$@"
}

# The fake docker: it logs every call, fails when told to, and answers ONE call, the rendering: `compose -f A -f B config --format json`.
fake_docker() {
  cat > "$T/bin/docker" <<'FAKE'
#!/bin/sh
set -u
printf '%s\n' "docker $*" >> "${FAKE_DOCKER_LOG:?}"
if [ "${FAKE_DOCKER_FAIL:-0}" = 1 ]; then echo "fake docker: told to fail" >&2; exit 1; fi
case "$*" in
  "compose -f "*" -f "*" config --format json") cat "${FAKE_DOCKER_JSON:?}"; exit 0 ;;
  *) echo "fake docker: unexpected call: $*" >&2; exit 97 ;;
esac
FAKE
  chmod 0755 "$T/bin/docker"
}

# ---- the override file and the merge of it over the base --------------------------------------------------

dc_01() { # the real override merged over the real base is clean: hardening kept, secrets are files, images are the release image, nothing else moved
  new_test
  need "the override exists, mode 644" test "$(stat -c %a "$ovr" 2> /dev/null)" = 644 || return 1
  lintrun m --base "$cbase" --override "$ovr"
  need "the lint says OK on the real files (got $rc): $(head -n 4 "$T/m.out" | tr '\n' '|') $(head -n 2 "$T/m.err" | tr '\n' '|')" test "$rc" -eq 0 || return 1
  need "and it says so in words" grep -q '^docker-compose-lint: OK' "$T/m.out" || return 1
}
dc_02() { # the override is SMALL and says what it does: tags, no restated base line, no secret value
  new_test
  need "it uses !override for the secrets and !reset for the build" sh -c "grep -q '!override' '$ovr' && grep -q '!reset' '$ovr'" || return 1
  need "the three secrets point at CORTEX_SECRETS_DIR" test "$(grep -c 'file: \${CORTEX_SECRETS_DIR:?' "$ovr")" -eq 3 || return 1
  need "the image is the release image" test "$(grep -c 'image: \${CORTEX_SERVER_IMAGE:?' "$ovr")" -eq 2 || return 1
  need "only the server tmpfs is replaced, whole" sh -c "[ \"\$(grep -c '^[[:space:]]*tmpfs:' '$ovr')\" -eq 1 ] && grep -q '^    tmpfs: !override$' '$ovr'" || return 1
  need "no other hardening line is restated (a copy is not an override)" test -z "$(grep -E '^[[:space:]]*(cap_drop|security_opt|read_only|user|init|ports|mem_limit|cpus|healthcheck|volumes|networks|depends_on|labels):' "$ovr" || true)" || return 1
  need "no secret value and no environment block" test -z "$(grep -E '^[[:space:]]*(environment|content):|PASSWORD|hunter' "$ovr" || true)" || return 1
  need "the file is 60 lines or fewer, comments included" test "$(wc -l < "$ovr")" -le 60 || return 1
}
dc_03() { # --print-merged on the real files: the facts, read back with a second pair of eyes (plain python over the printed JSON)
  new_test
  lintrun p --base "$cbase" --override "$ovr" --print-merged
  need "--print-merged succeeds (got $rc)" test "$rc" -eq 0 || return 1
  cp "$T/p.out" "$T/merged.json"
  need "three secrets, each only a file under CORTEX_SECRETS_DIR" test "$(pj "$T/merged.json" '[(k, sorted(v.keys()), v["file"].startswith("${CORTEX_SECRETS_DIR:?")) for k, v in sorted(d["secrets"].items())]')" = "[('cv2-s5-app-password', ['file'], True), ('cv2-s5-db-superuser', ['file'], True), ('cv2-s5-owner-password', ['file'], True)]" || return 1
  need "the server has no build and the release image" test "$(pj "$T/merged.json" '("build" in d["services"]["server"], d["services"]["server"]["image"].startswith("${CORTEX_SERVER_IMAGE:?"), d["services"]["migrate"]["image"].startswith("${CORTEX_SERVER_IMAGE:?"))')" = "(False, True, True)" || return 1
  need "pull_policy never is kept on both" test "$(pj "$T/merged.json" '(d["services"]["server"]["pull_policy"], d["services"]["migrate"]["pull_policy"])')" = "('never', 'never')" || return 1
  need "the hardening lines of the base are all still there" test "$(pj "$T/merged.json" '[(s, d["services"][s]["user"], d["services"][s]["read_only"], d["services"][s]["cap_drop"], d["services"][s]["security_opt"]) for s in ("migrate", "server")]')" = "[('migrate', '10001:10001', True, ['ALL'], ['no-new-privileges:true']), ('server', '10001:10001', True, ['ALL'], ['no-new-privileges:true'])]" || return 1
  need "the :z flags of the base are kept (the docs: ignored where there is no SELinux)" test "$(pj "$T/merged.json" '[v for v in d["services"]["server"]["volumes"] if isinstance(v, str) and v.endswith(",z")].__len__()')" -ge 1 || return 1
  need "the database still publishes no port and the server publishes loopback only" test "$(pj "$T/merged.json" '("ports" in d["services"]["db"], d["services"]["server"]["ports"])')" = "(False, ['127.0.0.1:\${CORTEX_PORT:-8601}:8601'])" || return 1
}
dc_04() { # the merge engine against the documented rules (docs.docker.com/reference/compose-file/merge/), on small inputs of its own
  new_test
  cat > "$T/b.yml" <<'YML'
a: {x: 1, y: 2}
l: [1, 2]
l2: [1, 2]
m: {x: 1}
s: 1
services:
  s:
    image: i
    build: {context: .}
    command: [a, b]
    entrypoint: [e1]
    healthcheck: {test: [CMD, one], interval: 5s}
    secrets: [a, b]
    volumes: ["./x:/t:ro", "./z:/u"]
    ports: ["1:1", "2:2"]
  t:
    secrets: [{source: a, target: f}, {source: c, target: g}]
    volumes: [{type: bind, source: ./x, target: /t, read_only: true}]
    ports: ["127.0.0.1:8601:8601"]
YML
  cat > "$T/o.yml" <<'YML'
a: {y: 3, z: 4}
l: [3]
l2: !override [9]
m: !override {y: 2}
s: 2
services:
  s:
    build: !reset null
    command: [c]
    entrypoint: [e2]
    healthcheck: {test: [CMD, two]}
    secrets: [b, c]
    volumes: ["./y:/t"]
    ports: ["2:2", "3:3"]
  t:
    secrets: [{source: b, target: f}, {source: c, target: h}]
    volumes: [{source: ./y, target: /t}]
    ports: [{host_ip: 127.0.0.1, published: "8601", target: 8601, protocol: tcp}]
YML
  lintrun e --base "$T/b.yml" --override "$T/o.yml" --print-merged
  need "--print-merged on small inputs succeeds (got $rc): $(head -n 2 "$T/e.err" | tr '\n' '|')" test "$rc" -eq 0 || return 1
  cp "$T/e.out" "$T/e.json"
  need "mappings merge (missing entries added, conflicting ones merged)" test "$(pj "$T/e.json" 'd["a"]')" = "{'x': 1, 'y': 3, 'z': 4}" || return 1
  need "sequences append" test "$(pj "$T/e.json" 'd["l"]')" = "[1, 2, 3]" || return 1
  need "scalars are overridden" test "$(pj "$T/e.json" 'd["s"]')" = "2" || return 1
  need "!override replaces a sequence and a mapping instead of merging" test "$(pj "$T/e.json" '(d["l2"], d["m"])')" = "([9], {'y': 2})" || return 1
  need "!reset removes the key (it is not left as null)" test "$(pj "$T/e.json" '"build" in d["services"]["s"]')" = "False" || return 1
  need "command, entrypoint and healthcheck.test are replaced, not appended" test "$(pj "$T/e.json" '(d["services"]["s"]["command"], d["services"]["s"]["entrypoint"], d["services"]["s"]["healthcheck"]["test"])')" = "(['c'], ['e2'], ['CMD', 'two'])" || return 1
  need "the rest of the healthcheck mapping merges" test "$(pj "$T/e.json" 'd["services"]["s"]["healthcheck"]["interval"]')" = "5s" || return 1
  need "secrets merge by their unique key" test "$(pj "$T/e.json" 'd["services"]["s"]["secrets"]')" = "['a', 'b', 'c']" || return 1
  need "volumes with the same target MERGE as mappings: the override's source wins, the base's ro flag is kept (short and short)" test "$(pj "$T/e.json" 'd["services"]["s"]["volumes"] == [{"type": "bind", "source": "./y", "target": "/t", "read_only": True}, "./z:/u"]')" = "True" || return 1
  need "ports merge by their unique key: no duplicate" test "$(pj "$T/e.json" 'd["services"]["s"]["ports"]')" = "['1:1', '2:2', '3:3']" || return 1
  need "secrets: two different sources with ONE target are one entry (the identity is the target), and one source with two targets stays two" test "$(pj "$T/e.json" 'd["services"]["t"]["secrets"] == [{"source": "b", "target": "f"}, {"source": "c", "target": "g"}, {"source": "c", "target": "h"}]')" = "True" || return 1
  need "volumes: a long entry updated in source and target only keeps the inherited type and read_only" test "$(pj "$T/e.json" 'd["services"]["t"]["volumes"] == [{"type": "bind", "source": "./y", "target": "/t", "read_only": True}]')" = "True" || return 1
  need "ports: a short string and the equivalent long mapping are ONE entry" test "$(pj "$T/e.json" 'd["services"]["t"]["ports"] == [{"host_ip": "127.0.0.1", "published": "8601", "target": 8601, "protocol": "tcp"}]')" = "True" || return 1
}

# ---- an override that weakens the base is refused ---------------------------------------------------------

mk_ovr() { # mk_ovr <name> <yaml body>: an override file of this cell's own
  printf '%s\n' "$2" > "$T/ovr-$1.yml"
}
secret_ok='secrets:
  cv2-s5-db-superuser: !override
    file: ${CORTEX_SECRETS_DIR:?set by the installer}/db-superuser
  cv2-s5-owner-password: !override
    file: ${CORTEX_SECRETS_DIR:?set by the installer}/owner-password
  cv2-s5-app-password: !override
    file: ${CORTEX_SECRETS_DIR:?set by the installer}/app-password'
svc_ok='services:
  migrate:
    image: ${CORTEX_SERVER_IMAGE:?set by the installer}
  server:
    build: !reset null
    image: ${CORTEX_SERVER_IMAGE:?set by the installer}'
weak() { # weak <name> <rule id> <yaml body>: the override must make the lint exit 1 and name the rule
  mk_ovr "$1" "$3"
  awk '{ print; if ($0 == "  server:") { print "    tmpfs: !override"; print "      - /tmp:rw,noexec,nosuid,size=128m,mode=1777"; print "      - /run/cortex:rw,noexec,nosuid,size=16m,uid=10001,gid=10001,mode=0700" } }' "$T/ovr-$1.yml" > "$T/ovr-$1.with-tmpfs.yml"
  mv "$T/ovr-$1.with-tmpfs.yml" "$T/ovr-$1.yml"
  lintrun "w$1" --base "$cbase" --override "$T/ovr-$1.yml"
  need "weakening '$1': exit 1 (got $rc)" test "$rc" -eq 1 || return 1
  for _r in $(printf '%s' "$2" | tr ',' ' '); do
    need "weakening '$1': the output names $_r: $(head -n 4 "$T/w$1.out" | tr '\n' '|')" grep -q "^$_r " "$T/w$1.out" || return 1
  done
}
dc_05() { # a control, then one weakening per hardening line and per secret rule
  new_test
  mk_ovr good "$svc_ok
    tmpfs: !override
      - /tmp:rw,noexec,nosuid,size=128m,mode=1777
      - /run/cortex:rw,noexec,nosuid,size=16m,uid=10001,gid=10001,mode=0700
$secret_ok"
  lintrun good --base "$cbase" --override "$T/ovr-good.yml"
  need "the control: an override of this cell's own, shaped like the real one, passes (got $rc): $(head -n 3 "$T/good.out" | tr '\n' '|')" test "$rc" -eq 0 || return 1
  weak capdrop DCL-01,DCL-02,DCL-05 "services:
  migrate:
    image: \${CORTEX_SERVER_IMAGE:?set by the installer}
  server:
    build: !reset null
    image: \${CORTEX_SERVER_IMAGE:?set by the installer}
    cap_drop: !override []
$secret_ok" || return 1
  weak readonly DCL-01,DCL-02,DCL-05 "services:
  migrate:
    image: \${CORTEX_SERVER_IMAGE:?set by the installer}
    read_only: false
  server:
    build: !reset null
    image: \${CORTEX_SERVER_IMAGE:?set by the installer}
$secret_ok" || return 1
  weak userroot DCL-01,DCL-02,DCL-05 "services:
  migrate:
    image: \${CORTEX_SERVER_IMAGE:?set by the installer}
  server:
    build: !reset null
    image: \${CORTEX_SERVER_IMAGE:?set by the installer}
    user: root
$secret_ok" || return 1
  weak nonewpriv DCL-01,DCL-02,DCL-05 "services:
  migrate:
    image: \${CORTEX_SERVER_IMAGE:?set by the installer}
  server:
    build: !reset null
    image: \${CORTEX_SERVER_IMAGE:?set by the installer}
    security_opt: !override []
$secret_ok" || return 1
  weak dbport DCL-01,DCL-02,DCL-06 "services:
  db:
    ports: [\"5432:5432\"]
  migrate:
    image: \${CORTEX_SERVER_IMAGE:?set by the installer}
  server:
    build: !reset null
    image: \${CORTEX_SERVER_IMAGE:?set by the installer}
$secret_ok" || return 1
  weak serverports DCL-01,DCL-02,DCL-07 "services:
  migrate:
    image: \${CORTEX_SERVER_IMAGE:?set by the installer}
  server:
    build: !reset null
    image: \${CORTEX_SERVER_IMAGE:?set by the installer}
    ports: [\"0.0.0.0:8601:8601\"]
$secret_ok" || return 1
  weak extrasvc DCL-01,DCL-02 "services:
  extra:
    image: x
  migrate:
    image: \${CORTEX_SERVER_IMAGE:?set by the installer}
  server:
    build: !reset null
    image: \${CORTEX_SERVER_IMAGE:?set by the installer}
$secret_ok" || return 1
  weak envpass DCL-01,DCL-02 "services:
  migrate:
    image: \${CORTEX_SERVER_IMAGE:?set by the installer}
  server:
    build: !reset null
    image: \${CORTEX_SERVER_IMAGE:?set by the installer}
    environment: {POSTGRES_PASSWORD: hunter2}
$secret_ok" || return 1
  weak traversal DCL-03 "$svc_ok
secrets:
  cv2-s5-db-superuser: !override
    file: \${CORTEX_SECRETS_DIR:?set by the installer}/../outside/db-superuser
  cv2-s5-owner-password: !override
    file: \${CORTEX_SECRETS_DIR:?set by the installer}/owner-password
  cv2-s5-app-password: !override
    file: \${CORTEX_SECRETS_DIR:?set by the installer}/app-password" || return 1
  weak imagesuffix DCL-04 "services:
  migrate:
    image: \${CORTEX_SERVER_IMAGE:?set by the installer}-unapproved
  server:
    build: !reset null
    image: \${CORTEX_SERVER_IMAGE:?set by the installer}-unapproved
$secret_ok" || return 1
  weak imageprefix DCL-04 "services:
  migrate:
    image: registry.example/\${CORTEX_SERVER_IMAGE:?set by the installer}
  server:
    build: !reset null
    image: \${CORTEX_SERVER_IMAGE:?set by the installer}
$secret_ok" || return 1
  weak extkept DCL-03 "$svc_ok
secrets:
  cv2-s5-db-superuser: !override
    file: \${CORTEX_SECRETS_DIR:?set by the installer}/db-superuser
  cv2-s5-app-password: !override
    file: \${CORTEX_SECRETS_DIR:?set by the installer}/app-password" || return 1
  weak secenv DCL-01,DCL-03 "$svc_ok
secrets:
  cv2-s5-db-superuser: !override
    file: \${CORTEX_SECRETS_DIR:?set by the installer}/db-superuser
  cv2-s5-owner-password: !override
    file: \${CORTEX_SECRETS_DIR:?set by the installer}/owner-password
  cv2-s5-app-password: !override
    environment: APP_PW" || return 1
  weak secoutside DCL-03 "$svc_ok
secrets:
  cv2-s5-db-superuser: !override
    file: /etc/cortex/db-superuser
  cv2-s5-owner-password: !override
    file: \${CORTEX_SECRETS_DIR:?set by the installer}/owner-password
  cv2-s5-app-password: !override
    file: \${CORTEX_SECRETS_DIR:?set by the installer}/app-password" || return 1
  weak secmerge DCL-01,DCL-03 "$svc_ok
secrets:
  cv2-s5-db-superuser:
    file: \${CORTEX_SECRETS_DIR:?set by the installer}/db-superuser
  cv2-s5-owner-password: !override
    file: \${CORTEX_SECRETS_DIR:?set by the installer}/owner-password
  cv2-s5-app-password: !override
    file: \${CORTEX_SECRETS_DIR:?set by the installer}/app-password" || return 1
  weak buildkept DCL-04 "services:
  migrate:
    image: \${CORTEX_SERVER_IMAGE:?set by the installer}
  server:
    image: \${CORTEX_SERVER_IMAGE:?set by the installer}
$secret_ok" || return 1
  weak literalimage DCL-04 "services:
  migrate:
    image: \${CORTEX_SERVER_IMAGE:?set by the installer}
  server:
    build: !reset null
    image: registry.example/cortex-server:latest
$secret_ok" || return 1
  weak migrateimage DCL-04 "services:
  server:
    build: !reset null
    image: \${CORTEX_SERVER_IMAGE:?set by the installer}
$secret_ok" || return 1
  weak toplevel DCL-01 "$svc_ok
volumes:
  cv2-s5-extra: {}
$secret_ok" || return 1
  weak twopart DCL-01,DCL-02,DCL-07 "services:
  migrate:
    image: \${CORTEX_SERVER_IMAGE:?set by the installer}
  server:
    build: !reset null
    image: \${CORTEX_SERVER_IMAGE:?set by the installer}
    ports: [\"8601:8601\"]
$secret_ok" || return 1
  weak secextra DCL-01,DCL-03 "$svc_ok
secrets:
  cv2-s5-db-superuser: !override
    file: \${CORTEX_SECRETS_DIR:?set by the installer}/db-superuser
  cv2-s5-owner-password: !override
    file: \${CORTEX_SECRETS_DIR:?set by the installer}/owner-password
  cv2-s5-app-password: !override
    file: \${CORTEX_SECRETS_DIR:?set by the installer}/app-password
  cv2-s5-extra: !override
    file: \${CORTEX_SECRETS_DIR:?set by the installer}/extra" || return 1
  weak secattr DCL-01,DCL-03 "$svc_ok
secrets:
  cv2-s5-db-superuser: !override
    file: \${CORTEX_SECRETS_DIR:?set by the installer}/db-superuser
    external: true
  cv2-s5-owner-password: !override
    file: \${CORTEX_SECRETS_DIR:?set by the installer}/owner-password
  cv2-s5-app-password: !override
    file: \${CORTEX_SECRETS_DIR:?set by the installer}/app-password" || return 1
}

# ---- the lint on a RENDERED configuration (docker compose config --format json) ---------------------------

rend() { # rend <name> <json file>: the lint in rendered mode
  lintrun "$1" --rendered "$2" --secrets-dir "$sdir" --server-image "$img"
}
dc_06() { # the good rendered fixture passes, in words
  new_test
  rend g "$fx/rendered-good.json"
  need "the lint says OK on the good fixture (got $rc): $(head -n 4 "$T/g.out" | tr '\n' '|') $(head -n 2 "$T/g.err" | tr '\n' '|')" test "$rc" -eq 0 || return 1
  need "and it says so in words" grep -q '^docker-compose-lint: OK' "$T/g.out" || return 1
  # documented legal spellings and explicit defaults are the same configuration and must pass (sam: preserve legal spellings and explicit defaults)
  python3 -I - "$fx/rendered-good.json" "$T" <<'PY' || return 1
import json, sys
good = json.load(open(sys.argv[1]))
out = sys.argv[2]
def save(name, d):
    json.dump(d, open('%s/legal-%s.json' % (out, name), 'w'))
def fresh():
    return json.loads(json.dumps(good))
d = fresh()
for s in d['services'].values():
    for v in s.get('volumes', []):
        if v['type'] == 'bind':
            v['bind']['selinux'] = 'z'
save('selinux', d)
d = fresh()
for s in d['services'].values():
    for dep in (s.get('depends_on') or {}).values():
        dep['restart'] = False
        dep['required'] = True
save('depends-defaults', d)
d = fresh()
for s in d['services'].values():
    for v in s.get('volumes', []):
        if v['type'] == 'bind':
            v['bind']['create_host_path'] = False
save('create-host-path', d)
d = fresh()
for s in d['services'].values():
    for dep in (s.get('depends_on') or {}).values():
        dep['restart'] = 'false'
        dep['required'] = 'true'
save('string-flags', d)
d = fresh()
for s in d['services'].values():
    s['networks'] = {'cv2-s5-net': {}}
save('network-empty-body', d)
d = fresh()
for s in d['services'].values():
    for v in s.get('volumes', []):
        if v['type'] == 'bind':
            v['bind']['propagation'] = 'rprivate'
save('propagation-default', d)
PY
  for n in selinux depends-defaults create-host-path string-flags network-empty-body propagation-default; do
    rend "l$n" "$T/legal-$n.json"
    need "the legal spelling '$n' passes (got $rc): $(head -n 3 "$T/l$n.out" | tr '\n' '|')" test "$rc" -eq 0 || return 1
  done
}
dc_07() { # one mutation of the good fixture per rule: each must exit 1 and name the rule (100 and more variants)
  new_test
  python3 -I - "$fx/rendered-good.json" "$T" > "$T/gen.out" <<'PY' || return 1
import json, sys
good = json.load(open(sys.argv[1]))
out = sys.argv[2]
V = {
 'capdrop-server': ('DCL-05', 'del d["services"]["server"]["cap_drop"]'),
 'capdrop-empty-migrate': ('DCL-05', 'd["services"]["migrate"]["cap_drop"]=[]'),
 'nonewpriv-server': ('DCL-05', 'd["services"]["server"]["security_opt"]=[]'),
 'readonly-migrate': ('DCL-05', 'd["services"]["migrate"]["read_only"]=False'),
 'user-root': ('DCL-05', 'd["services"]["server"]["user"]="root"'),
 'user-missing': ('DCL-05', 'del d["services"]["migrate"]["user"]'),
 'db-port': ('DCL-06', 'd["services"]["db"]["ports"]=[{"host_ip":"127.0.0.1","target":5432,"published":"5432","protocol":"tcp","mode":"ingress"}]'),
 'db-unpinned': ('DCL-06', 'd["services"]["db"]["image"]="docker.io/pgvector/pgvector:0.8.2-pg18"'),
 'db-digest-changed': ('DCL-06', 'd["services"]["db"]["image"]=d["services"]["db"]["image"][:-4]+"0000"'),
 'port-all': ('DCL-07', 'd["services"]["server"]["ports"][0]["host_ip"]="0.0.0.0"'),
 'port-nohost': ('DCL-07', 'del d["services"]["server"]["ports"][0]["host_ip"]'),
 'secret-external': ('DCL-03', 'd["secrets"]["cv2-s5-app-password"]={"name":"cv2-s5-app-password","external":True}'),
 'secret-outside': ('DCL-03', 'd["secrets"]["cv2-s5-app-password"]["file"]="/etc/app-password"'),
 'secret-env': ('DCL-03', 'd["secrets"]["cv2-s5-app-password"]={"name":"cv2-s5-app-password","environment":"APP_PW"}'),
 'secret-content': ('DCL-03', 'd["secrets"]["cv2-s5-app-password"]={"name":"cv2-s5-app-password","content":"hunter2"}'),
 'secret-extra': ('DCL-03', 'd["secrets"]["cv2-s5-extra"]={"name":"cv2-s5-extra","file":"/Users/test/cortex/secrets/extra"}'),
 'secret-missing': ('DCL-03', 'del d["secrets"]["cv2-s5-owner-password"]'),
 'server-build': ('DCL-04', 'd["services"]["server"]["build"]={"context":"."}'),
 'image-differs': ('DCL-04', 'd["services"]["server"]["image"]="registry.example/other@sha256:"+"b"*64'),
 'migrate-image-differs': ('DCL-04', 'd["services"]["migrate"]["image"]="localhost/cv2-s5-server:dev"'),
 'pullpolicy': ('DCL-04', 'd["services"]["server"]["pull_policy"]="always"'),
 'pgdata-bind': ('DCL-08', 'd["services"]["db"]["volumes"][0]={"type":"bind","source":"/Users/test/pgdata","target":"/var/lib/postgresql"}'),
 'secrets-dir-mounted': ('DCL-08', 'd["services"]["server"]["volumes"].append({"type":"bind","source":"/Users/test/cortex/secrets","target":"/x"})'),
 'server-owner-secret': ('DCL-09', 'd["services"]["server"]["secrets"].append({"source":"cv2-s5-owner-password"})'),
 'migrate-no-owner': ('DCL-09', 'd["services"]["migrate"]["secrets"]=[]'),
 'healthcheck-server': ('DCL-11', 'd["services"]["server"]["healthcheck"]["test"]=["CMD","true"]'),
 'healthcheck-db': ('DCL-11', 'd["services"]["db"]["healthcheck"]["test"]=["CMD","true"]'),
 'container-name': ('DCL-10', 'd["services"]["server"]["container_name"]="cortex-api"'),  # s5-never: ok a mutation value: the lint must refuse the live v1 container name
 'project-name': ('DCL-10', 'd["name"]="cortex"'),
 'network-name': ('DCL-10', 'd["networks"]["cv2-s5-net"]["name"]="cortex-net"'),
 'service-missing': ('DCL-02', 'del d["services"]["migrate"]'),
 'service-extra': ('DCL-02', 'd["services"]["extra"]={"image":"x"}'),
 'db-no-init': ('DCL-06', 'd["services"]["db"]["init"]=False'),
 'server-no-tmpfs': ('DCL-05', 'del d["services"]["server"]["tmpfs"]'),
 'migrate-no-memlimit': ('DCL-14', 'del d["services"]["migrate"]["mem_limit"]'),
 'privileged-server': ('DCL-12', 'd["services"]["server"]["privileged"]=True'),
 'cap-add-server': ('DCL-12', 'd["services"]["server"]["cap_add"]=["SYS_ADMIN"]'),
 'devices-migrate': ('DCL-12', 'd["services"]["migrate"]["devices"]=["/dev/kmem:/dev/kmem"]'),
 'network-host-server': ('DCL-12', 'd["services"]["server"]["network_mode"]="host"'),
 'user-on-db': ('DCL-12', 'd["services"]["db"]["user"]="root"'),
 'command-db-changed': ('DCL-12', 'd["services"]["db"]["command"]=d["services"]["db"]["command"]+["-c","ssl=off"]'),
 'command-migrate-changed': ('DCL-12', 'd["services"]["migrate"]["command"]=["sh"]'),
 'restart-server': ('DCL-12', 'd["services"]["server"]["restart"]="always"'),
 'depends-on-weak': ('DCL-12', 'del d["services"]["server"]["depends_on"]["migrate"]'),
 'config-bind-writable': ('DCL-08', 'd["services"]["server"]["volumes"][0]["read_only"]=False'),
 'initdb-bind-writable': ('DCL-08', 'd["services"]["db"]["volumes"][1]["read_only"]=False'),
 'dump-status-writable': ('DCL-08', 'd["services"]["server"]["volumes"][1]["read_only"]=False'),
 'extra-mount': ('DCL-08', 'd["services"]["server"]["volumes"].append({"type":"bind","source":"/Users/test/x","target":"/x","read_only":True})'),
 'tmpfs-exec-suid': ('DCL-05', 'd["services"]["server"]["tmpfs"]=["/tmp:rw,exec,suid,size=8g"]'),
 'tmpfs-extra-line': ('DCL-05', 'd["services"]["migrate"]["tmpfs"].append("/var:rw,exec")'),
 'security-opt-extra': ('DCL-05', 'd["services"]["server"]["security_opt"]=["no-new-privileges:true","seccomp=unconfined"]'),
 'db-mem-removed': ('DCL-14', 'del d["services"]["db"]["mem_limit"]'),
 'db-mem-8g': ('DCL-14', 'd["services"]["db"]["mem_limit"]="8g"'),
 'server-cpus-8': ('DCL-14', 'd["services"]["server"]["cpus"]=8'),
 'db-shm-big': ('DCL-14', 'd["services"]["db"]["shm_size"]="4294967296"'),
 'secret-target': ('DCL-09', 'd["services"]["server"]["secrets"]=[{"source":"cv2-s5-app-password","target":"renamed"}]'),
 'secret-uid': ('DCL-09', 'd["services"]["server"]["secrets"]=[{"source":"cv2-s5-app-password","uid":"0"}]'),
 'health-disabled': ('DCL-11', 'd["services"]["server"]["healthcheck"]["disable"]=True'),
 'health-interval': ('DCL-11', 'd["services"]["db"]["healthcheck"]["interval"]="1h"'),
 'health-retries': ('DCL-11', 'd["services"]["server"]["healthcheck"]["retries"]=1'),
 'env-password-literal': ('DCL-13', 'd["services"]["server"]["environment"]["APP_PASSWORD"]="hunter2"'),
 'env-initdb-trust': ('DCL-13', 'd["services"]["db"]["environment"]["POSTGRES_INITDB_ARGS"]="--auth-host=trust"'),
 'env-db-password': ('DCL-13', 'd["services"]["db"]["environment"]["POSTGRES_PASSWORD"]="x"'),
 'env-as-list': ('DCL-13', 'd["services"]["migrate"]["environment"]=["HOME=/tmp","EXTRA=1"]'),
 'health-gone': ('DCL-11', 'del d["services"]["server"]["healthcheck"]'),
 'migrate-healthcheck': ('DCL-11', 'd["services"]["migrate"]["healthcheck"]={"test":["CMD","true"]}'),
 'mount-gone': ('DCL-08', 'd["services"]["server"]["volumes"].pop(1)'),
 'mount-duplicate': ('DCL-08', 'd["services"]["server"]["volumes"].append(dict(d["services"]["server"]["volumes"][0]))'),
 'port-extra-loopback': ('DCL-07', 'd["services"]["server"]["ports"].append({"mode":"ingress","host_ip":"127.0.0.1","target":9000,"published":"9000","protocol":"tcp"})'),
 'migrate-no-init': ('DCL-05', 'd["services"]["migrate"]["init"]=False'),
 'db-no-shm': ('DCL-06', 'del d["services"]["db"]["shm_size"]'),
 'volume-extra': ('DCL-08', 'd["volumes"]["cv2-s5-extra"]={"name":"cv2-s5-extra"}'),
 'container-name-plain': ('DCL-10', 'd["services"]["server"]["container_name"]="other-server"'),
 'v1-name-in-env': ('DCL-10', 'd["services"]["server"]["environment"]={"X":"harness-appdb"}'),  # s5-never: ok a mutation value: the lint must refuse a live v1 name anywhere
 'port-two-part': ('DCL-07', 'd["services"]["server"]["ports"]=["8601:8601"]'),
 'secret-no-file': ('DCL-03', 'd["secrets"]["cv2-s5-app-password"]={"name":"cv2-s5-app-password"}'),
 'secret-extra-attr': ('DCL-03', 'd["secrets"]["cv2-s5-app-password"]["labels"]={"x":"y"}'),
 'labels-changed': ('DCL-12', 'd["services"]["server"]["labels"]={"cortexv2":"prod"}'),
 'labels-extra': ('DCL-12', 'd["services"]["db"]["labels"]["x"]="y"'),
 'labels-removed': ('DCL-12', 'del d["services"]["migrate"]["labels"]'),
 'networks-extra': ('DCL-12', 'd["services"]["server"]["networks"]["cv2-other"]=None'),
 'networks-dropped': ('DCL-12', 'del d["services"]["db"]["networks"]'),
 'secrets-file-mounted': ('DCL-08', 'd["services"]["migrate"]["volumes"].append({"type":"bind","source":"/Users/test/cortex/secrets/owner-password","target":"/y"})'),
 'volume-driver-opts': ('DCL-08', 'd["volumes"]["cv2-s5-pg-data"].update({"driver":"local","driver_opts":{"type":"none","o":"bind","device":"/Users/test/cortex/pgdata"}})'),
 'volume-labels': ('DCL-08', 'd["volumes"]["cv2-s5-state"]["labels"]={"x":"y"}'),
 'volume-name-differs': ('DCL-10', 'd["volumes"]["cv2-s5-state"]["name"]="cv2-s5-other"'),
 'volume-external-flag': ('DCL-08', 'd["volumes"]["cv2-s5-state"]["external"]=True'),
 'network-driver': ('DCL-12', 'd["networks"]["cv2-s5-net"]["driver"]="host"'),
 'network-external-flag': ('DCL-12', 'd["networks"]["cv2-s5-net"]["external"]=True'),
 'network-extra-defined': ('DCL-12', 'd["networks"]["cv2-s5-extra"]={"name":"cv2-s5-extra"}'),
 'depends-required-false': ('DCL-12', 'd["services"]["server"]["depends_on"]["migrate"]["required"]=False'),
 'depends-restart': ('DCL-12', 'd["services"]["server"]["depends_on"]["db"]["restart"]=True'),
 'bind-ancestor-secrets': ('DCL-08', 'd["services"]["server"]["volumes"][1]["source"]="/Users/test/cortex"'),
 'bind-ancestor-home': ('DCL-08', 'd["services"]["server"]["volumes"][1]["source"]="/Users/test"'),
 'bind-ancestor-root': ('DCL-08', 'd["services"]["server"]["volumes"][1]["source"]="/"'),
 'bind-propagation': ('DCL-08', 'd["services"]["server"]["volumes"][0]["bind"]["propagation"]="rshared"'),
 'volume-option-nocopy': ('DCL-08', 'd["services"]["server"]["volumes"][2]["volume"]={"nocopy":True}'),
 'toplevel-configs': ('DCL-12', 'd["configs"]={"x":{"file":"/etc/hostname"}}'),
 'toplevel-extension': ('DCL-12', 'd["x-anything"]={"a":1}'),
 'depends-unknown-key': ('DCL-12', 'd["services"]["server"]["depends_on"]["db"]["extra_key"]=True'),
 'depends-required-string-false': ('DCL-12', 'd["services"]["server"]["depends_on"]["migrate"]["required"]="false"'),
 'mount-unknown-key': ('DCL-08', 'd["services"]["server"]["volumes"][0]["consistency"]="cached"'),
 'bind-selinux-odd': ('DCL-08', 'd["services"]["server"]["volumes"][0]["bind"]["selinux"]="x"'),
 'network-alias': ('DCL-12', 'd["services"]["server"]["networks"]["cv2-s5-net"]={"aliases":["an-alias"]}'),
 'network-address': ('DCL-12', 'd["services"]["db"]["networks"]["cv2-s5-net"]={"ipv4_address":"172.20.0.9"}'),
}
index = []
for name, (rule, code) in sorted(V.items()):
    d = json.loads(json.dumps(good))
    exec(code)
    json.dump(d, open('%s/bad-%s.json' % (out, name), 'w'))
    index.append('%s %s' % (name, rule))
open('%s/variants.txt' % out, 'w').write('\n'.join(index) + '\n')
print(len(index))
PY
  n=0
  while read -r name rule; do
    n=$((n + 1))
    rend "v$n" "$T/bad-$name.json"
    need "variant '$name': exit 1 (got $rc)" test "$rc" -eq 1 || return 1
    need "variant '$name': the output names $rule: $(head -n 3 "$T/v$n.out" | tr '\n' '|')" grep -q "^$rule " "$T/v$n.out" || return 1
    need "variant '$name': it ends with FAILED" grep -q '^docker-compose-lint: FAILED' "$T/v$n.out" || return 1
  done < "$T/variants.txt"
  need "every variant the generator wrote ran (ran $n, generator said $(cat "$T/gen.out"))" test "$n" -eq "$(cat "$T/gen.out")" -a "$n" -ge 100 || return 1
  # the volume model also reports DCL-08 for ANY extra mount, so the secrets rule's own sentence is asked for: a mount of the secrets directory, of a file in it, or of a directory ABOVE it
  for name in secrets-dir-mounted secrets-file-mounted bind-ancestor-secrets bind-ancestor-home bind-ancestor-root; do
    rend "m$name" "$T/bad-$name.json"
    need "variant '$name': the secrets rule itself speaks: $(head -n 4 "$T/m$name.out" | tr '\n' '|')" grep -q 'the secrets directory is mounted into a container' "$T/m$name.out" || return 1
  done
}
dc_08() { # usage errors: exit 2 and a word about it, never a traceback
  new_test
  lintrun u1
  need "no arguments: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  need "and the message is a usage line, not python's own complaint about a missing file" grep -q '^docker-compose-lint: usage' "$T/u1.err" || return 1
  lintrun u2 --rendered "$T/missing.json" --secrets-dir "$sdir" --server-image "$img"
  need "a missing file: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  need "and it names the file" grep -q 'missing.json' "$T/u2.err" || return 1
  printf 'not json {' > "$T/broken.json"
  rend u3 "$T/broken.json"
  need "a file that is not JSON: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  lintrun u4 --rendered "$fx/rendered-good.json" --secrets-dir relative/dir --server-image "$img"
  need "a relative --secrets-dir: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  need "none of them printed a traceback" test -z "$(cat "$T"/u1.err "$T"/u2.err "$T"/u3.err "$T"/u4.err | grep -i traceback || true)" || return 1
}

# ---- the render script, against the fake docker -----------------------------------------------------------

renv() { # renv <name> <extra env pairs...> -- <render args...>   (TMPDIR is this cell's own, so a leftover temporary file shows)
  _n=$1
  shift
  _e=""
  while [ "$1" != -- ]; do
    _e="$_e $1"
    shift
  done
  shift
  mkdir -p "$T/tmp"
  run "$_n" env PATH="$T/bin:$PATH" TMPDIR="$T/tmp" FAKE_DOCKER_LOG="$T/docker.log" FAKE_DOCKER_JSON="${FAKE_JSON:-$fx/rendered-good.json}" \
    CORTEX_SECRETS_DIR=$sdir CORTEX_CONFIG=/Users/test/cortex/cortex.toml CORTEX_DUMP_STATUS=/Users/test/cortex/dump-status CORTEX_SERVER_IMAGE=$img $_e \
    $sut "$render" "$@"
}
dc_09() { # the render script: the exact docker call, the JSON passed through, --out, --lint
  new_test
  renv r1 -- --base "$cbase" --override "$ovr"
  need "it succeeds (got $rc): $(head -n 2 "$T/r1.err" | tr '\n' '|')" test "$rc" -eq 0 || return 1
  need "exactly one docker call, the documented rendering, base first and override second" test "$(cat "$T/docker.log")" = "docker compose -f $cbase -f $ovr config --format json" || { cat "$T/docker.log" >&2; return 1; }
  need "the JSON is passed through unchanged" cmp -s "$T/r1.out" "$fx/rendered-good.json" || return 1
  need "no temporary file is left behind ($(ls -A "$T/tmp" | tr '\n' ' '))" test -z "$(ls -A "$T/tmp")" || return 1
  : > "$T/docker.log"
  renv r2 -- --base "$cbase" --override "$ovr" --out "$T/rendered.json"
  need "--out succeeds (got $rc)" test "$rc" -eq 0 || return 1
  need "and writes the file (mode 600), nothing on stdout" sh -c "cmp -s '$T/rendered.json' '$fx/rendered-good.json' && [ \"\$(stat -c %a '$T/rendered.json')\" = 600 ] && [ ! -s '$T/r2.out' ]" || return 1
  renv r3 -- --base "$cbase" --override "$ovr" --lint
  need "--lint on the good rendering succeeds (got $rc): $(head -n 3 "$T/r3.err" | tr '\n' '|')" test "$rc" -eq 0 || return 1
  need "and says the lint is OK" grep -q 'docker-compose-lint: OK' "$T/r3.out" "$T/r3.err" || return 1
  need "and nothing is left in the temporary directory after the lint run either ($(ls -A "$T/tmp" | tr '\n' ' '))" test -z "$(ls -A "$T/tmp")" || return 1
  need "--lint does not print the rendering itself" test "$(grep -c '"services"' "$T/r3.out" || true)" -eq 0 || return 1
  python3 -I -c 'import json,sys; d=json.load(open(sys.argv[1])); del d["services"]["server"]["cap_drop"]; json.dump(d, open(sys.argv[2], "w"))' "$fx/rendered-good.json" "$T/bad.json" || return 1
  FAKE_JSON=$T/bad.json renv r4 -- --base "$cbase" --override "$ovr" --lint
  need "--lint on a rendering that dropped cap_drop fails with exit 1 (got $rc)" test "$rc" -eq 1 || return 1
  need "and names the rule" grep -q 'DCL-05' "$T/r4.out" "$T/r4.err" || return 1
  python3 -I -c 'import json,sys; d=json.load(open(sys.argv[1])); d["services"]["server"]["privileged"]=True; json.dump(d, open(sys.argv[2], "w"))' "$fx/rendered-good.json" "$T/priv.json" || return 1
  FAKE_JSON=$T/priv.json renv r5 -- --base "$cbase" --override "$ovr" --lint
  need "--lint on a rendering with privileged: true fails with exit 1 (got $rc)" test "$rc" -eq 1 || return 1
  need "and names DCL-12" grep -q 'DCL-12' "$T/r5.out" "$T/r5.err" || return 1
}
dc_10() { # refusals and failures: exit 2 with problem and fix, nothing called; docker failing is exit 1 and leaves no --out
  new_test
  for v in CORTEX_SECRETS_DIR CORTEX_CONFIG CORTEX_DUMP_STATUS CORTEX_SERVER_IMAGE; do
    : > "$T/docker.log"
    run "e$v" env PATH="$T/bin:$PATH" FAKE_DOCKER_LOG="$T/docker.log" FAKE_DOCKER_JSON="$fx/rendered-good.json" \
      CORTEX_SECRETS_DIR=$sdir CORTEX_CONFIG=/Users/test/cortex/cortex.toml CORTEX_DUMP_STATUS=/Users/test/cortex/dump-status CORTEX_SERVER_IMAGE=$img "$v=" \
      $sut "$render" --base "$cbase" --override "$ovr"
    need "without $v: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
    need "without $v: the message names it, with problem: and fix:" sh -c "grep -q 'problem:' '$T/e$v.err' && grep -q 'fix:' '$T/e$v.err' && grep -q '$v' '$T/e$v.err'" || return 1
    need "without $v: docker was never called" test ! -s "$T/docker.log" || return 1
  done
  : > "$T/docker.log"
  renv rel CORTEX_SECRETS_DIR=relative/dir -- --base "$cbase" --override "$ovr"
  need "a relative CORTEX_SECRETS_DIR: exit 2 (got $rc) and docker not called" test "$rc" -eq 2 -a ! -s "$T/docker.log" || return 1
  renv nof -- --base "$T/none.yml" --override "$ovr"
  need "a missing base file: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  renv nou -- --base "$cbase"
  need "no --override: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  mkdir -p "$T/nodocker"
  for t in sh bash env cat sed grep mv cp mkdir chmod stat cmp tr ls head dirname rm mktemp printf python3 umask; do
    p=$(command -v "$t" 2> /dev/null || true)
    [ -n "$p" ] && ln -sf "$p" "$T/nodocker/$t"
  done
  run nd env PATH="$T/nodocker" CORTEX_SECRETS_DIR=$sdir CORTEX_CONFIG=/Users/test/cortex/cortex.toml CORTEX_DUMP_STATUS=/Users/test/cortex/dump-status CORTEX_SERVER_IMAGE=$img $sut "$render" --base "$cbase" --override "$ovr"
  need "no docker on PATH: exit 2 (got $rc) with problem and fix" sh -c "[ $rc -eq 2 ] && grep -q 'problem:' '$T/nd.err' && grep -q 'fix:' '$T/nd.err' && grep -qi docker '$T/nd.err'" || return 1
  : > "$T/docker.log"
  rm -f "$T/never.json"
  renv df FAKE_DOCKER_FAIL=1 -- --base "$cbase" --override "$ovr" --out "$T/never.json"
  need "docker failing: exit 1 (got $rc)" test "$rc" -eq 1 || return 1
  need "and no --out file is left" test ! -e "$T/never.json" || return 1
  need "the message names a problem and a fix" sh -c "grep -q 'problem:' '$T/df.err' && grep -q 'fix:' '$T/df.err'" || return 1
}

dc_14() { # --out: a unique 0600 staging file beside the destination, the destination checked, published whole or not at all
  new_test
  out=$T/o/rendered.json
  mkdir -p "$T/o"
  # (a) an existing 0644 output is replaced by a 0600 one
  printf 'old\n' > "$out"
  chmod 0644 "$out"
  renv a -- --base "$cbase" --override "$ovr" --out "$out"
  need "(a) an existing 0644 output: the run succeeds (got $rc)" test "$rc" -eq 0 || return 1
  need "(a) and the file is 0600 with the rendering (mode $(stat -c %a "$out"))" sh -c "[ \"\$(stat -c %a '$out')\" = 600 ] && cmp -s '$out' '$fx/rendered-good.json'" || return 1
  need "(a) nothing else is left beside it ($(ls -A "$T/o" | tr '\n' ' '))" test "$(ls -A "$T/o")" = rendered.json || return 1
  # (b) the staging name is not predictable: a mktemp of the destination's own directory (a stand-in mktemp logs its template)
  mkdir -p "$T/mkbin"
  printf '#!/bin/sh\nprintf "%%s\\n" "$*" >> "%s"\nexec %s "$@"\n' "$T/mktemp.log" "$(command -v mktemp)" > "$T/mkbin/mktemp"
  chmod 0755 "$T/mkbin/mktemp"
  : > "$T/mktemp.log"
  rm -f "$out"
  run b env PATH="$T/mkbin:$T/bin:$PATH" TMPDIR="$T/tmp" FAKE_DOCKER_LOG="$T/docker.log" FAKE_DOCKER_JSON="$fx/rendered-good.json" CORTEX_SECRETS_DIR=$sdir CORTEX_CONFIG=/Users/test/cortex/cortex.toml CORTEX_DUMP_STATUS=/Users/test/cortex/dump-status CORTEX_SERVER_IMAGE=$img $sut "$render" --base "$cbase" --override "$ovr" --out "$out"
  need "(b) the run succeeds (got $rc)" test "$rc" -eq 0 || return 1
  need "(b) one staging file was made with a random template in the destination directory ($(tr '\n' '|' < "$T/mktemp.log"))" grep -q "^$T/o/rendered.json.XXXXXX\$" "$T/mktemp.log" || return 1
  # (c) an existing directory as --out is refused and nothing is put inside it
  mkdir -p "$T/d"
  renv c -- --base "$cbase" --override "$ovr" --out "$T/d"
  need "(c) a directory as --out: exit 2 (got $rc) with problem and fix" sh -c "[ $rc -eq 2 ] && grep -q 'problem:' '$T/c.err' && grep -q 'fix:' '$T/c.err'" || return 1
  need "(c) the message says it is a directory and what to give instead" sh -c "grep -q 'is a directory' '$T/c.err' && grep -q 'rendered.json' '$T/c.err'" || return 1
  need "(c) and nothing was put inside it ($(ls -A "$T/d" | tr '\n' ' '))" test -z "$(ls -A "$T/d")" || return 1
  # (d) a symbolic link as --out is refused and its target untouched
  printf 'precious\n' > "$T/target.txt"
  ln -s "$T/target.txt" "$T/o/link.json"
  renv d -- --base "$cbase" --override "$ovr" --out "$T/o/link.json"
  need "(d) a symbolic link as --out: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  need "(d) the link is still a link and its target is untouched" sh -c "[ -L '$T/o/link.json' ] && [ \"\$(cat '$T/target.txt')\" = precious ]" || return 1
  # (e) a missing destination directory is refused
  renv e -- --base "$cbase" --override "$ovr" --out "$T/nodir/x.json"
  need "(e) a missing directory: exit 2 (got $rc) and nothing created" test "$rc" -eq 2 -a ! -e "$T/nodir" || return 1
  # (f) a decoy beside the destination is never touched, and a failing lint publishes nothing and leaves no staging file
  printf 'decoy\n' > "$T/o/rendered.json.decoy"
  rm -f "$out"
  FAKE_JSON=$T/priv.json
  python3 -I -c 'import json,sys; d=json.load(open(sys.argv[1])); d["services"]["server"]["privileged"]=True; json.dump(d, open(sys.argv[2], "w"))' "$fx/rendered-good.json" "$T/priv.json" || return 1
  renv f -- --base "$cbase" --override "$ovr" --out "$out" --lint
  FAKE_JSON=""
  need "(f) a failing lint: exit 1 (got $rc)" test "$rc" -eq 1 || return 1
  need "(f) nothing was published and the decoy is untouched ($(ls -A "$T/o" | tr '\n' ' '))" test "$(ls -A "$T/o" | tr '\n' ' ')" = "link.json rendered.json.decoy " || return 1
  need "(f) the decoy kept its content" test "$(cat "$T/o/rendered.json.decoy")" = decoy || return 1
}

dc_15() { # sam M1B-SAM-01 (second part): the effective model keeps nested policy, in merge mode (a weakened base) and through the wrapper
  new_test
  python3 -I - "$cbase" "$T" > "$T/gen.out" <<'PY' || return 1
import sys, yaml
base = yaml.safe_load(open(sys.argv[1]))
out = sys.argv[2]
V = {
 'control': ('OK', 'pass'),
 'vol-driver-opts': ('DCL-08', 'd["volumes"]["cv2-s5-pg-data"].update({"driver": "local", "driver_opts": {"type": "none", "o": "bind", "device": "/Users/test/cortex/pgdata"}})'),
 'dep-optional': ('DCL-12', 'd["services"]["server"]["depends_on"]["migrate"]["required"] = False'),
 'dep-restart': ('DCL-12', 'd["services"]["migrate"]["depends_on"]["db"]["restart"] = True'),
 'net-driver': ('DCL-12', 'd["networks"]["cv2-s5-net"]["driver"] = "host"'),
 'top-extra': ('DCL-12', 'd["configs"] = {"x": {"file": "/etc/hostname"}}'),
 'vol-labels': ('DCL-08', 'd["volumes"]["cv2-s5-state"]["labels"] = {"x": "y"}'),
 'short-rshared': ('DCL-08', 'v = d["services"]["server"]["volumes"]; i = [k for k, x in enumerate(v) if "cortex.toml" in x][0]; v[i] = v[i] + ",rshared"'),
 'short-nocopy': ('DCL-08', 'd["services"]["db"]["volumes"][0] = d["services"]["db"]["volumes"][0] + ":nocopy"'),
 'short-slave': ('DCL-08', 'd["services"]["db"]["volumes"][1] = d["services"]["db"]["volumes"][1] + ",slave"'),
 'short-unknown-flag': ('DCL-08', 'v = d["services"]["server"]["volumes"]; i = [k for k, x in enumerate(v) if "cortex.toml" in x][0]; v[i] = v[i] + ",bogusflag"'),
 'short-cached': ('DCL-08', 'v = d["services"]["server"]["volumes"]; i = [k for k, x in enumerate(v) if "cortex.toml" in x][0]; v[i] = v[i] + ",cached"'),
 'short-prop-conflict-a': ('DCL-08', 'v = d["services"]["server"]["volumes"]; i = [k for k, x in enumerate(v) if "cortex.toml" in x][0]; v[i] = v[i] + ",rprivate,rshared"'),
 'short-prop-conflict-b': ('DCL-08', 'v = d["services"]["server"]["volumes"]; i = [k for k, x in enumerate(v) if "cortex.toml" in x][0]; v[i] = v[i] + ",rshared,rprivate"'),
 'short-prop-conflict-c': ('DCL-08', 'v = d["services"]["server"]["volumes"]; i = [k for k, x in enumerate(v) if "cortex.toml" in x][0]; v[i] = v[i] + ",rprivate,rslave"'),
 'short-selinux-conflict': ('DCL-08', 'v = d["services"]["server"]["volumes"]; i = [k for k, x in enumerate(v) if "cortex.toml" in x][0]; v[i] = v[i].replace(",z", ",z,Z")'),
 'legal-short-prop-dup': ('OK', 'v = d["services"]["server"]["volumes"]; i = [k for k, x in enumerate(v) if "cortex.toml" in x][0]; v[i] = v[i] + ",rprivate,rprivate"'),
 'port-extra-part': ('DCL-07', 'd["services"]["server"]["ports"][0] = d["services"]["server"]["ports"][0] + ":extra"'),
 'short-extra-part': ('DCL-08', 'v = d["services"]["server"]["volumes"]; i = [k for k, x in enumerate(v) if "cortex.toml" in x][0]; v[i] = v[i] + ":extra"'),
 'long-rshared': ('DCL-08', 'v = d["services"]["server"]["volumes"]; i = [k for k, x in enumerate(v) if "cortex.toml" in x][0]; v[i] = {"type": "bind", "source": "${CORTEX_CONFIG:?set by install.sh}", "target": "/etc/cortex/cortex.toml", "read_only": True, "bind": {"propagation": "rshared"}}'),
 'long-nocopy': ('DCL-08', 'd["services"]["db"]["volumes"][0] = {"type": "volume", "source": "cv2-s5-pg-data", "target": "/var/lib/postgresql", "volume": {"nocopy": True}}'),
 'legal-short-rprivate': ('OK', 'v = d["services"]["server"]["volumes"]; i = [k for k, x in enumerate(v) if "cortex.toml" in x][0]; v[i] = v[i] + ",rprivate"'),
 'legal-short-rw': ('OK', 'd["services"]["db"]["volumes"][0] = d["services"]["db"]["volumes"][0] + ":rw"'),
 'legal-short-upper-z': ('OK', 'v = d["services"]["server"]["volumes"]; i = [k for k, x in enumerate(v) if "cortex.toml" in x][0]; v[i] = v[i].replace(",z", ",Z")'),
 'legal-long-rprivate': ('OK', 'v = d["services"]["server"]["volumes"]; i = [k for k, x in enumerate(v) if "cortex.toml" in x][0]; v[i] = {"type": "bind", "source": "${CORTEX_CONFIG:?set by install.sh}", "target": "/etc/cortex/cortex.toml", "read_only": True, "bind": {"propagation": "rprivate"}}'),
}
index = []
for name, (rule, code) in sorted(V.items()):
    d = yaml.safe_load(yaml.safe_dump(base))
    exec(code)
    yaml.safe_dump(d, open('%s/base-%s.yml' % (out, name), 'w'), sort_keys=False)
    index.append('%s %s' % (name, rule))
open('%s/bases.txt' % out, 'w').write('\n'.join(index) + '\n')
PY
  n=0
  while read -r name rule; do
    n=$((n + 1))
    lintrun "b$n" --base "$T/base-$name.yml" --override "$ovr"
    if [ "$rule" = OK ]; then
      need "base variant '$name' (a plain round trip of the real base) passes (got $rc): $(head -n 3 "$T/b$n.out" | tr '\n' '|')" test "$rc" -eq 0 || return 1
      continue
    fi
    need "base variant '$name': exit 1 (got $rc)" test "$rc" -eq 1 || return 1
    need "base variant '$name': the output names $rule: $(head -n 3 "$T/b$n.out" | tr '\n' '|')" grep -q "^$rule " "$T/b$n.out" || return 1
  done < "$T/bases.txt"
  # the wrapper's --lint rejects a rendering whose status mount is a directory ABOVE the secrets directory
  python3 -I -c 'import json,sys; d=json.load(open(sys.argv[1])); d["services"]["server"]["volumes"][1]["source"]="/Users/test/cortex"; json.dump(d, open(sys.argv[2], "w"))' "$fx/rendered-good.json" "$T/anc.json" || return 1
  FAKE_JSON=$T/anc.json renv w1 -- --base "$cbase" --override "$ovr" --lint
  need "the wrapper's --lint rejects the ancestor bind: exit 1 (got $rc)" test "$rc" -eq 1 || return 1
  need "and names DCL-08 and says the secrets directory is mounted" sh -c "grep -q '^DCL-08 ' '$T/w1.out' '$T/w1.err' && grep -q 'the secrets directory is mounted into a container' '$T/w1.out' '$T/w1.err'" || return 1
  python3 -I -c 'import json,sys; d=json.load(open(sys.argv[1])); d["volumes"]["cv2-s5-pg-data"]["driver_opts"]={"type":"none","o":"bind","device":"/Users/test/cortex/pgdata"}; json.dump(d, open(sys.argv[2], "w"))' "$fx/rendered-good.json" "$T/vol.json" || return 1
  FAKE_JSON=$T/vol.json renv w2 -- --base "$cbase" --override "$ovr" --lint
  need "the wrapper's --lint rejects a named volume that is a host bind: exit 1 (got $rc) and DCL-08" sh -c "[ $rc -eq 1 ] && grep -q '^DCL-08 ' '$T/w2.out' '$T/w2.err'" || return 1
}

# ---- one place for the secret rules, and what must never be in these files ----------------------------------

dc_16() { # the only new Docker exception is a whole-list server tmpfs replacement
  new_test
  mk_ovr tmpfs "$svc_ok
    tmpfs: !override
      - /tmp:rw,noexec,nosuid,size=128m,mode=1777
      - /run/cortex:rw,noexec,nosuid,size=16m,uid=10001,gid=10001,mode=0700
$secret_ok"
  lintrun replacement --base "$cbase" --override "$T/ovr-tmpfs.yml"
  need "the narrow whole-list replacement passes: $(head -n 4 "$T/replacement.out" | tr '\n' '|')" test "$rc" -eq 0 || return 1
  sed 's/tmpfs: !override/tmpfs:/' "$T/ovr-tmpfs.yml" > "$T/ovr-plain.yml"
  lintrun plain --base "$cbase" --override "$T/ovr-plain.yml"
  need "an untagged list is rejected" sh -c "[ $rc -eq 1 ] && grep -q '^DCL-01 ' '$T/plain.out'" || return 1
}

dc_17() { # a replacement must not hide changes to the pinned Linux base
  new_test
  mk_ovr tmpfs "$svc_ok
    tmpfs: !override
      - /tmp:rw,noexec,nosuid,size=128m,mode=1777
      - /run/cortex:rw,noexec,nosuid,size=16m,uid=10001,gid=10001,mode=0700
$secret_ok"
  python3 -I - "$cbase" "$T/base-changed.yml" <<'PY' || return 1
import sys, yaml
with open(sys.argv[1]) as f:
    d = yaml.safe_load(f)
d['services']['server']['tmpfs'][0] = '/tmp:rw,noexec,nosuid,size=256m,mode=1777'
with open(sys.argv[2], 'w') as f:
    yaml.safe_dump(d, f, sort_keys=False)
PY
  lintrun changed --base "$T/base-changed.yml" --override "$T/ovr-tmpfs.yml"
  need "a changed base is identified even when the override hides it" sh -c "[ $rc -eq 1 ] && grep -q '^DCL-02 .*pinned base tmpfs' '$T/changed.out'" || return 1
}

dc_18() { # the render wrapper must refuse every weakened effective tmpfs
  new_test
  python3 -I - "$fx/rendered-good.json" "$T" <<'PY' || return 1
import copy, json, sys
with open(sys.argv[1]) as f:
    good = json.load(f)
out = sys.argv[2]
variants = {
    'U': lambda a: a.__setitem__(1, a[1] + ',U'),
    'missing-uid': lambda a: a.__setitem__(1, a[1].replace(',uid=10001', '')),
    'wrong-uid': lambda a: a.__setitem__(1, a[1].replace('uid=10001', 'uid=0')),
    'missing-gid': lambda a: a.__setitem__(1, a[1].replace(',gid=10001', '')),
    'wrong-gid': lambda a: a.__setitem__(1, a[1].replace('gid=10001', 'gid=0')),
    'mode': lambda a: a.__setitem__(1, a[1].replace('mode=0700', 'mode=0777')),
    'size': lambda a: a.__setitem__(1, a[1].replace('size=16m', 'size=32m')),
    'flag': lambda a: a.__setitem__(1, a[1].replace('noexec', 'exec')),
    'missing': lambda a: a.pop(0),
    'duplicate': lambda a: a.append(a[1]),
    'extra': lambda a: a.append('/extra:rw'),
}
for name, change in variants.items():
    d = copy.deepcopy(good)
    change(d['services']['server']['tmpfs'])
    with open('%s/%s.json' % (out, name), 'w') as f:
        json.dump(d, f)
PY
  renv control -- --base "$cbase" --override "$ovr" --lint
  need "the good rendering passes first" test "$rc" -eq 0 || return 1
  for variant in U missing-uid wrong-uid missing-gid wrong-gid mode size flag missing duplicate extra; do
    FAKE_JSON=$T/$variant.json renv "bad-$variant" -- --base "$cbase" --override "$ovr" --lint
    need "rendered $variant is refused as DCL-05" sh -c "[ $rc -eq 1 ] && grep -q '^DCL-05 ' '$T/bad-$variant.out'" || return 1
  done
}

dc_11() { # cox: keep the lint's secret rules in ONE place, so a change to option (B) is a small edit
  new_test
  rc=0
  python3 -I - "$lint" > "$T/one.out" 2>&1 <<'PY' || rc=$?
import ast, sys
src = open(sys.argv[1]).read()
tree = ast.parse(src)
fns = [n for n in ast.walk(tree) if isinstance(n, ast.FunctionDef) and n.name == 'check_secrets']
assert len(fns) == 1, 'check_secrets must exist exactly once, found %d' % len(fns)
inside = set()
for n in ast.walk(fns[0]):
    inside.add(id(n))
bad = []
for n in ast.walk(tree):
    if isinstance(n, ast.Constant) and isinstance(n.value, str) and id(n) not in inside:
        if 'CORTEX_SECRETS_DIR' in n.value or n.value in ('external', 'content'):  # 'environment' is also a SERVICE key, used by the service checks
            bad.append((n.lineno, n.value))
if bad:
    print('secret-rule strings outside check_secrets:', bad)
    sys.exit(1)
PY
  need "the secret rules (CORTEX_SECRETS_DIR, external, content) are named only inside check_secrets: $(tail -n 2 "$T/one.out" | tr '\n' '|')" test "$rc" -eq 0 || return 1
}
dc_12() { # no systemd tool, no swarm, no volume deletion, no real docker verbs other than compose config, in the new files
  new_test
  bad=""
  for f in "$render" "$lint" "$ovr"; do
    [ -f "$f" ] || { bad="$bad missing:$f"; continue; }
    hits=$(grep -vE '^[[:space:]]*#' "$f" | grep -E '(^|[;&|({]|then|do|exec|command)[[:space:]]*(systemctl|systemd-run|systemd-analyze|journalctl|loginctl)([[:space:]]|$)|docker[[:space:]]+(swarm|stack|secret|system|rm|volume|kill|run|exec)|compose[[:space:]]+(down|rm)|[[:space:]]-v([[:space:]]|$).*compose' || true)
    bad="$bad$hits"
  done
  need "none found: $bad" test -z "$bad" || return 1
  need "the render script calls docker exactly once, with compose ... config" test "$(grep -vE '^[[:space:]]*#' "$render" | grep -c 'docker compose' || true)" -eq 1 || return 1
}
dc_13() { # shell syntax under sh and bash --posix, python parses, modes and shebangs, no bashisms in the render script
  new_test
  for f in "$render" "$here/docker-compose-test.sh"; do
    need "$f exists" test -f "$f" || return 1
    need "sh -n $f" sh -n "$f" || return 1
    need "bash --posix -n $f" bash --posix -n "$f" || return 1
    need "$f starts with #!/bin/sh" test "$(head -n 1 "$f")" = "#!/bin/sh" || return 1
  done
  need "the lint exists and parses" python3 -I -c 'import ast,sys; ast.parse(open(sys.argv[1]).read())' "$lint" || return 1
  need "the lint starts with a python3 shebang" test "$(head -n 1 "$lint")" = "#!/usr/bin/env python3" || return 1
  need "mode 0755 on the three scripts" sh -c "[ \"\$(stat -c %a '$render')\" = 755 ] && [ \"\$(stat -c %a '$lint')\" = 755 ] && [ \"\$(stat -c %a '$here/docker-compose-test.sh')\" = 755 ]" || return 1
  hits=$(grep -vE '^[[:space:]]*#' "$render" | grep -E '\[\[|\]\]|pipefail|(^|[;&|({])[[:space:]]*local[[:space:]]|<\(|>\(|echo -e|&>|(^|[;&|({])[[:space:]]*source[[:space:]]|=\(' || true)
  need "no bashism in the render script: $hits" test -z "$hits" || return 1
}

# ---- run -------------------------------------------------------------------------------------------------

all_ids="DC-01 DC-02 DC-03 DC-04 DC-05 DC-06 DC-07 DC-08 DC-09 DC-10 DC-14 DC-15 DC-16 DC-17 DC-18 DC-11 DC-12 DC-13"
[ -n "$want" ] || want=$all_ids
trap cleanup_made EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
pass=0
fail=0
failed=""
for id in $want; do
  fn=$(printf '%s' "$id" | tr 'A-Z-' 'a-z_')
  if ! command -v "$fn" > /dev/null 2>&1; then
    echo "$id MISSING (no such cell)"
    fail=$((fail + 1))
    failed="$failed $id"
    continue
  fi
  if "$fn"; then
    echo "$id PASS"
    pass=$((pass + 1))
  else
    echo "$id FAIL"
    fail=$((fail + 1))
    failed="$failed $id"
  fi
done
if [ "$fail" -eq 0 ]; then
  echo "SUMMARY under '$sut': $pass passed, 0 failed"
  exit 0
fi
echo "SUMMARY under '$sut': $pass passed, $fail failed ($failed )"
exit 1
