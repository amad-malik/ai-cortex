#!/bin/sh
# VR3-01: the suite must reject even when every locale probe fails.
set -eu
suite=${1:?suite path required}
scratch=${TMPDIR:?scratch required}
case $scratch in /tmp|/tmp/*|/var/tmp|/var/tmp/*) exit 2 ;; esac
work=$(mktemp -d "$scratch/locale-all-skipped.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT HUP INT TERM
mkdir -p "$work/bin"
cat > "$work/bin/locale" <<'FAKE_LOCALE'
#!/bin/sh
if [ "${LC_ALL-}" = does_not_exist_helix_locale ] && [ "${1-}" = charmap ]; then
  printf '%s\n' 'locale: fake unavailable locale' >&2
  exit 1
fi
exec /usr/bin/locale "$@"
FAKE_LOCALE
chmod 700 "$work/bin/locale"
actual=0
PATH="$work/bin:$PATH" TMPDIR="$scratch" PREREQ_TEST_TMP="$scratch" \
  PREREQ_TEST_LOCALES=does_not_exist_helix_locale sh "$suite" > "$work/out" 2>&1 || actual=$?
if [ "$actual" -eq 0 ] || ! grep -Fq 'FAIL IP-53: locale matrix' "$work/out"; then
  printf 'FAIL VR3-01: all-skipped direct suite returned %s without matrix refusal\n' "$actual" >&2
  tail -10 "$work/out" >&2
  exit 1
fi
printf '%s\n' 'PASS VR3-01: all-skipped direct suite rejects incomplete matrix'
