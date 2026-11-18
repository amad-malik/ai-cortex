#!/bin/sh
# scripts/roles-proof.sh: vera's proof for deploy/db-roles.sh on a THROWAWAY database (cox row 62f7c446, kai's ruling b7ff983b).
#
#   CV2_DB=1 /path/to/workspace/spikes/cortex-v2-s0/admission/cv2-admit sh scripts/roles-proof.sh
#
# Conditions it keeps: one container named cv2-s5-test-roles with the label cortexv2=dev, --rm, NO published port (exec only), no named volume
# and no bind mount (the image's own anonymous volume only), --network none (no shared network), 2 GiB and 1 CPU. The pinned pgvector image is
# used as it is, never pulled. Everything it creates (the container, its anonymous volume, three Podman secrets) is removed BY EXACT NAME, and
# the removal is verified and printed. It never touches a v1 container, volume or port.
#
# What it proves (vera's list): a clean install; an idempotent re-run; the refusal of an existing privileged or misconfigured role; and the
# actual attributes and memberships of the owner, app, install-admin and module roles. It also records the local-login claim of
# S5-PEER-AUTH-MAP.md (the postgres user connecting over the unix socket without a password) and the locale flags compose.yml passes to initdb.
set -u

die() { echo "roles-proof: $*" >&2; exit 2; }
[ "${CV2_DB:-}" = 1 ] || die "run it as: CV2_DB=1 cv2-admit sh scripts/roles-proof.sh (a database run holds the VM-wide database lock)"
command -v podman >/dev/null 2>&1 || die "podman is required"
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo=$(CDPATH= cd -- "$here/.." && pwd)

C=cv2-s5-test-roles
# The initdb flags are READ FROM deploy/compose.yml, so this proof tests exactly what compose passes and cannot drift from it.
INITDB_ARGS=$(python3 -I -B -c "import sys, yaml; print(yaml.safe_load(open(sys.argv[1]))['services']['db']['environment']['POSTGRES_INITDB_ARGS'])" "$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)/deploy/compose.yml") || { echo "roles-proof: cannot read POSTGRES_INITDB_ARGS from deploy/compose.yml" >&2; exit 2; }
S_SUPER=cv2-s5-test-super; S_OWNER=cv2-s5-test-owner; S_APP=cv2-s5-test-app
IMAGE='docker.io/pgvector/pgvector:0.8.2-pg18@sha256:42e7f6b4e1eceb02ff14e3e6bc6108bbe259abbe83879dc1845d0da1ddeb555d'
MODULES="audit identity tokens policy records search handoffs boot"

# What this run tests, by content hash, so ci.sh can say whether the last live result is for the CURRENT scripts (nix N27)
echo "proof-under-test db-roles.sh $(sha256sum "$repo/deploy/db-roles.sh" | cut -d' ' -f1)"
echo "proof-under-test 00-cortex-roles.sh $(sha256sum "$repo/deploy/initdb/00-cortex-roles.sh" | cut -d' ' -f1)"
echo "proof-under-test initdb-args $(printf '%s' "$INITDB_ARGS" | sha256sum | cut -d' ' -f1)"

pass=0 fail=0
ok()  { pass=$((pass + 1)); echo "ok   $1"; }
bad() { fail=$((fail + 1)); echo "FAIL $1"; [ -z "${2:-}" ] || echo "       $2"; }
expect() {  # expect <label> <expected> <actual>
  if [ "$2" = "$3" ]; then ok "$1"; else bad "$1" "expected [$2] got [$3]"; fi
}
q() { podman exec --user postgres "$C" psql -U postgres -d cortex -v ON_ERROR_STOP=1 -tA -c "$1"; }
roles() { CORTEX_DB_CONTAINER="$C" sh "$repo/deploy/db-roles.sh" --deadline "$(( $(date +%s) + 60 ))" "$@"; }   # cox W1-55: the caller passes its absolute deadline

# ---- refuse to start over anything that already exists: never clobber, never guess ------------------------------------------------------
for n in "$C"; do podman container exists "$n" && die "container $n already exists; remove it by exact name first"; done
for n in "$S_SUPER" "$S_OWNER" "$S_APP"; do podman secret inspect "$n" >/dev/null 2>&1 && die "secret $n already exists; remove it by exact name first"; done
podman image exists "$IMAGE" || die "the pinned image is not local; this proof does not pull"
vol_before=$(podman volume ls -q | sort)

volname=""
teardown() {
  rc=$?
  trap - EXIT INT TERM
  echo
  echo "== teardown: removing by exact name"
  [ -n "$volname" ] || volname=$(podman inspect --format '{{range .Mounts}}{{.Name}} {{end}}' "$C" 2>/dev/null | tr -d '\n' || true)
  podman rm --force --volumes "$C" >/dev/null 2>&1 || true
  podman secret rm "$S_SUPER" >/dev/null 2>&1 || true
  podman secret rm "$S_OWNER" >/dev/null 2>&1 || true
  podman secret rm "$S_APP" >/dev/null 2>&1 || true
  gone=1
  if podman container exists "$C"; then echo "STILL THERE: container $C"; gone=0; fi
  for v in $volname; do if podman volume exists "$v"; then echo "STILL THERE: volume $v"; gone=0; fi; done
  for n in "$S_SUPER" "$S_OWNER" "$S_APP"; do if podman secret inspect "$n" >/dev/null 2>&1; then echo "STILL THERE: secret $n"; gone=0; fi; done
  # informational only: other workers may create or remove volumes of their own while this runs
  for v in $(podman volume ls -q); do case " $(echo $vol_before) " in *" $v "*) ;; *) echo "note: volume $v appeared since the start (not mine unless named above)" ;; esac; done
  if [ "$gone" = 1 ]; then echo "REMOVAL VERIFIED: container $C, its anonymous volume (${volname:-none}) and the three secrets are gone"; else echo "REMOVAL NOT VERIFIED"; rc=1; fi
  exit "$rc"
}
trap teardown EXIT INT TERM

# ---- start the throwaway database -------------------------------------------------------------------------------------------------------
echo "== start $C (label cortexv2=dev, --rm, --network none, no published port, no named volume, 2 GiB, 1 CPU)"
for pair in "$S_SUPER" "$S_OWNER" "$S_APP"; do
  val=$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')
  [ "${#val}" -eq 64 ] || die "could not read random bytes"
  printf '%s' "$val" | podman secret create "$pair" - >/dev/null || die "could not create secret $pair"
  val=""
done
podman run -d --rm --pull=never --name "$C" --label cortexv2=dev --network none --memory 2g --cpus 1 \
  --secret "$S_SUPER,target=cv2-s5-db-superuser" \
  --secret "$S_OWNER,target=cv2-s5-owner-password" \
  --secret "$S_APP,target=cv2-s5-app-password" \
  -e POSTGRES_DB=cortex -e POSTGRES_PASSWORD_FILE=/run/secrets/cv2-s5-db-superuser \
  -e "POSTGRES_INITDB_ARGS=$INITDB_ARGS" \
  "$IMAGE" postgres -c shared_buffers=128MB >/dev/null || die "could not start $C"
volname=$(podman inspect --format '{{range .Mounts}}{{.Name}} {{end}}' "$C" | tr -d '\n')
echo "container $C started; anonymous volume: ${volname:-none}"

waited=0
until podman exec --user postgres "$C" pg_isready -q -h 127.0.0.1 -U postgres -d cortex >/dev/null 2>&1; do
  [ "$waited" -lt 120 ] || { echo "the database did not become ready in 120 s"; podman logs "$C" 2>&1 | tail -5; exit 3; }
  sleep 2; waited=$((waited + 2))
done
echo "ready after about $waited s"

# ---- 0. the local-login claim and the initdb flags --------------------------------------------------------------------------------------
echo; echo "== 0. peer-auth map: the postgres user over the unix socket, no password"
expect "P0a OS user postgres connects over the unix socket without a password" "1" "$(q 'select 1' 2>/dev/null)"
hba=$(q "select pg_read_file('pg_hba.conf')" 2>/dev/null || true)
echo "pg_hba.conf, non-comment lines:"; printf '%s\n' "$hba" | grep -vE '^\s*(#|$)' | sed 's/^/   /'
expect "P0b locale flags reach the cortex database (provider builtin, collation C.UTF-8)" "bC.UTF-8" "$(q "select datlocprovider::text||datcollate from pg_database where datname='cortex'")"

# ---- 0b. the EFFECTIVE pg_hba rules (cox e0b0b76b, vera): captured from the running server, not from the environment ---------------------------
echo; echo "== 0b. effective pg_hba rules (pg_hba_file_rules), and the logins they allow or refuse"
echo "initdb flags under test (read from compose.yml): $INITDB_ARGS"
rules=$(q "select line_number||' '||type||' '||array_to_string(database, ',')||' '||array_to_string(user_name, ',')||' '||coalesce(address,'-')||' '||coalesce(netmask,'-')||' '||auth_method||' '||coalesce(error,'ok') from pg_hba_file_rules order by line_number")
echo "effective rules (line type database user address netmask method error):"; printf '%s\n' "$rules" | sed 's/^/   /'
expect "H1 no rule is trust and none has an error" "0" "$(q "select count(*) from pg_hba_file_rules where auth_method='trust' or error is not null")"
expect "H2 the socket rules (all and replication) are peer" "2|peer" "$(q "select count(*)||'|'||min(auth_method) from pg_hba_file_rules where type='local'")"
expect "H3 every TCP rule is scram-sha-256 (loopback v4 and v6 for all and for replication, and the network rule: five)" "5|scram-sha-256" "$(q "select count(*)||'|'||min(auth_method) from pg_hba_file_rules where type='host' and auth_method='scram-sha-256'")"
expect "H4 the rules are exactly the seven expected, in file order: no earlier rule can shadow a later one" "local|all|peer,host|all|scram-sha-256,host|all|scram-sha-256,local|replication|peer,host|replication|scram-sha-256,host|replication|scram-sha-256,host|all|scram-sha-256" "$(q "select string_agg(type||'|'||array_to_string(database, ',')||'|'||auth_method, ',' order by line_number) from pg_hba_file_rules")"
# the intended login: OS user postgres, over the socket, peer (this is what db-roles.sh and nix's dump use)
expect "H5 the OS user postgres still logs in over the socket (peer): db-roles.sh and the dump rely on it" "1" "$(podman exec --user postgres "$C" psql -U postgres -d cortex -tAc 'select 1' 2>/dev/null)"
if podman exec --user postgres "$C" pg_dump -U postgres -d cortex -s > /dev/null 2>&1; then ok "H6 nix's dump form (pg_dump -U postgres as the postgres OS user over the socket) works"; else bad "H6 nix's dump form (pg_dump -U postgres as the postgres OS user over the socket) works"; fi
# the refusals
out=$(podman exec --user postgres "$C" psql -h 127.0.0.1 -U postgres -d cortex -tAc 'select 1' 2>&1 </dev/null); rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -qi 'password'; then ok "H7 a TCP loopback login (127.0.0.1) without a password FAILS and asks for one"; else bad "H7 a TCP loopback login (127.0.0.1) without a password FAILS and asks for one" "rc=$rc out=$(printf '%s' "$out" | head -1)"; fi
out=$(podman exec --user postgres "$C" psql -h ::1 -U postgres -d cortex -tAc 'select 1' 2>&1 </dev/null); rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -qi 'password'; then ok "H8 a TCP loopback login (::1) without a password FAILS and asks for one"; elif [ "$rc" -ne 0 ]; then echo "note H8: ::1 is not reachable in this network namespace ($(printf '%s' "$out" | head -1)); the ::1 rule is verified in H3 and H4"; ok "H8 (no IPv6 loopback here; rule checked in H3 and H4)"; else bad "H8 a TCP loopback login (::1) without a password FAILS" "it succeeded without a password"; fi

# ---- 1. a clean install -----------------------------------------------------------------------------------------------------------------
echo; echo "== 1. clean install: the real roles script, then db-roles.sh for the eight skeleton modules"
podman cp "$repo/deploy/initdb/00-cortex-roles.sh" "$C:/tmp/00-cortex-roles.sh" || die "podman cp failed"
if podman exec --user postgres "$C" sh /tmp/00-cortex-roles.sh >/dev/null 2>&1; then ok "P1a deploy/initdb/00-cortex-roles.sh runs"; else bad "P1a deploy/initdb/00-cortex-roles.sh runs"; fi
# shellcheck disable=SC2086
if roles $MODULES >/dev/null 2>&1; then ok "P1b db-roles.sh creates the module roles (exit 0)"; else bad "P1b db-roles.sh creates the module roles (exit 0)"; fi

# ---- 1b. the logins that need the roles to exist (positive control and the socket is no backdoor)
# positive control: the same TCP path WITH the right password works, so the rule is scram and not a blanket reject (the password is read inside the container and never printed)
got=$(podman exec --user postgres "$C" sh -c 'PGPASSWORD=$(cat /run/secrets/cv2-s5-app-password) psql -h 127.0.0.1 -U cortex_app -d cortex -tAc "select current_user"' 2>/dev/null </dev/null || true)
expect "H9 positive control: cortex_app over TCP loopback WITH its password works (scram, not a blanket reject)" "cortex_app" "$got"
out=$(podman exec --user postgres "$C" psql -U cortex_app -d cortex -tAc 'select 1' 2>&1 </dev/null); rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -qi 'peer'; then ok "H10 the socket is no backdoor to other roles: cortex_app over the socket as OS user postgres is refused (peer mismatch)"; else bad "H10 the socket is no backdoor to other roles: cortex_app over the socket as OS user postgres is refused (peer mismatch)" "rc=$rc out=$(printf '%s' "$out" | head -1)"; fi


attrs="concat_ws('|', rolcanlogin::int, rolsuper::int, rolcreaterole::int, rolcreatedb::int, rolbypassrls::int, rolreplication::int, rolinherit::int)"
rattr() { q "select $attrs from pg_roles where rolname='$1'"; }
# login|super|createrole|createdb|bypassrls|replication|inherit
expect "P1c cortex_owner: login, no super, NO CREATEROLE, no createdb, no bypassrls, no replication, inherit" "1|0|0|0|0|0|1" "$(rattr cortex_owner)"
expect "P1d cortex_app: login, none of the privileges, NOINHERIT" "1|0|0|0|0|0|0" "$(rattr cortex_app)"
expect "P1e cortex_install_admin: NOLOGIN, none of the privileges" "0|0|0|0|0|0|1" "$(rattr cortex_install_admin)"
for id in $MODULES; do
  expect "P1f cortex_m_$id: NOLOGIN NOSUPERUSER NOCREATEROLE NOCREATEDB NOBYPASSRLS NOREPLICATION" "0|0|0|0|0|0|1" "$(rattr cortex_m_$id)"
  expect "P1g cortex_m_$id: a member of no role, and cortex_app is a member of it" "0|true" "$(q "select (select count(*) from pg_auth_members where member=(select oid from pg_roles where rolname='cortex_m_$id'))||'|'||pg_has_role('cortex_app','cortex_m_$id','member')")"
done
expect "P1h exactly eight module roles exist" "8" "$(q "select count(*) from pg_roles where rolname like 'cortex\\_m\\_%'")"
expect "P1i cortex_app is a member of cortex_install_admin and NOT of cortex_owner" "true|false" "$(q "select pg_has_role('cortex_app','cortex_install_admin','member')||'|'||pg_has_role('cortex_app','cortex_owner','member')")"
expect "P1j cortex_owner owns the cortex database" "cortex_owner" "$(q "select pg_get_userbyid(datdba) from pg_database where datname='cortex'")"

# ---- 2. an idempotent re-run ------------------------------------------------------------------------------------------------------------
echo; echo "== 2. idempotent re-run"
before=$(q "select string_agg(oid||':'||rolname, ',' order by rolname) from pg_roles where rolname like 'cortex%'")
mem_before=$(q "select string_agg(roleid||'>'||member, ',' order by roleid, member) from pg_auth_members where member in (select oid from pg_roles where rolname like 'cortex%')")
# shellcheck disable=SC2086
if roles $MODULES >/dev/null 2>&1; then ok "P2a the second run exits 0"; else bad "P2a the second run exits 0"; fi
expect "P2b the same roles with the same oids (nothing recreated)" "$before" "$(q "select string_agg(oid||':'||rolname, ',' order by rolname) from pg_roles where rolname like 'cortex%'")"
expect "P2c the same memberships" "$mem_before" "$(q "select string_agg(roleid||'>'||member, ',' order by roleid, member) from pg_auth_members where member in (select oid from pg_roles where rolname like 'cortex%')")"

# ---- 3. refusals: an existing unsafe role is refused, left as it is, and nothing else is created ----------------------------------------
echo; echo "== 3. refusals (the step must exit non-zero, say why, change nothing, and create no other role)"
refuse() {  # refuse <label> <setup sql> <ids> <message fragment> <check sql> <expected> <undo sql>
  q "$2" >/dev/null 2>&1 || { bad "$1: could not set up the bad state"; return; }
  # shellcheck disable=SC2086
  out=$(roles $3 2>&1 >/dev/null); rc=$?
  if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q "$4"; then ok "$1: refused (exit $rc, says: $4)"; else bad "$1: refused with the reason" "rc=$rc out=$(printf '%s' "$out" | head -2 | tr '\n' ' ')"; fi
  expect "$1: left exactly as it was (the step does not repair)" "$6" "$(q "$5")"
  q "$7" >/dev/null 2>&1 || bad "$1: could not undo the bad state"
  expect "$1: no other module role was created" "8" "$(q "select count(*) from pg_roles where rolname like 'cortex\\_m\\_%'")"
}
refuse "R1 existing module role with LOGIN and SUPERUSER" "create role cortex_m_evil login superuser" "evil" "unsafe existing role cortex_m_evil" "select rolcanlogin::int||'|'||rolsuper::int from pg_roles where rolname='cortex_m_evil'" "1|1" "drop role cortex_m_evil"
refuse "R2 existing module role with BYPASSRLS" "create role cortex_m_bypass nologin bypassrls" "bypass" "unsafe existing role cortex_m_bypass" "select rolbypassrls::int from pg_roles where rolname='cortex_m_bypass'" "1" "drop role cortex_m_bypass"
refuse "R3 existing module role with LOGIN only" "create role cortex_m_login login" "login" "unsafe existing role cortex_m_login" "select rolcanlogin::int from pg_roles where rolname='cortex_m_login'" "1" "drop role cortex_m_login"
refuse "R4 existing module role that is a member of another role" "create role cortex_m_member nologin; grant cortex_install_admin to cortex_m_member" "member" "must be a member of no role" "select count(*) from pg_auth_members where member=(select oid from pg_roles where rolname='cortex_m_member')" "1" "drop role cortex_m_member"
refuse "R5 cortex_owner given CREATEROLE (the standing privilege we removed)" "alter role cortex_owner createrole" "audit" "unsafe attributes on role cortex_owner" "select rolcreaterole::int from pg_roles where rolname='cortex_owner'" "1" "alter role cortex_owner nocreaterole"
refuse "R6 cortex_app made INHERIT" "alter role cortex_app inherit" "audit" "cortex_app must be NOINHERIT" "select rolinherit::int from pg_roles where rolname='cortex_app'" "1" "alter role cortex_app noinherit"
refuse "R7 cortex_app made a member of cortex_owner" "grant cortex_owner to cortex_app" "audit" "never be a member of cortex_owner" "select pg_has_role('cortex_app','cortex_owner','member')::int" "1" "revoke cortex_owner from cortex_app"
refuse "R8 cortex_app made SUPERUSER" "alter role cortex_app superuser" "audit" "unsafe attributes on role cortex_app" "select rolsuper::int from pg_roles where rolname='cortex_app'" "1" "alter role cortex_app nosuperuser"
expect "R9 after all refusals the attributes are back to the safe state" "1|0|0|0|0|0|0" "$(rattr cortex_app)"

# ---- 4. nix's extra cases (S5-REVIEW-nix-r5), cox's hardening (ce1b9137), and a concurrent run (08dd8528) --------------------------------
echo; echo "== 4. extra cases and the hardening"
nmod() { q "select count(*) from pg_roles where rolname like 'cortex\\_m\\_%'"; }
# nix 1: an existing cortex_m_records with LOGIN / CREATEROLE / SUPERUSER / BYPASSRLS: refused, and NOTHING is granted after it (the id after it is not created)
for attr in login createrole superuser bypassrls; do
  q "alter role cortex_m_records $attr" >/dev/null 2>&1 || { bad "E1 $attr: could not set up"; continue; }
  out=$(roles records zz1 2>&1 >/dev/null); rc=$?
  # cortex_app already belongs to cortex_m_records, so since the N23 check the refusal can come from the membership check (new message) or from the per-id check (old message): both are the refusal
  if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -qE 'unsafe existing role cortex_m_records|cortex_app is a member of the unsafe role cortex_m_records'; then ok "E1 cortex_m_records with $attr: refused"; else bad "E1 cortex_m_records with $attr: refused" "rc=$rc out=$(printf '%s' "$out" | head -1)"; fi
  expect "E1 $attr: the id after it was not created (nothing granted after a refusal)" "0" "$(q "select count(*) from pg_roles where rolname='cortex_m_zz1'")"
  q "alter role cortex_m_records no$attr" >/dev/null 2>&1 || bad "E1 $attr: could not undo"
done
expect "E1 records is back to the safe state" "0|0|0|0|0|0|1" "$(rattr cortex_m_records)"
# nix 3: cortex_app a member of cortex_owner THROUGH A MIDDLE ROLE
q "create role mid nologin; grant cortex_owner to mid; grant mid to cortex_app" >/dev/null 2>&1
out=$(roles audit 2>&1 >/dev/null); rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -qE 'never be a member of cortex_owner|must not be a member of'; then ok "E3 cortex_app a member of cortex_owner through a middle role: refused"; else bad "E3 cortex_app a member of cortex_owner through a middle role: refused" "rc=$rc out=$(printf '%s' "$out" | head -1)"; fi
q "revoke mid from cortex_app; revoke cortex_owner from mid; drop role mid" >/dev/null 2>&1
# nix 4 and cox (3): cortex_install_admin with LOGIN
q "alter role cortex_install_admin login" >/dev/null 2>&1
out=$(roles audit 2>&1 >/dev/null); rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'cortex_install_admin must be NOLOGIN'; then ok "E4 cortex_install_admin with LOGIN: refused"; else bad "E4 cortex_install_admin with LOGIN: refused" "rc=$rc out=$(printf '%s' "$out" | head -1)"; fi
q "alter role cortex_install_admin nologin" >/dev/null 2>&1
# nix 5 and cox (2): cortex_app a member of a predefined powerful role
q "grant pg_execute_server_program to cortex_app" >/dev/null 2>&1
out=$(roles audit 2>&1 >/dev/null); rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'cortex_app must not be a member of pg_execute_server_program'; then ok "E5 cortex_app a member of pg_execute_server_program: refused"; else bad "E5 cortex_app a member of pg_execute_server_program: refused" "rc=$rc out=$(printf '%s' "$out" | head -1)"; fi
q "revoke pg_execute_server_program from cortex_app" >/dev/null 2>&1
# cox (2): cortex_owner belongs to no role at all
q "grant cortex_install_admin to cortex_owner" >/dev/null 2>&1
out=$(roles audit 2>&1 >/dev/null); rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'cortex_owner must not be a member of cortex_install_admin'; then ok "E7 cortex_owner a member of cortex_install_admin: refused"; else bad "E7 cortex_owner a member of cortex_install_admin: refused" "rc=$rc out=$(printf '%s' "$out" | head -1)"; fi
q "revoke cortex_install_admin from cortex_owner" >/dev/null 2>&1
# nix 6: a module list that GROWS between runs
before2=$(q "select string_agg(oid||':'||rolname, ',' order by rolname) from pg_roles where rolname in ('cortex_m_g1','cortex_m_g2')")
roles g1 g2 >/dev/null 2>&1; a=$(q "select string_agg(oid||':'||rolname, ',' order by rolname) from pg_roles where rolname in ('cortex_m_g1','cortex_m_g2')")
roles g1 g2 g3 g4 >/dev/null 2>&1; b=$(q "select string_agg(oid||':'||rolname, ',' order by rolname) from pg_roles where rolname in ('cortex_m_g1','cortex_m_g2')")
expect "E6 a list that grows between runs keeps the old roles (same oids) and adds the new ones" "$a|4" "$b|$(q "select count(*) from pg_roles where rolname in ('cortex_m_g1','cortex_m_g2','cortex_m_g3','cortex_m_g4')")"
q "drop role cortex_m_g1; drop role cortex_m_g2; drop role cortex_m_g3; drop role cortex_m_g4" >/dev/null 2>&1
expect "E6 cleanup: back to eight module roles" "8" "$(nmod)"

# nix N23: a LEFTOVER module role (its id is NOT in this run's list) that is unsafe and granted to cortex_app must still be refused
q "create role cortex_m_old login superuser; grant cortex_m_old to cortex_app" >/dev/null 2>&1
out=$(roles audit 2>&1 >/dev/null); rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'cortex_app is a member of the unsafe role cortex_m_old'; then ok "E8 a leftover cortex_m_old (LOGIN, SUPERUSER) granted to cortex_app, not in this run's list: refused"; else bad "E8 a leftover cortex_m_old (LOGIN, SUPERUSER) granted to cortex_app, not in this run's list: refused" "rc=$rc out=$(printf '%s' "$out" | head -1)"; fi
q "revoke cortex_m_old from cortex_app; drop role cortex_m_old" >/dev/null 2>&1
q "create role cortex_m_old2 nologin; grant pg_execute_server_program to cortex_m_old2; grant cortex_m_old2 to cortex_app" >/dev/null 2>&1
out=$(roles audit 2>&1 >/dev/null); rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'cortex_m_old2 must be a member of no role'; then ok "E8b a leftover cortex_m_old2 that is a member of pg_execute_server_program, granted to cortex_app: refused"; else bad "E8b a leftover cortex_m_old2 that is a member of pg_execute_server_program, granted to cortex_app: refused" "rc=$rc out=$(printf '%s' "$out" | head -1)"; fi
q "revoke cortex_m_old2 from cortex_app; drop role cortex_m_old2" >/dev/null 2>&1
expect "E8 cleanup: back to eight module roles" "8" "$(nmod)"
# nix N24: "a refused run leaves nothing behind" is SHOWN, not assumed: the new id comes BEFORE the unsafe one, and must not survive the refusal
q "create role cortex_m_bad_b login" >/dev/null 2>&1
out=$(roles new_a bad_b 2>&1 >/dev/null); rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'unsafe existing role cortex_m_bad_b'; then ok "E9 ids 'new_a bad_b' with bad_b unsafe and existing: refused"; else bad "E9 ids 'new_a bad_b' with bad_b unsafe and existing: refused" "rc=$rc out=$(printf '%s' "$out" | head -1)"; fi
expect "E9 the role created EARLIER in the same run (cortex_m_new_a) was rolled back" "0" "$(q "select count(*) from pg_roles where rolname='cortex_m_new_a'")"
q "drop role cortex_m_bad_b" >/dev/null 2>&1
expect "E9 cleanup: back to eight module roles" "8" "$(nmod)"

# nix N28: nobody but cortex_app may belong to a cortex_m_* role (a login role granted one could SET ROLE into it)
q "create role cortex_x_login login; grant cortex_m_records to cortex_x_login" >/dev/null 2>&1
out=$(roles audit 2>&1 >/dev/null); rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'cortex_x_login must not be a member of the module role cortex_m_records'; then ok "E10 a login role that belongs to cortex_m_records: refused"; else bad "E10 a login role that belongs to cortex_m_records: refused" "rc=$rc out=$(printf '%s' "$out" | head -1)"; fi
q "revoke cortex_m_records from cortex_x_login; drop role cortex_x_login" >/dev/null 2>&1
expect "E10 cleanup: back to eight module roles" "8" "$(nmod)"

# cox 08dd8528: a concurrent run WAITS (the advisory lock), it does not race. A second session holds the lock for 6 s.
podman exec --user postgres "$C" psql -U postgres -d cortex -tA -c "select pg_advisory_lock(hashtext('cortex_db_roles')); select pg_sleep(6)" >/dev/null 2>&1 &
holder=$!
sleep 1
t0=$(date +%s); roles g5 g6 >/dev/null 2>&1; rc=$?; t1=$(date +%s)
wait "$holder" 2>/dev/null
if [ "$rc" = 0 ] && [ $((t1 - t0)) -ge 4 ]; then ok "C1 db-roles.sh waited for the lock held by another session ($((t1 - t0)) s) and then succeeded"; else bad "C1 db-roles.sh waited for the lock held by another session and then succeeded" "rc=$rc waited=$((t1 - t0)) s"; fi
expect "C1 both roles exist" "2" "$(q "select count(*) from pg_roles where rolname in ('cortex_m_g5','cortex_m_g6')")"
# two runs at once for the same new ids: both succeed, one role per id
rcdir=$(mktemp -d)
( roles g7 g8 >/dev/null 2>&1; echo $? > "$rcdir/a" ) &
pa=$!
( roles g7 g8 >/dev/null 2>&1; echo $? > "$rcdir/b" ) &
pb=$!
wait "$pa" "$pb"
expect "C2 two simultaneous runs for the same new ids both exit 0" "0|0" "$(cat "$rcdir/a")|$(cat "$rcdir/b")"
expect "C2 exactly one role per id" "2" "$(q "select count(*) from pg_roles where rolname in ('cortex_m_g7','cortex_m_g8')")"
rm -rf "$rcdir"
q "drop role cortex_m_g5; drop role cortex_m_g6; drop role cortex_m_g7; drop role cortex_m_g8" >/dev/null 2>&1
expect "C2 cleanup: back to eight module roles" "8" "$(nmod)"

# ---- cox W1-55 (mike's M-4): the CALLER's absolute deadline E; the guard (the LAST statement before COMMIT) refuses past E - 9 (lead = idle 5 + commit latency 2 + slack 2). ----------
echo; echo "== W1-55: the roles transaction cannot COMMIT past E - 9, whatever delayed it"
hostnow=$(date +%s); dbnow=$(q "select floor(extract(epoch from clock_timestamp()))::bigint")
if [ $((dbnow - hostnow)) -le 2 ] && [ $((hostnow - dbnow)) -le 2 ]; then ok "K1 the host clock and the database clock agree within 2 s (E is the host's, the guard reads the database's)"; else bad "K1 the host clock and the database clock agree within 2 s" "host=$hostnow database=$dbnow"; fi
before=$(nmod)
real_podman=$(command -v podman); shimdir=$(mktemp -d)
printf '#!/bin/sh\nsleep "${SHIM_DELAY:-0}"\nexec %s "$@"\n' "$real_podman" > "$shimdir/podman"; chmod 0755 "$shimdir/podman"
roles_e() { e=$1; shift; CORTEX_DB_CONTAINER="$C" PATH="$shimdir:$PATH" sh "$repo/deploy/db-roles.sh" --deadline "$e" "$@"; }
# C4 control: E = now + 60 and an exec delayed 1 s: well inside E - 9, applied
out=$(SHIM_DELAY=1 roles_e $(( $(date +%s) + 60 )) h2 2>&1); rc=$?
expect "C4 an exec delayed 1 s (inside E - 9) is applied (exit 0)" "0" "$rc"
expect "C4 the role exists" "1" "$(q "select count(*) from pg_roles where rolname = 'cortex_m_h2'")"
# C3 the engine's real failure mode: the launch is delayed, E was fixed BEFORE the delay. E = now + 20 gives a guard at now + 11; the shim sleeps 14 s before the real exec, so the guard (the last
# statement, read against the database clock) refuses and NOTHING is committed
out=$(SHIM_DELAY=14 roles_e $(( $(date +%s) + 20 )) h3 2>&1); rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'db-roles: past its deadline; nothing was committed'; then ok "C3 an exec delayed past E - 9 is refused with the fixed line (exit $rc)"; else bad "C3 an exec delayed past E - 9 is refused with the fixed line" "rc=$rc out=$(printf '%s' "$out" | tail -n 2 | cut -c1-160)"; fi
expect "C3 no role for the delayed id" "0" "$(q "select count(*) from pg_roles where rolname = 'cortex_m_h3'")"
rm -rf "$shimdir"
q "drop role cortex_m_h2" >/dev/null 2>&1
expect "W1-55 cleanup: back to the module roles before" "$before" "$(nmod)"

echo; echo "roles-proof: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
