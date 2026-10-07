#!/usr/bin/env bash
# Checker-count scaling probe for .depot/workflows/perf-probe.yml (`--input script=tools/perf/scaleprobe.sh`):
# tools/perf/scaleprobe.py on the projects the workflow set up, with PROBE_ARGS appended
# (for example `--checkers 8,12,16,24,32 --reps 5 --times`).
set -euo pipefail
: "${TSRS_BIN:?}" "${BENCH_WORK:?}" "${PROBE_OUT:?}"
mkdir -p "$PROBE_OUT"
# shellcheck disable=SC2086 # PROBE_ARGS is a list of arguments
python3 tools/perf/scaleprobe.py --tsrs "$TSRS_BIN" --work "$BENCH_WORK" --out "$PROBE_OUT" \
  --projects "${PROBE_PROJECTS:-vscode}" ${PROBE_ARGS:-} 2>&1 | tee "$PROBE_OUT/log.txt"
