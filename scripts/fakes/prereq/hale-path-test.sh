#!/bin/sh
# F-1: all executables are synthetic; no sudo, package, network or systemd calls.
set -eu
: "${TMPDIR:?private scratch required}"
case $TMPDIR in /tmp|/tmp/*) exit 2 ;; esac
include=$1
cleanup_rm=$(command -v rm)
work=$(mktemp -d "$TMPDIR/prereq-path.XXXXXXXX")
trap '"$cleanup_rm" -rf -- "$work"' EXIT HUP INT TERM
mkdir "$work/trusted" "$work/hostile" "$work/victim"
ln -s "$(command -v awk)" "$work/trusted/awk"
cat > "$work/trusted/stat" <<'TRUSTED'
#!/bin/sh
printf '%s\n' '1000|755'
TRUSTED
cat > "$work/hostile/stat" <<'HOSTILE'
#!/bin/sh
printf '%s\n' '0|755'
HOSTILE
chmod 700 "$work/trusted/stat" "$work/hostile/stat"
# The production path is fixed; replace that literal only in this private copy.
# This preserves an isolated fake-only runtime and does not introduce a test hook.
sed "s|/usr/bin:/usr/sbin|$work/trusted|g" "$include" > "$work/include.sh"
. "$work/include.sh"
probe() {
  if prereq_root_locked_path "$work/victim"; then printf '%s\n' 'FAIL F-1: hostile stat was trusted' >&2; return 1; fi
  for v in LD_PRELOAD LD_LIBRARY_PATH LD_AUDIT PYTHONPATH PYTHONHOME PYTHONSTARTUP BASH_ENV ENV CDPATH GCONV_PATH; do
    case $v in
      LD_PRELOAD) [ "${LD_PRELOAD+x}" != x ] ;;
      LD_LIBRARY_PATH) [ "${LD_LIBRARY_PATH+x}" != x ] ;;
      LD_AUDIT) [ "${LD_AUDIT+x}" != x ] ;;
      PYTHONPATH) [ "${PYTHONPATH+x}" != x ] ;;
      PYTHONHOME) [ "${PYTHONHOME+x}" != x ] ;;
      PYTHONSTARTUP) [ "${PYTHONSTARTUP+x}" != x ] ;;
      BASH_ENV) [ "${BASH_ENV+x}" != x ] ;;
      ENV) [ "${ENV+x}" != x ] ;;
      CDPATH) [ "${CDPATH+x}" != x ] ;;
      GCONV_PATH) [ "${GCONV_PATH+x}" != x ] ;;
    esac || { printf 'FAIL F-1: %s remained set\n' "$v" >&2; return 1; }
  done
  PROBE_RESULT=preserved
}
PATH="$work/hostile:$work/trusted"; export PATH
LD_LIBRARY_PATH=untrusted; PYTHONPATH=untrusted; GCONV_PATH=untrusted
export LD_LIBRARY_PATH PYTHONPATH GCONV_PATH
if command -v prereq_secure_call >/dev/null 2>&1; then prereq_secure_call probe; else probe; fi
[ "$PATH" = "$work/hostile:$work/trusted" ] && [ "$LD_LIBRARY_PATH" = untrusted ] && [ "$PYTHONPATH" = untrusted ] && [ "$GCONV_PATH" = untrusted ] && [ "$PROBE_RESULT" = preserved ] || { printf '%s\n' 'FAIL F-1: caller state changed' >&2; exit 1; }
machine_look_body() { probe; }
prereq_run_body() { probe; }
prereq_requalify_compose_body() { probe; }
machine_look
prereq_run
prereq_requalify_compose
[ "$PATH" = "$work/hostile:$work/trusted" ] && [ "$LD_LIBRARY_PATH" = untrusted ] && [ "$PYTHONPATH" = untrusted ] && [ "$GCONV_PATH" = untrusted ] || { printf '%s\n' 'FAIL F-1: wrapper changed caller environment' >&2; exit 1; }
printf '%s\n' 'PASS F-1: trusted tools selected, injection variables absent, caller state restored'
