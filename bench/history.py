#!/usr/bin/env python3
"""Per-commit bench history: what each benchmarked commit changed (bench/README.md "History").

  bench/history.py [--metrics instructions,peak,wall,wide_wall,wide_peak,wasm_warm] [--projects a,b] [--since <commit>]
                   [--branch origin/main] [--all] [--tsv]

Orders bench/results/*.json by the first-parent history of `--branch` (oldest first; a result whose commit is not on
that branch is left out unless --all, which appends such results by date) and prints one table per metric: a row per
result, a column per project, each cell the value and its change against the row above, which with one bench per push
(.depot/workflows/bench.yml) is the previous merge. Metrics: `instructions` and `peak` are the single-threaded
instruction count and peak RSS from bench/count.py (deterministic to ~0.001% and ~0.4%: a change is the code's);
`wall` is the default-mode wall on the fixed 8-vCPU runner (noisy, up to 10-20% between runs of the same code);
`wide_wall` and `wide_peak` are the 64-vCPU default-mode wall and peak, with `bun check`'s wall beside the wall;
`wasm_warm` is tsrs.wasm's warm run from the WebAssembly table (bench/wasm.py's own `*-wasm.json` files, one per
dispatch of .depot/workflows/bench-wasm.yml; noisy like the walls). A cell past the thresholds of bench/regressions.py
(instructions or peak up more than 1%, and peak by more than 2 MiB) is marked `!`; a wall or wide-peak change next
to an instruction change under 0.3% is marked `~`: the code did not change, so that is the runner's noise (a median 2%, up to 10%, between publishes of the same code on 2026-10-07). Use it to find which commit cost what, and whether a commit's gain was worth its cost.
"""

import argparse
import json
import subprocess
import sys
from pathlib import Path

METRICS = {
    # name: (mode, compiler, field, unit, suffix, decimals, flag percent, flag floor)
    "instructions": ("single", "tsrs", "instructions", 1e9, "G", 2, 1.0, 0),
    "peak": ("single", "tsrs", "max_rss_bytes", 2**20, "MiB", 0, 1.0, 2 * 2**20),
    "wall": ("default", "tsrs", "wall_s", 1, "s", 2, None, 0),
    "wide_wall": ("wide", "tsrs", "wall_s", 1, "s", 2, None, 0),
    "wide_peak": ("wide", "tsrs", "peak_rss_bytes", 2**20, "MiB", 0, None, 0),
    # bench/wasm.py's results (`"kind": "wasm"` files): tsrs.wasm's warm run, one thread.
    "wasm_warm": ("wasm", "tsrs-wasm", "warm_s", 1, "s", 2, None, 0),
}
DEFAULT_METRICS = "instructions,peak,wide_wall"


def git(*args):
    r = subprocess.run(["git", *args], capture_output=True, text=True)
    return r.stdout.strip() if r.returncode == 0 else None


def load(results_dir: Path) -> list[dict]:
    results = []
    for p in sorted(results_dir.glob("*.json")):
        try:
            r = json.loads(p.read_text())
        except ValueError:
            continue
        if "tsrs" in r and "projects" in r:
            r["_file"] = p.name
            results.append(r)
    return results


def order(results: list[dict], branch: str, include_all: bool) -> list[dict]:
    """Results in first-parent order of `branch`, oldest first; the same commit's runs by date."""
    line = git("rev-list", "--first-parent", branch)
    position = {}
    if line:
        for i, sha in enumerate(reversed(line.split())):
            position[sha] = i
    placed, unplaced = [], []
    for r in results:
        short = r["tsrs"].get("commit", "")
        full = git("rev-parse", "--verify", "--quiet", f"{short}^{{commit}}") if short else None
        if full in position:
            placed.append((position[full], r.get("date", ""), r))
        else:
            unplaced.append((r.get("date", ""), r))
    ordered = [r for _, _, r in sorted(placed, key=lambda t: (t[0], t[1]))]
    if include_all:
        ordered += [r for _, r in sorted(unplaced, key=lambda t: t[0])]
    elif unplaced and not line:
        sys.exit(f"history: no git history for {branch!r}; use --all to order by date")
    return ordered


def value(r: dict, project: str, metric: str):
    mode, compiler, field, *_ = METRICS[metric]
    if mode == "wasm":
        node = r["projects"].get(project, {}).get(compiler, {}).get(field)
        return node.get("median") if isinstance(node, dict) else None
    node = r["projects"].get(project, {}).get(mode, {}).get(compiler, {})
    return node.get(field) if isinstance(node, dict) else None


def bun_wall(r: dict, project: str):
    node = r["projects"].get(project, {}).get("wide", {}).get("bun", {})
    return node.get("wall_s") if isinstance(node, dict) else None


NOISE_INSTRUCTION_PERCENT = 0.3


def cell(metric: str, new, old, bun=None, same_code: bool = False) -> str:
    _, _, _, unit, suffix, decimals, flag_percent, flag_floor = METRICS[metric]
    if new is None:
        return ""
    text = f"{new / unit:.{decimals}f}"
    if old:
        change = (new - old) / old * 100
        flag = flag_percent is not None and change > flag_percent and new - old > flag_floor
        # A wall or peak change while the single-threaded instructions did not move is the runner's noise
        # (a few percent, up to 10%, between runs of the same code), marked `~`.
        noise = same_code and metric in ("wall", "wide_wall", "wide_peak")
        text += f" ({change:+.1f}%{'!' if flag else ''}{'~' if noise else ''})"
    if bun is not None:
        text += f" / bun {bun:.2f}"
    return text


def subject(commit: str) -> str:
    s = git("log", "-1", "--format=%s", commit) or ""
    return s if len(s) <= 72 else s[:69] + "..."


def table(rows: list[dict], projects: list[str], metric: str, tsv: bool) -> str:
    _, _, _, _, suffix, *_ = METRICS[metric]
    with_bun = metric == "wide_wall"
    header = ["commit", "date", "subject"] + projects
    lines = []
    previous = {}
    previous_instructions = {}
    for r in rows:
        commit = r["tsrs"].get("commit", "")[:12]
        cells = [commit, r.get("date", ""), subject(commit)]
        for p in projects:
            new = value(r, p, metric)
            instructions = value(r, p, "instructions")
            old_instructions = previous_instructions.get(p)
            same_code = (instructions is not None and old_instructions
                         and abs(instructions / old_instructions - 1) * 100 < NOISE_INSTRUCTION_PERCENT)
            cells.append(cell(metric, new, previous.get(p), bun_wall(r, p) if with_bun else None, same_code))
            if new is not None:
                previous[p] = new
            if instructions is not None:
                previous_instructions[p] = instructions
        lines.append(cells)
    if tsv:
        return "\n".join("\t".join(c) for c in [header] + lines)
    md = [f"### {metric} ({suffix}; change against the row above)", "",
          "| " + " | ".join(header) + " |", "| " + " | ".join(["---"] * 3 + ["---:"] * len(projects)) + " |"]
    md += ["| " + " | ".join(c) + " |" for c in lines]
    return "\n".join(md)


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--results-dir", type=Path, default=Path(__file__).parent / "results")
    ap.add_argument("--metrics", default=DEFAULT_METRICS, help=f"comma-separated, of {', '.join(METRICS)}")
    ap.add_argument("--projects", help="comma-separated bench/projects.json names (default: every project measured)")
    ap.add_argument("--since", help="start at this commit (a prefix is fine)")
    ap.add_argument("--branch", default="origin/main", help="the history that orders the results")
    ap.add_argument("--all", action="store_true", help="append results whose commit is not on --branch, by date")
    ap.add_argument("--tsv", action="store_true", help="tab-separated values instead of markdown")
    args = ap.parse_args(argv)
    metrics = [m.strip() for m in args.metrics.split(",") if m.strip()]
    unknown = [m for m in metrics if m not in METRICS]
    if unknown:
        ap.error(f"unknown metric(s) {', '.join(unknown)}; choose from {', '.join(METRICS)}")
    rows = order(load(args.results_dir), args.branch, args.all)
    if args.since:
        start = next((i for i, r in enumerate(rows) if r["tsrs"].get("commit", "").startswith(args.since)), None)
        if start is None:
            ap.error(f"--since {args.since}: no result for that commit")
        rows = rows[start:]
    if not rows:
        sys.exit("history: no results")
    tables = []
    for m in metrics:
        # The wasm metric's rows are bench/wasm.py's results; every other metric's are bench/run.py's.
        mrows = [r for r in rows if (r.get("kind") == "wasm") == (METRICS[m][0] == "wasm")]
        if args.projects:
            projects = [p.strip() for p in args.projects.split(",") if p.strip()]
        else:
            projects = []
            for r in mrows:
                projects += [p for p in r["projects"] if p not in projects]
        tables.append(table(mrows, projects, m, args.tsv))
    print("\n\n".join(tables))


if __name__ == "__main__":
    main(sys.argv[1:])
