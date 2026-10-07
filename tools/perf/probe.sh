#!/usr/bin/env bash
# THP probe (notes/mem-thp.md): wall, Check time, peak RSS, instructions and minor faults of tsrs on vscode at 4 / 16 / 64
# checkers on the 64-thread runner for mimalloc huge-page variants: the shipped build, the no_thp build (mimalloc drops
# MADV_HUGEPAGE on its arenas; the tsrs arena keeps its advice), and mimalloc options set through the environment
# (the same options main.rs could set with mi_option_set before the first allocation). Everything goes to $PROBE_OUT.
#
# Environment (set by the workflow; set them yourself to run locally):
#   TSRS_BIN    tsrs binary          BUN_BIN   bun binary with `bun check` (unused here)
#   BENCH_WORK  bench/.work with the projects set up (bench/run.py --setup-only --projects vscode)
#   PROBE_OUT   output directory
set -euo pipefail
: "${TSRS_BIN:?}" "${BENCH_WORK:?}" "${PROBE_OUT:?}"
mkdir -p "$PROBE_OUT"
# Re-exec from a copy: the variant build below edits the working tree.
if [ "${PROBE_SELF:-}" != 1 ]; then
  repo="$(cd "$(dirname "$0")/../.." && pwd)"
  cp "$0" /tmp/probe-self.sh
  PROBE_SELF=1 PROBE_REPO="$repo" exec bash /tmp/probe-self.sh "$@"
fi
repo="$PROBE_REPO"
proj="$BENCH_WORK/solutions/vscode"
flags=(-p src --noEmit --incremental false --extendedDiagnostics --pretty false)

cat > "$PROBE_OUT/measure.py" <<'PY'
"""measure.py <out.json> -- <command>: wall, instructions (perf counter, all threads), ru_maxrss, minor faults, and the
sampled maximum of AnonHugePages (smaps_rollup, every 10 ms)."""
import json, os, struct, sys, time
sys.path.insert(0, os.environ["COUNT_DIR"])
import count

def huge(pid):
    try:
        with open(f"/proc/{pid}/smaps_rollup") as f:
            for line in f:
                if line.startswith("AnonHugePages:"):
                    return int(line.split()[1])
    except OSError:
        pass
    return 0

out, command = sys.argv[1], sys.argv[3:]
fd, error = count.open_counter()
t0 = time.monotonic()
pid = os.fork()
if pid == 0:
    try:
        os.execvp(command[0], command)
    finally:
        os._exit(127)
max_huge = 0
while True:
    wpid, status, usage = os.wait4(pid, os.WNOHANG)
    if wpid == pid:
        break
    max_huge = max(max_huge, huge(pid))
    time.sleep(0.01)
wall = time.monotonic() - t0
instr = struct.unpack("Q", os.read(fd, 8))[0] if fd is not None else None
json.dump({"wall": wall, "instructions": instr, "max_rss_kib": usage.ru_maxrss, "minflt": usage.ru_minflt,
           "user": usage.ru_utime, "sys": usage.ru_stime, "hugepages_kib": max_huge,
           "exit": os.waitstatus_to_exitcode(status), **({"error": error} if error else {})}, open(out, "w"))
PY

cat > "$PROBE_OUT/table.py" <<'PY'
"""table.py <dir>: medians per (variant, checkers) over the reps."""
import glob, json, os, re, statistics, sys
rows = {}
for j in glob.glob(os.path.join(sys.argv[1], "*-k*-rep*.json")):
    m = re.match(r"(.*)-k(\d+)-rep(\d+)\.json$", os.path.basename(j))
    d = json.load(open(j))
    t = open(j[:-5] + ".txt").read()
    c = re.search(r"^Check time:\s+([\d.]+)s", t, re.M)
    d["check"] = float(c.group(1)) if c else float("nan")
    rows.setdefault((m.group(1), int(m.group(2))), []).append(d)
med = lambda xs: statistics.median(xs) if xs else float("nan")
print(f"{'variant':28s} {'k':>3s} {'n':>2s} {'wall s':>7s} {'check s':>8s} {'peak GiB':>9s} {'range':>13s} {'huge GiB':>8s} {'instr G':>8s} {'minflt K':>9s} {'sys s':>6s}")
for (v, k), ds in sorted(rows.items(), key=lambda x: (x[0][1], x[0][0])):
    rss = [d["max_rss_kib"] / 1048576 for d in ds]
    print(f"{v:28s} {k:3d} {len(ds):2d} {med([d['wall'] for d in ds]):7.3f} {med([d['check'] for d in ds]):8.3f} {med(rss):9.3f} "
          f"{min(rss):6.3f}-{max(rss):6.3f} {med([d['hugepages_kib'] for d in ds])/1048576:8.3f} "
          f"{med([(d['instructions'] or 0) for d in ds])/1e9:8.1f} {med([d['minflt'] for d in ds])/1e3:9.1f} {med([d['sys'] for d in ds]):6.2f}")
PY

measure() { # <tag> <bin> <env...>; runs tsrs with the flags and --checkers from $k
  local tag="$1" bin="$2"; shift 2
  local out="$PROBE_OUT/$tag"
  COUNT_DIR="$repo/bench" env "$@" python3 "$PROBE_OUT/measure.py" "$out.json" -- "/tmp/bins/$bin" "${flags[@]}" --checkers "$k" > "$out.txt" 2>&1 || true
}

mkdir -p /tmp/bins
cd "$repo"
{ git log --oneline -1; git status --short; } > "$PROBE_OUT/tree.txt"
build() { cargo build --release --locked -p tsrs_cli >> "$PROBE_OUT/build.log" 2>&1 && cp target/release/tsrs "/tmp/bins/$1"; }
cp "$TSRS_BIN" /tmp/bins/split   # this branch: src/alloc.rs, the cold heap off unless TSRS_MI_COLD_MIN is set
sed -i 's/^mimalloc = "0.1.52"$/mimalloc = { version = "0.1.52", features = ["no_thp"] }/' crates/tsrs_cli/Cargo.toml
grep -n '^mimalloc' crates/tsrs_cli/Cargo.toml >> "$PROBE_OUT/build.log"
build nothp
git checkout -q crates/tsrs_cli/Cargo.toml
git stash -q
build main
git stash pop -q
ls -la /tmp/bins >> "$PROBE_OUT/build.log"

{ echo "== tsrs: $(/tmp/bins/main --version)"; cat /sys/kernel/mm/transparent_hugepage/enabled /sys/kernel/mm/transparent_hugepage/defrag 2>/dev/null; } | tee "$PROBE_OUT/summary.txt" || true
cd "$proj"
/tmp/bins/main "${flags[@]}" > /dev/null 2>&1 || true   # warm the page cache

# variant name | binary | environment
variants=(
  "main|main|"
  "thp|split|"
  "nothp|nothp|"
  "cold-LH|split|TSRS_MI_COLD_MIN=86699"
  "cold-L|split|TSRS_MI_COLD_MIN=86699 TSRS_MI_COLD_MAX=524288"
  "cold-H|split|TSRS_MI_COLD_MIN=524289"
  "cold-MLH|split|TSRS_MI_COLD_MIN=10241"
)
for rep in 1 2 3; do
  for k in 4 16 64; do
    for v in "${variants[@]}"; do
      IFS='|' read -r name bin envs <<< "$v"
      # shellcheck disable=SC2086
      measure "$name-k$k-rep$rep" "$bin" $envs
    done
  done
done
python3 "$PROBE_OUT/table.py" "$PROBE_OUT" | tee -a "$PROBE_OUT/summary.txt"
# Diagnostics identity between the variants (the statistics block differs).
for k in 4 16 64; do
  for v in "${variants[@]}"; do
    name="${v%%|*}"
    if ! diff <(grep -E 'error TS' "$PROBE_OUT/main-k$k-rep1.txt") <(grep -E 'error TS' "$PROBE_OUT/$name-k$k-rep1.txt") > /dev/null; then
      echo "OUTPUT DIFFERS: main vs $name at $k checkers" | tee -a "$PROBE_OUT/summary.txt"
    fi
  done
done
echo "errors at 4 checkers: $(grep -c 'error TS' "$PROBE_OUT/main-k4-rep1.txt")" | tee -a "$PROBE_OUT/summary.txt"
