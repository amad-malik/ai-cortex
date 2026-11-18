#!/usr/bin/env python3
"""Interrupt only this test's private process group while a stage is live."""
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import time

if len(sys.argv) not in (5, 6) or (len(sys.argv) == 6 and sys.argv[5] not in ("C", "C.utf8")):
    raise SystemExit("FAIL PRQ-SAM01: invalid helper arguments")
include, shell, fake_bin, directory = sys.argv[1:5]
locale = sys.argv[5] if len(sys.argv) > 5 else "C"
directory = Path(directory)
home = directory / "signal-home"
home.mkdir(mode=0o700)
fifo = directory / "signal-fifo"
marker = directory / "signal-ready"
os.mkfifo(fifo)
env = {"PATH": fake_bin, "HOME": str(home), "LC_ALL": locale, "SIGNAL_FIFO": str(fifo), "SIGNAL_READY": str(marker),
       "FAKE_LOG": str(directory / "log"), "FAKE_STATE_DIR": str(directory / "state")}
program = r'''
. "$1"
say() { :; }
prereq_os_ok() { printf '%s\n' ready > "$SIGNAL_READY"; IFS= read -r unused < "$SIGNAL_FIFO"; }
EX_PREREQ=3
prereq_requalify_compose
'''
proc = subprocess.Popen([shell, "-c", program, "sh", include], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                        start_new_session=True)
handles = []
try:
    deadline = time.monotonic() + 10
    while not marker.exists() and proc.poll() is None and time.monotonic() < deadline:
        time.sleep(0.02)
    if not marker.exists() or proc.poll() is not None:
        raise RuntimeError("test did not reach the live private stage")
    # The marker is written by the inner command-substitution shell. The outer
    # shell can exit first, so waiting only for proc does not await EXIT cleanup.
    # Capture handles while both are blocked and the private session is live.
    for entry in Path('/proc').iterdir():
        if not entry.name.isdecimal():
            continue
        member = int(entry.name)
        handle = None
        try:
            if os.getpgid(member) != proc.pid or entry.stat().st_uid != os.getuid():
                continue
            handle = os.pidfd_open(member)
            if os.getpgid(member) != proc.pid:
                os.close(handle)
                continue
        except ProcessLookupError:
            if handle is not None:
                os.close(handle)
            continue
        handles.append(handle)
    if len(handles) < 2:
        raise RuntimeError("test did not capture both private cleanup shells")
    deadline = time.monotonic() + 10
    os.killpg(proc.pid, signal.SIGTERM)
    try:
        writer = os.open(fifo, os.O_WRONLY | os.O_NONBLOCK)
        os.write(writer, b"stop\n")
        os.close(writer)
    except OSError:
        pass
    proc.wait(timeout=max(0, deadline - time.monotonic()))
    pending = set(handles)
    poller = select.poll()
    for handle in handles:
        poller.register(handle, select.POLLIN)
    while pending:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise RuntimeError("private cleanup shells did not exit within 10 seconds")
        for handle, event in poller.poll(max(1, int(remaining * 1000))):
            if event & select.POLLIN:
                pending.remove(handle)
                poller.unregister(handle)
    # One assertion after actual completion, never retry stage absence to green.
    if list(home.glob(".cortex-prereq.*")):
        raise RuntimeError("private stage remains after interrupt")
except (OSError, RuntimeError, subprocess.TimeoutExpired) as error:
    for handle in handles:
        try:
            signal.pidfd_send_signal(handle, signal.SIGKILL)
        except ProcessLookupError:
            pass
    if proc.poll() is None:
        proc.kill()
        proc.wait()
    raise SystemExit("FAIL W1-73 signal cleanup: " + str(error))
finally:
    for handle in handles:
        os.close(handle)
    fifo.unlink(missing_ok=True)
    marker.unlink(missing_ok=True)
print("ok W1-73 interrupted provider verification removes private stage")
