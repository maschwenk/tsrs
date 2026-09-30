#!/usr/bin/env python3
"""Driver for the printer oracle: prints every file of a list with the Go and the Rust printer (hashes only), in
parallel shards, and reports mismatches (full diffs only for the first few).

  run.py LIST [--mode default|nocomments] [--jobs N] [--diffs K] [--out DIR]

LIST has one "path<TAB>flags" line per file, e.g. the unit list built by the AST oracle's splitter:
  bin/tsrs-oracle-ast split OUTDIR < cases > tests.list      (cases: every file under testdata/tests/cases/{compiler,conformance})
Build first: tools/oracle/printer/build.sh
Files with parse diagnostics are skipped (both sides report SKIP).
"""

import argparse
import concurrent.futures
import difflib
import os
import subprocess
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
GO = os.path.abspath(os.path.join(ROOT, "..", "..", "bin", "tsrs-oracle-printer"))
RS = os.path.join(ROOT, "target", "release", "examples", "printer_oracle")
TIMEOUT = 600


def run_hash(binary, mode, lines):
    try:
        p = subprocess.run([binary, "hash", mode], input="".join(l + "\n" for l in lines), capture_output=True, text=True, timeout=TIMEOUT, errors="replace")
        out = p.stdout
    except subprocess.TimeoutExpired as e:
        out = (e.stdout or b"").decode("utf-8", "replace") if isinstance(e.stdout, bytes) else (e.stdout or "")
    result = {}
    for row in out.splitlines():
        h, _, path = row.partition(" ")
        result[path] = h
    return result


def hash_all(binary, mode, lines, jobs, chunk=200):
    chunks = [lines[i : i + chunk] for i in range(0, len(lines), chunk)]
    result = {}
    with concurrent.futures.ThreadPoolExecutor(jobs) as ex:
        for r in ex.map(lambda c: run_hash(binary, mode, c), chunks):
            result.update(r)
    missing = [l for l in lines if l.split("\t")[0] not in result]
    if missing:
        with concurrent.futures.ThreadPoolExecutor(jobs) as ex:
            for l, r in zip(missing, ex.map(lambda l: run_hash(binary, mode, [l]), missing)):
                path = l.split("\t")[0]
                result[path] = r.get(path, "CRASH")
    return result


def output(binary, mode, line):
    path, _, flags = line.partition("\t")
    try:
        p = subprocess.run([binary, "print", mode, path, flags or "-"], capture_output=True, timeout=TIMEOUT)
        return p.stdout.decode("utf-8", "replace") + p.stderr.decode("utf-8", "replace")[-2000:]
    except subprocess.TimeoutExpired:
        return "TIMEOUT\n"


def show_diff(mode, line, out):
    g = output(GO, mode, line).splitlines(keepends=True)
    r = output(RS, mode, line).splitlines(keepends=True)
    d = list(difflib.unified_diff(g, r, "go", "rust", n=2))
    out.write(f"=== {line}\n")
    out.writelines(d[:60])
    if len(d) > 60:
        out.write(f"... ({len(d) - 60} more diff lines)\n")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("list")
    ap.add_argument("--mode", default="default")
    ap.add_argument("--jobs", type=int, default=os.cpu_count())
    ap.add_argument("--diffs", type=int, default=10)
    ap.add_argument("--out", default=None)
    ap.add_argument("--rust-only", action="store_true", help="reuse OUT/go.<mode>.hashes when present")
    args = ap.parse_args()

    lines = [l.rstrip("\n") for l in open(args.list) if l.strip()]
    out = args.out or os.path.join(ROOT, "target", "scratch", "printer-pkg", os.path.basename(args.list) + ".out")
    os.makedirs(out, exist_ok=True)
    go_cache = os.path.join(out, f"go.{args.mode}.hashes")
    if args.rust_only and os.path.exists(go_cache):
        go = dict(l.rstrip("\n").split(" ", 1)[::-1] for l in open(go_cache))
    else:
        go = hash_all(GO, args.mode, lines, args.jobs)
        with open(go_cache, "w") as f:
            for k, v in go.items():
                f.write(f"{v} {k}\n")
    rs = hash_all(RS, args.mode, lines, args.jobs)

    bad, skipped, go_panics = [], 0, 0
    total = 0
    for l in lines:
        path = l.split("\t")[0]
        if path not in go:
            continue
        if go[path] == "SKIP" and rs.get(path) == "SKIP":
            skipped += 1
            continue
        total += 1
        if go[path] == "PANIC":
            go_panics += 1
        if rs.get(path) != go[path]:
            bad.append(l)
    with open(os.path.join(out, f"mismatches.{args.mode}.list"), "w") as f:
        for l in bad:
            f.write(l + "\n")
    print(f"{args.list} [{args.mode}]: {total - len(bad)}/{total} identical ({skipped} skipped with parse errors, {go_panics} Go panics), {len(bad)} mismatches")
    for l in bad[: args.diffs]:
        show_diff(args.mode, l, sys.stdout)


if __name__ == "__main__":
    main()
