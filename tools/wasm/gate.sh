#!/usr/bin/env bash
# The WebAssembly differential gate: native tsrs --singleThreaded against tsrs.wasm on testdata/regressions, the
# fixtures, the 2,000-case conformance sample (node file system and in memory, one native run for both), the four
# bench projects (three times, read-only) and emit on the two Compiler projects. Results under
# target/wasm-diff/<set>/summary.json (the sample: target/wasm-diff/sample/{node,memory}/summary.json).
#   tools/wasm/gate.sh [native] [module]     (defaults: target/release/tsrs, npm/tsrs-wasm/tsrs.wasm)
# Needs tsrs-test (TSRS_TEST, default target/release/tsrs-test; the sample is materialized from ts-ref/tsc/testdata or
# $TSRS_TESTDATA) and the bench checkouts: BENCH_SOLUTIONS (default bench/.work/solutions: xstate-main, webpack) and
# BENCH_SUITE (default bench/.work/suite/cases/solutions: Compiler, Compiler-Unions). JOBS (default: the CPU count)
# is the sample's parallelism; the other sets run beside it.
set -uo pipefail
cd "$(dirname "$0")/../.."
N=${1:-target/release/tsrs}
M=${2:-npm/tsrs-wasm/tsrs.wasm}
T=${TSRS_TEST:-target/release/tsrs-test}
O=target/wasm-diff
B=${BENCH_SOLUTIONS:-bench/.work/solutions}
C=${BENCH_SUITE:-bench/.work/suite/cases/solutions}
F="--noEmit --incremental false --pretty false"
JOBS=${JOBS:-$(node -p 'os.availableParallelism()')}
rm -rf $O
mkdir -p $O
# run <name> <diff.mjs args...>: in the background; its summary line goes to <name>.out, its progress to <name>.log.
pids=()
names=()
run() {
  local name=$1; shift
  node tools/wasm/diff.mjs --native "$N" --module "$M" --out "$O/$name" "$@" > "$O/$name.out" 2> "$O/$name.log" &
  pids+=($!)
  names+=("$name")
}

E=$O/emit-cases
for p in Compiler:. Compiler-Unions:tsconfig.json; do
  n=${p%%:*}; prj=${p#*:}
  mkdir -p $E/$n && rsync -a --exclude .git "$C/$n/" "$E/$n/root/"
  printf '{"id":"emit/%s","cwd":".","args":["-p","%s","--declaration","--sourceMap","--declarationMap","--outDir","${ROOT}/emit-out","--incremental","false","--pretty","false"]}' "$n" "$prj" > $E/$n/case.json
done
"$T" materialize --list tools/wasm/sample-2000.txt --out $O/cases || exit 1

# The small sets first, so that their few long cases (deep-recursion fixtures, the bench projects) overlap the sample.
run fixtures --fixtures tools/wasm/fixtures --jobs 4 --stack-census --timeout 300
run bench --jobs 2 --repeat 3 --stack-census --timeout 300 \
  --project "xstate-main=$B/xstate-main:-p . $F" --project "webpack=$B/webpack:-p . $F" \
  --project "Compiler=$C/Compiler:-p . $F" --project "Compiler-Unions=$C/Compiler-Unions:-p tsconfig.json $F"
run emit --cases $E --jobs 2
run regressions --regressions testdata/regressions --stack-census --jobs 2
run sample --cases $O/cases --fs node,memory --jobs "$JOBS" --stack-census

status=0
for i in "${!pids[@]}"; do
  wait "${pids[$i]}" || status=1
  echo "== ${names[$i]}"
  cat "$O/${names[$i]}.out"
done
exit $status
