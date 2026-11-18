#!/bin/sh
# A fresh fake installer process must recompute its own findings.
RUN_INCLUDE=$1
. "$RUN_INCLUDE"
say() { printf '%s\n' "$*"; }
warn() { :; }
EX_PREREQ=3 EX_MACHINE=2 home_dir=$HOME port=8601
prereq_pgp_fingerprints() {
  case $1 in
    */certificate.key) printf '%s\n' 7D8D15CBFC4E62688591FB2633D98517E37ED158 ;;
    *) "$FAKE_BIN/sh" -c '. "$1"; prereq_pgp_fingerprints "$2"' sh "$RUN_INCLUDE" "$1" ;;
  esac
}
machine_look || exit $?
case $PREREQ_MISSING in
  *gnupg2*) ;;
  *) printf '%s\n' 'FAIL SAM-03: fresh machine look did not rediscover gnupg2' >&2; exit 97 ;;
esac
prereq_run
