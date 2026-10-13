# Rust Unicode normalization

Replace manual Hangul/canonical decomposition and combining-class ordering with `unicode-normalization =
"=0.1.25"`. Its Unicode 17 tables match the pinned Go reference. This is requested library cleanup, not a
speedup claim.

Keep the public `norm_nfd_string` signature and the existing omission of x/text's stream-safe joiners after 30
non-starters. Go's Mn and uppercase-letter category tables remain: Rust's character properties are different,
and the evaluated general-category crate uses Unicode 16. Keep simple case folding, byte decoding and
JavaScript casing compatibility helpers too.

The previous implementation and replacement matched on all 1,112,064 Unicode scalar values and 10,000
generated combining sequences (seed 20261014). Focused coverage includes Hangul, leading marks, equal
combining classes and 31 non-starters. All 116 core cases passed (one existing ignored case). Running the
updated oracle generator with Go 1.27.0 reproduces the retained category data byte for byte; 3,060 lines of
generated normalization data were removed.

## Validation

Compared with `5f0aada15baa9b930acda3737cf4f7c31e21e84b`, using TypeScript reference
`b85298b6a81f772d080b0455de0ca9d744cd6fd6`. Every conformance and fourslash result list is unchanged,
including failures/skips. The Go-history run passes 13,462 diagnostic, 12,779 type, 12,779 symbol, 13,392
JavaScript, 149 JavaScript-map and 156 sourcemap-text cases. Canonical/default history keeps its existing
object-literal normalization baseline difference. Fourslash remains 4,066 passed, 63 existing failures, 417
skipped. All 36 regression projects pass.

`cargo check --workspace --locked`, the required workspace Clippy command and `tools/lint/source.py` are
clean. `cargo check --locked -p tsrs_wasm --target wasm32-wasip1` passes. No public API, capability status or
conformance count changed.

## Measurements

Apple M3 Max (14 logical cores, 36 GiB), macOS arm64, Rust 1.99.0, release builds without PGO/BOLT. Each
candidate is compared independently with the same baseline. Public benchmark suite at
`41f652ab2df5077b1115f73eaeefc2fe9f674132`, Compiler and Compiler-Unions from `bench/projects.json`: warm-up
then five interleaved pairs per mode, reversing order each pair. Instructions are from single-threaded runs
(`RAYON_NUM_THREADS=1`); RSS is from default checker-count runs. Percentage changes are medians of paired
percentages, not ratios of independent medians. Diagnostic bytes and exit statuses match in every pair.

| Project | Single instructions, G: before → after | Paired change | Default peak RSS, MiB: before → after | Paired change |
| --- | ---: | ---: | ---: | ---: |
| Compiler | 1.813401 → 1.813070 | -0.017% | 78.500 → 78.438 | -0.198% |
| Compiler-Unions | 3.352304 → 3.352433 | -0.011% | 83.859 → 83.594 | -0.056% |

Reproduce each measurement with `/usr/bin/time -l` on macOS:

```sh
/usr/bin/time -l <binary> -p <suite>/cases/solutions/Compiler --noEmit --incremental false --pretty false
RAYON_NUM_THREADS=1 /usr/bin/time -l <binary> -p <suite>/cases/solutions/Compiler --noEmit --incremental false --pretty false --singleThreaded
# Repeat for Compiler-Unions and interleave baseline/candidate binaries.
```

These are macOS screening measurements, not deterministic Linux `bench/count.py` counts. They do not establish
a qualifying speedup or memory reduction; the motivation is reducing handwritten library code. The full
benchmark suite and native Linux execution were not measured locally.
