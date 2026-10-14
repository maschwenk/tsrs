# Rust decimal number parsing (2026-10-14)

Requested maintenance cleanup, with no speedup claim. Production Rust loses 121 net physical lines
(including comments/blank lines; excluding coverage, manifests and notes).

Use str::parse::<f64>() directly after the existing JavaScript validation and integer/radix path.
Remove decimal reconstruction and now-unused cutset/zero-trimming helpers. Keep JavaScript whitespace,
exact Infinity spellings, radix behavior, canonical NaN bits and negative zero.

The actual old/new entry points matched bit-for-bit on 499,662 inputs (seed 20261014): exhaustive short
spellings, generated decimals, malformed syntax, signed zero, overflow/underflow and padded exponents.
Core/scanner coverage passes (113/6 cases, one existing ignored core case).

Keep the separate JSON formatter. It differs from jsnum::Number::string on 227 of 999,502 finite random
float values sampled in this audit; sharing those formatters would change current output.

## Validation

Independent branch based on a0bc7f309ec83ee8c03f13d7447a98f22f81873f. Frozen baseline binaries were built
from 5f0aada15baa9b930acda3737cf4f7c31e21e84b; those revisions have identical Rust source and Cargo
manifests/lockfile. TypeScript reference: b85298b6a81f772d080b0455de0ca9d744cd6fd6.

All compiler/conformance and fourslash result lists match, including existing failures/skips. Go-history
passes: 13,462 diagnostic, 12,779 type, 12,779 symbol, 13,392 JavaScript, 149 JavaScript-map and 156
sourcemap-text cases. Canonical history retains its existing object-literal normalization difference.
Fourslash remains 4,066 passed, 63 failed and 417 skipped. All 36 regression projects pass.

Workspace check, required workspace Clippy, tools/lint/source.py and the wasm32-wasip1 check pass.
Capability status and counts are unchanged.

## Measurements

Apple M3 Max (14 logical cores, 36 GiB), macOS arm64, Rust 1.99.0, release builds without PGO/BOLT.
Compiler and Compiler-Unions are public bench/projects.json projects, suite revision
41f652ab2df5077b1115f73eaeefc2fe9f674132. Warm-up then five interleaved baseline/candidate pairs per
mode, reversing order each pair. Single-threaded runs set RAYON_NUM_THREADS=1; RSS uses the default
checker count. Changes are medians of paired percentages. Diagnostic bytes and exit statuses match.

| Project | Single instructions, G: before → after | Paired change | Default peak RSS, MiB: before → after | Paired change |
| --- | ---: | ---: | ---: | ---: |
| Compiler | 1.813729 → 1.813637 | -0.004% | 78.438 → 78.406 | +0.260% |
| Compiler-Unions | 3.352615 → 3.352708 | +0.002% | 83.359 → 83.875 | +0.881% |

Reproduce with /usr/bin/time -l on macOS:

~~~sh
/usr/bin/time -l <binary> -p <suite>/cases/solutions/Compiler --noEmit --incremental false --pretty false
RAYON_NUM_THREADS=1 /usr/bin/time -l <binary> -p <suite>/cases/solutions/Compiler --noEmit --incremental false --pretty false --singleThreaded
# Repeat for Compiler-Unions, interleaving baseline/candidate binaries.
~~~

These screening measurements establish no qualifying performance gain. They are not deterministic Linux
bench/count.py counts; the full project set and native Linux execution were not measured locally.
The outcome is the requested reduction of maintained support code, without a performance claim.
Local audit scripts/checks/differential logs: /private/tmp/tsrs-code-reduction. Frozen binaries,
baseline comparisons and raw resource counters: /private/tmp/tsrs-rust-stdlib.
