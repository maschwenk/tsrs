# Smaller compatible auto-import regex matcher

The auto-import module-specifier exclusion preference now uses Unicode-capable PikeVM matching instead of
`regex`'s meta-engine. On macOS arm64 the linked, stripped CLI shrinks **430,696 bytes (420.6 KiB, 2.09%)**,
from **19.626 MiB to 19.215 MiB**. This follows the explicit decision to prioritize binary size on this cold LSP
path while preserving existing regex features. The requested executable-size reduction supplies the adoption
criterion; runtime costs are measured below.

## Implementation and compatibility

`crates/tsrs_modulespecifiers/src/exclude_regex.rs` parses with `regex-syntax` and runs a Thompson NFA through
`regex-automata`'s PikeVM. Production features are `std`, `syntax`, `nfa-pikevm` and all `unicode` features.
The meta-engine, hybrid/DFA search, one-pass engine, bounded backtracking, literal search accelerators and
`perf-inline` are absent. `regex` remains a dev dependency for differential checks. No dependency versions,
release profile or public function signatures change.

Unicode properties, Unicode case folding, Unicode word boundaries, inline flags, captures and existing regex
syntax remain available. Slash-delimited patterns and the `i` flag still use the existing normalization code;
other slash flags still have their existing handling. Invalid patterns remain ignored and cached as `None`.
The process-wide cache, its clearing threshold and its allocation outside freeable request regions are preserved.
Each compiled PikeVM owns a reusable execution cache behind a mutex. The cache is boxed so a literal-only
pattern does not reserve space for a large VM cache.

A bare PikeVM replacement was rejected: it can reject large literal alternations that the old meta-engine accepts.
The adapter therefore mirrors two short-circuits in the locked `regex-automata 0.4.18` implementation:

- Exact finite nonempty literal sequences with no empty needle, captures or assertions use a byte comparison loop.
- Pure literal alternations with at least 3,000 branches use that loop too, preserving the old Aho-Corasick path's
  bypass of the NFA size limit.

Other patterns keep captures during compilation and the old 10 MiB NFA limit. A reverse NFA is also compiled
and discarded to preserve the old engine's rejection at that limit. Removing captures or skipping this validation
would change which patterns are accepted. These rules follow `meta::strategy::Pre` and
`meta::literal::alternation_literals`; rerun the differential checks when upgrading these dependencies.

`regex-lite` was not selected because it would remove supported Unicode behavior. Adding a faster engine or
substring accelerator would restore some of the bytes removed here. Large literal sets now scan linearly and
can be much slower than Aho-Corasick; this is the chosen size tradeoff.

## Linked size

2026-10-10, macOS 27.0.1 arm64, Rust 1.99.0 (`b940084d7`), cargo-bsize 0.0.2, base commit
`2fbb51f412d8b0d7a29dcc6f88e8bcd04ac27d64`, worktree `/private/tmp/tsrs-binary-size-audit` on
`codex/binary-size-audit`. Both builds use the same opt-level 3, fat LTO, one codegen unit and unwinding.
These are local analysis builds without PGO or BOLT; Linux release savings still need measurement.

| Bytes | Before | After | Reduction |
| --- | ---: | ---: | ---: |
| Stripped executable | 20,579,296 | 20,148,600 | 430,696 |
| Machine code | 12,820,592 | 12,462,384 | 358,208 |
| Read-only data | 5,690,979 | 5,664,205 | 26,774 |
| Unwind and exception tables | 1,804,384 | 1,752,460 | 51,924 |

The executable total also includes writable data, loader information, alignment and code signatures, so the
section reductions do not sum to the file-size reduction. The original retained-footprint estimate of 944.8 KiB
was an upper bound, not a saving. The new reference graph attributes 569.8 KiB to `is_excluded_by_regex`.

Build the unchanged base first and preserve its stripped copy, then build the source change with the same command:

```sh
SDKROOT=/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX14.5.sdk \
CFLAGS='-isysroot /Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX14.5.sdk' \
cargo bsize --bin=tsrs --limit=50 --frozen > target/size-audit/regex-after-bsize.md 2> target/size-audit/regex-after-bsize.log
strip -o target/size-audit/tsrs-regex-after-stripped target/bsize/release/tsrs
wc -c target/size-audit/tsrs-stripped target/size-audit/tsrs-regex-after-stripped
cargo tree -p tsrs_cli -e normal,build,features --offline
```

Use the host's matching SDK on other machines. `cargo bsize`'s dependency feature table includes workspace/dev
feature requests; the CLI feature tree and its compiled fingerprint confirm the smaller production feature set.
Actual stripped byte sizes take precedence over the report's shipped-size estimate.

## Instructions, memory and completion latency

The ignored local driver `target/size-audit/regex-runtime.py` generates a 701-file fixture: 400 public modules,
200 internal modules, 100 modules in a `café` directory and a main file. Every module exports a distinct
`AuditValueNNNN` and a generic function using a mapped type. The strict ES2022/ESNext CLI check uses `--noEmit
--incremental false --pretty false`. Five paired runs per mode alternate the before/after order. Both binaries
exit successfully with identical empty diagnostic output. macOS `/usr/bin/time -l` supplies retired instructions
and peak RSS; `bench/count.py`'s hardware counter is Linux-only.

The LSP session initializes, opens the main document, requests completions without exclusions, then configures
19 exclusion patterns: 16 anchored Unicode-aware nonmatches, one invalid pattern, a slash-delimited case-insensitive
internal-path pattern, and `café`. Eight edits and completion requests per session exercise new document snapshots;
three paired sessions per binary alternate order. Every session has 700 unfiltered and 400 filtered `AuditValue`
entries. All 48 filtered response objects agree after sorting completion entries and removing the transient cache
ID. Session counters include initialization, the unfiltered request, all filtered requests and shutdown.

| Median | Before | After | Change |
| --- | ---: | ---: | ---: |
| Single-threaded CLI instructions | 1,362,138,965 | 1,362,053,155 | -0.006% |
| Single-threaded CLI peak RSS | 50.922 MiB | 50.750 MiB | -0.34% |
| Default-mode CLI instructions | 2,047,401,386 | 2,051,160,804 | +0.18% |
| Default-mode CLI peak RSS | 80.219 MiB | 79.594 MiB | -0.78% |
| LSP session instructions | 1,189,574,454 | 1,195,778,314 | +0.52% |
| LSP session peak RSS | 84.188 MiB | 83.266 MiB | -1.10% |
| Filtered completion latency, 24 requests per binary | 5.735 ms | 5.831 ms | +0.096 ms (+1.68%) |

The ordinary CLI does not execute this regex path; these small differences do not establish a checker performance
gain. This synthetic fixture is not one of the pinned `bench/projects.json` projects and does not establish headline
performance. A prior standalone prototype on one million `^lodash/` matches retired 284,145,541 instructions with
the default engine versus 2,335,204,690 with the compatible PikeVM adapter (8.2x); the LSP request work here dilutes
that matcher cost. The prototype counters are separate from the implemented CLI measurements above.

Raw size/section data, five paired CLI runs, three paired LSP sessions and transcripts remain under the ignored
`target/size-audit/`: `regex-size-results.json`, `regex-runtime-results.json`, `regex-after-bsize.md` and
`regex-cli-features.txt`. Run the local driver with `/usr/bin/time -l python3 target/size-audit/regex-runtime.py`.

## Verification

- `cargo test -p tsrs_modulespecifiers --offline`: 17 passed. The fixed corpus covers 51 patterns and 16 inputs;
  the deterministic generated corpus covers 150 patterns and eight inputs. Matching and pattern acceptance are
  compared against the old engine, including Unicode, invalid syntax, captures and empty matches.
- Large-pattern checks cover 2,999, 3,000, 30,000 and 60,000 literal alternatives, captures/assertions around
  large alternations, a 600,000-byte literal, and capture/Unicode repetition limits.
- Concurrent searches reuse one VM cache safely; flag-normalized cache identity, invalid-pattern caching and
  cached matchers surviving request-region disposal are covered.
- All four `TestAutoImportSpecifierExcludeRegexes` fourslash cases pass against the pinned TypeScript baseline.
- `cargo check --workspace --offline` and `cargo check -p tsrs_wasm --target wasm32-wasip1 --offline` pass.
- `tools/lint/ratchet.py` passes with three existing findings and none new; `tools/lint/source.py` passes.

No user-visible capability or cited suite count changes, so the README capability table is unchanged.
