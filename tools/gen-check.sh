#!/usr/bin/env bash
# Re-runs the code generators and fails if any checked-in output differs: a hand edit to a generated file would be
# lost at the next regeneration. Needs node >= 22.18 or 23.6 (runs the .ts generators directly, with type stripping
# on by default; CI uses 24), python3, Go (the fourslash generator type-checks the Go tests with `go list -export` in
# ts-ref/tsc; GOTOOLCHAIN=auto fetches the version go.mod asks for), and ts-ref at the commit in Cargo.toml
# ([workspace.metadata.typescript]). CI runs it as the `generated-code` job.
#
#   tools/gen-check.sh           regenerate, then fail on a diff
#   tools/gen-check.sh --write   regenerate only (leaves the result in the working tree)
#
# Not covered (hand-run tooling): the oracle-only files under tools/oracle and crates/*/tests,
# crates/tsrs_api/src/checker/coverage_table.rs (written from proto.go), and the `BEGIN GENERATED` regions that
# tools/gen-tsoptions writes in crates/tsrs_tsoptions/src.
set -euo pipefail
cd "$(dirname "$0")/.."

generators=(
  "node tools/gen-ast/gen-ast.ts"
  "node tools/gen-ast/gen-flags.ts"
  "node crates/tsrs_api_codec/gen/gen-codec.ts"
  "python3 tools/gen/stringutil_tables.py"
  "python3 tools/gen-diagnostics/gen.py"
  "python3 tools/gen-libs/gen.py"
  "node tools/gen-lsproto/generate.mts"
)

for g in "${generators[@]}"; do
  echo "+ $g"
  # shellcheck disable=SC2086 # each entry is a command line
  $g > /dev/null
done
echo "+ tools/gen-fourslash: go run ."
(cd tools/gen-fourslash && GOTOOLCHAIN=auto go run . > /dev/null)

[ "${1:-}" = "--write" ] && exit 0

if ! git diff --quiet -- crates; then
  echo "::error::generated files differ from their generators (edit the generator, then run tools/gen-check.sh --write):"
  git diff --stat -- crates
  exit 1
fi
untracked=$(git ls-files --others --exclude-standard -- crates)
if [ -n "$untracked" ]; then
  echo "::error::generators wrote files that are not checked in:"
  echo "$untracked"
  exit 1
fi
echo "generated code is up to date ($(( ${#generators[@]} + 1 )) generators)"
