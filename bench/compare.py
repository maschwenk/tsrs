#!/usr/bin/env python3
"""Head-to-head type-check benchmark on one machine: tsgo 7.0.2, the TypeScript 7.1-dev nightly tsrs ports, tsrs and
`bun check`, at the same thread counts.

bench/run.py is the fixed-spec regression benchmark (8 vCPU, tsgo vs tsrs). This script answers a different question:
how do the four compilers compare on a wide machine, the way Bun's `bun check` announcement measured them (vscode,
Linux x64, 64 threads, mean of 20 runs). See bench/README.md "Head-to-head on a wide machine". Typical use:

    python3 bench/compare.py --tsrs target/release/tsrs --bun ~/.bun/bin/bun --projects vscode --reps 20
    python3 bench/compare.py --tsrs ... --bun ... --projects Compiler --threads default,4 --reps 2   # smoke test
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import re
import statistics
import subprocess
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import run as rb  # noqa: E402  (bench/run.py: project setup, compiler install, output parsing)

COMPILERS = ("tsgo", "tsgo-dev", "tsrs", "bun")
LABELS = {"tsgo": "tsgo 7.0.2", "tsgo-dev": "tsgo 7.1-dev", "tsrs": "tsrs", "bun": "bun check"}
BUN_FILES_RE = re.compile(r"checked ([\d,]+) files?")


def thread_flags(compiler: str, threads: str) -> list[str]:
    """`default` passes nothing (tsgo: 4 checkers; tsrs: half the cores, clamped to 4..32 checkers; bun: one thread
    per core). A number gives tsgo/tsrs that many checker threads (`--checkers N`; parsing still uses every core) and
    bun that many threads (`--threads N`, its only knob)."""
    if threads == "default":
        return []
    return ["--threads", threads] if compiler == "bun" else ["--checkers", threads]


def argv_for(compiler: str, exe: Path, proj: Path, threads: str) -> list[str]:
    if compiler == "bun":
        # --all: no grouping of repeated errors (bun groups above 50), so every error line is counted.
        return [str(exe), "check", "-p", str(proj), "--no-pretty", "--all", *thread_flags(compiler, threads)]
    return [str(exe), "-p", str(proj), "--noEmit", "--incremental", "false", "--extendedDiagnostics", "--pretty",
            "false", *thread_flags(compiler, threads)]


def normalize_key(key: str, cwd: Path, proj_dir: Path) -> str:
    """'path(l,c): error TSn' with the path relative to cwd, whichever directory the compiler printed it against."""
    m = re.match(r"^(.*)\((\d+,\d+)\): (error TS\d+)$", key)
    if not m:
        return key
    path = Path(m.group(1))
    for base in (cwd, proj_dir):
        cand = path if path.is_absolute() else base / path
        if cand.exists():
            try:
                return f"{os.path.relpath(cand.resolve(), cwd.resolve())}({m.group(2)}): {m.group(3)}"
            except ValueError:
                break
    return key


def run_once(compiler: str, exe: Path, cwd: Path, proj: Path, threads: str, log_path: Path, timeout: float) -> dict:
    argv = argv_for(compiler, exe, proj, threads)
    log_path.parent.mkdir(parents=True, exist_ok=True)
    with open(log_path, "wb") as out:
        t0 = time.perf_counter()
        p = subprocess.Popen(argv, cwd=cwd, stdout=out, stderr=subprocess.STDOUT)
        timer = threading.Timer(timeout, p.kill)
        timer.start()
        _, status, ru = os.wait4(p.pid, 0)
        wall = time.perf_counter() - t0
        timer.cancel()
    p.returncode = os.waitstatus_to_exitcode(status)
    peak = ru.ru_maxrss * (1 if sys.platform == "darwin" else 1024)
    r: dict = {"exit": p.returncode, "wall_s": round(wall, 3), "peak_rss_bytes": peak}
    keys = []
    proj_dir = proj if proj.is_dir() else proj.parent
    for line in log_path.read_text(errors="replace").splitlines():
        if m := rb.ERROR_RE.match(line):
            keys.append(normalize_key(m.group(1), cwd, proj_dir))
        elif m := rb.COUNTER_RE.match(line):
            r[m.group(1).lower()] = int(m.group(2))
        elif m := rb.TIME_RE.match(line):
            r[f"{m.group(1).lower()}_s"] = float(m.group(2))
        elif compiler == "bun" and (m := BUN_FILES_RE.search(line)):
            r["files"] = int(m.group(1).replace(",", ""))
    r["errors"] = len(keys)
    r["error_keys"] = sorted(keys)
    # tsgo/tsrs print --extendedDiagnostics after a completed check; bun prints its "checked N files" summary.
    finished = "files" in r if compiler == "bun" else "check_s" in r
    r["ok"] = p.returncode in (0, 1, 2) and finished
    return r


def stats(xs: list[float]) -> dict:
    if not xs:
        return {}
    return {"mean": statistics.fmean(xs), "stdev": statistics.stdev(xs) if len(xs) > 1 else 0.0,
            "median": statistics.median(xs), "min": min(xs), "max": max(xs)}


def summarize(runs: list[dict], ref_keys: tuple | None) -> dict:
    ok = [r for r in runs if r["ok"]]
    keysets = {tuple(r["error_keys"]) for r in ok}
    s = {"runs": len(runs), "ok_runs": len(ok), "wall_s": stats([r["wall_s"] for r in ok]),
         "peak_rss_bytes": stats([r["peak_rss_bytes"] for r in ok]),
         "error_counts": sorted({r["errors"] for r in ok}), "files": sorted({r["files"] for r in ok if "files" in r})}
    if ref_keys is not None and keysets:
        s["errors_equal_ref"] = keysets == {ref_keys}
        if not s["errors_equal_ref"]:
            mine = set().union(*keysets)
            s["only_here"] = sorted(mine - set(ref_keys))[:20]
            s["missing"] = sorted(set(ref_keys) - mine)[:20]
    return s


def gib(b: float | None) -> str:
    return "n/a" if b is None else f"{b / 2**30:.2f} GiB"


def markdown(result: dict) -> str:
    m = result["machine"]
    v = result["versions"]
    lines = [
        f"## Head-to-head on {m['cpus']} threads: tsgo, tsgo 7.1-dev, tsrs, bun check",
        "",
        f"Machine: {m['label']}. Date: {result['date']}. Mean of {result['reps']} interleaved runs per cell (the "
        f"compiler order rotates every rep), after one untimed warm-up per compiler and project.",
        "",
        "Compilers: " + "; ".join(f"{LABELS[c]} = `{v[c]}`" for c in result["compilers"]) + ".",
        "",
        "Threads: `default` passes no flag (tsgo then uses 4 checker threads, tsrs half the cores clamped to 4..32, bun "
        "one thread per core). N passes `--checkers N` to tsgo/tsrs (checker threads; parsing still uses every core) "
        "and `--threads N` to bun (all of its threads, so bun at small N is capped harder than the others).",
        "",
    ]
    base = "tsgo"
    for name, pr in result["projects"].items():
        lines += [f"### {name}", "",
                  "| threads | compiler | wall mean ± sd (s) | median | min | peak RSS (mean) | vs tsgo 7.0.2 default | "
                  "vs tsgo 7.0.2 same threads | errors | = 7.1-dev errors |",
                  "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |"]
        base_default = pr.get("default", {}).get(base, {}).get("wall_s", {}).get("mean")
        for threads in result["threads"]:
            cell = pr.get(threads, {})
            same = cell.get(base, {}).get("wall_s", {}).get("mean")
            for c in result["compilers"]:
                s = cell.get(c)
                if not s or not s["ok_runs"]:
                    lines.append(f"| {threads} | {LABELS[c]} | **FAILED** | | | | | | | |")
                    continue
                w, rss = s["wall_s"], s["peak_rss_bytes"]
                vs_d = f"{base_default / w['mean']:.2f}x" if base_default else "n/a"
                vs_s = f"{same / w['mean']:.2f}x" if same else "n/a"
                eq = {True: "yes", False: "**no**", None: "n/a"}[s.get("errors_equal_ref")]
                lines.append(f"| {threads} | {LABELS[c]} | {w['mean']:.2f} ± {w['stdev']:.2f} | {w['median']:.2f} | "
                             f"{w['min']:.2f} | {gib(rss['mean'])} | {vs_d} | {vs_s} | "
                             f"{'/'.join(map(str, s['error_counts']))} | {eq} |")
        files = {c: pr.get("default", {}).get(c, {}).get("files") for c in result["compilers"]}
        lines += ["", "Files: " + ", ".join(f"{LABELS[c]} {'/'.join(map(str, f))}" for c, f in files.items() if f)
                  + " (tsgo/tsrs: files in the program; bun: its own \"checked N files\" count, not comparable).", ""]
    lines += ["vs tsgo 7.0.2: tsgo 7.0.2's mean wall divided by this row's (above 1 = faster). = 7.1-dev errors: the "
              "(file, line, column, code) list equals tsgo 7.1-dev's at default threads, the TypeScript commit tsrs "
              "ports. Peak RSS: `ru_maxrss` from `wait4`. Wall: process wall clock including startup."]
    return "\n".join(lines) + "\n"


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--tsrs", type=Path)
    ap.add_argument("--bun", type=Path, help="bun executable that has `bun check` (1.4.3 canary or later)")
    ap.add_argument("--compilers", default=",".join(COMPILERS), help="comma-separated subset of " + ", ".join(COMPILERS))
    ap.add_argument("--projects", default="vscode", help="comma-separated subset of bench/projects.json")
    ap.add_argument("--threads", default="default,4,8,16,all",
                    help="comma-separated: default, a number, or all (= os.cpu_count())")
    ap.add_argument("--reps", type=int, default=20)
    ap.add_argument("--timeout", type=float, default=900)
    ap.add_argument("--work-dir", type=Path, default=rb.BENCH / ".work")
    ap.add_argument("--label", help="machine label")
    ap.add_argument("--out-dir", type=Path, default=rb.BENCH / "results" / "compare")
    ap.add_argument("--render", type=Path, help="only re-render the .md of this results .json (no benchmarking)")
    args = ap.parse_args()
    if args.render:
        args.render.with_suffix(".md").write_text(markdown(json.loads(args.render.read_text())))
        return
    if ("tsrs" in args.compilers and not args.tsrs) or ("bun" in args.compilers and not args.bun):
        ap.error("--tsrs and --bun are required for those compilers")

    cfg = json.loads((rb.BENCH / "projects.json").read_text())
    by_name = {p["name"]: p for p in cfg["projects"]}
    names = args.projects.split(",")
    if unknown := set(names) - set(by_name):
        sys.exit(f"unknown projects: {', '.join(sorted(unknown))}")
    compilers = args.compilers.split(",")
    if unknown := set(compilers) - set(COMPILERS):
        sys.exit(f"unknown compilers: {', '.join(sorted(unknown))}")
    threads = [str(os.cpu_count()) if t == "all" else t for t in args.threads.split(",")]
    threads = list(dict.fromkeys(threads))
    for t in threads:
        if t != "default" and not t.isdigit():
            sys.exit(f"bad --threads entry {t!r}")

    work = args.work_dir.resolve()
    exes: dict[str, Path] = {}
    versions: dict[str, str] = {}
    for c in compilers:
        if c == "tsgo":
            exes[c] = rb.ensure_tsgo(cfg["tsgo"], work).resolve()
        elif c == "tsgo-dev":
            exes[c] = rb.ensure_tsgo(cfg["reference"], work).resolve()
        else:
            exes[c] = (args.tsrs if c == "tsrs" else args.bun).resolve()
        flag = "--revision" if c == "bun" else "--version"
        versions[c] = subprocess.run([str(exes[c]), flag], capture_output=True, text=True, check=True).stdout.strip()
    commit = subprocess.run(["git", "-C", str(rb.REPO), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    versions["tsrs"] = f"{versions.get('tsrs', '')} @ {commit[:12]}" if "tsrs" in versions else ""
    versions = {c: versions[c] for c in compilers}
    for n in names:
        rb.setup_project(cfg, by_name[n], work)

    now = dt.datetime.now(dt.timezone.utc)
    machine = rb.machine_info(False, args.label)
    result: dict = {"date": now.strftime("%Y-%m-%d %H:%M UTC"), "machine": machine, "tsrs_commit": commit,
                    "versions": versions, "compilers": compilers, "threads": threads, "reps": args.reps,
                    "projects": {}, "raw": []}
    logs = work / "logs" / f"compare-{now.strftime('%Y%m%d-%H%M%S')}"
    t_start = time.perf_counter()
    for n in names:
        cwd, proj = rb.project_path(cfg, by_name[n], work)
        for c in compilers:
            rb.log(f"{n}: warm-up ({c}, untimed)")
            run_once(c, exes[c], cwd, proj, "default", logs / f"{n}-warmup-{c}.log", args.timeout)
        runs = {t: {c: [] for c in compilers} for t in threads}
        for rep in range(args.reps):
            order = compilers[rep % len(compilers):] + compilers[:rep % len(compilers)]
            for t in threads:
                for c in order:
                    r = run_once(c, exes[c], cwd, proj, t, logs / f"{n}-{t}-{c}-{rep}.log", args.timeout)
                    rb.log(f"{n} threads={t:7} {c:8} rep {rep}: wall {r['wall_s']:.2f} s, peak "
                           f"{rb.fmt_mem(r['peak_rss_bytes'])}, errors {r['errors']}, exit {r['exit']}"
                           + ("" if r["ok"] else "  FAILED"))
                    runs[t][c].append(r)
                    result["raw"].append({"project": n, "threads": t, "compiler": c, "rep": rep,
                                          **{k: v for k, v in r.items() if k != "error_keys"}})
        ref_runs = [r for r in runs.get("default", {}).get("tsgo-dev", []) if r["ok"]]
        ref_keys = tuple(ref_runs[0]["error_keys"]) if ref_runs else None
        result["projects"][n] = {t: {c: summarize(runs[t][c], ref_keys) for c in compilers} for t in threads}
    result["duration_s"] = round(time.perf_counter() - t_start)

    table = markdown(result)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    stem = f"{now.strftime('%Y-%m-%d')}-{commit[:12]}-{machine['cpus']}t"
    (args.out_dir / f"{stem}.json").write_text(json.dumps(result, indent=1) + "\n")
    (args.out_dir / f"{stem}.md").write_text(table)
    if summary := os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(summary, "a") as f:
            f.write(table)
    print(table)
    rb.log(f"wrote {args.out_dir / stem}.json/.md in {result['duration_s']} s; logs in {logs}")


if __name__ == "__main__":
    main()
