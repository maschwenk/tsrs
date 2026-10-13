import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

from run import (HARNESS, MODES, Context, check_config, check_projects, child_env, compare, difference, execute,
                 install_plan, read_compilation, summarize, write_json)


def context(tsgo, tsrs, temp):
    return Context(tsgo, tsrs, child_env(Path(temp) / "home", Path(temp) / "cache"))


def compilation(text="", files=1, code=0, timeout=False):
    return {"diagnostics": text, "files": files, "exit": code, "timeout": timeout}


class ComparisonTests(unittest.TestCase):
    def test_dropped_changed_or_reordered_diagnostics_fail_parity(self):
        first = "a.ts(1,1): error TS2322: wrong type.\n  'foo' is missing."
        second = "b.ts(1,1): error TS2345: wrong argument."
        baseline = compilation(first + "\n" + second, code=2)
        for text in (second, first.replace("foo", "bar") + "\n" + second, second + "\n" + first):
            with self.subTest(text=text):
                self.assertEqual(compare(baseline, compilation(text, code=2)), "mismatch")

    def test_exit_codes_and_loaded_file_counts_must_match(self):
        self.assertEqual(compare(compilation(), compilation(code=2)), "mismatch")
        self.assertEqual(compare(compilation(), compilation(files=2)), "mismatch")

    def test_crashes_timeouts_and_empty_projects_are_not_matches(self):
        for code, timeout in ((-9, False), (101, False), (0, True)):
            with self.subTest(code=code, timeout=timeout):
                broken = compilation(code=code, timeout=timeout)
                self.assertEqual(compare(compilation(), broken), "tsrs_failed")
                self.assertEqual(compare(broken, broken), "reference_failed")
        self.assertEqual(compare(compilation(files=0), compilation(files=0)), "no_inputs")

    def test_differences_say_whether_diagnostics_or_only_other_output_changed(self):
        # zod's packages/bench: the same errors, but traceResolution lines in another order.
        error = "a.ts(1,1): error TS2345: wrong argument.\n  Detail."
        go = compilation("======== Resolving 'x' ========\nFound 'package.json'.\n" + error, code=2)
        traces = compilation("======== Resolving 'x' ========\nExists according to earlier cached lookups.\n" + error, code=2)
        self.assertEqual(compare(go, traces), "mismatch")
        self.assertEqual(difference(go, traces), ["other output"])
        detail = compilation(go["diagnostics"].replace("Detail.", "Other detail."), code=2)
        self.assertEqual(difference(go, detail), ["diagnostics"])
        self.assertEqual(difference(go, compilation(go["diagnostics"], files=2, code=1)), ["exit code", "loaded files"])

    def test_parsing_keeps_multiline_diagnostics_and_excludes_counters(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            log = root / "compiler.log"
            log.write_text("$ compiler\r\n" + str(root) + "/a.ts(1,1): error TS2322: wrong.\r\n"
                           "  Detail.\r\nFiles: 3\r\nCheck time: 0.01s\r\nFS: stat: 5\r\n")
            result = read_compilation({"exit": 2, "timeout": False}, log, root, root / "bin" / "tsrs")
            self.assertEqual(result["diagnostics"], "<repo>/a.ts(1,1): error TS2322: wrong.\n  Detail.")
            self.assertEqual(result["files"], 3)

    def test_timeout_kills_child_processes(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            marker = root / "escaped-child"
            child = f"import time; from pathlib import Path; time.sleep(0.5); Path({str(marker)!r}).touch()"
            parent = f"import subprocess,sys,time; subprocess.Popen([sys.executable,'-c',{child!r}]); time.sleep(30)"
            result = execute([sys.executable, "-c", parent], root, root / "timeout.log", 0.2,
                             child_env(root / "home", root / "cache"))
            self.assertTrue(result["timeout"])
            time.sleep(0.6)
            self.assertFalse(marker.exists())


class IsolationTests(unittest.TestCase):
    def test_children_get_a_fresh_home_without_host_credentials_or_config(self):
        """A package-manager hook or config lookup must not reach the runner's npm/git config or CI tokens."""
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp).resolve()
            host = root / "host-home"
            host.mkdir()
            (host / ".npmrc").write_text("init-author-name=sentinel-host-npmrc\n")
            (host / ".gitconfig").write_text("[user]\n\tname = sentinel-host-gitconfig\n")
            ambient = {"HOME": str(host), "XDG_CONFIG_HOME": str(host), "XDG_CACHE_HOME": str(root / "ambient-cache"),
                       "NPM_TOKEN": "sentinel-npm-token", "GH_TOKEN": "sentinel-gh-token",
                       "npm_config_userconfig": str(host / ".npmrc")}
            saved = {key: os.environ.get(key) for key in ambient}
            os.environ.update(ambient)
            try:
                env = child_env(root / "home", root / "cache")
            finally:
                for key, value in saved.items():
                    if value is None:
                        os.environ.pop(key, None)
                    else:
                        os.environ[key] = value
            def run(*command):
                return subprocess.run(command, cwd=root, env=env, capture_output=True, text=True).stdout.strip()
            leaked = run(sys.executable, "-c", "import os; print([k for k, v in os.environ.items() if 'sentinel' in v])")
            self.assertEqual(leaked, "[]")
            self.assertEqual(run("npm", "config", "get", "init-author-name"), "")
            self.assertEqual(run("npm", "config", "get", "userconfig"), str(root / "home" / ".npmrc"))
            self.assertEqual(run("npm", "config", "get", "cache"), str(root / "cache" / "npm"))
            self.assertEqual(run("git", "config", "user.name"), "")
            self.assertFalse((root / "ambient-cache").exists())


class InstallTests(unittest.TestCase):
    def plan(self, files):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for name, text in files.items():
                (root / name).write_text(text if isinstance(text, str) else json.dumps(text))
            return install_plan(root)

    def test_frozen_installs_use_the_manager_that_wrote_the_lockfile(self):
        pnpm9 = "lockfileVersion: '9.0'\n"
        cases = [
            ({"package.json": {}, "pnpm-lock.yaml": pnpm9}, "corepack pnpm@12.10.1 install --frozen-lockfile"),
            ({"package.json": {"pnpm": {"overrides": {}}}, "pnpm-lock.yaml": pnpm9}, "corepack pnpm@10.33.0 install"),
            ({"package.json": {"workspaces": ["packages/*"]}, "pnpm-lock.yaml": pnpm9}, "corepack pnpm@10.33.0 install"),
            ({"package.json": {}, "pnpm-lock.yaml": "lockfileVersion: '6.0'\n"}, "corepack pnpm@8.15.9 install"),
            ({"package.json": {"packageManager": "pnpm@10.23.0"}, "bun.lock": "{}"}, "bun install --frozen-lockfile"),
            ({"package.json": {"packageManager": "nub@0.8.3"}, "pnpm-lock.yaml": pnpm9}, "nub install --frozen-lockfile"),
            ({"package.json": {"devEngines": {"packageManager": {"name": "pnpm", "version": "11.11.0"}}},
              "pnpm-lock.yaml": pnpm9}, "corepack pnpm@11.11.0 install --frozen-lockfile"),
            ({"package.json": {"packageManager": "pnpm@12.4.1"}}, "corepack pnpm@12.4.1 install --no-frozen-lockfile"),
        ]
        for files, command in cases:
            with self.subTest(files=files):
                plan = self.plan(files)
                self.assertTrue(" ".join(plan["command"]).startswith(command), plan)
                self.assertIn("--ignore-scripts", plan["command"])
        self.assertEqual(self.plan(cases[4][0])["tool"], ("bun", "1.4.3"))
        self.assertEqual(self.plan({"package.json": {}})["dependencies"], "unpinned")
        with self.assertRaisesRegex(ValueError, "Unsupported package manager: utoo@1.0.0"):
            self.plan({"package.json": {"packageManager": "utoo@1.0.0"}})

    def test_manager_versions_cannot_select_another_package_or_executable(self):
        for name in ("npm", "pnpm", "yarn", "bun", "nub"):
            for version in ("npm:evil@1.0.0", "https://example.org/tool.tgz", "file:/tmp/tool", "latest",
                            "^1.2.3", "1.2.3 --prefix=/tmp", "1.2.3+sha224.evil", "01.2.3", "1.2.3-01"):
                with self.subTest(name=name, version=version), self.assertRaisesRegex(ValueError, "Invalid"):
                    self.plan({"package.json": {"packageManager": f"{name}@{version}"}})
        for name in ("npm", "pnpm", "yarn"):
            spec = f"{name}@1.2.3+sha224." + "a" * 56
            plan = self.plan({"package.json": {"packageManager": spec}})
            self.assertEqual(plan["command"][:2], ["corepack", spec])
        for name in ("bun", "nub"):
            self.assertEqual(self.plan({"package.json": {"packageManager": f"{name}@1.2.3-beta.1"}})["tool"],
                             (name, "1.2.3-beta.1"))

    def test_pnpm_hooks_are_disabled_even_when_install_scripts_are_off(self):
        plan = self.plan({"package.json": {"packageManager": "pnpm@10.23.0"}})
        self.assertIn("--ignore-pnpmfile", plan["command"])


class RerunTests(unittest.TestCase):
    def run_cli(self, *args):
        return subprocess.run([sys.executable, str(Path(__file__).with_name("run.py")), *args], capture_output=True,
                              text=True)

    def test_rerun_inputs_must_come_as_a_valid_pair(self):
        for run_id, shards, ok in (("", "", True), ("38278199589", "1, 4", True), ("", "1", False),
                                   ("38278199589", "", False), ("latest", "1", False), ("38278199589", "20", False),
                                   ("38278199589", "1;2", False)):
            with self.subTest(run_id=run_id, shards=shards):
                result = self.run_cli("dispatch", "--run-id", run_id, "--shards", shards)
                self.assertEqual(result.returncode == 0, ok, result.stderr)

    def test_a_rerun_needs_a_pinned_manifest_from_the_named_run(self):
        sha = "0123456789abcdef0123456789abcdef01234567"
        manifest = {"harness": HARNESS, "tsrs_commit": sha, "typescript_commit": sha, "shards": 3,
                    "repositories": [{"repository": "a/one", "commit": sha}]}
        run = {"id": 7, "path": ".github/workflows/repo-conformance.yml", "event": "workflow_dispatch", "head_sha": sha,
               "repository": {"full_name": "o/tsrs"}, "head_repository": {"full_name": "o/tsrs"}}
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for name in ("tsgo", "tsrs"):
                write_json(root / "inputs" / "bin" / name, {})
            # A fork pull request's run would run the fork's compilers in this repository's cache scope.
            fork = {"event": "pull_request", "head_repository": {"full_name": "fork/tsrs"}}
            cases = [({}, {}, "2,0", True), ({"harness": None}, {}, "1", False),
                     ({"harness": HARNESS - 1}, {}, "1", False),
                     ({}, {"event": "pull_request", "head_sha": "f" * 40}, "0,2", True), ({}, fork, "1", False),
                     ({}, {"head_repository": None}, "1", False),
                     ({"repositories": [{"repository": "a/one"}]}, {}, "1", False),
                     ({}, {"head_sha": "f" * 40}, "1", False), ({}, {"path": ".github/workflows/ci.yml"}, "1", False),
                     ({}, {}, "3", False)]
            for manifest_change, run_change, shards, ok in cases:
                with self.subTest(manifest=manifest_change, run=run_change, shards=shards):
                    write_json(root / "inputs" / "manifest.json", dict(manifest, **manifest_change))
                    write_json(root / "run.json", dict(run, **run_change))
                    result = self.run_cli("rerun", "--manifest", str(root / "inputs" / "manifest.json"),
                                          "--shards", shards, "--run", str(root / "run.json"))
                    self.assertEqual(result.returncode == 0, ok, result.stderr)
                    if ok:
                        self.assertEqual(json.loads(result.stdout), {"shard": [0, 2]})
                        recovered = json.loads((root / "inputs" / "manifest.json").read_text())
                        self.assertEqual(recovered["recovery_of_run_id"], 7)
                        chained = self.run_cli("rerun", "--manifest", str(root / "inputs" / "manifest.json"),
                                               "--shards", "0,2", "--run", str(root / "run.json"))
                        self.assertNotEqual(chained.returncode, 0)
                        self.assertIn("use the original run 7", chained.stderr)
            (root / "inputs" / "bin" / "tsrs").unlink()
            write_json(root / "inputs" / "manifest.json", manifest)
            write_json(root / "run.json", run)
            result = self.run_cli("rerun", "--manifest", str(root / "inputs" / "manifest.json"), "--shards", "1",
                                  "--run", str(root / "run.json"))
            self.assertIn("no tsrs binary", result.stderr)


class ReportTests(unittest.TestCase):
    def test_missing_duplicate_or_wrong_revision_results_fail_the_report(self):
        manifest = {"tsrs_commit": "rust-pin", "typescript_commit": "go-pin", "shards": 2,
                    "repositories": [{"repository": "a/one"}, {"repository": "a/two"}]}
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            results = root / "results"
            out = root / "report.md"
            def shard(index, name):
                return {"tsrs_commit": "rust-pin", "typescript_commit": "go-pin", "shard": index,
                        "results": [{"repository": name, "status": "match", "available_configs": 1, "configs": [
                            {"config": "tsconfig.json", "status": "match", "tsgo": {"exit": 2}}]}]}
            write_json(results / "shard-0.json", shard(0, "a/one"))
            self.assertFalse(summarize(manifest, results, out))
            self.assertIn("Missing repository result: a/two", out.read_text())
            write_json(results / "shard-1.json", shard(1, "a/two"))
            self.assertTrue(summarize(manifest, results, out))
            self.assertIn("These shard reports predate strict mode", out.read_text())
            write_json(results / "duplicate" / "shard-0.json", shard(0, "a/one"))
            self.assertFalse(summarize(manifest, results, out))
            (results / "duplicate" / "shard-0.json").unlink()
            # A shard from other compilers is left out entirely: its repository is missing, not counted.
            for key in ("tsrs_commit", "typescript_commit"):
                with self.subTest(key=key):
                    write_json(results / "shard-1.json", dict(shard(1, "a/two"), **{key: "other-compiler"}))
                    self.assertFalse(summarize(manifest, results, out))
                    text = out.read_text()
                    self.assertIn(f"{key} do not match the manifest; its results are left out", text)
                    self.assertIn("1 of 2 repositories have a result; 1 had at least one project", text)
                    self.assertIn("Both compilers completed on 1 projects: 1 identical", text)
                    self.assertIn("Missing repository result: a/two", text)
                    self.assertIn("| [a/two](https://github.com/a/two) | `` | missing |  | 0 / 0 | 0 |", text)

    def test_a_rerun_shard_fills_a_lost_one_only_with_matching_provenance(self):
        manifest = {"tsrs_commit": "rust-pin", "typescript_commit": "go-pin", "shards": 2,
                    "repositories": [{"repository": "a/one", "commit": "c1"}, {"repository": "a/two", "commit": "c2"},
                                     {"repository": "a/three", "commit": "c3"}]}
        def shard(index, *results, **settings):
            return {"tsrs_commit": "rust-pin", "typescript_commit": "go-pin", "shard": index, "check_timeout": 120,
                    "modes": MODES, **settings, "results": [
                        {"repository": name, "commit": commit, "status": status, "available_configs": 1,
                         "configs": [{"config": "tsconfig.json", "status": status, "tsgo": {"exit": 0}}]}
                        for name, commit, status in results]}
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            def report(previous=True):
                ok = summarize(manifest, root / "rerun", root / "report.md", root / "previous" if previous else None)
                return ok, (root / "report.md").read_text()
            # Shard 1 of the earlier run was lost: it left no report.
            write_json(root / "previous" / "shard-0.json", shard(0, ("a/one", "c1", "match"), ("a/three", "c3", "match")))
            write_json(root / "rerun" / "shard-1.json", shard(1, ("a/two", "c2", "match")))
            ok, text = report()
            self.assertTrue(ok, text)
            self.assertIn("3 of 3 repositories have a result; 3 had at least one project", text)
            # A result needs the pinned commit; only a clone that failed before any checkout has none.
            for commit, status, configs, accepted in (("newer", "match", True, False), (None, "match", True, False),
                                                      (None, "setup_failed", False, True)):
                with self.subTest(commit=commit, status=status):
                    rerun = shard(1, ("a/two", commit, status))
                    if not configs:
                        rerun["results"][0]["configs"] = []
                    write_json(root / "rerun" / "shard-1.json", rerun)
                    ok, text = report()
                    self.assertEqual(ok, accepted, text)
                    self.assertEqual("manifest pins c2" in text, not accepted)
            # A report for the wrong shard can't stand in for the lost one.
            write_json(root / "rerun" / "shard-1.json", shard(1, ("a/one", "c1", "match"), ("a/three", "c3", "match")))
            ok, text = report(previous=False)
            self.assertFalse(ok)
            self.assertIn("results for repositories outside shard 1: a/one, a/three", text)
            self.assertIn("Missing repository result: a/two", text)
            # Shards checked with other settings or modes are not merged, including reports from before strict mode.
            write_json(root / "rerun" / "shard-1.json", shard(1, ("a/two", "c2", "match")))
            for settings in ({"check_timeout": 60}, {"modes": {"default": MODES["default"]}}):
                with self.subTest(settings=settings):
                    write_json(root / "previous" / "shard-0.json",
                               shard(0, ("a/one", "c1", "match"), ("a/three", "c3", "match"), **settings))
                    ok, text = report()
                    self.assertFalse(ok)
                    self.assertIn("Shard 0 used other settings", text)
                    self.assertIn("Missing repository result: a/one", text)
            # Round 2's Go mode (no --noLazyMembers, forced 4-checker default reference) is not a known mode.
            write_json(root / "previous" / "shard-0.json", shard(0, ("a/one", "c1", "match"), ("a/three", "c3", "match"),
                                                                 modes=["default", "go"]))
            ok, text = report()
            self.assertFalse(ok)
            self.assertIn('unknown comparison modes ["default", "go"]', text)

    def test_a_rerun_cannot_clear_an_earlier_difference_or_replace_a_complete_shard(self):
        """Rerunning a shard until it matches must not hide a difference or tsrs failure the earlier run recorded."""
        manifest = {"tsrs_commit": "rust-pin", "typescript_commit": "go-pin", "shards": 2,
                    "repositories": [{"repository": f"a/{n}", "commit": "c" + n} for n in ("1", "2", "3", "4")]}
        def result(name, default="match", strict="match"):
            return {"repository": name, "commit": "c" + name[-1], "status": default, "strict_status": strict,
                    "available_configs": 1, "configs": [{"config": "tsconfig.json", "status": default,
                                                         "tsgo": {"exit": 0}, "strict": {"status": strict}}]}
        def shard(index, *results, **fields):
            return {"tsrs_commit": "rust-pin", "typescript_commit": "go-pin", "shard": index, "modes": MODES,
                    **fields, "results": list(results)}
        cases = [
            # A partial earlier shard (a/4 was not reached) that recorded a difference in either mode.
            ("default mismatch", [shard(1, result("a/2", "mismatch", "match"))],
             "a/2: the earlier run's shard 1", "tsconfig.json: default mismatch; a rerun does not clear it"),
            ("strict tsrs failure", [shard(1, result("a/2", "match", "tsrs_failed"))],
             "a/2: the earlier run's shard 1", "tsconfig.json: strict tsrs_failed; a rerun does not clear it"),
            # A report from before strict mode, without modes or strict results.
            ("legacy tsrs failure", [{"tsrs_commit": "rust-pin", "typescript_commit": "go-pin", "shard": 1, "results": [
                {"repository": "a/2", "commit": "c2", "status": "tsrs_failed", "configs": [
                    {"config": "tsconfig.json", "status": "tsrs_failed", "tsgo": {"exit": 0}}]}]}],
             "a/2: the earlier run's shard 1", "tsconfig.json: default tsrs_failed"),
            # A complete earlier shard is kept with its difference, and the rerun is refused.
            ("complete shard", [shard(1, result("a/2", "mismatch"), result("a/4"))],
             "Shard 1 was complete in the earlier run", "| [a/2](https://github.com/a/2) | `c2` | mismatch | match |"),
            ("complete clean shard", [shard(1, result("a/2"), result("a/4"))],
             "Shard 1 was complete in the earlier run", "a rerun replaces only lost or partial shards"),
            # Two artifacts claim the earlier shard 1; neither is silently picked.
            ("duplicate earlier shard", [shard(1, result("a/2")), shard(1, result("a/2", "mismatch"))],
             "duplicate or unexpected shard 1", ""),
            # The only acceptable recovery: a partial earlier shard without differences.
            ("clean partial shard", [shard(1, result("a/2"))], None, None),
        ]
        for label, earlier, error, evidence in cases:
            with self.subTest(label), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                write_json(root / "previous" / "repo-conformance-shard-0" / "shard-0.json",
                           shard(0, result("a/1"), result("a/3")))
                for n, report in enumerate(earlier):
                    write_json(root / "previous" / f"copy-{n}" / "shard-1.json", report)
                write_json(root / "rerun" / "repo-conformance-shard-1" / "shard-1.json",
                           shard(1, result("a/2"), result("a/4")))
                ok = summarize(manifest, root / "rerun", root / "report.md", root / "previous")
                text = (root / "report.md").read_text()
                self.assertEqual(ok, error is None, text)
                if error:
                    self.assertIn(error, text)
                    self.assertIn(evidence, text)
                self.assertIn("4 of 4 repositories have a result", text)

    def test_empty_projects_fail_a_run_with_no_comparisons(self):
        manifest = {"tsrs_commit": "rust-pin", "typescript_commit": "go-pin", "shards": 1,
                    "repositories": [{"repository": "a/one"}]}
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            report = {"tsrs_commit": "rust-pin", "typescript_commit": "go-pin", "shard": 0,
                      "results": [{"repository": "a/one", "status": "not_checked", "configs": [
                          {"config": "tsconfig.json", "status": "no_inputs"}]}]}
            write_json(root / "shard-0.json", report)
            self.assertFalse(summarize(manifest, root, root / "report.md"))
            self.assertIn("No project completed", (root / "report.md").read_text())


@unittest.skipUnless(os.environ.get("TSRS_CONFORMANCE_TSRS") and os.environ.get("TSRS_CONFORMANCE_TSGO"),
                     "Set TSRS_CONFORMANCE_TSRS and TSRS_CONFORMANCE_TSGO to run real compiler controls")
class CompilerControls(unittest.TestCase):
    def test_repeated_composite_checks_do_not_reuse_diagnostics(self):
        tsrs = Path(os.environ["TSRS_CONFORMANCE_TSRS"]).resolve()
        tsgo = Path(os.environ["TSRS_CONFORMANCE_TSGO"]).resolve()
        with tempfile.TemporaryDirectory(dir=Path.cwd()) as temp:
            root = Path(temp)
            write_json(root / "tsconfig.json", {"compilerOptions": {"strict": True, "composite": True}, "files": ["a.ts"]})
            out = (root / "logs").relative_to(Path.cwd())
            for source, code in (("const x: number = 1;\n", 0), ("const x: number = 'no';\n", 2),
                                 ("const x: number = 1;\n", 0)):
                (root / "a.ts").write_text(source)
                result = check_config(root, "tsconfig.json", context(tsgo, tsrs, root), out, 30)
                self.assertEqual(result["status"], "match", result)
                self.assertEqual(result["tsgo"]["exit"], code)
                self.assertGreater(result["tsgo"]["files"], 0)
                if code:
                    self.assertIn("error TS2322", (root / "logs" / "tsrs.log").read_text())
                self.assertFalse((root / "a.js").exists())
                self.assertFalse(any((root / "logs").glob("*.tsbuildinfo")))

    def test_solution_configs_compare_their_references_and_count_only_loaded_projects(self):
        tsrs = Path(os.environ["TSRS_CONFORMANCE_TSRS"]).resolve()
        tsgo = Path(os.environ["TSRS_CONFORMANCE_TSGO"]).resolve()
        with tempfile.TemporaryDirectory() as temp, tempfile.TemporaryDirectory() as host:
            root = Path(temp)
            # An absolute reference to a config outside the checkout is not followed.
            write_json(Path(host) / "tsconfig.json", {"files": ["a.ts"]})
            (Path(host) / "a.ts").write_text("export {};\n")
            references = [{"path": "./tsconfig.app.json"}, {"path": "lib"}, {"path": str(Path(host).resolve())}]
            write_json(root / "tsconfig.json", {"files": [], "references": references})
            write_json(root / "tsconfig.app.json", {"compilerOptions": {"strict": True}, "files": ["app.ts"]})
            write_json(root / "lib" / "tsconfig.json", {"compilerOptions": {"composite": True}, "files": ["lib.ts"]})
            write_json(root / "empty" / "tsconfig.json", {"files": []})
            write_json(root / "solution" / "tsconfig.json", {"files": [], "references": [{"path": "../lib"}]})
            (root / "app.ts").write_text("const x: number = 'no';\n")
            (root / "lib" / "lib.ts").write_text("export const y = 1;\n")
            configs = ["tsconfig.json", "empty/tsconfig.json", "lib/tsconfig.json", "solution/tsconfig.json"]
            ctx = context(tsgo, tsrs, root)
            result = check_projects(root, configs, ctx, root / "logs", 60, 2)
            statuses = [(c["config"], c["status"]) for c in result["configs"]]
            self.assertEqual(statuses, [("tsconfig.json", "references"), ("tsconfig.app.json", "match"),
                                        ("lib/tsconfig.json", "match")])
            self.assertEqual(result["status"], "partial")
            self.assertEqual(result["configs"][1]["tsgo"]["exit"], 2)
            result = check_projects(root, configs, ctx, root / "logs", 60, 5)
            self.assertEqual([c["status"] for c in result["configs"][3:]], ["no_inputs", "no_inputs"])
            self.assertEqual(result["status"], "partial")

    def test_strict_mode_is_reported_separately_and_does_not_clear_a_default_difference(self):
        """Reduced from the property-order configs (langfuse, qwen-code, OmniRoute, ...): tsrs's default mode prints
        `user?: undefined` where the declaration puts it, tsgo where a.ts's checker first created it."""
        tsrs = Path(os.environ["TSRS_CONFORMANCE_TSRS"]).resolve()
        tsgo = Path(os.environ["TSRS_CONFORMANCE_TSGO"]).resolve()
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp).resolve()
            write_json(root / "tsconfig.json", {"compilerOptions": {"strict": True}, "files": ["a.ts", "b.ts"]})
            (root / "a.ts").write_text('export const a = Math.random() > 0.5 ? { user: 1 } : { status: "a" };\n')
            (root / "b.ts").write_text('export const b = Math.random() > 0.5 ? { status: "no" } : { status: "ok", user: 2 };\n'
                                       "b.missing;\n")
            result = check_projects(root, ["tsconfig.json"], context(tsgo, tsrs, root), root / "logs", 60, 5)
            [config] = result["configs"]
            self.assertEqual((config["status"], config["difference"]), ("mismatch", ["diagnostics"]))
            self.assertEqual(config["strict"]["status"], "match")
            self.assertEqual((result["status"], result["strict_status"]), ("mismatch", "match"))
            logs = root / "logs" / "0"
            self.assertIn("{ status: string; user?: undefined; }", (logs / "tsrs.log").read_text())
            self.assertIn("{ user?: undefined; status: string; }", (logs / "tsrs-strict.log").read_text())
            self.assertTrue((logs / "diagnostics.diff").exists())
            self.assertFalse((logs / "diagnostics-strict.diff").exists())
            manifest = {"tsrs_commit": "rust-pin", "typescript_commit": "go-pin", "shards": 1,
                        "repositories": [{"repository": "a/one"}]}
            write_json(root / "results" / "shard-0.json", {
                "tsrs_commit": "rust-pin", "typescript_commit": "go-pin", "shard": 0, "modes": MODES,
                "results": [dict(result, repository="a/one", available_configs=1)]})
            self.assertFalse(summarize(manifest, root / "results", root / "report.md"))
            report = (root / "report.md").read_text()
            self.assertIn("0 identical (0 without errors, 0 with the same errors) and 1 different", report)
            self.assertIn("tsrs `--checkerAssignment go --checkers 4 --noLazyMembers`", report)
            self.assertIn("1 identical, 0 different, 0 tsrs failures in strict mode", report)
            self.assertIn("1 projects that differ or fail in default mode are identical in strict mode", report)
            self.assertIn("tsconfig.json: mismatch (diagnostics); strict match", report)
            self.assertIn("| mismatch | match | 0 / 1 | 1 |", report)

    def test_default_mode_keeps_the_projects_checkers_and_strict_mode_has_its_own_reference(self):
        """make(...) costs just over the 5M instantiation limit. tsgo with the project's 8 checkers checks b.ts's
        `id(big)` on a checker that has not seen a.ts and reports TS2589 there too; with 4 checkers it does not. A
        default reference forced to 4 checkers would hide b.ts's error from the default comparison."""
        tsrs = Path(os.environ["TSRS_CONFORMANCE_TSRS"]).resolve()
        tsgo = Path(os.environ["TSRS_CONFORMANCE_TSGO"]).resolve()
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp).resolve()
            write_json(root / "tsconfig.json", {"compilerOptions": {"strict": True, "target": "es2022", "types": [],
                                                                    "checkers": 8}, "files": ["a.ts", "b.ts"]})
            (root / "a.ts").write_text(
                'type D = "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9";\n'
                "type U = `${D}${D}${D}${D}`;\n"
                'type Chain<T, N extends unknown[] = []> = N["length"] extends 100 ? { value: T } : '
                "T extends string ? Chain<T, [...N, 0]> : never;\n"
                "declare function make<T>(f: (x: T) => void): Chain<T>;\n"
                "const big = make((x: U) => {});\n")
            (root / "b.ts").write_text("declare function id<T>(x: T): T;\nconst y = id(big);\n")
            result = check_projects(root, ["tsconfig.json"], context(tsgo, tsrs, root), root / "logs", 60, 5)
            [config] = result["configs"]
            logs = root / "logs" / "0"
            depth = "error TS2589: Type instantiation is excessively deep and possibly infinite."
            default, strict = (logs / "tsgo.log").read_text(), (logs / "tsgo-strict.log").read_text()
            self.assertNotIn("--checkers", default.partition("\n")[0])
            self.assertEqual([line for line in default.splitlines() if depth in line],
                             ["a.ts(5,13): " + depth, "b.ts(2,11): " + depth])
            self.assertEqual([line for line in strict.splitlines() if depth in line], ["a.ts(5,13): " + depth])
            self.assertIn("--noLazyMembers", (logs / "tsrs-strict.log").read_text().partition("\n")[0])
            self.assertEqual(config["strict"]["status"], "match")
            # Default-mode tsrs (locality assignment) currently reports only a.ts's error here; checked against the
            # project's 8-checker reference, that is a difference, where a 4-checker reference would have matched.
            both = "b.ts(2,11): " + depth in (logs / "tsrs.log").read_text()
            self.assertEqual(config["status"], "match" if both else "mismatch")

    def test_strict_mode_stays_within_the_check_budget(self):
        """A tsrs that hangs only in strict mode: the default comparisons keep their time, the whole check stays within
        its budget, and a strict timeout is a failure only when tsrs had at least the default run's limit."""
        tsrs = Path(os.environ["TSRS_CONFORMANCE_TSRS"]).resolve()
        tsgo = Path(os.environ["TSRS_CONFORMANCE_TSGO"]).resolve()
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp).resolve()
            hang = root / "tsrs-hangs-in-strict-mode"
            hang.write_text(f'#!/bin/sh\ncase "$*" in *noLazyMembers*) sleep 60;; esac\nexec "{tsrs}" "$@"\n')
            hang.chmod(0o755)
            configs = []
            for name in ("one", "two", "three"):
                write_json(root / name / "tsconfig.json", {"files": ["a.ts"]})
                (root / name / "a.ts").write_text("export const x: number = 1;\n")
                configs.append(name + "/tsconfig.json")
            for budget, checked, expected in ((6, configs, "budget_exhausted"), (3, configs[:1], "tsrs_failed")):
                with self.subTest(budget=budget):
                    start = time.monotonic()
                    result = check_projects(root, checked, context(tsgo, hang, root), root / f"logs{budget}", budget, 5)
                    self.assertLess(time.monotonic() - start, budget + 1)
                    self.assertEqual([c["status"] for c in result["configs"]], ["match"] * len(checked))
                    self.assertEqual([c["strict"]["status"] for c in result["configs"]], [expected] * len(checked))
                    self.assertEqual(result["status"], "match")

    def test_a_strict_reference_failure_does_not_run_strict_tsrs(self):
        tsrs = Path(os.environ["TSRS_CONFORMANCE_TSRS"]).resolve()
        tsgo = Path(os.environ["TSRS_CONFORMANCE_TSGO"]).resolve()
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp).resolve()
            broken = root / "tsgo-fails-with-checkers"
            broken.write_text(f'#!/bin/sh\ncase "$*" in *--checkers*) exit 3;; esac\nexec "{tsgo}" "$@"\n')
            broken.chmod(0o755)
            write_json(root / "tsconfig.json", {"files": ["a.ts"]})
            (root / "a.ts").write_text("export const x: number = 1;\n")
            result = check_projects(root, ["tsconfig.json"], context(broken, tsrs, root), root / "logs", 30, 5)
            [config] = result["configs"]
            self.assertEqual((config["status"], config["strict"]["status"]), ("match", "reference_failed"))
            self.assertNotIn("tsrs", config["strict"])
            self.assertFalse((root / "logs" / "0" / "tsrs-strict.log").exists())
            self.assertEqual(result["strict_status"], "not_checked")

    def test_a_reference_timeout_does_not_spend_the_budget_on_tsrs(self):
        tsrs = Path(os.environ["TSRS_CONFORMANCE_TSRS"]).resolve()
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            slow = root / "slow-tsgo"
            slow.write_text("#!/bin/sh\nsleep 30\n")
            slow.chmod(0o755)
            write_json(root / "tsconfig.json", {"files": ["a.ts"]})
            (root / "a.ts").write_text("export {};\n")
            result = check_config(root, "tsconfig.json", context(slow, tsrs, root), root / "logs", 0.5)
            self.assertEqual(result["status"], "reference_failed")
            self.assertTrue(result["tsgo"]["timeout"])
            self.assertNotIn("tsrs", result)
            self.assertFalse((root / "logs" / "tsrs.log").exists())


if __name__ == "__main__":
    unittest.main()
