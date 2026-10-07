#!/usr/bin/env bash
# Probe for .depot/workflows/perf-probe.yml: wall time and peak RSS of tsrs at checker counts between the 32-checker
# default and 64 on the 64-thread runner, per project, interleaved (count inner, rep outer), to find the largest
# count whose peak memory still ties `bun check`'s (notes/perf-checker-64.md derived the 32 cap when 32 and 64 were
# equal in wall time; after the inference memo and the module affinity 64 is 15-19% faster on the large projects).
#
# PROBE_ARGS: --counts 32,40,48,56,64 --reps 5 (defaults). Environment from the workflow: TSRS_BIN, BENCH_WORK,
# PROBE_OUT, PROBE_PROJECTS (comma-separated bench/projects.json names). Writes $PROBE_OUT/summary.txt (one line per
# run) and $PROBE_OUT/medians.tsv (project, checkers, median wall s, min, max, median peak MiB, median check s).
set -euo pipefail
: "${TSRS_BIN:?}" "${BENCH_WORK:?}" "${PROBE_OUT:?}"
counts="32,40,48,56,64"
reps=5
args=(${PROBE_ARGS:-})
for ((i = 0; i < ${#args[@]}; i++)); do
  case "${args[i]}" in
    --counts) counts="${args[i + 1]}"; i=$((i + 1)) ;;
    --reps) reps="${args[i + 1]}"; i=$((i + 1)) ;;
  esac
done
mkdir -p "$PROBE_OUT"
root="$(cd "$(dirname "$0")/../.." && pwd)"
echo "== tsrs: $("$TSRS_BIN" --version); counts $counts; reps $reps; projects ${PROBE_PROJECTS:-vscode}" | tee "$PROBE_OUT/summary.txt"
IFS=',' read -r -a names <<< "${PROBE_PROJECTS:-vscode}"
IFS=',' read -r -a ks <<< "$counts"
for name in "${names[@]}"; do
  proj=$(python3 -c 'import json,sys; print(next(p["project"] for p in json.load(open(sys.argv[1])) if p["name"] == sys.argv[2]))' "$root/bench/projects.json" "$name")
  dir="$BENCH_WORK/solutions/$name"
  [ -d "$dir" ] || { echo "no checkout for $name at $dir" | tee -a "$PROBE_OUT/summary.txt"; continue; }
  cd "$dir"
  flags=(-p "$proj" --noEmit --incremental false --extendedDiagnostics --pretty false)
  "$TSRS_BIN" "${flags[@]}" > /dev/null 2>&1 || true   # warm the page cache
  for rep in $(seq 1 "$reps"); do
    for k in "${ks[@]}"; do
      out="$PROBE_OUT/$name-checkers$k-rep$rep.txt"
      /usr/bin/time -f "%e %M" -o "$out.time" "$TSRS_BIN" "${flags[@]}" --checkers "$k" > "$out" 2>&1 || true
      read -r wall kib < "$out.time"
      check=$(grep -E '^Check time' "$out" | tr -s ' ' | awk '{print $3}' | tr -d 's')
      printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$name" "$k" "$rep" "$wall" "$kib" "${check:-}" >> "$PROBE_OUT/runs.tsv"
      printf '%-18s --checkers %-3s rep %s: wall %s s, peak %s MiB, check %s s\n' "$name" "$k" "$rep" "$wall" "$((kib / 1024))" "${check:-?}" | tee -a "$PROBE_OUT/summary.txt"
    done
  done
done
python3 - "$PROBE_OUT/runs.tsv" "$PROBE_OUT/medians.tsv" <<'EOF'
import collections, statistics, sys
runs = collections.defaultdict(list)
for line in open(sys.argv[1]):
    name, k, rep, wall, kib, check = line.rstrip("\n").split("\t")
    runs[(name, int(k))].append((float(wall), int(kib) / 1024, float(check) if check else None))
with open(sys.argv[2], "w") as f:
    f.write("project\tcheckers\twall_median_s\twall_min\twall_max\tpeak_median_MiB\tcheck_median_s\n")
    for (name, k), rows in sorted(runs.items()):
        walls = [r[0] for r in rows]; peaks = [r[1] for r in rows]; checks = [r[2] for r in rows if r[2] is not None]
        f.write(f"{name}\t{k}\t{statistics.median(walls):.3f}\t{min(walls):.3f}\t{max(walls):.3f}\t{statistics.median(peaks):.0f}\t"
                f"{statistics.median(checks) if checks else float('nan'):.3f}\n")
print(open(sys.argv[2]).read())
EOF
cp "$PROBE_OUT/medians.tsv" "$PROBE_OUT/medians.txt"
