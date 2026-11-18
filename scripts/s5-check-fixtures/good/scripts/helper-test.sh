#!/bin/sh
# Fixture: a test script may read FAKE tokens; the destructive-verb rules still apply to it.
set -eu
first=$(cat "$T/h/lead.token")
