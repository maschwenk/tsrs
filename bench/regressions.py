#!/usr/bin/env python3
"""Flag instruction-count regressions between this bench run and the previous one (bench/README.md "Regression flag").

  bench/regressions.py (<result.json> | --latest) [--threshold 1] [--comment]

Compares each project's single-threaded tsrs instruction count (bench/count.py, recorded by bench/run.py) with the
newest earlier result in bench/results from the same runner label and build. The count repeats to about 0.001%, so
any change is the code's; only a different CPU model or C library (which pick different memcpy-style routines) can
move it otherwise, and then nothing is judged. A count up by more than --threshold percent is a regression.
Regressions are printed as warnings and, with --comment, posted as a comment on each pull request merged since the
previous run (or on the commit when there is none). This script never fails the job.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import urllib.request
from pathlib import Path

MAX_PR_COMMENTS = 3


def instructions(result, project, compiler):
    single = result.get("projects", {}).get(project, {}).get("single") or {}
    return (single.get(compiler) or {}).get("instructions")


def previous_result(new, path, results_dir):
    """The newest earlier result from the same machine and build that has instruction counts."""
    candidates = []
    for p in Path(results_dir).glob("*.json"):
        if p.resolve() == Path(path).resolve():
            continue
        try:
            r = json.loads(p.read_text())
        except ValueError:
            continue
        same_setup = (r.get("machine", {}).get("label") == new.get("machine", {}).get("label")
                      and r.get("tsrs", {}).get("build") == new.get("tsrs", {}).get("build"))
        has_counts = any(instructions(r, name, "tsrs") for name in r.get("projects", {}))
        if same_setup and has_counts and r.get("date", "") <= new.get("date", ""):
            candidates.append(r)
    return max(candidates, key=lambda r: r["date"]) if candidates else None


def machine_difference(old, new):
    """Why counts from these two runs are not comparable, or None."""
    for key in ("cpu", "libc"):
        a, b = old.get("machine", {}).get(key), new.get("machine", {}).get(key)
        if a != b:
            return f"{key} differs ({a!r} -> {b!r})"
    return None


def compare(old, new, threshold):
    """Rows of (project, old, new, change %, verdict)."""
    rows = []
    for name in new.get("projects", {}):
        before, after = instructions(old, name, "tsrs"), instructions(new, name, "tsrs")
        if not before or not after:
            continue
        change = 100.0 * (after / before - 1)
        verdict = "regression" if change > threshold else "improvement" if change < -threshold else "unchanged"
        rows.append((name, before, after, change, verdict))
    return rows


def table(rows):
    lines = ["| project | tsrs instructions before | after | change | |", "| --- | ---: | ---: | ---: | --- |"]
    for name, before, after, change, verdict in rows:
        lines.append(f"| {name} | {before / 1e9:.3f} G | {after / 1e9:.3f} G | {change:+.2f}% | {verdict} |")
    return "\n".join(lines)


def merged_pull_requests(old_commit, new_commit):
    """Pull request numbers merged on the first-parent line in old_commit..new_commit, oldest first."""
    log = subprocess.run(["git", "log", "--first-parent", "--reverse", "--format=%s", f"{old_commit}..{new_commit}"],
                         capture_output=True, text=True)
    numbers = []
    for subject in log.stdout.splitlines():
        m = re.match(r"Merge pull request #(\d+) ", subject) or re.search(r"\(#(\d+)\)$", subject)
        if m and int(m.group(1)) not in numbers:
            numbers.append(int(m.group(1)))
    return numbers


def post(url, body):
    token = os.environ.get("GITHUB_TOKEN")
    if not token:
        raise RuntimeError("GITHUB_TOKEN is not set")
    req = urllib.request.Request(url, data=json.dumps({"body": body}).encode(), method="POST", headers={
        "Authorization": f"Bearer {token}", "Accept": "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28"})
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.loads(resp.read()).get("html_url")


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("result", type=Path, nargs="?")
    ap.add_argument("--latest", action="store_true", help="judge the newest result in --results-dir (by its date)")
    ap.add_argument("--results-dir", type=Path, default=Path(__file__).parent / "results")
    ap.add_argument("--threshold", type=float, default=1.0, help="percent increase that counts as a regression")
    ap.add_argument("--comment", action="store_true", help="comment on the merged pull requests (needs GITHUB_TOKEN)")
    ap.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY"))
    args = ap.parse_args(argv)
    if args.latest:
        dated = [(json.loads(p.read_text()).get("date", ""), p) for p in args.results_dir.glob("*.json")]
        args.result = max(dated)[1] if dated else None
    if args.result is None:
        ap.error("give a result file or --latest")

    new = json.loads(args.result.read_text())
    old = previous_result(new, args.result, args.results_dir)
    if old is None:
        print("regressions: no earlier result with instruction counts to compare with")
        return
    old_commit, new_commit = old["tsrs"]["commit"], new["tsrs"]["commit"]
    why_not = machine_difference(old, new)
    if why_not:
        print(f"regressions: not comparing with {old_commit[:12]}: {why_not}")
        return
    rows = compare(old, new, args.threshold)
    regressions = [r for r in rows if r[4] == "regression"]
    summary = (f"Instruction counts, single-threaded, {old_commit[:12]} -> {new_commit[:12]} "
               f"(regression: tsrs up more than {args.threshold:g}%)\n\n" + table(rows) + "\n")
    print(summary)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as f:
            f.write("### " + summary)
    for name, before, after, change, _ in regressions:
        print(f"::warning title=Instruction-count regression::{name}: tsrs instructions {change:+.2f}% "
              f"({before / 1e9:.3f} G -> {after / 1e9:.3f} G) since {old_commit[:12]}")
    if not regressions or not args.comment:
        return

    prs = merged_pull_requests(old_commit, new_commit)
    scope = (f"this merge" if len(prs) <= 1 else
             f"one of the {len(prs)} merges in this range ({', '.join(f'#{n}' for n in prs)})")
    body = (f"**Bench: instruction-count regression** after {scope}.\n\n"
            f"The bench on `{new_commit[:12]}` measured more user-space instructions for tsrs than the previous run on "
            f"`{old_commit[:12]}` (single-threaded type check, same CPU model and C library). The count repeats to about "
            f"0.001%, so the change comes from the code. Wall time and memory are in `bench/results/`.\n\n"
            f"{table(regressions)}\n\n"
            f"Flag only; nothing fails. A deliberate trade (such as memory for CPU) needs no action. "
            f"Details: `bench/README.md`, \"Regression flag\".")
    # More than a few merges in one run (a skipped or failed bench) gets one comment on the commit, not one per PR.
    if 1 <= len(prs) <= MAX_PR_COMMENTS:
        targets = [f"https://api.github.com/repos/{args.repo}/issues/{n}/comments" for n in prs]
    else:
        targets = [f"https://api.github.com/repos/{args.repo}/commits/{new_commit}/comments"]
    for url in targets:
        try:
            print("commented:", post(url, body))
        except Exception as e:  # a failed comment must not fail the bench
            print(f"::warning title=Regression comment failed::{url}: {e}")


if __name__ == "__main__":
    main(sys.argv[1:])
