#!/bin/sh
# VR3-01: a locale diagnostic cannot be hidden by exit-zero fallback.
set -eu
suite=${1:?suite path required}
scratch=${TMPDIR:?scratch required}
case $scratch in /tmp|/tmp/*|/var/tmp|/var/tmp/*) exit 2 ;; esac
work=$(mktemp -d "$scratch/locale-status-zero.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT HUP INT TERM
mkdir -p "$work/bin"
cat > "$work/bin/locale" <<'FAKE_LOCALE'
#!/bin/sh
if [ "${1-}" = -a ]; then
  /usr/bin/locale -a || exit "$?"
  printf '%s\n' fake_status_zero
  exit 0
fi
if [ "${LC_ALL-}" = fake_status_zero ] && [ "${1-}" = charmap ]; then
  printf '%s\n' ANSI_X3.4-1968
  printf '%s\n' 'locale: Cannot set LC_ALL: fake unavailable locale' >&2
  exit 0
fi
exec /usr/bin/locale "$@"
FAKE_LOCALE
chmod 700 "$work/bin/locale"
actual=0
PATH="$work/bin:$PATH" TMPDIR="$scratch" PREREQ_TEST_TMP="$scratch" \
  PREREQ_TEST_LOCALES='C C.utf8 fake_status_zero' sh "$suite" > "$work/out" 2>&1 || actual=$?
if [ "$actual" -eq 0 ] || ! grep -Fq 'FAIL IP-53: locale matrix requested unavailable locale fake_status_zero' "$work/out"; then
  printf 'FAIL VR3-01: zero-status locale diagnostic was not refused (status %s)\n' "$actual" >&2
  cat "$work/out" >&2
  exit 1
fi
printf '%s\n' 'PASS VR3-01: zero-status locale diagnostic is refused'
