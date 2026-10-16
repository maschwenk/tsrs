import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from run import (child_env, compare_failures, markdown_report, normalize_text, package_progress, read_failures,
                 resolve_dt_commit, validate_run)


class ResultTests(unittest.TestCase):
    def test_package_progress_accepts_parallel_prefixes_and_failures(self):
        with tempfile.TemporaryDirectory() as temporary:
            log = Path(temporary) / "run.log"
            log.write_text("1> alpha START\n2> beta/v1 START\n2> beta/v1 failing:\n1> alpha OK\n")
            self.assertEqual(package_progress(log), ({"alpha", "beta/v1"}, {"alpha", "beta/v1"}))

    def test_failures_are_normalized_sorted_and_compared_exactly(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / "failures.json"
            path.write_text(json.dumps([
                {"path": "zeta", "error": f"{root}/z.ts: changed\r\n"},
                {"path": "alpha", "error": "same\n"},
            ]))
            failures = read_failures(path, {root: "<root>"})
            self.assertEqual(list(failures), ["alpha", "zeta"])
            self.assertEqual(failures["zeta"], "<root>/z.ts: changed\n")
            self.assertEqual(compare_failures(failures, dict(failures)), {
                "reference_only": [], "candidate_only": [], "changed": [],
            })
            candidate = {"alpha": "different", "candidate": "new"}
            self.assertEqual(compare_failures(failures, candidate), {
                "reference_only": ["zeta"], "candidate_only": ["candidate"], "changed": ["alpha"],
            })

    def test_duplicate_or_malformed_failures_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "failures.json"
            for value in ([{"path": "a", "error": "x"}, {"path": "a", "error": "y"}], {"path": "a"},
                          [{"path": 1, "error": "x"}]):
                with self.subTest(value=value):
                    path.write_text(json.dumps(value))
                    with self.assertRaises(ValueError):
                        read_failures(path, {})

    def test_run_validation_requires_complete_coverage_and_expected_exit(self):
        validate_run({"exit": 2, "timeout": False}, {"a": "x", "b": "y"}, {"a", "b"}, {"a", "b"}, "ref")
        cases = [
            ({"exit": 0, "timeout": True}, {"a"}, {"a"}),
            ({"exit": -9, "timeout": False}, {"a"}, {"a"}),
            ({"exit": 0, "timeout": False}, {"a"}, set()),
            ({"exit": 0, "timeout": False}, set(), set()),
        ]
        for run, started, finished in cases:
            with self.subTest(run=run, started=started, finished=finished), self.assertRaises(RuntimeError):
                validate_run(run, {"a": "x"}, started, finished, "ref")

    def test_report_records_matches_differences_and_setup_failures(self):
        base = {
            "definitely_typed_commit": "d" * 40, "typescript_commit": "t" * 40, "tsrs_commit": "r" * 40,
            "packages": 9000, "reference_failures": 2, "candidate_failures": 2,
            "dtslint_runner_version": "1.2.3",
        }
        matched = markdown_report(dict(base, status="match", differences={
            "reference_only": [], "candidate_only": [], "changed": [], "coverage": [],
        }))
        self.assertIn("Every package produced the same result", matched)
        mismatch = markdown_report(dict(base, status="mismatch", differences={
            "reference_only": ["a"], "candidate_only": [], "changed": ["b"], "coverage": [],
        }))
        self.assertIn("Reference only", mismatch)
        self.assertIn("`b`", mismatch)
        self.assertIn("boom", markdown_report({"status": "setup_failed", "reason": "boom"}))

    def test_only_full_lowercase_shas_are_accepted_as_manual_commits(self):
        commit = "a" * 40
        self.assertEqual(resolve_dt_commit(commit), commit)
        for invalid in ("main", "A" * 40, "a" * 39, "a" * 41, "a" * 39 + ";"):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                resolve_dt_commit(invalid)


class IsolationTests(unittest.TestCase):
    def test_child_environment_drops_credentials_and_host_configuration(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            saved = dict(os.environ)
            try:
                os.environ.update({"GH_TOKEN": "secret", "ACTIONS_RUNTIME_TOKEN": "secret", "HOME": "/host"})
                env = child_env(root / "home", root / "cache")
            finally:
                os.environ.clear()
                os.environ.update(saved)
            self.assertNotIn("GH_TOKEN", env)
            self.assertNotIn("ACTIONS_RUNTIME_TOKEN", env)
            self.assertEqual(env["HOME"], str(root / "home"))
            output = subprocess.check_output(
                [sys.executable, "-c", "import os; print(os.environ.get('GH_TOKEN', ''))"], env=env, text=True,
            )
            self.assertEqual(output.strip(), "")

    def test_normalization_changes_only_declared_paths_and_line_endings(self):
        replacements = {Path("/a"): "<a>", Path("/a/package"): "<package>"}
        self.assertEqual(normalize_text("/a/package/x\r\n/a/lib keep  ", replacements),
                         "<package>/x\n<a>/lib keep  ")


if __name__ == "__main__":
    unittest.main()
