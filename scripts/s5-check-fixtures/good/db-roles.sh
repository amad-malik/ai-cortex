#!/bin/sh
set -eu
# deploy/db-roles.sh <module-id>...   create the cortex_m_<id> roles and grant them to cortex_app (cox row 2350a624; vera's boundaries, row ca513646).
#
# A SUPERUSER step through the database container's local connection (`podman exec --user postgres ... psql -U postgres`: an explicit OS
# user, peer authentication over the container's unix socket, no password), run by install.sh and by `cortex upgrade` after the database is
# healthy and BEFORE the migrations, so cortex_owner needs no standing CREATEROLE. Idempotent, and it VERIFIES rather than trusting:
#   - a new module role is created NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS NOREPLICATION;
#   - an EXISTING module role must already be like that and a member of no role, or the step REFUSES (RAISE EXCEPTION), it does not repair;
#   - the fixed roles must not have drifted: cortex_owner, cortex_app and cortex_install_admin carry none of those privileges, cortex_app is
#     NOINHERIT, cortex_install_admin is NOLOGIN, and cortex_app is never a member of cortex_owner (directly or through another role);
#   - membership is closed: cortex_app belongs to cortex_install_admin and the cortex_m_<module> roles ONLY, cortex_owner and
#     cortex_install_admin to NO role. A NOINHERIT member can still SET ROLE to any role it was granted (pg_execute_server_program,
#     pg_write_server_files, ...), so "no inheritance" is not enough (nix N20, cox ce1b9137). cortex_owner is held to the stricter "none"
#     (nix); relax that in one line if the S2 migrations ever need it;
#   - roles are CLUSTER-WIDE, so two runs at once would race (duplicate_object, unique_violation: finn's plug hit it). The whole script is ONE
#     transaction (psql -1) that first takes an advisory lock: a concurrent run WAITS, then finds the roles in place and succeeds; a refusal
#     rolls everything back, so a refused run leaves nothing behind. (Advisory locks are per database: both runs use the cortex database.)
# The ids come from the build's module list (the mounted config's [modules] enabled); each is validated here too, because it ends up in SQL.
# The SQL goes over stdin: nothing in argv, no password, no secret.
#
# PEER-AUTH MAP (VERIFIED 5 Nov on a throwaway container, scripts/roles-proof.sh): this relies on the pgvector image letting OS user `postgres` in
# over the container's unix socket without a password (its pg_hba has `local all all trust`). See S5-PEER-AUTH-MAP.md for everything that relies on it.
# SERVER-SIDE BOUND (cox 19:41Z on mike's R6; A106). `cortex upgrade` ends THIS sh when its run_roles deadline passes, but the psql in the container is another
# process: a transaction waiting on the advisory lock could still COMMIT later. So the SESSION carries a budget that the SERVER enforces: transaction_timeout
# (PostgreSQL 17 and later; it ends the session) through PGOPTIONS, set when the connection opens so that it is armed for the transaction `psql -1` begins; never
# a SET inside the transaction, which may arm too late. 30 s is shorter than the engine's run_roles deadline by at least 15 s (`cortex upgrade` refuses
# --roles-timeout below 45 s: design r13). Checker rule S5-22 holds this number.
budget=30
# THE ENGINE'S DEADLINE, AND THE ONE RELATION (cox W1-55; mike's M-4). transaction_timeout bounds how long the transaction RUNS, not when it STARTS; and a script that read
# its own start (date +%s) could be scheduled late: its deadline would be later than the engine's, and a transaction waiting on the lock could COMMIT after `cortex upgrade`
# gave up and began its rollback. So the CALLER passes its absolute deadline E (epoch seconds): the real adapter, E = the moment it launches this script + its run_roles
# timeout; install.sh, which has no engine, now + 60. THIS SCRIPT READS NO CLOCK OF ITS OWN FOR A DEADLINE (checker rule S5-22 refuses `date` in it).
# The last statement of the roles transaction, right before COMMIT, reads the DATABASE clock and raises when it is past E - lead; the three terms and their sum, stated ONCE
# (the checker holds the numbers, the engine's cell en_09 mirrors them):
#   idle    5 s  a client that stalls between the guard and COMMIT is ended by idle_in_transaction_session_timeout (the session is killed, nothing commits)
#   latency 2 s  the time COMMIT itself may take
#   slack   2 s  what is left before E
#   lead    = idle + latency + slack = 9 s:  the guard passes only if the clock is <= E - 9, so the LATEST COMPLETED COMMIT IS E - 2, whatever delayed the launch, the lock or the client.
# Known limits, not defended by these constants: a step of the host's wall clock between the caller's read and the guard (the host and the container share one kernel
# clock, so a step moves both, but not between the two reads); scheduling between the guard and COMMIT beyond the 5 s idle bound; the script committed in time but its exit
# status was lost (a hung podman exec): the roles are then applied while the engine rolls the binary back, and the next attempt re-applies the idempotent script.
idle=5
latency=2
slack=2
lead=$((idle + latency + slack))
# The container is cv2-s5-db unless a test names another of OUR containers (cox 62f7c446: the throwaway-database proof). Only a cv2-s5-* name
# with plain characters is accepted, so the override can never point at v1's containers; a refused name reaches podman not at all.
db=${CORTEX_DB_CONTAINER:-cv2-s5-db}
case $db in cv2-s5-?*) ;; *) echo "db-roles: CORTEX_DB_CONTAINER must be a cv2-s5-* container name" >&2; exit 2 ;; esac
case $db in *[!abcdefghijklmnopqrstuvwxyz0123456789-]*) echo "db-roles: CORTEX_DB_CONTAINER may contain only a-z, 0-9 and -" >&2; exit 2 ;; esac
if [ "${1:-}" != "--deadline" ] || [ $# -lt 2 ]; then echo "db-roles: usage: db-roles.sh --deadline <epoch-seconds> <module-id>..." >&2; exit 2; fi
deadline=$2
case $deadline in ""|0*|*[!0123456789]*) echo "db-roles: --deadline must be epoch seconds (digits only, no leading zero)" >&2; exit 2 ;; esac
[ "${#deadline}" -le 11 ] || { echo "db-roles: --deadline is too long" >&2; exit 2; }
shift 2
guard_by=$((deadline - lead))
[ $# -gt 0 ] || { echo "db-roles: no module ids given" >&2; exit 2; }
for id in "$@"; do
  case $id in ""|[!abcdefghijklmnopqrstuvwxyz]*|*[!abcdefghijklmnopqrstuvwxyz0123456789_]*) echo "db-roles: invalid module id" >&2; exit 2 ;; esac
  [ "${#id}" -le 32 ] || { echo "db-roles: invalid module id (longer than 32 characters)" >&2; exit 2; }
done
{
  cat <<'SQL'
DO $$
DECLARE r record;
BEGIN
  PERFORM pg_advisory_xact_lock(hashtext('cortex_db_roles'));
  FOR r IN SELECT rolname, rolsuper, rolcreaterole, rolcreatedb, rolbypassrls, rolreplication, rolinherit, rolcanlogin
             FROM pg_roles WHERE rolname IN ('cortex_owner', 'cortex_app', 'cortex_install_admin') LOOP
    IF r.rolsuper OR r.rolcreaterole OR r.rolcreatedb OR r.rolbypassrls OR r.rolreplication THEN
      RAISE EXCEPTION 'unsafe attributes on role %', r.rolname;
    END IF;
    IF r.rolname = 'cortex_app' AND r.rolinherit THEN
      RAISE EXCEPTION 'cortex_app must be NOINHERIT';
    END IF;
    IF r.rolname = 'cortex_install_admin' AND r.rolcanlogin THEN
      RAISE EXCEPTION 'cortex_install_admin must be NOLOGIN';
    END IF;
  END LOOP;
  IF pg_has_role('cortex_app', 'cortex_owner', 'member') THEN
    RAISE EXCEPTION 'cortex_app must never be a member of cortex_owner';
  END IF;
  FOR r IN SELECT mem.rolname AS member, grp.rolname AS granted
             FROM pg_auth_members am
             JOIN pg_roles mem ON mem.oid = am.member
             JOIN pg_roles grp ON grp.oid = am.roleid
            WHERE mem.rolname IN ('cortex_owner', 'cortex_app', 'cortex_install_admin') LOOP
    IF r.member = 'cortex_app' AND (r.granted = 'cortex_install_admin' OR r.granted ~ '^cortex_m_[a-z][a-z0-9_]{0,31}$') THEN
      IF r.granted <> 'cortex_install_admin' THEN
        -- every module role cortex_app belongs to is checked, whether or not its id is in this run's list (nix N23): a leftover cortex_m_old that can
        -- log in or is a superuser, or that is itself in another role, would otherwise pass the name pattern and be one SET ROLE away
        IF EXISTS (SELECT 1 FROM pg_roles g WHERE g.rolname = r.granted
                    AND (g.rolcanlogin OR g.rolsuper OR g.rolcreaterole OR g.rolcreatedb OR g.rolbypassrls OR g.rolreplication)) THEN
          RAISE EXCEPTION 'cortex_app is a member of the unsafe role %', r.granted;
        END IF;
        IF EXISTS (SELECT 1 FROM pg_auth_members m2 WHERE m2.member = (SELECT oid FROM pg_roles WHERE rolname = r.granted)) THEN
          RAISE EXCEPTION '% must be a member of no role', r.granted;
        END IF;
      END IF;
      CONTINUE;
    END IF;
    RAISE EXCEPTION '% must not be a member of %', r.member, r.granted;
  END LOOP;
  -- and nobody but cortex_app may belong to a module role: a login role granted one could SET ROLE into it (nix N28)
  FOR r IN SELECT mem.rolname AS member, grp.rolname AS granted
             FROM pg_auth_members am
             JOIN pg_roles mem ON mem.oid = am.member
             JOIN pg_roles grp ON grp.oid = am.roleid
            WHERE grp.rolname ~ '^cortex_m_[a-z][a-z0-9_]{0,31}$' AND mem.rolname <> 'cortex_app' LOOP
    RAISE EXCEPTION '% must not be a member of the module role %', r.member, r.granted;
  END LOOP;
END $$;
SQL
  for id in "$@"; do
    printf "DO \$\$ BEGIN\n  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cortex_m_%s') THEN\n    CREATE ROLE cortex_m_%s NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS NOREPLICATION;\n  ELSIF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cortex_m_%s' AND (rolcanlogin OR rolsuper OR rolcreaterole OR rolcreatedb OR rolbypassrls OR rolreplication)) THEN\n    RAISE EXCEPTION 'unsafe existing role cortex_m_%s';\n  ELSIF EXISTS (SELECT 1 FROM pg_auth_members m WHERE m.member = (SELECT oid FROM pg_roles WHERE rolname = 'cortex_m_%s')) THEN\n    RAISE EXCEPTION 'cortex_m_%s must be a member of no role';\n  END IF;\nEND \$\$;\n" "$id" "$id" "$id" "$id" "$id" "$id"
    printf 'GRANT cortex_m_%s TO cortex_app;\n' "$id"
  done
  cat <<'SQL'
DO $$ BEGIN
  IF extract(epoch FROM clock_timestamp()) > current_setting('db_roles.guard_by')::float8 THEN
    RAISE EXCEPTION 'db-roles: past its deadline; nothing was committed';
  END IF;
END $$;
SQL
} | podman exec -i --user postgres --env "PGOPTIONS=-c transaction_timeout=${budget}s -c idle_in_transaction_session_timeout=${idle}s -c db_roles.guard_by=${guard_by}" "$db" psql -v ON_ERROR_STOP=1 -1 -U postgres -d cortex -q
