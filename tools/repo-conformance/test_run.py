import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

from run import (Context, check_config, check_projects, child_env, compare, difference, execute, install_plan,
                 read_compilation, summarize, write_json)


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
            self.assertIn("These shard reports predate Go mode", out.read_text())
            write_json(results / "duplicate" / "shard-0.json", shard(0, "a/one"))
            self.assertFalse(summarize(manifest, results, out))
            (results / "duplicate" / "shard-0.json").unlink()
            wrong = shard(1, "a/two")
            wrong["typescript_commit"] = "different-go-version"
            write_json(results / "shard-1.json", wrong)
            self.assertFalse(summarize(manifest, results, out))

    def test_a_rerun_shard_fills_a_lost_one_only_at_the_pinned_commits(self):
        manifest = {"tsrs_commit": "rust-pin", "typescript_commit": "go-pin", "shards": 2,
                    "repositories": [{"repository": "a/one", "commit": "c1"}, {"repository": "a/two", "commit": "c2"}]}
        def shard(index, name, commit, status="match"):
            return {"tsrs_commit": "rust-pin", "typescript_commit": "go-pin", "shard": index,
                    "results": [{"repository": name, "commit": commit, "status": status, "available_configs": 1,
                                 "configs": [{"config": "tsconfig.json", "status": status, "tsgo": {"exit": 0}}]}]}
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            write_json(root / "previous" / "shard-0.json", shard(0, "a/one", "c1"))
            write_json(root / "previous" / "shard-1.json", shard(1, "a/two", "c2", "mismatch"))
            write_json(root / "rerun" / "shard-1.json", shard(1, "a/two", "c2"))
            self.assertTrue(summarize(manifest, root / "rerun", root / "report.md", root / "previous"))
            report = (root / "report.md").read_text()
            self.assertIn("2 of 2 repositories have a result; 2 had at least one project", report)
            self.assertIn("2 identical (2 without errors, 0 with the same errors), 0 different", report)
            write_json(root / "rerun" / "shard-1.json", shard(1, "a/two", "newer-commit"))
            self.assertFalse(summarize(manifest, root / "rerun", root / "report.md", root / "previous"))
            self.assertIn("a/two: checked newer-commit, manifest pins c2", (root / "report.md").read_text())

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
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            write_json(root / "tsconfig.json", {"files": [], "references": [{"path": "./tsconfig.app.json"}, {"path": "lib"}]})
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

    def test_go_mode_is_reported_separately_and_does_not_clear_a_default_difference(self):
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
            self.assertEqual(config["go"]["status"], "match")
            self.assertEqual((result["status"], result["go_status"]), ("mismatch", "match"))
            logs = root / "logs" / "0"
            self.assertIn("{ status: string; user?: undefined; }", (logs / "tsrs.log").read_text())
            self.assertIn("{ user?: undefined; status: string; }", (logs / "tsrs-go.log").read_text())
            self.assertTrue((logs / "diagnostics.diff").exists())
            self.assertFalse((logs / "diagnostics-go.diff").exists())
            manifest = {"tsrs_commit": "rust-pin", "typescript_commit": "go-pin", "shards": 1,
                        "repositories": [{"repository": "a/one"}]}
            write_json(root / "results" / "shard-0.json", {
                "tsrs_commit": "rust-pin", "typescript_commit": "go-pin", "shard": 0, "modes": ["default", "go"],
                "results": [dict(result, repository="a/one", available_configs=1)]})
            self.assertFalse(summarize(manifest, root / "results", root / "report.md"))
            report = (root / "report.md").read_text()
            self.assertIn("1 different, 0 tsrs failures", report)
            self.assertIn("1 of the 1 projects compared in default mode were also compared in Go mode: 1 identical", report)
            self.assertIn("1 projects that differ or fail in default mode are identical in Go mode", report)
            self.assertIn("tsconfig.json: mismatch (diagnostics); Go mode match", report)

    def test_go_mode_stays_within_the_check_budget(self):
        """A tsrs that hangs only in Go mode: the default comparisons keep their time, the whole check stays within its
        budget, and a Go-mode timeout is a failure only when tsrs had at least the default run's limit."""
        tsrs = Path(os.environ["TSRS_CONFORMANCE_TSRS"]).resolve()
        tsgo = Path(os.environ["TSRS_CONFORMANCE_TSGO"]).resolve()
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp).resolve()
            hang = root / "tsrs-hangs-in-go-mode"
            hang.write_text(f'#!/bin/sh\ncase "$*" in *checkerAssignment*) sleep 60;; esac\nexec "{tsrs}" "$@"\n')
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
                    self.assertEqual([c["go"]["status"] for c in result["configs"]], [expected] * len(checked))
                    self.assertEqual(result["status"], "match")

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
