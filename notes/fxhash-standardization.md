# FxHash standardization (2026-10-12)

The owner requested fewer custom hash functions where FxHash makes sense, with simplification accepted without a
minimum speedup, but not at the cost of a repeatable slowdown. This is a maintenance change, not a speedup claim.
No cache, thread, field, allocation or externally visible hash format is added.

The retained change is `tsrs_checker/src/flowmemo.rs`: use the existing `rustc_hash::FxBuildHasher` dependency to
hash the flow handle and full 128-bit memo key for the direct-mapped slot. Full-key checks, cache layout, key
identity construction and invalidation remain unchanged.

Slots use a low-bit mask. Fx's state is pointer-sized, so taking the high bits of a 64-bit result would collapse
the table to slot zero on a 32-bit target. The new tests exercise both slot distribution and full-key equality
under forced collisions.

## Which hashes are interchangeable

Bucket hashes can change when a lookup still compares the complete key. Identity encodings and cross-language
fingerprints have different requirements. The audit found these intentional exceptions:

| Location | Purpose and reason to retain it |
| --- | --- |
| `tsrs_checker/src/checker.rs` (`CacheHashKey`), `checker_09.rs` (`keyBuilder`), `flowmemo.rs` (`compute_hashed_flow_memo_key`) | XXH3-128 identities for checker inputs; replacing them with a pointer-sized Fx hash would change collision semantics. |
| `tsrs_project/src/overlayfs.rs`, `extendedconfigcache.rs`; `tsrs_incremental/src/snapshot.rs`; `tsrs_tsoptions/src/contentmappers.rs`; `tsrs_contentmapper/src/hostimpl.rs` | XXH3-128 content/configuration/mapper fingerprints used as identities or invalidation signatures. |
| `checker_09.rs` (`packed_type_list_key`), `checker_11.rs` (`active_mapper_cache_key`), `flowmemo.rs` (`compute_flow_memo_key`), `relater_types.rs` (`RelationKey::pair`), `relater_1.rs` (`recursion_key`) | Exact compact encodings of IDs/handles, not general-purpose hashes. |
| `tsrs_scanner/src/scanner.rs` (`keyword_hash`) | A compile-time collision-checked perfect keyword table. |
| Scanner/parser/binder/printer oracle examples; `tsrs_testrunner/src/types_dump.rs` | FNV-1a output must match the Go oracles. |
| `tsrs_checker/src/flow.rs`, `relater_2.rs`, `checker_05.rs` | Checkpoint/census selection and census fingerprints; changing the mixers changes the sampled work. |
| `tsrs_compiler/src/checkerpool.rs` (`splitmix64`) | Seeded debug permutations; this is a random-number generator. |
| `tsrs_ast/src/symbol.rs` (`index_hash`), `tsrs_checker/src/packedmap.rs` (`PackedKey for CacheHashKey`) | Adapt or reuse an already-computed hash. Rehashing adds work. |

The short-name mixer in `tsrs_ast/src/symbol.rs` is another measured exception. The rejected experiment in
`perf-symbol-map-lookups.md` predates the implementation introduced by #299 and its subsequent ARM64 constant
changes in #327 (`perf-icount-hillclimb.md`, `perf-icount-round4.md`). Those older results do not establish the cost
of replacing today's implementation.

## Measurement method

Baseline: `3f1c229de87be0099086cce33da1c2770cfa8a5d`, Rust 1.99.0, the unchanged release profile (fat LTO, one
codegen unit). Each checker candidate was built independently against the baseline; all used the macOS 26.5 SDK.
The installed SDK default could not be linked by the installed linker, so the SDK selection was explicit for both
baseline and candidates. No build configuration was changed in the repository.

Public projects and revisions came from `bench/projects.json`: vscode, t3code-server, formbricks-web,
supabase-studio and cal-diy. Every run used `--noEmit --incremental false --pretty false`. Single-checker runs also
used `--singleThreaded` and `RAYON_NUM_THREADS=1`; default runs left both unset. macOS ARM64 (Darwin 27.0.0,
14 reported CPUs) `/usr/bin/time -l` provided retired instructions, peak RSS and wall time.

For each project/mode, a shared warm-up preceded three rounds of baseline, the four independently built variants
in rotated/reversed order, and baseline again. The paired delta is the median of each candidate divided by the
mean of its two bracketing baselines, minus one. Adjacent baseline runs and the full baseline range describe the
observed noise; they are not confidence intervals. Every measured run compared stdout bytes (only CRLF
normalized) and exit status against the baseline.

Our builds and checks had finished before measurement. Other work on the shared machine still makes wall time
noisy. macOS instruction counters also include kernel work and are not the deterministic Linux counts from
`bench/count.py`. No Linux runner was available; these results cannot establish Linux performance or support a
sub-percent speedup claim.

## Results and validation

The symbol-name replacement was not retained. A preliminary seven-pair screen on vscode, t3code-server and
supabase-studio showed median instruction increases of 0.95%, 1.00% and 1.43%, with effectively unchanged peak RSS.
These initial baseline/candidate binaries used the same older SDK, and builds/setup were still running; wall time
was unusable and baseline instruction spans were broad. This screen did not establish performance neutrality, so
the existing short-name fast path stays. Names longer than 16 bytes already use FxHash.

The signature-key `PackedMap` replacement was also not retained. Its initial cal-diy single-checker result was
+2.67% paired instructions, and a separate five-repetition confirmation was still +1.74% (+3.46% as a ratio of
medians). Both baseline and candidate had noisy high/low counter levels, so this does not isolate a causal cost of
FxHash. It nevertheless fails to establish a neutral replacement; retain the existing mixer and revisit only with
more stable counter evidence. No speculative replacement or its new tests remain in the code.

Flow memo's initial formbricks single-checker result was +1.40% paired instructions. A separate five-repetition
confirmation gave +0.07% paired / -0.03% ratio of medians, with peak RSS +0.002%. The initial increase did not
repeat. This is consistent with neutral cleanup, not evidence of a speedup.

The union-cache and node-file replacements were initially near neutral individually, but were not retained after
their combination with flow memo failed to establish neutrality on supabase-studio, including a focused
confirmation. This does not isolate either helper as the cause: the counters are noisy. Conservatively narrow the
change to the independently measured flow-memo replacement rather than accept an unresolved combined increase.
The union cache's short/long mixers, table-size shift and node-file mixer therefore remain unchanged.

### Retained flow-memo measurements

These are the isolated flow-memo binary's measurements from the independent screen, not the rejected combination.
All 90 selected measurements (30 flow-memo and 60 baseline) matched stdout/exit. Absolute values are medians;
parenthesized changes are paired deltas. The formbricks signal has the separate five-repetition confirmation below.
The final rebuilt CLI is byte-identical to this measured binary (SHA-256
`42b5cfc605b29915a3dc7c7d3513f3575cde8b80aa768cf168c0606eabf5824b`).

| Project / mode | Instructions G before→after (paired) | Peak MiB before→after (paired) | Baseline instructions G range (span) | Baseline peak MiB range (span) |
| --- | ---: | ---: | --- | --- |
| vscode / single | 84.337→84.235 (-0.13%) | 1608.453→1608.484 (0.00%) | 83.321–84.350 (1.22%) | 1608.422–1608.484 (0.004%) |
| vscode / default | 95.545→95.348 (-0.05%) | 2107.664→2111.125 (+0.16%) | 95.299–96.802 (1.57%) | 2096.109–2123.719 (1.310%) |
| t3code-server / single | 39.544→39.320 (-0.57%) | 729.594→729.609 (0.00%) | 39.350–39.736 (0.98%) | 729.578–729.594 (0.002%) |
| t3code-server / default | 124.018→124.385 (+0.76%) | 2095.438→2093.500 (+0.26%) | 122.460–125.136 (2.16%) | 2069.984–2116.172 (2.204%) |
| formbricks-web / single | 40.158→41.299 (+1.40%) | 1095.773→1095.812 (0.00%) | 40.127–41.297 (2.92%) | 1095.766–1095.781 (0.001%) |
| formbricks-web / default | 71.130→70.949 (-0.19%) | 1962.836→1958.234 (-0.18%) | 69.977–71.636 (2.33%) | 1950.922–1972.688 (1.109%) |
| supabase-studio / single | 45.102→44.176 (-2.11%) | 868.555→868.594 (0.00%) | 44.182–45.496 (2.91%) | 868.547–868.594 (0.005%) |
| supabase-studio / default | 76.135→76.145 (-0.21%) | 1644.391→1646.547 (-0.21%) | 75.406–76.472 (1.40%) | 1637.781–1661.031 (1.414%) |
| cal-diy / single | 31.295→31.302 (+0.01%) | 767.641→767.672 (0.00%) | 31.282–31.839 (1.78%) | 767.625–767.641 (0.002%) |
| cal-diy / default | 74.040→74.469 (+0.57%) | 1839.484→1839.734 (-0.14%) | 71.935–75.101 (4.28%) | 1811.844–1877.500 (3.569%) |

Apart from the separately confirmed formbricks signal, the largest positive paired instruction medians are t3code
at the default (+0.76%, mixed repetition signs, baseline span 2.16%) and cal-diy at the default (+0.57%, baseline
span 4.28%). No repeatable material increase above the observed variation was established. This supports the narrow
maintenance cleanup, but is not proof of exact cost neutrality on this machine or on Linux. The negative supabase
result is likewise not claimed as a speedup. Paired peak-RSS changes range from -0.21% to +0.26%.

### Independent screen

All 180 measured runs preserved stdout and exit status. Each candidate column is paired instruction delta /
paired peak-RSS delta. Baseline values are medians of six runs; the range shows observed instruction variability.
The rejected signature candidate is included so its result is not lost.

| Project | Mode | Base instructions (G) | Base peak RSS (MiB) | Base instruction range (G; span) | Union cache | Flow memo | Utilities | Packed map |
| --- | --- | ---: | ---: | --- | ---: | ---: | ---: | ---: |
| vscode | single | 84.337 | 1608.5 | 83.321–84.350; 1.22% | +0.07% / 0.00% | -0.13% / 0.00% | -0.77% / 0.00% | -0.64% / 0.00% |
| vscode | default | 95.545 | 2107.7 | 95.299–96.802; 1.57% | -0.35% / +0.46% | -0.05% / +0.16% | -0.06% / -0.77% | +0.05% / -1.15% |
| t3code-server | single | 39.544 | 729.6 | 39.350–39.736; 0.98% | -0.42% / -0.01% | -0.57% / 0.00% | -0.48% / 0.00% | -0.49% / 0.00% |
| t3code-server | default | 124.018 | 2095.4 | 122.460–125.136; 2.16% | -0.17% / -0.45% | +0.76% / +0.26% | +0.08% / -0.43% | +0.10% / -0.50% |
| formbricks-web | single | 40.158 | 1095.8 | 40.127–41.297; 2.92% | +0.11% / 0.00% | +1.40% / 0.00% | -0.85% / 0.00% | -1.14% / 0.00% |
| formbricks-web | default | 71.130 | 1962.8 | 69.977–71.636; 2.33% | +0.39% / +0.56% | -0.19% / -0.18% | +0.19% / +0.38% | +0.62% / +0.28% |
| supabase-studio | single | 45.102 | 868.6 | 44.182–45.496; 2.91% | +0.35% / -0.01% | -2.11% / 0.00% | +0.08% / 0.00% | -0.54% / 0.00% |
| supabase-studio | default | 76.135 | 1644.4 | 75.406–76.472; 1.40% | -0.51% / -0.68% | -0.21% / -0.21% | -0.73% / -0.75% | -0.06% / -0.11% |
| cal-diy | single | 31.295 | 767.6 | 31.282–31.839; 1.78% | +0.13% / -0.01% | +0.01% / 0.00% | +0.06% / 0.00% | +2.67% / 0.00% |
| cal-diy | default | 74.040 | 1839.5 | 71.935–75.101; 4.28% | -1.22% / -0.47% | +0.57% / -0.14% | +1.04% / -0.41% | +1.42% / -0.03% |

### Isolated confirmations

Five repetitions each, bracketed by ten baseline runs. All 30 measured output/exit comparisons matched. Absolute
values are medians; delta columns show paired median / ratio of medians.

| Candidate | Project | Base G | Candidate G | Instruction delta | Base MiB | Candidate MiB | Peak-RSS delta | Baseline instruction range |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| flowmemo | formbricks-web | 41.370 | 41.359 | +0.066% / -0.026% | 1095.773 | 1095.797 | +0.002% / +0.002% | 40.262–41.444 G (2.86%) |
| packedmap | cal-diy | 31.361 | 32.446 | +1.741% / +3.459% | 767.625 | 767.641 | +0.002% / +0.002% | 31.323–32.449 G (3.59%) |

### Rejected combined candidate

Union-cache, flow-memo and node-file changes together, three candidate repetitions per project/mode. All 90 measured stdout/exit
comparisons match. Absolute values below are medians; percentages are paired deltas and can differ from a ratio of
those medians. Wall seconds are recorded for completeness and are too noisy for acceptance decisions.

| Project | Mode | Instructions G before→after (paired) | Peak MiB before→after (paired) | Wall s before→after — noisy | Baseline instructions G range (span) | Baseline peak MiB range (span) |
| --- | --- | ---: | ---: | ---: | --- | --- |
| vscode | single | 84.464→84.270 (-0.06%) | 1608.453→1608.438 (0.00%) | 9.455→9.260 | 83.295–84.614 (1.56%) | 1608.453–1608.484 (0.002%) |
| vscode | default | 96.039→95.473 (+0.05%) | 2099.000→2113.047 (+0.67%) | 1.405→1.190 | 95.342–96.741 (1.46%) | 2076.844–2112.625 (1.705%) |
| t3code-server | single | 39.815→39.751 (-0.03%) | 729.578→729.562 (0.00%) | 4.565→4.340 | 39.739–39.963 (0.56%) | 729.578–729.609 (0.004%) |
| t3code-server | default | 124.803→124.979 (+0.14%) | 2094.227→2096.391 (-0.22%) | 2.035→2.060 | 123.319–126.142 (2.26%) | 2080.719–2118.344 (1.797%) |
| formbricks-web | single | 41.338→41.363 (+1.30%) | 1095.781→1095.750 (0.00%) | 3.460→3.450 | 40.249–41.379 (2.73%) | 1095.766–1095.812 (0.004%) |
| formbricks-web | default | 70.382→70.024 (-0.21%) | 1963.977→1962.656 (+0.04%) | 1.620→1.240 | 69.790–70.716 (1.32%) | 1953.359–1970.203 (0.858%) |
| supabase-studio | single | 44.960→45.472 (+1.40%) | 868.562→868.531 (0.00%) | 3.850→8.380 | 44.226–45.491 (2.81%) | 868.547–868.594 (0.005%) |
| supabase-studio | default | 75.891→76.065 (+0.73%) | 1637.000→1642.109 (+0.33%) | 1.410→1.400 | 75.397–77.262 (2.46%) | 1630.547–1643.906 (0.816%) |
| cal-diy | single | 31.310→31.367 (+0.18%) | 767.641→767.625 (0.00%) | 2.660→2.620 | 31.292–31.324 (0.10%) | 767.625–767.641 (0.002%) |
| cal-diy | default | 73.038→73.747 (+1.32%) | 1834.211→1842.188 (+0.56%) | 1.235→1.260 | 72.695–74.443 (2.39%) | 1828.016–1858.109 (1.641%) |

The positive formbricks/supabase single-checker medians and cal-diy outlier prompted one focused confirmation on
all three projects in both modes. Their baseline and candidate counts occupy overlapping high/low bands, which
can put a bracket midpoint between those bands. That explains why paired and unpaired medians differ; it does not
by itself establish neutrality or identify a causal regression.

The focused confirmation used five repetitions in each cell; all 90 measured output/exit comparisons matched:

| Project / mode | Instructions G before→after (paired) | Peak MiB before→after (paired) | Baseline instructions G range (span) | Baseline peak MiB range (span) |
| --- | ---: | ---: | --- | --- |
| formbricks-web / single | 41.332→41.325 (+0.01%) | 1095.773→1095.766 (0.00%) | 40.216–41.470 (3.03%) | 1095.766–1095.797 (0.003%) |
| formbricks-web / default | 70.199→69.939 (-0.62%) | 1964.086→1965.969 (+0.19%) | 69.708–70.718 (1.44%) | 1957.328–1988.078 (1.566%) |
| supabase-studio / single | 44.450→44.621 (+0.40%) | 868.562→868.531 (0.00%) | 44.139–45.533 (3.14%) | 868.547–868.562 (0.002%) |
| supabase-studio / default | 77.134→77.784 (+1.01%) | 1640.969→1648.719 (+0.67%) | 76.019–77.875 (2.41%) | 1625.219–1645.469 (1.234%) |
| cal-diy / single | 32.398→32.465 (+0.11%) | 767.625→767.609 (0.00%) | 31.317–32.662 (4.15%) | 762.891–767.672 (0.623%) |
| cal-diy / default | 73.545→73.235 (-0.71%) | 1845.344→1832.219 (-0.74%) | 72.903–74.707 (2.45%) | 1831.500–1870.688 (2.124%) |

Supabase at the default still had a +1.01% paired instruction signal, following +0.73% in the first combined run.
Although the 2.41% baseline span prevents attributing that change to code, the combination did not clear the
neutrality check. Only the isolated flow-memo replacement is retained; no further combination is inferred to pass.

### Correctness

- Native library tests: 21 AST and 9 checker tests pass. The new flow-memo test forces slot collisions and checks
  both flow identity and the high bits of the memo key. Its distribution test catches 32-bit slot collapse.
- Four focused tests execute under Node WASI on `wasm32-wasip1`, including the new flow tests.
- `cargo check --workspace` and `cargo check -p tsrs_wasm --target wasm32-wasip1` pass without warnings.
- Full conformance in reference and canonical history: all 18 pass/status lists in each mode exactly match the
  baseline. Errors: 13,458 pass, two codes-only and two existing failures. Types/symbols: 12,779 each pass in
  reference mode; 12,778 each in canonical mode, retaining its known `objectLiteralNormalization` difference.
  No crashes or timeouts in either mode.
- All 29 regressions pass normally and with both `TSRS_FLOW_MEMO=shadow` and `TSRS_UNION_CACHE=shadow`. The shadow
  wrapper saves only union-cache counter lines separately; all diagnostic stderr and exit statuses remain checked.
- `tools/lint/source.py` and `tools/lint/ratchet.py` pass with no new findings; lint baselines are unchanged.

The retained code was also independently reviewed for full-key equality, cache layout and 32-bit indexing.
