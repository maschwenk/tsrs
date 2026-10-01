#!/usr/bin/env python3
"""Benchmarks tsrs against tsgo (typescript@7.0.2) on the typescript-benchmarking projects.

See bench/README.md. Typical use:

    python3 bench/run.py --local                       # everything in bench/projects.json
    python3 bench/run.py --local --projects webpack,Compiler --reps 1
    python3 bench/run.py --setup-only                  # clone + install only (CI cache warm-up)
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import platform
import re
import shutil
import statistics
import subprocess
import sys
import threading
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
BENCH = REPO / "bench"
MARKER = ".tsrs-bench.json"
START, END = "<!-- bench:start -->", "<!-- bench:end -->"

COUNTER_RE = re.compile(r"^(Files|Lines|Identifiers|Symbols|Types|Instantiations):\s+(\d+)\s*$")
TIME_RE = re.compile(r"^(Config|Parse|Bind|Check|Total) time:\s+([\d.]+)s\s*$")
MEMORY_RE = re.compile(r"^Memory used:\s+(\d+)K\s*$")
# `--pretty false`: "path(line,col): error TS1234: message" or "error TS5023: message"; continuation lines are indented.
ERROR_RE = re.compile(r"^((?:\S.*\(\d+,\d+\): )?error TS\d+):")


def log(msg: str) -> None:
    print(f"[bench {time.strftime('%H:%M:%S')}] {msg}", file=sys.stderr, flush=True)


def sh(cmd: str | list[str], cwd: Path | None = None, env: dict | None = None) -> None:
    shell = isinstance(cmd, str)
    log(f"$ {cmd if shell else ' '.join(cmd)}" + (f"   (in {cwd})" if cwd else ""))
    subprocess.run(cmd, cwd=cwd, shell=shell, check=True, env=env, executable="/bin/bash" if shell else None)


def git_checkout(repo: str, commit: str, dest: Path) -> None:
    """Shallow checkout of exactly one commit (GitHub serves fetches by SHA)."""
    if dest.exists():
        shutil.rmtree(dest)
    dest.mkdir(parents=True)
    sh(["git", "init", "-q", str(dest)])
    sh(["git", "-C", str(dest), "fetch", "-q", "--depth", "1", repo, commit])
    sh(["git", "-C", str(dest), "-c", "advice.detachedHead=false", "checkout", "-q", "FETCH_HEAD"])


def read_marker(d: Path) -> dict | None:
    try:
        return json.loads((d / MARKER).read_text())
    except (OSError, ValueError):
        return None


def ensure_checkout(name: str, repo: str, commit: str, install: str | None, dest: Path) -> None:
    want = {"repo": repo, "commit": commit, "install": install}
    if read_marker(dest) == want:
        log(f"{name}: cached checkout {commit[:12]}")
        return
    log(f"{name}: setting up {repo} @ {commit[:12]}")
    t0 = time.perf_counter()
    git_checkout(repo, commit, dest)
    if install:
        env = dict(os.environ, CI="true", HUSKY="0")
        sh(install, cwd=dest, env=env)
    (dest / MARKER).write_text(json.dumps(want))
    log(f"{name}: setup took {time.perf_counter() - t0:.0f} s")


def project_path(cfg: dict, p: dict, work: Path) -> tuple[Path, Path]:
    """(cwd, path given to -p)."""
    if "suite_dir" in p:
        root = work / "suite" / p["suite_dir"]
    else:
        root = work / "solutions" / p["name"]
    return root, root / p["project"]


def setup_project(cfg: dict, p: dict, work: Path) -> None:
    if "suite_dir" in p:
        ensure_checkout("typescript-benchmarking", cfg["suite"]["repo"], cfg["suite"]["commit"], None, work / "suite")
    else:
        ensure_checkout(p["name"], p["repo"], p["commit"], p.get("install"), work / "solutions" / p["name"])


def native_platform() -> str:
    arch = {"x86_64": "x64", "amd64": "x64", "arm64": "arm64", "aarch64": "arm64"}[platform.machine().lower()]
    return f"{sys.platform}-{arch}"


def ensure_tsgo(pkgcfg: dict, work: Path) -> Path:
    """Installs typescript@<version> and returns the native compiler its `tsc` launcher execs.

    TypeScript 7's npm `bin/tsc` is a small Node launcher (lib/tsc.js -> getExePath.js) that `execve`s
    `@typescript/typescript-<os>-<arch>/lib/tsc`, the Go compiler. Running that binary directly gives the same
    process minus ~50 ms of Node startup, and makes wall time and peak RSS the compiler's own.
    """
    pkg, version = pkgcfg["package"], pkgcfg["version"]
    d = work / "tsgo" / version
    exe = d / "node_modules" / "@typescript" / f"typescript-{native_platform()}" / "lib" / "tsc"
    if not exe.exists():
        d.mkdir(parents=True, exist_ok=True)
        (d / "package.json").write_text('{"name": "tsgo-bench", "private": true}\n')
        sh(["npm", "install", "--no-audit", "--no-fund", "--no-save", f"{pkg}@{version}"], cwd=d)
    with open(exe, "rb") as f:
        magic = f.read(4)
    if magic not in (b"\x7fELF", b"\xcf\xfa\xed\xfe", b"\xca\xfe\xba\xbe"):
        sys.exit(f"{exe} is not a native executable (magic {magic!r})")
    out = subprocess.run([str(exe), "--version"], capture_output=True, text=True, check=True).stdout.strip()
    if not version.startswith(out.removeprefix("Version ")):
        sys.exit(f"{exe} --version printed {out!r}, expected 'Version {version}'")
    return exe


def run_once(exe: Path, cwd: Path, proj: Path, single: bool, log_path: Path, timeout: float) -> dict:
    argv = [str(exe), "-p", str(proj), "--noEmit", "--incremental", "false", "--extendedDiagnostics", "--pretty", "false"]
    if single:
        argv.append("--singleThreaded")
    log_path.parent.mkdir(parents=True, exist_ok=True)
    with open(log_path, "wb") as out:
        t0 = time.perf_counter()
        p = subprocess.Popen(argv, cwd=cwd, stdout=out, stderr=subprocess.STDOUT)
        timer = threading.Timer(timeout, p.kill)
        timer.start()
        _, status, ru = os.wait4(p.pid, 0)
        wall = time.perf_counter() - t0
        timer.cancel()
    p.returncode = os.waitstatus_to_exitcode(status)  # keep Popen from waiting on a reaped pid
    # ru_maxrss: KiB on Linux, bytes on macOS (the number `/usr/bin/time -v` / `-l` print).
    peak = ru.ru_maxrss * (1 if sys.platform == "darwin" else 1024)
    r: dict = {"exit": p.returncode, "wall_s": round(wall, 3), "peak_rss_bytes": peak, "errors": 0}
    keys = []
    for line in log_path.read_text(errors="replace").splitlines():
        if m := ERROR_RE.match(line):
            keys.append(m.group(1))
        elif m := COUNTER_RE.match(line):
            r[m.group(1).lower()] = int(m.group(2))
        elif m := TIME_RE.match(line):
            r[f"{m.group(1).lower()}_s"] = float(m.group(2))
        elif m := MEMORY_RE.match(line):
            r["memory_used_kb"] = int(m.group(1))
    r["errors"] = len(keys)
    r["error_keys"] = sorted(keys)
    r["ok"] = p.returncode in (0, 1, 2) and "check_s" in r
    return r


def median(xs: list[float]) -> float | None:
    xs = [x for x in xs if x is not None]
    return statistics.median(xs) if xs else None


def summarize(runs: list[dict]) -> dict:
    ok = [r for r in runs if r["ok"]]
    s: dict = {"runs": len(runs), "ok_runs": len(ok)}
    for k in ("wall_s", "check_s", "total_s", "peak_rss_bytes", "memory_used_kb", "files", "symbols", "types",
              "instantiations", "errors"):
        s[k] = median([r.get(k) for r in ok])
    s["error_counts"] = sorted({r["errors"] for r in ok})
    return s


def machine_info(local: bool, label: str | None) -> dict:
    info = {"platform": f"{platform.system()} {platform.machine()}", "cpus": os.cpu_count()}
    if sys.platform == "darwin":
        q = lambda k: subprocess.run(["sysctl", "-n", k], capture_output=True, text=True).stdout.strip()
        info["cpu"] = q("machdep.cpu.brand_string")
        info["memory_gb"] = round(int(q("hw.memsize")) / 2**30)
    else:
        try:
            info["cpu"] = next(l.split(":", 1)[1].strip() for l in open("/proc/cpuinfo") if l.startswith("model name"))
        except (OSError, StopIteration):
            info["cpu"] = platform.processor()
        try:
            kb = next(int(l.split()[1]) for l in open("/proc/meminfo") if l.startswith("MemTotal"))
            info["memory_gb"] = round(kb / 2**20)
        except (OSError, StopIteration):
            pass
    if label:
        info["label"] = label
    elif os.environ.get("GITHUB_ACTIONS") == "true" and not local:
        info["ci"] = True
        runner = os.environ.get("BENCH_RUNNER") or os.environ.get("RUNNER_NAME", "hosted runner")
        info["label"] = (f"CI runner `{runner}` ({info['cpus']} vCPU, {info.get('memory_gb')} GB, "
                         f"{platform.machine()}, {info['cpu']})")
    else:
        info["label"] = f"local machine ({info['cpu']}, {info['cpus']} cores, {info.get('memory_gb')} GB)"
    return info


def fmt_s(x: float | None) -> str:
    return "n/a" if x is None else f"{x:.2f} s"


def fmt_mem(b: float | None) -> str:
    if b is None:
        return "n/a"
    return f"{b / 2**30:.2f} GiB" if b >= 2**30 else f"{b / 2**20:.0f} MiB"


def fmt_num(x: float | None) -> str:
    return "n/a" if x is None else f"{x:.2f}"


def fmt_ratio(x: float | None) -> str:
    return "n/a" if x is None else f"{x:.2f}x"


def markdown(result: dict) -> str:
    m = result["machine"]
    tv = result["tsgo"]["version"]
    commit = result["tsrs"]["commit"][:12]
    names = ", ".join(result["projects"])
    lines = [
        f"## Benchmark: tsrs vs tsgo {tv}",
        "",
        f"tsrs is a Rust port of the TypeScript 7 type checker (the Go compiler, \"tsgo\"). Each row type-checks one "
        f"project from [microsoft/typescript-benchmarking](https://github.com/microsoft/typescript-benchmarking), the "
        f"suite the TypeScript team benchmarks tsgo on ({names}), with tsgo {tv} (npm `typescript@{tv}`) and with tsrs "
        f"at commit `{commit}`: `tsc -p <project> --noEmit`, median of {result['reps']} interleaved runs, on "
        f"{m['label']}.",
        "",
    ]
    drift = False
    titles = {"default": "Default mode: 4 checker threads in both (tsrs also resolves members lazily, its default)",
              "single": "`--singleThreaded`: one checker thread in both"}
    for mode in result["modes"]:
        lines += [f"**{titles[mode]}**", "",
                  "| project | errors, tsgo / tsrs | tsgo wall (s) | tsrs wall (s) | speedup | tsgo peak memory | "
                  "tsrs peak memory | memory, tsrs / tsgo |",
                  "| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |"]
        for name, pr in result["projects"].items():
            if mode not in pr:
                continue
            g, t = pr[mode]["tsgo"], pr[mode]["tsrs"]
            if not g["ok_runs"] or not t["ok_runs"]:
                failed = [c for c, s in (("tsgo", g), ("tsrs", t)) if not s["ok_runs"]]
                lines.append(f"| {name} | **FAILED: {', '.join(failed)}** | | | | | | |")
                continue
            ge, te = ("/".join(map(str, s["error_counts"])) for s in (g, t))
            err = f"{ge} / {te}"
            ref = pr[mode].get("reference")
            if ref and ref["same_as_tsrs"]:
                err = f"{err} (ref {ref['errors']})"
                drift = True
            elif pr[mode]["errors_match"] is False:
                err = f"**MISMATCH {err}**" + (f" (ref {ref['errors']})" if ref else "")
            elif pr[mode]["error_locations_match"] is False:
                err = f"**{err} (locations differ)**"
            speed = g["wall_s"] / t["wall_s"] if g["wall_s"] and t["wall_s"] else None
            mem = t["peak_rss_bytes"] / g["peak_rss_bytes"] if g["peak_rss_bytes"] and t["peak_rss_bytes"] else None
            lines.append(f"| {name} | {err} | {fmt_num(g['wall_s'])} | {fmt_num(t['wall_s'])} | {fmt_ratio(speed)} | "
                         f"{fmt_mem(g['peak_rss_bytes'])} | {fmt_mem(t['peak_rss_bytes'])} | {fmt_ratio(mem)} |")
        lines.append("")
    lines.append("errors: the number of type errors each compiler reports on the project; they must be equal (a bold "
                 "cell is a disagreement, i.e. a correctness bug). wall: process wall-clock time. speedup: tsgo wall / "
                 "tsrs wall (above 1 = tsrs faster). peak memory: maximum resident set size. memory, tsrs / tsgo: below 1 "
                 "= tsrs uses less.")
    if drift:
        ref = result["reference"]
        lines += ["", f"(ref N): tsgo {tv} and tsrs disagree, but `typescript@{ref['version']}`, built from the "
                      f"TypeScript commit tsrs ports (`{ref['commit'][:8]}`), reports exactly tsrs's errors: a TypeScript "
                      f"7.0 vs 7.1-dev difference, not a tsrs bug."]
    lines += ["", f"Runner: {m['label']}. Date: {result['date']}. tsrs commit: `{commit}`. "
                  + ("Numbers from shared CI machines are noisy; compare trends, not single runs. " if m.get("ci") else "")
                  + "How it is measured: [`bench/README.md`](bench/README.md)."]
    return "\n".join(lines).rstrip() + "\n"


def update_readme(readme: Path, table: str) -> None:
    text = readme.read_text()
    block = f"{START}\n{table}{END}"
    if START in text and END in text:
        pre, rest = text.split(START, 1)
        _, post = rest.split(END, 1)
        text = pre + block + post
    else:
        text = f"{block}\n\n{text}"
    readme.write_text(text)


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--local", action="store_true", help="label results as measured on this machine")
    ap.add_argument("--label", help="machine label for the table header")
    ap.add_argument("--work-dir", type=Path, default=BENCH / ".work", help="clones, installs, logs (default bench/.work)")
    ap.add_argument("--tsrs", type=Path, default=REPO / "target" / "release" / "tsrs")
    ap.add_argument("--tsgo", type=Path, help="native tsgo binary (default: install typescript@<version> from npm)")
    ap.add_argument("--projects", help="comma-separated subset of bench/projects.json")
    ap.add_argument("--modes", default="default,single")
    ap.add_argument("--reps", type=int, default=3)
    ap.add_argument("--timeout", type=float, default=900, help="per-run timeout in seconds")
    ap.add_argument("--no-warmup", action="store_true")
    ap.add_argument("--setup-only", action="store_true")
    ap.add_argument("--out-dir", type=Path, default=BENCH / "results")
    ap.add_argument("--readme", type=Path, help="rewrite the bench block of this README")
    args = ap.parse_args()

    cfg = json.loads((BENCH / "projects.json").read_text())
    projects = cfg["projects"]
    if args.projects:
        want = args.projects.split(",")
        unknown = set(want) - {p["name"] for p in projects}
        if unknown:
            sys.exit(f"unknown projects: {', '.join(sorted(unknown))}")
        projects = [p for p in projects if p["name"] in want]
    modes = args.modes.split(",")
    work = args.work_dir.resolve()

    tsgo = (args.tsgo or ensure_tsgo(cfg["tsgo"], work)).resolve()
    ref_cfg = cfg.get("reference")
    if ref_cfg:
        port_commit = re.search(r'^commit = "([0-9a-f]+)"', (REPO / "Cargo.toml").read_text(), re.M)
        if port_commit and port_commit.group(1) != ref_cfg["commit"]:
            log(f"WARNING: bench/projects.json reference.commit {ref_cfg['commit'][:12]} is not the TypeScript commit "
                f"tsrs ports ({port_commit.group(1)[:12]}); pick the nightly built from it")
    for p in projects:
        setup_project(cfg, p, work)
    if args.setup_only:
        return

    tsrs = args.tsrs.resolve()
    tsrs_version = subprocess.run([str(tsrs), "--version"], capture_output=True, text=True, check=True).stdout.strip()
    commit = subprocess.run(["git", "-C", str(REPO), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    dirty = bool(subprocess.run(["git", "-C", str(REPO), "status", "--porcelain", "--untracked-files=no"],
                                capture_output=True, text=True).stdout.strip())
    now = dt.datetime.now(dt.timezone.utc)
    result: dict = {
        "date": now.strftime("%Y-%m-%d %H:%M UTC"),
        "machine": machine_info(args.local, args.label),
        "tsrs": {"commit": commit, "dirty": dirty, "version": tsrs_version},
        "tsgo": {"version": cfg["tsgo"]["version"], "binary": str(tsgo).replace(str(Path.home()), "~")},
        "suite": cfg["suite"],
        "reference": ref_cfg,
        "reps": args.reps,
        "modes": modes,
        "flags": "--noEmit --incremental false --extendedDiagnostics --pretty false",
        "projects": {},
        "raw": [],
    }
    logs = work / "logs" / now.strftime("%Y%m%d-%H%M%S")
    compilers = {"tsgo": tsgo, "tsrs": tsrs}
    t_start = time.perf_counter()
    for p in projects:
        name = p["name"]
        cwd, proj = project_path(cfg, p, work)
        if not args.no_warmup:
            log(f"{name}: warm-up (tsgo, untimed)")
            run_once(tsgo, cwd, proj, False, logs / f"{name}-warmup.log", args.timeout)
        runs: dict = {m: {"tsgo": [], "tsrs": []} for m in modes}
        for rep in range(args.reps):
            order = ["tsgo", "tsrs"] if rep % 2 == 0 else ["tsrs", "tsgo"]
            for mode in modes:
                for c in order:
                    r = run_once(compilers[c], cwd, proj, mode == "single", logs / f"{name}-{mode}-{c}-{rep}.log",
                                 args.timeout)
                    log(f"{name} {mode:7} {c} rep {rep}: wall {r['wall_s']:.2f} s, check {r.get('check_s')} s, "
                        f"peak {fmt_mem(r['peak_rss_bytes'])}, errors {r['errors']}, exit {r['exit']}")
                    if not r["ok"]:
                        log(f"  FAILED; log: {logs / f'{name}-{mode}-{c}-{rep}.log'}")
                    runs[mode][c].append(r)
                    result["raw"].append({"project": name, "mode": mode, "compiler": c, "rep": rep,
                                          **{k: v for k, v in r.items() if k != "error_keys"}})
        pr: dict = {"commit": p.get("commit") or cfg["suite"]["commit"], "project": p["project"]}
        for mode in modes:
            g, t = summarize(runs[mode]["tsgo"]), summarize(runs[mode]["tsrs"])
            gk = {tuple(r["error_keys"]) for r in runs[mode]["tsgo"] if r["ok"]}
            tk = {tuple(r["error_keys"]) for r in runs[mode]["tsrs"] if r["ok"]}
            match = bool(gk and tk) and g["error_counts"] == t["error_counts"] and len(g["error_counts"]) == 1
            pr[mode] = {"tsgo": g, "tsrs": t, "errors_match": match if gk and tk else None,
                        "error_locations_match": (gk == tk) if gk and tk else None}
            if gk and tk and gk != tk:
                only_g = sorted(set().union(*gk) - set().union(*tk))[:20]
                only_t = sorted(set().union(*tk) - set().union(*gk))[:20]
                pr[mode]["error_diff_sample"] = {"tsgo_only": only_g, "tsrs_only": only_t}
                log(f"{name} {mode}: ERROR SETS DIFFER tsgo-only {only_g[:5]} tsrs-only {only_t[:5]}")
                if ref_cfg:
                    if "ref" not in compilers:
                        compilers["ref"] = ensure_tsgo(ref_cfg, work).resolve()
                    rr = run_once(compilers["ref"], cwd, proj, mode == "single", logs / f"{name}-{mode}-ref.log",
                                  args.timeout)
                    pr[mode]["reference"] = {"ok": rr["ok"], "errors": rr["errors"],
                                             "same_as_tsrs": rr["ok"] and tuple(rr["error_keys"]) in tk}
                    log(f"{name} {mode}: reference {ref_cfg['version']}: {rr['errors']} errors, same as tsrs: "
                        f"{pr[mode]['reference']['same_as_tsrs']}")
        result["projects"][name] = pr
    result["duration_s"] = round(time.perf_counter() - t_start)

    table = markdown(result)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    stem = f"{now.strftime('%Y-%m-%d')}-{commit[:12]}" + ("-local" if args.local else "")
    (args.out_dir / f"{stem}.json").write_text(json.dumps(result, indent=1) + "\n")
    (args.out_dir / f"{stem}.md").write_text(table)
    if args.readme:
        update_readme(args.readme, table)
    print(table)
    log(f"wrote {args.out_dir / stem}.json/.md in {result['duration_s']} s; logs in {logs}")
    bad = [n for n, pr in result["projects"].items() for m in modes
           if (pr[m]["errors_match"] is False and not pr[m].get("reference", {}).get("same_as_tsrs"))
           or not pr[m]["tsgo"]["ok_runs"] or not pr[m]["tsrs"]["ok_runs"]]
    if bad:
        log(f"WARNING: error-count mismatch or failed runs: {', '.join(sorted(set(bad)))}")


if __name__ == "__main__":
    main()
