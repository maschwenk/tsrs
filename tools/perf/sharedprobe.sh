#!/usr/bin/env bash
# Probe for the shared-graph prototype (notes/spike-shared-graph.md), for .depot/workflows/perf-probe.yml with
# `--input script=tools/perf/sharedprobe.sh`. The workflow's TSRS_BIN is the default build, where the prototype is
# compiled out (main's code paths). This script keeps a copy of it, builds the prototype (`--features shared-graph`)
# in the same target directory (the dependencies are reused), and runs tools/perf/leafprobe.py with two interleaved
# variants: off (the default build) and on (the prototype, TSRS_SHARED_GRAPH=1, 10 permille seed). PROBE_ARGS is
# appended; on the 16-vCPU runner use `--checkers 8,16 --reps 5 --no-strace --no-perf-stat` (8 is tsrs's default
# there, 16 is bench.yml measure-wide's `--checkers 16`).
set -euo pipefail
: "${TSRS_BIN:?}" "${BENCH_WORK:?}" "${PROBE_OUT:?}"
OFF="$PROBE_OUT/tsrs-off"
cp "$TSRS_BIN" "$OFF"
cargo build --release --locked -p tsrs_cli --features shared-graph
ON="$PWD/target/release/tsrs"
# shellcheck disable=SC2086 # PROBE_ARGS is a list of arguments
python3 tools/perf/leafprobe.py --tsrs "$OFF" --work "$BENCH_WORK" --out "$PROBE_OUT" \
  --projects "${PROBE_PROJECTS:-t3code-server}" \
  --variant "off:BIN=$OFF;TSRS_SHARED_GRAPH=0" \
  --variant "on:BIN=$ON;TSRS_SHARED_GRAPH=1" \
  ${PROBE_ARGS:-} 2>&1 | tee "$PROBE_OUT/log.txt"
rm -f "$OFF"
