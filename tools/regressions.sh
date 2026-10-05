#!/usr/bin/env bash
# Runs every testdata/regressions/<case> (a tsconfig.json project whose expected.txt is tsgo-ref's output) with the
# given tsrs binary and reports the cases whose output differs.
#   tools/regressions.sh [path/to/tsrs]   (default: target/release/tsrs)
set -uo pipefail
repo="$(cd "$(dirname "$0")/.." && pwd)"
tsrs="$(cd "$(dirname "${1:-$repo/target/release/tsrs}")" && pwd)/$(basename "${1:-$repo/target/release/tsrs}")"
fail=0
for dir in "$repo"/testdata/regressions/*/; do
  name="$(basename "$dir")"
  actual="$(cd "$dir" && "$tsrs" -p . --pretty false --singleThreaded 2>&1)"
  if [ "$actual" == "$(cat "$dir/expected.txt")" ]; then
    echo "pass $name"
  else
    echo "FAIL $name"; diff <(cat "$dir/expected.txt") <(printf '%s\n' "$actual") | head -20; fail=1
  fi
done
exit $fail
