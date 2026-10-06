#!/usr/bin/env bash
# Default probe for .depot/workflows/perf-probe.yml: tsrs's checker-count scaling curve on vscode on the 64-thread
# runner, with per-checker wall / thread CPU / stolen files (TSRS_ASSIGNMENT_STATS=times), user-space instructions and
# peak RSS (bench/count.py), the default checker count the machine gets, and head-to-head walls against bun check.
# Experiments replace or extend this script in their branch (notes/perf-checker-64.md ran three variants of it); keep
# everything worth keeping under $PROBE_OUT.
#
# Environment (set by the workflow; set them yourself to run locally):
#   TSRS_BIN    tsrs binary          BUN_BIN   bun binary with `bun check`
#   BENCH_WORK  bench/.work with the projects set up (bench/run.py --setup-only --projects vscode)
#   PROBE_OUT   output directory     TIME_BIN  GNU time (default /usr/bin/time; macOS needs a shim)
#
# Per measured run: the --extendedDiagnostics output with the stats in <name>.txt, {instructions, max_rss_bytes} in
# <name>.json, and /usr/bin/time in <name>.time (its wall includes python's startup for count.py; "Total time" in
# <name>.txt is the compiler's own clock).
set -euo pipefail
: "${TSRS_BIN:?}" "${BENCH_WORK:?}" "${PROBE_OUT:?}"
mkdir -p "$PROBE_OUT"
repo="$(cd "$(dirname "$TSRS_BIN")/../.." && pwd)"
count="$repo/bench/count.py"
proj="$BENCH_WORK/solutions/vscode"
cd "$proj"
flags=(-p src --noEmit --incremental false --extendedDiagnostics --pretty false)
summary="$PROBE_OUT/summary.txt"
time_bin="${TIME_BIN:-/usr/bin/time}"

echo "== tsrs: $("$TSRS_BIN" --version)" | tee "$summary"
"$TSRS_BIN" "${flags[@]}" > /dev/null 2>&1 || true   # warm the page cache

# run <name> [--checkers N] [ENV=VALUE...]: one measured run with the assignment stats.
run() {
  local name=$1; shift
  local checkers=()
  if [ "${1:-}" = "--checkers" ]; then checkers=(--checkers "$2"); shift 2; fi
  local base="$PROBE_OUT/$name"
  "$time_bin" -f "wall %e s, maxrss %M KiB" -o "$base.time" \
    env TSRS_ASSIGNMENT_STATS=times "$@" python3 "$count" "$base.json" -- "$TSRS_BIN" "${flags[@]}" "${checkers[@]}" > "$base.txt" 2>&1 || true
  local k; k=$(grep -m1 '^checker group cpu seconds:' "$base.txt" | awk '{print NF - 4}')
  printf '%-18s checkers=%-3s %s; %s; %s\n' "$name" "$k" "$(tail -1 "$base.time")" \
    "$(grep -E '^(Check|Total) time' "$base.txt" | tr -s ' ' | tr '\n' ' ')" "$(tr -d '{}"' < "$base.json")" | tee -a "$summary"
}

# The default checker count on this machine, then the curve.
for rep in 1 2 3; do run "default-rep$rep"; done
for k in 4 8 16 32 64; do
  for rep in 1 2 3; do run "k$k-rep$rep" --checkers "$k"; done
done
# Front-end instructions (no checking), for the duplicated-work column.
python3 "$count" "$PROBE_OUT/listfiles.json" -- "$TSRS_BIN" -p src --noEmit --incremental false --listFilesOnly > /dev/null 2>&1 || true
echo "listFilesOnly: $(cat "$PROBE_OUT/listfiles.json")" | tee -a "$summary"

# Head-to-head walls without --extendedDiagnostics or stats, 5 reps, interleaved.
echo "== walls (no stats, no extendedDiagnostics)" | tee -a "$summary"
for rep in 1 2 3 4 5; do
  for k in default 16 32 64; do
    checkers=(); [ "$k" = default ] || checkers=(--checkers "$k")
    t="$PROBE_OUT/wall-tsrs-$k-rep$rep.time"
    "$time_bin" -f "%e %M" -o "$t" "$TSRS_BIN" -p src --noEmit --incremental false --pretty false "${checkers[@]}" > /dev/null 2>&1 || true
    printf 'wall tsrs %-8s rep %s: %s s, %s KiB\n' "$k" "$rep" $(tail -1 "$t") | tee -a "$summary"
  done
  if [ -n "${BUN_BIN:-}" ]; then
    t="$PROBE_OUT/wall-bun-default-rep$rep.time"
    "$time_bin" -f "%e %M" -o "$t" "$BUN_BIN" check -p src --no-pretty --all > /dev/null 2>&1 || true
    printf 'wall bun  %-8s rep %s: %s s, %s KiB\n' default "$rep" $(tail -1 "$t") | tee -a "$summary"
  fi
done

if [ -n "${BUN_BIN:-}" ]; then
  echo "== bun: $("$BUN_BIN" --revision)" | tee -a "$summary"
  for t in 16 64; do
    for rep in 1 2 3; do
      out="$PROBE_OUT/bun-threads$t-rep$rep.txt"
      "$time_bin" -f "wall %e s, maxrss %M KiB" -o "$out.time" \
        "$BUN_BIN" check -p src --no-pretty --all --timing --threads "$t" > "$out" 2>&1 || true
      printf 'bun --threads %-3s rep %s: %s; %s\n' "$t" "$rep" "$(tail -1 "$out.time")" "$(grep -E 'loaded|checked' "$out" | tail -2 | tr '\n' ' ')" \
        | tee -a "$summary"
    done
  done
fi
