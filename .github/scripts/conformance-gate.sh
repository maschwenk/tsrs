#!/usr/bin/env bash
# Runs the TypeScript conformance suite and fails unless it is at least as good as docs/STATUS.md says main is: the
# error baselines, plus the `.types` / `.symbols` baselines that BASELINES names (default both; BASELINES=errors
# checks the error baselines only). Any crash or timeout fails it.
# Needs ts-ref/tsc/testdata (microsoft/TypeScript at [workspace.metadata.typescript] commit) and a built tsrs-test.
# Raise MIN_PASS / MIN_TYPES / MIN_SYMBOLS when a fix lands more passes.
set -euo pipefail

MIN_PASS="${MIN_PASS:-13458}"
MIN_TYPES="${MIN_TYPES:-12779}"
MIN_SYMBOLS="${MIN_SYMBOLS:-12779}"
BASELINES="${BASELINES:-types,symbols}"
TSRS_TEST="${TSRS_TEST:-target/release/tsrs-test}"
export TSRS_TEST_RESULTS="${TSRS_TEST_RESULTS:-$PWD/target/test-results}"

# The default 20 s per-test timeout is tuned for a fast dev machine; on 2-core CI runners
# compiler/intersectionConstructorReductionCrash (~8 s locally) exceeds it.
"$TSRS_TEST" run --suite all --baselines "$BASELINES" --timeout "${TEST_TIMEOUT:-120}"

count() { wc -l < "$TSRS_TEST_RESULTS/$1.txt" | tr -d ' '; }

pass=$(count pass) crash=$(count crash) timeout=$(count timeout)
summary="conformance: pass=$pass (minimum $MIN_PASS) crash=$crash timeout=$timeout"
lists=(crash timeout fail)
ok=1
if [ "$pass" -lt "$MIN_PASS" ] || [ "$crash" -ne 0 ] || [ "$timeout" -ne 0 ]; then ok=0; fi

# tsrs-test writes the <ext>-<class>.txt lists only for the baselines it compared, so check only those.
for ext in types symbols; do
  case ",$BASELINES," in *",$ext,"* | *",all,"*) ;; *) continue ;; esac
  case $ext in types) min=$MIN_TYPES ;; symbols) min=$MIN_SYMBOLS ;; esac
  p=$(count "$ext-pass") c=$(count "$ext-crash") t=$(count "$ext-timeout")
  summary="$summary; .$ext pass=$p (minimum $min) crash=$c timeout=$t"
  lists+=("$ext-crash" "$ext-timeout" "$ext-fail")
  if [ "$p" -lt "$min" ] || [ "$c" -ne 0 ] || [ "$t" -ne 0 ]; then ok=0; fi
done

if [ "$ok" -ne 1 ]; then
  echo "::error::conformance gate failed: $summary"
  (cd "$TSRS_TEST_RESULTS" && head -50 "${lists[@]/%/.txt}") || true
  exit 1
fi
echo "$summary"
