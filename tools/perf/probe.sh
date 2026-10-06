#!/usr/bin/env bash
# Probe for notes/perf-checker-64.md (checker scaling 16 -> 64 on the 64-thread runner, vscode). Experiments replace or
# extend this script in their branch; keep everything worth keeping under $PROBE_OUT.
#
# Environment (set by .depot/workflows/perf-probe.yml; set them yourself to run locally):
#   TSRS_BIN    tsrs binary          BUN_BIN   bun binary with `bun check`
#   BENCH_WORK  bench/.work with the projects set up (bench/run.py --setup-only --projects vscode)
#   PROBE_OUT   output directory
#
# Per run: the full --extendedDiagnostics output with TSRS_ASSIGNMENT_STATS=times (per-checker wall / thread CPU /
# stolen files) in <name>.txt, user-space instructions and peak RSS (bench/count.py) in <name>.json, /usr/bin/time in
# <name>.time (its wall includes python's startup for count.py; "Total time" in <name>.txt is the compiler's own).
set -euo pipefail
: "${TSRS_BIN:?}" "${BENCH_WORK:?}" "${PROBE_OUT:?}"
mkdir -p "$PROBE_OUT"
repo="$(cd "$(dirname "$TSRS_BIN")/../.." && pwd)"
count="$repo/bench/count.py"
proj="$BENCH_WORK/solutions/vscode"
cd "$proj"
flags=(-p src --noEmit --incremental false --extendedDiagnostics --pretty false)
summary="$PROBE_OUT/summary.txt"

echo "== tsrs: $("$TSRS_BIN" --version)" | tee "$summary"
"$TSRS_BIN" "${flags[@]}" > /dev/null 2>&1 || true   # warm the page cache

# run <name> <checkers> [ENV=VALUE...]: one measured run with the assignment stats.
run() {
  local name=$1 k=$2; shift 2
  local base="$PROBE_OUT/$name"
  "${TIME_BIN:-/usr/bin/time}" -f "wall %e s, maxrss %M KiB" -o "$base.time" \
    env TSRS_ASSIGNMENT_STATS=times "$@" python3 "$count" "$base.json" -- "$TSRS_BIN" "${flags[@]}" --checkers "$k" > "$base.txt" 2>&1 || true
  printf '%-28s k=%-3s %s; %s; %s\n' "$name" "$k" "$(cat "$base.time")" \
    "$(grep -E '^(Check|Total) time' "$base.txt" | tr -s ' ' | tr '\n' ' ')" "$(tr -d '{}"' < "$base.json")" | tee -a "$summary"
}

# Run 2 (notes/perf-checker-64.md): visiting order x stealing side, 3 reps each.
#   order:  program (today), weight (heaviest first), heavy (files above 4x the queue mean first, the rest program order)
#   steal:  back (today), front (the heaviest remaining), heavy (the front while it is a heavy file, then the back)
for k in 16 24 32 48 64; do
  for rep in 1 2 3; do
    run "heavy-heavy-k$k-rep$rep" "$k" TSRS_CHECKER_FILE_ORDER=heavy TSRS_CHECKER_STEAL=heavy
    run "weight-front-k$k-rep$rep" "$k" TSRS_CHECKER_FILE_ORDER=weight TSRS_CHECKER_STEAL=front
  done
done
for k in 32 48 64; do
  for rep in 1 2 3; do run "weight-heavy-k$k-rep$rep" "$k" TSRS_CHECKER_FILE_ORDER=weight TSRS_CHECKER_STEAL=heavy; done
done
for k in 16 24 48; do
  for rep in 1 2 3; do run "heavy-back-k$k-rep$rep" "$k" TSRS_CHECKER_FILE_ORDER=heavy TSRS_CHECKER_STEAL=back; done
done
run "filetimes-heavy-heavy-k48" 48 TSRS_CHECKER_FILE_ORDER=heavy TSRS_CHECKER_STEAL=heavy TSRS_FILE_TIMES="$PROBE_OUT/filetimes-heavy-heavy-k48.tsv"
run "filetimes-weight-front-k48" 48 TSRS_CHECKER_FILE_ORDER=weight TSRS_CHECKER_STEAL=front TSRS_FILE_TIMES="$PROBE_OUT/filetimes-weight-front-k48.tsv"

# Headline walls without --extendedDiagnostics or stats, 5 reps, interleaved.
echo "== headline walls (no stats, no extendedDiagnostics)" | tee -a "$summary"
for rep in 1 2 3 4 5; do
  for k in 32 48 64; do
    for variant in program:back heavy:heavy weight:front; do
      order=${variant%%:*}; side=${variant##*:}
      t="$PROBE_OUT/wall-$order-$side-k$k-rep$rep.time"
      "${TIME_BIN:-/usr/bin/time}" -f "%e %M" -o "$t" env TSRS_CHECKER_FILE_ORDER=$order TSRS_CHECKER_STEAL=$side "$TSRS_BIN" -p src --noEmit --incremental false --pretty false --checkers "$k" > /dev/null 2>&1 || true
      printf 'wall %-14s k=%-3s rep %s: %s s, %s KiB\n' "$order/$side" "$k" "$rep" $(tail -1 "$t") | tee -a "$summary"
    done
  done
done

if [ -n "${BUN_BIN:-}" ]; then
  echo "== bun: $("$BUN_BIN" --revision)" | tee -a "$summary"
  for t in 32 64; do
    for rep in 1 2 3; do
      out="$PROBE_OUT/bun-threads$t-rep$rep.txt"
      "${TIME_BIN:-/usr/bin/time}" -f "wall %e s, maxrss %M KiB" -o "$out.time" \
        "$BUN_BIN" check -p src --no-pretty --all --timing --threads "$t" > "$out" 2>&1 || true
      printf 'bun --threads %-3s rep %s: %s; %s\n' "$t" "$rep" "$(cat "$out.time")" "$(grep -E 'loaded|checked' "$out" | tail -2 | tr '\n' ' ')" \
        | tee -a "$summary"
    done
  done
fi
