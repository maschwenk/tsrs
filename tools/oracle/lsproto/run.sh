#!/bin/sh
# Regenerates crates/tsrs_lsproto/tests/testdata/oracle.txt: random values of every protocol type (from the Go
# types, by reflection), mutated variants and edge cases, each with Go's json.Unmarshal + json.Marshal result.
# tests/oracle.rs checks that tsrs_lsproto gives the same result for every line.
set -e -o pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../.." && pwd)
BIN=${TSRS_WORK:-$ROOT/../..}/bin
python3 "$HERE/gen_registry.py"
mkdir -p "$ROOT/ts-ref/tsc/cmd/tsrs-oracle-lsproto"
cp "$HERE/main.go" "$HERE/registry_gen.go" "$ROOT/ts-ref/tsc/cmd/tsrs-oracle-lsproto/"
(cd "$ROOT/ts-ref/tsc" && GOTOOLCHAIN=auto go build -o "$BIN/tsrs-oracle-lsproto" ./cmd/tsrs-oracle-lsproto)
OUT="$ROOT/crates/tsrs_lsproto/tests/testdata"
mkdir -p "$OUT"
TMP=$(mktemp)
{ "$BIN/tsrs-oracle-lsproto" gen 1 2 | python3 "$HERE/mutate.py"; cat "$HERE/edge_cases.txt"; } > "$TMP"
"$BIN/tsrs-oracle-lsproto" check < "$TMP" > "$TMP.out"
paste "$TMP" "$TMP.out" > "$OUT/oracle.txt"
rm -f "$TMP" "$TMP.out"
wc -l "$OUT/oracle.txt"
"$BIN/tsrs-oracle-lsproto" filename < "$HERE/uris.txt" > "$OUT/filename.txt"
