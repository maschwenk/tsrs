#!/usr/bin/env bash
# Usage: ./run.sh <tsrs> <slow> [runs] [checkers]: regenerates the project and prints one digit per run, the number of
# TS2590s that run reported.
set -euo pipefail
tsrs=$1 slow=$2 runs=${3:-30} checkers=${4:-2}
dir=$(mktemp -d)
node "$(dirname "$0")/make.mjs" "$dir" "$slow"
for _ in $(seq "$runs"); do
	("$tsrs" -p "$dir" --checkers "$checkers" 2>&1 || true) | grep -c "error TS2590" | tr -d '\n' || true
done
echo
rm -rf "$dir"
