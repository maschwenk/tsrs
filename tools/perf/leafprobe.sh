#!/usr/bin/env bash
# Probe for leaf-file freeing (notes/mem-leaf-regions-cost.md), for .depot/workflows/perf-probe.yml with
# `--input script=tools/perf/leafprobe.sh`: tools/perf/leafprobe.py on the projects the workflow set up, with
# PROBE_ARGS appended (for example `--reps 20 --checkers 1,4,8 --no-strace`).
set -euo pipefail
: "${TSRS_BIN:?}" "${BENCH_WORK:?}" "${PROBE_OUT:?}"
# shellcheck disable=SC2086 # PROBE_ARGS is a list of arguments
python3 tools/perf/leafprobe.py --tsrs "$TSRS_BIN" --work "$BENCH_WORK" --out "$PROBE_OUT" \
  --projects "${PROBE_PROJECTS:-vscode}" ${PROBE_ARGS:-} 2>&1 | tee "$PROBE_OUT/log.txt"
