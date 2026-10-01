#!/bin/bash
# Compare the project-wide .types/.symbols walk (tools/oracle/project-types vs `tsrs-test types-dump`) for a list of
# tsconfig projects, one at a time.
#
#   tools/project-types-packages.sh <root> <list of project dirs relative to root> <tsrs-test binary> <out dir>
#
# Prints one line per project and walk: identical files / total, whether the checker counters after the walk agree,
# and the diagnostic count. Differing files are listed in <out>/<project>/diff.<kind>.
ROOT=$1; LIST=$2; RS=$3; OUT=$4
TOOLS=$(cd "$(dirname "$0")" && pwd)
ORACLE=${ORACLE:-$TSRS_WORK/bin/tsrs-oracle-project-types}
mkdir -p "$OUT"
while read -r p; do
  n=$(basename "$p"); o=$OUT/$n
  cd "$ROOT/$p" || continue
  "$RS" types-dump -p tsconfig.json --mode both --text none --out "$o/rs" > "$o.rs.log" 2>&1 || echo "$n tsrs failed ($?)"
  "$ORACLE" --mode both --text none --out "$o/go" tsconfig.json > "$o.go.log" 2>&1 || echo "$n oracle failed ($?)"
  for k in types symbols; do
    [ -f "$o/go/manifest.$k" ] && [ -f "$o/rs/manifest.$k" ] || continue
    r=$(python3 "$TOOLS/project-types-compare.py" "$o/go" "$o/rs" --kind $k --list "$o/diff.$k" | head -1)
    c=$(cmp -s <(tail -1 "$o/go/manifest.$k") <(tail -1 "$o/rs/manifest.$k") && echo counts-eq || echo COUNTS-DIFF)
    echo "$n $k $r $c $(grep -h diagnostics "$o.go.log" | head -1)"
  done
done < "$LIST"
