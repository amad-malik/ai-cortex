#!/bin/sh
# RF-2: trust-tool configuration does not enter a hardened wrapper, and the
# caller's set/unset values are restored without reading a configuration file.
set -eu
source_file=$1
if ! sh -c '
  . "$1"
  probe() { [ "${OPENSSL_CONF+x}${OPENSSL_MODULES+x}${OPENSSL_ENGINES+x}" = "" ]; }
  OPENSSL_CONF=synthetic-conf OPENSSL_MODULES=synthetic-modules OPENSSL_ENGINES=synthetic-engines
  export OPENSSL_CONF OPENSSL_MODULES OPENSSL_ENGINES
  prereq_secure_call probe || exit 31
  [ "$OPENSSL_CONF" = synthetic-conf ] && [ "$OPENSSL_MODULES" = synthetic-modules ] && [ "$OPENSSL_ENGINES" = synthetic-engines ] || exit 32
  unset OPENSSL_CONF OPENSSL_MODULES OPENSSL_ENGINES
  prereq_secure_call probe || exit 33
  [ "${OPENSSL_CONF+x}${OPENSSL_MODULES+x}${OPENSSL_ENGINES+x}" = "" ] || exit 34
' sh "$source_file"; then
  printf '%s\n' 'FAIL RF-2: OpenSSL configuration was not isolated and restored' >&2
  exit 1
fi
printf '%s\n' 'PASS RF-2: OpenSSL configuration isolated and caller state restored'
