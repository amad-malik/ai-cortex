#!/usr/bin/env bash
# ADOPTED from hale's review receipt of S5 increment 4b (reviews/S5-INC4-hale-receipts/probe-classes.sh); run by scripts/install-test.sh as IT-64.
# Extended (5d, hale's 5b note N-1) with the --home class line, and (6d-1b) with the three classes of valid_semver and read_stack_env: everything else is hers, unchanged.
# It needs bash (arrays are not used, but ${!v} and local are). A locale the host does not have makes the shell fall back to C for that round, so the round proves less there.
# hale: the spelled-out character classes of deploy/install.sh, taken VERBATIM, fed every printable ASCII character plus a few non-ASCII ones, under three locales.
# Each class must accept EXACTLY its intended alphabet. Usage: probe-classes.sh <install.sh>. A class that accepts too much or too little prints a MISMATCH line.
set -u
F=$1
T=$(mktemp -d "${TMPDIR:-/tmp}/hale-classes.XXXXXX")
line() { grep -m1 -F -- "$1" "$F"; }
L_HOME=$(line 'case $home_dir in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZ'); L_PORT=$(line 'case $port in'); L_NAME=$(line 'valid_name() {'); L_VER=$(line 'case $cvn in ""|'); L_HEX=$(line 'case $val in'); L_SEV=$(line 'case $se_val in'); L_DIG=$(line 'case $se_dig in'); L_VS=$(line 'case $vs in ""|*.part|'); L_MOD=$(line 'case $id in'); L_TRUST=$(line 'case $trust_rules in')
for v in L_HOME L_PORT L_NAME L_VER L_HEX L_MOD L_TRUST L_SEV L_DIG L_VS; do [ -n "${!v}" ] || { echo "NOT FOUND: $v"; exit 3; }; done
cat > "$T/harness.sh" <<EOF
die() { REJ=1; }
home_ok()  { REJ=0; home_dir=\$1; $L_HOME
  [ "\$REJ" = 0 ]; }
port_ok()  { REJ=0; port=\$1; $L_PORT
  [ "\$REJ" = 0 ]; }
name_ok()  { $L_NAME
  valid_name "\$1"; }
ver_ok()   { REJ=0; cvn=\$1; cant=x; $L_VER
  [ "\$REJ" = 0 ]; }
hex_ok()   { REJ=0; val=\$1; $L_HEX
  [ "\$REJ" = 0 ]; }
mod_ok()   { REJ=0; id=\$1; CORTEX_CONFIG=x; $L_MOD
  [ "\$REJ" = 0 ]; }
trust_ok() { REJ=0; trust_rules=\$1; $L_TRUST
  [ "\$REJ" = 0 ]; }
sev_ok()   { se_val=\$1; se_n=1; $L_SEV
  return 0; }
dig_ok()   { se_dig=\$1; $L_DIG
  return 0; }
vs_ok()    { vs=\$1; $L_VS
  return 0; }
accepted() {   # accepted <fn> <prefix> <suffix>: the characters c for which <fn> "<prefix>c<suffix>" succeeds, as one string
  local fn=\$1 pre=\$2 suf=\$3 out="" c i
  for i in \$(seq 32 126); do c=\$(printf "\\\\\$(printf '%03o' \$i)"); \$fn "\$pre\$c\$suf" && out="\$out\$c"; done
  for c in '１' '⁵' '٥' 'é' 'Ａ'; do \$fn "\$pre\$c\$suf" && out="\$out\$c"; done
  printf '%s' "\$out"
}
EOF
. "$T/harness.sh" 2>/dev/null || { cat "$T/harness.sh" | head -20; exit 3; }
DIG=0123456789; LET=abcdefghijklmnopqrstuvwxyz
sortc() { printf '%s' "$1" | fold -w1 | LC_ALL=C sort | tr -d '\n'; }
chk() {   # chk <label> <got> <want>
  local g w; g=$(sortc "$2"); w=$(sortc "$3")
  if [ "$g" = "$w" ]; then echo "   ok       $1"; else echo "   MISMATCH $1: accepts [$g] but should accept [$w]"; BAD=1; fi
}
BAD=0
for loc in C C.utf8 en_AU.utf8; do
  echo "locale $loc:"
  export LC_ALL=$loc
  chk "--home, a character in the middle of a path"       "$(accepted home_ok /h x)" "${LET}ABCDEFGHIJKLMNOPQRSTUVWXYZ${DIG}._/-"
  chk "port: every character of a number"                 "$(accepted port_ok 1 1)"  "$DIG"
  chk "trust count: every character of a number"          "$(accepted trust_ok 1 1)" "$DIG"
  chk "name, first letter"                                "$(accepted name_ok '' x)" "$LET"
  chk "name, later characters"                            "$(accepted name_ok a a)"  "$LET$DIG-"
  chk "version, a character between two digits"           "$(accepted ver_ok 1 1)"   "$DIG."
  chk "random bytes (hex), every character"               "$(accepted hex_ok 1 1)"   "${DIG}abcdef"
  chk "stack.env value, every character"                 "$(accepted sev_ok 1 1)"   "${LET}ABCDEFGHIJKLMNOPQRSTUVWXYZ${DIG}._:/@-"
  chk "stack.env digest, every character"                 "$(accepted dig_ok 1 1)"   "${DIG}abcdef"
  chk "SemVer, every character of a pre-release"          "$(accepted vs_ok 1.2.3-a b)" "${LET}ABCDEFGHIJKLMNOPQRSTUVWXYZ${DIG}.-"
  chk "module id, first letter"                           "$(accepted mod_ok '' x)"  "$LET"
  chk "module id, later characters"                       "$(accepted mod_ok a a)"   "$LET${DIG}_"
done
unset LC_ALL
echo "result: $([ "$BAD" = 0 ] && echo 'every class accepts exactly its alphabet' || echo 'AT LEAST ONE CLASS IS WRONG')"
rm -rf -- "$T"
