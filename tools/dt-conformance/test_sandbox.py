"""Real Docker test for the single-container isolation boundary."""

import json
import os
from pathlib import Path
import tempfile
import time
import unittest

from sandbox import copy_outputs, launch


IMAGE = os.environ.get("TSRS_DT_CONFORMANCE_SANDBOX_IMAGE")
WORKER = Path(__file__).with_name("run.py").resolve()


@unittest.skipUnless(IMAGE, "set TSRS_DT_CONFORMANCE_SANDBOX_IMAGE to run the Docker isolation smoke")
class SandboxTests(unittest.TestCase):
    def test_inputs_and_host_stay_private_and_detached_children_die(self):
        with tempfile.TemporaryDirectory(prefix="tsrs-dt-sandbox-") as temporary:
            root = Path(temporary).resolve()
            token = root / "host-token"
            token.write_text("secret")
            typescript = root / "typescript"
            candidate = root / "candidate"
            platform = root / "platform"
            typescript.mkdir()
            candidate.mkdir()
            platform.mkdir()
            (typescript / "package.json").write_text("{}")
            (candidate / "package.json").write_text("{}")
            binary = platform / "tsrs"
            binary.write_text("synthetic")
            original = {path: path.read_bytes() for path in (typescript / "package.json", candidate / "package.json", binary)}
            work = root / "work"
            request = root / "request.json"
            request.write_text(json.dumps({"probe": {"host_token": str(token)}}))
            ran = launch(IMAGE, WORKER, typescript, candidate, platform, work, request, root / "sandbox.log", 30)
            self.assertEqual(ran, {"exit": 0, "timeout": False}, (root / "sandbox.log").read_text())
            evidence = json.loads((work / "out" / "probe.json").read_text())
            self.assertTrue(all(evidence.values()), evidence)
            self.assertEqual(token.read_text(), "secret")
            for path, contents in original.items():
                self.assertEqual(path.read_bytes(), contents)
            heartbeat = work / "out" / "heartbeat"
            before = heartbeat.read_bytes()
            self.assertTrue(before)
            time.sleep(0.3)
            self.assertEqual(heartbeat.read_bytes(), before)
            copied = root / "copied"
            copy_outputs(work / "out", copied)
            self.assertFalse((copied / "host-link").exists())
            self.assertFalse((copied / "host-link").is_symlink())


if __name__ == "__main__":
    unittest.main()
