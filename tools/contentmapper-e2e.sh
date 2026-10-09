#!/usr/bin/env bash
# Runs every testdata/contentmapper/<case> (a tsconfig.json project with a `contentMappers` entry whose expected.txt
# is tsgo's output) with the given tsrs binary and the real process spawner, and reports the cases whose output
# differs. Each case is copied to a temporary directory with testdata/contentmapper/header-mapper installed as
# node_modules/header-mapper, so the cases share one mapper. Needs `node` on PATH.
#   tools/contentmapper-e2e.sh [path/to/tsrs]   (default: target/release/tsrs)
set -uo pipefail
repo="$(cd "$(dirname "$0")/.." && pwd)"
tsrs="$(cd "$(dirname "${1:-$repo/target/release/tsrs}")" && pwd)/$(basename "${1:-$repo/target/release/tsrs}")"
command -v node > /dev/null || { echo "contentmapper-e2e: node is not on PATH" >&2; exit 1; }
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
fail=0
count=0
for dir in "$repo"/testdata/contentmapper/*/; do
  [ -f "$dir/expected.txt" ] || continue
  name="$(basename "$dir")"
  count=$((count + 1))
  mkdir -p "$work/$name/node_modules"
  cp -R "$dir"/. "$work/$name/"
  cp -R "$repo/testdata/contentmapper/header-mapper" "$work/$name/node_modules/header-mapper"
  actual="$(cd "$work/$name" && "$tsrs" -p . --runExternalCode --pretty false --singleThreaded 2>&1)"
  if [ "$actual" == "$(cat "$dir/expected.txt")" ]; then
    echo "pass $name"
  else
    echo "FAIL $name"; diff <(cat "$dir/expected.txt") <(printf '%s\n' "$actual") | head -20; fail=1
  fi
done
if [ "$count" -eq 0 ]; then echo "contentmapper-e2e: no cases found" >&2; exit 1; fi
exit $fail
