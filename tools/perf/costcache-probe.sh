#!/usr/bin/env bash
# perf-probe.yml script for the default cost cache: t3code-server, mui-docs, supabase-studio and vscode at 8, 16 and
# 32 checkers, three arms interleaved (cache: the branch default, warm after an untimed run per project and count;
# weights: cache plus TSRS_COST_CACHE_WEIGHTS=1; nocache: --noCheckerCostCache), REPS timed runs each. Records wall
# and peak RSS, Check time, per-checker rows (TSRS_ASSIGNMENT_STATS=times), user-space instructions (perf stat when
# available) and a hash of the diagnostics. Summary in $PROBE_OUT/summary.md.
set -uo pipefail
: "${TSRS_BIN:?}" "${BENCH_WORK:?}" "${PROBE_OUT:?}"
mkdir -p "$PROBE_OUT"
REPS="${PROBE_REPS:-7}"
COUNTS="${PROBE_ARGS:-${PROBE_COUNTS:-8 16}}"   # checker counts; perf-probe.yml passes probe_args as PROBE_ARGS
declare -A DIRS=( [t3code-server]=apps/server [mui-docs]=docs [supabase-studio]=apps/studio [vscode]=src )
PERF=""; for p in /usr/lib/linux-tools/*/perf /usr/bin/perf; do [ -x "$p" ] && { PERF="$p"; break; }; done
echo "tsrs: $("$TSRS_BIN" --version)  runner: $(nproc) vCPU  perf: ${PERF:-none}  reps: $REPS" | tee "$PROBE_OUT/summary.txt"
flags=(--noEmit --incremental false --extendedDiagnostics --pretty false)
run_one() { # project arm count rep
  local proj=$1 arm=$2 k=$3 rep=$4 out="$PROBE_OUT/$1-$2-c$3-rep$4"
  local -a env=(TSRS_ASSIGNMENT_STATS=times) extra=()
  case "$arm" in weights) env+=(TSRS_COST_CACHE_WEIGHTS=1);; nocache) extra+=(--noCheckerCostCache);; esac
  local cmd=(env "${env[@]}" "$TSRS_BIN" -p "$BENCH_WORK/solutions/$proj/${DIRS[$proj]}" "${flags[@]}" --checkers "$k" "${extra[@]}")
  if [ -n "$PERF" ]; then cmd=("$PERF" stat -e instructions:u -x, -o "$out.perf" -- "${cmd[@]}"); fi
  /usr/bin/time -f "%e %M" -o "$out.time" "${cmd[@]}" > "$out.out" 2> "$out.err" || true
  grep -E "error TS" "$out.out" | sort | sha256sum | cut -c1-16 > "$out.hash"
  printf '%s %s c%s rep%s: wall/peak %s; %s; hash %s\n' "$proj" "$arm" "$k" "$rep" "$(cat "$out.time")" \
    "$(grep -E '^Check time' "$out.out" | tr -s ' ')" "$(cat "$out.hash")" | tee -a "$PROBE_OUT/summary.txt"
}
for proj in "${!DIRS[@]}"; do
  [ -d "$BENCH_WORK/solutions/$proj" ] || { echo "skip $proj (not set up)" | tee -a "$PROBE_OUT/summary.txt"; continue; }
  for k in $COUNTS; do
    for arm in cache weights nocache; do run_one "$proj" "$arm" "$k" 0 >/dev/null; done   # warm-up: writes the cache for this count
    for rep in $(seq 1 "$REPS"); do
      case $((rep % 3)) in 0) order="cache weights nocache";; 1) order="weights nocache cache";; *) order="nocache cache weights";; esac
      for arm in $order; do run_one "$proj" "$arm" "$k" "$rep"; done
    done
  done
done
python3 - "$PROBE_OUT" "$REPS" <<'PY'
import sys, glob, os, re, statistics as st
out, reps = sys.argv[1], int(sys.argv[2])
rows = {}
for t in glob.glob(f"{out}/*-rep[1-9]*.time"):
    base = t[:-5]; m = re.match(r".*/(.+?)-(cache|weights|nocache)-c(\d+)-rep(\d+)$", base)
    if not m: continue
    proj, arm, k, rep = m.group(1), m.group(2), int(m.group(3)), int(m.group(4))
    last = [l for l in open(t).read().splitlines() if l.strip()][-1]   # GNU time prepends an exit-status line when tsrs exits non-zero
    wall, peak = last.split()[:2]
    chk = re.search(r"Check time:\s+([\d.]+)s", open(base + ".out").read())
    ins = None
    if os.path.exists(base + ".perf"):
        mm = re.search(r"^(\d+),,instructions:u", open(base + ".perf").read(), re.M); ins = int(mm.group(1)) if mm else None
    slow = None
    err = open(base + ".err").read() + open(base + ".out").read()
    ms = re.search(r"slowest\s*/\s*mean[^\d]*([\d.]+)", err)
    if ms: slow = float(ms.group(1))
    rows.setdefault((proj, k, arm), []).append((float(wall), float(peak) / 1024, float(chk.group(1)) if chk else None, ins, slow, open(base + ".hash").read().strip()))
def med(xs): xs = [x for x in xs if x is not None]; return st.median(xs) if xs else None
lines = ["| project | checkers | arm | wall s | check s | instr G | slowest/mean | peak MiB | hashes |", "|---|---:|---|---:|---:|---:|---:|---:|---|"]
for (proj, k, arm) in sorted(rows):
    r = rows[(proj, k, arm)]
    f = lambda v, fmt: ("" if v is None else fmt % v)
    lines.append(f"| {proj} | {k} | {arm} | {f(med([x[0] for x in r]), '%.3f')} | {f(med([x[2] for x in r]), '%.3f')} | {f((med([x[3] for x in r]) or 0)/1e9 if any(x[3] for x in r) else None, '%.2f')} | {f(med([x[4] for x in r]), '%.2f')} | {f(med([x[1] for x in r]), '%.0f')} | {','.join(sorted(set(x[5] for x in r)))} |")
open(f"{out}/summary.md", "w").write("\n".join(lines) + "\n"); print("\n".join(lines))
PY
