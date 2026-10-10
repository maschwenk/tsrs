#!/usr/bin/env bash
# Does a non-PIE x86_64 build still unwind? The release dry run of the non-PIE build failed the fourslash gate on the
# BOLT-optimized tsrs-fourslash ("fatal runtime error: failed to initiate panic, error 3"). This builds tsrs, tsrs-test
# and tsrs-fourslash as release.yml does (non-PIE), runs the fourslash gate on the PGO binary before BOLT, then on
# tsrs-fourslash BOLT-optimized with bolt.sh's flags and with parts of them left out.
#
#   nopie-unwind.sh <out dir> <bench work dir>
set -euo pipefail
mkdir -p "$1"
out=$(cd "$1" && pwd)
work=$(cd "$2" && pwd)
tgt=x86_64-unknown-linux-gnu
nopie="-Crelocation-model=static -Clink-arg=-no-pie"
profdata="$(rustc --print sysroot)/lib/rustlib/$tgt/bin/llvm-profdata"
summary=$out/summary.txt
: > "$summary"
log() { echo "$*" | tee -a "$summary"; }

CARGO_TARGET_DIR=$out/inst RUSTFLAGS="-Cprofile-generate=$out/raw $nopie" CARGO_PROFILE_DIST_STRIP=none \
  cargo build --profile dist --locked -p tsrs_cli -p tsrs_testrunner -p tsrs_fourslash --target $tgt > "$out/inst.log" 2>&1 \
  || { tail -40 "$out/inst.log"; exit 1; }
.github/scripts/pgo-train.sh "$out/inst/$tgt/dist" "$out/raw" "$work" > "$out/train.log" 2>&1 || { tail -40 "$out/train.log"; exit 1; }
"$profdata" merge -o "$out/merged.profdata" "$out"/raw/*.profraw
CARGO_TARGET_DIR=$out/final RUSTFLAGS="-Cprofile-use=$out/merged.profdata -Clink-arg=-Wl,--emit-relocs $nopie" \
  CARGO_PROFILE_DIST_STRIP=none cargo build --profile dist --locked -p tsrs_cli -p tsrs_testrunner -p tsrs_fourslash \
  --target $tgt > "$out/final.log" 2>&1 || { tail -40 "$out/final.log"; exit 1; }
d=$out/final/$tgt/dist
file "$d/tsrs-fourslash" | tee -a "$summary"

gate() { # $1 = name, $2 = tsrs-fourslash binary
  local st
  TSRS_FOURSLASH=$2 TSRS_FOURSLASH_RESULTS=$out/fs-$1 .github/scripts/fourslash-gate.sh > "$out/gate-$1.txt" 2>&1 && st=pass || st=FAIL
  log "$1: gate $st; $(grep '^fourslash:' "$out/gate-$1.txt" | head -1); crashes $(grep -c 'crash' "$out/fs-$1/fail.txt" 2>/dev/null || true); 'failed to initiate panic' $(grep -c 'failed to initiate panic' "$out/fs-$1/fail.txt" 2>/dev/null || true)"
}
gate instrumented "$out/inst/$tgt/dist/tsrs-fourslash"
gate prebolt "$d/tsrs-fourslash"

mkdir -p "$out/fsprof"
llvm-bolt "$d/tsrs-fourslash" -instrument -o "$out/fs.inst" --instrumentation-file="$out/fsprof/prof" \
  --instrumentation-file-append-pid > "$out/bolt-inst.log" 2>&1 || { cat "$out/bolt-inst.log"; exit 1; }
TSRS_FOURSLASH_RESULTS=$out/train-fs "$out/fs.inst" run > /dev/null 2>&1 || true
merge-fdata "$out"/fsprof/prof* > "$out/fs.fdata"
grep -c 'failed to initiate panic' "$out/train-fs/fail.txt" 2>/dev/null | sed 's/^/instrumented BOLT binary, failed to initiate panic: /' | tee -a "$summary" || true

common=(-reorder-blocks=ext-tsp -reorder-functions=cdsort -use-gnu-stack -update-debug-sections)
declare -A sets=(
  [full]="-split-functions -split-all-cold -split-eh -icf=1"
  [no-split-eh]="-split-functions -split-all-cold -icf=1"
  [no-split]="-icf=1"
  [no-icf]="-split-functions -split-all-cold -split-eh"
  [reorder-only]=""
)
for v in full no-split-eh no-split no-icf reorder-only; do
  read -ra extra <<< "${sets[$v]}"
  llvm-bolt "$d/tsrs-fourslash" -o "$out/fs-$v.bolt" -data="$out/fs.fdata" "${common[@]}" "${extra[@]}" \
    > "$out/bolt-$v.log" 2>&1 || { log "$v: llvm-bolt failed"; tail -20 "$out/bolt-$v.log"; continue; }
  grep -E 'BOLT-(WARNING|ERROR)' "$out/bolt-$v.log" | sort | uniq -c | head -8 >> "$summary" || true
  gate "bolt-$v" "$out/fs-$v.bolt"
done
cat "$summary"
