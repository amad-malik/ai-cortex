#!/usr/bin/env python3
"""A private, filesystem-only Podman model for the supervisor volume controls."""
import json
import os
import sys
import time
from pathlib import Path

d = Path(os.environ["VOLUME_FAKE_DIR"])
state_file = d / "state.json"
state = json.loads(state_file.read_text()) if state_file.exists() else {"volumes": {}, "containers": {}, "runs": 0}
a = sys.argv[1:]
with (d / "calls.jsonl").open("a") as f:
    f.write(json.dumps(a) + "\n")

def save():
    state_file.write_text(json.dumps(state))

def option(flag):
    return a[a.index(flag) + 1]

fault = os.environ.get("VOLUME_FAKE_FAULT", "")
injected = os.environ.get("CV2_FAKE", "").split()
if (a[:2] == ["container", "exists"] and "exists=fail" in injected) or (a[0] == "ps" and "ps=fail" in injected):
    sys.exit(125)
if a[:2] == ["volume", "create"]:
    if fault == "create-fail":
        sys.exit(125)
    name = a[-1]
    labels = [a[i + 1] for i, v in enumerate(a) if v == "--label"]
    state["volumes"][name] = labels
    save()
    if fault == "create-slow":
        (d / "created").touch()
        time.sleep(1)
    print(name)
elif a[:2] == ["volume", "exists"]:
    if fault == "volume-exists-fail":
        sys.exit(125)
    sys.exit(0 if a[-1] in state["volumes"] else 1)
elif a[:2] == ["volume", "inspect"]:
    labels = state["volumes"].get(a[-1])
    if labels is None or fault == "volume-inspect-fail":
        sys.exit(125)
    print(next((s.split("=", 1)[1] for s in labels if s.startswith("cv2.owner=")), ""))
elif a[:2] == ["volume", "ls"]:
    if fault == "volume-list-fail" or (fault == "volume-list-fail-after" and state["runs"] and not state["containers"]):
        sys.exit(125)
    label = option("--filter").removeprefix("label=")
    for name, labels in state["volumes"].items():
        if label in labels:
            print(name)
elif a[:2] == ["volume", "rm"]:
    if fault != "volume-rm-noop":
        state["volumes"].pop(a[-1], None)
        save()
elif a[0] == "run":
    state["runs"] += 1
    name = option("--name")
    mounts = [a[i + 1] for i, v in enumerate(a) if v in ("--volume", "-v")]
    if not any(m.endswith(":/var/lib/postgresql") for m in mounts):
        state["volumes"]["anonymous-" + str(state["runs"])] = []
    if fault == "collision" and state["runs"] == 1:
        save()
        print("Address already in use", file=sys.stderr)
        sys.exit(125)
    if fault == "run-fail":
        save()
        sys.exit(125)
    labels = [a[i + 1] for i, v in enumerate(a) if v == "--label"]
    state["containers"][name] = labels
    save()
elif a[0] == "exec":
    if fault == "not-ready" and "pg_isready" in a:
        sys.exit(1)
    if "postgres" in a:
        print("postgres (PostgreSQL) 18.0")
elif a[0] in ("stop", "rm"):
    if a[0] == "stop" and fault == "cleanup-slow":
        time.sleep(0.5)
    if not (a[0] == "rm" and fault == "container-rm-noop"):
        state["containers"].pop(a[-1], None)
        save()
elif a[:2] == ["container", "inspect"]:
    labels = state["containers"].get(a[-1])
    if labels is None or fault == "container-inspect-fail":
        sys.exit(125)
    print(next((s.split("=", 1)[1] for s in labels if s.startswith("cv2.owner=")), ""))
elif a[:2] == ["container", "exists"]:
    if fault == "container-observe-fail":
        sys.exit(125)
    sys.exit(0 if a[-1] in state["containers"] else 1)
elif a[0] == "ps":
    if fault == "container-observe-fail":
        sys.exit(125)
    selected = option("--filter")
    if selected.startswith("volume="):
        volume = selected.removeprefix("volume=")
        owner_container = volume.removesuffix("-pgdata")
        if owner_container in state["containers"] or volume in state.get("references", []):
            print(owner_container)
        sys.exit(0)
    label = selected.removeprefix("label=")
    for name, labels in state["containers"].items():
        if label in labels:
            print(name)
else:
    print("unsupported fake podman call: " + repr(a), file=sys.stderr)
    sys.exit(125)
