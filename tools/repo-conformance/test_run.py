import os
from pathlib import Path
import sys
import tempfile
import time
import unittest

from run import check_config, compare, execute, read_compilation, summarize, write_json


def compilation(text="", files=1, code=0, timeout=False):
    return {"diagnostics": text, "files": files, "exit": code, "timeout": timeout}


class ComparisonTests(unittest.TestCase):
    def test_dropped_errors_changed_elaborations_and_reordered_errors_fail_parity(self):
        first = "a.ts(1,1): error TS2322: wrong type.\n  'foo' is missing."
        second = "b.ts(1,1): error TS2345: wrong argument."
        baseline = compilation(first + "\n" + second, code=2)
        for text in (second, first.replace("foo", "bar") + "\n" + second, second + "\n" + first):
            with self.subTest(text=text):
                self.assertEqual(compare(baseline, compilation(text, code=2)), "mismatch")

    def test_identical_diagnostics_cannot_hide_exit_or_loaded_file_differences(self):
        self.assertEqual(compare(compilation(), compilation(code=2)), "mismatch")
        self.assertEqual(compare(compilation(), compilation(files=2)), "mismatch")

    def test_crashes_timeouts_and_empty_projects_never_count_as_matches(self):
        for code, timeout in ((-9, False), (101, False), (0, True)):
            with self.subTest(code=code, timeout=timeout):
                broken = compilation(code=code, timeout=timeout)
                self.assertEqual(compare(compilation(), broken), "tsrs_failed")
                self.assertEqual(compare(broken, broken), "reference_failed")
        self.assertEqual(compare(compilation(files=0), compilation(files=0)), "no_inputs")

    def test_timing_counters_are_excluded_but_multiline_diagnostics_are_preserved(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            log = root / "compiler.log"
            log.write_text("$ compiler\r\n" + str(root) + "/a.ts(1,1): error TS2322: wrong.\r\n"
                           "  Detail.\r\nFiles: 3\r\nCheck time: 0.01s\r\nFS: stat: 5\r\n")
            result = read_compilation({"exit": 2, "timeout": False}, log, root, root / "bin" / "tsrs")
            self.assertEqual(result["diagnostics"], "<repo>/a.ts(1,1): error TS2322: wrong.\n  Detail.")
            self.assertEqual(result["files"], 3)

    def test_timeout_kills_descendants_before_they_can_write_after_the_repository_is_removed(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            marker = root / "escaped-child"
            child = f"import time; from pathlib import Path; time.sleep(0.5); Path({str(marker)!r}).touch()"
            parent = f"import subprocess,sys,time; subprocess.Popen([sys.executable,'-c',{child!r}]); time.sleep(30)"
            result = execute([sys.executable, "-c", parent], root, root / "timeout.log", 0.2)
            self.assertTrue(result["timeout"])
            time.sleep(0.6)
            self.assertFalse(marker.exists())


class ReportTests(unittest.TestCase):
    def test_missing_duplicate_and_wrong_revision_results_cannot_produce_a_green_report(self):
        manifest = {"tsrs_commit": "rust-pin", "typescript_commit": "go-pin", "shards": 2,
                    "repositories": [{"repository": "a/one"}, {"repository": "a/two"}]}
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            results = root / "results"
            out = root / "report.md"
            def shard(index, name):
                return {"tsrs_commit": "rust-pin", "typescript_commit": "go-pin", "shard": index,
                        "results": [{"repository": name, "status": "match", "available_configs": 1,
                                     "configs": [{"config": "tsconfig.json", "status": "match"}]}]}
            write_json(results / "shard-0.json", shard(0, "a/one"))
            self.assertFalse(summarize(manifest, results, out))
            self.assertIn("Missing repository result: a/two", out.read_text())
            write_json(results / "shard-1.json", shard(1, "a/two"))
            self.assertTrue(summarize(manifest, results, out))
            write_json(results / "duplicate" / "shard-0.json", shard(0, "a/one"))
            self.assertFalse(summarize(manifest, results, out))
            (results / "duplicate" / "shard-0.json").unlink()
            wrong = shard(1, "a/two")
            wrong["typescript_commit"] = "different-go-version"
            write_json(results / "shard-1.json", wrong)
            self.assertFalse(summarize(manifest, results, out))

    def test_setup_failures_and_empty_projects_do_not_establish_compatibility(self):
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
    def test_real_compilers_check_valid_and_invalid_composite_projects_without_reusing_build_info(self):
        tsrs = Path(os.environ["TSRS_CONFORMANCE_TSRS"]).resolve()
        tsgo = Path(os.environ["TSRS_CONFORMANCE_TSGO"]).resolve()
        with tempfile.TemporaryDirectory(dir=Path.cwd()) as temp:
            root = Path(temp)
            write_json(root / "tsconfig.json", {"compilerOptions": {"strict": True, "composite": True}, "files": ["a.ts"]})
            out = (root / "logs").relative_to(Path.cwd())
            for source, code in (("const x: number = 1;\n", 0), ("const x: number = 'no';\n", 2),
                                 ("const x: number = 1;\n", 0)):
                (root / "a.ts").write_text(source)
                result = check_config(root, "tsconfig.json", tsgo, tsrs, out, 30)
                self.assertEqual(result["status"], "match", result)
                self.assertEqual(result["tsgo"]["exit"], code)
                self.assertGreater(result["tsgo"]["files"], 0)
                if code:
                    self.assertIn("error TS2322", (root / "logs" / "tsrs.log").read_text())
                self.assertFalse((root / "a.js").exists())
                self.assertFalse(any((root / "logs").glob("*.tsbuildinfo")))


if __name__ == "__main__":
    unittest.main()
