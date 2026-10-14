# Rust Base64 encoding (2026-10-14)

Requested maintenance cleanup, with no speedup claim. Production Rust loses 11 net physical lines
(including comments/blank lines; excluding coverage, manifests and notes).

Replace the manual alphabet/chunk/padding loop with base64 0.23.1's padded STANDARD.encode.
Retain the wrapper and RFC 4648 vectors. Enable only the std feature, disabling optional SIMD.
The lockfile adds only base64, without runtime transitive dependencies or unrelated upgrades.

The actual wrapper matched the old encoder on 88,868 inputs (seed 20261014): all one/two-byte inputs,
length remainders, zero/255/sequential buffers and random binary data. RFC 4648 unit vectors and all
27 Rust API transport cases pass, including Go frame/JSON fixtures and reentrancy.

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

The pinned Node clients pass 14 of 15 cases on this macOS volume, identically before and after. The
remaining assertion at tests/node/roundtrip.test.mjs:130 requires a case-sensitive filesystem, while the
OS filesystem reports false. This existing platform limitation remains. Runs use TMPDIR=/private/tmp
to avoid macOS temporary-path aliases, with local Unix sockets enabled.

## Measurements

Apple M3 Max (14 logical cores, 36 GiB), macOS arm64, Rust 1.99.0, release builds without PGO/BOLT.
Compiler and Compiler-Unions are public bench/projects.json projects, suite revision
41f652ab2df5077b1115f73eaeefc2fe9f674132. Warm-up then five interleaved baseline/candidate pairs per
mode, reversing order each pair. Single-threaded runs set RAYON_NUM_THREADS=1; RSS uses the default
checker count. Changes are medians of paired percentages. Diagnostic bytes and exit statuses match.

| Project | Single instructions, G: before → after | Paired change | Default peak RSS, MiB: before → after | Paired change |
| --- | ---: | ---: | ---: | ---: |
| Compiler | 1.813673 → 1.813582 | -0.005% | 78.422 → 78.594 | +0.557% |
| Compiler-Unions | 3.352723 → 3.352803 | +0.005% | 83.656 → 83.500 | +0.130% |

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
