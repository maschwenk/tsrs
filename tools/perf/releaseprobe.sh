#!/usr/bin/env bash
# Probe for .depot/workflows/perf-probe.yml: the published tsrs releases against each other on vscode, on one
# 64-vCPU machine, with tsgo and `bun check` beside them. Each release's binary is the one npm ships
# (@maschwenk/tsrs-linux-x64@<version>, the PGO + BOLT dist build of its tag), so this measures what users got, not a
# rebuild. Three modes per release, interleaved across releases within each rep: each release at its own default
# (0.4.0 and earlier chose 8 checkers on this machine, 0.5.0 and later 32), `--checkers 32` (the same thread count for
# every release), and `--singleThreaded` (the deterministic trend of the single-thread code).
#
# PROBE_ARGS: --versions 0.3.0,0.4.0,0.5.0,0.6.0,0.7.0,0.8.0 --reps 5 --single-reps 2 (defaults). Environment from the
# workflow: BUN_BIN, BENCH_WORK, PROBE_OUT, PROBE_PROJECTS (vscode). Writes $PROBE_OUT/summary.txt (one line per run),
# runs.tsv, and medians.md (the table).
set -uo pipefail   # no -e: a reference tool that is missing or exits non-zero must not end the probe
: "${BENCH_WORK:?}" "${PROBE_OUT:?}"
versions="0.3.0,0.4.0,0.5.0,0.6.0,0.7.0,0.8.0"
reps=5
single_reps=2
suffix="-ts7.1.0-dev.20260929"
args=(${PROBE_ARGS:-})
for ((i = 0; i < ${#args[@]}; i++)); do
  case "${args[i]}" in
    --versions) versions="${args[i + 1]}"; i=$((i + 1)) ;;
    --reps) reps="${args[i + 1]}"; i=$((i + 1)) ;;
    --single-reps) single_reps="${args[i + 1]}"; i=$((i + 1)) ;;
  esac
done
mkdir -p "$PROBE_OUT/pkgs"
IFS=',' read -r -a vs <<< "$versions"

# Fetch each release's Linux x64 binary from npm.
declare -A bin
for v in "${vs[@]}"; do
  full="$v$suffix"
  dir="$PROBE_OUT/pkgs/$v"
  mkdir -p "$dir"
  (cd "$dir" && npm pack "@maschwenk/tsrs-linux-x64@$full" --silent > /dev/null 2>&1 && tar -xzf ./*.tgz)
  bin[$v]="$dir/package/tsrs"
  chmod +x "${bin[$v]}"
  echo "== tsrs $v: $("${bin[$v]}" --version 2>&1 | head -1)" | tee -a "$PROBE_OUT/summary.txt"
done
tsgo=$(find "$BENCH_WORK/tsgo" -type f \( -name tsgo -o -name 'tsgo*' \) -perm -u+x 2>/dev/null | grep -v '\.js$' | head -1)
if [ -n "$tsgo" ]; then echo "== tsgo: $("$tsgo" --version 2>&1 | head -1) ($tsgo)" | tee -a "$PROBE_OUT/summary.txt"; else echo "== tsgo: not found under $BENCH_WORK/tsgo" | tee -a "$PROBE_OUT/summary.txt"; fi
if [ -n "${BUN_BIN:-}" ]; then echo "== bun: $("$BUN_BIN" --revision 2>&1 | head -1)" | tee -a "$PROBE_OUT/summary.txt"; fi

root="$(cd "$(dirname "$0")/../.." && pwd)"
IFS=',' read -r -a projects <<< "${PROBE_PROJECTS:-vscode}"
for project in "${projects[@]}"; do
proj=$(python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); d=d["projects"] if isinstance(d, dict) else d; print(next(p["project"] for p in d if p["name"] == sys.argv[2]))' "$root/bench/projects.json" "$project")
cd "$BENCH_WORK/solutions/$project" || { echo "no checkout for $project" | tee -a "$PROBE_OUT/summary.txt"; continue; }
flags=(-p "$proj" --noEmit --incremental false --extendedDiagnostics --pretty false)
run() { # name mode rep cmd...
  local name=$1 mode=$2 rep=$3; shift 3
  name="$project/$name"
  local out="$PROBE_OUT/${name//\//_}-$mode-rep$rep.txt"
  /usr/bin/time -f "%e %M" -o "$out.time" "$@" > "$out" 2>&1 || true
  read -r wall kib < <(tail -n 1 "$out.time")
  local check; check=$(grep -E '^Check time' "$out" | tr -s ' ' | awk '{print $3}' | tr -d 's')
  local errors; errors=$(grep -c -E 'error TS[0-9]+' "$out" || true)
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$name" "$mode" "$rep" "$wall" "$kib" "${check:-}" "$errors" >> "$PROBE_OUT/runs.tsv"
  printf '%-10s %-9s rep %s: wall %s s, peak %s MiB, check %s s, errors %s\n' "$name" "$mode" "$rep" "$wall" "$((kib / 1024))" "${check:-?}" "$errors" | tee -a "$PROBE_OUT/summary.txt"
}
# Warm the page cache once per binary.
for v in "${vs[@]}"; do "${bin[$v]}" "${flags[@]}" > /dev/null 2>&1 || true; done
for rep in $(seq 1 "$reps"); do
  for v in "${vs[@]}"; do run "tsrs-$v" default "$rep" "${bin[$v]}" "${flags[@]}"; done
  for v in "${vs[@]}"; do run "tsrs-$v" checkers32 "$rep" "${bin[$v]}" "${flags[@]}" --checkers 32; done
  if [ -n "$tsgo" ]; then run tsgo default "$rep" "$tsgo" "${flags[@]}"; fi
  if [ -n "${BUN_BIN:-}" ]; then run bun default "$rep" "$BUN_BIN" check -p "$proj" --no-pretty --all; fi
done
for rep in $(seq 1 "$single_reps"); do
  for v in "${vs[@]}"; do run "tsrs-$v" single "$rep" "${bin[$v]}" "${flags[@]}" --singleThreaded; done
  if [ -n "$tsgo" ]; then run tsgo single "$rep" "$tsgo" "${flags[@]}" --singleThreaded; fi
done
done
python3 - "$PROBE_OUT/runs.tsv" "$PROBE_OUT/medians.md" <<'EOF'
import collections, statistics, sys
runs = collections.defaultdict(list)
order = []
for line in open(sys.argv[1]):
    name, mode, rep, wall, kib, check, errors = line.rstrip("\n").split("\t")
    key = (name, mode)
    if key not in runs: order.append(key)
    runs[key].append((float(wall), int(kib) / 1024, float(check) if check else None, int(errors)))
lines = ["| binary | mode | wall median s (min-max) | check median s | peak median MiB | errors |", "| --- | --- | ---: | ---: | ---: | ---: |"]
for key in order:
    rows = runs[key]; walls = [r[0] for r in rows]; peaks = [r[1] for r in rows]; checks = [r[2] for r in rows if r[2] is not None]
    lines.append(f"| {key[0]} | {key[1]} | {statistics.median(walls):.2f} ({min(walls):.2f}-{max(walls):.2f}) | "
                 f"{(statistics.median(checks) if checks else float('nan')):.2f} | {statistics.median(peaks):.0f} | {rows[0][3]} |")
open(sys.argv[2], "w").write("\n".join(lines) + "\n")
print("\n".join(lines))
EOF
