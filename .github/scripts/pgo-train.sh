#!/usr/bin/env bash
# PGO training workload for release builds and the README benchmark (.github/workflows/release.yml,
# .depot/workflows/bench.yml; notes/perf-pgo.md): runs PGO-instrumented binaries and leaves their
# raw profiles (*.profraw) in <raw-dir>. Exit codes of the runs are ignored (the suite has known failures and the
# bench projects have type errors); the script fails only if a run crashes or writes no profile.
#
#   pgo-train.sh <dir with instrumented tsrs + tsrs-test + tsrs-fourslash> <raw-dir> <bench work dir>
#
# Workload: the conformance suite (in-process in tsrs-test, which links the same crate builds as tsrs, so its
# profile counts apply to the checker code in tsrs), the fourslash suite (in-process language server in
# tsrs-fourslash's worker processes, for the language-service and project-system code behind `tsrs --lsp`), plus
# the tsrs binary itself on two open-source projects from bench/projects.json (xstate-main, webpack; default mode,
# tsrs's default checker count for the runner). Needs ts-ref/tsc/testdata and network for the bench projects' clone + install
# (bench/run.py --setup-only). Never train on private code.
set -euo pipefail

bin=$(cd "$1" && pwd)
mkdir -p "$2" "$3"
raw=$(cd "$2" && pwd)
work=$(cd "$3" && pwd)

python3 bench/run.py --setup-only --projects xstate-main,webpack --work-dir "$work"

check_exit() { # $1 = exit status, $2 = what ran; 0/1/2 are tsc exit codes, anything else is a crash
  if [ "$1" -gt 2 ]; then echo "::error::$2 exited with $1"; exit 1; fi
}

LLVM_PROFILE_FILE="$raw/suite-%m.profraw" TSRS_TEST_RESULTS="$work/test-results" \
  "$bin/tsrs-test" run --suite all --timeout 120 && status=0 || status=$?
check_exit "$status" "tsrs-test run --suite all"

# Workers exit normally at the end of their batch, so each writes its counts; %m merges them per binary.
LLVM_PROFILE_FILE="$raw/fourslash-%m.profraw" TSRS_FOURSLASH_RESULTS="$work/fourslash-results" \
  "$bin/tsrs-fourslash" run > /dev/null && status=0 || status=$?
check_exit "$status" "tsrs-fourslash run"

for p in xstate-main webpack; do
  (cd "$work/solutions/$p" && LLVM_PROFILE_FILE="$raw/$p-%p.profraw" \
    "$bin/tsrs" -p . --noEmit --incremental false --pretty false > /dev/null) && status=0 || status=$?
  check_exit "$status" "tsrs -p $p"
done

ls -l "$raw"
# Every workload must have left a profile with counts in it: an instrumented binary that ends with `_exit` leaves a
# 0-byte file (the profile is written from an exit handler), which `llvm-profdata merge` accepts silently.
for f in suite fourslash xstate-main webpack; do
  found=0
  for g in "$raw"/$f-*.profraw; do
    [ -s "$g" ] && found=1
  done
  [ "$found" = 1 ] || { echo "::error::no profile with counts from $f"; ls -l "$raw"; exit 1; }
done
