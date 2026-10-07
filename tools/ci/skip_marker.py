#!/usr/bin/env python3
"""Fail a pull request whose squash commit would skip CI and the bench (bench/README.md "CI").

  tools/ci/skip_marker.py --title-file <f> --body-file <f> --base <sha> --head <sha>
  tools/ci/skip_marker.py --self-test

`gh pr merge --squash` builds the commit message from the pull request's title and body, or for a pull request of
several commits from the list of their subjects, and GitHub Actions and Depot CI skip a push whose message carries a
marker such as "[skip ci]" anywhere in it. A notes-only pull request carries the marker in its title on purpose, so
that prose does not start a bench run. A pull request that changes anything else must not carry one anywhere: on
2026-10-07 two code changes landed on main without CI and without a results file because their bodies and commit
subjects mentioned the marker. This reads the title, the body and the subjects of base..head and fails when any of
them has a marker while the diff touches a file that is not prose: anything outside notes/, docs/, bench/results/ and
README.md that is not a .md file.
"""

import argparse
import re
import subprocess
import sys

MARKERS = re.compile(r"\[(skip ci|ci skip|no ci|skip actions|actions skip)\]", re.IGNORECASE)
PROSE_DIRS = ("notes/", "docs/", "bench/results/")


def is_prose(path: str) -> bool:
    return path == "README.md" or path.endswith(".md") or path.startswith(PROSE_DIRS)


def violations(texts: dict[str, str], files: list[str]) -> list[str]:
    """Messages describing what is wrong, empty when the pull request may merge: `texts` maps a name (title, body,
    a commit subject) to its text, `files` are the paths the pull request changes."""
    marked = [name for name, text in texts.items() if MARKERS.search(text or "")]
    code = [f for f in files if not is_prose(f)]
    if not marked or not code:
        return []
    shown = ", ".join(code[:5]) + (f" and {len(code) - 5} more" if len(code) > 5 else "")
    return [f"{name} carries a CI-skip marker, but the pull request changes {shown}" for name in marked]


def self_test() -> None:
    assert violations({"title": "notes: x [skip ci]"}, ["notes/a.md", "docs/B.md", "README.md"]) == []
    assert violations({"title": "fix: y", "body": "plain"}, ["crates/a.rs"]) == []
    assert violations({"title": "fix: y", "body": "see [skip ci] commits"}, ["crates/a.rs"]) == [
        "body carries a CI-skip marker, but the pull request changes crates/a.rs"
    ]
    assert violations({"title": "t", "body": "", "commit 2": "notes: z [SKIP CI]"}, ["notes/z.md", "bench/run.py"]) == [
        "commit 2 carries a CI-skip marker, but the pull request changes bench/run.py"
    ]
    assert violations({"title": "t [ci skip]", "body": None}, ["bench/results/x.json"]) == []
    print("skip_marker self-test: ok")


def main(argv) -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--title-file")
    ap.add_argument("--body-file")
    ap.add_argument("--base", help="the base branch's commit")
    ap.add_argument("--head", help="the pull request's head commit")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args(argv)
    if args.self_test:
        self_test()
        return 0
    if not (args.title_file and args.body_file and args.base and args.head):
        ap.error("--title-file, --body-file, --base and --head are required")
    texts = {"title": open(args.title_file).read(), "body": open(args.body_file).read()}
    subjects = subprocess.run(["git", "log", "--format=%s", f"{args.base}..{args.head}"], capture_output=True, text=True, check=True)
    for i, subject in enumerate(subjects.stdout.splitlines(), 1):
        texts[f"commit {i} ({subject[:50]})"] = subject
    files = subprocess.run(["git", "diff", "--name-only", f"{args.base}...{args.head}"], capture_output=True, text=True, check=True)
    problems = violations(texts, files.stdout.split())
    for p in problems:
        print(f"::error::{p}: its squash commit would land on main without CI and without a bench run; "
              "rephrase (quote the marker in a code span with a space) or make the pull request notes-only")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
