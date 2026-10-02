#!/bin/sh
# Regenerates the parser oracle data for tsrs_fourslash::test_parser.
#   target/scratch/fsgen/parser_out.jsonl            all constant test contents with Go's ParseTestData result
#   crates/tsrs_fourslash/testdata/parser_oracle_sample.jsonl   committed sample (see sample.py)
# The unit test compares the sample; TSRS_FOURSLASH_PARSER_ORACLE=target/scratch/fsgen/parser_out.jsonl
# cargo test -p tsrs_fourslash test_parser_oracle compares everything.
set -e
cd "$(dirname "$0")/../../.."
root=$PWD
mkdir -p target/scratch/fsgen ts-ref/tsc/cmd/tsrs-oracle-fourslash-parser
cp tools/oracle/fourslash-parser/oracle_test.go ts-ref/tsc/cmd/tsrs-oracle-fourslash-parser/
(cd tools/gen-fourslash && GOTOOLCHAIN=auto go run . -parser-inputs "$root/target/scratch/fsgen/parser_in.jsonl")
(cd ts-ref/tsc && FOURSLASH_PARSER_IN="$root/target/scratch/fsgen/parser_in.jsonl" FOURSLASH_PARSER_OUT="$root/target/scratch/fsgen/parser_out.jsonl" \
    GOTOOLCHAIN=auto go test ./cmd/tsrs-oracle-fourslash-parser -run TestDump -count=1 >/dev/null || true)
python3 tools/oracle/fourslash-parser/sample.py target/scratch/fsgen/parser_out.jsonl crates/tsrs_fourslash/testdata/parser_oracle_sample.jsonl
wc -l target/scratch/fsgen/parser_out.jsonl crates/tsrs_fourslash/testdata/parser_oracle_sample.jsonl
