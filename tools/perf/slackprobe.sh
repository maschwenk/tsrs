#!/usr/bin/env bash
# Probe for the residency slack of a many-checker run (notes/mem-linux-residency-32.md), for
# .depot/workflows/perf-probe.yml with `--input script=tools/perf/slackprobe.sh`: tools/perf/slackprobe.py on the
# projects the workflow set up, with PROBE_ARGS appended. `--base-ref <ref>` (first in PROBE_ARGS) also builds that
# ref into target-base/ and exposes it as $BASE_BIN, for variants such as `main:BIN=$BASE_BIN`.
set -euo pipefail
: "${TSRS_BIN:?}" "${BENCH_WORK:?}" "${PROBE_OUT:?}"
mkdir -p "$PROBE_OUT"
args=${PROBE_ARGS:-}
if [[ "$args" == --base-ref* ]]; then
  ref=$(awk '{print $2}' <<<"$args")
  args=$(cut -d' ' -f3- <<<"$args")
  git fetch -q origin "$ref" 2>/dev/null || git fetch -q --unshallow origin 2>/dev/null || true
  git worktree add -f /tmp/base-src FETCH_HEAD 2>/dev/null || git worktree add -f /tmp/base-src "origin/$ref"
  (cd /tmp/base-src && CARGO_TARGET_DIR="$PWD/../base-target" cargo build --release --locked -p tsrs_cli 2>&1 | tail -3)
  export BASE_BIN=/tmp/base-target/release/tsrs
  echo "base: $ref $(git -C /tmp/base-src rev-parse HEAD)" | tee "$PROBE_OUT/base.txt"
fi
args=${args//\$BASE_BIN/${BASE_BIN:-}}
# shellcheck disable=SC2086 # args is a list of arguments
eval python3 tools/perf/slackprobe.py --tsrs "$TSRS_BIN" --work "$BENCH_WORK" --out "$PROBE_OUT" \
  --projects "${PROBE_PROJECTS:-vscode}" $args 2>&1 | tee "$PROBE_OUT/log.txt"
