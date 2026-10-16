# Ad hoc tasks

Repository maintenance and generation tasks live here. Run them from the repository root after
`pnpm install --frozen-lockfile`.

## JavaScript Unicode casing

```sh
node tasks/gen-stringutil.mjs          # regenerate the compact Rust table
node tasks/gen-stringutil.mjs --check  # check freshness without writing
node tasks/gen-stringutil.mjs --check --verify-go ts-ref/tsc/internal/stringutil/js_case_generated.go
```

The generator reads the pinned `@unicode/unicode-15.1.0` package directly, including simple and full case mappings,
final sigma, and the Cased / Case_Ignorable properties. It validates sorted mappings, UTF-8 pool offsets, lengths,
and exact round-trips before writing. `--verify-go` also compares every mapping and property range with the pinned
TypeScript Go table; `tools/gen-check.sh` runs that comparison. The Go file is only a verification oracle.

Update the Unicode dependency only when the TypeScript pin changes its Unicode data. Generation uses no casing
or Unicode-property APIs from the host runtime.

The casing probe checks all code points (including lone surrogates) and contextual combinations, or runs one
million casing calls. Build it against each revision to compare its verification digest and resource counters:

```sh
cargo build --release --locked -p tsrs_core
rustc --edition=2024 -C opt-level=3 -C lto=fat tasks/bench-js-case.rs \
  --extern tsrs_core=target/release/libtsrs_core.rlib -L dependency=target/release/deps \
  -o target/bench-js-case
target/bench-js-case verify
/usr/bin/time -l target/bench-js-case unicode-lower # macOS; bench/count.py on Linux
```

Workloads: `ascii-lower`, `ascii-upper`, `ascii-unchanged`, `unicode-lower`, `unicode-upper`.
