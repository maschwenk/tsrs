#!/usr/bin/env python3
"""Flag instruction-count and peak-memory regressions between this bench run and the previous one (bench/README.md
"Regression flag").

  bench/regressions.py (<result.json> | --latest) [--threshold 1] [--comment]

Compares each project's single-threaded instruction count and peak RSS (bench/count.py, recorded by bench/run.py)
with the newest earlier result in bench/results from the same runner label and build. The instruction count repeats
to about 0.001% and peak RSS to under 0.4%, so a change past the thresholds below is the code's; only a different CPU
model or C library (which pick different memcpy-style routines) can move them otherwise, and then the project is not
judged (the machine is compared per project: a parallel run records a project's machine when it is not the run's).
A different Rust compiler (a rust-toolchain.toml bump) moves them too: the table is printed as the upgrade's
measurement, and nothing is flagged.
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

# The result fields this repository's bench/run.py writes. maschwenk/tsrslint keeps a copy of this script; the
# differences are these constants.
TOOL = "tsrs"
COUNT_PATH = ("single", "tsrs")
WHAT = "single-threaded type check"

# A metric regresses when it rises by more than `percent` and by more than `floor` (peak RSS of an 80 MiB run moves
# by up to 0.4 MiB between identical runs).
METRICS = [
    {"key": "instructions", "label": "instructions", "percent": 1.0, "floor": 0, "unit": 1e9, "suffix": "G"},
    {"key": "max_rss_bytes", "label": "peak memory", "percent": 1.0, "floor": 2 * 2**20, "unit": 2**20, "suffix": "MiB"},
]
MAX_PR_COMMENTS = 3


def measured(result, project, key="instructions"):
    node = result.get("projects", {}).get(project)
    for k in COUNT_PATH + (key,):
        node = node.get(k) if isinstance(node, dict) else None
    return node


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
                      and r.get(TOOL, {}).get("build") == new.get(TOOL, {}).get("build"))
        has_counts = any(measured(r, name) for name in r.get("projects", {}))
        if same_setup and has_counts and r.get("date", "") <= new.get("date", ""):
            candidates.append(r)
    return max(candidates, key=lambda r: r["date"]) if candidates else None


def project_machine(result, project):
    """The machine a project was measured on: the run's, unless bench/run.py --merge recorded another on the project."""
    return result.get("projects", {}).get(project, {}).get("machine") or result.get("machine", {})


def machine_difference(old, new, project):
    """Why the project's counts in these two runs are not comparable, or None."""
    for key in ("cpu", "libc"):
        a, b = project_machine(old, project).get(key), project_machine(new, project).get(key)
        if a != b:
            return f"{key} differs ({a!r} -> {b!r})"
    return None


def compiler_difference(old, new):
    """The rustc change between the two runs, or None (bench/run.py records `rustc -V`)."""
    a, b = old.get(TOOL, {}).get("rustc"), new.get(TOOL, {}).get("rustc")
    return None if a == b else f"{a or 'unrecorded'} -> {b or 'unrecorded'}"


def compare(old, new, metrics=METRICS):
    """Rows of (project, metric, old, new, change %, verdict)."""
    rows = []
    for metric in metrics:
        for name in new.get("projects", {}):
            before, after = measured(old, name, metric["key"]), measured(new, name, metric["key"])
            if not before or not after or machine_difference(old, new, name):
                continue
            change = 100.0 * (after / before - 1)
            if change > metric["percent"] and after - before > metric["floor"]:
                verdict = "regression"
            elif change < -metric["percent"] and before - after > metric["floor"]:
                verdict = "improvement"
            else:
                verdict = "unchanged"
            rows.append((name, metric, before, after, change, verdict))
    return rows


def fmt(value, metric):
    return f"{value / metric['unit']:.3f} {metric['suffix']}" if metric["suffix"] == "G" else f"{value / metric['unit']:.1f} {metric['suffix']}"


def table(rows):
    lines = [f"| project | {TOOL} | before | after | change | |", "| --- | --- | ---: | ---: | ---: | --- |"]
    for name, metric, before, after, change, verdict in rows:
        lines.append(f"| {name} | {metric['label']} | {fmt(before, metric)} | {fmt(after, metric)} | {change:+.2f}% | {verdict} |")
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
    ap.add_argument("--threshold", type=float, help="override the percent for instructions (default 1)")
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
    old_commit, new_commit = old[TOOL]["commit"], new[TOOL]["commit"]
    skipped = {name: why for name in new.get("projects", {}) if (why := machine_difference(old, new, name))}
    for name, why in skipped.items():
        print(f"regressions: not comparing {name} with {old_commit[:12]}: {why}")
    if skipped and len(skipped) == len(new.get("projects", {})):
        return
    metrics = [dict(m, percent=args.threshold) if m["key"] == "instructions" and args.threshold is not None else m
               for m in METRICS]
    rows = compare(old, new, metrics)
    compiler = compiler_difference(old, new)
    regressions = [r for r in rows if r[5] == "regression"] if not compiler else []
    rules = "; ".join(f"{m['label']} up more than {m['percent']:g}%" + (f" and {m['floor'] / m['unit']:g} {m['suffix']}"
                                                                        if m["floor"] else "") for m in metrics)
    judged = (f"rustc changed, {compiler}: the changes measure the compiler, nothing is flagged" if compiler else
              f"regression: {rules}")
    summary = (f"Bench, {WHAT}, {old_commit[:12]} -> {new_commit[:12]} ({judged})\n\n" + table(rows) + "\n")
    print(summary)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as f:
            f.write("### " + summary)
    for name, metric, before, after, change, _ in regressions:
        print(f"::warning title=Bench regression::{name}: {TOOL} {metric['label']} {change:+.2f}% "
              f"({fmt(before, metric)} -> {fmt(after, metric)}) since {old_commit[:12]}")
    if not regressions or not args.comment:
        return

    prs = merged_pull_requests(old_commit, new_commit)
    scope = (f"this merge" if len(prs) <= 1 else
             f"one of the {len(prs)} merges in this range ({', '.join(f'#{n}' for n in prs)})")
    body = (f"**Bench: regression** after {scope}.\n\n"
            f"The bench on `{new_commit[:12]}` measured {TOOL} higher than the previous run on `{old_commit[:12]}` "
            f"({WHAT}, same CPU model and C library). Instruction counts repeat to about 0.001% and peak memory to "
            f"under 0.4%, so the change comes from the code. Wall time is in `bench/results/`.\n\n"
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
