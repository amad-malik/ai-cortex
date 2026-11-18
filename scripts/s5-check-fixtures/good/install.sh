#!/bin/sh
# Fixture: a GOOD S5 install script. The real one is deploy/install.sh.
set -eu
umask 077
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
[ ! -f "$here/install-prereq.sh" ] || . "$here/install-prereq.sh"   # @include
command -v podman >/dev/null 2>&1 || { echo "install: podman is required" >&2; exit 1; }
make_secret() {
  # a random value straight into a Podman secret: never in argv, in a file, or on the terminal
  name=$1
  if podman secret inspect "$name" >/dev/null 2>&1; then return 0; fi
  head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n' | podman secret create "$name" - >/dev/null
}
make_secret cv2-s5-db-superuser
make_secret cv2-s5-owner-password
make_secret cv2-s5-app-password
podman compose -f "$here/compose.yml" up -d
