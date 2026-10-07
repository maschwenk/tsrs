#!/usr/bin/env bash
# Probe for the checker assignment (notes/perf-clustered-assignment.md), for .depot/workflows/perf-probe.yml with
# `--input script=tools/perf/clusterprobe.sh`: builds the branch and its merge base with origin/main as `--profile dist`
# in this same directory, one after the other (a plain release A/B moves check time 15-60 ms on unrelated diffs,
# notes/perf-front-end-fixed-costs.md), then runs tools/perf/clusterprobe.py on the projects the workflow set up with
# PROBE_ARGS appended (for example `--reps 10 --checkers 16,32`). Variants: `main` (the merge base), then those PROBE_ARGS
# names (`--variant name:new:K=V;K=V`, the branch under that environment), or by default `branch`, `nosticky`
# (TSRS_STEAL_STICKY=0), `nocluster` (TSRS_MODULE_AFFINITY=off) and `off` (both).
set -euo pipefail
: "${BENCH_WORK:?}" "${PROBE_OUT:?}"
mkdir -p "$PROBE_OUT" /tmp/cprobe/base /tmp/cprobe/new
[ "$(git rev-parse --is-shallow-repository)" = true ] && git fetch -q --no-tags --unshallow origin
git fetch -q --no-tags origin "+refs/heads/main:refs/remotes/origin/main"
base=$(git merge-base HEAD origin/main)
{ echo "HEAD $(git rev-parse HEAD)"; echo "base $base"; git status --porcelain; } | tee "$PROBE_OUT/commits.txt"

cargo build --profile dist --locked -p tsrs_cli 2>&1 | tail -2
cp target/dist/tsrs /tmp/cprobe/new/tsrs
# The merge base in the same directory: set the branch's changes (committed or not) aside, build, put them back.
dirty=$([ -n "$(git status --porcelain)" ] && echo true || echo false)
[ "$dirty" = true ] && git -c user.name=probe -c user.email=probe@localhost stash push -q -u -m clusterprobe
head=$(git rev-parse HEAD)
git checkout -q --detach "$base"
cargo build --profile dist --locked -p tsrs_cli 2>&1 | tail -2
cp target/dist/tsrs /tmp/cprobe/base/tsrs
git checkout -q --detach "$head"
[ "$dirty" = true ] && git stash pop -q
sha256sum /tmp/cprobe/*/tsrs | tee -a "$PROBE_OUT/commits.txt"

# shellcheck disable=SC2086 # PROBE_ARGS is a list of arguments
python3 tools/perf/clusterprobe.py --work "$BENCH_WORK" --out "$PROBE_OUT" --projects "${PROBE_PROJECTS:-vscode}" \
  --bin base=/tmp/cprobe/base/tsrs --bin new=/tmp/cprobe/new/tsrs \
  --variant main:base $(case " ${PROBE_ARGS:-} " in *" --variant "*) ;; *) echo "--variant branch:new \
  --variant nosticky:new:TSRS_STEAL_STICKY=0 --variant nocluster:new:TSRS_MODULE_AFFINITY=off \
  --variant off:new:TSRS_MODULE_AFFINITY=off;TSRS_STEAL_STICKY=0";; esac) \
  ${PROBE_ARGS:-} 2>&1 | tee "$PROBE_OUT/log.txt"

