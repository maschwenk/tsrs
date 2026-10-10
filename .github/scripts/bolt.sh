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
#
# BOLT_TSRS_ONLY=1 (.depot/workflows/bench.yml): only tsrs, whose BOLT training and byte comparison are the same as
# above; <dist dir> then needs no tsrs-test or tsrs-fourslash, and the suite gates (release.yml's job) are not run.
set -euo pipefail

dist=$(cd "$1" && pwd)
mkdir -p "$2"
work=$(cd "$2" && pwd)
bench=$(cd "$3" && pwd)
flags=(-reorder-blocks=ext-tsp -reorder-functions=cdsort -split-functions -split-all-cold -split-eh -icf=1
  -use-gnu-stack -update-debug-sections -dyno-stats)
# aarch64: rustc links aarch64-unknown-linux-gnu with -Wl,--fix-cortex-a53-843419 (its target spec; GNU ld 2.38 has no
# --no- form), and llvm-bolt refuses a binary with the erratum veneers unless told to drop them. The BOLT-optimized
# binaries therefore carry no 843419 workaround; it only matters on Cortex-A53 r0p0-r0p4 cores.
arch_flags=()
[ "$(uname -m)" = aarch64 ] && arch_flags=(--drop-cortex-a53-843419-veneers)
bins=(tsrs tsrs-test tsrs-fourslash)
[ "${BOLT_TSRS_ONLY:-}" = 1 ] && bins=(tsrs)

for b in "${bins[@]}"; do
  rm -rf "${work:?}/$b.fdata.d"; mkdir -p "$work/$b.fdata.d"
  llvm-bolt "$dist/$b" -instrument -o "$work/$b.inst" --instrumentation-file="$work/$b.fdata.d/prof" \
    --instrumentation-file-append-pid "${arch_flags[@]}" | tail -1
done

check_exit() { # $1 = exit status, $2 = what ran; 0/1/2 are tsc exit codes, anything else is a crash
  if [ "$1" -gt 2 ]; then echo "::error::$2 exited with $1"; exit 1; fi
}
# tsrs ends with `exit`, so BOLT's exit handler writes the profile. (MIMALLOC_SHOW_STATS=1 is a leftover from when
# the CLI ended with `_exit`.)
for p in xstate-main webpack; do
  (cd "$bench/solutions/$p" && MIMALLOC_SHOW_STATS=1 "$work/tsrs.inst" -p . --noEmit --incremental false --pretty false \
    > /dev/null 2>> "$work/tsrs-train-stderr.log") && status=0 || status=$?
  check_exit "$status" "instrumented tsrs -p $p"
done
if [ ${#bins[@]} -gt 1 ]; then
  TSRS_TEST_RESULTS="$work/train-test-results" "$work/tsrs-test.inst" run --suite all --timeout 300 > /dev/null \
    && status=0 || status=$?
  check_exit "$status" "instrumented tsrs-test run --suite all"
  TSRS_FOURSLASH_RESULTS="$work/train-fourslash-results" "$work/tsrs-fourslash.inst" run > /dev/null \
    && status=0 || status=$?
  check_exit "$status" "instrumented tsrs-fourslash run"
fi

for b in "${bins[@]}"; do
  [ -n "$(find "$work/$b.fdata.d" -name 'prof*' -size +0)" ] || { echo "::error::no BOLT profile from $b"; exit 1; }
  merge-fdata "$work/$b.fdata.d"/prof* > "$work/$b.fdata"
  llvm-bolt "$dist/$b" -o "$work/$b.bolt" -data="$work/$b.fdata" "${flags[@]}" "${arch_flags[@]}" > "$work/$b.bolt.log" 2>&1 \
    || { cat "$work/$b.bolt.log"; exit 1; }
  grep -E 'BOLT-INFO: (basic block reordering|splitting|ICF)' "$work/$b.bolt.log" || true
done

# Gates on the BOLT-optimized binaries (same thresholds as ci.yml).
if [ ${#bins[@]} -gt 1 ]; then
  TSRS_TEST="$work/tsrs-test.bolt" TSRS_TEST_RESULTS="$work/test-results" .github/scripts/conformance-gate.sh | tail -1
  TSRS_FOURSLASH="$work/tsrs-fourslash.bolt" TSRS_FOURSLASH_RESULTS="$work/fourslash-results" \
    .github/scripts/fourslash-gate.sh | tail -1
fi
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

# BOLT needs symbols in its input, but the staged release binary does not. Strip only after instrumentation,
# optimization and the correctness comparisons above have finished.
strip --strip-all "$work/tsrs.bolt"
mv "$dist/tsrs" "$dist/tsrs.prebolt"
cp "$work/tsrs.bolt" "$dist/tsrs"
ls -l "$dist/tsrs" "$dist/tsrs.prebolt"
