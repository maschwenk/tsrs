#!/usr/bin/env python3
"""A/B of two tsrs binaries (notes/perf-serial-steps.md; not committed): interleaved runs, wall from wait4, every
`--extendedDiagnostics` row, peak RSS; output identity without --extendedDiagnostics; one `perf stat` run per binary
and one single-threaded bench/count.py run per binary. Writes summary.md and runs.json to --out."""

import argparse
import json
import os
import re
import shutil
import statistics
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "bench"))
import run as bench  # noqa: E402

ROW = re.compile(r"^([A-Za-z][^\n]*?):\s+([0-9.]+)s\s*$")


def cmd(cfg, name, tsrs, work, extra):
    p = next(p for p in cfg["projects"] if p["name"] == name)
    cwd, proj = bench.project_path(cfg, p, work)
    return cwd, [tsrs, "-p", str(proj), "--noEmit", "--incremental", "false", "--pretty", "false"] + extra


def timed(cwd, args, out, env=None):
    with open(out, "wb") as f:
        t0 = time.perf_counter()
        proc = subprocess.Popen(args, cwd=cwd, stdout=f, stderr=subprocess.STDOUT, env=env)
        _, status, ru = os.wait4(proc.pid, 0)
        wall = time.perf_counter() - t0
    rows = {}
    for line in out.read_text(errors="replace").splitlines():
        m = ROW.match(line)
        if m:
            rows[m.group(1).strip()] = float(m.group(2))
    rows["wall"] = wall
    if "Total time" in rows:
        rows["wall - Total time"] = wall - rows["Total time"]
    rows["maxrss MiB"] = ru.ru_maxrss / 1024
    rows["user s"] = ru.ru_utime
    rows["sys s"] = ru.ru_stime
    return rows


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", required=True)
    ap.add_argument("--new", required=True)
    ap.add_argument("--work", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--projects", default="vscode")
    ap.add_argument("--reps", type=int, default=9)
    ap.add_argument("--checkers", default="", help="comma list; empty: the default count")
    ap.add_argument("--rows", default="", help="regex of rows to show first in the summary")
    a = ap.parse_args()
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    work = Path(a.work)
    cfg = json.loads((ROOT / "bench" / "projects.json").read_text())
    bins = {"base": a.base, "new": a.new}
    checker_sets = [c for c in a.checkers.split(",") if c] or [""]
    summary = ["# A/B", "", f"base {a.base}, new {a.new}, {a.reps} interleaved runs per cell", ""]
    all_runs = {}
    for name in a.projects.split(","):
        # Identity (no --extendedDiagnostics: its counters vary with stealing).
        for flags in ([], ["--listFiles"], ["--explainFiles"]):
            outs = {}
            for k, b in bins.items():
                cwd, args = cmd(cfg, name, b, work, flags)
                r = subprocess.run(args, cwd=cwd, capture_output=True)
                outs[k] = (r.returncode, r.stdout)
                (out / f"{name}-{k}{''.join(flags)}.out").write_bytes(r.stdout)
            same = outs["base"] == outs["new"]
            summary.append(f"- {name} {' '.join(flags) or 'diagnostics'}: {'identical' if same else 'DIFFERENT'} "
                           f"(exit {outs['base'][0]}/{outs['new'][0]}, {len(outs['base'][1])} bytes)")
        summary.append("")
        for ch in checker_sets:
            extra = ["--extendedDiagnostics"] + (["--checkers", ch] if ch else [])
            cell = f"{name} checkers={ch or 'default'}"
            runs = {"base": [], "new": []}
            for k, b in bins.items():  # warm the page cache
                cwd, args = cmd(cfg, name, b, work, extra)
                timed(cwd, args, out / "warm.txt")
            for rep in range(a.reps):
                order = ["base", "new"] if rep % 2 == 0 else ["new", "base"]
                for k in order:
                    cwd, args = cmd(cfg, name, bins[k], work, extra)
                    runs[k].append(timed(cwd, args, out / f"{name}-{ch or 'd'}-{k}-{rep}.txt"))
            all_runs[cell] = runs
            keys = [k for k in runs["base"][0] if k in runs["new"][0]]
            if a.rows:
                first = [k for k in keys if re.search(a.rows, k)]
                keys = first + [k for k in keys if k not in first]
            summary += [f"## {cell}", "", "| row | base median | new median | delta | base min-max | new min-max |",
                        "| --- | ---: | ---: | ---: | --- | --- |"]
            for k in keys:
                bv = [r[k] for r in runs["base"] if k in r]
                nv = [r[k] for r in runs["new"] if k in r]
                if len(bv) != len(nv) or not bv:
                    continue
                bm, nm = statistics.median(bv), statistics.median(nv)
                if bm == 0 and nm == 0:
                    continue
                scale, unit = (1, "") if ("MiB" in k) else (1000, " ms")
                summary.append(f"| {k} | {bm*scale:.1f}{unit} | {nm*scale:.1f}{unit} | {(nm-bm)*scale:+.1f} | "
                               f"{min(bv)*scale:.1f}-{max(bv)*scale:.1f} | {min(nv)*scale:.1f}-{max(nv)*scale:.1f} |")
            summary.append("")
        # perf stat and single-threaded instruction counts.
        for k, b in bins.items():
            cwd, args = cmd(cfg, name, b, work, [])
            perf = os.environ.get("PERF") or shutil.which("perf")
            if perf:
                r = subprocess.run([perf, "stat", "-x,", "-e", "cycles:u,instructions:u,instructions:k,page-faults,task-clock"] + args,
                                   cwd=cwd, capture_output=True, text=True)
                stat = [l.split(",")[0] + " " + l.split(",")[2] for l in r.stderr.splitlines() if l.count(",") > 3]
                summary.append(f"- perf stat {name} {k}: {'; '.join(stat)}")
            j = out / f"count-{name}-{k}.json"
            subprocess.run([sys.executable, str(ROOT / "bench" / "count.py"), str(j), "--"] + args + ["--singleThreaded"],
                           cwd=cwd, capture_output=True)
            if j.exists():
                c = json.loads(j.read_text())
                summary.append(f"- count.py single-threaded {name} {k}: {(c.get('instructions') or 0)/1e9:.4f} G instructions, "
                               f"max RSS {(c.get('max_rss_bytes') or 0)/2**20:.0f} MiB")
        summary.append("")
    (out / "runs.json").write_text(json.dumps(all_runs, indent=1))
    (out / "summary.md").write_text("\n".join(summary) + "\n")
    print("\n".join(summary))


if __name__ == "__main__":
    main()
