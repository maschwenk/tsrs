#!/usr/bin/env python3
"""Emit oracle (docs/EMIT.md section 12): emit one project with the reference compiler and with tsrs, then compare
every emitted file byte for byte.

    tools/oracle/emit/run.py <tsconfig|dir> [--name N] [--go-out DIR] [--rs-out DIR] [--json] [-- extra tsc flags]

Both compilers run with the same flags; every output path is redirected (`--outDir`, `--declarationDir`,
`--tsBuildInfoFile`), so nothing is ever written into the project. `--incremental false` is not passed: the
reference writes its build info to the redirected path, which lies outside the compared output tree.

Reference binary: $TSGO, else $TSRS_WORK/bin/tsgo-ref. tsrs binary: $TSRS (default target/release/tsrs), run with
TSRS_EMIT=1.

Prints `files: N identical, D different, M missing, X extra` (missing = emitted by the reference only, extra = by
tsrs only), plus diagnostics/exit-code agreement; with --json one JSON object instead. Exit status 1 on any
difference.
"""

import argparse
import difflib
import json
import os
import shutil
import subprocess
import sys

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))


def reference_binary():
    if os.environ.get("TSGO"):
        return os.environ["TSGO"]
    work = os.environ.get("TSRS_WORK")
    if work and os.path.exists(os.path.join(work, "bin", "tsgo-ref")):
        return os.path.join(work, "bin", "tsgo-ref")
    sys.exit("set TSGO to the reference tsgo binary")


def tsrs_binary():
    return os.environ.get("TSRS", os.path.join(REPO, "target", "release", "tsrs"))


def walk(root):
    out = {}
    if not os.path.isdir(root):
        return out
    for d, _, files in os.walk(root):
        for f in files:
            p = os.path.join(d, f)
            out[os.path.relpath(p, root)] = p
    return out


def run(cmd, env, cwd, timeout):
    try:
        p = subprocess.run(cmd, env=env, cwd=cwd, capture_output=True, timeout=timeout)
        return p.returncode, p.stdout.decode("utf-8", "replace") + p.stderr.decode("utf-8", "replace")
    except subprocess.TimeoutExpired:
        return "timeout", ""


def first_diff(a, b, name):
    try:
        ta, tb = a.decode("utf-8").splitlines(), b.decode("utf-8").splitlines()
    except UnicodeDecodeError:
        return "binary difference"
    for i, (x, y) in enumerate(zip(ta, tb)):
        if x != y:
            break
    else:
        i = min(len(ta), len(tb))
    excerpt = "".join(list(difflib.unified_diff(ta, tb, f"ref/{name}", f"rs/{name}", n=2, lineterm="\n"))[:30])
    return f"first difference at line {i + 1}\n{excerpt}"


def main():
    argv = sys.argv[1:]
    extra = []
    if "--" in argv:
        i = argv.index("--")
        argv, extra = argv[:i], argv[i + 1:]
    ap = argparse.ArgumentParser()
    ap.add_argument("project")
    ap.add_argument("--name")
    ap.add_argument("--go-out")
    ap.add_argument("--rs-out")
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--timeout", type=int, default=900)
    ap.add_argument("--quiet", action="store_true")
    # Also compare the tsbuildinfo files (<out>.tsbuildinfo) byte for byte. With tsgo built from ts-ref
    # (`go build ./cmd/tsc`: embedded libs, version 7.1.0-dev) the default tsrs build matches as is. With the npm
    # (noembed) tsgo, tsrs reads the libraries from tsgo's directory (TSRS_LIB_PATH) and must be built with
    # TSRS_TS_VERSION=<tsgo --version>.
    ap.add_argument("--buildinfo", action="store_true")
    args = ap.parse_args(argv)

    project = os.path.abspath(args.project)
    name = args.name or os.path.basename(project.rstrip("/"))
    work = os.path.join(REPO, "target", "scratch", "emit-oracle", name)
    go_out = os.path.abspath(args.go_out or os.path.join(work, "ref"))
    rs_out = os.path.abspath(args.rs_out or os.path.join(work, "rs"))
    for d in (go_out, rs_out):
        shutil.rmtree(d, ignore_errors=True)
        if os.path.exists(d + ".tsbuildinfo"):
            os.remove(d + ".tsbuildinfo")
    cwd = project if os.path.isdir(project) else os.path.dirname(project)

    def flags(out):
        return ["-p", project, "--outDir", out, "--declarationDir", out, "--tsBuildInfoFile", out + ".tsbuildinfo", "--pretty", "false"] + extra

    env = dict(os.environ)
    env.pop("TSRS_EMIT", None)
    go_status, go_text = run([reference_binary()] + flags(go_out), env, cwd, args.timeout)
    env_rs = dict(env, TSRS_EMIT="1")
    # A noembed reference (the npm tsgo: lib.d.ts next to the binary) reads its libraries from disk; match it.
    ref_dir = os.path.dirname(os.path.realpath(reference_binary()))
    if args.buildinfo and "TSRS_LIB_PATH" not in env_rs and os.path.exists(os.path.join(ref_dir, "lib.d.ts")):
        env_rs["TSRS_LIB_PATH"] = ref_dir
    rs_status, rs_text = run([tsrs_binary()] + flags(rs_out), env_rs, cwd, args.timeout)
    rs_text_cmp = rs_text.replace(rs_out, "<OUT>")
    go_text_cmp = go_text.replace(go_out, "<OUT>")

    ref_files, rs_files = walk(go_out), walk(rs_out)
    identical, different, missing, extra_files = [], [], [], []
    details = []
    for rel in sorted(ref_files):
        if rel not in rs_files:
            missing.append(rel)
            continue
        a = open(ref_files[rel], "rb").read()
        b = open(rs_files[rel], "rb").read()
        if a == b:
            identical.append(rel)
        else:
            different.append(rel)
            if len(details) < 5:
                details.append(f"--- {rel}: {first_diff(a, b, rel)}")
    for rel in sorted(rs_files):
        if rel not in ref_files:
            extra_files.append(rel)
    if args.buildinfo:
        rel = os.path.basename(go_out) + ".tsbuildinfo"
        gb, rb = go_out + ".tsbuildinfo", rs_out + ".tsbuildinfo"
        if os.path.exists(gb):
            if not os.path.exists(rb):
                missing.append(rel)
            elif open(gb, "rb").read() == open(rb, "rb").read():
                identical.append(rel)
            else:
                different.append(rel)
                if len(details) < 5:
                    details.append(f"--- {rel}: {first_diff(open(gb, 'rb').read(), open(rb, 'rb').read(), rel)}")
        elif os.path.exists(rb):
            extra_files.append(rel)

    panicked = "panicked at" in rs_text
    result = {
        "name": name,
        "identical": len(identical),
        "different": len(different),
        "missing": len(missing),
        "extra": len(extra_files),
        "ref_status": go_status,
        "rs_status": rs_status,
        "diagnostics_match": go_text_cmp == rs_text_cmp,
        "rs_panic": (next((l.strip() for l in rs_text.splitlines() if "not ported" in l), "") or next((l.strip() for l in rs_text.splitlines() if "panicked at" in l), ""))
        if panicked or rs_status not in (0, 1, 2)
        else "",
    }
    ok = not different and not missing and not extra_files and go_status == rs_status and result["diagnostics_match"]
    if args.json:
        print(json.dumps(result))
    else:
        print(f"files: {len(identical)} identical, {len(different)} different, {len(missing)} missing, {len(extra_files)} extra")
        print(f"exit: ref {go_status}, tsrs {rs_status}; diagnostics {'match' if result['diagnostics_match'] else 'differ'}")
        if result["rs_panic"]:
            print(f"tsrs panic: {result['rs_panic']}")
        if not args.quiet:
            for rel in missing[:10]:
                print(f"missing: {rel}")
            for rel in extra_files[:10]:
                print(f"extra: {rel}")
            for d in details:
                print(d)
            if not result["diagnostics_match"]:
                print("".join(difflib.unified_diff(go_text_cmp.splitlines(True), rs_text_cmp.splitlines(True), "ref", "tsrs", n=1)) [:3000])
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
