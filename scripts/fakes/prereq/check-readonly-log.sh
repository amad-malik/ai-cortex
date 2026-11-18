#!/bin/sh
# Reject all loginctl and podman operations except the documented read-only probes.
set -eu
[ "$#" -eq 1 ] || exit 2
LC_ALL=C awk '
  /^loginctl( |$)/ && $2 != "<show-user>" { bad = 1 }
  /^podman( |$)/ && $2 != "<--version>" && $2 != "<info>" { bad = 1 }
  END { exit bad ? 1 : 0 }
' "$1"
