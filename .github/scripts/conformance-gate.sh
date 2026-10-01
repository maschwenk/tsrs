#!/usr/bin/env bash
# Runs the TypeScript conformance suite and fails unless it is at least as good as docs/STATUS.md says main is.
# Needs ts-ref/tsc/testdata (microsoft/TypeScript at [workspace.metadata.typescript] commit) and a built tsrs-test.
# Raise MIN_PASS when a fix lands more passes.
set -euo pipefail

MIN_PASS="${MIN_PASS:-13458}"
TSRS_TEST="${TSRS_TEST:-target/release/tsrs-test}"
export TSRS_TEST_RESULTS="${TSRS_TEST_RESULTS:-$PWD/target/test-results}"

"$TSRS_TEST" run --suite all

pass=$(wc -l < "$TSRS_TEST_RESULTS/pass.txt" | tr -d ' ')
crash=$(wc -l < "$TSRS_TEST_RESULTS/crash.txt" | tr -d ' ')
timeout=$(wc -l < "$TSRS_TEST_RESULTS/timeout.txt" | tr -d ' ')
echo "conformance: pass=$pass (minimum $MIN_PASS) crash=$crash timeout=$timeout"

if [ "$pass" -lt "$MIN_PASS" ] || [ "$crash" -ne 0 ] || [ "$timeout" -ne 0 ]; then
  echo "::error::conformance gate failed: pass=$pass (minimum $MIN_PASS), crash=$crash, timeout=$timeout"
  head -50 "$TSRS_TEST_RESULTS/crash.txt" "$TSRS_TEST_RESULTS/timeout.txt" "$TSRS_TEST_RESULTS/fail.txt" || true
  exit 1
fi
