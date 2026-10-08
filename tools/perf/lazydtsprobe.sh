#!/usr/bin/env bash
# Probe for lazily parsed declaration-file member lists (notes/mem-lazy-dts-members.md), for
# .depot/workflows/perf-probe.yml with `--input script=tools/perf/lazydtsprobe.sh`: builds origin/main next to the
# branch build, then runs tools/perf/leafprobe.py on the projects the workflow set up with three interleaved variants:
# main's binary, this binary with TSRS_LAZY_DTS=0, and this binary as it ships. PROBE_ARGS is appended (for example
# `--reps 7 --checkers 1,4,16,32 --no-strace`).
set -euo pipefail
: "${TSRS_BIN:?}" "${BENCH_WORK:?}" "${PROBE_OUT:?}"
target="$(dirname "$(dirname "$TSRS_BIN")")"
cp "$TSRS_BIN" /tmp/tsrs-branch
git fetch -q --depth 1 origin main
rm -rf /tmp/tsrs-main && git worktree add -f --detach /tmp/tsrs-main FETCH_HEAD
(cd /tmp/tsrs-main && cargo build --release --locked -p tsrs_cli --target-dir "$target")
cp "$target/release/tsrs" /tmp/tsrs-main-bin
cp /tmp/tsrs-branch "$TSRS_BIN"
{ echo "branch: $("$TSRS_BIN" --version) ${GITHUB_SHA:-}"; echo "main: $(/tmp/tsrs-main-bin --version) $(git -C /tmp/tsrs-main rev-parse HEAD)"; } | tee "$PROBE_OUT/binaries.txt"
# shellcheck disable=SC2086 # PROBE_ARGS is a list of arguments
python3 tools/perf/leafprobe.py --tsrs "$TSRS_BIN" --work "$BENCH_WORK" --out "$PROBE_OUT" \
  --projects "${PROBE_PROJECTS:-formbricks-web}" \
  --variant main:BIN=/tmp/tsrs-main-bin --variant off:TSRS_LAZY_DTS=0 --variant on:TSRS_LAZY_DTS=1 \
  ${PROBE_ARGS:-} 2>&1 | tee "$PROBE_OUT/log.txt"
