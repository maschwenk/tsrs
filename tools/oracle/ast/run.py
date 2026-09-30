#!/usr/bin/env python3
"""Driver for the AST oracle: hashes every file of a list with the Go and the Rust dumper, in parallel
shards, and reports mismatches (full diffs only for the first few).

  run.py LIST [--jobs N] [--diffs K] [--out DIR]

LIST has one "path<TAB>flags" line per file (see tools/oracle/ast/main.go). Build first:
  (cd ts-ref/tsc && GOTOOLCHAIN=auto go build -o ../../../bin/tsrs-oracle-ast ./cmd/tsrs-oracle-ast)
  CARGO_TARGET_DIR=target/parser-integrate cargo build --release -p tsrs_parser --example ast_oracle

Corpus lists:
  run.py --make-tests OUTDIR > tests.list    split every compiler/conformance test case into units
  run.py --make-libs > libs.list
"""

import argparse
import concurrent.futures
import os
import subprocess
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
GO = os.path.abspath(os.path.join(ROOT, "..", "bin", "tsrs-oracle-ast"))
RS = os.path.join(ROOT, "target", "parser-integrate", "release", "examples", "ast_oracle")
TIMEOUT = 600


def run_hash(binary, lines):
    """Returns {path: hash}; paths missing from the output crashed the process (or timed out)."""
    try:
        p = subprocess.run([binary, "hash"], input="".join(l + "\n" for l in lines), capture_output=True, text=True, timeout=TIMEOUT, errors="replace")
        out = p.stdout
    except subprocess.TimeoutExpired as e:
        out = (e.stdout or b"").decode("utf-8", "replace") if isinstance(e.stdout, bytes) else (e.stdout or "")
    result = {}
    for row in out.splitlines():
        h, _, path = row.partition(" ")
        result[path] = h
    return result


def hash_all(binary, lines, jobs, chunk=200):
    chunks = [lines[i : i + chunk] for i in range(0, len(lines), chunk)]
    result = {}
    with concurrent.futures.ThreadPoolExecutor(jobs) as ex:
        for r in ex.map(lambda c: run_hash(binary, c), chunks):
            result.update(r)
    # Re-run files a crashed shard never reported, one per process, to pin down the culprit.
    missing = [l for l in lines if l.split("\t")[0] not in result]
    if missing:
        with concurrent.futures.ThreadPoolExecutor(jobs) as ex:
            for l, r in zip(missing, ex.map(lambda l: run_hash(binary, [l]), missing)):
                path = l.split("\t")[0]
                result[path] = r.get(path, "CRASH")
    return result


def dump(binary, line):
    path, _, flags = line.partition("\t")
    try:
        p = subprocess.run([binary, "dump", path, flags or "-"], capture_output=True, timeout=TIMEOUT)
        return p.stdout.decode("utf-8", "replace") + p.stderr.decode("utf-8", "replace")[-2000:]
    except subprocess.TimeoutExpired:
        return "TIMEOUT\n"


def is_utf8(path):
    with open(path, "rb") as f:
        b = f.read()
    if b[:2] in (b"\xff\xfe", b"\xfe\xff"):
        return True
    try:
        b.decode("utf-8")
        return True
    except UnicodeDecodeError:
        return False


def show_diff(line, out):
    import difflib

    g = dump(GO, line).splitlines(keepends=True)
    r = dump(RS, line).splitlines(keepends=True)
    d = list(difflib.unified_diff(g, r, "go", "rust", n=3))
    out.write(f"=== {line}\n")
    out.writelines(d[:80])
    if len(d) > 80:
        out.write(f"... ({len(d) - 80} more diff lines)\n")


def make_tests(outdir):
    cases = []
    for sub in ("compiler", "conformance"):
        base = os.path.join(ROOT, "ts-ref", "tsc", "testdata", "tests", "cases", sub)
        for dirpath, _, files in os.walk(base):
            for f in sorted(files):
                cases.append(os.path.join(dirpath, f))
    cases.sort()
    p = subprocess.run([GO, "split", os.path.abspath(outdir)], input="".join(c + "\n" for c in cases), capture_output=True, text=True, errors="replace")
    sys.stderr.write(p.stderr)
    sys.stdout.write(p.stdout)


def make_libs():
    base = os.path.join(ROOT, "ts-ref", "tsc", "internal", "bundled", "libs")
    for f in sorted(os.listdir(base)):
        if f.endswith(".d.ts"):
            print(os.path.join(base, f) + "\t-")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("list", nargs="?")
    ap.add_argument("--jobs", type=int, default=os.cpu_count())
    ap.add_argument("--diffs", type=int, default=10)
    ap.add_argument("--out", default=None, help="write mismatching lines to OUT/mismatches.list")
    ap.add_argument("--make-tests")
    ap.add_argument("--make-libs", action="store_true")
    ap.add_argument("--rust-only", action="store_true", help="reuse OUT/go.hashes when present")
    args = ap.parse_args()
    if args.make_tests:
        return make_tests(args.make_tests)
    if args.make_libs:
        return make_libs()

    lines = [l.rstrip("\n") for l in open(args.list) if l.strip()]
    out = args.out or os.path.join(ROOT, "target", "scratch", "parser-integrate", os.path.basename(args.list) + ".out")
    os.makedirs(out, exist_ok=True)
    go_cache = os.path.join(out, "go.hashes")
    if args.rust_only and os.path.exists(go_cache):
        go = dict(l.rstrip("\n").split(" ", 1)[::-1] for l in open(go_cache))
    else:
        go = hash_all(GO, lines, args.jobs)
        with open(go_cache, "w") as f:
            for k, v in go.items():
                f.write(f"{v} {k}\n")
    rs = hash_all(RS, lines, args.jobs)

    bad, panics, crashes, non_utf8 = [], [], [], []
    for l in lines:
        path = l.split("\t")[0]
        if path not in go:
            continue  # unreadable for Go too
        if rs.get(path) == go[path]:
            continue
        if not is_utf8(path):
            non_utf8.append(l)
        elif rs.get(path) == "PANIC":
            panics.append(l)
        elif rs.get(path) in (None, "CRASH"):
            crashes.append(l)
        else:
            bad.append(l)
    total = sum(1 for l in lines if l.split("\t")[0] in go)
    ok = total - len(bad) - len(panics) - len(crashes) - len(non_utf8)
    with open(os.path.join(out, "mismatches.list"), "w") as f:
        for l in panics + crashes + bad:
            f.write(l + "\n")
    with open(os.path.join(out, "non_utf8.list"), "w") as f:
        for l in non_utf8:
            f.write(l + "\n")
    print(f"{args.list}: {ok}/{total} identical, {len(bad)} mismatches, {len(panics)} panics, {len(crashes)} crashes, {len(non_utf8)} non-UTF-8 mismatches")
    for l in (panics + crashes + bad)[: args.diffs]:
        show_diff(l, sys.stdout)


if __name__ == "__main__":
    main()
