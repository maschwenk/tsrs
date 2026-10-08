#!/usr/bin/env python3
"""The WebAssembly table: native tsrs `--singleThreaded` vs tsrs.wasm (npm/tsrs-wasm) vs ts-rust's wasm module.

See bench/README.md, "WebAssembly". Typical use:

    tools/wasm/build.sh                                   # npm/tsrs-wasm/tsrs.wasm
    python3 bench/wasm.py --build-ts-rust                 # ts-rust's module at TS_RUST's pinned commit (cached)
    python3 bench/wasm.py --local --projects xstate-main,Compiler
    python3 bench/run.py --merge <partials> --wasm <wasm result.json>   # CI: one results file with every table

Per project, in three rounds whose order rotates: one native run (bench/run.py's `single` mode, with
RAYON_NUM_THREADS=1 so that parsing is on one thread too, like the module), and one new Node process per module
(tools/wasm/bench.mjs --child: the package's `tsc()`, which bin/tsrs-wasm.js calls). The first call of each process is
a cold run (compiling the module included); the first round's processes then make --warm more calls, the warm runs.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import run as bench  # noqa: E402  (bench/run.py: project setup, machine info, the native run, formatting)

REPO, BENCH = bench.REPO, bench.BENCH
START, END = "<!-- bench-wasm:start -->", "<!-- bench-wasm:end -->"
# The projects whose native single-threaded check takes under 2 s on the fixed-spec runner (0.2-1.7 s), so that the
# modules' runs fit the 4 GiB of linear memory and ts-rust's take under ~30 s each. Left out: vscode, mui-docs, the
# four applications (cal-diy, formbricks-web, supabase-studio, t3code-server) and mikro-orm, at 3.1-10.4 s and
# 0.75-1.6 GiB native (8 runs per module at 3-8x that would be 5-15 min per project), and next-root (the same
# repository as next-packages-next).
PROJECTS = "xstate-main,webpack,Compiler,Compiler-Unions,next-packages-next,storybook,playwright,nuxt,drizzle-orm"
FLAGS = ["--noEmit", "--incremental", "false", "--extendedDiagnostics", "--pretty", "false"]
# ts-rust (pingdotgg/ts-rust, a Rust port of TypeScript 7 by other authors) publishes no wasm package (npm
# `ts-rust-wasm` is not on the registry; its v0.1.0 release has native binaries only), so the bench builds its
# npm/wasm module from source with its own script, at this commit, with its default settings (opt-level z).
TS_RUST = {"repo": "https://github.com/pingdotgg/ts-rust", "commit": "272f79b0388177334cdaa2b3c729d8dc60d75e28",
           "build": "`scripts/wasm/build.sh` (its default: opt-level z, `wasm-opt --flatten --rereloop -Oz -Oz`, "
                    "function reordering)"}
TSRS_WASM_BUILD = "`tools/wasm/build.sh` (opt-level s, inline threshold 150, `wasm-opt -Os`)"
ENGINES = ["tsrs-wasm", "ts-rust"]


def log(msg: str) -> None:
    print(f"[wasm {time.strftime('%H:%M:%S')}] {msg}", file=sys.stderr, flush=True)


def module_sizes(path: Path) -> dict:
    """Raw, gzip -9 and brotli quality 11 sizes, computed with Node's zlib as tools/wasm/build.sh does."""
    js = ("const z=require('zlib'),b=require('fs').readFileSync(process.argv[1]);console.log(JSON.stringify({raw:b.length,"
          "gzip:z.gzipSync(b,{level:9}).length,brotli:z.brotliCompressSync(b,{params:{[z.constants.BROTLI_PARAM_QUALITY]"
          ":11,[z.constants.BROTLI_PARAM_SIZE_HINT]:b.length}}).length}))")
    sizes = json.loads(subprocess.run(["node", "-e", js, str(path)], capture_output=True, text=True, check=True).stdout)
    sizes["sha256"] = hashlib.sha256(path.read_bytes()).hexdigest()
    return sizes


def build_ts_rust(dest: Path, work: Path) -> None:
    """ts-rust's npm/wasm package (its JS host files and ts_rust.wasm) at TS_RUST's commit, in `dest`; a no-op when
    `dest` already holds that commit's build (the CI cache)."""
    info = dest / "build.json"
    try:
        if json.loads(info.read_text())["commit"] == TS_RUST["commit"]:
            log(f"ts-rust: cached module {TS_RUST['commit'][:12]}")
            return
    except (OSError, ValueError, KeyError):
        pass
    src = work / "ts-rust-src"
    bench.git_checkout(TS_RUST["repo"], TS_RUST["commit"], src)
    t0 = time.perf_counter()
    # CI=1: its script otherwise runs cargo under systemd-run when that exists.
    bench.sh([str(src / "scripts/wasm/build.sh")], cwd=src, env=dict(os.environ, CI="1"))
    if dest.exists():
        shutil.rmtree(dest)
    dest.mkdir(parents=True)
    for f in (src / "npm/wasm").iterdir():
        if f.suffix in (".js", ".wasm") or f.name == "package.json":
            shutil.copy2(f, dest / f.name)
    rustc = subprocess.run(["rustc", "-V"], capture_output=True, text=True, cwd=src).stdout.strip()
    info.write_text(json.dumps({**TS_RUST, "rustc": rustc, "build_s": round(time.perf_counter() - t0)}) + "\n")
    shutil.rmtree(src)
    log(f"ts-rust: built {dest / 'ts_rust.wasm'} in {time.perf_counter() - t0:.0f} s")


def run_module(pkg: Path, module: Path, cwd: Path, proj: Path, calls: int, timeout: float) -> dict:
    """One new Node process making `calls` tsc() calls (tools/wasm/bench.mjs --child)."""
    case = json.dumps({"cwd": str(cwd), "args": ["-p", str(proj), *FLAGS]})
    argv = ["node", str(REPO / "tools/wasm/bench.mjs"), "--child", str(pkg), str(module), case, str(calls)]
    t0 = time.perf_counter()
    try:
        p = subprocess.run(argv, cwd=cwd, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return {"ok": False, "failure": f"timeout after {timeout:.0f} s", "times_s": []}
    process_s = round(time.perf_counter() - t0, 3)
    try:
        r = json.loads(p.stdout)
    except ValueError:
        tail = (p.stderr or p.stdout).strip().splitlines()[-3:]
        return {"ok": False, "failure": f"node exited {p.returncode}: {' | '.join(tail)}"[:300], "times_s": []}
    # 0-2 are tsc's (2: errors, outputs skipped); 5 is a crash (a trap, e.g. out of linear memory); null a throw.
    ok = len(r["times"]) == calls and all(c in (0, 1, 2) for c in r["exitCodes"])
    out = {"ok": ok, "times_s": [round(t / 1000, 3) for t in r["times"]], "exit_codes": r["exitCodes"],
           "errors": r["errors"], "process_s": process_s, "max_rss_bytes": r["maxRssKiB"] * 1024,
           "linear_memory_bytes": r.get("memoryBytes") or None}
    if not ok:
        out["failure"] = f"exit {r['exitCodes'][-1] if r['exitCodes'] else '?'}: {r.get('stderr', '').strip()[-300:]}"
    return out


def stats(xs: list[float]) -> dict:
    xs = [x for x in xs if x is not None]
    return {"median": bench.median(xs), "min": min(xs, default=None), "max": max(xs, default=None), "n": len(xs)}


def summarize_engine(procs: list[dict]) -> dict:
    """procs[0] is the first round's process (1 + warm calls), the others make one call each."""
    s: dict = {"processes": len(procs), "ok": bool(procs) and all(p["ok"] for p in procs)}
    if not s["ok"]:
        s["failure"] = next((p["failure"] for p in procs if not p["ok"]), "not run")
        return s
    s["cold_s"] = stats([p["times_s"][0] for p in procs])
    s["warm_s"] = stats(procs[0]["times_s"][1:])
    s["process_s"] = stats([p["process_s"] for p in procs[1:]] or [procs[0]["process_s"]])
    # Peak RSS of a one-call process (a CLI run); the first round's process holds several runs.
    s["peak_rss_bytes"] = bench.median([p["max_rss_bytes"] for p in procs[1:]] or [procs[0]["max_rss_bytes"]])
    s["linear_memory_bytes"] = procs[0]["linear_memory_bytes"]
    s["error_counts"] = sorted({e for p in procs for e in p["errors"]})
    s["errors"] = s["error_counts"][0] if len(s["error_counts"]) == 1 else None
    return s


def measure(args: argparse.Namespace, cfg: dict, projects: list[dict], work: Path, logs: Path) -> dict:
    tsrs = args.tsrs.resolve()
    modules = {"tsrs-wasm": (args.tsrs_wasm_pkg.resolve(), args.tsrs_wasm.resolve()),
               "ts-rust": (args.ts_rust_pkg.resolve(), (args.ts_rust_pkg / "ts_rust.wasm").resolve())}
    for name, (pkg, module) in modules.items():
        if not module.exists():
            sys.exit(f"{name}: no module at {module} (tools/wasm/build.sh / bench/wasm.py --build-ts-rust)")
    commit = subprocess.run(["git", "-C", str(REPO), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    dirty = bool(subprocess.run(["git", "-C", str(REPO), "status", "--porcelain", "--untracked-files=no"],
                                capture_output=True, text=True).stdout.strip())
    try:
        ts_rust_info = json.loads((args.ts_rust_pkg / "build.json").read_text())
    except (OSError, ValueError):
        ts_rust_info = {**TS_RUST, "note": "no build.json next to the module: not built by bench/wasm.py"}
    now = dt.datetime.now(dt.timezone.utc)
    node = subprocess.run(["node", "--version"], capture_output=True, text=True, check=True).stdout.strip()
    result: dict = {
        "kind": "wasm",
        "date": now.strftime("%Y-%m-%d %H:%M UTC"),
        "machine": bench.machine_info(args.local, args.label),
        "tsrs": {"commit": commit, "dirty": dirty,
                 "version": subprocess.run([str(tsrs), "--version"], capture_output=True, text=True).stdout.strip(),
                 "build": args.tsrs_build, "rustc": args.rustc or bench.rustc_version()},
        "node": node,
        "modules": {"tsrs-wasm": {"build": TSRS_WASM_BUILD, "commit": commit, "rustc": args.rustc or bench.rustc_version(),
                                  **module_sizes(modules["tsrs-wasm"][1])},
                    "ts-rust": {**{k: ts_rust_info.get(k) for k in ("repo", "commit", "build", "rustc")},
                                **module_sizes(modules["ts-rust"][1])}},
        "flags": " ".join(FLAGS), "native_flags": "--singleThreaded (RAYON_NUM_THREADS=1)",
        "rounds": args.cold, "warm_calls": args.warm,
        "projects": {}, "raw": [],
    }
    order = ["native", *ENGINES]
    t_start = time.perf_counter()
    # bench.run_once passes this process's environment to native tsrs: parse on one thread too, as the module does.
    os.environ["RAYON_NUM_THREADS"] = "1"
    for p in projects:
        name = p["name"]
        cwd, proj = bench.project_path(cfg, p, work)
        native: list[dict] = []
        procs: dict = {e: [] for e in ENGINES}
        for rnd in range(args.cold):
            for engine in order[rnd % len(order):] + order[:rnd % len(order)]:
                if engine == "native":
                    r = bench.run_once(tsrs, cwd, proj, "single", logs / f"{name}-native-{rnd}.log", args.timeout)
                    r.pop("error_keys", None)
                    native.append(r)
                    log(f"{name} native round {rnd}: wall {r['wall_s']:.2f} s, peak {bench.fmt_mem(r['peak_rss_bytes'])}, "
                        f"errors {r['errors']}, exit {r['exit']}")
                    result["raw"].append({"project": name, "engine": "native", "round": rnd, **r})
                    continue
                if procs[engine] and not procs[engine][-1]["ok"]:
                    continue  # failed once (out of memory, a crash, the timeout): not again
                pkg, module = modules[engine]
                r = run_module(pkg, module, cwd, proj, 1 + args.warm if rnd == 0 else 1, args.timeout)
                procs[engine].append(r)
                result["raw"].append({"project": name, "engine": engine, "round": rnd, **r})
                if r["ok"]:
                    log(f"{name} {engine} round {rnd}: " + ", ".join(f"{t:.2f}" for t in r["times_s"])
                        + f" s, peak {bench.fmt_mem(r['max_rss_bytes'])}, errors {sorted(set(r['errors']))}")
                else:
                    log(f"{name} {engine} round {rnd}: FAILED: {r['failure']}")
        n = bench.summarize(native)
        n["wall"] = stats([r["wall_s"] for r in native if r["ok"]])
        pr: dict = {"commit": p.get("commit") or cfg["suite"]["commit"], "project": p["project"], "native": n}
        for e in ENGINES:
            pr[e] = summarize_engine(procs[e])
        w = pr["tsrs-wasm"]
        # The module must print what native --singleThreaded prints (tools/wasm/gate.sh checks it byte for byte);
        # here the error count is the check. ts-rust follows another TypeScript commit: recorded, not compared.
        pr["errors_match"] = (n["error_counts"] == w["error_counts"] and len(w["error_counts"]) == 1) if w["ok"] and n["ok_runs"] else None
        result["projects"][name] = pr
    result["duration_s"] = round(time.perf_counter() - t_start)
    return result


def fmt_range(s: dict | None) -> str:
    if not s or s.get("median") is None:
        return "n/a"
    text = f"{s['median']:.2f}"
    if s["n"] > 1:
        text += f" ({s['min']:.2f}-{s['max']:.2f})"
    return text


def ratio(a: float | None, b: float | None) -> str:
    return bench.fmt_ratio(a / b) if a and b else "n/a"


def markdown(result: dict, compact: bool = False) -> str:
    """The WebAssembly table and the module sizes. `compact` (the README's block): medians only, no notes on errors."""
    commit = result["tsrs"]["commit"][:12]
    mods = result["modules"]
    tr = mods["ts-rust"]
    lines = [
        "### WebAssembly: tsrs-wasm vs ts-rust's wasm module vs native tsrs, one thread",
        "",
        f"The same type check (`-p <project> {result['flags']}`) three ways: native tsrs `--singleThreaded` "
        f"({bench.TSRS_BUILDS.get(result['tsrs'].get('build'), result['tsrs'].get('build'))}, parsing on one thread "
        f"too), the tsrs.wasm module of `npm/tsrs-wasm` at the same commit, and the wasm module of "
        f"[ts-rust](https://github.com/pingdotgg/ts-rust) (`npm/wasm`, commit `{(tr.get('commit') or '?')[:12]}`), "
        f"both run by their package's Node `tsc()` (Node {result['node']}, one worker thread per run). Cold: the "
        f"first run in a new Node process, compiling the module included; warm: the median of "
        f"{result['warm_calls']} more runs in that process. Seconds, median"
        + ("" if compact else " (min-max)") + f" of {result['rounds']} interleaved rounds"
        + ("." if compact else f"; native also {result['rounds']} runs."),
        "",
    ]
    header = ("| project | errors, native / tsrs-wasm / ts-rust | native | tsrs-wasm cold | tsrs-wasm warm | "
              "tsrs-wasm warm / native | ts-rust cold | ts-rust warm | ts-rust warm / tsrs-wasm warm | "
              "native peak memory | tsrs-wasm peak memory | ts-rust peak memory |")
    lines += [header, "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"]
    med = (lambda s: bench.fmt_num(s.get("median")) if s else "n/a") if compact else fmt_range
    failed = []
    for name, pr in result["projects"].items():
        n, w, t = pr["native"], pr["tsrs-wasm"], pr["ts-rust"]
        errs = []
        for s, key in ((n, "error_counts"), (w, "error_counts"), (t, "error_counts")):
            errs.append("/".join(map(str, s.get(key) or [])) or "n/a")
        err = " / ".join(errs)
        if pr.get("errors_match") is False:
            err = f"**MISMATCH {err}**"
        cells = [name, err, med(n.get("wall"))]
        # Each module's warm time against the column before it: tsrs-wasm against native, ts-rust against tsrs-wasm.
        for engine, s, base in (("tsrs-wasm", w, n.get("wall_s")), ("ts-rust", t, w["warm_s"]["median"] if w["ok"] else None)):
            if s["ok"]:
                cells += [med(s["cold_s"]), med(s["warm_s"]), ratio(s["warm_s"]["median"], base)]
            else:
                cells += ["**FAILED**", "n/a", "n/a"]
                failed.append(f"{name} ({engine}: {s.get('failure', '')[:160]})")
        cells += [bench.fmt_mem(n.get("peak_rss_bytes"))] + [bench.fmt_mem(s.get("peak_rss_bytes")) if s["ok"] else "n/a"
                                                             for s in (w, t)]
        lines.append("| " + " | ".join(cells) + " |")
    lines += ["", "| module | raw | gzip -9 | brotli -q 11 | build |", "| --- | ---: | ---: | ---: | --- |"]
    for label, m in (("tsrs.wasm", mods["tsrs-wasm"]), ("ts_rust.wasm", tr)):
        lines.append(f"| {label} | {m['raw']:,} | {m['gzip']:,} | {m['brotli']:,} | {m['build']}, {m.get('rustc') or '?'} |")
    lines.append("")
    if failed:
        lines += ["Failed: " + "; ".join(failed) + ".", ""]
    lines.append(
        "errors: the number of type errors each reports; native's and tsrs-wasm's must be equal (a bold cell is a "
        "disagreement, i.e. a bug in the module). ts-rust ports another TypeScript commit, so its count is recorded, not "
        "compared. peak memory: maximum resident set size of the process (for a module, a Node process making one "
        "run: the module's linear memory plus Node and the compiled code). The module is single-threaded; the native "
        "column is the fair comparison for it, not the multi-threaded default."
        + ("" if compact else " Not measured: projects whose native single-threaded check takes over 3 s "
                              "(vscode, mui-docs, the four applications, mikro-orm) and next-root (bench/wasm.py PROJECTS)."))
    lines += ["", f"Runner: {result['machine']['label']}. Date: {result['date']}. tsrs commit: `{commit}`. "
                  "How it is measured: [`bench/README.md`](bench/README.md#webassembly)."]
    return "\n".join(lines).rstrip() + "\n"


def update_readme(readme: Path, table: str) -> None:
    """Rewrites the WebAssembly block; a README without one gets it right after the main bench block."""
    text = readme.read_text()
    block = f"{START}\n{table}{END}"
    if START in text and END in text:
        pre, rest = text.split(START, 1)
        _, post = rest.split(END, 1)
        text = pre + block + post
    elif bench.END in text:
        pre, post = text.split(bench.END, 1)
        text = f"{pre}{bench.END}\n\n{block}{post}"
    else:
        sys.exit(f"{readme}: no {bench.END} marker to put the WebAssembly block after")
    readme.write_text(text)


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--local", action="store_true", help="label results as measured on this machine")
    ap.add_argument("--label", help="machine label for the table header")
    ap.add_argument("--work-dir", type=Path, default=BENCH / ".work", help="clones, installs, logs (default bench/.work)")
    ap.add_argument("--tsrs", type=Path, default=REPO / "target" / "release" / "tsrs", help="native tsrs")
    ap.add_argument("--tsrs-build", choices=sorted(bench.TSRS_BUILDS), default="release")
    ap.add_argument("--rustc", help="`rustc -V` of the toolchain that built --tsrs and the module (default: this machine's)")
    ap.add_argument("--tsrs-wasm", type=Path, default=REPO / "npm/tsrs-wasm/tsrs.wasm", help="the module (tools/wasm/build.sh)")
    ap.add_argument("--tsrs-wasm-pkg", type=Path, default=REPO / "npm/tsrs-wasm", help="the package whose node.js runs it")
    ap.add_argument("--ts-rust-pkg", type=Path, help="ts-rust's npm/wasm package with ts_rust.wasm (default "
                                                     "<work-dir>/ts-rust-wasm, where --build-ts-rust puts it)")
    ap.add_argument("--build-ts-rust", action="store_true", help="only build ts-rust's module at the pinned commit")
    ap.add_argument("--print-ts-rust-commit", action="store_true", help="print the pinned ts-rust commit (CI cache key)")
    ap.add_argument("--print-projects", action="store_true", help="print --projects, comma-separated (CI cache keys)")
    ap.add_argument("--projects", default=PROJECTS, help=f"comma-separated (default {PROJECTS})")
    ap.add_argument("--warm", type=int, default=5, help="warm runs in the first round's process (default 5)")
    ap.add_argument("--cold", type=int, default=3, help="rounds: new processes per module, and native runs (default 3)")
    ap.add_argument("--timeout", type=float, default=900, help="per process, seconds")
    ap.add_argument("--setup-only", action="store_true", help="clone and install the projects only")
    ap.add_argument("--out-dir", type=Path, default=BENCH / "results",
                    help="writes <date>-<commit>[-local]-wasm.{json,md} (CI: bench/run.py --merge --wasm folds it into "
                         "the run's results file)")
    ap.add_argument("--readme", type=Path, help="rewrite the WebAssembly block of this README")
    ap.add_argument("--render", type=Path, metavar="RESULT", help="only print the table of a wasm result .json")
    args = ap.parse_args()
    if args.print_ts_rust_commit:
        print(TS_RUST["commit"])
        return
    if args.print_projects:
        print(args.projects)
        return
    if args.render:
        print(markdown(json.loads(args.render.read_text())), end="")
        return
    work = args.work_dir.resolve()
    args.ts_rust_pkg = (args.ts_rust_pkg or work / "ts-rust-wasm").resolve()
    if args.build_ts_rust:
        build_ts_rust(args.ts_rust_pkg, work)
        return
    cfg = json.loads((BENCH / "projects.json").read_text())
    want = args.projects.split(",")
    if unknown := set(want) - {p["name"] for p in cfg["projects"]}:
        sys.exit(f"unknown projects: {', '.join(sorted(unknown))}")
    projects = [p for p in cfg["projects"] if p["name"] in want]
    for p in projects:
        bench.setup_project(cfg, p, work)
    if args.setup_only:
        return
    logs = work / "logs" / ("wasm-" + time.strftime("%Y%m%d-%H%M%S"))
    result = measure(args, cfg, projects, work, logs)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    stem = f"{result['date'][:10]}-{result['tsrs']['commit'][:12]}" + ("-local" if args.local else "") + "-wasm"
    table = markdown(result)
    (args.out_dir / f"{stem}.json").write_text(json.dumps(result, indent=1) + "\n")
    (args.out_dir / f"{stem}.md").write_text(table)
    if args.readme:
        update_readme(args.readme, markdown(result, compact=True))
    print(table)
    log(f"wrote {args.out_dir / stem}.json/.md in {result['duration_s']} s; logs in {logs}")
    bad = [n for n, pr in result["projects"].items() if pr["errors_match"] is False or not pr["native"]["ok_runs"]
           or not all(pr[e]["ok"] for e in ENGINES)]
    if bad:
        log(f"WARNING: error-count mismatch or failed runs: {', '.join(bad)}")


if __name__ == "__main__":
    main()
