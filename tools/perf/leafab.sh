#!/usr/bin/env bash
# Leaf-freeing probe against a base build (notes/mem-leaf-regions-32.md), for .depot/workflows/perf-probe.yml with
# `--input script=tools/perf/leafab.sh`: builds the merge base of this checkout and main with `cargo build --release` in
# a worktree next to it, then runs tools/perf/leafprobe.py with PROBE_ARGS, where a variant's `BIN=@BASE@` runs the base
# binary (for example `--variant base-on:BIN=@BASE@;TSRS_FREE_LEAVES=1`).
set -euo pipefail
: "${TSRS_BIN:?}" "${BENCH_WORK:?}" "${PROBE_OUT:?}"
git fetch -q --unshallow origin main
base=$(git merge-base HEAD FETCH_HEAD)
echo "base $base"
base_dir="$(pwd)/../tsrs-base"
git worktree add -q --detach "$base_dir" "$base"
(cd "$base_dir" && CARGO_TARGET_DIR="$(pwd)/../tsrs-base-target" cargo build --release --locked -p tsrs_cli 2>&1 | tail -2)
BASE_BIN="$(cd "$base_dir/.." && pwd)/tsrs-base-target/release/tsrs"
export BASE_BIN
"$BASE_BIN" --version
args=${PROBE_ARGS:-}
# shellcheck disable=SC2086 # a list of arguments
python3 tools/perf/leafprobe.py --tsrs "$TSRS_BIN" --work "$BENCH_WORK" --out "$PROBE_OUT" \
  --projects "${PROBE_PROJECTS:-vscode}" ${args//@BASE@/$BASE_BIN} 2>&1 | tee "$PROBE_OUT/log.txt"
