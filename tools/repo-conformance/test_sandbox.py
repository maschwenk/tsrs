"""Real container tests, required by CI before any public repository is installed."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import unittest

from run import child_env
from sandbox import check_in_container, copy_logs, launch


IMAGE = os.environ.get("TSRS_CONFORMANCE_SANDBOX_IMAGE")
WORKER = Path(__file__).with_name("run.py").resolve()


@unittest.skipUnless(IMAGE, "set TSRS_CONFORMANCE_SANDBOX_IMAGE to run the Docker isolation smoke")
class SandboxTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="tsrs-sandbox-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.compilers = self.root / "bin"
        self.compilers.mkdir()
        for name in ("tsgo", "tsrs"):
            path = self.compilers / name
            path.write_text("#!/usr/bin/python3\nimport os, sys\n"
                            "print(os.getcwd() + '/a.ts(1,1): error TS2322: synthetic diagnostic.')\n"
                            "print('Files: 1')\nsys.exit(2)\n")
            path.chmod(0o755)

    def fixture(self, manager=None, extra=None):
        source = Path(tempfile.mkdtemp(prefix="source-", dir=self.root))
        (source / "tsconfig.json").write_text('{"files":["a.ts"]}')
        (source / "a.ts").write_text("const a: number = 'a';\n")
        if manager:
            (source / "package.json").write_text(json.dumps(dict(name="sandbox-smoke", private=True,
                                                                 packageManager=manager)))
        for name, text in (extra or {}).items():
            path = source / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        env = child_env(self.root / "git-home", self.root / "git-cache")
        for args in (("init", "--quiet", "--initial-branch=main", "--template="), ("add", "."),
                     ("-c", "user.name=Sandbox", "-c", "user.email=sandbox@example.invalid", "commit", "--quiet", "-m", "fixture")):
            subprocess.run(["git", *args], cwd=source, env=env, check=True, capture_output=True)
        sha = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=source, env=env, text=True).strip()
        return source, dict(repository="synthetic/sandbox", branch="main", commit=sha)

    def run_worker(self, source, repo, timeout=90):
        work = self.root / "work"
        request = self.root / "request.json"
        request.write_text(json.dumps(dict(repo=repo, setup_timeout=60, check_timeout=10, max_configs=5, source=True)))
        result = launch(IMAGE, WORKER, self.compilers / "tsgo", self.compilers / "tsrs", work, request,
                        self.root / "sandbox.log", timeout, source)
        return result, work

    def save_evidence(self, name, data, output):
        if directory := os.environ.get("TSRS_SANDBOX_EVIDENCE"):
            destination = Path(directory) / name
            destination.mkdir(parents=True, exist_ok=True)
            (destination / "result.json").write_text(json.dumps(data, indent=2) + "\n")
            copy_logs(output, destination)
            if (self.root / "sandbox.log").is_file():
                shutil.copyfile(self.root / "sandbox.log", destination / "sandbox.log")

    def test_yarn_plugin_cannot_reach_host_or_change_inputs_and_its_detached_process_dies(self):
        """Yarn plugins still execute with lifecycle scripts off; they must stay inside one repository's container."""
        token = self.root / "parent-token-marker"
        cache = self.root / "host-release-cache"
        token.write_text("synthetic-parent-token")
        cache.write_text("trusted-cache-marker")
        compiler = (self.compilers / "tsrs").read_bytes()
        worker = WORKER.read_bytes()
        plugin = r"""
const fs = require('fs');
const cp = require('child_process');
exports.name = 'sandbox-probe';
exports.factory = () => {
  const blocked = (fn) => { try { fn(); return false; } catch { return true; } };
  const evidence = {
    hostToken: blocked(() => fs.readFileSync(__TOKEN_PATH__)),
    parentRoot: blocked(() => fs.readFileSync('/proc/1/root' + __TOKEN_PATH__)),
    hostCache: blocked(() => fs.writeFileSync(__CACHE_PATH__, 'poisoned')),
    compiler: blocked(() => fs.appendFileSync('/inputs/tsrs', 'poisoned')),
    harness: blocked(() => fs.appendFileSync('/harness/tools/repo-conformance/run.py', 'poisoned')),
    dockerSocket: !fs.existsSync('/var/run/docker.sock'),
    nonRoot: process.getuid() !== 0,
    noNewPrivileges: /NoNewPrivs:\s+1/.test(fs.readFileSync('/proc/self/status', 'utf8')),
    noCapabilities: /CapEff:\s+0+\n/.test(fs.readFileSync('/proc/self/status', 'utf8')),
    noRunnerToken: !process.env.ACTIONS_RUNTIME_TOKEN && !process.env.GITHUB_TOKEN,
    noSudo: cp.spawnSync('sudo', ['-n', 'true']).status !== 0
  };
  fs.writeFileSync('/work/out/hook.json', JSON.stringify(evidence));
  try { fs.symlinkSync(__TOKEN_PATH__, '/work/out/host-link'); } catch {}
  const child = cp.spawn(process.execPath, ['-e',
    "const fs=require('fs'); const f='/work/out/heartbeat'; fs.appendFileSync(f,'.'); " +
    "setInterval(()=>fs.appendFileSync(f,'.'),20)"], {detached:true, stdio:'ignore'});
  child.unref();
  return {};
};
""".replace("__TOKEN_PATH__", json.dumps(str(token))).replace("__CACHE_PATH__", json.dumps(str(cache)))
        source, repo = self.fixture("yarn@4.9.2", {
            ".yarnrc.yml": "enableTelemetry: false\nplugins:\n  - path: .yarn/plugins/probe.cjs\n    spec: sandbox-probe\n",
            ".yarn/plugins/probe.cjs": plugin,
            "yarn.lock": '# This file is generated by running "yarn install" inside your project.\n'
                         '# Manual changes might be lost - proceed with caution!\n\n'
                         '__metadata:\n  version: 8\n  cacheKey: 10c0\n\n"sandbox-smoke@workspace:.":\n'
                         '  version: 0.0.0-use.local\n  resolution: "sandbox-smoke@workspace:."\n'
                         '  languageName: unknown\n  linkType: soft\n',
        })
        ran, work = self.run_worker(source, repo)
        self.assertEqual(ran, {"exit": 0, "timeout": False}, (self.root / "sandbox.log").read_text())
        result = json.loads((work / "result.json").read_text())
        evidence = json.loads((work / "out/hook.json").read_text())
        self.save_evidence("yarn-plugin", dict(result=result, isolation=evidence), work / "out")
        self.assertEqual(result["status"], "match", result)
        self.assertEqual(result["strict_status"], "match", result)
        self.assertTrue(all(evidence.values()), evidence)
        self.assertEqual(token.read_text(), "synthetic-parent-token")
        self.assertEqual(cache.read_text(), "trusted-cache-marker")
        self.assertEqual((self.compilers / "tsrs").read_bytes(), compiler)
        self.assertEqual(WORKER.read_bytes(), worker)
        heartbeat = work / "out/heartbeat"
        before = heartbeat.read_bytes()
        self.assertTrue(before)
        time.sleep(0.3)
        self.assertEqual(heartbeat.read_bytes(), before, "detached hook survived container removal")
        copied = self.root / "exported"
        copy_logs(work / "out", copied)
        self.assertFalse((copied / "host-link").is_symlink())
        self.assertFalse((copied / "host-link").exists())

    def test_timeout_removes_detached_compiler_children(self):
        compiler = self.compilers / "tsgo"
        child = ("import time\nfrom pathlib import Path\np=Path('/work/out/heartbeat')\n"
                 "while True:\n p.open('a').write('.')\n time.sleep(0.02)\n")
        compiler.write_text("#!/usr/bin/python3\nimport subprocess, sys, time\n"
                            f"subprocess.Popen([sys.executable, '-c', {child!r}], start_new_session=True)\n"
                            "time.sleep(60)\n")
        source, repo = self.fixture()
        ran, work = self.run_worker(source, repo, timeout=3)
        self.save_evidence("timeout", ran, work / "out")
        self.assertTrue(ran["timeout"])
        heartbeat = work / "out/heartbeat"
        before = heartbeat.read_bytes()
        self.assertTrue(before)
        time.sleep(0.3)
        self.assertEqual(heartbeat.read_bytes(), before, "detached compiler survived timeout cleanup")

    def test_supported_managers_still_install_inside_the_container(self):
        """Empty synthetic packages exercise the real install commands without installing a public dataset."""
        for manager in ("npm@11.6.2", "pnpm@10.23.0", "pnpm@12.9.1", "pnpm@12.10.1", "yarn@1.22.22",
                        "bun@1.4.3", "nub@0.8.3"):
            with self.subTest(manager=manager):
                extra = {".pnpmfile.cjs": "throw new Error('pnpmfile must not execute');\n"}
                source, repo = self.fixture(manager, extra)
                out = self.root / manager.replace("@", "-")
                result = check_in_container(repo, self.compilers / "tsgo", self.compilers / "tsrs", out,
                                            self.root / "scratch", 60, 10, 5, IMAGE, source)
                self.save_evidence(manager, result, out)
                self.assertEqual(result["status"], "match", result)
                self.assertEqual(result["strict_status"], "match", result)
                self.assertFalse(list((self.root / "scratch").iterdir()))
                shutil.rmtree(source)

    def test_default_difference_and_strict_match_keep_their_logs_and_statuses(self):
        compiler = self.compilers / "tsrs"
        compiler.write_text("#!/usr/bin/python3\nimport os, sys\n"
                            "message = 'synthetic diagnostic.' if '--checkerAssignment' in sys.argv else 'changed.'\n"
                            "print(os.getcwd() + '/a.ts(1,1): error TS2322: ' + message)\n"
                            "print('Files: 1')\nsys.exit(2)\n")
        source, repo = self.fixture()
        out = self.root / "modes"
        result = check_in_container(repo, self.compilers / "tsgo", compiler, out, self.root / "scratch",
                                    60, 10, 5, IMAGE, source)
        self.save_evidence("modes", result, out)
        self.assertEqual(result["status"], "mismatch", result)
        self.assertEqual(result["strict_status"], "match", result)
        self.assertEqual(result["configs"][0]["difference"], ["diagnostics"])
        self.assertIn("changed.", (out / "0/diagnostics.diff").read_text())
        self.assertIn("synthetic diagnostic.", (out / "0/tsrs-strict.log").read_text())


if __name__ == "__main__":
    unittest.main()
