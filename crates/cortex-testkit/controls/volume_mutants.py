#!/usr/bin/env python3
"""W1-305/305a: mutate private supervisor or control copies and require the named cell to reject each."""
import os
import subprocess
import sys
import tempfile
from pathlib import Path

root = Path(__file__).resolve().parents[1]
source = Path(os.environ.get('VOLUME_SUPERVISOR', root / 'supervise.sh')).read_text()
controls_source = Path(os.environ.get('VOLUME_CONTROLS', root / 'controls/controls.sh')).read_text()
run_anchor = 'log "podman run failed or ran out of its ${START_S} s (attempt $ATTEMPTS of 3, rc $RUN_RC)"; trap - EXIT; finish 72'
ready_anchor = 'log "not ready within ${READY_S} s ($(( NOW - R0 )) ms)"; trap - EXIT; finish 72'
mutants = [
    ('missing mount', 'test_success', [('--volume "$VOLUME:/var/lib/postgresql"', '')]),
    ('driver failure skips cleanup', 'test_driver_failure', [('finish "$DRIVER_RC"', 'exit "$DRIVER_RC"')]),
    ('collision does not retry', 'test_port_collision', [('log "start: port collision on attempt $ATTEMPTS; retrying on a new port"\n    continue', 'log "start: port collision on attempt $ATTEMPTS; retrying on a new port"')]),
    ('start failure skips cleanup', 'test_start_failure', [(run_anchor, run_anchor.replace('finish 72', 'exit 72'))]),
    ('readiness failure skips cleanup', 'test_readiness_timeout', [(ready_anchor, ready_anchor.replace('finish 72', 'exit 72'))]),
    ('TERM skips cleanup', 'test_sigterm', [("trap 'on_signal TERM 143' TERM", "trap 'exit 143' TERM")]),
    ('INT skips cleanup', 'test_sigint', [("trap 'on_signal INT 130' INT", "trap 'exit 130' INT")]),
    ('second signal interrupts cleanup', 'test_second_signal_during_cleanup', [("trap 'defer INT 130' INT", "trap 'exit 130' INT")]),
    ('creation signal leaves volume', 'test_signal_during_volume_create', [('VOLUME_ELIGIBLE=1\nif ! run_capped', 'VOLUME_ELIGIBLE=0\nif ! run_capped')]),
    ('unrelated volume removed', 'test_unrelated_volume_is_preserved', [('podman volume rm "$VOLUME"', 'podman volume rm other-volume; podman volume rm "$VOLUME"')]),
    ('remaining volume treated as clean', 'test_volume_removal_failure_is_not_success', [('present) CLEAN_RC=70; log "cleanup: FAILED, volume', 'present) CLEAN_RC=0; log "cleanup: FAILED, volume')]),
    ('unknown volume treated as clean', 'test_volume_observation_failure_is_not_success', [ ('*)       CLEAN_RC=71; log "cleanup: volume', '*)       CLEAN_RC=0; log "cleanup: volume')]),
    ('unverified container permits removal', 'test_unknown_container_does_not_remove_volume', [('[ "$OBS" = absent ] || return', ': # mutant removes despite unknown container')]),
    ('creation failure ignored', 'test_create_failure_stops_before_run', [('if ! run_capped "$START_DL" 20 podman volume create', 'if false && ! run_capped "$START_DL" 20 podman volume create'), ('if [ "$VOLUME_OBS" != present ]; then', 'if false; then')]),
]
control_mutants = [
    ('K3 follow-up leaves volume', 'test_followup_k3', '"$REAL" volume rm "$__v" >/dev/null 2>&1; __rc=$?', 'true; __rc=0'),
    ('K9 follow-up leaves volume', 'test_followup_k9', '"$REAL" volume rm "$__v" >/dev/null 2>&1; __rc=$?', 'true; __rc=0'),
    ('follow-up ignores owner mismatch', 'test_followup_foreign_owner_refused', 'if [ "$__vo" != "$__co" ]; then', 'if false; then'),
    ('follow-up ignores volume still present', 'test_followup_volume_removal_failure_counted', 'if [ "$__rc" = 0 ] && [ "$__exists" = 1 ]; then', 'if [ "$__rc" = 0 ]; then'),
    ('follow-up removes volume with container present', 'test_followup_present_container_preserves_volume', 'if [ "$__s" != absent ]; then FAILS=$((FAILS + 1)); return; fi', ':'),
    ('general per-case follow-up removed', 'test_general_followup_k4', '  general_volume_followup "$k"\n', '  : # general follow-up removed\n'),
    ('general follow-up ignores owner mismatch', 'test_general_foreign_owner_refused', 'if [ "$__actual" != "$__owner" ]; then', 'if false; then'),
    ('general follow-up ignores container references', 'test_general_referenced_volume_refused', 'if ! "$REAL" ps -a --filter "volume=$__v" --format \'{{.Names}}\' > "$__base.volume-refs" 2>/dev/null || [ -s "$__base.volume-refs" ]; then', 'if false; then'),
]
failed = []
with tempfile.TemporaryDirectory(prefix='volume-mutants-', dir=os.environ['TMPDIR']) as tmp:
    for number, (name, cell, edits) in enumerate(mutants, 1):
        text = source
        for old, new in edits:
            n = text.count(old)
            # Signal traps are armed twice; mutate both phases.
            expected = 2 if old.startswith(("trap 'on_signal ", "trap 'defer ")) else 1
            if n != expected:
                failed.append(name + ': anchor count %d, expected %d' % (n, expected))
                break
            text = text.replace(old, new)
        else:
            mutant = Path(tmp) / ('mutant-%02d.sh' % number)
            mutant.write_text(text)
            env = dict(os.environ, VOLUME_SUPERVISOR=str(mutant))
            p = subprocess.run([sys.executable, '-I', '-B', str(root / 'controls/test_volume_lifecycle.py'), 'VolumeLifecycle.' + cell], env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=60)
            dead = p.returncode != 0 and ('FAIL: ' + cell) in p.stdout and 'ERROR:' not in p.stdout
            print(('%s %02d %s: %s' % ('PASS' if dead else 'FAIL', number, name, cell)), flush=True)
            if not dead:
                failed.append(name + ': mutant survived or its cell errored: ' + p.stdout[-1000:])
    for number, (name, cell, old, new) in enumerate(control_mutants, len(mutants) + 1):
        if controls_source.count(old) != 1:
            failed.append(name + ': anchor count %d, expected 1' % controls_source.count(old))
            continue
        mutant = Path(tmp) / ('controls-mutant-%02d.sh' % number)
        mutant.write_text(controls_source.replace(old, new))
        env = dict(os.environ, VOLUME_CONTROLS=str(mutant))
        p = subprocess.run([sys.executable, '-I', '-B', str(root / 'controls/test_volume_lifecycle.py'), 'VolumeLifecycle.' + cell], env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=60)
        dead = p.returncode != 0 and ('FAIL: ' + cell) in p.stdout and 'ERROR:' not in p.stdout
        print(('%s %02d %s: %s' % ('PASS' if dead else 'FAIL', number, name, cell)), flush=True)
        if not dead:
            failed.append(name + ': mutant survived or its cell errored: ' + p.stdout[-1000:])
for problem in failed:
    print('FAIL ' + problem)
total = len(mutants) + len(control_mutants)
print('volume mutants: %d/%d killed' % (total - len(failed), total))
sys.exit(bool(failed))
