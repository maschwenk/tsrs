# Rust arbitrary-precision integers

Replace the handwritten math/big limb arithmetic with `num-bigint` 0.5.1 and `num-traits`, behind the existing
private `Int` interface. This is requested library cleanup, not a speedup claim. The lockfile adds only the
required dependencies; existing dependency versions stay unchanged.

Keep Go's base-0 prefixes, sign and underscore validation, non-positive exponent behavior,
round-to-nearest-even intermediate precision, and float overflow. `round_to_prec(256)` remains before float
conversion: skipping it would change double-rounding cases. The Go-compatible floating-point math in
`jsnum/math.rs` remains because replacing it with native transcendental functions would change bits.

The baseline and adapter matched on 141,649 cases (seed 20261014): 120,000 generated parsing strings, 10,000
long hexadecimal integers, and 11,649 powers. Valid values were compared as decimal text, raw float bits, and
rounding at 1, 3, 53, 64 and 256 significant bits. Core/scanner coverage passed (117/6 cases, one existing
ignored core case).

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
| Compiler | 1.813385 → 1.813450 | +0.007% | 78.453 → 78.344 | -0.139% |
| Compiler-Unions | 3.352353 → 3.352295 | -0.001% | 83.516 → 83.312 | -0.188% |

Reproduce each measurement with `/usr/bin/time -l` on macOS:

```sh
/usr/bin/time -l <binary> -p <suite>/cases/solutions/Compiler --noEmit --incremental false --pretty false
RAYON_NUM_THREADS=1 /usr/bin/time -l <binary> -p <suite>/cases/solutions/Compiler --noEmit --incremental false --pretty false --singleThreaded
# Repeat for Compiler-Unions and interleave baseline/candidate binaries.
```

These are macOS screening measurements, not deterministic Linux `bench/count.py` counts. They do not establish
a qualifying speedup or memory reduction; the motivation is reducing handwritten library code. The full
benchmark suite and native Linux execution were not measured locally.

An additional all-target core Clippy run encounters the same 251 diagnostic locations and 231 warnings in the
existing baseline coverage/examples; none is in `jsnum/big.rs`. The required workspace Clippy scope passes.
