# Rust parsing and whitespace helpers

Remove the three duplicated Go whitespace predicates in folding, inlay hints and fourslash; use Rust's
trimming and `char::is_whitespace`. Use `str::parse::<u32>()` for semver components, keeping the ASCII-digit
precheck and existing syntax/range error text. Use `parse::<i64>().is_ok()` for supplemental-file indexes,
preserving optional signs, decimal-only input and range rejection. This is requested library cleanup, not a
speedup claim.

The old whitespace predicate and Rust's predicate agree on all 1,112,064 Unicode scalar values. Added semver
coverage checks u32 boundaries, leading zeros, signs, separators, Unicode digits and invalid syntax after an
overflowing prefix. Core/execute/language-service coverage passed (114/2/69 cases, one existing ignored core
case).

Go boolean spellings, integer parsing with base detection or clamping, quoting, JSON errors/escapes/number
formatting and TypeScript's portable path algorithms remain. Their contracts differ from direct Rust library
substitutions.

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
| Compiler | 1.813360 → 1.813542 | +0.013% | 78.281 → 78.594 | +0.379% |
| Compiler-Unions | 3.352475 → 3.352263 | -0.009% | 84.094 → 83.688 | -0.537% |

Reproduce each measurement with `/usr/bin/time -l` on macOS:

```sh
/usr/bin/time -l <binary> -p <suite>/cases/solutions/Compiler --noEmit --incremental false --pretty false
RAYON_NUM_THREADS=1 /usr/bin/time -l <binary> -p <suite>/cases/solutions/Compiler --noEmit --incremental false --pretty false --singleThreaded
# Repeat for Compiler-Unions and interleave baseline/candidate binaries.
```

These are macOS screening measurements, not deterministic Linux `bench/count.py` counts. They do not establish
a qualifying speedup or memory reduction; the motivation is reducing handwritten library code. The full
benchmark suite and native Linux execution were not measured locally.
