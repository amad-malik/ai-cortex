#!/bin/sh
# Kai 5f13b475: use the pinned Gitleaks binary and fail on a missing or changed tool.
set -eu
scanner=${CORTEX_GITLEAKS_BIN:-/path/to/workspace/scratch/cox/d182/gitleaks/bin/gitleaks}
case $scanner in /*) ;; *) echo 'secret-scan: scanner path must be absolute' >&2; exit 1 ;; esac
[ -f "$scanner" ] && [ ! -L "$scanner" ] && [ -x "$scanner" ] || { echo 'secret-scan: pinned scanner missing or not executable' >&2; exit 1; }
actual=$(/usr/bin/sha256sum -- "$scanner") || { echo 'secret-scan: cannot hash scanner' >&2; exit 1; }
actual=${actual%% *}
[ "$actual" = 88f91962aa2f93ac6ab281d553b9e125f5197bbbce38f9f2437f7299c32e5509 ] || { echo 'secret-scan: scanner digest mismatch' >&2; exit 1; }
# Scan the checked-out commit's reachable history, not unrelated refs in a shared repository.
exec "$scanner" detect --source . --config .gitleaks.toml --log-opts=HEAD --no-banner --redact
