#!/usr/bin/env python3
"""Probe for the residency slack of a many-checker run (notes/mem-linux-residency-32.md).

Two parts, both on the projects and checker counts given:

- `--split`: one run per cell and variant with `TSRS_MEM_SPLIT=1` (tsrs prints its own split at `parse end`,
  `check end` and `exit`) while a sampler reads `/proc/<pid>/status` and `/proc/<pid>/smaps_rollup` every
  `--sample-ms`; the sample with the largest Rss is kept with its AnonHugePages, thread count and time.
- timed reps: `--reps` interleaved runs per variant (wall, user/sys, peak RSS, phases), no sampling but `status`.

A variant is `name:K=V;K=V`; `BIN=<path>` in it runs another binary. Results go to $PROBE_OUT (summary.md).
"""

import argparse
import json
import os
import re
import statistics
import subprocess
import sys
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "bench"))
import run as bench  # noqa: E402

PHASES = ["Parse time", "Bind time", "Check time", "Total time"]


def project_cmd(cfg: dict, name: str, tsrs: str, work: Path, checkers: int) -> tuple[Path, list[str]]:
    p = next(p for p in cfg["projects"] if p["name"] == name)
    cwd, proj = bench.project_path(cfg, p, work)
    args = [tsrs, "-p", str(proj), "--noEmit", "--incremental", "false", "--pretty", "false", "--extendedDiagnostics"]
    args += ["--singleThreaded"] if checkers == 1 else ["--checkers", str(checkers)]
    return cwd, args


def kib_field(text: str, name: str) -> int:
    m = re.search(rf"^{name}:\s+(\d+) kB", text, re.M)
    return int(m.group(1)) if m else 0


def sampler(pid: int, rollup: bool, period: float, out: list, stop: threading.Event) -> None:
    t0 = time.perf_counter()
    while not stop.is_set():
        try:
            st = Path(f"/proc/{pid}/status").read_text()
            s = {"t": time.perf_counter() - t0, "rss": kib_field(st, "VmRSS"), "hwm": kib_field(st, "VmHWM"),
                 "threads": int(re.search(r"^Threads:\s+(\d+)", st, re.M).group(1))}
            if rollup:
                ro = Path(f"/proc/{pid}/smaps_rollup").read_text()
                s["huge"] = kib_field(ro, "AnonHugePages")
                s["anon"] = kib_field(ro, "Anonymous")
            out.append(s)
        except (OSError, AttributeError):
            break
        time.sleep(period)


def run_one(cwd: Path, args: list[str], env: dict, out: Path, rollup: bool, period: float) -> dict:
    samples: list = []
    stop = threading.Event()
    with open(out, "wb") as f, open(out.with_suffix(".err"), "wb") as ferr:
        t0 = time.perf_counter()
        proc = subprocess.Popen(args, cwd=cwd, env=env, stdout=f, stderr=ferr)
        th = threading.Thread(target=sampler, args=(proc.pid, rollup, period, samples, stop), daemon=True)
        th.start()
        _, status, ru = os.wait4(proc.pid, 0)
        wall = time.perf_counter() - t0
        stop.set()
        th.join()
    text = out.read_text(errors="replace") + out.with_suffix(".err").read_text(errors="replace")
    r = {"wall": wall, "user": ru.ru_utime, "sys": ru.ru_stime, "maxrss_kib": ru.ru_maxrss, "minflt": ru.ru_minflt,
         "exit": os.waitstatus_to_exitcode(status), "errors": len(re.findall(r"error TS\d+", text))}
    for ph in PHASES:
        m = re.search(rf"^{ph}:\s+([\d.]+)s", text, re.M)
        if m:
            r[ph] = float(m.group(1))
    if samples:
        peak = max(samples, key=lambda s: s["rss"])
        r["peak_sample"] = peak
        r["samples"] = len(samples)
        r["max_threads"] = max(s["threads"] for s in samples)
    return r


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--tsrs", required=True)
    ap.add_argument("--work", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--projects", default="t3code-server,formbricks-web,vscode")
    ap.add_argument("--checkers", default="32")
    ap.add_argument("--reps", type=int, default=0)
    ap.add_argument("--rep-checkers", default="", help="checker:reps overrides, e.g. 4:5,16:5,32:10")
    ap.add_argument("--variant", action="append", default=[], help="name:K=V[;K=V...]")
    ap.add_argument("--split", action="store_true")
    ap.add_argument("--sample-ms", type=float, default=10)
    args = ap.parse_args()
    args.tsrs = str(Path(args.tsrs).resolve())
    variants = []
    for v in args.variant or ["base:"]:
        name, _, kvs = v.partition(":")
        variants.append((name, dict(kv.split("=", 1) for kv in kvs.split(";") if kv)))

    def with_bin(cmd: list[str], venv: dict) -> tuple[list[str], dict]:
        venv = dict(venv)
        binary = venv.pop("BIN", None)
        return ([binary] + cmd[1:] if binary else cmd), venv

    cfg = json.loads((ROOT / "bench" / "projects.json").read_text())
    out = args.out
    (out / "runs").mkdir(parents=True, exist_ok=True)
    base_env = {k: v for k, v in os.environ.items() if not k.startswith("TSRS_") and not k.startswith("MIMALLOC_")}
    machine = {"nproc": os.cpu_count(), "uname": " ".join(os.uname()),
               "thp": {f: Path(f"/sys/kernel/mm/transparent_hugepage/{f}").read_text().strip()
                       for f in ["enabled", "defrag"] if Path(f"/sys/kernel/mm/transparent_hugepage/{f}").exists()},
               "version": subprocess.run([args.tsrs, "--version"], capture_output=True, text=True).stdout.strip()}
    for _, venv in variants:
        if "BIN" in venv:
            machine[venv["BIN"]] = subprocess.run([venv["BIN"], "--version"], capture_output=True, text=True).stdout.strip()
    print(json.dumps(machine, indent=1), flush=True)
    projects = args.projects.split(",")
    checkers = [int(c) for c in args.checkers.split(",")]
    reps_for = {int(k): int(v) for k, v in (kv.split(":") for kv in args.rep_checkers.split(",") if kv)}
    lines = [f"# slack probe\n\n`{machine['version']}`, {machine['nproc']} CPUs, THP {machine['thp']}\n"]

    if args.split:
        lines.append("## Split runs (TSRS_MEM_SPLIT=1, smaps_rollup sampled)\n")
        for name in projects:
            for k in checkers:
                cwd, cmd = project_cmd(cfg, name, args.tsrs, args.work, k)
                subprocess.run(cmd, cwd=cwd, env=base_env, capture_output=True)  # warm the page cache
                for vname, venv in variants:
                    vcmd, venv_ = with_bin(cmd, venv)
                    o = out / "runs" / f"split-{name}-c{k}-{vname}.txt"
                    r = run_one(cwd, vcmd, dict(base_env, TSRS_MEM_SPLIT="1", **venv_), o, True, args.sample_ms / 1000)
                    ps = r.get("peak_sample", {})
                    head = (f"### {name}, {k} checkers, {vname}: peak RSS {r['maxrss_kib'] / 1048576:.3f} GiB; sampled peak "
                            f"{ps.get('rss', 0) / 1024:.0f} MiB at {ps.get('t', 0):.3f} s (AnonHugePages {ps.get('huge', 0) / 1024:.0f} MiB, "
                            f"threads {ps.get('threads')}), {r.get('samples')} samples, max threads {r.get('max_threads')}; "
                            f"wall {r['wall']:.3f} s, Parse {r.get('Parse time')} Check {r.get('Check time')} Total {r.get('Total time')}")
                    print(head, flush=True)
                    text = o.with_suffix(".err").read_text(errors="replace")
                    block = [l for l in text.splitlines() if l.startswith("tsrs mem split") or l.startswith("  ")]
                    lines += [head, "```", *block, "```", ""]
                    (out / "runs" / f"split-{name}-c{k}-{vname}.json").write_text(json.dumps(r, indent=1))

    results: list[dict] = []
    if args.reps or reps_for:
        for name in projects:
            for k in checkers:
                reps = reps_for.get(k, args.reps)
                cwd, cmd = project_cmd(cfg, name, args.tsrs, args.work, k)
                subprocess.run(cmd, cwd=cwd, env=base_env, capture_output=True)
                for rep in range(reps):
                    for vname, venv in variants:
                        vcmd, venv_ = with_bin(cmd, venv)
                        o = out / "runs" / f"{name}-c{k}-{vname}-rep{rep}.txt"
                        r = run_one(cwd, vcmd, dict(base_env, **venv_), o, False, 0.05)
                        r.update(project=name, checkers=k, variant=vname, rep=rep)
                        results.append(r)
                        print(f"{name} c{k} {vname:>12} rep{rep}: wall {r['wall']:.3f} user {r['user']:.2f} sys {r['sys']:.2f} "
                              f"rss {r['maxrss_kib'] / 1048576:.3f} GiB check {r.get('Check time')} parse {r.get('Parse time')} "
                              f"errors {r['errors']} exit {r['exit']}", flush=True)
        (out / "results.json").write_text(json.dumps(results, indent=1))

        def med(xs):
            return statistics.median(xs) if xs else float("nan")

        lines.append("## Timed reps (medians; paired = median of per-rep ratios to the first variant)\n")
        lines.append("| project | checkers | variant | reps | wall s | paired | wall min-max | check s | parse s | user+sys s | sys s | peak GiB | vs first | peak min-max | errors |")
        lines.append("| --- | --- | --- | ---: | ---: | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | --- | --- |")
        for name in projects:
            for k in checkers:
                cell = [r for r in results if r["project"] == name and r["checkers"] == k]
                base = {r["rep"]: r for r in cell if r["variant"] == variants[0][0]}
                first = None
                for vname, _ in variants:
                    rs = [r for r in cell if r["variant"] == vname]
                    if not rs:
                        continue
                    w, rss = med([r["wall"] for r in rs]), med([r["maxrss_kib"] for r in rs])
                    paired = med([r["wall"] / base[r["rep"]]["wall"] for r in rs if r["rep"] in base])
                    first = first or (w, rss)
                    lines.append(
                        f"| {name} | {k} | {vname} | {len(rs)} | {w:.3f} | {100 * (paired - 1):+.1f}% | {min(r['wall'] for r in rs):.3f}-{max(r['wall'] for r in rs):.3f} | "
                        f"{med([r.get('Check time', 0) for r in rs]):.3f} | {med([r.get('Parse time', 0) for r in rs]):.3f} | "
                        f"{med([r['user'] + r['sys'] for r in rs]):.2f} | {med([r['sys'] for r in rs]):.2f} | {rss / 1048576:.3f} | "
                        f"{100 * (rss / first[1] - 1):+.1f}% | {min(r['maxrss_kib'] for r in rs) / 1048576:.3f}-{max(r['maxrss_kib'] for r in rs) / 1048576:.3f} | "
                        f"{sorted({r['errors'] for r in rs})} |")
    (out / "summary.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines), flush=True)


if __name__ == "__main__":
    main()
