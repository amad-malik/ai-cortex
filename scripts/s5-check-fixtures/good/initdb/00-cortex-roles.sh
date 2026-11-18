#!/bin/sh
# Fixture: a GOOD S5 first-start script. The real one is deploy/initdb/00-cortex-roles.sh.
set -eu
psql -v ON_ERROR_STOP=1 --username postgres --dbname cortex <<'SQL'
\set owner_pw `cat /run/secrets/cv2-s5-owner-password`
\set app_pw `cat /run/secrets/cv2-s5-app-password`
CREATE ROLE cortex_owner LOGIN PASSWORD :'owner_pw';
CREATE ROLE cortex_app LOGIN NOINHERIT PASSWORD :'app_pw';
SQL
