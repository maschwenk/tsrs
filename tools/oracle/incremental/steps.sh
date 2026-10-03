#!/usr/bin/env bash
# Runs a scripted sequence of edits on a copy of a small project with tsgo and with TSRS_EMIT=1 tsrs, and compares
# the tsbuildinfo (modulo the build-stamped version) and the emitted files after every step.
# usage: steps.sh <project-dir> <steps-file> [extra tsc flags...]
# steps-file: one shell command per line, run inside the project copy before each build (first line: initial build, use `true`).
set -u
src=$1; steps=$2; shift 2
TSGO=${TSGO:-/root/bin/tsgo}
TSRS=${TSRS:-/root/tsrs/target/release/tsrs}
W=$(mktemp -d /tmp/incr-oracle.XXXX)
cp -r "$src" "$W/go"; cp -r "$src" "$W/rs"
# `-b ...` runs build mode (no -p); otherwise `-p .` is prepended.
if [ "${1:-}" = "-b" ]; then PRE=(); else PRE=(-p .); fi
n=0; fail=0
while IFS= read -r step; do
  n=$((n+1))
  (cd "$W/go" && eval "$step"); (cd "$W/rs" && eval "$step")
  (cd "$W/go" && "$TSGO" "${PRE[@]}" "$@" > "$W/go.out$n" 2>&1; echo "exit $?" >> "$W/go.out$n")
  (cd "$W/rs" && TSRS_EMIT=1 "$TSRS" "${PRE[@]}" "$@" > "$W/rs.out$n" 2>&1; echo "exit $?" >> "$W/rs.out$n")
  sed -i -E 's/[0-9]{2}:[0-9]{2}:[0-9]{2} [AP]M/HH:MM:SS AM/g' "$W/go.out$n" "$W/rs.out$n"
  if ! cmp -s "$W/go.out$n" "$W/rs.out$n"; then echo "step $n ($step): output differs"; diff "$W/go.out$n" "$W/rs.out$n" | head -10; fail=1; fi
  d=$(diff -r -q -x node_modules "$W/go" "$W/rs" 2>&1 | grep -v tsbuildinfo)
  if [ -n "$d" ]; then echo "step $n ($step): trees differ"; echo "$d" | head; fail=1; fi
  for b in $(cd "$W/go" && find . -name '*.tsbuildinfo'); do
    if ! sed 's/"version":"[^"]*"/"version":"V"/' "$W/go/$b" | cmp -s - <(sed 's/"version":"[^"]*"/"version":"V"/' "$W/rs/$b"); then
      echo "step $n ($step): $b differs"; fail=1
      python3 - "$W/go/$b" "$W/rs/$b" <<'PY'
import json,sys
a=json.load(open(sys.argv[1])); b=json.load(open(sys.argv[2]))
for k in sorted(set(a)|set(b)):
    if k!='version' and a.get(k)!=b.get(k): print("  ",k,"go:",json.dumps(a.get(k))[:400],"\n   rs:",json.dumps(b.get(k))[:400])
PY
    fi
  done
done < "$steps"
[ $fail = 0 ] && echo "all $n steps identical ($W)" || echo "differences ($W)"
