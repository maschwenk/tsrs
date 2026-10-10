#!/usr/bin/env bash
# Builds the binaries of the build-std / non-PIE probe (.depot/workflows/perf-buildstd-probe.yml,
# notes/perf-build-std.md), Linux x86-64. Every variant runs the whole release pipeline on its own: instrumented build
# of tsrs + tsrs-test + tsrs-fourslash, pgo-train.sh, llvm-profdata merge, final build with --emit-relocs and
# CARGO_PROFILE_DIST_STRIP=none, then bolt.sh (BOLT_TSRS_ONLY=1, as bench.yml).
#
#   A    the release pipeline as it is
#   B    A + RUSTC_BOOTSTRAP=1 -Zbuild-std=std,panic_unwind (std compiled with tsrs's profile, PGO and LTO)
#   C    B + -Zlocation-detail=none
#   D    B + non-PIE (-Crelocation-model=static -Clink-arg=-no-pie)
#   DA   A + non-PIE
#   E    A + -Ztune-cpu=znver4 (RUSTC_BOOTSTRAP=1; scheduling for Zen 4/5, the ISA stays x86-64)
#   F    A + -Cno-vectorize-loops -Cno-vectorize-slp
#   AN   A with BOLT minus -split-eh
#   DN   DA with BOLT minus -split-eh
#
#   buildstd-build.sh <out dir> <bench work dir> [variants, default A,B,C,D,DA]
#
# Writes <out>/bin/<name>/tsrs (BOLT-optimized, stripped), <out>/times.txt, <out>/sizes.txt, <out>/profiles.txt.
set -euo pipefail

mkdir -p "$1"
out=$(cd "$1" && pwd)
work=$(cd "$2" && pwd)
IFS=, read -ra variants <<< "${3:-A,B,C,D,DA}"
tgt=x86_64-unknown-linux-gnu
profdata="$(rustc --print sysroot)/lib/rustlib/$tgt/bin/llvm-profdata"
: > "$out/times.txt"

step() { # $1 = name; runs the rest and records its duration
  local name=$1 t0; shift; t0=$(date +%s.%N)
  "$@"
  awk -v n="$name" -v a="$t0" -v b="$(date +%s.%N)" 'BEGIN { printf "%-40s %6.1f s\n", n, b - a }' | tee -a "$out/times.txt"
}

# Per variant: extra RUSTFLAGS, extra cargo arguments, RUSTC_BOOTSTRAP.
rustflags_of() {
  case $1 in
    C) echo "-Zlocation-detail=none" ;;
    D|DA|DN) echo "-Crelocation-model=static -Clink-arg=-no-pie" ;;
    E) echo "-Ztune-cpu=znver4" ;;
    F) echo "-Cno-vectorize-loops -Cno-vectorize-slp" ;;
    *) echo "" ;;
  esac
}
buildstd_of() { case $1 in B|C|D) echo 1 ;; *) echo 0 ;; esac; }
bootstrap_of() { case $1 in B|C|D|E) echo 1 ;; *) echo 0 ;; esac; }

cargo_dist() { # $1 = variant, $2 = target dir, $3 = RUSTFLAGS, rest = packages
  local v=$1 dir=$2 flags=$3; shift 3
  local extra=()
  if [ "$(bootstrap_of "$v")" = 1 ]; then export RUSTC_BOOTSTRAP=1; else unset RUSTC_BOOTSTRAP; fi
  if [ "$(buildstd_of "$v")" = 1 ]; then extra=(-Zbuild-std=std,panic_unwind); fi
  CARGO_TARGET_DIR="$dir" RUSTFLAGS="$flags" CARGO_PROFILE_DIST_STRIP=none \
    cargo build --profile dist --locked "$@" --target "$tgt" "${extra[@]}"
}

instrumented() { # $1 = variant
  local v=$1
  mkdir -p "$out/$v/raw"
  cargo_dist "$v" "$out/$v/inst" "-Cprofile-generate=$out/$v/raw $(rustflags_of "$v")" \
    -p tsrs_cli -p tsrs_testrunner -p tsrs_fourslash > "$out/$v/inst.log" 2>&1 || { tail -40 "$out/$v/inst.log"; return 1; }
}
final() { # $1 = variant
  local v=$1
  cargo_dist "$v" "$out/$v/final" "-Cprofile-use=$out/$v/merged.profdata -Clink-arg=-Wl,--emit-relocs $(rustflags_of "$v")" \
    -p tsrs_cli -p tsrs_testrunner -p tsrs_fourslash > "$out/$v/final.log" 2>&1 || { tail -40 "$out/$v/final.log"; return 1; }
}
parallel() { # $1 = function, rest = variants; at most $JOBS at a time
  local f=$1 pids=() v; shift
  for v in "$@"; do
    "$f" "$v" & pids+=($!)
    if [ ${#pids[@]} -ge "$JOBS" ]; then wait "${pids[0]}"; pids=("${pids[@]:1}"); fi
  done
  for v in "${pids[@]}"; do wait "$v"; done
}
# A fat-LTO link of tsrs-test takes a few GiB; three per variant.
mem_gb=$(awk '/MemTotal/ {printf "%d", $2 / 1048576}' /proc/meminfo)
JOBS=$(( mem_gb / 24 )); [ "$JOBS" -lt 1 ] && JOBS=1
echo "memory ${mem_gb} GiB: $JOBS variant builds at a time"
for v in "${variants[@]}"; do mkdir -p "$out/$v"; done

step "instrumented builds (${variants[*]})" parallel instrumented "${variants[@]}"
for v in "${variants[@]}"; do
  step "pgo-train.sh $v" bash -c ".github/scripts/pgo-train.sh '$out/$v/inst/$tgt/dist' '$out/$v/raw' '$work' > '$out/$v/train.log' 2>&1 || { tail -40 '$out/$v/train.log'; exit 1; }"
  "$profdata" merge -o "$out/$v/merged.profdata" "$out/$v"/raw/*.profraw
  { echo "== $v"; "$profdata" show "$out/$v/merged.profdata" | tail -5; } | tee -a "$out/profiles.txt"
  grep -E '^(all|conformance) ' "$out/$v/train.log" | tee -a "$out/profiles.txt" || true
  grep -E '^fourslash:' "$out/$v/train.log" | tee -a "$out/profiles.txt" || true
done
step "final builds (${variants[*]})" parallel final "${variants[@]}"
for v in "${variants[@]}"; do
  d=$out/$v/final/$tgt/dist
  split_eh=1; case $v in AN|DN) split_eh=0 ;; esac
  # The whole of bolt.sh, gates included (conformance and fourslash on the BOLT-optimized tsrs-test and tsrs-fourslash).
  step "BOLT $v (split-eh $split_eh)" bash -c "BOLT_SPLIT_EH=$split_eh .github/scripts/bolt.sh '$d' '$out/$v/bolt' '$work' > '$out/$v/bolt.log' 2>&1 || { tail -40 '$out/$v/bolt.log'; exit 1; }"
  grep -E '^(conformance|fourslash):' "$out/$v/bolt.log" | sed "s/^/  $v /" | tee -a "$out/profiles.txt" || true
  mkdir -p "$out/bin/$v"; cp "$d/tsrs" "$out/bin/$v/tsrs"
  {
    echo "== $v"
    file "$d/tsrs.prebolt" "$d/tsrs" | sed "s|$out/||"
    ls -l "$d/tsrs.prebolt" "$d/tsrs" | awk '{print $5, $9}' | sed "s|$out/||"
    size -A "$d/tsrs.prebolt" | awk '$1 ~ /^\.(text|rodata|data\.rel\.ro|rela\.dyn|got|eh_frame|gcc_except_table)$/ {printf "  prebolt %-18s %10d\n", $1, $2}'
    echo "  prebolt R_X86_64_RELATIVE dynamic relocations: $(readelf -r "$d/tsrs.prebolt" 2>/dev/null | grep -c R_X86_64_RELATIVE || true)"
  } | tee -a "$out/sizes.txt"
done
# The reference compiler's linking, for comparison (Go builds linux/amd64 executables without PIE by default).
find "$work/tsgo" -path '*linux-x64*' -name tsgo -type f -exec file {} \; 2>/dev/null | sed "s|$work/||" | tee -a "$out/sizes.txt" || true
cat "$out/times.txt"
