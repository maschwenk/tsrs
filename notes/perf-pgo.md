# perf-pgo: PGO + fat LTO for the release binaries

Question: is profile-guided optimization (with fat LTO and one codegen unit) worth it for the shipped `tsrs`
binaries? Answer: yes. PGO cuts CPU time by 7-14% and instructions by 13-14% on every project measured,
counters and output unchanged. Release builds now use it (`.github/workflows/release.yml`, profile `dist` in
`Cargo.toml`, training script `.github/scripts/pgo-train.sh`). `cargo build --release` (development, bench CI)
is unchanged.

## Binaries

- (a) current `release` profile (opt-level 3, 16 codegen units, thin-local LTO).
- (b) (a) + `lto = "fat"`, `codegen-units = 1` (= profile `dist`).
- (c) (b) + PGO: `-Cprofile-generate` build of tsrs + tsrs-test, training run, `llvm-profdata merge`,
  `-Cprofile-use` build (rustc 1.95, llvm-tools from rustup; `cargo pgo` not needed).

Training workload (reproducible on a GitHub runner, no private code): the conformance suite
(`tsrs-test run --suite all`, default mode, no `--baselines`) plus the `tsrs` binary on xstate-main and webpack
from `bench/projects.json` (default mode, 4 checkers). tsrs-test runs the checker in-process, but it links the very
same crate units as tsrs (cargo builds `tsrs_checker` etc. once, with identical metadata hashes for
`-p tsrs_cli` alone and together with `-p tsrs_testrunner`), so its counts apply to tsrs's checker code. With
`-Cllvm-args=-pgo-warn-missing-function` the final arm64 build reports 1,340 functions without profile data
(never-executed code and helpers inlined before instrumentation), 0 hash mismatches. Training takes ~15 s on an
18-core M-series machine (suite 12 s instrumented).

## Results (Apple M-series, 18 cores, shared machine at load 5-10)

Medians of 5 interleaved runs (a, b, c, a, b, c, ...), `/usr/bin/time -l`; brackets: min-max; % vs (a). The
private monorepo has 9 errors in its current checkout, the same 9 (file, line, col, code) as `tsgo-ref`.
Measured on vscode and mui-docs, which are not part of the training workload.

| project | binary | user+sys s | user s | wall s | instructions G | cycles G |
| --- | --- | --- | --- | --- | --- | --- |
| private monorepo `--checkers 1` | a | 26.81 [26.65-29.54] | 20.07 | 19.03 [17.19-20.71] | 306.6 [306.0-307.5] | 116.0 [114.4-125.7] |
| | b | 29.05 [26.08-29.49] +8.4% | 20.54 | 19.87 [17.25-20.37] | 299.8 [298.5-300.1] -2.2% | 123.6 [112.4-125.4] +6.6% |
| | c | 24.95 [24.23-25.74] **-6.9%** | 16.84 (-16%) | 16.21 [15.31-18.72] -14.8% | 265.3 [265.1-265.3] **-13.5%** | 106.2 [101.7-110.1] **-8.4%** |
| private monorepo, 4 checkers | a | 41.81 [38.45-47.28] | 34.01 | 10.46 [9.04-15.77] | 416.3 [414.4-418.6] | 171.8 [159.6-190.8] |
| | b | 40.73 [36.76-44.07] -2.6% | 32.69 | 10.29 [8.66-11.45] | 407.7 [405.5-408.8] -2.1% | 166.2 [155.4-172.6] -3.3% |
| | c | 37.41 [31.25-41.13] **-10.5%** | 28.94 (-15%) | 9.15 [7.03-11.49] -12.5% | 359.2 [357.4-360.2] **-13.7%** | 149.2 [132.6-160.2] **-13.2%** |
| vscode (4 checkers) | a | 11.80 [10.73-12.21] | 9.74 | 2.96 | 124.9 | 49.6 |
| | b | 12.98 +10.0% | 10.88 | 3.60 | 121.3 -2.9% | 46.8 -5.7% |
| | c | 10.49 **-11.1%** | 8.11 | 2.66 -10.1% | 106.8 **-14.4%** | 42.0 **-15.4%** |
| mui-docs (4 checkers) | a | 7.15 [6.74-9.95] | 4.79 | 1.93 | 84.2 | 29.9 |
| | b | 7.39 +3.4% | 4.48 | 1.81 | 82.2 -2.3% | 31.6 +5.8% |
| | c | 6.14 **-14.1%** | 3.65 | 1.49 -22.8% | 72.9 **-13.4%** | 26.2 **-12.4%** |

- Instructions are the stable metric (spread < 1%); cycles and CPU time move with the machine's load (other
  agents), but (c)'s CPU median is below (a)'s minimum in single mode and its cycles median is below (a)'s
  minimum in all four rows.
- System time (~8 s on the private monorepo: page faults of a 6-8 GB heap) does not change; the gain is all
  in user time (-15/-16%).
- Fat LTO alone (b): -2-3% instructions, CPU/cycles within noise (as in notes/speed-frontend.md). It is kept in
  (c) because PGO is applied with it; PGO without fat LTO was not measured.
- Peak memory and every `--extendedDiagnostics` counter are identical across the three binaries.

Binary size (arm64): a 16.3 MB (12.9 MB stripped), b 13.3 MB (11.5), c 13.4 MB (11.6).

Build time (clean target dir, both binaries, same machine): a 34 s; b 68 s; c = instrumented build 87-90 s +
training ~15 s + final build 72-76 s, ~3 min in total (~+2.5 min over a). On GitHub runners expect more, since the
one codegen unit of `tsrs_checker` serializes its compilation (the build job's timeout is now 90 minutes).

## macOS x86_64 (cross-compiled on the arm64 runner)

The arm64 profile does not fit the x86_64 build: profile entries are keyed by mangled symbol names, and the
symbol hashes include the crate's metadata hash, which cargo derives from the target triple. With the arm64
profile the x86_64 build had 9,306 functions without data (1 hash mismatch); with a profile from its own x86_64
instrumented build 1,787. So the x86_64 job builds an x86_64 instrumented binary and runs the same training under
Rosetta 2 on the arm64 runner (~80 s locally, including a cold clone + install of the two bench projects); profile
counts are counts of the program's own branches, so they do not depend on running under translation.

Measured under Rosetta 2 on the same machine (no x86_64 hardware available; Rosetta translates ahead of time, so
relative differences are indicative, not exact): xa = release, xc = dist + arm64 profile, xr = dist + x86_64
profile trained under Rosetta.

| project | binary | user+sys s | instructions G | cycles G |
| --- | --- | --- | --- | --- |
| private monorepo `--checkers 1` (n=3) | xa | 45.81 | 498.2 | 194.2 |
| | xc | 45.50 -0.7% | 478.9 -3.9% | 192.3 -1.0% |
| | xr | 39.97 -12.7% | 416.2 -16.4% | 169.6 -12.7% |
| vscode (n=5) | xa | 17.37 | 207.9 | 73.2 |
| | xc | 16.89 -2.8% | 198.7 -4.4% | 71.1 -2.8% |
| | xr | 15.58 -10.3% | 174.1 -16.2% | 65.7 -10.2% |

## Correctness gates (PGO binary from the workflow's exact commands, profile `dist`)

- Conformance `tsrs-test run --suite all --baselines types,symbols`, default mode and `TSRS_LAZY_MEMBERS=0`:
  whole `TSRS_TEST_RESULTS` trees identical to (a)'s except the per-test `ms` fields in `summary.json`
  (12,779 pass / 0 codes / 0 fail / 0 crash in both modes).
- The private monorepo, `--checkers 1` and 4 checkers: the same 9 errors and identical `--extendedDiagnostics`
  counters (files, lines, identifiers, symbols, types, instantiations and the lazy-member counters) as (a).

## Notes

- Training on the bench projects as well as the suite was the plan from the start; a suite-only profile was not
  compared. The private monorepo was not used for training.
- The bench CI table (README) measures `cargo build --release`, i.e. (a), not the shipped binaries.
- Linux release targets train natively on their own runners; they were not measured here.
