# Rust string helper cleanup (2026-10-14)

Requested maintenance cleanup, with no speedup claim. Production Rust loses 53 net physical lines
(including comments/blank lines; excluding coverage, manifests and notes).

Use standard splitting for split_lines: terminate on LF, remove a trailing CR from each chunk, then
split remaining lone CRs. Preserve empty input, CR/LF/CRLF, interior empty lines, UTF-8 slices and final
terminators. Replace four equivalent whitespace helpers in printing, snippets and command-line parsing
with standard trim/trim_end operations. Use parse::<i64>().is_ok() for supplemental build-info indexes.

The actual old/new line splitters matched on 349,525 cases with CR, LF, ASCII and multibyte text. Whitespace
predicates match on all 1,112,064 Unicode scalars. ECMAScript whitespace and Go regexp whitespace remain
distinct. Affected core, printer, compiler, options, execute and runner units pass.

PR 317's unconditional iterator conversion was previously rejected (perf-iterator-regressions.md). This
round is explicitly requested code reduction limited to proven equivalent support helpers, with no
general conversion of measured hot loops.

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
| Compiler | 1.813805 → 1.813747 | -0.002% | 78.391 → 78.750 | +0.458% |
| Compiler-Unions | 3.352687 → 3.352884 | +0.008% | 83.766 → 83.609 | +0.056% |

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
