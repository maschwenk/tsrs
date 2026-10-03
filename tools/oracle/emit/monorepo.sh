#!/usr/bin/env bash
# Emit oracle over a monorepo (read-only): for every package whose package.json `build` script runs `tsc`, run
# `tsgo` and `TSRS_EMIT=1 tsrs` with the same flags, every output path redirected under /tmp, and diff every emitted
# file byte for byte. Prints per-package identical / different / missing (not emitted by tsrs) / extra counts and a
# total. Nothing is ever written into the monorepo: outDir, declarationDir and tsBuildInfoFile are always
# overridden, and `git status --short` of the monorepo is checked before and after.
#
# usage: tools/oracle/emit/monorepo.sh [--root DIR] [--mode emit|noemit|dts] [--filter REGEX] [--jobs N]
#   --mode emit    (default) the package's own options
#   --mode noemit  adds --noEmit: compares only the tsbuildinfo (incremental packages)
#   --mode dts     adds --emitDeclarationOnly --declarationMap false: .d.ts + tsbuildinfo (emit signatures)
# env: TSGO (reference binary), TSRS (tsrs binary; build it with TSRS_TS_VERSION=<tsgo --version> for byte-equal
#      tsbuildinfo `version` fields), OUT (default /tmp; results in $OUT/emit-go, $OUT/emit-rs).
# Packages built with `tsc -b`/`--build` are compiled with -p on their tsconfig (build mode would write next to the
# sources); the report marks them with (b).
set -u
ROOT=/root/Owner
MODE=emit
FILTER=.
JOBS=4
while [ $# -gt 0 ]; do
  case $1 in
    --root) ROOT=$2; shift 2 ;;
    --mode) MODE=$2; shift 2 ;;
    --filter) FILTER=$2; shift 2 ;;
    --jobs) JOBS=$2; shift 2 ;;
    *) echo "unknown argument $1" >&2; exit 2 ;;
  esac
done
TSGO=${TSGO:-/root/bin/tsgo}
TSRS=${TSRS:-$(cd "$(dirname "$0")/../../.." && pwd)/target/release/tsrs}
OUT=${OUT:-/tmp}
# Read the default libraries from tsgo's directory like the npm (noembed) tsgo does (crates/tsrs_cli/src/sys.rs).
export TSRS_LIB_PATH=${TSRS_LIB_PATH:-$(dirname "$(readlink -f "$TSGO")")}
GO_OUT=$OUT/emit-go
RS_OUT=$OUT/emit-rs

before=$(git -C "$ROOT" status --short)

list=$(mktemp)
python3 - "$ROOT" "$FILTER" > "$list" <<'PY'
import json, os, re, subprocess, sys
root, flt = sys.argv[1], sys.argv[2]
files = subprocess.run(["git", "-C", root, "ls-files", "*package.json"], capture_output=True, text=True).stdout.split()
for p in sorted(files):
    if "node_modules" in p:
        continue
    try:
        d = json.load(open(os.path.join(root, p)))
    except Exception:
        continue
    b = d.get("scripts", {}).get("build", "")
    m = re.search(r"(?:^|[\s;&|(])tsc((?:\s+[^\s;&|)]+)*)", b)
    if not m:
        continue
    pkg = os.path.dirname(p) or "."
    if not re.search(flt, pkg):
        continue
    args = m.group(1).split()
    build = any(a in ("-b", "--b", "-build", "--build") for a in args)
    cfg = "tsconfig.json"
    rest = []
    i = 0
    while i < len(args):
        a = args[i]
        if a in ("-p", "--project") and i + 1 < len(args):
            cfg = args[i + 1]; i += 2; continue
        if a in ("-b", "--b", "-build", "--build"):
            i += 1; continue
        if build and not a.startswith("-"):
            cfg = a; i += 1; continue
        if a in ("--verbose", "-v", "--force", "-f"):
            i += 1; continue
        rest.append(a); i += 1
    if os.path.isdir(os.path.join(root, pkg, cfg)):
        cfg = os.path.join(cfg, "tsconfig.json")
    print("\t".join([pkg, cfg, "b" if build else "", " ".join(rest)]))
PY

run_pkg() {
  local pkg=$1 cfg=$2 build=$3 rest=$4
  local name=${pkg//\//__}
  local dir=$ROOT/$pkg
  # Effective options decide which redirects are valid (tsBuildInfoFile needs incremental/composite,
  # declarationDir needs declaration).
  local opts
  opts=$(cd "$dir" && "$TSGO" -p "$cfg" --showConfig 2>/dev/null | python3 -c '
import json,sys
try: c=json.load(sys.stdin).get("compilerOptions",{})
except Exception: c={}
print(int(bool(c.get("incremental") or c.get("composite"))), int(bool(c.get("declaration") or c.get("composite"))), int(bool(c.get("declarationDir"))))')
  local inc=${opts:0:1} decl=${opts:2:1} ddir=${opts:4:1}
  local extra=()
  case $MODE in
    noemit) extra+=(--noEmit) ;;
    dts) extra+=(--emitDeclarationOnly --declarationMap false) ;;
  esac
  for side in go rs; do
    local o=$OUT/emit-$side/$name
    rm -rf "$o" "$o.tsbuildinfo"
    mkdir -p "$o"
    local flags=(-p "$cfg" --outDir "$o")
    [ "$ddir" = 1 ] && flags+=(--declarationDir "$o")
    [ "$inc" = 1 ] && flags+=(--tsBuildInfoFile "$o.tsbuildinfo")
    # shellcheck disable=SC2086
    if [ $side = go ]; then
      (cd "$dir" && "$TSGO" "${flags[@]}" $rest "${extra[@]}" > "$o.stdout" 2>&1; echo "exit $?" >> "$o.stdout")
    else
      (cd "$dir" && TSRS_EMIT=1 timeout 600 "$TSRS" "${flags[@]}" $rest "${extra[@]}" > "$o.stdout" 2>&1; echo "exit $?" >> "$o.stdout")
    fi
    [ -f "$o.tsbuildinfo" ] && cp "$o.tsbuildinfo" "$o/.tsbuildinfo"
  done
  python3 - "$GO_OUT/$name" "$RS_OUT/$name" "$pkg" "$build" <<'PY'
import os, sys
go, rs, pkg, build = sys.argv[1:5]
def files(d):
    out = {}
    for r, _, fs in os.walk(d):
        for f in fs:
            p = os.path.join(r, f)
            out[os.path.relpath(p, d)] = p
    return out
g, r = files(go), files(rs)
ident = diff = missing = 0
first = ""
for k in sorted(g):
    if k not in r:
        missing += 1
        continue
    if open(g[k], "rb").read() == open(r[k], "rb").read():
        ident += 1
    else:
        diff += 1
        first = first or k
extra = len([k for k in r if k not in g])
out_same = open(go + ".stdout", "rb").read() == open(rs + ".stdout", "rb").read()
crash = b"panicked at" in open(rs + ".stdout", "rb").read()
print(f"{pkg}{' (b)' if build else ''}\tidentical {ident}\tdifferent {diff}\tnot-emitted {missing}\textra {extra}\tstdout {'same' if out_same else 'differs'}{'  CRASH' if crash else ''}{'  first-diff ' + first if first else ''}")
PY
}
export -f run_pkg
export ROOT MODE TSGO TSRS OUT GO_OUT RS_OUT

results=$(mktemp)
while IFS=$'\t' read -r pkg cfg build rest; do
  printf '%s\0%s\0%s\0%s\0' "$pkg" "$cfg" "$build" "$rest"
done < "$list" | xargs -0 -n 4 -P "$JOBS" bash -c 'run_pkg "$@"' _ | tee "$results"

python3 - "$results" <<'PY'
import re, sys
t = {"identical": 0, "different": 0, "not-emitted": 0, "extra": 0}
pk = {"all-identical": 0, "packages": 0, "stdout-same": 0, "crash": 0}
for l in open(sys.argv[1]):
    pk["packages"] += 1
    for k in t:
        t[k] += int(re.search(k + r" (\d+)", l).group(1))
    if "different 0" in l and "not-emitted 0" in l and "extra 0" in l:
        pk["all-identical"] += 1
    pk["stdout-same"] += "stdout same" in l
    pk["crash"] += "CRASH" in l
print("TOTAL files: " + ", ".join(f"{v} {k}" for k, v in t.items()))
print("TOTAL packages: " + ", ".join(f"{v} {k}" for k, v in pk.items()))
PY

after=$(git -C "$ROOT" status --short)
if [ "$before" != "$after" ]; then
  echo "ERROR: $ROOT changed during the run" >&2
  exit 1
fi
rm -f "$list" "$results"
