#!/usr/bin/env bash
# Follow-up to memprobe.sh: name the hot libc addresses, callers (DWARF unwinding) of libc / mimalloc / rehash hot
# spots at 32 checkers, and peak smaps.
set -uo pipefail
PERF=${PERF:-$(ls /usr/lib/linux-tools/*/perf 2>/dev/null | head -1)}
: "${TSRS_BIN:?}" "${BENCH_WORK:?}" "${PROBE_OUT:?}"
mkdir -p "$PROBE_OUT"
root=$(pwd)
libc=$(ldd "$TSRS_BIN" | awk '/libc.so/{print $3}')
echo "libc $libc" > "$PROBE_OUT/libc-syms.txt"
nm -D --defined-only "$libc" | sort > "$PROBE_OUT/libc-nm.txt" 2>&1 || true
python3 - "$PROBE_OUT/libc-nm.txt" >> "$PROBE_OUT/libc-syms.txt" <<'PY'
import sys
syms=[]
for l in open(sys.argv[1]):
    p=l.split()
    if len(p)==3:
        try: syms.append((int(p[0],16),p[2]))
        except ValueError: pass
syms.sort()
for a in (0x1986a5,0x198684,0x19869e,0x1986ab,0x1a16f7):
    best=max((s for s in syms if s[0]<=a), default=None)
    print(hex(a), best and (best[1], hex(a-best[0])))
PY
for name in ${PROBE_PROJECTS//,/ }; do
  read -r cwd proj < <(python3 - "$name" <<'PY'
import json, sys
sys.path.insert(0, "bench")
import run as bench
from pathlib import Path
cfg = json.loads(Path("bench/projects.json").read_text())
p = next(p for p in cfg["projects"] if p["name"] == sys.argv[1])
cwd, proj = bench.project_path(cfg, p, Path("bench/.work"))
print(Path(cwd).resolve(), Path(proj).resolve())
PY
)
  flags=(-p "$proj" --noEmit --incremental false --pretty false)
  (cd "$cwd" && "$TSRS_BIN" "${flags[@]}" > /dev/null 2>&1) || true
  for k in ${MEMPROBE_CHECKERS:-32}; do
    tag="$name-k$k"
    (cd "$cwd"; "$TSRS_BIN" "${flags[@]}" --checkers "$k" > /dev/null 2>&1 & pid=$!
     best=""; bestrss=0
     while kill -0 $pid 2>/dev/null; do
       s=$(cat /proc/$pid/smaps_rollup 2>/dev/null) || break
       rss=$(printf '%s\n' "$s" | awk '/^Rss:/{print $2}')
       if [ -n "$rss" ] && [ "$rss" -gt "$bestrss" ]; then bestrss=$rss; best=$s; fi
       sleep 0.01
     done
     printf '%s\n' "$best" > "$PROBE_OUT/$tag-smaps-peak.txt")
    (cd "$cwd" && "$PERF" record -F 499 -e cycles:u --call-graph dwarf,4096 -o "$root/dw-$tag.data" -- \
      "$TSRS_BIN" "${flags[@]}" --checkers "$k" > /dev/null 2>&1) || true
    "$PERF" script --no-inline -i "$root/dw-$tag.data" -F ip,sym,dso 2>/dev/null | python3 tools/perf/callers.py \
      > "$PROBE_OUT/$tag-callers.txt" 2>&1 || true
  done
done
echo done > "$PROBE_OUT/done.txt"
