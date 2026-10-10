# Smaller regex feature set for auto-import exclusions

The auto-import module-specifier exclusion preference keeps its existing `regex::Regex` implementation and
disables optional performance features. The owner chose the simpler crate-feature change after comparing it
with a custom PikeVM adapter. Binary size takes priority on this cold LSP path; matching features and pattern
acceptance must remain compatible. The requested executable-size saving supplies the adoption criterion;
runtime costs are measured below.

## Implementation and compatibility

The production change is one dependency declaration in `crates/tsrs_modulespecifiers/Cargo.toml`:

```toml
regex = { version = "1", default-features = false, features = ["std", "unicode", "perf-dfa", "perf-literal"] }
```

This removes one-pass search, bounded backtracking and aggressive inlining. All Unicode features, literal
optimizations and the lazy DFA remain enabled. No dependency versions, release profile or matcher source change.
Slash-delimited flags, invalid-pattern caching, the cache clearing threshold and allocation outside freeable
request regions continue to use the existing code. The custom matcher and its engine-specific checks are removed.

Keeping `perf-literal` preserves acceptance of 30,000- and 60,000-branch literal alternations. Keeping `perf-dfa`
preserves the reverse-NFA compilation limit: without it, `\w{224}`, `\p{L}{256}` and `[\pL\pN]{224}`, among
others, become accepted even though the default engine rejects them. Keeping `unicode` preserves Unicode
properties, case folding and word boundaries. `regex-lite` was rejected because it changes that behavior.

The feature meanings are documented by [regex 1.13.1](https://docs.rs/regex/1.13.1/regex/#performance-features).
The compilation-limit differences were checked against the locked `regex-automata 0.4.18` sources, especially
`meta::strategy::Core::new` and `meta::literal::alternation_literals`. Recheck acceptance when upgrading regex.

## Feature comparison

2026-10-10, macOS 27.0.1 arm64, Rust 1.99.0 (`b940084d7`). Five isolated consumers use locked `regex 1.13.1`,
`regex-automata 0.4.18` and `regex-syntax 0.8.11`, with opt-level 3, fat LTO, one codegen unit and symbol stripping.
Separate consumers avoid Cargo feature unification accidentally restoring default features.

The driver covers 51 fixed patterns, 150 deterministic generated patterns, large literal alternations,
capture/assertion limits and 64 Unicode repetition-limit cases: 278 pattern groups. All common accepted patterns
match identically; the differences below concern whether patterns compile.

| Configuration | Probe bytes | Saving from default | Pattern-acceptance differences |
| --- | ---: | ---: | ---: |
| Default `regex` | 1,427,872 | — | — |
| `std`, `unicode` | 1,063,456 | 355.9 KiB | 22 |
| `std`, `unicode`, `perf-literal` | 1,245,904 | 177.7 KiB | 20 |
| Selected: `std`, `unicode`, `perf-literal`, `perf-dfa` | 1,345,264 | 80.7 KiB | 0 |
| Previous custom PikeVM adapter | 964,112 | 452.9 KiB | 0 |

These are standalone probe savings. The adapter achieved a larger reduction but needed custom literal matching,
a mutex-protected VM cache and reverse-NFA validation to mirror the default engine's compilation acceptance.
The owner selected the feature-only configuration for simpler maintenance. A bare PikeVM and the smaller
feature-only sets were rejected for compatibility.

Three executions of the adversarial compilation-and-matching corpus per compatible variant give these macOS
`time -l` medians. This corpus reaches roughly 500 MiB and is not representative of CLI or LSP memory:

| Corpus counters | Default | Selected features | Previous adapter |
| --- | ---: | ---: | ---: |
| Retired instructions | 24,286,209,947 | 24,248,488,610 | 23,521,074,475 |
| Peak RSS, bytes | 519,864,320 | 520,667,136 | 512,458,752 |

Sources, per-variant executables, acceptance output and counters are under the ignored
`target/size-audit/regex-feature-comparison/`; `results.json` records every acceptance difference.

## Linked CLI size

The stripped executable shrinks **66,144 bytes (64.6 KiB, 0.32%)**, from **19.626 MiB to 19.563 MiB**.

| Bytes | Before | Selected features | Reduction |
| --- | ---: | ---: | ---: |
| Stripped executable | 20,579,312 | 20,513,168 | 66,144 |
| Machine code | 12,820,592 | 12,754,672 | 65,920 |
| Read-only data | 5,690,979 | 5,688,987 | 1,992 |
| Unwind and exception tables | 1,804,384 | 1,800,028 | 4,356 |

The executable also includes writable data, loader information, alignment and code signatures, so the section
reductions do not sum to the file-size reduction. The original 944.8 KiB retained-footprint estimate is an upper
bound on removing the entire old matcher, not the saving from this feature adjustment.

The linked measurements on 2026-10-11 use cargo-bsize 0.0.2 and the original audit base
`2fbb51f412d8b0d7a29dcc6f88e8bcd04ac27d64`. Both binaries are rebuilt from an isolated copy of that commit at
`/private/tmp/tsrs-regex-features-size`; only the dependency features change between builds. The PR is based on
`9b51574c847927ce76e813d5194cdc2d8b40b1d3`, so these are measurements at the audit base, not current-main release
measurements. Both builds use opt-level 3, fat LTO, one codegen unit and unwinding, without PGO or BOLT.
Linux release savings still need measurement.

Build the default dependency first, preserve a stripped copy, then repeat after changing the feature declaration:

```sh
SDKROOT=/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX14.5.sdk \
CFLAGS='-isysroot /Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX14.5.sdk' \
cargo bsize --bin=tsrs --limit=50 --frozen
strip -o tsrs-stripped target/bsize/release/tsrs
wc -c tsrs-stripped
cargo tree -p tsrs_cli -e normal,build,features --offline
```

Use the host's matching SDK on other machines. `cargo bsize`'s dependency feature table includes workspace/dev
feature requests; the CLI feature tree and compiled fingerprint confirm the reduced production feature set.
Actual stripped byte sizes take precedence over the report's shipped-size estimate.

The previous adapter experiment at the same audit base saved 420.6 KiB (2.09%) in the linked CLI and added
0.096 ms to the synthetic filtered-completion median. Those figures describe the removed adapter, not the
selected feature-only change. Its original data remains in `regex-size-results.json` and `regex-runtime-results.json`.

## Instructions, memory and completion latency

The ignored local driver `target/size-audit/regex-simple-runtime.py` generates a 701-file fixture: 400 public
modules, 200 internal modules, 100 modules in a `café` directory and a main file. Every module exports a distinct
`AuditValueNNNN` and a generic function using a mapped type. Five paired CLI runs per mode alternate before/after
order, checking strict ES2022/ESNext with `--noEmit --incremental false --pretty false`. Both binaries exit with
identical empty diagnostic output. macOS `/usr/bin/time -l` supplies retired instructions and peak RSS;
`bench/count.py`'s hardware counter is Linux-only.

Each LSP session requests completions without exclusions, then configures 19 patterns: 16 anchored Unicode-aware
nonmatches, one invalid pattern, a slash-delimited case-insensitive internal-path pattern and `café`. Eight edits
and requests per session exercise new document snapshots; three paired sessions alternate binary order. Every
session has 700 unfiltered and 400 filtered `AuditValue` entries. All 48 filtered response objects agree after
sorting entries and removing the transient cache ID. Counters include initialization, all requests and shutdown.

| Median | Before | Selected features | Change |
| --- | ---: | ---: | ---: |
| Single-threaded CLI instructions | 1,362,660,277 | 1,362,300,995 | -0.026% |
| Single-threaded CLI peak RSS | 51.078 MiB | 50.953 MiB | -0.245% |
| Default-mode CLI instructions | 2,048,167,052 | 2,048,922,950 | +0.037% |
| Default-mode CLI peak RSS | 80.156 MiB | 80.188 MiB | +0.039% |
| LSP session instructions | 1,195,162,586 | 1,192,008,246 | -0.264% |
| LSP session peak RSS | 87.781 MiB | 84.672 MiB | -3.542% |
| Filtered completion latency, 24 requests per binary | 5.726 ms | 5.617 ms | -0.109 ms (-1.91%) |

The ordinary CLI does not execute this regex path. Small changes in CLI counters do not establish a checker
performance gain. This synthetic fixture is not a pinned `bench/projects.json` project and does not establish
headline performance. Size, raw paired counters and transcripts remain in the ignored `target/size-audit/`:
`regex-simple-size-results.json`, `regex-simple-runtime-results.json` and `regex-simple-{before,after}-bsize.md`.
Run the local driver with `/usr/bin/time -l python3 target/size-audit/regex-simple-runtime.py`.

## Verification

- `cargo test -p tsrs_modulespecifiers --offline`: 12 passed with the reduced features, including Unicode flags,
  normalized cache identity, invalid-pattern caching and a matcher surviving request-region disposal.
- All four `TestAutoImportSpecifierExcludeRegexes` fourslash cases pass against the pinned TypeScript baseline.
  The fourslash harness itself requests default regex features; the isolated comparison, module cases and linked
  LSP exercise the reduced configuration without that unification.
- `cargo check --workspace --offline` and `cargo check -p tsrs_wasm --target wasm32-wasip1 --offline` pass.
- `tools/lint/ratchet.py` passes with no new findings; `tools/lint/source.py` passes.

No user-visible capability or cited suite count changes, so the README capability table is unchanged.
