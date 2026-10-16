# Generate Unicode casing tables

```sh
cargo run --locked -p tsrs_gen_stringutil
cargo run --locked -p tsrs_gen_stringutil -- --check
cargo run --locked -p tsrs_gen_stringutil -- --check --verify-go ts-ref/tsc/internal/stringutil/js_case_generated.go
```

The generator reads Unicode 15.1.0's `UnicodeData.txt`, `SpecialCasing.txt` and `DerivedCoreProperties.txt`
directly. On the first run it downloads them with `curl` from `https://www.unicode.org/Public/15.1.0/ucd/`
into `target/unicode/15.1.0/`. Every run verifies their pinned SHA-256 checksums. For an offline run, populate
that directory first, or use `--ucd-dir <directory>` with those three files.

It emits simple/full case mappings, final sigma, and the Cased / Case_Ignorable property ranges. Mappings are
sorted; UTF-8 pool offsets, lengths and exact round-trips are validated before writing. `--check` checks freshness
without writing. `--verify-go` independently compares every mapping and range against the pinned TypeScript Go
table; `tools/gen-check.sh` runs that comparison. The Go file is only a verification oracle.

Update the Unicode version and checksums only when the TypeScript pin changes its Unicode data. Generation uses
no casing or Unicode-property APIs from the host runtime.
