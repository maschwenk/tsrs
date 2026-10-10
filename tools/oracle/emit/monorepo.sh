#!/usr/bin/env bash
# Emit oracle over a pnpm monorepo (docs/EMIT.md section 12): for every workspace package whose `build` script runs
# `tsc`, emit with the reference compiler into /tmp/emit-go/<pkg> and with tsrs into /tmp/emit-rs/<pkg>
# (same flags; outDir, declarationDir and tsBuildInfoFile always redirected), then diff every emitted file byte for
# byte (tools/oracle/emit/run.py). The monorepo itself is never written: its `git status --short` is checked before
# and after, and the script fails if it changed.
#
#   tools/oracle/emit/monorepo.sh <monorepo root> [-j N] [--filter REGEX] [--buildinfo] [-- extra flags for both compilers]
#
# --buildinfo also compares the tsbuildinfo files (run.py --buildinfo; use tsgo built from ts-ref, see run.py).
# With `-- --noEmit` that is the incremental-only mode (tsbuildinfo of a noEmit program).
#
# Env: TSGO (reference tsgo binary, required), TSRS (default target/release/tsrs), TSRS_CHECKER_ASSIGNMENT (default
# go), OUT_GO (/tmp/emit-go),
# OUT_RS (/tmp/emit-rs). A package built with `tsc -p <file>` uses that config; `tsc --build`/`-b` packages are
# emitted with `-p` on their tsconfig (this script has no build mode; tsrs itself supports `-b`).
# Output: one line per selected package (verdict, identical/different/missing/extra counts, exit codes, diagnostics
# agreement, tsrs panic) and totals (summarize.py); JSON rows in $OUT_RS.results.jsonl. Exit status: 0 only if every
# selected package has one result and it is identical (nonzero compiler statuses are fine when both agree); 1 on any
# mismatch, panic, timeout, missing/duplicate/unexpected/malformed result or empty selection; 2/3 for the
# read-only guard.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="${1:?usage: monorepo.sh <monorepo root> [-j N] [--filter REGEX] [-- extra flags]}"
shift
jobs=4
filter=""
buildinfo=""
extra=()
while [ $# -gt 0 ]; do
  case "$1" in
    -j) jobs="$2"; shift 2 ;;
    --filter) filter="$2"; shift 2 ;;
    --buildinfo) buildinfo="--buildinfo"; shift ;;
    --) shift; extra=("$@"); break ;;
    *) echo "unknown argument $1" >&2; exit 2 ;;
  esac
done
: "${TSGO:?set TSGO to the reference tsgo binary}"
export TSGO
export TSRS="${TSRS:-$here/../../../target/release/tsrs}"
# Go assigns files to checkers with FENNEL; tsrs defaults to directory locality, and printed types (inferred
# declaration types) can depend on which files a checker saw first. Compare in the Go assignment by default.
export TSRS_CHECKER_ASSIGNMENT="${TSRS_CHECKER_ASSIGNMENT:-go}"
OUT_GO="${OUT_GO:-/tmp/emit-go}"
OUT_RS="${OUT_RS:-/tmp/emit-rs}"
mkdir -p "$OUT_GO" "$OUT_RS"

before="$(git -C "$root" status --short)"
if [ -n "$before" ]; then
  echo "monorepo has local changes before the run; refusing to start (git status --short not empty)" >&2
  exit 2
fi

# package dir <TAB> tsconfig path
list="$(python3 - "$root" "$filter" <<'EOF'
import glob, json, os, re, sys
root, flt = sys.argv[1], sys.argv[2]
for pj in sorted(glob.glob(os.path.join(root, "apps/*/package.json")) + glob.glob(os.path.join(root, "packages/*/package.json"))):
    d = os.path.dirname(pj)
    rel = os.path.relpath(d, root)
    if flt and not re.search(flt, rel):
        continue
    build = json.load(open(pj)).get("scripts", {}).get("build", "")
    # the commands of the script that invoke tsc directly
    for cmd in re.split(r"&&|;|\|\|", build):
        words = cmd.split()
        if not words or words[0] not in ("tsc", "tsgo"):
            continue
        cfg = "tsconfig.json"
        for i, w in enumerate(words):
            if w in ("-p", "--project") and i + 1 < len(words):
                cfg = words[i + 1]
        cfg = os.path.join(d, cfg)
        if os.path.isdir(cfg):
            cfg = os.path.join(cfg, "tsconfig.json")
        if os.path.exists(cfg):
            print(f"{rel}\t{cfg}")
        break
EOF
)"

if [ -z "$(printf '%s' "$list" | tr -d '[:space:]')" ]; then
  echo "ERROR: no package selected (filter '${filter}'); refusing to report success" >&2
  exit 1
fi
selected="$OUT_RS.selected.txt"
printf '%s\n' "$list" | grep -v '^$' | cut -f1 | sed 's|/|__|g' > "$selected"
results="$OUT_RS.results.jsonl"
: > "$results"
export here OUT_GO OUT_RS buildinfo
export extra_flags="${extra[*]:-}"
# run.py exits 1 on any difference; its verdict is re-derived by summarize.py from the JSON row, and a run.py that
# dies without a row is caught there as a missing result.
printf '%s\n' "$list" | grep -v '^$' | while IFS=$'\t' read -r rel cfg; do printf '%s\0%s\0' "$rel" "$cfg"; done |
  xargs -0 -n 2 -P "$jobs" bash -c 'name="${0//\//__}"; python3 "$here/run.py" "$1" --name "$name" --go-out "$OUT_GO/$name" --rs-out "$OUT_RS/$name" --json $buildinfo -- $extra_flags || true' >> "$results"

after="$(git -C "$root" status --short)"
if [ -n "$after" ]; then
  echo "ERROR: the monorepo changed during the run:" >&2
  echo "$after" >&2
  exit 3
fi

echo "monorepo git status unchanged (empty before and after)"
python3 "$here/summarize.py" "$selected" "$results"
