#!/usr/bin/env bash
# The WebAssembly differential gate: native tsrs --singleThreaded against tsrs.wasm on testdata/regressions, the
# fixtures, the 2,000-case conformance sample (node file system and in memory), the four bench projects (three
# times, read-only) and emit on the two Compiler projects. Results under target/wasm-diff/<set>/summary.json.
#   tools/wasm/gate.sh [native] [module]     (defaults: target/release/tsrs, npm/tsrs-wasm/tsrs.wasm)
# Needs target/release/tsrs-test (the sample is materialized from ts-ref/tsc/testdata or $TSRS_TESTDATA) and the
# bench checkouts: BENCH_SOLUTIONS (default bench/.work/solutions: xstate-main, webpack) and BENCH_SUITE (default
# bench/.work/suite/cases/solutions: Compiler, Compiler-Unions).
set -uo pipefail
cd "$(dirname "$0")/../.."
N=${1:-target/release/tsrs}
M=${2:-npm/tsrs-wasm/tsrs.wasm}
O=target/wasm-diff
B=${BENCH_SOLUTIONS:-bench/.work/solutions}
C=${BENCH_SUITE:-bench/.work/suite/cases/solutions}
F="--noEmit --incremental false --pretty false"
JOBS=${JOBS:-6}
status=0
run() {
  local name=$1; shift
  echo "== $name"
  node tools/wasm/diff.mjs --native "$N" --module "$M" --out "$O/$name" "$@" 2> "$O/$name.log"; local s=$?
  if [ $s -ne 0 ]; then status=1; fi
}
mkdir -p $O
run regressions --regressions testdata/regressions --stack-census
run fixtures --fixtures tools/wasm/fixtures --jobs 4 --stack-census --timeout 300
./target/release/tsrs-test materialize --list tools/wasm/sample-2000.txt --out $O/cases
run sample-node --cases $O/cases --jobs "$JOBS" --stack-census
run sample-memory --cases $O/cases --fs memory --jobs "$JOBS"
run bench --jobs 2 --repeat 3 --stack-census --timeout 300 \
  --project "xstate-main=$B/xstate-main:-p . $F" --project "webpack=$B/webpack:-p . $F" \
  --project "Compiler=$C/Compiler:-p . $F" --project "Compiler-Unions=$C/Compiler-Unions:-p tsconfig.json $F"
E=$O/emit-cases
rm -rf $E
for p in Compiler:. Compiler-Unions:tsconfig.json; do
  n=${p%%:*}; prj=${p#*:}
  mkdir -p $E/$n && rsync -a --exclude .git "$C/$n/" "$E/$n/root/"
  printf '{"id":"emit/%s","cwd":".","args":["-p","%s","--declaration","--sourceMap","--declarationMap","--outDir","${ROOT}/emit-out","--incremental","false","--pretty","false"]}' "$n" "$prj" > $E/$n/case.json
done
run emit --cases $E --jobs 2
exit $status
