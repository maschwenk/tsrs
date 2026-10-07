#!/usr/bin/env bash
# A/B probe (notes/perf-serial-steps.md, not committed): base and new built in this directory with --profile dist,
# base = this working copy with tools/perf/ab.patch reversed. Then tools/perf/abprobe.py runs them interleaved.
set -euo pipefail
PERF=${PERF:-$(ls /usr/lib/linux-tools/*/perf 2>/dev/null | head -1)}; export PERF
: "${BENCH_WORK:?}" "${PROBE_OUT:?}"
mkdir -p "$PROBE_OUT"
root=$(pwd)
if [ "${PROFILE_ONLY:-0}" != 1 ]; then
cargo build --profile dist --locked -p tsrs_cli 2>&1 | tail -2
cp target/dist/tsrs "$PROBE_OUT/../tsrs-new"
git apply -R tools/perf/ab.patch
cargo build --profile dist --locked -p tsrs_cli 2>&1 | tail -2
cp target/dist/tsrs "$PROBE_OUT/../tsrs-base"
git apply tools/perf/ab.patch
python3 tools/perf/abprobe.py --base "$PROBE_OUT/../tsrs-base" --new "$PROBE_OUT/../tsrs-new" --work "$BENCH_WORK" \
  --out "$PROBE_OUT" ${PROBE_ARGS:-}
fi

# Program-thread profile of the new binary built with frame pointers (FP_PROFILE=0 skips it).
if [ "${FP_PROFILE:-0}" = 1 ] && [ -n "${PERF:-}" ]; then
  RUSTFLAGS="-C force-frame-pointers=yes" cargo build --release --locked -p tsrs_cli --target-dir target/fp 2>&1 | tail -2
  proj="$BENCH_WORK/solutions/vscode"
  for rep in 1 2 3; do
    (cd "$proj" && "$PERF" record -F 20000 -g --call-graph fp -o "$PROBE_OUT/../perf-$rep.data" -- \
      "$root/target/fp/release/tsrs" -p src --noEmit --incremental false --pretty false --extendedDiagnostics \
      > "$PROBE_OUT/fp-run-$rep.txt" 2>&1 || true)
    "$PERF" script -i "$PROBE_OUT/../perf-$rep.data" -F comm,tid,time,ip,sym --comms tsrs 2>/dev/null \
      | python3 tools/perf/fpprofile.py --hz 20000 ${FP_REGIONS:-get_processed_files create_checkers compute_associations \
        verify_compiler_options get_program_diagnostics get_global_diagnostics new_program} --timeline 3 > "$PROBE_OUT/fp-profile-$rep.txt" || true
  done
  a2l=$(ls "$(rustc --print sysroot)"/lib/rustlib/*/bin/llvm-addr2line 2>/dev/null | head -1 || true)
  for sym in ${FP_SRCLINE:-get_processed_files}; do
    name=$(printf '%s' "$sym" | tr -c 'A-Za-z0-9_\n' '_')
    for rep in 1 2 3; do
      "$PERF" script -i "$PROBE_OUT/../perf-$rep.data" --max-stack 1 -F comm,tid,ip,sym,symoff --comms tsrs 2>/dev/null
    done | grep -F "$sym" | tee "$PROBE_OUT/fp-samples-$name.txt" | python3 tools/perf/srclines.py "$root/target/fp/release/tsrs" "$sym" "${a2l:-llvm-addr2line}" \
      > "$PROBE_OUT/fp-srcline-$name.txt" 2>&1 || true
  done
fi
