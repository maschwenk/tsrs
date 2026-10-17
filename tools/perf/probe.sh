#!/usr/bin/env bash
# Default probe for .github/workflows/perf-probe.yml: the checker-count scaling curve of tsrs on vscode with per-checker
# CPU times, plus bun check's load/check split, on the selected runner. Experiments replace or extend this script in
# their branch; keep everything worth keeping under $PROBE_OUT.
#
# The timed tsrs runs leave --extendedDiagnostics off: it adds end-of-run work (tsgo forces two GCs and builds every
# file's line map; tsrs sums its counters across checkers). One untimed run per checker count passes it with
# TSRS_ASSIGNMENT_STATS=1 for the phase times and the per-checker report.
#
# Environment (set by the workflow; set them yourself to run locally):
#   TSRS_BIN    tsrs binary          BUN_BIN   bun binary with `bun check`
#   BENCH_WORK  bench/.work with the projects set up (bench/run.py --setup-only --projects vscode)
#   PROBE_OUT   output directory
set -euo pipefail
: "${TSRS_BIN:?}" "${BENCH_WORK:?}" "${PROBE_OUT:?}"
mkdir -p "$PROBE_OUT"
proj="$BENCH_WORK/solutions/vscode"
cd "$proj"
flags=(--project src --noEmit --incremental false --pretty false)

echo "== tsrs: $("$TSRS_BIN" --version)" | tee "$PROBE_OUT/summary.txt"
"$TSRS_BIN" "${flags[@]}" > /dev/null 2>&1 || true   # warm the page cache

cpus=$(nproc)
echo "== runner: $cpus vCPU" | tee -a "$PROBE_OUT/summary.txt"
for k in 4 8 16 32 64; do
  [ "$k" -le "$cpus" ] || continue
  for rep in 1 2 3; do
    out="$PROBE_OUT/tsrs-checkers$k-rep$rep.txt"
    /usr/bin/time --format "wall %e s, maxrss %M KiB" --output "$out.time" \
      "$TSRS_BIN" "${flags[@]}" --checkers "$k" > "$out" 2>&1 || true
    printf 'tsrs --checkers %-3s rep %s: %s\n' "$k" "$rep" "$(cat "$out.time")" | tee --append "$PROBE_OUT/summary.txt"
  done
  out="$PROBE_OUT/tsrs-checkers$k-stats.txt"
  TSRS_ASSIGNMENT_STATS=1 "$TSRS_BIN" "${flags[@]}" --extendedDiagnostics --checkers "$k" > "$out" 2>&1 || true
  printf 'tsrs --checkers %-3s stats: %s\n' "$k" \
    "$(grep --extended-regexp '^(Parse|Check|Total) time' "$out" | tr --squeeze-repeats ' ' | tr '\n' ' ')" \
    | tee --append "$PROBE_OUT/summary.txt"
done

if [ -n "${BUN_BIN:-}" ]; then
  echo "== bun: $("$BUN_BIN" --revision)" | tee -a "$PROBE_OUT/summary.txt"
  for t in 4 8 16 32 64; do
    [ "$t" -le "$cpus" ] || continue
    for rep in 1 2 3; do
      out="$PROBE_OUT/bun-threads$t-rep$rep.txt"
      /usr/bin/time -f "wall %e s, maxrss %M KiB" -o "$out.time" \
        "$BUN_BIN" check -p src --no-pretty --all --timing --threads "$t" > "$out" 2>&1 || true
      printf 'bun --threads %-3s rep %s: %s; %s\n' "$t" "$rep" "$(cat "$out.time")" "$(grep -E 'loaded|checked' "$out" | tail -2 | tr '\n' ' ')" \
        | tee -a "$PROBE_OUT/summary.txt"
    done
  done
fi
