#!/usr/bin/env bash
# BOLT post-link optimization of the PGO release binaries (Linux; .github/workflows/release.yml, notes/perf-build-level.md).
#
#   bolt.sh <dist dir> <work dir> <bench work dir>
#
# <dist dir> holds tsrs, tsrs-test and tsrs-fourslash from the final PGO build, linked with -Wl,--emit-relocs. Each
# binary is instrumented (llvm-bolt -instrument), runs its part of the PGO training workload (pgo-train.sh: tsrs on
# xstate-main and webpack, tsrs-test on the conformance suite, tsrs-fourslash on the fourslash suite), and is
# rewritten with the profile. The BOLT-optimized tsrs-test and tsrs-fourslash then run the conformance and fourslash
# gates, and the BOLT-optimized tsrs must print byte-identical output to the input tsrs on both bench projects
# (one and four checkers). Only then is <dist dir>/tsrs replaced; the input is kept as tsrs.prebolt.
#
# Needs llvm-bolt and merge-fdata (LLVM release matching rustc's LLVM) on PATH, ts-ref/tsc/testdata, and the bench
# projects pgo-train.sh set up in <bench work dir>.
set -euo pipefail

dist=$(cd "$1" && pwd)
mkdir -p "$2"
work=$(cd "$2" && pwd)
bench=$(cd "$3" && pwd)
flags=(-reorder-blocks=ext-tsp -reorder-functions=cdsort -split-functions -split-all-cold -split-eh -icf=1
  -use-gnu-stack -update-debug-sections -dyno-stats)

for b in tsrs tsrs-test tsrs-fourslash; do
  rm -rf "${work:?}/$b.fdata.d"; mkdir -p "$work/$b.fdata.d"
  llvm-bolt "$dist/$b" -instrument -o "$work/$b.inst" --instrumentation-file="$work/$b.fdata.d/prof" \
    --instrumentation-file-append-pid | tail -1
done

check_exit() { # $1 = exit status, $2 = what ran; 0/1/2 are tsc exit codes, anything else is a crash
  if [ "$1" -gt 2 ]; then echo "::error::$2 exited with $1"; exit 1; fi
}
for p in xstate-main webpack; do
  (cd "$bench/solutions/$p" && "$work/tsrs.inst" -p . --noEmit --incremental false --pretty false > /dev/null) \
    && status=0 || status=$?
  check_exit "$status" "instrumented tsrs -p $p"
done
TSRS_TEST_RESULTS="$work/train-test-results" "$work/tsrs-test.inst" run --suite all --timeout 300 > /dev/null \
  && status=0 || status=$?
check_exit "$status" "instrumented tsrs-test run --suite all"
TSRS_FOURSLASH_RESULTS="$work/train-fourslash-results" "$work/tsrs-fourslash.inst" run > /dev/null \
  && status=0 || status=$?
check_exit "$status" "instrumented tsrs-fourslash run"

for b in tsrs tsrs-test tsrs-fourslash; do
  compgen -G "$work/$b.fdata.d/prof*" > /dev/null || { echo "::error::no BOLT profile from $b"; exit 1; }
  merge-fdata "$work/$b.fdata.d"/prof* > "$work/$b.fdata"
  llvm-bolt "$dist/$b" -o "$work/$b.bolt" -data="$work/$b.fdata" "${flags[@]}" > "$work/$b.bolt.log" 2>&1 \
    || { cat "$work/$b.bolt.log"; exit 1; }
  grep -E 'BOLT-INFO: (basic block reordering|splitting|ICF)' "$work/$b.bolt.log" || true
done

# Gates on the BOLT-optimized binaries (same thresholds as ci.yml).
TSRS_TEST="$work/tsrs-test.bolt" TSRS_TEST_RESULTS="$work/test-results" .github/scripts/conformance-gate.sh | tail -1
TSRS_FOURSLASH="$work/tsrs-fourslash.bolt" TSRS_FOURSLASH_RESULTS="$work/fourslash-results" \
  .github/scripts/fourslash-gate.sh | tail -1
for p in xstate-main webpack; do
  for c in 1 4; do
    for v in prebolt bolt; do
      exe="$dist/tsrs"; [ "$v" = bolt ] && exe="$work/tsrs.bolt"
      (cd "$bench/solutions/$p" && "$exe" -p . --noEmit --incremental false --pretty false --checkers "$c" \
        > "$work/out-$p-$c-$v.txt") && status=0 || status=$?
      check_exit "$status" "$v tsrs -p $p --checkers $c"
      echo "exit $status" >> "$work/out-$p-$c-$v.txt"
    done
    cmp "$work/out-$p-$c-prebolt.txt" "$work/out-$p-$c-bolt.txt" \
      || { echo "::error::BOLT-optimized tsrs prints different output on $p (--checkers $c)"; exit 1; }
  done
done

mv "$dist/tsrs" "$dist/tsrs.prebolt"
cp "$work/tsrs.bolt" "$dist/tsrs"
ls -l "$dist/tsrs" "$dist/tsrs.prebolt"
