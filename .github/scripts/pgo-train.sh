#!/usr/bin/env bash
# PGO training workload for release builds (notes/perf-pgo.md): runs PGO-instrumented binaries and leaves their
# raw profiles (*.profraw) in <raw-dir>. Exit codes of the runs are ignored (the suite has known failures and the
# bench projects have type errors); the script fails only if a run crashes or writes no profile.
#
#   pgo-train.sh <dir with instrumented tsrs + tsrs-test> <raw-dir> <bench work dir>
#
# Workload: the conformance suite (in-process in tsrs-test, which links the same crate builds as tsrs, so its
# profile counts apply to the checker code in tsrs) plus the tsrs binary itself on two open-source projects from
# bench/projects.json (xstate-main, webpack; default mode, 4 checkers). Needs ts-ref/tsc/testdata and network for
# the bench projects' clone + install (bench/run.py --setup-only). Never train on private code.
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

for p in xstate-main webpack; do
  (cd "$work/solutions/$p" && LLVM_PROFILE_FILE="$raw/$p-%p.profraw" \
    "$bin/tsrs" -p . --noEmit --incremental false --pretty false > /dev/null) && status=0 || status=$?
  check_exit "$status" "tsrs -p $p"
done

ls -l "$raw"
for f in suite xstate-main webpack; do
  compgen -G "$raw/$f-*.profraw" > /dev/null || { echo "::error::no profile from $f"; exit 1; }
done
