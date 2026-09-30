#!/bin/sh
# Builds both printer oracle binaries.
set -e
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
mkdir -p "$ROOT/ts-ref/tsc/cmd/tsrs-oracle-printer"
cp "$ROOT/tools/oracle/printer/main.go" "$ROOT/ts-ref/tsc/cmd/tsrs-oracle-printer/"
(cd "$ROOT/ts-ref/tsc" && GOTOOLCHAIN=auto go build -o "$ROOT/../../bin/tsrs-oracle-printer" ./cmd/tsrs-oracle-printer)
(cd "$ROOT" && CARGO_TARGET_DIR=$ROOT/target cargo build --release -q -p tsrs_printer --example printer_oracle)
