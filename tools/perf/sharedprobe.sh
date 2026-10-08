#!/usr/bin/env bash
# Probe for the shared-graph prototype (notes/spike-shared-graph.md), for .depot/workflows/perf-probe.yml with
# `--input script=tools/perf/sharedprobe.sh`. The workflow's TSRS_BIN is the default build, where the prototype is
# compiled out (main's code paths); this script builds the prototype (`--features shared-graph`) next to it and runs
# tools/perf/leafprobe.py with three interleaved variants: off (the default build), on (10 permille seed) and on20
# (20 permille). PROBE_ARGS is appended, for example `--checkers 4,16,32 --reps 5 --no-strace`.
set -euo pipefail
: "${TSRS_BIN:?}" "${BENCH_WORK:?}" "${PROBE_OUT:?}"
CARGO_TARGET_DIR=target/sg cargo build --release --locked -p tsrs_cli --features shared-graph
SG="$PWD/target/sg/release/tsrs"
# shellcheck disable=SC2086 # PROBE_ARGS is a list of arguments
python3 tools/perf/leafprobe.py --tsrs "$TSRS_BIN" --work "$BENCH_WORK" --out "$PROBE_OUT" \
  --projects "${PROBE_PROJECTS:-t3code-server}" \
  --variant "off:TSRS_SHARED_GRAPH=0" \
  --variant "on:BIN=$SG;TSRS_SHARED_GRAPH=1" \
  --variant "on20:BIN=$SG;TSRS_SHARED_GRAPH=1;TSRS_SHARED_GRAPH_SEED=spread:20" \
  ${PROBE_ARGS:-} 2>&1 | tee "$PROBE_OUT/log.txt"
