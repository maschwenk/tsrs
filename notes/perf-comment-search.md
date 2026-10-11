# Block-comment terminator search (2026-10-11)

Retain the stateless block-comment search: scan through the first actual line break, then find `*/` in one
operation and recover the final line start with a short reverse search. Formbricks uses 3.116% fewer deterministic
Linux single-threaded instructions, clearing the 1% gate; default peak RSS is effectively unchanged. No checker
code changes. The per-scanner cached Finder alternative is rejected for lack of a clear additional gain.

## Prior art and reason to revisit

`notes/perf-parse.md` rejected line-oriented `memchr3` comment scanning: it saved instructions but cost 5% more
cycles because short lines favored the predictable scalar loop. It also rejected SWAR whitespace scanning.
This experiment changes the unit of search: one terminator search over the remaining block comment, after the
first line break. One-line comments and the existing first-line scan are unchanged.

The new evidence is the Formbricks block-comment hotspot in `notes/perf-nonchecker-xctrace.md` and the census below.
The local Oxc source was inspected at `4b756621b65ab54c19dc66fbdcb3cd902d662c20`:

- `crates/oxc_parser/src/lexer/comment.rs:84` finds the first line break or closing delimiter with byte-table scans.
  `lexer/search.rs:331` batches 32 scalar table lookups to encourage unrolling; it is not handwritten SIMD.
- After the first line break, `comment.rs:162` needs only to find `*/`. Its lexer lazily retains a fixed-pattern
  `memmem::Finder` (`lexer/mod.rs:113`).

Tsrs also needs the final line start for `@ts-ignore` / `@ts-expect-error` recognition and ranges. After finding the
first delimiter with `memchr::memmem::find`, the new helper searches backward within the skipped body for CR, LF,
or a valid UTF-8 LS/PS sequence. False `0xE2` candidates are skipped. CRLF leaves the position after LF. If no later
line break exists, the position after the first break remains the final line start. Searches are bounded by the
scanner end and first closing delimiter. Existing JSDoc-tag handling, directives, errors and trivia flags retain
their order. The helper enters after a line break in this comment, independently of flags inherited from earlier
trivia. No new dependency, unsafe code or retained scanner state is needed.

## Workload census

Temporary instrumentation ran the single-threaded frontend with `--listFilesOnly`; all three file lists matched
the baseline byte-for-byte. Counts include parser rescans and are scanner invocations, not unique comments.
Instrumentation rescans bodies and logs to stderr, so its timings are not performance evidence. Body bytes exclude
`/*` and `*/`; the final-line suffix excludes its preceding line-break bytes and is not a count of vector loads.

| project | block-comment scans | multiline scans | body bytes | after first break | final-line suffix bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| vscode | 98,620 | 77,678 | 18,353,872 | 16,142,807 | 1,133,918 |
| formbricks-web | 422,404 | 410,629 | 145,900,417 | 144,355,811 | 3,168,107 |
| t3code-server | 45,611 | 38,995 | 14,648,197 | 14,068,724 | 187,734 |

For Formbricks, 98.94% of comment-body bytes lie after the first line break; final-line suffixes total 2.19% of
those bytes. The remaining body has median/p90/p99 lengths 100/581/4,013 bytes, while the final-line suffix is
9/9/13 bytes. No unterminated block comments occurred in these project census runs.

## macOS screening

Base is `63ac5a23a4b6253e85378e6b4bb56e2fcec91731`. All binaries use the same source path, Rust 1.99.0 release profile,
fat LTO, one codegen unit, no PGO, symbols and line-table debug information; builds use Command Line Tools for the
linker. The cached candidate was temporarily rustfmt-formatted during its CLI build; unrelated formatting was
removed, and the saved build source and minimal candidate patch preserve both forms. Only formatting differed
outside the candidate scanner changes.

Public projects are the same prepared workloads as `notes/perf-nonchecker-traversal.md`: Formbricks
`ec45c28e4d20bd9daf3191c991b67981aabf5db8`, t3code `9bd1d8009a6b7c50f9dd9458e2bf27d481ff3b43`, and the existing
archive-backed VS Code checkout (expected benchmark pin `9cf0128b9822dcd8a533f08ef7312956a4390301`; this run did
not independently establish its archive SHA). File counts are 10,427 / 10,455 / 2,945 for VS Code / Formbricks / t3code.

Each variant has one discarded warmup pair and five retained alternating base/candidate pairs per project/mode.
No builds or other benchmark jobs ran concurrently. Single-threaded full checks set `RAYON_NUM_THREADS=1` and
`--singleThreaded`. Default checks omit a checker override (14-core host; the compiler also caps the default by checked-file count). Frontend-only checks add
`--listFilesOnly` and omit `--extendedDiagnostics`, avoiding checker initialization. Inherited `TSRS_`, `TS_TEST_`,
`RAYON_` and `MIMALLOC_` overrides are cleared. `/usr/bin/time -l` records instructions, cycles, wall time and peak
RSS with kernel accounting available outside the sandbox. Diagnostics, exits and frontend file lists agree in
all runs; full checks retain 371 / 0 / 6 diagnostics and exit 2 / 0 / 2.

macOS instruction counts include kernel work and have visible noise clusters. These numbers select a candidate;
they do not establish the deterministic Linux instruction gate. Wall times are not two README benchmark publishes.
Before/after columns are independent medians; deltas are medians of the five paired ratios and therefore need
not equal the ratio of those columns.

Stateless full single-threaded checks:

| project | before instructions (G) | after instructions (G) | paired instructions | paired cycles | paired wall |
| --- | ---: | ---: | ---: | ---: | ---: |
| vscode | 87.141 | 88.246 | -0.17% | +0.48% | +3.63% |
| formbricks-web | 44.710 | 43.891 | -2.67% | -4.42% | -5.11% |
| t3code-server | 41.982 | 41.761 | -0.70% | -1.01% | -0.99% |

Formbricks' single-threaded instruction pairs all improve (range -2.87% to -1.83%).

Stateless frontend-only checks:

| project | paired instructions | paired cycles | paired wall |
| --- | ---: | ---: | ---: |
| vscode | -0.28% | -1.84% | -2.38% |
| formbricks-web | -8.06% | -10.72% | -10.64% |
| t3code-server | -1.66% | -5.05% | -5.17% |

Stateless peak RSS at the default checker count (MiB):

| project | before | after | paired RSS | paired wall |
| --- | ---: | ---: | ---: | ---: |
| vscode | 2092.30 | 2095.91 | +0.47% | +3.70% |
| formbricks-web | 1975.69 | 1963.73 | -0.30% | -4.72% |
| t3code-server | 2126.78 | 2136.12 | +0.38% | +2.65% |

No memory improvement is claimed.

### Cached Finder comparison

A second prototype retained `Option<memchr::memmem::Finder<'static>>` on `Scanner`, initialized lazily for the fixed
`b"*/"` delimiter. It retained no source text, cursor or prior result; mark/rewind therefore required no new saved
state, and reset dropped the finder. Its inline scanner storage is larger even while the option is empty.

Against the original baseline:

| project | full single-thread instructions | full cycles | frontend instructions | frontend cycles |
| --- | ---: | ---: | ---: | ---: |
| vscode | +0.02% | +0.20% | -0.47% | -0.69% |
| formbricks-web | -3.02% | -1.94% | -6.75% | -8.73% |
| t3code-server | -0.65% | -0.59% | -1.62% | -2.11% |

The separate baseline comparisons were noisy enough to justify a direct Formbricks comparison. One warmup and
five alternating retained pairs compared stateless (base) to cached (new), with no intervening builds:

| mode | cached instructions relative to stateless | cycles | wall | RSS |
| --- | ---: | ---: | ---: | ---: |
| single | -0.39% | -1.93% | -2.13% | +0.00% |
| frontend | +0.72% | -0.44% | +0.00% | -0.01% |

The full-check instruction pair range is -2.26% to +0.21%; frontend is -1.13% to +5.19%. There is no clear extra
instruction gain to justify retaining the cache. Select the stateless implementation; do not infer an Oxc-style
per-scanner cache is beneficial for tsrs without stronger evidence.

## Correctness

Both stateless and cached candidates were compared with the original binaries and the pinned Go reference,
TypeScript `b85298b6a81f772d080b0455de0ca9d744cd6fd6`. The isolated Go source manifest matched the pinned archive;
`ts-ref`'s hybrid Git metadata was not used as source provenance.

- All 18,456 scanner hashes agree with both baseline Rust and Go, including token values, flags, spans, ordered
  errors and directives in the oracle's four modes.
- All 18,461 AST hashes agree with baseline Rust. Both candidates retain the same single pre-existing Go gap,
  `regexInvalidUtf8WithUnicodeFlag`; there are no new differences.
- No missing outputs, panics, process failures, malformed results, stderr or retries occurred.
- The corpora include 500 new valid-UTF-8 TS/TSX cases covering CR/LF/CRLF/LS/PS, misleading Unicode prefixes,
  final-line directives, JSDoc, overlapping delimiters, unterminated EOF, search boundaries, JSX and lookahead.
- Two focused scanner cases cover directive snapshots and flags across mark/rewind, and final-line directives
  with Unicode and trailing code. Both variants pass all eight scanner unit cases.

Full conformance ran on source-matched baseline and candidate testrunners with the verified pinned testdata, six
workers and separate result directories. Every error/type/symbol pass list was byte-identical before and after;
no passes were lost or added. All modes had zero crashes and timeouts:

| mode | error-baseline passes | type-baseline passes | symbol-baseline passes |
| --- | ---: | ---: | ---: |
| Go history | 13,458 | 12,779 | 12,779 |
| Go history, `TSRS_LAZY_MEMBERS=0` | 13,458 | 12,779 | 12,779 |
| Canonical, multi-checker | 13,458 | 12,778 | 12,778 |

`cargo test --locked -p tsrs_core -p tsrs_scanner -p tsrs_cli` passed 141 unit/integration cases (one pre-existing
ignored case); all 29 regression fixtures passed. Workspace and `wasm32-wasip1` checks passed with warnings denied.
`tools/lint/ratchet.py` passed with three baseline findings and none new; `tools/lint/source.py` passed. No lint
baseline or generated source changed. The final validation CLI has byte-identical executable and runtime-data
sections to the measured stateless CLI; only temporary debug object-path strings and corresponding signature
bytes differ. The pre-existing notes merge conflict was preserved.

## Linux gate and decision

[Depot run `xh2xbk2tvh`](https://depot.dev/orgs/qw3zz3l8cs/workflows/xh2xbk2tvh) passed on a 16-vCPU AMD EPYC 9R45
runner, Linux x86_64, glibc 2.39, 63 GiB reported RAM. Both source trees and all benchmark scripts/project pins were
fixed to `63ac5a23a4b6253e85378e6b4bb56e2fcec91731`; candidate compiler patch SHA-256 is
`d34225386e5c9a445c13d3295be89a38bcaf1898ca0ea9a23100608970550bbf`. Both use Rust 1.99.0 and
`cargo build --release --locked -p tsrs_cli`, without PGO. The source patch contains only scanner code and the two
focused cases. No unrelated notes, merge conflict or workspace files are included.

The temporary workflow first required working hardware counters, then screened Formbricks before setting up the
other projects. Its first run (`3rb1shqvj7`) stopped before compilation because `setup-node` mishandled an absolute
version-file input; an explicit pinned version fixed that workflow. No failed-setup measurements are used.

The final `bench/count.py` single-threaded instruction counts exclude kernel work:

| project | before instructions | after instructions | change | single-thread RSS before / after (MiB) |
| --- | ---: | ---: | ---: | ---: |
| formbricks-web | 41,973,443,237 | 40,665,609,417 | -3.116% | 1105.12 / 1105.32 |
| vscode | 88,957,144,758 | 88,813,057,871 | -0.162% | 1605.42 / 1605.71 |
| t3code-server | 43,334,035,911 | 43,192,740,714 | -0.326% | 739.29 / 739.41 |

The earlier Formbricks screen independently recorded 41,973,444,451 → 40,665,614,381 instructions (-3.115851%),
reproducing the final -3.115860% result. All counters are positive with no counter errors.

The 1/4/16-checker matrix used three interleaved pairs per cell plus one instruction pair and a 16-checker
poisoned-arena run per project. All 15 comparisons matched diagnostics and exits. Separate default-mode runs
omitted both `--checkers` and `--singleThreaded` to honor the compiler's checked-file cap; one warmup pair and three
retained alternating pairs per project all completed and matched outputs (24 runs including warmups):

| project | default RSS before (MiB) | after (MiB) | ratio-of-medians change | paired-median change |
| --- | ---: | ---: | ---: | ---: |
| formbricks-web | 2037.79 | 2036.13 | -0.081% | -0.081% |
| vscode | 2139.57 | 2137.82 | -0.081% | -0.081% |
| t3code-server | 2197.02 | 2215.05 | +0.821% | +0.869% |

T3code's paired RSS range is -1.17% to +2.24%; no memory improvement or material regression is established.
Linux wall time is mostly flat: Formbricks' changes at 1/4/16 checkers are +0.34% / -1.11% / -0.14%; VS Code
-0.05% / +0.08% / +0.43%; t3code -0.66% / -0.22% / -2.15%. This is not a wall-time threshold claim or a README
benchmark publish.

**Decision:** retain the stateless helper and focused coverage. The repeated 3.116% Formbricks instruction gain
clears the required 1% bar with margin; correctness gates pass and peak RSS has no material change. Retain the
census and cached-Finder rejection as prior art. Do not add the cache without target-specific evidence of a clear
additional whole-program gain.

## Reproduction and artifacts

Scratch artifacts are under `/tmp/tsrs-comment-search/`: census instrumentation and JSON, all three saved CLI
binaries with SHA-256 in `binary-provenance.json`, candidate patches, Go source provenance, edge-case generator,
scanner/AST oracle binaries and complete comparison reports, raw measurement output, and JSON summaries. The
Linux artifact is `depot-gate-results-xh2xbk2tvh.zip` (SHA-256
`a2990078ba4b5b8deabe82fbcc3dee3fc3e2fdbc9450deb85ffb8267dd9eba55`), extracted alongside it;
`depot-gate-summary-xh2xbk2tvh.json` verifies source/binary provenance and the reported counters. Retain
these locally for the current experiment; they are not repository build inputs.

```sh
DEVELOPER_DIR=/Library/Developer/CommandLineTools \
CARGO_PROFILE_RELEASE_STRIP=none CARGO_PROFILE_RELEASE_DEBUG=line-tables-only \
CARGO_TARGET_DIR=target/xctrace cargo build --release --locked -p tsrs_cli
python3 /tmp/tsrs-comment-search/measure.py --new stateless-tsrs --tag stateless --reps 5
python3 /tmp/tsrs-comment-search/measure.py --new cached-tsrs --tag cached --reps 5
python3 /tmp/tsrs-comment-search/measure-head-to-head.py --new cached-tsrs --tag head-to-head \
  --reps 5 --projects formbricks-web --modes single,frontend
```

From each project root, the full-check command is `tsrs -p <config> --noEmit --incremental false --pretty false
--extendedDiagnostics`, with `--singleThreaded` added for the serial cell. Configs are `src`,
`apps/web/tsconfig.typecheck.json` and `apps/server` respectively. Use `/usr/bin/time -l` for the macOS counters.
