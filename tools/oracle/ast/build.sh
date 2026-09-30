#!/bin/sh
# Builds both AST oracle binaries.
set -e
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
mkdir -p "$ROOT/ts-ref/tsc/cmd/tsrs-oracle-ast"
cp "$ROOT/tools/oracle/ast/main.go" "$ROOT/tools/oracle/ast/fields_gen.go" "$ROOT/ts-ref/tsc/cmd/tsrs-oracle-ast/"
(cd "$ROOT/ts-ref/tsc" && GOTOOLCHAIN=auto go build -o "$ROOT/../bin/tsrs-oracle-ast" ./cmd/tsrs-oracle-ast)
(cd "$ROOT" && CARGO_TARGET_DIR=target/parser-integrate cargo build --release -q -p tsrs_parser --example ast_oracle)
