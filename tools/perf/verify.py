#!/usr/bin/env python3
"""Before/after check of a tsrs branch: are its diagnostics identical to a base build's, and what did it do to wall
time, peak memory and instructions? (.depot/workflows/pr-verify.yml; bench/README.md "Verifying a branch".)

    tools/perf/verify.py --base /tmp/verify/base/tsrs --new /tmp/verify/new/tsrs --out verify-out
    tools/perf/verify.py --base ... --new ... --projects xstate-main,webpack --checkers 1,4 --reps 1 --no-poison

For each project of bench/projects.json and each checker count, both binaries run
`-p <project> --noEmit --incremental false --extendedDiagnostics --pretty false --checkers N`, `--reps` times,
interleaved (the order flips every rep). A cell is identical when every run of both binaries printed the same
diagnostics (each `error TS` line and its indented continuation lines, in order) and exited with the same code as
the base's first run. The rest of the output (the --extendedDiagnostics counters; times and memory left out) is
compared too and its first difference recorded in verify.json, but it does not decide.

Per project, also: one untimed single-threaded run of each binary through bench/count.py (`--singleThreaded`,
RAYON_NUM_THREADS=1) for instructions retired and peak RSS (Linux only), whose diagnostics must match too; and, unless
--no-poison, one run of the new binary at 16 checkers with TSRS_ARENA_POISON=1 (freed arena memory filled and never
reused), which must not crash and must print the base's diagnostics.

Writes <out>/verify.json, <out>/verify.md and the compiler output under <out>/logs/. Exits 1 when any cell is not
identical.
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

sys.path.insert(0, str(Path(__file__).resolve().parent.parent.parent / "bench"))
import run as rb  # noqa: E402  (bench/run.py: project setup and paths, output parsing, machine info)

FLAGS = ["--noEmit", "--incremental", "false", "--extendedDiagnostics", "--pretty", "false"]
POISON_CHECKERS = 16
ERROR_LINE_RE = re.compile(r"(^|: )error TS\d+:")
# --extendedDiagnostics lines that differ between any two runs; left out of the full-text comparison.
VOLATILE_RE = re.compile(r"^(?:[A-Za-z ]+ time|Memory used):\s")


def diagnostics(text: str) -> list[str]:
    """The `error TS` lines with their indented continuation lines (message chains, related information)."""
    out, inside = [], False
    for line in text.splitlines():
        if ERROR_LINE_RE.search(line):
            out.append(line)
            inside = True
        elif inside and line[:1] in (" ", "\t") and line.strip():
            out.append(line)
        else:
            inside = False
    return out


def first_difference(a: list[str], b: list[str]) -> dict | None:
    """The first line where two outputs differ: {"line": 1-based, "base": ..., "new": ...}, or None."""
    for i in range(max(len(a), len(b))):
        x, y = (a[i] if i < len(a) else "<end of output>"), (b[i] if i < len(b) else "<end of output>")
        if x != y:
            return {"line": i + 1, "base": x, "new": y}
    return None


def run_once(exe: Path, cwd: Path, proj: Path, checkers: int, log: Path, timeout: float, env: dict | None = None) -> dict:
    """One timed run; stdout and stderr go to <log>.out and <log>.err. Wall, user CPU and peak RSS from wait4, as in
    bench/run.py."""
    argv = [str(exe), "-p", str(proj), *FLAGS, "--checkers", str(checkers)]
    log.parent.mkdir(parents=True, exist_ok=True)
    out_path, err_path = log.with_suffix(".out"), log.with_suffix(".err")
    with open(out_path, "wb") as out, open(err_path, "wb") as err:
        t0 = time.perf_counter()
        p = subprocess.Popen(argv, cwd=cwd, stdout=out, stderr=err, env=env)
        timer = threading.Timer(timeout, p.kill)
        timer.start()
        _, status, ru = os.wait4(p.pid, 0)
        wall = time.perf_counter() - t0
        timer.cancel()
    p.returncode = os.waitstatus_to_exitcode(status)  # keep Popen from waiting on a reaped pid
    stdout = out_path.read_text(errors="replace")
    stderr = err_path.read_text(errors="replace")
    r: dict = {"exit": p.returncode, "wall_s": round(wall, 3), "user_s": round(ru.ru_utime, 3),
               "peak_rss_bytes": ru.ru_maxrss * (1 if sys.platform == "darwin" else 1024),
               "log": str(out_path)}
    for line in stdout.splitlines():
        if m := rb.TIME_RE.match(line):
            r[f"{m.group(1).lower()}_s"] = float(m.group(2))
    r["ok"] = p.returncode in (0, 1, 2) and "check_s" in r
    r["_diagnostics"] = diagnostics(stdout + "\n" + stderr)
    r["_text"] = [l for l in stdout.splitlines() + stderr.splitlines() if not VOLATILE_RE.match(l)]
    return r


def count_run(exe: Path, cwd: Path, proj: Path, log: Path, timeout: float) -> dict:
    """The untimed single-threaded run through bench/count.py (instructions retired, peak RSS; Linux only)."""
    argv = [str(exe), "-p", str(proj), "--noEmit", "--incremental", "false", "--singleThreaded", "--pretty", "false"]
    log.parent.mkdir(parents=True, exist_ok=True)
    out = log.with_suffix(".json")
    with open(log, "wb") as f:
        subprocess.run([sys.executable, str(rb.BENCH / "count.py"), str(out), "--", *argv], cwd=cwd, stdout=f,
                       stderr=subprocess.STDOUT, env=dict(os.environ, RAYON_NUM_THREADS="1"), timeout=timeout)
    c = json.loads(out.read_text())
    return {"exit": c.get("exit"), "instructions": c.get("instructions"), "peak_rss_bytes": c.get("max_rss_bytes"),
            "count_error": c.get("error"), "log": str(log),
            "_diagnostics": diagnostics(log.read_text(errors="replace"))}


def median(xs: list) -> float | None:
    xs = [x for x in xs if x is not None]
    return statistics.median(xs) if xs else None


def delta(base: float | None, new: float | None) -> float | None:
    return None if not base or new is None else (new - base) / base * 100


def public(r: dict) -> dict:
    return {k: v for k, v in r.items() if not k.startswith("_")}


def compare_cell(base_runs: list[dict], new_runs: list[dict]) -> dict:
    """Identity of one (project, checkers) cell: every run equals the base's first run in diagnostics and exit code."""
    ref = base_runs[0]
    problems = []
    for which, runs in (("base", base_runs), ("new", new_runs)):
        for i, r in enumerate(runs):
            if not r["ok"]:
                problems.append(f"{which} rep {i}: did not finish (exit {r['exit']})")
            if r["exit"] != ref["exit"]:
                problems.append(f"{which} rep {i}: exit {r['exit']}, base rep 0 exit {ref['exit']}")
            if r is not ref and r["_diagnostics"] != ref["_diagnostics"]:
                d = first_difference(ref["_diagnostics"], r["_diagnostics"])
                problems.append(f"{which} rep {i}: diagnostics differ from base rep 0 at diagnostic line {d['line']}: "
                                f"base `{d['base']}`, {which} `{d['new']}`")
    text = first_difference(ref["_text"], new_runs[0]["_text"])
    return {"identical": not problems, "problems": problems,
            "errors": {"base": sum(1 for l in ref["_diagnostics"] if ERROR_LINE_RE.search(l)),
                       "new": sum(1 for l in new_runs[0]["_diagnostics"] if ERROR_LINE_RE.search(l))},
            "full_text_first_difference": text}


def summarize(runs: list[dict]) -> dict:
    ok = [r for r in runs if r["ok"]] or runs
    return {k: median([r.get(k) for r in ok]) for k in ("wall_s", "user_s", "peak_rss_bytes", "check_s")}


# Markdown ----------------------------------------------------------------------------------------------------------


def fmt_pct(x: float | None) -> str:
    return "n/a" if x is None else f"{x:+.1f}%"


def fmt_s(x: float | None) -> str:
    return "n/a" if x is None else f"{x:.2f}"


def fmt_g(x: float | None) -> str:
    return "n/a" if x is None else f"{x / 1e9:.3f} G"


def yes(ok: bool) -> str:
    return "yes" if ok else "**NO**"


def markdown(result: dict) -> str:
    cells = [(name, c) for name, pr in result["projects"].items() for c in pr["cells"]]
    identical = sum(c["identical"] for _, c in cells)
    timed = [(name, c) for name, c in cells if c.get("kind") == "checkers"]
    biggest = lambda key: max(((name, c) for name, c in timed if c.get(key) is not None),
                              key=lambda nc: abs(nc[1][key]), default=None)
    where = lambda nc, key: "n/a" if nc is None else f"{fmt_pct(nc[1][key])} ({nc[0]}, {nc[1]['label']})"
    b, n = result["base"], result["new"]
    lines = [
        "<!-- pr-verify -->",
        f"## pr-verify: `{b['label']}` → `{n['label']}`",
        "",
        f"**Identical diagnostics in {identical} of {len(cells)} cells**"
        + ("" if identical == len(cells) else " ❌") + f". Largest peak memory change: "
        f"{where(biggest('rss_delta_pct'), 'rss_delta_pct')}. Largest wall time change: "
        f"{where(biggest('wall_delta_pct'), 'wall_delta_pct')}.",
        "",
    ]
    failed = [(name, c) for name, c in cells if not c["identical"]]
    if failed:
        lines += ["**Not identical:**", ""]
        for name, c in failed:
            lines.append(f"- {name}, {c['label']}: " + "; ".join(c["problems"][:3])
                         + (f" (and {len(c['problems']) - 3} more)" if len(c["problems"]) > 3 else ""))
        lines.append("")
    lines += ["| project | identical | wall Δ (median over checker counts) | peak RSS Δ (largest) | instructions Δ (1 thread) |",
              "| --- | --- | ---: | ---: | ---: |"]
    for name, pr in result["projects"].items():
        tc = [c for c in pr["cells"] if c.get("kind") == "checkers"]
        walls = [c["wall_delta_pct"] for c in tc if c.get("wall_delta_pct") is not None]
        rss = max((c["rss_delta_pct"] for c in tc if c.get("rss_delta_pct") is not None), key=abs, default=None)
        instr = next((c for c in pr["cells"] if c.get("kind") == "instructions"), None)
        lines.append(f"| {name} | {yes(all(c['identical'] for c in pr['cells']))} | "
                     f"{fmt_pct(median(walls))} | {fmt_pct(rss)} | {fmt_pct(instr and instr.get('instructions_delta_pct'))} |")
    lines += ["", "<details><summary>Per-project tables</summary>", ""]
    for name, pr in result["projects"].items():
        lines += [f"#### {name}", "",
                  "| checkers | identical | errors | wall base (s) | wall new (s) | Δ | peak RSS base | peak RSS new | Δ | "
                  "check base (s) | check new (s) |",
                  "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"]
        for c in pr["cells"]:
            errs = c.get("errors")
            e = "n/a" if not errs else (str(errs["base"]) if errs["base"] == errs["new"] else f"{errs['base']} → {errs['new']}")
            if c["kind"] == "checkers":
                bs, ns = c["base"], c["new"]
                lines.append(f"| {c['checkers']} | {yes(c['identical'])} | {e} | {fmt_s(bs['wall_s'])} | {fmt_s(ns['wall_s'])} | "
                             f"{fmt_pct(c['wall_delta_pct'])} | {rb.fmt_mem(bs['peak_rss_bytes'])} | "
                             f"{rb.fmt_mem(ns['peak_rss_bytes'])} | {fmt_pct(c['rss_delta_pct'])} | "
                             f"{fmt_s(bs['check_s'])} | {fmt_s(ns['check_s'])} |")
            elif c["kind"] == "instructions":
                bs, ns = c["base"], c["new"]
                lines.append(f"| 1 thread, instructions | {yes(c['identical'])} | {e} | {fmt_g(bs['instructions'])} | "
                             f"{fmt_g(ns['instructions'])} | {fmt_pct(c['instructions_delta_pct'])} | "
                             f"{rb.fmt_mem(bs['peak_rss_bytes'])} | {rb.fmt_mem(ns['peak_rss_bytes'])} | "
                             f"{fmt_pct(c['rss_delta_pct'])} | | |")
            else:  # poison
                ns = c["new"]
                lines.append(f"| {c['checkers']}, poisoned arenas (new) | {yes(c['identical'])} | {e} | | "
                             f"{fmt_s(ns['wall_s'])} | | | {rb.fmt_mem(ns['peak_rss_bytes'])} | | | {fmt_s(ns.get('check_s'))} |")
        lines.append("")
    lines += ["</details>", "",
              f"Wall, peak RSS (`ru_maxrss`) and check time: median of {result['reps']} interleaved run{'s' if result['reps'] != 1 else ''} per binary. "
              "The instructions row: user-space instructions retired by one `--singleThreaded` run "
              "(`bench/count.py`; repeats to about 0.001%) in the wall columns, its peak RSS in the memory columns. "
              "Wall time on these machines moves 2-4% between identical runs; instructions and single-threaded peak "
              "RSS are the stable signals. Identical: same `error TS` lines (with continuation lines) and exit code "
              "in every run of both binaries.",
              "",
              f"Base `{b['commit'][:12]}`, new `{n['commit'][:12]}`{' + local changes' if n.get('dirty') else ''}; "
              f"`cargo build --release` of each. Machine: {result['machine']['label']}. "
              f"Flags: `{' '.join(FLAGS)} --checkers N`. Took {result['duration_s']} s."]
    return "\n".join(lines) + "\n"


# Main --------------------------------------------------------------------------------------------------------------


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--base", type=Path, required=True, help="the baseline tsrs binary")
    ap.add_argument("--new", type=Path, required=True, help="the tsrs binary under test")
    ap.add_argument("--base-label", help="name for the base in the table (default: its --version)")
    ap.add_argument("--new-label", help="name for the new binary in the table (default: its --version)")
    ap.add_argument("--base-commit", default="", help="commit the base was built from (recorded)")
    ap.add_argument("--new-commit", default="", help="commit the new binary was built from (recorded)")
    ap.add_argument("--new-dirty", action="store_true", help="the new binary includes uncommitted changes")
    ap.add_argument("--projects", default="all", help="comma-separated bench/projects.json names, or all")
    ap.add_argument("--checkers", default="1,4,16,32", help="comma-separated checker counts")
    ap.add_argument("--reps", type=int, default=3)
    ap.add_argument("--poison", action=argparse.BooleanOptionalAction, default=True,
                    help=f"run the new binary once per project at {POISON_CHECKERS} checkers with TSRS_ARENA_POISON=1")
    ap.add_argument("--timeout", type=float, default=900, help="per-run timeout in seconds")
    ap.add_argument("--work-dir", type=Path, default=rb.BENCH / ".work", help="bench/run.py's work dir (projects)")
    ap.add_argument("--out", type=Path, default=Path("verify-out"))
    ap.add_argument("--render", action="store_true", help="only re-render <out>/verify.md from <out>/verify.json")
    args = ap.parse_args()
    if args.render:
        (args.out / "verify.md").write_text(markdown(json.loads((args.out / "verify.json").read_text())))
        return

    cfg = json.loads((rb.BENCH / "projects.json").read_text())
    by_name = {p["name"]: p for p in cfg["projects"]}
    names = list(by_name) if args.projects in ("", "all") else args.projects.split(",")
    if unknown := set(names) - set(by_name):
        sys.exit(f"unknown projects: {', '.join(sorted(unknown))}")
    checkers = [int(c) for c in args.checkers.split(",")]
    work = args.work_dir.resolve()
    exes = {"base": args.base.resolve(), "new": args.new.resolve()}
    versions = {k: subprocess.run([str(e), "--version"], capture_output=True, text=True, check=True).stdout.strip()
                for k, e in exes.items()}
    for n in names:
        rb.setup_project(cfg, by_name[n], work)

    args.out = args.out.resolve()  # the compilers and bench/count.py run in the project directories
    logs = args.out / "logs"
    result: dict = {
        "date": dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d %H:%M UTC"),
        "machine": rb.machine_info(False, None),
        "base": {"binary": str(exes["base"]), "version": versions["base"], "commit": args.base_commit,
                 "label": args.base_label or versions["base"]},
        "new": {"binary": str(exes["new"]), "version": versions["new"], "commit": args.new_commit,
                "dirty": args.new_dirty, "label": args.new_label or versions["new"]},
        "projects_selected": names, "checkers": checkers, "reps": args.reps, "poison": args.poison,
        "flags": " ".join(FLAGS), "projects": {}, "runs": [],
    }
    t_start = time.perf_counter()
    for name in names:
        cwd, proj = rb.project_path(cfg, by_name[name], work)
        rb.log(f"{name}: warm-up (base, untimed)")
        run_once(exes["base"], cwd, proj, max(checkers), logs / name / "warmup", args.timeout)
        runs: dict = {c: {"base": [], "new": []} for c in checkers}
        for rep in range(args.reps):
            order = ["base", "new"] if rep % 2 == 0 else ["new", "base"]
            for c in checkers:
                for which in order:
                    r = run_once(exes[which], cwd, proj, c, logs / name / f"checkers{c}-{which}-{rep}", args.timeout)
                    rb.log(f"{name} checkers {c:>2} {which:4} rep {rep}: wall {r['wall_s']:.2f} s, check "
                           f"{r.get('check_s')} s, peak {rb.fmt_mem(r['peak_rss_bytes'])}, exit {r['exit']}"
                           + ("" if r["ok"] else "  FAILED"))
                    runs[c][which].append(r)
                    result["runs"].append({"project": name, "checkers": c, "binary": which, "rep": rep, **public(r)})
        cells = []
        for c in checkers:
            cell = {"kind": "checkers", "checkers": c, "label": f"{c} checker" + ("s" if c != 1 else ""),
                    **compare_cell(runs[c]["base"], runs[c]["new"]),
                    "base": summarize(runs[c]["base"]), "new": summarize(runs[c]["new"])}
            cell["wall_delta_pct"] = delta(cell["base"]["wall_s"], cell["new"]["wall_s"])
            cell["rss_delta_pct"] = delta(cell["base"]["peak_rss_bytes"], cell["new"]["peak_rss_bytes"])
            cell["user_delta_pct"] = delta(cell["base"]["user_s"], cell["new"]["user_s"])
            cells.append(cell)

        # One untimed single-threaded run of each binary: instructions retired (Linux) and the single-threaded output.
        counted = {w: count_run(exes[w], cwd, proj, logs / name / f"instructions-{w}.log", args.timeout)
                   for w in ("base", "new")}
        problems = []
        for w, r in counted.items():
            if r["exit"] not in (0, 1, 2):
                problems.append(f"{w}: exit {r['exit']}")
        if counted["base"]["exit"] != counted["new"]["exit"]:
            problems.append(f"exit {counted['base']['exit']} vs {counted['new']['exit']}")
        if d := first_difference(counted["base"]["_diagnostics"], counted["new"]["_diagnostics"]):
            problems.append(f"diagnostics differ at diagnostic line {d['line']}: base `{d['base']}`, new `{d['new']}`")
        instr = {"kind": "instructions", "label": "single-threaded", "identical": not problems, "problems": problems,
                 "errors": {w: sum(1 for l in counted[w]["_diagnostics"] if ERROR_LINE_RE.search(l)) for w in counted},
                 "base": public(counted["base"]), "new": public(counted["new"]),
                 "instructions_delta_pct": delta(counted["base"]["instructions"], counted["new"]["instructions"]),
                 "rss_delta_pct": delta(counted["base"]["peak_rss_bytes"], counted["new"]["peak_rss_bytes"])}
        rb.log(f"{name} 1 thread: instructions {fmt_g(counted['base']['instructions'])} -> "
               f"{fmt_g(counted['new']['instructions'])} ({fmt_pct(instr['instructions_delta_pct'])}), identical: "
               f"{instr['identical']}")
        cells.append(instr)

        if args.poison:
            # The base at the same checker count is the expected output (measured above, or run once now).
            ref = (runs[POISON_CHECKERS]["base"][0] if POISON_CHECKERS in runs else
                   run_once(exes["base"], cwd, proj, POISON_CHECKERS, logs / name / f"poison-reference-base", args.timeout))
            r = run_once(exes["new"], cwd, proj, POISON_CHECKERS, logs / name / "poison-new", args.timeout,
                         env=dict(os.environ, TSRS_ARENA_POISON="1"))
            problems = [] if r["ok"] else [f"did not finish (exit {r['exit']})"]
            if r["exit"] != ref["exit"]:
                problems.append(f"exit {r['exit']}, base exit {ref['exit']}")
            if d := first_difference(ref["_diagnostics"], r["_diagnostics"]):
                problems.append(f"diagnostics differ from the base at diagnostic line {d['line']}: base `{d['base']}`, "
                                f"poisoned `{d['new']}`")
            rb.log(f"{name} poison, {POISON_CHECKERS} checkers: exit {r['exit']}, identical: {not problems}")
            cells.append({"kind": "poison", "checkers": POISON_CHECKERS, "label": f"poisoned arenas, {POISON_CHECKERS} checkers",
                          "identical": not problems, "problems": problems,
                          "errors": {"base": sum(1 for l in ref["_diagnostics"] if ERROR_LINE_RE.search(l)),
                                     "new": sum(1 for l in r["_diagnostics"] if ERROR_LINE_RE.search(l))},
                          "new": public(r)})
        result["projects"][name] = {"cells": cells}
    result["duration_s"] = round(time.perf_counter() - t_start)

    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "verify.json").write_text(json.dumps(result, indent=1) + "\n")
    table = markdown(result)
    (args.out / "verify.md").write_text(table)
    print(table)
    bad = [f"{n} {c['label']}" for n, pr in result["projects"].items() for c in pr["cells"] if not c["identical"]]
    if bad:
        rb.log(f"NOT IDENTICAL: {', '.join(bad)}")
        sys.exit(1)
    rb.log(f"identical everywhere; wrote {args.out}/verify.json and verify.md in {result['duration_s']} s")


if __name__ == "__main__":
    main()
