# Rust slice APIs for pure comparisons

Use Rust's stable sort for four folding/organize-imports sorts, and `partition_point` for four line-map,
source-map and span-map lookups. Comparators at these sites only read positions or import names. Preserve
stable equal-element order and the first exact match; line and segment gaps still choose the previous entry.

Keep `goslices` for checker comparisons that assign IDs lazily, incomplete symbol comparators and unstable
sorts whose equal-element order affects source mappings, edits or wire IDs. This does not retry the blanket
sort changes rejected in `notes/cpu-checker.md`, `notes/lsp-memfix.md` and `notes/perf-round2-followups.md`.
The scope is requested library cleanup at pure-comparison callers, not a performance change.

Core, language-service, source-map and span-map coverage passed (113/70/32/31 cases, one existing ignored core
case). The added line-map case covers duplicate exact positions, gaps, both boundaries and an empty map.

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
| Compiler | 1.813671 → 1.813607 | -0.002% | 78.578 → 78.453 | -0.179% |
| Compiler-Unions | 3.352547 → 3.352335 | -0.006% | 83.578 → 83.891 | +0.261% |

Reproduce each measurement with `/usr/bin/time -l` on macOS:

```sh
/usr/bin/time -l <binary> -p <suite>/cases/solutions/Compiler --noEmit --incremental false --pretty false
RAYON_NUM_THREADS=1 /usr/bin/time -l <binary> -p <suite>/cases/solutions/Compiler --noEmit --incremental false --pretty false --singleThreaded
# Repeat for Compiler-Unions and interleave baseline/candidate binaries.
```

These are macOS screening measurements, not deterministic Linux `bench/count.py` counts. They do not establish
a qualifying speedup or memory reduction; the motivation is reducing handwritten library code. The full
benchmark suite and native Linux execution were not measured locally.
