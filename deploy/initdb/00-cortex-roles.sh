#!/bin/sh
# S5 first-start script. The postgres image runs /docker-entrypoint-initdb.d once, on an empty data volume.
# It creates the S1 section 9 login roles with passwords read from Podman secrets. The module roles
# (cortex_m_<module>) come from the migrations, not from here (P-1 CONTRACT H1).
#
# The passwords never appear in argv, in a file this script writes, or in the server log: psql reads
# each one with a backtick command, and statement logging is switched off for this session.
#
# This runs ONLY on an empty data volume (nix N8). Changing a Podman secret later does not change the role's password in an existing
# database: run ALTER ROLE ... PASSWORD in the database first, then recreate the secret, then restart the stack.
#
# cortex_owner has NO standing CREATEROLE (cox and vera). The cortex_m_<module> roles are created by deploy/db-roles.sh, a superuser
# step that install.sh and `cortex upgrade` run after the database is healthy and before the migrations.
set -eu
psql -v ON_ERROR_STOP=1 --username "${POSTGRES_USER:-postgres}" --dbname "${POSTGRES_DB:-cortex}" <<'SQL'
SET log_statement = 'none';
SET log_min_error_statement = 'panic';
\set owner_pw `cat /run/secrets/cv2-s5-owner-password`
\set app_pw `cat /run/secrets/cv2-s5-app-password`
CREATE ROLE cortex_owner LOGIN PASSWORD :'owner_pw';
CREATE ROLE cortex_app LOGIN NOINHERIT PASSWORD :'app_pw';
CREATE ROLE cortex_install_admin NOLOGIN;
GRANT cortex_install_admin TO cortex_app;
ALTER DATABASE cortex OWNER TO cortex_owner;
REVOKE ALL ON DATABASE cortex FROM PUBLIC;
GRANT CONNECT ON DATABASE cortex TO cortex_owner, cortex_app;
SQL
