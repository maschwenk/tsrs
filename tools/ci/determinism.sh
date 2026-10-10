#!/usr/bin/env bash
# Determinism gate: diagnostics must not depend on which checker checks a file or in what order
# (notes/perf-order-independence.md). The single-threaded run of each project and regression case is the reference;
# the default mode at 2 and 4 checkers (locality assignment and work stealing, so a different split every run) and
# random static assignments with random visit orders (`--checkerAssignment random:<seed>`) at 2 and 4 checkers must
# print the same bytes and exit with the same status. Both history-dependent diagnostics of
# notes/fix-history-dependent-diagnostics.md (nuxt's TS2320, drizzle-orm's TS2769) appeared under such assignments at
# 2-4 checkers, so those two projects are in the default set with the two cheapest bench projects. pr-verify (every pull
# request that changes crates/, or with the `verify` label) is the other half: every bench project at 1/4/16/32 checkers against the base build.
#
#   tools/ci/determinism.sh <tsrs binary> [--projects xstate-main,webpack,nuxt,drizzle-orm] [--seeds 6]
#                           [--default-reps 3] [--work-dir bench/.work] [--no-regressions]
#
# Projects are bench/projects.json names checked out under <work-dir> (`python3 bench/run.py --setup-only --projects
# <names>`); the regression cases are testdata/regressions/*/ (tools/regressions.sh checks their expected.txt).
set -uo pipefail
repo="$(cd "$(dirname "$0")/../.." && pwd)"

# --self-test: a binary whose output depends on the assignment must fail the gate (a gate that cannot fail gates nothing).
self_test() {
  local tmp status log; tmp="$(mktemp -d "${TMPDIR:-/tmp}/tsrs-determinism-selftest.XXXXXX")"
  mkdir -p "$tmp/work/solutions/xstate-main"
  printf '#!/bin/sh\ncase "$*" in *random:2*) echo "x.ts(2,1): error TS2: only under one assignment";; esac\necho "x.ts(1,1): error TS1: always"\nexit 2\n' > "$tmp/tsrs"
  chmod +x "$tmp/tsrs"
  log="$("$0" "$tmp/tsrs" --work-dir "$tmp/work" --projects xstate-main --no-regressions --seeds 2 --default-reps 1 2>&1)"; status=$?
  rm -rf "$tmp"
  if [ "$status" = 1 ] && grep -q "^FAIL xstate-main random-2-2" <<< "$log" && grep -q "^FAIL xstate-main random-4-2" <<< "$log" \
     && ! grep -q "^FAIL xstate-main default" <<< "$log" && ! grep -q "^FAIL xstate-main random-2-1" <<< "$log"; then
    echo "self-test: the gate fails an assignment-dependent binary"; return 0
  fi
  echo "self-test: expected exit 1 with FAIL for random-2-2 and random-4-2 only, got exit $status:"; echo "$log"; return 1
}
[ "${1:-}" = "--self-test" ] && { self_test; exit $?; }
[ $# -ge 1 ] || { echo "usage: $0 <tsrs binary> [--projects a,b] [--seeds N] [--default-reps N] [--work-dir DIR] [--no-regressions]"; exit 2; }
tsrs="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"; shift
projects="xstate-main,webpack,nuxt,drizzle-orm"; seeds=6; default_reps=3; work="$repo/bench/.work"; regressions=1
while [ $# -gt 0 ]; do
  case "$1" in
    --projects) projects=$2; shift 2 ;;
    --seeds) seeds=$2; shift 2 ;;
    --default-reps) default_reps=$2; shift 2 ;;
    --work-dir) work="$(cd "$2" && pwd)"; shift 2 ;;
    --no-regressions) regressions=0; shift ;;
    *) echo "unknown argument $1"; exit 2 ;;
  esac
done
out="$(mktemp -d "${TMPDIR:-/tmp}/tsrs-determinism.XXXXXX")"
trap 'rm -rf "$out"' EXIT
fail=0; runs=0

# variant <name> <dir> <label> <flags...>: one run of `tsrs ${flags[@]} <flags...>` in <dir>, compared with the
# reference run of `sweep` ($out/<name>-single.txt, exit status $ref_status).
variant() {
  local name=$1 dir=$2 label=$3 status; shift 3
  (cd "$dir" && "$tsrs" "${flags[@]}" "$@" > "$out/$name-$label.txt" 2>&1); status=$?
  runs=$((runs + 1))
  if [ "$status" != "$ref_status" ] || ! cmp -s "$out/$name-single.txt" "$out/$name-$label.txt"; then
    echo "FAIL $name $label: exit $status, output differs from --singleThreaded (exit $ref_status):"
    diff "$out/$name-single.txt" "$out/$name-$label.txt" | head -8
    fail=1
  fi
}

# sweep <name> <dir> <flags...>: the reference run and every variant of one project or case.
sweep() {
  local name=$1 dir=$2 n rep seed; shift 2
  flags=("$@")
  (cd "$dir" && "$tsrs" "${flags[@]}" --singleThreaded > "$out/$name-single.txt" 2>&1); ref_status=$?
  for n in 2 4; do
    for rep in $(seq 1 "$default_reps"); do variant "$name" "$dir" "default-$n-$rep" --checkers "$n"; done
    for seed in $(seq 1 "$seeds"); do variant "$name" "$dir" "random-$n-$seed" --checkers "$n" --checkerAssignment "random:$seed"; done
  done
}

IFS=',' read -r -a names <<< "$projects"
for name in "${names[@]}"; do
  if ! { read -r sub; read -r proj; } < <(python3 -c '
import json, sys
projects = json.load(open(sys.argv[1]))["projects"]
p = next((p for p in projects if p["name"] == sys.argv[2]), None) or sys.exit(f"unknown project {sys.argv[2]!r}")
print("suite/" + p["suite_dir"] if "suite_dir" in p else "solutions/" + p["name"])
print(p["project"])' "$repo/bench/projects.json" "$name"); then
    echo "FAIL $name: not in bench/projects.json"; fail=1; continue
  fi
  dir="$work/$sub"
  [ -d "$dir" ] || { echo "FAIL $name: no checkout at $dir (python3 bench/run.py --setup-only --projects $name)"; fail=1; continue; }
  before=$runs
  sweep "$name" "$dir" -p "$proj" --noEmit --incremental false --pretty false
  echo "$name: $(wc -l < "$out/$name-single.txt" | tr -d ' ') lines single-threaded, $((runs - before)) runs compared"
done
if [ "$regressions" = 1 ]; then
  before=$runs
  for dir in "$repo"/testdata/regressions/*/; do
    sweep "case-$(basename "$dir")" "$dir" -p . --pretty false
  done
  echo "testdata/regressions: $((runs - before)) runs compared"
fi
echo "determinism: $runs runs, $([ "$fail" = 0 ] && echo "all identical to the single-threaded output" || echo "DIFFERENCES FOUND")"
exit "$fail"
