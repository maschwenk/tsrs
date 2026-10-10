#!/usr/bin/env python3
"""Emit oracle over conformance test cases, through the CLIs: tsgo vs tsrs (both emit by default).

    tools/oracle/emit/cases.py [--jobs N] [--show] <test file or directory>...

Each test case is split at `// @filename:` like the harness does, written to a scratch directory, and compiled by
both compilers with the case's `// @option: value` directives as command-line flags (every combination of
comma-separated values, as the harness's variants) plus `--outDir ../go` (tsgo) or `--outDir ../rs` (tsrs) and
`--pretty false`. Every emitted file is compared byte for byte. This is a stand-in for `tsrs-test --baselines js` (E2): it compares against the reference compiler instead of
the stored baselines, and harness-only directives are ignored.

Environment: TSGO (reference tsgo built from ts-ref, required), TSRS (binaries), OUT (scratch root, default /tmp/emit-cases).
"""
import concurrent.futures
import itertools
import os
import re
import shutil
import subprocess
import sys

TSGO = os.environ["TSGO"]
TSRS = os.environ.get("TSRS", os.path.join(os.path.dirname(__file__), "../../../target/release/tsrs"))
OUT = os.environ.get("OUT", "/tmp/emit-cases")

# Harness directives that are not compiler options (testutil/harnessutil).
HARNESS = {"filename", "allowNonTsExtensions", "useCaseSensitiveFileNames", "baselineFile", "includeBuiltFile", "fileName", "libFiles",
           "noErrorTruncation", "suppressOutputPathCheck", "noImplicitReferences", "currentDirectory", "symlink", "link", "noTypesAndSymbols",
           "fullEmitPaths", "noCheck", "reportDiagnostics", "captureSuggestions", "typeScriptVersion", "skipDefaultLibCheck", "outFile", "out"}
HARNESS_LOWER = {h.lower() for h in HARNESS}
OPT_RE = re.compile(r"^\s*//\s*@(\w+)\s*:\s*(.*?)\s*$")


def parse(path):
    text = open(path, encoding="utf-8", errors="surrogateescape").read()
    opts = {}
    files = []
    cur_name, cur_lines = None, []
    base = os.path.basename(path)
    for line in text.splitlines(keepends=True):
        m = OPT_RE.match(line)
        if m:
            key, value = m.group(1), m.group(2)
            if key.lower() == "filename":
                if cur_name is not None or any(l.strip() for l in cur_lines):
                    files.append((cur_name or base, "".join(cur_lines)))
                cur_name, cur_lines = value, []
                continue
            opts[key] = value
            continue
        cur_lines.append(line)
    files.append((cur_name or base, "".join(cur_lines)))
    return opts, files


def variants(opts):
    keys, values = [], []
    for k, v in opts.items():
        if k.lower() in HARNESS_LOWER:
            continue
        vs = [x.strip() for x in v.split(",")] if "," in v and not k.lower() in ("lib", "types", "paths", "rootdirs", "typeroots") else [v]
        keys.append(k)
        values.append(vs)
    for combo in itertools.product(*values):
        yield dict(zip(keys, combo))


def flags(variant):
    out = []
    for k, v in variant.items():
        if k.lower() in ("paths", "plugins"):
            continue
        out += ["--" + k, v]
    return out


def run(compiler, cwd, args, env=None):
    try:
        p = subprocess.run([compiler] + args, cwd=cwd, capture_output=True, timeout=60, env=env)
        return p.returncode, p.stdout + p.stderr
    except subprocess.TimeoutExpired:
        return -1, b"timeout"


def tree(root):
    res = {}
    for d, _, fs in os.walk(root):
        for f in fs:
            p = os.path.join(d, f)
            res[os.path.relpath(p, root)] = open(p, "rb").read()
    return res


def one(path):
    name = os.path.splitext(os.path.basename(path))[0]
    opts, files = parse(path)
    results = []
    for i, variant in enumerate(variants(opts)):
        work = os.path.join(OUT, f"{name}.{abs(hash(path)) % 10000}.{i}")
        shutil.rmtree(work, ignore_errors=True)
        src = os.path.join(work, "src")
        roots = []
        for fname, content in files:
            p = os.path.normpath(os.path.join(src, fname.lstrip("/")))
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, "w", encoding="utf-8", errors="surrogateescape") as f:
                f.write(content)
            if not fname.endswith(".json") or fname.endswith("tsconfig.json"):
                roots.append(os.path.relpath(p, src))
        roots = [r for r in roots if not r.endswith("tsconfig.json")]
        args = flags(variant) + roots
        rc1, out1 = run(TSGO, src, ["--outDir", "../go", "--pretty", "false"] + args)
        rc2, out2 = run(TSRS, src, ["--outDir", "../rs", "--pretty", "false"] + args)
        go, rs = tree(os.path.join(work, "go")), tree(os.path.join(work, "rs"))
        if b"panicked" in out2 or rc2 < 0 or rc2 > 2:
            status = "crash"
        elif go == rs:
            status = "pass" if go else "empty"
        else:
            status = "fail"
        detail = ""
        if status == "crash":
            m = re.search(rb"panicked at ([^\n]*)\n([^\n]*)", out2)
            detail = (m.group(1) + b" " + m.group(2)).decode(errors="replace") if m else out2[-200:].decode(errors="replace")
        elif status == "fail":
            diff = sorted(set(go) ^ set(rs)) or [k for k in sorted(go) if go[k] != rs.get(k)]
            detail = ",".join(diff[:3])
        if status in ("pass", "empty"):
            shutil.rmtree(work, ignore_errors=True)
        results.append((f"{name}.{i}" if i else name, status, detail))
    return results


def main():
    args = sys.argv[1:]
    jobs = os.cpu_count() or 4
    show = False
    if "--jobs" in args:
        i = args.index("--jobs"); jobs = int(args[i + 1]); del args[i:i + 2]
    if "--show" in args:
        args.remove("--show"); show = True
    paths = []
    for a in args:
        if os.path.isdir(a):
            for d, _, fs in os.walk(a):
                paths += [os.path.join(d, f) for f in fs if re.search(r"\.(ts|tsx|js|jsx|mts|cts)$", f)]
        else:
            paths.append(a)
    paths.sort()
    counts = {}
    with concurrent.futures.ThreadPoolExecutor(jobs) as ex:
        for res in ex.map(one, paths):
            for name, status, detail in res:
                counts[status] = counts.get(status, 0) + 1
                if show or status not in ("pass", "empty"):
                    print(f"{status:6} {name} {detail}")
    total = sum(counts.values())
    print("variants: " + ", ".join(f"{k} {v}" for k, v in sorted(counts.items())) + f", total {total}")


if __name__ == "__main__":
    main()
