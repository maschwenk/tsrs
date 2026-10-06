#!/usr/bin/env bash
# Probe for .depot/workflows/perf-probe.yml (perf/frontend): program construction ("Parse time") on vscode by thread
# count, before (origin/main, built here from a stash of the uploaded diff) and after (TSRS_BIN), with the env-gated
# TSRS_FRONTEND_STATS breakdown, a flat perf profile of the front end at 64 threads, and bun's "loaded in" time.
#
# Environment (set by the workflow; set them yourself to run locally):
#   TSRS_BIN    tsrs binary          BUN_BIN   bun binary with `bun check`
#   BENCH_WORK  bench/.work with the projects set up (bench/run.py --setup-only --projects vscode)
#   PROBE_OUT   output directory
set -euo pipefail
: "${TSRS_BIN:?}" "${BENCH_WORK:?}" "${PROBE_OUT:?}"
mkdir -p "$PROBE_OUT"
repo=$(cd "$(dirname "$0")/../.." && pwd)
proj="$BENCH_WORK/solutions/vscode"
flags=(-p src --noEmit --incremental false --extendedDiagnostics --pretty false)
summary="$PROBE_OUT/summary.txt"

# The base binary: origin/main without the uploaded diff.
cp "$TSRS_BIN" /tmp/tsrs-new
base_bin=""
if (cd "$repo" && git stash -u -q && cargo build --release --locked -p tsrs_cli >/tmp/base-build.log 2>&1 && cp target/release/tsrs /tmp/tsrs-base && git stash pop -q); then
  base_bin=/tmp/tsrs-base
else
  echo "base build failed (see base-build.log)" | tee -a "$summary"; cp /tmp/base-build.log "$PROBE_OUT/" || true
fi
bins=()
[ -n "$base_bin" ] && bins+=("base:$base_bin")
bins+=("new:/tmp/tsrs-new")

cd "$proj"
echo "== tsrs: $(/tmp/tsrs-new --version); nproc $(nproc)" | tee "$summary"
/tmp/tsrs-new "${flags[@]}" --listFilesOnly > /dev/null 2>&1 || true   # warm the page cache

row() { # label file
  printf '%s: %s; %s\n' "$1" "$(cat "$2.time")" "$(grep -E '^(Config|Parse|Bind|Check|Total) time|parallel parse|sequential load|root file|collect files|verify options' "$2" | sed -E 's/ +/ /g' | tr '\n' ';')" | tee -a "$summary"
}

# 1. Front end only (--listFilesOnly), by rayon thread count, interleaving the binaries.
for t in 1 4 16 32 64; do
  for rep in 1 2 3; do
    for b in "${bins[@]}"; do
      name=${b%%:*}; bin=${b#*:}
      out="$PROBE_OUT/fe-$name-t$t-rep$rep.txt"
      /usr/bin/time -f "wall %e s, user %U s, sys %S s, maxrss %M KiB" -o "$out.time" \
        env RAYON_NUM_THREADS=$t "$bin" "${flags[@]}" --listFilesOnly > "$out" 2>&1 || true
      row "fe $name t=$t rep=$rep" "$out"
    done
  done
done

# 2. The same with the per-category breakdown (new binary only; the stats are new).
for t in 4 16 64; do
  out="$PROBE_OUT/fe-stats-new-t$t.txt"
  /usr/bin/time -f "wall %e s, user %U s, sys %S s, maxrss %M KiB" -o "$out.time" \
    env RAYON_NUM_THREADS=$t TSRS_FRONTEND_STATS=1 /tmp/tsrs-new "${flags[@]}" --listFilesOnly > "$out" 2>&1 || true
  { echo "fe-stats new t=$t: $(cat "$out.time")"; grep -E '^Program|^Parse time|^FS' "$out"; } | tee -a "$summary"
done

# 2b. Experiment: largest files first in the parallel phase (TSRS_FE_LPT=1), 64 and 16 threads, interleaved with off.
for t in 16 64; do
  for rep in 1 2 3 4; do
    for lpt in 0 1; do
      out="$PROBE_OUT/fe-lpt$lpt-t$t-rep$rep.txt"
      env_lpt=(); [ "$lpt" = 1 ] && env_lpt=(TSRS_FE_LPT=1)
      /usr/bin/time -f "wall %e s, user %U s, sys %S s, maxrss %M KiB" -o "$out.time" \
        env RAYON_NUM_THREADS=$t "${env_lpt[@]}" TSRS_FRONTEND_STATS=1 /tmp/tsrs-new "${flags[@]}" --listFilesOnly > "$out" 2>&1 || true
      printf 'fe lpt=%s t=%s rep=%s: %s; %s\n' "$lpt" "$t" "$rep" "$(cat "$out.time")" \
        "$(grep -E '^Parse time|parallel parse|longest job|job wall' "$out" | sed -E 's/ +/ /g' | tr '\n' ';')" | tee -a "$summary"
    done
  done
done

# 3. Full check at 64 and 16 checkers, both binaries.
for k in 16 64; do
  for rep in 1 2 3; do
    for b in "${bins[@]}"; do
      name=${b%%:*}; bin=${b#*:}
      out="$PROBE_OUT/full-$name-k$k-rep$rep.txt"
      /usr/bin/time -f "wall %e s, user %U s, sys %S s, maxrss %M KiB" -o "$out.time" \
        "$bin" "${flags[@]}" --checkers "$k" > "$out" 2>&1 || true
      printf 'full %s k=%s rep=%s: %s; %s; errors %s\n' "$name" "$k" "$rep" "$(cat "$out.time")" \
        "$(grep -E '^(Parse|Check|Total) time' "$out" | tr -s ' ' | tr '\n' ' ')" "$(grep -c 'error TS' "$out")" | tee -a "$summary"
    done
  done
done
# Output identity (full check) between the binaries: everything but the statistics block (its checker counters vary
# from run to run with work stealing, for the base binary too).
diags() { grep -v -E '^[A-Z][A-Za-z: -]*: +[0-9]' "$1" | grep -v -E '^(Program|Config|Checkers|Diagnostics|FS|Lazy|Statistics|List|Error summary|Mapped|some|Conditional|Augmented|Existence|Empty)'; }
if [ -n "$base_bin" ]; then
  for k in 16 64; do
    if diff <(diags "$PROBE_OUT/full-base-k$k-rep1.txt") <(diags "$PROBE_OUT/full-new-k$k-rep1.txt") > "$PROBE_OUT/full-k$k-diff.txt"; then
      echo "output identical: base vs new at $k checkers ($(diags "$PROBE_OUT/full-new-k$k-rep1.txt" | grep -c 'error TS') errors)" | tee -a "$summary"
    else
      echo "OUTPUT DIFFERS: base vs new at $k checkers (full-k$k-diff.txt)" | tee -a "$summary"
    fi
  done
fi

# 4. perf (best effort): flat profile of the front end at 64 threads, user + kernel symbols.
perf_bin=$(ls /usr/lib/linux-tools/*/perf 2>/dev/null | head -1 || true)
if [ -n "$perf_bin" ]; then
  sudo sysctl -q kernel.perf_event_paranoid=-1 kernel.kptr_restrict=0 || true
  for b in "${bins[@]}"; do
    name=${b%%:*}; bin=${b#*:}
    if sudo "$perf_bin" record -q -e cpu-clock -F 5000 -o "/tmp/perf-$name.data" -- "$bin" "${flags[@]}" --listFilesOnly > /dev/null 2>&1; then
      sudo "$perf_bin" report -i "/tmp/perf-$name.data" --stdio --no-children --sort dso,sym --percent-limit 0.4 2>/dev/null | head -120 > "$PROBE_OUT/perf-fe-$name.txt" || true
      echo "perf fe $name: $(grep -c . "$PROBE_OUT/perf-fe-$name.txt") lines" | tee -a "$summary"
    else
      echo "perf record failed for $name" | tee -a "$summary"
    fi
    sudo "$perf_bin" stat -e task-clock,context-switches,cpu-migrations,page-faults,instructions,cycles -o "$PROBE_OUT/perfstat-fe-$name-t64.txt" -- "$bin" "${flags[@]}" --listFilesOnly > /dev/null 2>&1 || true
    sudo "$perf_bin" stat -e task-clock,context-switches,cpu-migrations,page-faults,instructions,cycles -o "$PROBE_OUT/perfstat-fe-$name-t1.txt" -- env RAYON_NUM_THREADS=1 "$bin" "${flags[@]}" --listFilesOnly > /dev/null 2>&1 || true
  done
else
  echo "perf not installed" | tee -a "$summary"
fi

# 5. bun: load phase by threads.
if [ -n "${BUN_BIN:-}" ]; then
  echo "== bun: $("$BUN_BIN" --revision)" | tee -a "$summary"
  for t in 16 64; do
    for rep in 1 2 3; do
      out="$PROBE_OUT/bun-threads$t-rep$rep.txt"
      /usr/bin/time -f "wall %e s, user %U s, sys %S s, maxrss %M KiB" -o "$out.time" \
        "$BUN_BIN" check -p src --no-pretty --all --timing --threads "$t" > "$out" 2>&1 || true
      printf 'bun --threads %-3s rep %s: %s; %s\n' "$t" "$rep" "$(cat "$out.time")" "$(grep -E 'loaded in' "$out" | tail -1)" | tee -a "$summary"
    done
  done
fi
