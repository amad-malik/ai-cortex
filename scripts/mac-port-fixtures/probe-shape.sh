#!/bin/bash
# probe-shape.sh: a fixture of mac-port-test.sh. It reports the shape it runs under, and whether the shape reaches a child `sh` (the runner puts a
# shim `sh` first in PATH). It changes nothing and calls nothing but a child sh that only echoes.
self_posix=no
if [[ -o posix ]]; then self_posix=yes; fi
echo "shape=${MAC_PORT_SHAPE:-none}"
echo "args=$*"
echo "self_posix=$self_posix"
echo "self_compat=${BASH_COMPAT:-none}"
echo "child_sh=$(command -v sh)"
sh -c 'p=no; case ":$SHELLOPTS:" in *:posix:*) p=yes ;; esac; echo "child_posix=$p"; echo "child_compat=${BASH_COMPAT:-none}"'
