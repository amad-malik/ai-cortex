#!/bin/sh
# Fixture: constructs the NEVER-list lint (S5-15) must NOT flag. Each one is allowed on purpose.
set -eu
token="$HOME/lead.token"
echo "token file $token"                                                 # prints a PATH, not a value
printf '%s' "$val" | podman secret create cv2-s5-x - >/dev/null          # a secret piped into podman
podman exec cv2-s5-server cat /run/cortex/bootstrap.token > "$token.part" # redirected into a file
kill "$pid"                                                              # one exact pid, no -9
kill -TERM "$pid"                                                        # a named signal to one pid
kill -"$sig" "$pid"                                                      # a signal chosen by variable, still one pid
podman ps -q                                                             # listing is fine
cat "$part" > "$token.copy"                                              # a redirected cat of a path variable
kill -0 "$pid"                                                           # a liveness check, not a signal
podman rm cv2-s5-server                                                  # one named container
podman compose -f compose.yml down                                       # no volumes
git log --oneline
cat "$FAKE_STATE/bootstrap.token"   # s5-never: ok a fixture line that proves an allowed line is accepted and listed as a NOTE
