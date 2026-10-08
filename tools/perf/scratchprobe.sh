#!/usr/bin/env bash
# A/B probe for notes/mem-checker-scratch.md, for .depot/workflows/perf-probe.yml with
# `--input script=tools/perf/scratchprobe.sh`: builds the merge base with origin/main (release, like the workflow's
# build of the branch) and runs tools/perf/leafprobe.py with the variants `base` (merge base) and `new` (the branch),
# interleaved, on the projects the workflow set up, with PROBE_ARGS appended (for example `--reps 5 --checkers 1,4,32`).
set -euo pipefail
: "${TSRS_BIN:?}" "${BENCH_WORK:?}" "${PROBE_OUT:?}"
mkdir -p "$PROBE_OUT" /tmp/sprobe/base /tmp/sprobe/new
[ "$(git rev-parse --is-shallow-repository)" = true ] && git fetch -q --no-tags --unshallow origin
git fetch -q --no-tags origin "+refs/heads/main:refs/remotes/origin/main"
base=$(git merge-base HEAD origin/main)
{ echo "HEAD $(git rev-parse HEAD)"; echo "base $base"; } | tee "$PROBE_OUT/commits.txt"
cp "$TSRS_BIN" /tmp/sprobe/new/tsrs
head=$(git rev-parse HEAD)
git checkout -q --detach "$base"
cargo build --release --locked -p tsrs_cli 2>&1 | tail -2
cp target/release/tsrs /tmp/sprobe/base/tsrs
git checkout -q --detach "$head"
sha256sum /tmp/sprobe/*/tsrs | tee -a "$PROBE_OUT/commits.txt"
# shellcheck disable=SC2086 # PROBE_ARGS is a list of arguments
python3 tools/perf/leafprobe.py --tsrs /tmp/sprobe/new/tsrs --work "$BENCH_WORK" --out "$PROBE_OUT" \
  --projects "${PROBE_PROJECTS:-formbricks-web}" --variant "base:BIN=/tmp/sprobe/base/tsrs" --variant "new:" \
  --no-strace --no-perf-stat ${PROBE_ARGS:-} 2>&1 | tee "$PROBE_OUT/log.txt"
