#!/usr/bin/env bash
# Runs every fixture's edit sequence against tsgo (TSGO, default /root/bin/tsgo-ref = `go build ./cmd/tsc` of ts-ref)
# with tools/oracle/incremental/steps.sh. Exit status 1 if any step differs. tsrs runs with TSRS_HUB_SHORTCUT=0
# (tsgo's algorithm), then inc1 once more with the shortcut on.
set -uo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
fail=0
run() { local out; out=$("$here/../steps.sh" "$here/$1" "$here/$2" "${@:3}" 2>&1); echo "$1 $2: ${out##*$'\n'}"; [[ "$out" == *"all "*" steps identical"* ]] || fail=1; }
export TSRS_HUB_SHORTCUT=0
run inc1 inc1.steps
run inc2 inc2.steps
run b1 b1.steps -b app --verbose
run b1 b1-outputs.steps -b app --verbose
run dmap dmap.steps -b . --verbose --builders 4
# With the hub-edit shortcut (inc1 is the --noEmit fixture): identical except the documented signature values.
TSRS_HUB_SHORTCUT=1 SIGNATURES_MAY_DIFFER=1 run inc1 inc1.steps
exit $fail
