#!/usr/bin/env bash
# Probe for the shared-graph prototype (notes/spike-shared-graph.md), for .depot/workflows/perf-probe.yml with
# `--input script=tools/perf/sharedprobe.sh`. The workflow's TSRS_BIN is the default build, where the prototype is
# compiled out (main's code paths). This script keeps a copy of it, builds the prototype (`--features shared-graph`)
# in the same target directory (the dependencies are reused), and runs tools/perf/leafprobe.py with interleaved
# variants. PROBE_ARGS is appended; on the 16-vCPU runner use `--checkers 8 --reps 7 --no-strace --no-perf-stat`.
#
# Variants (SHAREDPROBE_VARIANTS, comma-separated, default off,foff,on,on0,on2.5):
#   off    the default build (prototype compiled out)
#   foff   the prototype build with the switch off (what compiling the read paths in costs)
#   on     the prototype, TSRS_SHARED_GRAPH=1, 10 permille seed
#   on0    the prototype with an empty seed (forks of a fresh checker: the overlay's cost without any sharing)
#   on<p>  the prototype with a <p> permille seed
# Every variant runs with TSRS_TIMELINE=1 (spike/shared-graph-seed): each run's output in runs/ ends with the
# timeline of the front end, the seed, the forks and each checker's start and end.
set -euo pipefail
: "${TSRS_BIN:?}" "${BENCH_WORK:?}" "${PROBE_OUT:?}"
OFF="$PROBE_OUT/tsrs-off"
cp "$TSRS_BIN" "$OFF"
cargo build --release --locked -p tsrs_cli --features shared-graph
ON="$PWD/target/release/tsrs"
variants=()
IFS=, read -r -a names <<< "${SHAREDPROBE_VARIANTS:-off,foff,on,on0,on2.5}"
for v in "${names[@]}"; do
  case "$v" in
    off) variants+=(--variant "off:BIN=$OFF;TSRS_SHARED_GRAPH=0;TSRS_TIMELINE=1") ;;
    foff) variants+=(--variant "foff:BIN=$ON;TSRS_SHARED_GRAPH=0;TSRS_TIMELINE=1") ;;
    on) variants+=(--variant "on:BIN=$ON;TSRS_SHARED_GRAPH=1;TSRS_TIMELINE=1") ;;
    on*) variants+=(--variant "$v:BIN=$ON;TSRS_SHARED_GRAPH=1;TSRS_TIMELINE=1;TSRS_SHARED_GRAPH_SEED=spread:${v#on}") ;;
  esac
done
# shellcheck disable=SC2086 # PROBE_ARGS is a list of arguments
python3 tools/perf/leafprobe.py --tsrs "$OFF" --work "$BENCH_WORK" --out "$PROBE_OUT" \
  --projects "${PROBE_PROJECTS:-t3code-server}" "${variants[@]}" \
  ${PROBE_ARGS:-} 2>&1 | tee "$PROBE_OUT/log.txt"
rm -f "$OFF"
