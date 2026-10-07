#!/usr/bin/env bash
# Memory-behaviour probe (notes/perf-memory-traffic-32.md): counters, per-symbol cycles and instructions, miss sites,
# c2c and huge-page coverage at 1 and 32 checkers. PROBE_PROJECTS from the workflow; MEMPROBE_CHECKERS overrides "1 32".
set -uo pipefail
PERF=${PERF:-$(ls /usr/lib/linux-tools/*/perf 2>/dev/null | head -1)}
: "${TSRS_BIN:?}" "${BENCH_WORK:?}" "${PROBE_OUT:?}"
mkdir -p "$PROBE_OUT"
root=$(pwd)
"$PERF" list > "$PROBE_OUT/perf-list.txt" 2>&1 || true
cat /sys/kernel/mm/transparent_hugepage/enabled /sys/kernel/mm/transparent_hugepage/defrag > "$PROBE_OUT/thp.txt" 2>&1 || true
dmesg 2>/dev/null | grep -i -E 'ibs|perf' | head -20 >> "$PROBE_OUT/thp.txt" || true
ls /sys/bus/event_source/devices/ >> "$PROBE_OUT/thp.txt" 2>&1 || true

EV_GEN=cycles:u,instructions:u,cache-misses,cache-references,LLC-load-misses,LLC-loads,dTLB-load-misses,dTLB-loads,page-faults,context-switches,cpu-migrations
EV_AMD=ls_any_fills_from_sys.local_l2,ls_any_fills_from_sys.local_ccx,ls_any_fills_from_sys.near_cache,ls_any_fills_from_sys.far_cache,ls_any_fills_from_sys.dram_io_all,ls_any_fills_from_sys.remote_cache,ls_dmnd_fills_from_sys.far_cache,ls_dmnd_fills_from_sys.near_cache,ls_dmnd_fills_from_sys.dram_io_all
EV_TLB=ls_l1_d_tlb_miss.all,ls_l1_d_tlb_miss.all_l2_miss,ls_l1_d_tlb_miss.tlb_reload_2m_l2_miss,ls_l1_d_tlb_miss.tlb_reload_4k_l2_miss,l2_cache_req_stat.ic_dc_miss_in_l2,l2_cache_req_stat.ic_dc_hit_in_l2,ls_st_commit_cancel2.st_commit_cancel_wcb_full,de_dis_dispatch_token_stalls1.load_queue_rsrc_stall

for name in ${PROBE_PROJECTS//,/ }; do
  read -r cwd proj < <(python3 - "$name" <<'EOF'
import json, sys
sys.path.insert(0, "bench")
import run as bench
from pathlib import Path
cfg = json.loads(Path("bench/projects.json").read_text())
p = next(p for p in cfg["projects"] if p["name"] == sys.argv[1])
cwd, proj = bench.project_path(cfg, p, Path("bench/.work"))
print(Path(cwd).resolve(), proj)
EOF
)
  flags=(-p "$proj" --noEmit --incremental false --pretty false)
  (cd "$cwd" && "$TSRS_BIN" "${flags[@]}" > /dev/null 2>&1) || true  # warm
  for k in ${MEMPROBE_CHECKERS:-1 32}; do
    tag="$name-k$k"
    for ev in "$EV_GEN" "$EV_AMD" "$EV_TLB"; do
      for rep in 1 2 3; do
        (cd "$cwd" && TSRS_ASSIGNMENT_STATS=times "$PERF" stat -x, -e "$ev" -o "$PROBE_OUT/$tag-stat-$rep.csv" --append -- \
          "$TSRS_BIN" "${flags[@]}" --extendedDiagnostics --checkers "$k" > "$PROBE_OUT/$tag-run-$rep.txt" 2>&1) || true
      done
    done
    # Peak RSS and huge-page coverage.
    (cd "$cwd" && "$TSRS_BIN" "${flags[@]}" --checkers "$k" > /dev/null 2>&1 & pid=$!
     best=""; bestrss=0
     while kill -0 $pid 2>/dev/null; do
       s=$(cat /proc/$pid/smaps_rollup 2>/dev/null) || break
       rss=$(printf '%s\n' "$s" | awk '/^Rss:/{print $2}')
       if [ -n "$rss" ] && [ "$rss" -gt "$bestrss" ]; then bestrss=$rss; best=$s; fi
       sleep 0.02
     done
     printf '%s\n' "$best" > "$PROBE_OUT/$tag-smaps-peak.txt")
    # Per-symbol cycles and instructions, checker threads only.
    comms=$(seq -s, -f 'checker-%g' 0 $((k > 1 ? k - 1 : 0)))
    (cd "$cwd" && "$PERF" record -F 4999 -e '{cycles:u,instructions:u}' -o "$root/perf-$tag.data" -- \
      "$TSRS_BIN" "${flags[@]}" --checkers "$k" > /dev/null 2>&1) || true
    "$PERF" report -i "$root/perf-$tag.data" --group --no-children --sort sym --stdio --percent-limit 0.15 \
      > "$PROBE_OUT/$tag-sym-all.txt" 2>&1 || true
    "$PERF" report -i "$root/perf-$tag.data" --group --no-children --sort sym --stdio --percent-limit 0.15 \
      --comms "$comms" > "$PROBE_OUT/$tag-sym-checkers.txt" 2>&1 || true
    "$PERF" report -i "$root/perf-$tag.data" --group --no-children --sort comm --stdio \
      > "$PROBE_OUT/$tag-comm.txt" 2>&1 || true
    if [ "$k" -gt 1 ]; then
      for ev in ls_any_fills_from_sys.far_cache ls_any_fills_from_sys.dram_io_all ls_l1_d_tlb_miss.all_l2_miss cache-misses; do
        (cd "$cwd" && "$PERF" record -e "$ev:u" -c 2003 -o "$root/perf-$tag-$ev.data" -- \
          "$TSRS_BIN" "${flags[@]}" --checkers "$k" > /dev/null 2>&1) || true
        "$PERF" report -i "$root/perf-$tag-$ev.data" --no-children --sort sym --stdio --percent-limit 0.3 \
          > "$PROBE_OUT/$tag-miss-$ev.txt" 2>&1 || true
      done
      (cd "$cwd" && timeout 300 "$PERF" c2c record -o "$root/c2c-$tag.data" -- \
        "$TSRS_BIN" "${flags[@]}" --checkers "$k" > /dev/null 2> "$PROBE_OUT/$tag-c2c-record.err") || true
      timeout 300 "$PERF" c2c report -i "$root/c2c-$tag.data" --stdio --full-symbols 2>&1 | head -400 \
        > "$PROBE_OUT/$tag-c2c.txt" || true
      (cd "$cwd" && timeout 300 "$PERF" mem record -o "$root/mem-$tag.data" -- \
        "$TSRS_BIN" "${flags[@]}" --checkers "$k" > /dev/null 2> "$PROBE_OUT/$tag-mem-record.err") || true
      timeout 300 "$PERF" mem report -i "$root/mem-$tag.data" --stdio --sort=mem,sym 2>&1 | head -200 \
        > "$PROBE_OUT/$tag-mem.txt" || true
    fi
  done
done
echo done > "$PROBE_OUT/done.txt"
