# perf-pgo: PGO + fat LTO for the release binaries

Question: is profile-guided optimization (with fat LTO and one codegen unit) worth it for the shipped `tsrs`
binaries? Answer: yes. PGO cuts CPU time by 7-14% and instructions by 13-14% on every project measured,
counters and output unchanged. Release builds now use it (`.github/workflows/release.yml`, profile `dist` in
`Cargo.toml`, training script `.github/scripts/pgo-train.sh`), and so does the README benchmark
(`.depot/workflows/bench.yml`, since 2026-10-01). `cargo build --release` (development) was unchanged at the time.

Status (2026-10-10): `[profile.release]` in Cargo.toml is now fat LTO, one codegen unit, `debug = false` and
`strip = "symbols"` (the strip setting came with PR #245, section "2026-10-09" below; the size change is at the end),
so `release` differs from `dist` only by PGO, and on Linux BOLT. Binary (a) below is the `release` profile as it was
when this was measured; `cargo build --release` numbers in notes older than that change are from the thin-LTO profile.

## Binaries

- (a) the `release` profile of the time (opt-level 3, 16 codegen units, thin-local LTO; since changed, see the status
  above).
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
training ~15 s + final build 72-76 s, ~3 min in total (~+2.5 min over a). On GitHub runners (release dry run
36922215992, warm rust-cache) the build jobs went from 1.3-2.1 min to 4.7 min (linux x64), 6.2 (linux arm64),
8.7 (macOS arm64) and 10.5 (macOS x64: the training run is ~2x slower under Rosetta); the instrumented build takes
1.5-4 min, the training run 1-2.5 min (incl. the bench projects' clone + install, uncached), the final build
1.4-2.7 min. The job timeout is now 90 minutes.

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

## C code (mimalloc) is not PGO-optimized

cc (1.5) forwards `-Cprofile-generate` / `-Cprofile-use` from RUSTFLAGS to clang (not to gcc). On macOS that
instruments mimalloc with Apple clang, whose instrumentation does not have to match the profiler runtime of
rustc's LLVM: with rustc 1.95 it happened to work, with 1.99 (the release runners' stable on 2026-10-01) every
instrumented binary crashed at startup in `__llvm_profile_instrument_target` called from mimalloc. The workflow
sets `CFLAGS=-fno-profile-generate` / `-fno-profile-use` on macOS (CFLAGS come last on cc's command line), so
mimalloc is built as before on every platform. Measured: same instructions as with an instrumented mimalloc
(private monorepo 266.3 vs 266.2 G single, 359.1 vs 358.9 G on 4 checkers), and the conformance and counter gates
above were rerun on this binary with the same result.

## Notes

- Training on the bench projects as well as the suite was the plan from the start; a suite-only profile was not
  compared. The private monorepo was not used for training.
- The bench CI table (README) measures the shipped binary: (c), built and trained on the bench runner with the
  release workflow's commands for x86_64-unknown-linux-gnu (before 2026-10-01 it measured (a)).
- Linux release targets train natively on their own runners; they were not measured here.

## Language-service training (2026-10-02)

The training set above never ran `tsrs --lsp`, so the language service (`tsrs_ls`, `tsrs_project`, `tsrs_lsp`) was
laid out as cold code. `pgo-train.sh` now also runs the fourslash suite with an instrumented `tsrs-fourslash`
(4,546 tests through the in-process server; worker processes exit normally, `%m` merges their counts), and the
release and bench workflows build that binary in the instrumented step.

Same source and instrumented build, final binaries differing only in the profile, 200-edit editor session on
xstate-main (`tools/lsp-mem/lsp_mem.py --completion`: diagnostics, hover and completion after every edit), server
process: 94.4 G -> 85.0 G instructions (-10%), ~34.9 G -> ~31.5 G cycles (2 rounds each, machine under load). The
checker is unaffected on the private monorepo: 269.2 G vs 268.9 G instructions with one checker, 365.9 G vs
363.7 G with four; identical output. A `workflow_dispatch` dry run of the release produced a fourslash profile on
all four targets (macOS x86_64 under Rosetta included).

## 2026-10-07: the CLI's `_exit` skipped the profile write

PR #143 ends a command-line run with `_exit` once the output is flushed. The LLVM profile runtime writes `.profraw`
from an exit handler, so the two `tsrs -p` training runs (xstate-main, webpack) left 0-byte files from the bench
build of 8b3e4f4 on (`depot ci logs w7mq83j6f0`: `webpack-46807.profraw` and `xstate-main-46772.profraw` at 0 bytes,
against 6.1 MB each in the build of 7262f61), `llvm-profdata merge` took them without a word, and the dist binary
was trained on the in-process suites only. The README bench showed it as +1.8-3.5% single-threaded user-space
instructions on every project between those two publishes while a release-build pr-verify of the same sources
measured -0.2%. `finish` now calls `exit` when `LLVM_PROFILE_FILE` is set (only the training sets it), and
pgo-train.sh fails on an empty profile.


## 2026-10-09: a strip setting that differed between the two builds dropped the profile

PR #245 set `strip = "symbols"` in `[profile.release]`, which `dist` inherits, and `CARGO_PROFILE_DIST_STRIP=none`
for the final build on Linux in release.yml and bench.yml, because BOLT needs the symbols. The instrumented build kept
`symbols`. Cargo hashes the profile, `strip` included, into each crate's `-C metadata`, which is part of every symbol
name, and the final build looks a function's counts up by its symbol name. So on Linux it found counts for no
function, without a warning, and the README bench measured +10.6% to +18.0% single-threaded instructions on every
project between 0f136be0 and 20261d4d. macOS builds were not affected: they use `symbols` in both builds. Both
workflows now give the instrumented build the final build's value.

Reproduced on macOS arm64 with the workflows' commands (instrumented build, pgo-train.sh, `llvm-profdata merge`,
final build with `-Cllvm-args=-pgo-warn-missing-function`), on main at cab0f26e unless noted. "Same metadata": of
the 56 crates both builds compile, those whose `-C metadata` is the same in both. Instructions: `/usr/bin/time -l`
with `--singleThreaded` and `RAYON_NUM_THREADS=1`, minimum [median] of 5 interleaved runs; macOS counts include
kernel work, so they spread by up to a few percent where bench/count.py's Linux counts repeat to 0.001%.

| build | same metadata | functions without profile data | vscode G | drizzle-orm G |
| --- | ---: | ---: | ---: | ---: |
| final `none`, instrumented `symbols` (Linux before the fix) | 0 | 24,738 | 97.68 [97.73] | 14.73 [15.00] |
| `symbols` in both (macOS) | 56 | 1,599 | 84.29 [84.86] | 12.84 [13.03] |
| `none` in both (Linux with the fix) | 56 | 1,599 | 83.83 [84.99] | 12.86 [12.98] |
| 0f136be0 (before #245), its own steps | 56 | 1,599 | 84.73 [84.78] | 12.82 [13.23] |

Diagnostics and exit codes were the same in every run. The mismatched build is +15.3% / +14.9% over 0f136be0, the
bench's +15.8% / +16.2%.

Size (moved here from a former note on the release profile): on macOS arm64 at 3183bbd3, a clean `tsrs_cli` build with the
new `[profile.release]` was 20,233,280 bytes against 33,226,472 before (-39.1%). Stripping symbols and debug
information changes file metadata, not generated code, so it does not change retired instructions or peak RSS. The
Linux `dist` build stays unstripped until after BOLT: release.yml and bench.yml set `CARGO_PROFILE_DIST_STRIP=none`
for the PGO-use build (and, since the fix, the instrumented build), and `bolt.sh` strips the optimized `tsrs` after
BOLT's training, optimization and correctness comparisons.

## 2026-10-10: ThinLTO for the instrumented build

The release, benchmark and benchmark-comparison workflows append `-Clto=thin` to the
`-Cprofile-generate` RUSTFLAGS. The final build uses fat LTO with the collected profile; Linux also runs BOLT.
This follows [Charlie Marsh's training-build experiment](https://x.com/charliermarsh/status/2108901980504567932).

Use the rustc override rather than `CARGO_PROFILE_DIST_LTO=thin`. Cargo hashes its profile and LTO plan into
crate metadata, but excludes RUSTFLAGS from symbol metadata ([pinned Cargo implementation](https://github.com/rust-lang/cargo/blob/5f94df478/src/compiler/build_runner/compilation_files.rs#L776-L873)).
The trailing rustc option overrides Cargo's `-C lto=fat` without changing the function names PGO matches.
Both LLVM pre-link pipelines insert the ordinary PGO counters before their post-link LTO work diverges
([pinned rustc pipeline](https://github.com/rust-lang/rust/blob/b940084d7eb6a299eb4bfeb8e34901bc051e7ac4/compiler/rustc_llvm/llvm-wrapper/PassWrapper.cpp#L924-L935)).

### Build time

Source `831df111530243c753150bf4370c368719411cd1`, Rust 1.99.0 / LLVM 23.1.1, native
`aarch64-apple-darwin`, 18-core Apple Silicon, 128 GiB RAM, shared machine. One paired run with fresh Cargo
target directories and 18 build jobs. Both passes build `tsrs`, `tsrs-test` and `tsrs-fourslash` so the final
binaries can run the existing correctness gates. The macOS release's final pass builds only `tsrs`, so these
totals describe the comparison pipeline. Project clone/install time and Linux BOLT are excluded.
The initial fat training step refreshed the webpack install; the reported training time is its repeat with
prepared projects. The final binaries use the original profiles, each trained once.

| Step | Fat training (s) | Thin training (s) | Change |
| --- | ---: | ---: | ---: |
| Instrumented build | 186.42 | 116.45 | -37.5% |
| Training, prepared projects | 16.49 | 18.50 | +12.2% |
| Profile merge | 0.20 | 0.20 | -2.2% |
| Final fat-LTO build | 148.49 | 151.53 | +2.0% |
| Total | 351.61 | 286.68 | -18.5% |

The recording build saves 70 seconds (37.5%); this comparison pipeline saves 65 seconds (18.5%).
These are single-pair build timings, not a cross-platform release-time estimate.

### Profile application and runtime

`cargo build -vv` shows identical metadata for all 61 common target compilation units between generation
and use in both pipelines. Both final builds enable `-Cllvm-args=-pgo-warn-missing-function` and report zero
PGO hash mismatches. Missing-function warnings rise from 2,421 to 3,076. Of the 699 function names missing
only with thin training, 304 have zero counters in the fat profile and 395 have no entry there either;
none has nonzero counters in the fat profile. All four workloads write nonempty profiles.

Five interleaved rounds per binary on the current `bench/projects.json` pins: one checker with
`RAYON_NUM_THREADS=1`, and the default checker count. Medians below; `/usr/bin/time -l` instruction counts
include kernel work. Wall time on this shared machine is noisy. Peak RSS is the running compiler's, not the build's.

| Project / mode | Instructions (G), fat / thin | Change | Peak RSS (MiB), fat / thin | Wall (s), fat / thin |
| --- | ---: | ---: | ---: | ---: |
| vscode / single | 83.07 / 83.07 | -0.00% | 1604.4 / 1604.5 | 6.44 / 6.35 |
| vscode / default | 95.68 / 95.60 | -0.08% | 2108.2 / 2104.5 | 0.74 / 0.75 |
| webpack / single | 11.26 / 11.05 | -1.80% | 311.1 / 311.1 | 0.74 / 0.73 |
| webpack / default | 15.45 / 15.59 | +0.90% | 507.0 / 510.7 | 0.12 / 0.12 |
| mui-docs / single | 45.17 / 44.21 | -2.14% | 732.1 / 732.1 | 2.63 / 2.65 |
| mui-docs / default | 115.18 / 115.59 | +0.35% | 1633.0 / 1635.7 | 0.98 / 1.04 |
| t3code-server / single | 40.15 / 39.97 | -0.45% | 734.4 / 734.4 | 2.65 / 2.69 |
| t3code-server / default | 128.82 / 127.35 | -1.14% | 2208.4 / 2178.4 | 1.12 / 1.08 |
| formbricks-web / single | 43.35 / 42.82 | -1.23% | 1100.3 / 1100.3 | 2.93 / 2.89 |
| formbricks-web / default | 75.92 / 75.78 | -0.18% | 1992.8 / 1995.2 | 0.59 / 0.60 |

The initial MUI default-mode wall increase (+6.1%) did not repeat: ten additional interleaved rounds gave
1.040 / 1.015 seconds (-2.4%), +0.24% instructions and +0.08% peak RSS. No consistent runtime regression was
observed. The main comparison's largest instruction increase is 0.90%; largest peak-RSS increase is 0.73%.
The runtime measurements cover macOS; Linux performance has not been measured in this experiment.

### Correctness

Full diagnostic output and exit codes match byte for byte on all five projects at 1, 4 and 16 checkers
(15 comparisons). Existing conformance gates pass for both final builds in reference and canonical histories,
with identical outcome lists: 13,458 error-baseline passes; 12,779 `.types` and `.symbols` passes in reference
history; 12,778 each in canonical history with its documented `objectLiteralNormalization` difference.
Both fourslash gates pass with the same 4,066 passes / 63 known failures. There are no conformance crashes or timeouts.

Raw build, runtime, profile-coverage and gate measurements for this run are in `/tmp/tsrs-thin-pgo/ab/`.
