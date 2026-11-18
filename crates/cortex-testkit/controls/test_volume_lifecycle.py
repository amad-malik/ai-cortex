#!/usr/bin/env python3
"""W1-305 regression cells: actual supervisor with an isolated, fake Podman store."""
import json
import os
import re
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FAKE = ROOT / "controls/volume_fake_podman.py"


class VolumeLifecycle(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="volume-cells-", dir=os.environ["TMPDIR"])
        self.dir = Path(self.tmp.name)
        (self.dir / "bin").mkdir()
        shutil.copyfile(FAKE, self.dir / "bin/podman")
        (self.dir / "bin/podman").chmod(0o755)
        shutil.copyfile(ROOT / "group_runner.py", self.dir / "group_runner.py")
        source = Path(os.environ.get("VOLUME_SUPERVISOR", ROOT / "supervise.sh")).read_text()
        # Preserve the production control flow while shortening the readiness and cleanup budgets.
        source = source.replace("START_S=60 READY_S=30 CLEANUP_S=120", "START_S=10 READY_S=2 CLEANUP_S=5")
        self.sup = self.dir / "supervise.sh"
        self.sup.write_text(source)
        self.sup.chmod(0o755)
        self.env = dict(os.environ, VOLUME_FAKE_DIR=str(self.dir), PATH=str(self.dir / "bin") + os.pathsep + os.environ["PATH"])

    def tearDown(self):
        self.tmp.cleanup()

    def run_case(self, fault="", driver=("true",), sig=None, second_sig=None, trigger="ready"):
        env = dict(self.env, VOLUME_FAKE_FAULT=fault)
        p = subprocess.Popen([str(self.sup), *driver], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        def send(value):
            p.send_signal(value)
        if sig:
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                log = (self.dir / "calls.jsonl").read_text() if (self.dir / "calls.jsonl").exists() else ""
                if (trigger == "ready" and '"pg_isready"' in log) or (trigger == "create" and (self.dir / "created").exists()):
                    break
                time.sleep(0.02)
            else:
                p.terminate()
                p.communicate(timeout=5)
                self.fail("supervisor did not reach readiness")
            send(sig)
            if second_sig:
                deadline = time.monotonic() + 10
                while time.monotonic() < deadline:
                    if '["stop",' in (self.dir / "calls.jsonl").read_text():
                        break
                    time.sleep(0.02)
                else:
                    p.communicate(timeout=20)
                    self.fail("supervisor did not begin cleanup")
                send(second_sig)
        out, err = p.communicate(timeout=25)
        state = json.loads((self.dir / "state.json").read_text()) if (self.dir / "state.json").exists() else {"volumes": {}, "containers": {}, "runs": 0}
        calls = [json.loads(x) for x in (self.dir / "calls.jsonl").read_text().splitlines()]
        return p.returncode, state, calls, out + err

    def check_clean(self, fault="", driver=("true",), expected=0, sig=None):
        rc, state, calls, log = self.run_case(fault, driver, sig)
        self.assertEqual(rc, expected, log)
        self.assertEqual(state["volumes"], {}, log)
        self.assertEqual(state["containers"], {}, log)
        creates = [a for a in calls if a[:2] == ["volume", "create"]]
        self.assertEqual(len(creates), 1, calls)
        name = creates[0][-1]
        self.assertIn("cortexv2=dev", creates[0])
        self.assertTrue(any(x.startswith("cv2.owner=f1-") for x in creates[0]))
        runs = [a for a in calls if a[0] == "run"]
        for call in runs:
            self.assertIn(name + ":/var/lib/postgresql", call)
        self.assertIn(name, log)
        return calls

    def test_success(self):
        self.check_clean()

    def test_driver_failure(self):
        self.check_clean(driver=("false",), expected=1)

    def test_port_collision(self):
        calls = self.check_clean("collision")
        self.assertEqual(sum(a[0] == "run" for a in calls), 2)

    def test_start_failure(self):
        self.check_clean("run-fail", expected=72)

    def test_readiness_timeout(self):
        self.check_clean("not-ready", expected=72)

    def test_sigterm(self):
        self.check_clean(driver=("sleep", "10"), expected=143, sig=signal.SIGTERM)

    def test_sigint(self):
        self.check_clean(driver=("sleep", "10"), expected=130, sig=signal.SIGINT)

    def test_second_signal_during_cleanup(self):
        rc, state, _, log = self.run_case("cleanup-slow", ("sleep", "10"), signal.SIGTERM, signal.SIGINT)
        self.assertEqual(rc, 143, log)
        self.assertEqual(state["volumes"], {}, log)
        self.assertIn("deferred until cleanup", log)

    def test_signal_during_volume_create(self):
        rc, state, _, log = self.run_case("create-slow", ("true",), signal.SIGTERM, trigger="create")
        self.assertEqual(rc, 143, log)
        self.assertEqual(state["volumes"], {}, log)

    def test_unrelated_volume_is_preserved(self):
        (self.dir / "state.json").write_text(json.dumps({"volumes": {"other-volume": ["cv2.owner=another-run"]}, "containers": {}, "runs": 0}))
        rc, state, calls, log = self.run_case()
        self.assertEqual(rc, 0, log)
        self.assertEqual(state["volumes"], {"other-volume": ["cv2.owner=another-run"]}, log)
        self.assertFalse(any(a[:2] == ["volume", "rm"] and a[-1] == "other-volume" for a in calls), calls)

    def test_volume_removal_failure_is_not_success(self):
        rc, state, calls, log = self.run_case("volume-rm-noop")
        self.assertEqual(rc, 70, log)
        self.assertEqual(len(state["volumes"]), 1, log)
        self.assertIn(next(iter(state["volumes"])), log)
        self.assertTrue(any(a[:2] == ["volume", "rm"] for a in calls), calls)

    def test_volume_observation_failure_is_not_success(self):
        rc, state, calls, log = self.run_case("volume-list-fail-after")
        self.assertEqual(rc, 71, log)
        self.assertEqual(len(state["volumes"]), 1, log)
        self.assertTrue(any(a[0] == "run" for a in calls), calls)

    def test_unknown_container_does_not_remove_volume(self):
        rc, state, calls, log = self.run_case("container-observe-fail")
        self.assertEqual(rc, 71, log)
        self.assertEqual(len(state["volumes"]), 1, log)
        self.assertFalse(any(a[:2] == ["volume", "rm"] for a in calls), calls)

    def test_create_failure_stops_before_run(self):
        rc, _, calls, log = self.run_case("create-fail")
        self.assertNotEqual(rc, 0, log)
        self.assertFalse(any(a[0] == "run" for a in calls), calls)

    def followup(self, case, fault="", foreign=False):
        name = "cv2-a-f1-12345-99"
        volume = name + "-pgdata"
        owner = "f1-20261115T153403Z-12345"
        volume_owner = "f1-20261115T153403Z-99999" if foreign else owner
        initial = {"volumes": {volume: ["cortexv2=dev", "cv2.owner=" + volume_owner], "other-volume": ["cv2.owner=other"]},
                   "containers": {name: ["cortexv2=dev", "cv2.owner=" + owner]}, "runs": 1}
        (self.dir / "state.json").write_text(json.dumps(initial))
        controls = Path(os.environ.get("VOLUME_CONTROLS", ROOT / "controls/controls.sh")).read_text()
        match = re.search(r"(?ms)^remove_exact\(\) \{.*?^\}", controls)
        self.assertIsNotNone(match, "remove_exact function missing")
        q = shlex.quote
        script = "set -u\nREAL=%s\nOUT=%s\nFAILS=0\n. %s\n%s\nremove_exact %s %s\nprintf 'FAILS=%%s\\n' \"$FAILS\"\n" % (
            q(str(self.dir / "bin/podman")), q(str(self.dir)), q(str(ROOT / "controls/consumers.sh")), match.group(), q(name), q(case))
        p = subprocess.run(["bash", "-c", script], env=dict(self.env, VOLUME_FAKE_FAULT=fault), capture_output=True, text=True, timeout=15)
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        state = json.loads((self.dir / "state.json").read_text())
        return name, volume, state, p.stdout + p.stderr

    def test_followup_k3(self):
        _, volume, state, log = self.followup("K3")
        self.assertNotIn(volume, state["volumes"], log)
        self.assertEqual(state["volumes"], {"other-volume": ["cv2.owner=other"]}, log)
        self.assertIn("FAILS=0", log)

    def test_followup_k9(self):
        _, volume, state, log = self.followup("K9")
        self.assertNotIn(volume, state["volumes"], log)
        self.assertEqual(state["volumes"], {"other-volume": ["cv2.owner=other"]}, log)
        self.assertIn("FAILS=0", log)

    def test_followup_foreign_owner_refused(self):
        _, volume, state, log = self.followup("foreign", foreign=True)
        self.assertIn(volume, state["volumes"], log)
        self.assertIn("FAILS=1", log)

    def test_followup_volume_removal_failure_counted(self):
        _, volume, state, log = self.followup("stuck", fault="volume-rm-noop")
        self.assertIn(volume, state["volumes"], log)
        self.assertIn("FAILS=1", log)

    def test_followup_present_container_preserves_volume(self):
        name, volume, state, log = self.followup("container", fault="container-rm-noop")
        self.assertIn(name, state["containers"], log)
        self.assertIn(volume, state["volumes"], log)
        self.assertIn("FAILS=1", log)

    def test_general_followup_k4(self):
        """Run the actual K4 control in a private fake-only crate, including its loop follow-up."""
        controls = Path(os.environ.get("VOLUME_CONTROLS", ROOT / "controls/controls.sh")).read_text()
        real_anchor = "REAL=/usr/bin/podman"
        fake_anchor = 'FAKE_DIR="$PWD/controls/fake-podman"'
        self.assertEqual(controls.count(real_anchor), 1)
        self.assertEqual(controls.count(fake_anchor), 1)
        controls = controls.replace(real_anchor, 'REAL=' + shlex.quote(str(self.dir / "bin/podman")))
        controls = controls.replace(fake_anchor, 'FAKE_DIR=' + shlex.quote(str(self.dir / "bin")))
        d = self.dir / "crate/controls"
        d.mkdir(parents=True)
        for f in ("consumers.sh", "certify.sh", "sigsafe.py", "observer.sh"):
            shutil.copyfile(ROOT / "controls" / f, d / f)
        path = d / "controls.sh"
        path.write_text(controls)
        case_out = self.dir / "k4-out"
        p = subprocess.run(["bash", str(path), str(case_out), str(self.sup), "K4"], env=self.env, capture_output=True, text=True, timeout=30)
        state = json.loads((self.dir / "state.json").read_text())
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertIn("PASS  K4-observe-fails", p.stdout)
        self.assertEqual(state["volumes"], {}, p.stdout + p.stderr)
        self.assertEqual(state["containers"], {}, p.stdout + p.stderr)

    def general_followup(self, foreign=False, referenced=False):
        name = "cv2-a-f1-12345-99"; volume = name + "-pgdata"
        owner = "f1-20261115T153403Z-12345"
        volume_owner = "f1-20261115T153403Z-99999" if foreign else owner
        state = {"volumes": {volume: ["cortexv2=dev", "cv2.owner=" + volume_owner]}, "containers": {}, "runs": 1,
                 "references": [volume] if referenced else []}
        (self.dir / "state.json").write_text(json.dumps(state))
        (self.dir / "K4-fixture.name").write_text(name + "\n")
        (self.dir / "K4-fixture.stderr").write_text("start: container " + name + " (cv2.owner=" + owner + "), image fixture\n")
        controls = Path(os.environ.get("VOLUME_CONTROLS", ROOT / "controls/controls.sh")).read_text()
        match = re.search(r"(?ms)^general_volume_followup\(\) \{.*?^\}", controls)
        self.assertIsNotNone(match, "general_volume_followup function missing")
        q = shlex.quote
        script = "set -u\nREAL=%s\nOUT=%s\nFAILS=0\nFOLLOWUP_FAILED=0\n. %s\n%s\ngeneral_volume_followup K4\nprintf 'FAILS=%%s FOLLOWUP_FAILED=%%s\\n' \"$FAILS\" \"$FOLLOWUP_FAILED\"\n" % (
            q(str(self.dir / "bin/podman")), q(str(self.dir)), q(str(ROOT / "controls/consumers.sh")), match.group())
        p = subprocess.run(["bash", "-c", script], env=self.env, capture_output=True, text=True, timeout=15)
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        return volume, json.loads((self.dir / "state.json").read_text()), p.stdout + p.stderr

    def test_general_foreign_owner_refused(self):
        volume, state, log = self.general_followup(foreign=True)
        self.assertIn(volume, state["volumes"], log)
        self.assertIn("FAILS=1 FOLLOWUP_FAILED=1", log)

    def test_general_referenced_volume_refused(self):
        volume, state, log = self.general_followup(referenced=True)
        self.assertIn(volume, state["volumes"], log)
        self.assertIn("FAILS=1 FOLLOWUP_FAILED=1", log)


if __name__ == "__main__":
    unittest.main(verbosity=2)
