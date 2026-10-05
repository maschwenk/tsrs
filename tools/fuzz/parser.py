#!/usr/bin/env python3
"""Differential fuzzing of the parser: mutated inputs, parsed by tsgo's parser and by tsrs's, must give the same AST.

  tools/fuzz/parser.py [--rounds N] [--per-round M] [--seed S] [--jobs J] [--out DIR]

Each round takes M files from the conformance corpus (ts-ref/tsc/testdata/tests/cases), mutates them (deletes,
duplicates or swaps spans, inserts tokens that stress the scanner and parser, splices two files, truncates), and
hashes every mutant with both AST oracles (tools/oracle/ast: the Go dumper built from ts-ref, and the Rust
`ast_oracle` example). A different hash is a divergence from Go; a hash missing from the Rust output is a crash or a
timeout. Each finding is saved under --out with the input, both dumps and their diff. Exit status 1 if any finding.
Inputs are kept valid UTF-8: tsrs reads invalid UTF-8 lossily on purpose (Rust strings), where Go keeps the bytes.

Build the oracles first: sh tools/oracle/ast/build.sh
"""

import argparse
import concurrent.futures
import difflib
import os
import random
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
GO = os.path.abspath(os.path.join(ROOT, "..", "bin", "tsrs-oracle-ast"))
RS = os.path.join(ROOT, "target", "parser-integrate", "release", "examples", "ast_oracle")
CORPUS = os.path.join(ROOT, "ts-ref", "tsc", "testdata", "tests", "cases")
EXTENSIONS = (".ts", ".tsx", ".mts", ".cts", ".js", ".jsx")
FLAGS = ("-", "j", "f", "jf")
TIMEOUT = 300

# Fragments that exercise the scanner's and parser's less common paths.
TOKENS = [
    "{", "}", "(", ")", "[", "]", "<", ">", "=>", "?.", "??", "?", ":", ";", ",", ".", "...", "@", "#", "!", "=",
    "`", "${", "/", "/*", "*/", "//", "'", '"', "\\", "\n", "\r\n", " ", "﻿", "​", "\\uD800",
    "<div>", "</div>", "<>", "</>", "{...x}", "<T,>", "<T extends U>", "as const", "satisfies", "readonly",
    "async", "await", "yield", "function*", "class", "enum", "namespace", "declare", "abstract", "accessor",
    "import(", "import.meta", "export =", "export default", "type", "interface", "keyof", "infer", "asserts",
    "unique symbol", "this", "super", "new.target", "0x", "0b1_", "1n", ".5e+", "1__0", "08", "/[a-z]/gu",
    "/(?<n>.)\\k<n>/", "\\u{10FFFF}", "#!", "@ts-ignore", "/** @type {X} */", "/// <reference path=\"x\" />",
    "using", "await using", "out", "in", "const enum", "get", "set", "static {", "#priv", "override",
]


def corpus_files():
    files = []
    for dirpath, _dirs, names in os.walk(CORPUS):
        for name in names:
            if name.endswith(EXTENSIONS):
                files.append(os.path.join(dirpath, name))
    files.sort()
    return files


def read(path):
    with open(path, "rb") as f:
        return f.read().decode("utf-8", "replace")


def mutate(rng, text, other):
    """One to three random edits."""
    for _ in range(rng.randint(1, 3)):
        n = len(text)
        kind = rng.randrange(7)
        i, j = sorted((rng.randint(0, n), rng.randint(0, n)))
        j = min(j, i + rng.randint(1, 200))
        if kind == 0 and n:                      # delete a span
            text = text[:i] + text[j:]
        elif kind == 1 and n:                    # duplicate a span
            text = text[:j] + text[i:j] + text[j:]
        elif kind == 2:                          # insert a token
            text = text[:i] + rng.choice(TOKENS) + text[i:]
        elif kind == 3 and n:                    # swap two spans
            k = rng.randint(0, n)
            a, b = sorted((i, k))
            text = text[:a] + text[b:b + (j - i)] + text[a + (j - i):b] + text[a:a + (j - i)] + text[b + (j - i):]
        elif kind == 4:                          # splice in part of another file
            o = len(other)
            a = rng.randint(0, o)
            text = text[:i] + other[a:a + rng.randint(1, 400)] + text[i:]
        elif kind == 5 and n:                    # truncate
            text = text[:i]
        else:                                    # replace one character
            if n:
                text = text[:i] + rng.choice(TOKENS)[:1] + text[i + 1:]
    return text


def run_hash(binary, lines):
    """{path: hash}; a path missing from the output crashed the process or timed out."""
    try:
        p = subprocess.run([binary, "hash"], input="".join(l + "\n" for l in lines), capture_output=True, text=True,
                           timeout=TIMEOUT, errors="replace")
        out = p.stdout
    except subprocess.TimeoutExpired as e:
        out = e.stdout.decode("utf-8", "replace") if isinstance(e.stdout, bytes) else (e.stdout or "")
    result = {}
    for row in out.splitlines():
        h, _, path = row.partition(" ")
        result[path] = h
    return result


def hash_all(binary, lines, jobs):
    """Shards the list so one crash loses only its shard's tail; reruns missing paths one by one."""
    shards = [lines[k::jobs] for k in range(jobs)]
    result = {}
    with concurrent.futures.ThreadPoolExecutor(jobs) as pool:
        for part in pool.map(lambda s: run_hash(binary, s), shards):
            result.update(part)
    for line in lines:
        path = line.split("\t")[0]
        if path not in result:
            result.update(run_hash(binary, [line]))
    return result


def dump(binary, path, flags):
    try:
        p = subprocess.run([binary, "dump", path, flags], capture_output=True, text=True, timeout=TIMEOUT, errors="replace")
        return p.stdout + (f"\n[exit {p.returncode}]\n{p.stderr[-4000:]}" if p.returncode else "")
    except subprocess.TimeoutExpired:
        return "[timeout]"


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--rounds", type=int, default=5)
    ap.add_argument("--per-round", type=int, default=2000)
    ap.add_argument("--seed", type=int, default=int(time.time()))
    ap.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 2) // 2))
    ap.add_argument("--out", default=os.path.join(ROOT, "target", "fuzz-parser"))
    args = ap.parse_args(argv)
    for b in (GO, RS):
        if not os.path.exists(b):
            sys.exit(f"missing {b}; build the oracles with: sh tools/oracle/ast/build.sh")
    rng = random.Random(args.seed)
    files = corpus_files()
    if not files:
        sys.exit(f"no corpus under {CORPUS}")
    print(f"parser fuzz: seed {args.seed}, {len(files)} corpus files, {args.rounds} rounds of {args.per_round}")
    os.makedirs(args.out, exist_ok=True)
    findings = 0
    total = 0
    compared = 0
    for rnd in range(args.rounds):
        work = tempfile.mkdtemp(prefix="tsrs-fuzz-")
        lines = []
        for k in range(args.per_round):
            src = rng.choice(files)
            text = mutate(rng, read(src), read(rng.choice(files)))
            try:
                text.encode("utf-8")
            except UnicodeEncodeError:
                continue  # invalid UTF-8: tsrs reads it lossily by design (Rust strings are UTF-8); Go keeps the bytes
            ext = os.path.splitext(src)[1]
            path = os.path.join(work, f"m{rnd}_{k}{ext}")
            with open(path, "w", encoding="utf-8") as f:
                f.write(text)
            lines.append(f"{path}\t{rng.choice(FLAGS)}")
        go, rs = hash_all(GO, lines, args.jobs), hash_all(RS, lines, args.jobs)
        for line in lines:
            path, flags = line.split("\t")
            g, r = go.get(path), rs.get(path)
            if g is None:
                continue  # Go itself crashed or timed out: not a port divergence
            compared += 1
            if r == g:
                continue
            findings += 1
            kind = "crash" if r is None else "diverges"
            dest = os.path.join(args.out, f"{args.seed}-{rnd}-{len(os.listdir(args.out))}-{kind}")
            os.makedirs(dest)
            shutil.copy(path, dest)
            gd, rd = dump(GO, path, flags), dump(RS, path, flags)
            with open(os.path.join(dest, "flags"), "w") as f:
                f.write(flags + "\n")
            with open(os.path.join(dest, "diff.txt"), "w") as f:
                f.writelines(difflib.unified_diff(gd.splitlines(True), rd.splitlines(True), "go", "rust", n=2))
            print(f"  {kind}: {dest}")
        total += len(lines)
        shutil.rmtree(work, ignore_errors=True)
        print(f"round {rnd + 1}/{args.rounds}: {total} inputs, {findings} findings")
    print(f"parser fuzz: {total} inputs, {compared} compared, {findings} findings (seed {args.seed})")
    if compared < 0.9 * total:
        sys.exit(f"only {compared} of {total} inputs got a Go hash: is the Go oracle built and current?")
    sys.exit(1 if findings else 0)


if __name__ == "__main__":
    main(sys.argv[1:])
