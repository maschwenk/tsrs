# Compact JavaScript Unicode casing (issue #543)

Requested change: generate the compact table directly from the pinned Unicode source data, with ad hoc tasks
under `tasks/`, with one Rust crate per task. `tasks/gen_stringutil` replaces the Go-to-Rust transliterator.
It reads checksum-pinned Unicode 15.1.0 UCD files directly, including simple/full mappings, final sigma, Cased
and Case_Ignorable. No npm Unicode package is needed.
`tools/gen-check.sh` independently compares every mapping and range with the pinned Go table.

The 2,927 records shrink from 64 to 16 bytes each: 187,328 → 46,832 bytes. Their 8,781 string fields use 2,970
unique strings in an 8,517-byte UTF-8 pool. Keys remain sorted and use the existing binary search. Offsets are
`u16`, byte lengths `u8`, and the conditional-casing flag remains explicit. Generation verifies bounds and exact
round-trips. Runtime access uses checked byte slices and the existing append helper; there is no runtime cache,
decompression or extra allocation. ASCII fast paths are unchanged.

## Measurements

Baseline: `94cdbb78c9764634414dd846c2a16d7b7d4edbbd`. macOS arm64, Rust 1.99.0, ordinary release builds
(fat LTO, one codegen unit, stripped symbols; no PGO/BOLT). The candidate contains only the casing change.
Unrelated kind-table edits appeared in the shared checkout after the candidate's AST library was built; they
are absent from both measured binaries.

| Binary size | Before | After | Saving |
| --- | ---: | ---: | ---: |
| Stripped executable | 18,320,064 bytes | 18,176,192 bytes | 143,872 bytes (140.5 KiB) |
| gzip level 9, fixed timestamp | 8,454,295 bytes | 8,452,136 bytes | 2,159 bytes (2.1 KiB) |

Warm-up followed by seven interleaved pairs, reversing order each pair. Values are medians; percentages are the
median of paired changes. Instructions and peak RSS come from `/usr/bin/time -l`. Its wall times have 0.01 s
resolution. Other builds were active during parts of this session, so wall times are screening measurements.
Compiler workloads use the public benchmark-suite revision `41f652ab2df5077b1115f73eaeefc2fe9f674132` from
`bench/projects.json`. Single-threaded runs use `--singleThreaded` and `RAYON_NUM_THREADS=1`; default runs specify
no checker count. Every compiler run has identical diagnostic bytes and exit status before/after.

| Workload | Wall seconds, before → after | Instructions G, before → after (paired change) | Peak RSS MiB, before → after (paired change) |
| --- | ---: | ---: | ---: |
| ascii-lower | 0.03 → 0.03 | 1.058260 → 1.057469 (-0.081%) | 2.000 → 1.828 (-8.594%) |
| ascii-upper | 0.03 → 0.04 | 1.058288 → 1.056490 (-0.081%) | 2.000 → 1.812 (-9.302%) |
| ascii-unchanged | 0.04 → 0.04 | 0.855408 → 0.854447 (-0.112%) | 2.000 → 1.828 (-8.594%) |
| unicode-lower | 1.09 → 1.12 | 15.152205 → 15.351532 (+1.293%) | 2.047 → 1.922 (-6.107%) |
| unicode-upper | 0.60 → 0.62 | 7.187179 → 7.365545 (+2.482%) | 2.047 → 1.938 (-5.344%) |
| Compiler-single | 0.16 → 0.16 | 1.780039 → 1.779035 (-0.046%) | 57.125 → 56.906 (-0.383%) |
| Compiler-default | 0.07 → 0.07 | 2.095988 → 2.096857 (-0.020%) | 78.250 → 78.078 (-0.140%) |
| Compiler-Unions-single | 0.24 → 0.24 | 3.268571 → 3.267526 (-0.042%) | 59.797 → 59.578 (-0.366%) |
| Compiler-Unions-default | 0.11 → 0.11 | 3.950206 → 3.951641 (-0.002%) | 83.188 → 83.156 (-0.019%) |

The compiler workloads show no material change. Non-ASCII casing still costs +1.3–2.5% instructions; the measured
uppercasing wall time also rises from 0.60 to 0.62 s. The compact representation therefore does **not** establish
the issue's strict no-performance-regression acceptance criterion. The owner accepted this size tradeoff on
2026-10-16 and requested a pull request. There is no qualifying whole-compiler performance improvement.

The first implementation sliced `str` and added roughly 4–8% instructions to non-ASCII casing. Checked byte
slicing removes repeated UTF-8 boundary checks; generation proves that each span contains a complete UTF-8
string. Alternative lower-bound and early-return binary searches were measured and rejected; the existing
binary search is retained.

## Validation and reproduction

- All 2,927 mappings, all string fields, and both property range tables match the pinned Go source exactly.
- Old/new casers agree over all 1,114,112 code points, including surrogates, and 4,913 contextual combinations.
  The casing probe's digest is `8a50f0eab6fa3087` on both.
- Core units: 129 passed, one ignored. Conformance: 13,458 diagnostic passes, 12,779 type passes and 12,779 symbol
  passes, with no crashes/timeouts. All 36 regression projects pass.
- All eight generators were rerun without unexpected changes. Final casing freshness/parity, the required
  workspace Clippy command, source lint and `git diff --check` pass.

`tasks/README.md` links to the generator and `tasks/bench_js_case` probe. The measurements above compile that
probe with the old/new casing modules and the same common core helpers, using `rustc -C opt-level=3 -C lto=fat`.
Moving generation into Rust leaves the generated table byte-identical apart from its provenance comment.
For compiler measurements, build each revision with `cargo build --release --locked -p tsrs_cli`, save the
binaries, then interleave:

```sh
/usr/bin/time -l <binary> -p <suite>/cases/solutions/Compiler --noEmit --incremental false --pretty false
RAYON_NUM_THREADS=1 /usr/bin/time -l <binary> -p <suite>/cases/solutions/Compiler \
  --noEmit --incremental false --pretty false --singleThreaded
# Repeat for Compiler-Unions.
```

Local frozen binaries, probes, raw resource counters and the measurement script are in `/private/tmp/tsrs-543`.
These are macOS screening results; Linux `bench/count.py` and the full benchmark project set were not run.
