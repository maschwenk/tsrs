#!/usr/bin/env python3
"""Checker-count scaling probe: wall time, check time and peak RSS of one tsrs binary at several checker counts on
several projects, interleaved (rep -> project -> checker count), medians in summary.md and every run in runs.tsv.

    tools/perf/scaleprobe.py --tsrs target/release/tsrs --work bench/.work --out probe-out \
        --projects t3code-server,formbricks-web --checkers 8,12,16,24,32 --reps 5 [--env TSRS_FOO=1 ...]

`--env` sets a variable for every run (repeatable). With `--times`, every run also prints the per-checker CPU
seconds (`TSRS_ASSIGNMENT_STATS=times`) into its log.
"""

import argparse
import json
import os
import statistics
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "bench"))
import run as bench  # noqa: E402

PHASES = ["Parse time", "Check time", "Total time"]


def project_cmd(cfg: dict, name: str, tsrs: str, work: Path, checkers: int) -> tuple[Path, list[str]]:
    p = next(p for p in cfg["projects"] if p["name"] == name)
    cwd, proj = bench.project_path(cfg, p, work)
    args = [tsrs, "-p", str(proj), "--noEmit", "--incremental", "false", "--pretty", "false", "--extendedDiagnostics"]
    args += ["--singleThreaded"] if checkers == 1 else ["--checkers", str(checkers)]
    return cwd, args


def run_once(cwd: Path, args: list[str], env: dict, log: Path) -> dict:
    with open(log, "w") as out:
        start = time.perf_counter()
        proc = subprocess.Popen(args, cwd=cwd, stdout=out, stderr=subprocess.STDOUT, env=env)
        _, status, ru = os.wait4(proc.pid, 0)
        wall = time.perf_counter() - start
    text = log.read_text(errors="replace")
    phases = {}
    for line in text.splitlines():
        for ph in PHASES:
            if line.startswith(ph + ":"):
                phases[ph] = float(line.split(":")[1].strip().rstrip("s"))
    errors = sum(1 for line in text.splitlines() if ": error TS" in line)
    # Linux reports ru_maxrss in KiB, macOS in bytes.
    maxrss = ru.ru_maxrss if sys.platform != "darwin" else ru.ru_maxrss // 1024
    return {"wall": wall, "user": ru.ru_utime, "sys": ru.ru_stime, "maxrss_kib": maxrss, "status": status,
            "errors": errors, **{ph: phases.get(ph, float("nan")) for ph in PHASES}}


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--tsrs", required=True)
    ap.add_argument("--work", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--projects", required=True)
    ap.add_argument("--checkers", default="8,16,32")
    ap.add_argument("--reps", type=int, default=5)
    ap.add_argument("--env", action="append", default=[])
    ap.add_argument("--times", action="store_true")
    a = ap.parse_args()

    cfg = json.loads((ROOT / "bench" / "projects.json").read_text())
    work = Path(a.work)
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    projects = a.projects.split(",")
    counts = [int(c) for c in a.checkers.split(",")]
    env = dict(os.environ)
    for kv in a.env:
        k, v = kv.split("=", 1)
        env[k] = v
    if a.times:
        env["TSRS_ASSIGNMENT_STATS"] = "times"

    # Warm the page cache once per project.
    for name in projects:
        cwd, args = project_cmd(cfg, name, a.tsrs, work, counts[0])
        subprocess.run(args, cwd=cwd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=env)

    runs: dict[tuple[str, int], list[dict]] = {}
    tsv = out / "runs.tsv"
    with open(tsv, "w") as f:
        f.write("rep\tproject\tcheckers\twall\tuser\tsys\tmaxrss_kib\tstatus\terrors\tparse\tcheck\ttotal\n")
    for rep in range(1, a.reps + 1):
        for name in projects:
            for k in counts:
                cwd, args = project_cmd(cfg, name, a.tsrs, work, k)
                log = out / f"{name}-c{k}-rep{rep}.txt"
                r = run_once(cwd, args, env, log)
                runs.setdefault((name, k), []).append(r)
                with open(tsv, "a") as f:
                    f.write(f"{rep}\t{name}\t{k}\t{r['wall']:.3f}\t{r['user']:.2f}\t{r['sys']:.2f}\t{r['maxrss_kib']}\t"
                            f"{r['status']}\t{r['errors']}\t{r['Parse time']:.3f}\t{r['Check time']:.3f}\t{r['Total time']:.3f}\n")
                print(f"rep {rep} {name} --checkers {k}: wall {r['wall']:.3f} s, check {r['Check time']:.3f} s, "
                      f"peak {r['maxrss_kib'] / 1024:.0f} MiB, {r['errors']} errors", flush=True)

    lines = ["# Checker-count scaling", "", f"tsrs `{a.tsrs}`, {a.reps} reps per cell, interleaved. Medians (min-max for wall).", ""]
    lines.append("| project | checkers | wall s | check s | parse s | peak MiB | user s | sys s | errors |")
    lines.append("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |")
    for name in projects:
        for k in counts:
            rs = runs[(name, k)]
            med = lambda key: statistics.median(r[key] for r in rs)  # noqa: E731
            walls = [r["wall"] for r in rs]
            errs = sorted({r["errors"] for r in rs})
            lines.append(f"| {name} | {k} | {med('wall'):.3f} ({min(walls):.3f}-{max(walls):.3f}) | {med('Check time'):.3f} | "
                         f"{med('Parse time'):.3f} | {med('maxrss_kib') / 1024:,.0f} | {med('user'):.2f} | {med('sys'):.2f} | "
                         f"{'/'.join(map(str, errs))} |")
    (out / "summary.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))


if __name__ == "__main__":
    main()
