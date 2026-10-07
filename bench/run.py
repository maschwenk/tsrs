#!/usr/bin/env python3
"""Benchmarks tsrs against tsgo (typescript@7.0.2) on the typescript-benchmarking projects.

See bench/README.md. Typical use:

    python3 bench/run.py --local                       # everything in bench/projects.json
    python3 bench/run.py --local --projects webpack,Compiler --reps 1
    python3 bench/run.py --setup-only                  # clone + install only (CI cache warm-up)
    python3 bench/run.py --projects vscode --out-dir /tmp/p/vscode   # one project of a parallel run (CI)
    python3 bench/run.py --merge /tmp/p/*/*.json --readme README.md  # join such results into one
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
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
from collections import Counter
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
BENCH = REPO / "bench"
MARKER = ".tsrs-bench.json"
START, END = "<!-- bench:start -->", "<!-- bench:end -->"
# --modes: extra compiler flags per mode. "default" passes none (each compiler picks its own checker count), "single"
# is one checker thread, "checkers8" gives both compilers 8 checker threads (the scaling comparison).
MODE_FLAGS = {
    "default": [],
    "single": ["--singleThreaded"],
    "checkers8": ["--checkers", "8"],
}
# --tsrs-build: how the measured tsrs binary was built, for the table header.
TSRS_BUILDS = {
    "release": "`cargo build --release`",
    "pgo-dist": "the PGO-optimized `dist` build, built like the npm release binaries",
}

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


def retry(what: str, fn, attempts: int = 3) -> None:
    """Network steps (clone, package installs) get a few attempts: the CI job runs unattended."""
    for i in range(1, attempts + 1):
        try:
            return fn()
        except subprocess.CalledProcessError as e:
            if i == attempts:
                raise
            log(f"{what}: attempt {i} failed ({e}); retrying in {10 * i} s")
            time.sleep(10 * i)


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


def overlay_files(p: dict) -> list[tuple[str, bytes]]:
    """The files of a project's `overlay` directory (bench/overlays/<name>), as (relative path, content)."""
    if "overlay" not in p:
        return []
    root = REPO / p["overlay"]
    files = sorted((f.relative_to(root).as_posix(), f.read_bytes()) for f in root.rglob("*") if f.is_file())
    if not files:
        sys.exit(f"{p['name']}: overlay {p['overlay']} has no files")
    return files


def overlay_digest(p: dict) -> str | None:
    """Content hash of a project's overlay; part of its checkout marker and CI cache key, so editing it re-installs."""
    files = overlay_files(p)
    if not files:
        return None
    h = hashlib.sha256()
    for rel, data in files:
        h.update(rel.encode() + b"\0" + hashlib.sha256(data).digest())
    return h.hexdigest()[:16]


def ensure_checkout(name: str, repo: str, commit: str, install: str | None, dest: Path, p: dict | None = None) -> None:
    want = {"repo": repo, "commit": commit, "install": install}
    overlay = overlay_files(p) if p else []
    if overlay:
        want["overlay"] = overlay_digest(p)
    if read_marker(dest) == want:
        log(f"{name}: cached checkout {commit[:12]}")
        return
    log(f"{name}: setting up {repo} @ {commit[:12]}")
    t0 = time.perf_counter()

    def attempt() -> None:
        git_checkout(repo, commit, dest)
        # Overlay files replace or add checkout files before the install (bench/README.md "Application projects").
        for rel, data in overlay:
            (dest / rel).parent.mkdir(parents=True, exist_ok=True)
            (dest / rel).write_bytes(data)
        if install:
            sh(install, cwd=dest, env=dict(os.environ, CI="true", HUSKY="0"))

    retry(name, attempt)
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
        ensure_checkout(p["name"], p["repo"], p["commit"], p.get("install"), work / "solutions" / p["name"], p)


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
        retry(f"{pkg}@{version}", lambda: sh(["npm", "install", "--no-audit", "--no-fund", "--no-save", f"{pkg}@{version}"], cwd=d))
    with open(exe, "rb") as f:
        magic = f.read(4)
    if magic not in (b"\x7fELF", b"\xcf\xfa\xed\xfe", b"\xca\xfe\xba\xbe"):
        sys.exit(f"{exe} is not a native executable (magic {magic!r})")
    out = subprocess.run([str(exe), "--version"], capture_output=True, text=True, check=True).stdout.strip()
    if not version.startswith(out.removeprefix("Version ")):
        sys.exit(f"{exe} --version printed {out!r}, expected 'Version {version}'")
    return exe


def run_once(exe: Path, cwd: Path, proj: Path, mode: str, log_path: Path, timeout: float) -> dict:
    argv = [str(exe), "-p", str(proj), "--noEmit", "--incremental", "false", "--extendedDiagnostics", "--pretty", "false"]
    argv += MODE_FLAGS[mode]
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


def count_instructions(exe: Path, cwd: Path, proj: Path, log_path: Path, timeout: float) -> dict | None:
    """User-space instructions and peak RSS of one single-threaded type check (bench/count.py), or None where it cannot
    count.

    Untimed and separate from the timed runs. One thread (`--singleThreaded`, and RAYON_NUM_THREADS=1 for the parse
    pool) makes the count reproducible to about 0.001%, so one run is enough."""
    if sys.platform != "linux":
        return None
    out = log_path.with_suffix(".json")
    argv = [str(exe), "-p", str(proj), "--noEmit", "--incremental", "false", "--singleThreaded", "--pretty", "false"]
    log_path.parent.mkdir(parents=True, exist_ok=True)
    with open(log_path, "wb") as log_file:
        subprocess.run([sys.executable, str(BENCH / "count.py"), str(out), "--", *argv], cwd=cwd, stdout=log_file,
                       stderr=subprocess.STDOUT, env=dict(os.environ, RAYON_NUM_THREADS="1"), timeout=timeout)
    r = json.loads(out.read_text())
    if r.get("exit") not in (0, 1, 2) or r.get("instructions") is None:
        return None
    return {"instructions": r["instructions"], "max_rss_bytes": r["max_rss_bytes"]}


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


def rustc_version() -> str | None:
    """`rustc -V` in the checkout: the toolchain rust-toolchain.toml pins, which the bench workflow builds with."""
    try:
        return subprocess.run(["rustc", "-V"], capture_output=True, text=True, cwd=REPO, check=True).stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return None


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
        libc, version = platform.libc_ver()
        if libc:
            info["libc"] = f"{libc} {version}"
    if label:
        info["label"] = label
    elif os.environ.get("GITHUB_ACTIONS") == "true" and not local:
        info["ci"] = True
        runner = os.environ.get("BENCH_RUNNER") or os.environ.get("RUNNER_NAME", "hosted runner")
        info["label"] = (f"{runner} ({info['cpus']} vCPU, {info.get('memory_gb')} GB RAM, {platform.system()} "
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
    suite_names = [n for n, pr in result["projects"].items() if pr.get("source") != "application"]
    app_names = [n for n, pr in result["projects"].items() if pr.get("source") == "application"]
    sources = []
    if suite_names:
        sources.append(f"[microsoft/typescript-benchmarking](https://github.com/microsoft/typescript-benchmarking), "
                       f"the suite the TypeScript team benchmarks tsgo on ({', '.join(suite_names)})")
    if app_names:
        sources.append(f"a set of large open-source applications ({', '.join(app_names)})")
    lines = [
        f"## Benchmark: tsrs vs tsgo {tv}",
        "",
        f"tsrs is a Rust port of the TypeScript 7 type checker (the Go compiler, \"tsgo\"). Each row type-checks one "
        f"project from {', or from '.join(sources)}, with tsgo {tv} (npm `typescript@{tv}`) and with tsrs "
        f"at commit `{commit}` ({TSRS_BUILDS[result['tsrs'].get('build', 'release')]}): `tsc -p <project> --noEmit`, "
        f"median of {result['reps']} interleaved runs, on {m['label']}.",
        "",
    ]
    drift = False
    # tsrs's default checker count (checkerpool.rs default_checker_count; the small-program floor does not bind here).
    tsrs_default = max(4, min(32, (m.get("cpus") or 1) // 2))
    titles = {"default": f"Default mode: no thread flag; tsgo uses 4 checker threads, tsrs half the cores clamped to 4..32 "
                         f"({tsrs_default} here; tsrs also resolves members lazily, its default)",
              "single": "`--singleThreaded`: one checker thread in both",
              "checkers8": "`--checkers 8`: 8 checker threads in both (how each compiler scales with more checkers)"}
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
    if {"single", "checkers8"} <= set(result["modes"]):
        lines += ["", "**Scaling: wall time of `--singleThreaded` / wall time of `--checkers 8`, per compiler**", "",
                  "| project | tsgo | tsrs | tsrs scaling / tsgo scaling | tsgo memory, 8 checkers / 1 | "
                  "tsrs memory, 8 checkers / 1 |",
                  "| --- | ---: | ---: | ---: | ---: | ---: |"]
        for name, pr in result["projects"].items():
            if "single" not in pr or "checkers8" not in pr:
                continue
            sg, st, eg, et = (pr[m][c] for m in ("single", "checkers8") for c in ("tsgo", "tsrs"))
            if not all(s["ok_runs"] for s in (sg, st, eg, et)):
                continue
            gs, ts = sg["wall_s"] / eg["wall_s"], st["wall_s"] / et["wall_s"]
            lines.append(f"| {name} | {fmt_ratio(gs)} | {fmt_ratio(ts)} | {fmt_ratio(ts / gs)} | "
                         f"{fmt_ratio(eg['peak_rss_bytes'] / sg['peak_rss_bytes'])} | "
                         f"{fmt_ratio(et['peak_rss_bytes'] / st['peak_rss_bytes'])} |")
        lines += ["", "scaling: how many times faster a compiler gets going from one checker thread to eight; "
                      "memory: peak memory with 8 checkers divided by peak memory with one."]
    if drift:
        ref = result["reference"]
        lines += ["", f"(ref N): tsgo {tv} and tsrs disagree, but `typescript@{ref['version']}`, built from the "
                      f"TypeScript commit tsrs ports (`{ref['commit'][:8]}`), reports exactly tsrs's errors: a TypeScript "
                      f"7.0 vs 7.1-dev difference, not a tsrs bug."]
    if mixed := [n for n, pr in result["projects"].items() if "machine" in pr]:
        lines += ["", "Not measured on the machine named below (a parallel run landed on more than one machine model): "
                      + "; ".join(f"{n} on {result['projects'][n]['machine']['label']}" for n in mixed) + "."]
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


def merge_results(cfg: dict, paths: list[Path]) -> dict:
    """One result from the per-project results of a parallel run (.depot/workflows/bench.yml: one measuring job per
    project, each `run.py --projects <name> --out-dir <dir>`), in bench/projects.json order.

    The binary, the compilers, reps and modes must agree. The machines need not: the machine most projects ran on is
    the run's (ties: the first project's), and a project measured on another records its own `machine`
    (bench/regressions.py then compares its counts only with runs on the same CPU model and C library)."""
    order = {p["name"]: i for i, p in enumerate(cfg["projects"])}
    rank = lambda names: min((order.get(n, len(order)) for n in names), default=len(order))
    partials = sorted((json.loads(p.read_text()) for p in paths), key=lambda r: rank(r["projects"]))
    base = partials[0]
    fixed = lambda r: {"tsrs": r["tsrs"], "tsgo": r["tsgo"]["version"], "suite": r["suite"],
                       "reference": r.get("reference"), "reps": r["reps"], "modes": r["modes"], "flags": r["flags"]}
    for r in partials[1:]:
        for k, v in fixed(base).items():
            if fixed(r)[k] != v:
                sys.exit(f"--merge: {k} differs: {json.dumps(v)} ({', '.join(base['projects'])}) vs "
                         f"{json.dumps(fixed(r)[k])} ({', '.join(r['projects'])})")
    # most_common keeps insertion order among equal counts, so a tie goes to the first project's machine.
    machine = json.loads(Counter(json.dumps(r["machine"], sort_keys=True) for r in partials).most_common(1)[0][0])
    result: dict = {"date": dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%d %H:%M UTC"), "machine": machine,
                    **{k: base.get(k) for k in ("tsrs", "tsgo", "suite", "reference", "reps", "modes", "flags")},
                    "projects": {}, "raw": [], "partials": [], "duration_s": 0}
    for r in partials:
        for name, pr in r["projects"].items():
            if name in result["projects"]:
                sys.exit(f"--merge: {name} is in two results")
            # The table header names the run's machine; a project measured elsewhere says so.
            result["projects"][name] = dict(pr, machine=r["machine"]) if r["machine"] != machine else pr
        result["raw"] += r["raw"]
        result["partials"].append({"projects": list(r["projects"]), "date": r["date"], "duration_s": r.get("duration_s")})
        result["duration_s"] += r.get("duration_s") or 0
    result["projects"] = dict(sorted(result["projects"].items(), key=lambda kv: order.get(kv[0], len(order))))
    result["raw"].sort(key=lambda row: order.get(row["project"], len(order)))
    if mixed := [n for n, pr in result["projects"].items() if "machine" in pr]:
        log(f"WARNING: not measured on the run's machine ({machine['label']}): "
            + "; ".join(f"{n} on {result['projects'][n]['machine']['label']}" for n in mixed))
    if missing := [p["name"] for p in cfg["projects"] if p["name"] not in result["projects"]]:
        log(f"WARNING: no result for {', '.join(missing)}")
    return result


def write_results(result: dict, args: argparse.Namespace, note: str = "") -> None:
    """<out-dir>/<date>-<commit>[-local].{json,md}, the README block with --readme, and the table on stdout."""
    table = markdown(result)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    stem = f"{result['date'][:10]}-{result['tsrs']['commit'][:12]}" + ("-local" if args.local else "")
    (args.out_dir / f"{stem}.json").write_text(json.dumps(result, indent=1) + "\n")
    (args.out_dir / f"{stem}.md").write_text(table)
    if args.readme:
        update_readme(args.readme, table)
    print(table)
    log(f"wrote {args.out_dir / stem}.json/.md in {result['duration_s']} s" + (f"; {note}" if note else ""))
    bad = [n for n, pr in result["projects"].items() for m in result["modes"]
           if (pr[m]["errors_match"] is False and not pr[m].get("reference", {}).get("same_as_tsrs"))
           or not pr[m]["tsgo"]["ok_runs"] or not pr[m]["tsrs"]["ok_runs"]]
    if bad:
        log(f"WARNING: error-count mismatch or failed runs: {', '.join(sorted(set(bad)))}")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--local", action="store_true", help="label results as measured on this machine")
    ap.add_argument("--label", help="machine label for the table header")
    ap.add_argument("--work-dir", type=Path, default=BENCH / ".work", help="clones, installs, logs (default bench/.work)")
    ap.add_argument("--tsrs", type=Path, default=REPO / "target" / "release" / "tsrs")
    ap.add_argument("--tsrs-build", choices=sorted(TSRS_BUILDS), default="release",
                    help="how --tsrs was built (named in the table header; CI: pgo-dist, see .depot/workflows/bench.yml)")
    ap.add_argument("--tsgo", type=Path, help="native tsgo binary (default: install typescript@<version> from npm)")
    ap.add_argument("--projects", help="comma-separated subset of bench/projects.json")
    ap.add_argument("--modes", default="default,single,checkers8", help="comma-separated: " + ", ".join(MODE_FLAGS))
    ap.add_argument("--reps", type=int, default=3)
    ap.add_argument("--timeout", type=float, default=900, help="per-run timeout in seconds")
    ap.add_argument("--no-warmup", action="store_true")
    ap.add_argument("--no-instructions", action="store_true",
                    help="skip the untimed instruction-count run of tsrs (Linux only; bench/regressions.py compares it)")
    ap.add_argument("--setup-only", action="store_true", help="clone and install everything, including the reference compiler")
    ap.add_argument("--print-cache-keys", action="store_true",
                    help="print `<name>=<key>` lines (GITHUB_OUTPUT format) for the CI caches of bench/.work")
    ap.add_argument("--print-projects", action="store_true",
                    help="print the project names as a JSON list (the CI matrix: one measuring job per project)")
    ap.add_argument("--rustc", help="`rustc -V` of the toolchain that built --tsrs (default: this machine's; the CI "
                                    "measuring jobs have no Rust)")
    ap.add_argument("--merge", type=Path, nargs="+", metavar="RESULT",
                    help="only join these per-project results of one run into one result file (no benchmarking)")
    ap.add_argument("--out-dir", type=Path, default=BENCH / "results")
    ap.add_argument("--readme", type=Path, help="rewrite the bench block of this README")
    ap.add_argument("--apply-table", type=Path,
                    help="only rewrite --readme's bench block from this results .md (no benchmarking)")
    args = ap.parse_args()
    if args.apply_table:
        if not args.readme:
            ap.error("--apply-table needs --readme")
        update_readme(args.readme, args.apply_table.read_text())
        return

    cfg = json.loads((BENCH / "projects.json").read_text())
    projects = cfg["projects"]
    if args.projects:
        want = args.projects.split(",")
        unknown = set(want) - {p["name"] for p in projects}
        if unknown:
            sys.exit(f"unknown projects: {', '.join(sorted(unknown))}")
        projects = [p for p in projects if p["name"] in want]
    if args.print_projects:
        print(json.dumps([p["name"] for p in projects]))
        return
    if args.merge:
        write_results(merge_results(cfg, args.merge), args)
        return
    modes = args.modes.split(",")
    if unknown_modes := set(modes) - set(MODE_FLAGS):
        sys.exit(f"unknown modes: {', '.join(sorted(unknown_modes))}")
    work = args.work_dir.resolve()
    if args.print_cache_keys:
        digest = lambda x: hashlib.sha256(json.dumps(x, sort_keys=True).encode()).hexdigest()[:16]
        # bench/.work/{suite,tsgo}: the suite checkout (Compiler, Compiler-Unions) and the npm compilers.
        print(f"shared={digest([cfg['suite'], cfg['tsgo'], cfg.get('reference'), native_platform()])}")
        # bench/.work/solutions/<name>: one cache per cloned project, keyed on its commit and install command.
        for p in projects:
            if "repo" in p:
                key = [p["repo"], p["commit"], p.get("install")] + ([overlay_digest(p)] if "overlay" in p else [])
                print(f"{p['name']}={p['commit'][:12]}-{digest(key)}")
        return

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
        if ref_cfg:
            ensure_tsgo(ref_cfg, work)
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
        "tsrs": {"commit": commit, "dirty": dirty, "version": tsrs_version, "build": args.tsrs_build,
                 "rustc": args.rustc or rustc_version()},
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
            run_once(tsgo, cwd, proj, "default", logs / f"{name}-warmup.log", args.timeout)
        runs: dict = {m: {"tsgo": [], "tsrs": []} for m in modes}
        for rep in range(args.reps):
            order = ["tsgo", "tsrs"] if rep % 2 == 0 else ["tsrs", "tsgo"]
            for mode in modes:
                for c in order:
                    r = run_once(compilers[c], cwd, proj, mode, logs / f"{name}-{mode}-{c}-{rep}.log", args.timeout)
                    log(f"{name} {mode:7} {c} rep {rep}: wall {r['wall_s']:.2f} s, check {r.get('check_s')} s, "
                        f"peak {fmt_mem(r['peak_rss_bytes'])}, errors {r['errors']}, exit {r['exit']}")
                    if not r["ok"]:
                        log(f"  FAILED; log: {logs / f'{name}-{mode}-{c}-{rep}.log'}")
                    runs[mode][c].append(r)
                    result["raw"].append({"project": name, "mode": mode, "compiler": c, "rep": rep,
                                          **{k: v for k, v in r.items() if k != "error_keys"}})
        pr: dict = {"commit": p.get("commit") or cfg["suite"]["commit"], "project": p["project"],
                    "source": "suite" if "case" in p else "application"}
        if "overlay" in p:
            pr["overlay"] = overlay_digest(p)
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
                    rr = run_once(compilers["ref"], cwd, proj, mode, logs / f"{name}-{mode}-ref.log", args.timeout)
                    pr[mode]["reference"] = {"ok": rr["ok"], "errors": rr["errors"],
                                             "same_as_tsrs": rr["ok"] and tuple(rr["error_keys"]) in tk}
                    log(f"{name} {mode}: reference {ref_cfg['version']}: {rr['errors']} errors, same as tsrs: "
                        f"{pr[mode]['reference']['same_as_tsrs']}")
        if "single" in modes and not args.no_instructions:
            counted = count_instructions(tsrs, cwd, proj, logs / f"{name}-instructions-tsrs.log", args.timeout)
            if counted is not None:
                pr["single"]["tsrs"].update(counted)
                log(f"{name} single  tsrs: {counted['instructions'] / 1e9:.3f} G instructions, peak "
                    f"{fmt_mem(counted['max_rss_bytes'])} (one thread, untimed)")
        result["projects"][name] = pr
    result["duration_s"] = round(time.perf_counter() - t_start)
    write_results(result, args, f"logs in {logs}")


if __name__ == "__main__":
    main()
