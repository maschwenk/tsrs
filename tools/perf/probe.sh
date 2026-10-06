#!/usr/bin/env bash
# Memory probe (notes/mem-64.md): peak RSS of tsrs on vscode at 4 / 16 / 64 checkers on the 64-thread runner, with the
# arena / heap split and the huge-page share sampled from /proc, instructions from a perf counter (bench/count.py's), and
# the allocator variants that tell allocator retention from live data. Everything goes to $PROBE_OUT.
#
# Environment (set by the workflow; set them yourself to run locally):
#   TSRS_BIN    tsrs binary          BUN_BIN   bun binary with `bun check` (unused here)
#   BENCH_WORK  bench/.work with the projects set up (bench/run.py --setup-only --projects vscode)
#   PROBE_OUT   output directory
set -euo pipefail
: "${TSRS_BIN:?}" "${BENCH_WORK:?}" "${PROBE_OUT:?}"
mkdir -p "$PROBE_OUT"
# Re-exec from a copy: the variant builds below stash and restore the working tree, which would rewrite this file
# under a bash that reads it incrementally.
if [ "${PROBE_SELF:-}" != 1 ]; then
  repo="$(cd "$(dirname "$0")/../.." && pwd)"
  cp "$0" /tmp/probe-self.sh
  PROBE_SELF=1 PROBE_REPO="$repo" exec bash /tmp/probe-self.sh "$@"
fi
repo="$PROBE_REPO"
proj="$BENCH_WORK/solutions/vscode"
flags=(-p src --noEmit --incremental false --extendedDiagnostics --pretty false)

cat > "$PROBE_OUT/measure.py" <<'PY'
"""measure.py <out.json> -- <command>: instructions (perf counter, all threads), max RSS, and sampled /proc maxima:
Rss and AnonHugePages (smaps_rollup, every 10 ms), and the resident bytes inside the compressed-pointer arena
reservation (0x4001_0000_0000 .. +32 GiB) against the rest (smaps, every 50 ms)."""
import json, os, struct, sys, time
sys.path.insert(0, os.environ["COUNT_DIR"])
import count

ARENA_LO, ARENA_HI = 0x4001_0000_0000, 0x4001_0000_0000 + (32 << 30)

def rollup(pid):
    rss = huge = 0
    try:
        with open(f"/proc/{pid}/smaps_rollup") as f:
            for line in f:
                if line.startswith("Rss:"):
                    rss = int(line.split()[1])
                elif line.startswith("AnonHugePages:"):
                    huge = int(line.split()[1])
    except OSError:
        return None
    return rss, huge

def split(pid):
    arena = other = 0
    try:
        with open(f"/proc/{pid}/smaps") as f:
            in_arena = False
            for line in f:
                if line[0] in "0123456789abcdef" and "-" in line.split()[0]:
                    lo = int(line.split("-")[0], 16)
                    in_arena = ARENA_LO <= lo < ARENA_HI
                elif line.startswith("Rss:"):
                    if in_arena:
                        arena += int(line.split()[1])
                    else:
                        other += int(line.split()[1])
    except OSError:
        return None
    return arena, other

out, command = sys.argv[1], sys.argv[3:]
fd, error = count.open_counter()
pid = os.fork()
if pid == 0:
    try:
        os.execvp(command[0], command)
    finally:
        os._exit(127)
max_rss = max_huge = max_arena = max_other = 0
at_max = (0, 0)
last_split = 0.0
while True:
    wpid, status, usage = os.wait4(pid, os.WNOHANG)
    if wpid == pid:
        break
    r = rollup(pid)
    if r:
        if r[0] > max_rss:
            max_rss = r[0]
        if r[1] > max_huge:
            max_huge = r[1]
        now = time.monotonic()
        if now - last_split > 0.05:
            last_split = now
            s = split(pid)
            if s:
                if s[0] + s[1] > at_max[0] + at_max[1]:
                    at_max = s
                max_arena = max(max_arena, s[0])
                max_other = max(max_other, s[1])
    time.sleep(0.01)
code = os.waitstatus_to_exitcode(status)
instr = struct.unpack("Q", os.read(fd, 8))[0] if fd is not None else None
json.dump({"instructions": instr, "max_rss_kib": usage.ru_maxrss, "sampled_rss_kib": max_rss, "sampled_hugepages_kib": max_huge,
           "max_arena_kib": max_arena, "max_other_kib": max_other, "arena_kib_at_peak": at_max[0], "other_kib_at_peak": at_max[1],
           "exit": code, **({"error": error} if error else {})}, open(out, "w"))
sys.exit(0)
PY

measure() { # <tag> <env...> -- <cmd...>
  local tag="$1"; shift
  local envs=()
  while [ "$1" != "--" ]; do envs+=("$1"); shift; done; shift
  local out="$PROBE_OUT/$tag"
  COUNT_DIR="$repo/bench" env "${envs[@]}" python3 "$PROBE_OUT/measure.py" "$out.json" -- "$@" > "$out.txt" 2>&1 || true
  python3 - "$tag" "$out.json" "$out.txt" <<'PY' | tee -a "$PROBE_OUT/summary.txt"
import json, re, sys
tag, j, t = sys.argv[1], json.load(open(sys.argv[2])), open(sys.argv[3]).read()
times = " ".join(re.findall(r"^(?:Check|Total) time:\s+[\d.]+s", t, re.M))
err = len(re.findall(r"error TS", t))
g = lambda k: j.get(k) or 0
print(f"{tag:40s} maxrss {g('max_rss_kib')/1048576:.3f} GiB  sampled {g('sampled_rss_kib')/1048576:.3f}  huge {g('sampled_hugepages_kib')/1048576:.3f}  "
      f"arena@peak {g('arena_kib_at_peak')/1048576:.3f}  other@peak {g('other_kib_at_peak')/1048576:.3f}  instr {g('instructions')/1e9:.1f} G  {times}  errors {err}")
PY
}

# Variant binaries: the workflow built origin/main + this branch's patch ("new"); "base" is origin/main, and the
# "-nothp" builds give the mimalloc heap 4 KiB pages (its `no_thp` feature drops the MADV_HUGEPAGE advice on its
# arenas) while the tsrs arena keeps its transparent huge pages.
mkdir -p /tmp/bins
cd "$repo"
{ git log --oneline -1; git status --short; } > "$PROBE_OUT/tree.txt"
build() { cargo build --release -p tsrs_cli >> "$PROBE_OUT/build.log" 2>&1 && cp target/release/tsrs "/tmp/bins/$1"; }
nothp() { sed -i 's/^mimalloc = "0.1.52"$/mimalloc = { version = "0.1.52", features = ["no_thp"] }/' crates/tsrs_cli/Cargo.toml; grep -n '^mimalloc' crates/tsrs_cli/Cargo.toml >> "$PROBE_OUT/build.log"; }
build new
nothp; build new-nothp; git checkout -q crates/tsrs_cli/Cargo.toml
git stash -q
build base
nothp; build base-nothp; git checkout -q crates/tsrs_cli/Cargo.toml
git stash pop -q
ls -la /tmp/bins >> "$PROBE_OUT/build.log"

echo "== tsrs: $("$TSRS_BIN" --version)" | tee "$PROBE_OUT/summary.txt"
cat /sys/kernel/mm/transparent_hugepage/enabled /sys/kernel/mm/transparent_hugepage/defrag 2>/dev/null | tee -a "$PROBE_OUT/summary.txt" || true
cd "$proj"
"$TSRS_BIN" "${flags[@]}" > /dev/null 2>&1 || true   # warm the page cache

for rep in 1 2 3; do
  for k in 4 16 64; do
    for bin in base new; do
      measure "$bin-k$k-rep$rep" -- "/tmp/bins/$bin" "${flags[@]}" --checkers "$k"
    done
    if [ "$rep" != 3 ]; then
      for bin in base-nothp new-nothp; do
        measure "$bin-k$k-rep$rep" -- "/tmp/bins/$bin" "${flags[@]}" --checkers "$k"
      done
    fi
  done
done
for bin in base new base-nothp new-nothp; do
  measure "$bin-k1-rep1" -- "/tmp/bins/$bin" "${flags[@]}" --checkers 1
  measure "$bin-nocheck-rep1" -- "/tmp/bins/$bin" "${flags[@]}" --noCheck
done
# mimalloc knobs that could cut the huge-page amplification of its heap without giving up the advice.
for k in 4 64; do
  measure "new-eagercommit0-k$k-rep1" MIMALLOC_ARENA_EAGER_COMMIT=0 -- /tmp/bins/new "${flags[@]}" --checkers "$k"
  measure "new-purge100-k$k-rep1" MIMALLOC_PURGE_DELAY=100 -- /tmp/bins/new "${flags[@]}" --checkers "$k"
  measure "new-allthpoff-k$k-rep1" MIMALLOC_ALLOW_THP=0 -- /tmp/bins/new "${flags[@]}" --checkers "$k"
done
# Diagnostics identity between the variants (the statistics block differs).
for k in 4 16 64; do
  for bin in new base-nothp new-nothp; do
    if ! diff <(grep -E 'error TS' "$PROBE_OUT/base-k$k-rep1.txt") <(grep -E 'error TS' "$PROBE_OUT/$bin-k$k-rep1.txt") > /dev/null; then
      echo "OUTPUT DIFFERS: base vs $bin at $k checkers" | tee -a "$PROBE_OUT/summary.txt"
    fi
  done
done
# The containers each checker owns, at 64 checkers (notes/mem-checker-heap.md).
TSRS_HEAP_CENSUS=1 TSRS_HEAP_CENSUS_MIN=0 /tmp/bins/new "${flags[@]}" --checkers 64 > /dev/null 2> "$PROBE_OUT/heap-census-k64.txt" || true
