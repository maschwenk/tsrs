# Oxc numeric conversion and Dragonbox (2026-10-11)

## Decision

Applied at the owner's explicit request on 2026-10-11 to align numeric conversion and formatting with Oxc,
despite not clearing the performance gate. Neither Dragonbox formatting alone nor formatting plus Oxc-style
numeric-literal conversion established a qualifying whole-project improvement. After repeating the noisy xstate
measurement, the largest median paired single-threaded instruction reduction was 0.030%; default-mode peak RSS
was essentially unchanged. Both changes are retained without a speedup claim. Capability counts and the README
table are unchanged.

Further performance work needs a profile showing numeric conversion as a material cost on a configured benchmark
project, or new evidence of a qualifying end-to-end improvement. A formatter microbenchmark alone does not meet
the gate.

## Candidates

The reference was the local Oxc checkout at `4b756621b65ab54c19dc66fbdcb3cd902d662c20`, specifically
`crates/oxc_parser/src/lexer/number.rs` and `crates/oxc_syntax/src/number.rs`.

- **format:** `dragonbox_ecma` 0.1.12 in `tsrs_core::jsnum::Number::string`, after the existing non-finite and
  safe-integer cases. `Buffer::format_finite(...).to_owned()` replaced the custom shortest-decimal formatting and
  big-integer rounding-tie correction. Its now-unused `big::Int::mul` and `lsh` wrappers were removed too.
- **both:** the formatter plus a private scanner conversion helper for normalized, separator-free literal text.
  Decimal integers of at most 19 digits accumulated in `u64`; decimal floats and longer integers used Rust's
  `str::parse::<f64>()`, as Oxc does. Binary/octal/hex fast paths accepted at most 64/21/16 digits, respectively.
  Larger radix literals used Oxc's retained significand, rounding bit and sticky bit, followed by exact
  power-of-two scaling. The helper validated digits even after overflow and fell back to `jsnum::from_string`
  for spellings outside its contract.

The scanner kept its existing token boundaries, flags, diagnostics, recovery, normalization, canonical-string
reuse and caches. Legacy-octal clamping, BigInt handling, general-purpose `jsnum::from_string`, and the separate
JSON formatter were unchanged. No public API, parser dependency, unsafe code, thread or cache was added.

## Method and limits

Apple M3 Max, 14 cores, 36 GiB RAM, macOS 27.0.1, rustc 1.99.0. All binaries were release builds from one isolated
snapshot of `0188eed0333447c52f57a9ed8afd4ab29f65d088` plus the pre-existing uncommitted workspace changes, including
the Unicode and project/headless work. Those changes were identical across variants. Only the numeric experiment
varied. No PGO or BOLT.

Four cached projects from `bench/projects.json` were measured: Compiler and Compiler-Unions from
typescript-benchmarking `41f652ab2df5077b1115f73eaeefc2fe9f674132`, xstate
`fbee62e7c1586315ed478c2fedf530d7e0ff5a3e`, and vscode `9cf0128b9822dcd8a533f08ef7312956a4390301`.
The other configured projects were not measured.

For each project and mode, one warm-up followed by five interleaved pairs, reversing binary order each round:

```sh
/usr/bin/time -l -o run.time <binary> -p <project> --noEmit --incremental false --pretty false
# Single-threaded: add --singleThreaded and RAYON_NUM_THREADS=1.
# Default: neither thread flag nor RAYON_NUM_THREADS override.
```

The tables report median instructions in billions from single-threaded runs and median maximum RSS in MiB from
default-mode runs. Percentage columns are the **median of the paired percentage changes**, not the ratio of the
two independent medians; these can differ, especially with noisy samples.

These are macOS resource-counter screening measurements, not the deterministic Linux `bench/count.py` gate.
Instruction counts fluctuated on this host. Other local work and candidate builds overlapped parts of the first
screening, so its wall times do not support a performance claim. No two-publish headline comparison was made.

The initial five-pair xstate combined run was particularly noisy: independent instruction medians were
7.312440 -> 7.115859 G, but the median paired change was only -0.770%. A fresh 15-round interleaved comparison of
all three binaries, after the builds and correctness runs completed, did not reproduce that apparent gain.
The tables use this repeat for xstate; its single-threaded paired changes were +0.072% for formatting and -0.017%
for both. Nothing establishes the 1% instruction, 2% headline wall-time or 5% default-memory landing threshold.

### Formatting alone

| Project | Single instructions, G: before → after | Paired change | Default peak RSS, MiB: before → after | Paired change |
| --- | ---: | ---: | ---: | ---: |
| Compiler | 2.152016 → 2.152528 | +0.055% | 76.844 → 76.641 | +0.527% |
| Compiler-Unions | 4.833461 → 4.838664 | +0.108% | 80.781 → 80.953 | +0.135% |
| xstate-main | 7.095379 → 7.097296 | +0.072% | 256.156 → 256.656 | +0.232% |
| vscode | 98.597677 → 98.458094 | -0.016% | 1943.266 → 1937.172 | -0.198% |

### Formatting and numeric conversion

| Project | Single instructions, G: before → after | Paired change | Default peak RSS, MiB: before → after | Paired change |
| --- | ---: | ---: | ---: | ---: |
| Compiler | 2.144642 → 2.144446 | -0.030% | 76.859 → 77.359 | +0.872% |
| Compiler-Unions | 4.831724 → 4.828831 | +0.006% | 82.203 → 82.500 | +0.246% |
| xstate-main | 7.095379 → 7.100297 | -0.017% | 256.156 → 256.656 | -0.030% |
| vscode | 98.592410 → 98.661650 | +0.070% | 1956.188 → 1947.406 | +0.274% |

## Correctness

- The candidate formatter matched the Go `jsnum.Number.String` implementation at pinned TypeScript commit
  `b85298b6a81f772d080b0455de0ca9d744cd6fd6` for 162,466 values: 100,000 random bit patterns with seed 20261011,
  exponent boundaries and their neighbors, signs, subnormals, non-finite values and formatting transitions.
- Scanner oracle: all 18,014 files matched pinned Go and the baseline exactly, including 14,296 generated numeric
  literals covering radix lengths, separators, large integers and halfway rounding. The oracle compares token
  values, flags, spans and ordered errors in four scan modes.
- Core and scanner units: 100 core cases passed, one ignored; nine scanner cases passed. All 28 regression
  projects passed. Every benchmark run preserved diagnostic bytes and exit status.
- Full compiler/conformance run: 15,197 variants, no lost or gained passes. Both baseline and candidate passed
  13,462 diagnostic, 12,779 types, 12,779 symbols, 13,392 JavaScript, 149 JavaScript-map and 156 sourcemap-text
  cases; the rest were skipped for those baselines. Two baseline cases exceeded the initial 20-second limit and
  passed when retried with 120 seconds; the candidate used 120 seconds and had no timeouts or crashes.
- Workspace check, WASM (`wasm32-wasip1`), lint ratchet and source checks passed. The initial ratchet caught the
  two newly unused big-integer wrappers; removing them resolved it without changing the lint baseline.
- The CLI memory unit `api::memory_tests::failed_builds_free_their_orchestrator` exited with SIGSEGV. This also
  reproduced in a separate build with the original numeric code; the full CLI unit run is therefore not claimed
  as passing. This is a limitation of the starting workspace, not a new numeric regression.

Local experiment artifacts remain under `target/scratch/numeric-alignment/` (git-ignored): the numeric-only
`candidate.patch`, candidate sources and binaries, `measurements-base-format.json`, `measurements-base-both.json`,
`measurements-repeat-base-format-both.json`, measurement and oracle scripts, and validation logs. These preserve
the original comparison; the combined candidate is now applied to the working source.
