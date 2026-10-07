#!/usr/bin/env python3
"""Probe for the checker assignment (notes/perf-clustered-assignment.md): interleaved runs of several tsrs binaries,
each under its own environment, on the bench projects at several checker counts.

Per run: wall (`os.wait4`), user + system CPU, peak RSS, the `--extendedDiagnostics` phase times (check, total,
checker creation, file assignment) and a hash of the diagnostics (every `error TS` line with its continuation lines),
which must be the same for every variant. After the timed runs, `--stats-reps` runs per cell with
`TSRS_ASSIGNMENT_STATS=times` (which turns leaf freeing off, so they are not timed) give the CPU summed over the
checkers. summary.md has the tables (medians), results.json every run.

    tools/perf/clusterprobe.py --work bench/.work --out probe-out --projects vscode,cal-diy --checkers 16,32 \\
        --reps 10 --bin base=/tmp/base/tsrs --bin new=/tmp/new/tsrs \\
        --variant main:base --variant cluster:new --variant off:new:TSRS_MODULE_AFFINITY=off
"""

import argparse
import hashlib
import json
import os
import re
import statistics
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "bench"))
import run as bench  # noqa: E402

PHASES = ["Check time", "Total time", "Checkers: create", "Checkers: assign files"]


def diagnostics_hash(text: str) -> str:
    lines, keep = [], False
    for line in text.splitlines():
        if re.search(r"error TS\d+", line):
            keep = True
        elif not line.startswith(" "):
            keep = False
        if keep:
            lines.append(line)
    return hashlib.sha256("\n".join(lines).encode()).hexdigest()[:16]


def timed(cwd: Path, args: list[str], env: dict, out: Path) -> dict:
    with open(out, "wb") as f:
        t0 = time.perf_counter()
        proc = subprocess.Popen(args, cwd=cwd, env=env, stdout=f, stderr=subprocess.STDOUT)
        _, status, ru = os.wait4(proc.pid, 0)
        wall = time.perf_counter() - t0
    text = out.read_text(errors="replace")
    r = {"wall": wall, "cpu": ru.ru_utime + ru.ru_stime, "maxrss_kib": ru.ru_maxrss, "exit": os.waitstatus_to_exitcode(status),
         "diagnostics": diagnostics_hash(text)}
    for ph in PHASES:
        m = re.search(rf"^{re.escape(ph)}:\s+([\d.]+)s", text, re.M)
        if m:
            r[ph] = float(m.group(1))
    m = re.search(r"^checker group cpu seconds: (.*)$", text, re.M)
    if m:
        r["checker_cpu"] = sum(float(x) for x in m.group(1).split())
    return r


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--work", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--projects", default="vscode")
    ap.add_argument("--checkers", default="16,32")
    ap.add_argument("--reps", type=int, default=10)
    ap.add_argument("--stats-reps", type=int, default=2)
    ap.add_argument("--bin", action="append", default=[], help="key=path")
    ap.add_argument("--variant", action="append", default=[], help="name:binkey[:K=V;K=V]")
    args = ap.parse_args()
    bins = dict(b.split("=", 1) for b in args.bin)
    variants = []
    for v in args.variant:
        name, key, *rest = v.split(":", 2)
        env = dict(kv.split("=", 1) for kv in (rest[0] if rest else "").split(";") if kv)
        variants.append((name, bins[key], env))
    cfg = json.loads((ROOT / "bench" / "projects.json").read_text())
    out = args.out
    (out / "runs").mkdir(parents=True, exist_ok=True)
    base_env = {k: v for k, v in os.environ.items() if not k.startswith("TSRS_")}
    results: list[dict] = []
    for name in args.projects.split(","):
        p = next(p for p in cfg["projects"] if p["name"] == name)
        cwd, proj = bench.project_path(cfg, p, args.work)
        for k in [int(c) for c in args.checkers.split(",")]:
            flags = ["-p", str(proj), "--noEmit", "--incremental", "false", "--pretty", "false", "--extendedDiagnostics", "--checkers", str(k)]
            subprocess.run([variants[0][1], *flags], cwd=cwd, env=base_env, capture_output=True)  # warm the page cache
            for rep in range(args.reps + args.stats_reps):
                stats = rep >= args.reps
                for vname, binary, venv in variants:
                    env = dict(base_env, **venv, **({"TSRS_ASSIGNMENT_STATS": "times"} if stats else {}))
                    o = out / "runs" / f"{name}-c{k}-{vname}-{'stats' if stats else 'rep'}{rep}.txt"
                    r = timed(cwd, [binary, *flags], env, o)
                    r.update(project=name, checkers=k, variant=vname, rep=rep, stats=stats)
                    results.append(r)
                    print(f"{name} c{k} {vname:>8} {'stats' if stats else 'rep'}{rep}: wall {r['wall']:.3f} cpu {r['cpu']:.2f}"
                          f" rss {r['maxrss_kib'] / 1048576:.3f} GiB check {r.get('Check time')} assign {r.get('Checkers: assign files')}"
                          f" checker_cpu {r.get('checker_cpu')} diag {r['diagnostics']} exit {r['exit']}", flush=True)
    (out / "results.json").write_text(json.dumps(results, indent=1))

    def med(rs, key):
        xs = [r[key] for r in rs if key in r]
        return statistics.median(xs) if xs else float("nan")

    first = variants[0][0]
    lines = [f"Medians of {args.reps} interleaved runs per cell (checker CPU: {args.stats_reps} runs with TSRS_ASSIGNMENT_STATS=times);"
             f" deltas against `{first}`.", "",
             "| project | checkers | variant | wall s | vs first | check s | vs first | CPU s | vs first | checker CPU s | peak MiB | vs first | assign ms | create ms | diagnostics |",
             "| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |"]
    identical = True
    for name in args.projects.split(","):
        for k in [int(c) for c in args.checkers.split(",")]:
            cell = [r for r in results if r["project"] == name and r["checkers"] == k]
            hashes = {(r["diagnostics"], r["exit"]) for r in cell}
            identical &= len(hashes) == 1
            ref = None
            for vname, _, _ in variants:
                timed_rs = [r for r in cell if r["variant"] == vname and not r["stats"]]
                stats_rs = [r for r in cell if r["variant"] == vname and r["stats"]]
                row = {"wall": med(timed_rs, "wall"), "check": med(timed_rs, "Check time"), "cpu": med(timed_rs, "cpu"),
                       "peak": med(timed_rs, "maxrss_kib") / 1024}
                ref = ref or row
                d = lambda key: f"{(row[key] / ref[key] - 1) * 100:+.1f}%" if ref[key] else ""  # noqa: E731
                lines.append(f"| {name} | {k} | {vname} | {row['wall']:.3f} | {d('wall')} | {row['check']:.3f} | {d('check')} |"
                             f" {row['cpu']:.2f} | {d('cpu')} | {med(stats_rs, 'checker_cpu'):.2f} | {row['peak']:.0f} | {d('peak')} |"
                             f" {med(timed_rs, 'Checkers: assign files') * 1000:.0f} | {med(timed_rs, 'Checkers: create') * 1000:.0f} |"
                             f" {'same' if len(hashes) == 1 else 'DIFFERENT'} |")
    lines += ["", f"Diagnostics and exit codes identical in every run of every cell: {'yes' if identical else 'NO'}"]
    (out / "summary.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))
    sys.exit(0 if identical else 1)


if __name__ == "__main__":
    main()
