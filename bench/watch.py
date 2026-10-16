#!/usr/bin/env python3
"""Watch-mode benchmark: tsrs --watch against tsgo --watch on one project (bench/README.md "Watch mode").

    python3 bench/watch.py --local                                  # vscode, 3 sessions of each compiler
    python3 bench/watch.py --tsrs <binary> --out-dir <dir>          # the CI job (.github/workflows/bench.yml)

Each session starts `<compiler> --watch -p <project> --noEmit --preserveWatchOutput --pretty false` (no thread
flags) and makes edits to one leaf file (a file nothing imports), each after the previous cycle has settled:

- initial build: from the start of the process to the first "Watching for file changes" report;
- first comment edit: a comment appended to the leaf file;
- repeat comment edit: --edits more comment edits (the median of them);
- type error in a leaf file: an assignment error appended (the cycle must report one more error than the build);
- then the file is restored (its cycle must report the build's errors again; not timed in the table).

An edit's time runs from just before the write to the last report of the cycles it started: the harness waits
--quiet seconds after a report for another cycle (a watch that rebuilds twice for one save is timed to the second).
Memory: the process's peak RSS over the session (`wait4`) and its RSS after the last edit. Sessions alternate the
compiler order; the result holds the medians and every session. The leaf file is restored after every session.

bun check has no watch mode (Bun 1.4.4 canary: no `--watch` flag, and `bun --watch check` checks once).
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import queue
import re
import signal
import subprocess
import sys
import threading
import time
from pathlib import Path

import run

PROJECT = "vscode"
LEAF = {"vscode": "src/vs/base/test/common/strings.test.ts"}
WATCHING_RE = re.compile(r"Watching for file changes")
FOUND_RE = re.compile(r"Found (\d+) errors?")
ERROR_RE = re.compile(r"\berror TS\d+:")
TYPE_ERROR = "\nexport const tsrsBenchWatchError: number = 'not a number';\n"
METRICS = ("initial_s", "first_edit_s", "repeat_edit_s", "type_error_s", "peak_rss_bytes", "final_rss_bytes")


def rss_bytes(pid: int) -> int | None:
    """The process's current resident set size."""
    try:
        with open(f"/proc/{pid}/status") as f:
            return next(int(line.split()[1]) * 1024 for line in f if line.startswith("VmRSS:"))
    except OSError:
        out = subprocess.run(["ps", "-o", "rss=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
        return int(out) * 1024 if out else None


class Session:
    """One watch process; its output lines arrive on a queue with their arrival time."""

    def __init__(self, exe: Path, cwd: Path, proj: Path, log_path: Path):
        argv = [str(exe), "--watch", "-p", str(proj), "--noEmit", "--preserveWatchOutput", "--pretty", "false"]
        log_path.parent.mkdir(parents=True, exist_ok=True)
        self.log = open(log_path, "w")
        self.lines: queue.Queue = queue.Queue()
        self.t0 = time.perf_counter()
        self.proc = subprocess.Popen(argv, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                                     bufsize=1, errors="replace")
        threading.Thread(target=self._pump, daemon=True).start()

    def _pump(self) -> None:
        for line in self.proc.stdout:
            self.lines.put((time.perf_counter(), line))
            self.log.write(line)
        self.lines.put((time.perf_counter(), None))

    def settle(self, since: float, quiet: float, timeout: float) -> dict:
        """Waits for a report, then until `quiet` seconds pass without output. Returns the time from `since` to the
        last report, the number of reports, and the errors the last cycle printed."""
        last = None
        reports = 0
        errors = 0
        found = None
        deadline = time.perf_counter() + timeout
        while True:
            wait = quiet if last is not None else max(0.0, deadline - time.perf_counter())
            try:
                t, line = self.lines.get(timeout=wait)
            except queue.Empty:
                if last is None:
                    raise TimeoutError(f"no watch report within {timeout:g} s")
                return {"s": round(last - since, 3), "reports": reports, "errors": found if found is not None else errors}
            if line is None:
                raise RuntimeError(f"the watch exited (status {self.proc.poll()})")
            if ERROR_RE.search(line):
                errors += 1
            if WATCHING_RE.search(line):
                m = FOUND_RE.search(line)
                found = int(m.group(1)) if m else errors
                last = t
                reports += 1
                errors = 0

    def stop(self) -> int:
        """Ends the watch (SIGINT, then SIGKILL) and returns its peak RSS in bytes."""
        self.proc.send_signal(signal.SIGINT)
        timer = threading.Timer(15, self.proc.kill)
        timer.start()
        _, status, ru = os.wait4(self.proc.pid, 0)
        timer.cancel()
        self.proc.returncode = os.waitstatus_to_exitcode(status)
        self.log.close()
        # ru_maxrss: KiB on Linux, bytes on macOS.
        return ru.ru_maxrss * (1 if sys.platform == "darwin" else 1024)


def session(exe: Path, cwd: Path, proj: Path, leaf: Path, args: argparse.Namespace, log_path: Path) -> dict:
    original = leaf.read_bytes()
    text = original.decode()
    s = Session(exe, cwd, proj, log_path)
    r: dict = {"ok": False}
    try:
        build = s.settle(s.t0, args.quiet, args.timeout)
        r.update(initial_s=build["s"], initial_reports=build["reports"], errors=build["errors"])

        def edit(content: str) -> dict:
            t = time.perf_counter()
            leaf.write_text(content)
            return s.settle(t, args.quiet, args.timeout)

        edits = [edit(text + f"\n// tsrs bench watch edit {i}\n") for i in range(args.edits + 1)]
        r["first_edit_s"] = edits[0]["s"]
        r["repeat_edit_s"] = run.median([e["s"] for e in edits[1:]])
        r["repeat_edits_s"] = [e["s"] for e in edits[1:]]
        bad = edit(text + TYPE_ERROR)
        r["type_error_s"] = bad["s"]
        restored = edit(text)
        r["restore_s"] = restored["s"]
        r["reports"] = [e["reports"] for e in (*edits, bad, restored)]
        r["edit_errors"] = [e["errors"] for e in (*edits, bad, restored)]
        r["final_rss_bytes"] = rss_bytes(s.proc.pid)
        # A cycle that missed the edit, or reported other errors, is not a measurement of the edit.
        r["ok"] = (all(e["errors"] == build["errors"] for e in (*edits, restored))
                   and bad["errors"] == build["errors"] + 1)
    except (TimeoutError, RuntimeError) as e:
        r["failure"] = str(e)
    finally:
        leaf.write_bytes(original)
        r["peak_rss_bytes"] = s.stop()
    return r


def summarize(sessions: list[dict]) -> dict:
    ok = [r for r in sessions if r["ok"]]
    return {"runs": len(sessions), "ok_runs": len(ok), **{k: run.median([r.get(k) for r in ok]) for k in METRICS},
            "errors": run.median([r.get("errors") for r in ok])}


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--local", action="store_true", help="label results as measured on this machine")
    ap.add_argument("--label", help="machine label for the table header")
    ap.add_argument("--work-dir", type=Path, default=run.BENCH / ".work")
    ap.add_argument("--tsrs", type=Path, default=run.REPO / "target" / "release" / "tsrs")
    ap.add_argument("--tsrs-build", choices=sorted(run.TSRS_BUILDS), default="release")
    ap.add_argument("--tsgo", type=Path, help="native tsgo binary (default: install typescript@<version> from npm)")
    ap.add_argument("--project", default=PROJECT, choices=sorted(LEAF))
    ap.add_argument("--reps", type=int, default=3, help="watch sessions of tsrs")
    ap.add_argument("--tsgo-reps", type=int, help="watch sessions of tsgo (default: --reps)")
    ap.add_argument("--edits", type=int, default=5, help="repeat comment edits per session")
    ap.add_argument("--quiet", type=float, default=2.0, help="seconds without output that end a cycle's wait")
    ap.add_argument("--timeout", type=float, default=600, help="seconds to wait for one report")
    ap.add_argument("--rustc", help="`rustc -V` of the toolchain that built --tsrs (default: this machine's)")
    ap.add_argument("--out-dir", type=Path, default=run.BENCH / "results")
    args = ap.parse_args()

    cfg = json.loads((run.BENCH / "projects.json").read_text())
    p = next(p for p in cfg["projects"] if p["name"] == args.project)
    work = args.work_dir.resolve()
    run.setup_project(cfg, p, work)
    cwd, proj = run.project_path(cfg, p, work)
    leaf = cwd / LEAF[args.project]
    tsgo = (args.tsgo or run.ensure_tsgo(cfg["tsgo"], work)).resolve()
    tsrs = args.tsrs.resolve()
    tsgo_reps = args.tsgo_reps if args.tsgo_reps is not None else args.reps
    now = dt.datetime.now(dt.timezone.utc)
    logs = work / "logs" / now.strftime("%Y%m%d-%H%M%S")
    commit = subprocess.run(["git", "-C", str(run.REPO), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
    dirty = bool(subprocess.run(["git", "-C", str(run.REPO), "status", "--porcelain", "--untracked-files=no"],
                                capture_output=True, text=True).stdout.strip())
    tsrs_version = subprocess.run([str(tsrs), "--version"], capture_output=True, text=True, check=True).stdout.strip()
    compilers = {"tsgo": tsgo, "tsrs": tsrs}
    sessions: dict = {c: [] for c in compilers}
    t_start = time.perf_counter()
    for rep in range(max(args.reps, tsgo_reps)):
        for c in (list(compilers) if rep % 2 == 0 else list(reversed(compilers))):
            if rep >= (tsgo_reps if c == "tsgo" else args.reps):
                continue
            r = session(compilers[c], cwd, proj, leaf, args, logs / f"{args.project}-watch-{c}-{rep}.log")
            sessions[c].append(r)
            run.log(f"{args.project} watch {c} rep {rep}: initial {run.fmt_ms(r.get('initial_s'))}, first edit "
                    f"{run.fmt_ms(r.get('first_edit_s'))}, repeat edit {run.fmt_ms(r.get('repeat_edit_s'))}, type error "
                    f"{run.fmt_ms(r.get('type_error_s'))}, peak {run.fmt_mem(r.get('peak_rss_bytes'))}, after the edits "
                    f"{run.fmt_mem(r.get('final_rss_bytes'))}, reports {r.get('reports')}"
                    + ("" if r["ok"] else f"; FAILED: {r.get('failure') or r.get('edit_errors')}"))
    result = {
        "kind": "watch",
        "date": now.strftime("%Y-%m-%d %H:%M UTC"),
        "machine": run.machine_info(args.local, args.label),
        "tsrs": {"commit": commit, "dirty": dirty, "version": tsrs_version, "build": args.tsrs_build,
                 "rustc": args.rustc or run.rustc_version()},
        "tsgo": {"version": cfg["tsgo"]["version"], "binary": str(tsgo).replace(str(Path.home()), "~")},
        "watch": {"project": args.project, "project_path": p["project"], "leaf": LEAF[args.project],
                  "reps": args.reps, "tsgo_reps": tsgo_reps, "edits": args.edits, "quiet_s": args.quiet,
                  "tsgo": summarize(sessions["tsgo"]), "tsrs": summarize(sessions["tsrs"]), "sessions": sessions},
        "duration_s": round(time.perf_counter() - t_start),
    }
    result["watch"]["machine"] = result["machine"]
    args.out_dir.mkdir(parents=True, exist_ok=True)
    stem = f"{result['date'][:10]}-{commit[:12]}-watch" + ("-local" if args.local else "")
    (args.out_dir / f"{stem}.json").write_text(json.dumps(result, indent=1) + "\n")
    print("\n".join(run.watch_markdown(result["watch"])))
    run.log(f"wrote {args.out_dir / stem}.json in {result['duration_s']} s; logs in {logs}")
    if not all(result["watch"][c]["ok_runs"] for c in compilers):
        sys.exit("a compiler has no successful watch session")


if __name__ == "__main__":
    main()
