#!/usr/bin/env bash
# Runs TypeScript's fourslash (language service) tests against the in-process tsrs language server and fails unless
# the result is at least as good as docs/LSP.md says main is. Needs ts-ref/tsc/testdata and a built tsrs-fourslash.
# Raise MIN_PASS when a fix lands more passes; MAX_FAIL counts the out-of-scope tests (55 content-mapper tests and 8
# `@tsc` tests that need the harness to run `tsc --build` first, Go `tsctests.GetFileMapWithBuild`).
set -euo pipefail

MIN_PASS="${MIN_PASS:-4066}"
MAX_FAIL="${MAX_FAIL:-63}"
TSRS_FOURSLASH="${TSRS_FOURSLASH:-target/release/tsrs-fourslash}"
results="${TSRS_FOURSLASH_RESULTS:-$PWD/target/fourslash-results}"

"$TSRS_FOURSLASH" run || true

pass=$(wc -l < "$results/pass.txt" | tr -d ' ')
fail=$(wc -l < "$results/fail.txt" | tr -d ' ')
echo "fourslash: pass=$pass (minimum $MIN_PASS) fail=$fail (maximum $MAX_FAIL)"

if [ "$pass" -lt "$MIN_PASS" ] || [ "$fail" -gt "$MAX_FAIL" ]; then
  echo "::error::fourslash gate failed: pass=$pass (minimum $MIN_PASS), fail=$fail (maximum $MAX_FAIL)"
  head -80 "$results/fail.txt" || true
  exit 1
fi
