#!/usr/bin/env bash
# Dumps the tsc/tsbuild scenarios of Go's execute/tsctests as JSON for the Rust harness
# (crates/tsrs_cli/src/tsctests, `cargo test --release -p tsrs_cli tsctests`).
#   tools/oracle/tsctests/dump.sh [OUT]   (default OUT: target/tsctests-dump)
# Applies runner.patch + tsrs_dump.go to ts-ref/tsc/internal/execute/tsctests, runs the Go tests with TSRS_DUMP=OUT,
# then restores runner.go. Needs Go (GOTOOLCHAIN=auto).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$here/../../.."
out="$(realpath -m "${1:-$repo/target/tsctests-dump}")"
dir="$repo/ts-ref/tsc/internal/execute/tsctests"
cp "$dir/runner.go" "$dir/runner.go.orig"
trap 'mv "$dir/runner.go.orig" "$dir/runner.go"; rm -f "$dir/tsrs_dump.go"' EXIT
(cd "$repo/ts-ref/tsc" && patch -s -p1 < "$here/runner.patch")
cp "$here/tsrs_dump.go" "$dir/tsrs_dump.go"
rm -rf "$out"; mkdir -p "$out"
(cd "$repo/ts-ref/tsc" && TSRS_DUMP="$out" GOTOOLCHAIN=auto go test ./internal/execute/tsctests/ -count=1)
find "$out" -name '*.json' | wc -l
