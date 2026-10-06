#!/usr/bin/env bash
# Probe 3 (perf/frontend): leaf size of the parallel parse (TSRS_FE_MAXLEN) with and without largest-first order
# (TSRS_FE_LPT), on vscode, 64 and 16 threads, interleaved. See tools/perf/probe.sh history for the full probe.
set -euo pipefail
: "${TSRS_BIN:?}" "${BENCH_WORK:?}" "${PROBE_OUT:?}"
mkdir -p "$PROBE_OUT"
proj="$BENCH_WORK/solutions/vscode"
flags=(-p src --noEmit --incremental false --extendedDiagnostics --pretty false)
summary="$PROBE_OUT/summary.txt"
cd "$proj"
echo "== tsrs: $("$TSRS_BIN" --version); nproc $(nproc)" | tee "$summary"
"$TSRS_BIN" "${flags[@]}" --listFilesOnly > /dev/null 2>&1 || true

variants=("inf:" "32:TSRS_FE_MAXLEN=32" "8:TSRS_FE_MAXLEN=8" "2:TSRS_FE_MAXLEN=2" "8lpt:TSRS_FE_MAXLEN=8 TSRS_FE_LPT=1" "2lpt:TSRS_FE_MAXLEN=2 TSRS_FE_LPT=1" "inflpt:TSRS_FE_LPT=1")
for t in 64 16; do
  for rep in 1 2 3 4 5; do
    for v in "${variants[@]}"; do
      name=${v%%:*}; envs=${v#*:}
      out="$PROBE_OUT/fe-$name-t$t-rep$rep.txt"
      /usr/bin/time -f "wall %e s, user %U s, sys %S s, maxrss %M KiB" -o "$out.time" \
        env RAYON_NUM_THREADS=$t $envs TSRS_FRONTEND_STATS=1 "$TSRS_BIN" "${flags[@]}" --listFilesOnly > "$out" 2>&1 || true
      printf 'fe %-7s t=%s rep=%s: %s; %s\n' "$name" "$t" "$rep" "$(cat "$out.time")" \
        "$(grep -E '^Parse time|parallel parse|longest job|job wall|thread cpu' "$out" | sed -E 's/ +/ /g; s/Program: +//; s/stats: //' | tr '\n' ';')" | tee -a "$summary"
    done
  done
done

# Full check at 64 checkers for the best guesses (default vs max_len 8 vs 8+lpt), 3 reps each.
for rep in 1 2 3; do
  for v in "inf:" "8:TSRS_FE_MAXLEN=8" "8lpt:TSRS_FE_MAXLEN=8 TSRS_FE_LPT=1"; do
    name=${v%%:*}; envs=${v#*:}
    out="$PROBE_OUT/full-$name-k64-rep$rep.txt"
    /usr/bin/time -f "wall %e s, user %U s, sys %S s, maxrss %M KiB" -o "$out.time" \
      env $envs "$TSRS_BIN" "${flags[@]}" --checkers 64 > "$out" 2>&1 || true
    printf 'full %-5s k=64 rep=%s: %s; %s; errors %s\n' "$name" "$rep" "$(cat "$out.time")" \
      "$(grep -E '^(Parse|Check|Total) time' "$out" | tr -s ' ' | tr '\n' ' ')" "$(grep -c 'error TS' "$out")" | tee -a "$summary"
  done
done
