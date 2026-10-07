#!/usr/bin/env python3
"""Interleaved A/B/... measurement of several tsrs builds of the same source (.depot/workflows/perf-layout-probe.yml,
notes/perf-binary-layout.md).

    tools/perf/layout-measure.py --bin A=/tmp/l/bin/A/tsrs --bin R=/tmp/l/bin/R/tsrs --out probe-out \
        [--projects all] [--checkers 32,4] [--reps 7] [--perf /usr/bin/perf]

1. Timed runs: `-p <project> --noEmit --incremental false --pretty false --extendedDiagnostics [--checkers N]`
   (no --checkers for the default count), `--reps` rounds; each round runs every project and checker count with every
   binary, in an order that rotates per round. Wall time and peak RSS from wait4, check time from the output.
2. Output identity: every binary's full stdout without --extendedDiagnostics at 1, 4 and 32 checkers, compared byte
   for byte (with the exit code) against the first binary's.
3. Counters: one `--singleThreaded` run per project and binary under `perf stat` (instructions, cycles, iTLB and L1i
   misses), if --perf is given.

Writes <out>/layout.json (every run) and <out>/layout.md (medians, ratios to the first binary). Exits 1 if any
output differs.
"""

from __future__ import annotations

import argparse
import json
import os
import statistics
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent.parent / "bench"))
import run as rb  # noqa: E402

FLAGS = ["--noEmit", "--incremental", "false", "--pretty", "false"]
EVENTS = "instructions,cycles,iTLB-load-misses,L1-icache-load-misses"


def timed(exe: str, cwd: Path, proj: Path, extra: list[str], log: Path) -> dict:
    argv = [exe, "-p", str(proj), *FLAGS, "--extendedDiagnostics", *extra]
    with open(log, "wb") as f:
        t0 = time.perf_counter()
        p = subprocess.Popen(argv, cwd=cwd, stdout=f, stderr=subprocess.STDOUT)
        _, status, ru = os.wait4(p.pid, 0)
        wall = time.perf_counter() - t0
    p.returncode = os.waitstatus_to_exitcode(status)
    r = {"exit": p.returncode, "wall_s": wall, "user_s": ru.ru_utime, "sys_s": ru.ru_stime, "rss_b": ru.ru_maxrss * 1024}
    for line in log.read_text(errors="replace").splitlines():
        if m := rb.TIME_RE.match(line):
            r[f"{m.group(1).lower()}_s"] = float(m.group(2))
    if p.returncode not in (0, 1, 2) or "check_s" not in r:
        sys.exit(f"{exe} on {proj} {extra}: exit {p.returncode}, see {log}")
    return r


def med(xs):
    xs = [x for x in xs if x is not None]
    return statistics.median(xs) if xs else None


def pct(x, base):
    return f"{(x / base - 1) * 100:+.1f}%" if x is not None and base else "-"


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--bin", action="append", required=True, help="NAME=PATH; the first is the baseline")
    ap.add_argument("--projects", default="all")
    ap.add_argument("--checkers", default="default,4", help="comma-separated; `default` = no --checkers flag")
    ap.add_argument("--identity-checkers", default="1,4,32")
    ap.add_argument("--reps", type=int, default=7)
    ap.add_argument("--perf", help="perf executable for the counter runs")
    ap.add_argument("--work-dir", type=Path, default=rb.BENCH / ".work")
    ap.add_argument("--out", type=Path, default=Path("probe-out"))
    args = ap.parse_args()

    bins = dict(b.split("=", 1) for b in args.bin)
    names = list(bins)
    cfg = json.loads((rb.BENCH / "projects.json").read_text())
    projects = [p for p in cfg["projects"] if args.projects == "all" or p["name"] in args.projects.split(",")]
    paths = {p["name"]: rb.project_path(cfg, p, args.work_dir.resolve()) for p in projects}
    checkers = args.checkers.split(",")
    logs = args.out / "logs"
    logs.mkdir(parents=True, exist_ok=True)
    result: dict = {"bins": bins, "runs": [], "identity": [], "perf": [], "machine": rb.machine_info(False, None)}

    def extra(c: str) -> list[str]:
        return [] if c == "default" else ["--checkers", c]

    # Warm the page cache (sources, node_modules, the binaries).
    for name in names:
        for p, (cwd, proj) in paths.items():
            timed(bins[name], cwd, proj, extra(checkers[0]), logs / "warm.txt")

    for rep in range(args.reps):
        for p, (cwd, proj) in paths.items():
            for c in checkers:
                order = names[rep % len(names):] + names[:rep % len(names)]
                for name in order:
                    r = timed(bins[name], cwd, proj, extra(c), logs / f"{name}-{p}-{c}.txt")
                    result["runs"].append({"bin": name, "project": p, "checkers": c, "rep": rep, **r})
        print(f"round {rep + 1}/{args.reps} done", flush=True)

    same = True
    for p, (cwd, proj) in paths.items():
        for c in args.identity_checkers.split(","):
            outs = {}
            for name in names:
                f = logs / f"out-{name}-{p}-{c}.txt"
                with open(f, "wb") as fh:
                    rc = subprocess.run([bins[name], "-p", str(proj), *FLAGS, "--checkers", c], cwd=cwd, stdout=fh,
                                        stderr=subprocess.STDOUT).returncode
                    fh.write(f"\nexit {rc}\n".encode())
                outs[name] = f.read_bytes()
            for name in names[1:]:
                ok = outs[name] == outs[names[0]]
                same &= ok
                result["identity"].append({"project": p, "checkers": c, "bin": name, "identical": ok,
                                           "bytes": len(outs[name])})

    if args.perf:
        for p, (cwd, proj) in paths.items():
            for name in names:
                f = logs / f"perf-{name}-{p}.csv"
                subprocess.run([args.perf, "stat", "-x", ",", "-o", str(f), "-e", EVENTS, "--", bins[name], "-p",
                                str(proj), *FLAGS, "--singleThreaded"], cwd=cwd, stdout=subprocess.DEVNULL,
                               stderr=subprocess.DEVNULL)
                counts = {}
                for line in f.read_text().splitlines():
                    parts = line.split(",")
                    if len(parts) > 2 and parts[0].replace(".", "").isdigit():
                        counts[parts[2].split(":")[0]] = float(parts[0])
                result["perf"].append({"bin": name, "project": p, **counts})

    (args.out / "layout.json").write_text(json.dumps(result, indent=1))
    (args.out / "layout.md").write_text(render(result, names, list(paths), checkers))
    print((args.out / "layout.md").read_text())
    sys.exit(0 if same else 1)


def render(result: dict, names: list[str], projects: list[str], checkers: list[str]) -> str:
    base = names[0]
    lines = [f"Binaries: {', '.join(f'{n} = {p}' for n, p in result['bins'].items())}", "",
             "Medians of the timed runs; spread = half the min-max range of the baseline's runs, % of its median; "
             f"Δ = median vs {base}.", ""]
    for c in checkers:
        lines += [f"### checkers: {c}", "",
                  "| project | " + " | ".join(f"{n} wall s" for n in names) + f" | {base} spread | "
                  + " | ".join(f"{n} Δwall" for n in names[1:]) + " | "
                  + " | ".join(f"{n} Δcheck" for n in names[1:]) + " | " + " | ".join(f"{n} Δrss" for n in names[1:])
                  + " |", "|---" * (1 + len(names) + 1 + 3 * (len(names) - 1)) + "|"]
        for p in projects:
            cell = {n: [r for r in result["runs"] if r["bin"] == n and r["project"] == p and r["checkers"] == c]
                    for n in names}
            m = {n: {k: med([r.get(k) for r in cell[n]]) for k in ("wall_s", "check_s", "rss_b")} for n in names}
            walls = [r["wall_s"] for r in cell[base]]
            spread = (max(walls) - min(walls)) / 2 / m[base]["wall_s"] * 100 if walls else 0
            lines.append(f"| {p} | " + " | ".join(f"{m[n]['wall_s']:.3f}" for n in names) + f" | ±{spread:.1f}% | "
                         + " | ".join(pct(m[n]["wall_s"], m[base]["wall_s"]) for n in names[1:]) + " | "
                         + " | ".join(pct(m[n]["check_s"], m[base]["check_s"]) for n in names[1:]) + " | "
                         + " | ".join(pct(m[n]["rss_b"], m[base]["rss_b"]) for n in names[1:]) + " |")
        lines.append("")
    if result["perf"]:
        events = EVENTS.split(",")
        lines += ["### perf stat, one `--singleThreaded` run (G = 1e9, M = 1e6)", "",
                  "| project | bin | " + " | ".join(events) + " |", "|---" * (2 + len(events)) + "|"]
        for p in projects:
            b = next((r for r in result["perf"] if r["bin"] == base and r["project"] == p), {})
            for n in names:
                r = next((r for r in result["perf"] if r["bin"] == n and r["project"] == p), {})
                vals = []
                for e in events:
                    v = r.get(e)
                    unit, div = ("G", 1e9) if e in ("instructions", "cycles") else ("M", 1e6)
                    vals.append("-" if v is None else f"{v / div:.2f}{unit}" + ("" if n == base else f" ({pct(v, b.get(e))})"))
                lines.append(f"| {p} | {n} | " + " | ".join(vals) + " |")
        lines.append("")
    diff = [i for i in result["identity"] if not i["identical"]]
    lines += [f"Output identity ({len(result['identity'])} comparisons against {base}, checkers "
              f"{sorted({i['checkers'] for i in result['identity']})}): "
              + ("all byte-identical" if not diff else f"DIFFERENT: {diff}"), ""]
    return "\n".join(lines)


if __name__ == "__main__":
    main()
