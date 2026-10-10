# perf-binary-layout: PGO training set, BOLT, huge pages and codegen settings on the 64-vCPU bench machine

Question: does how the Linux x86-64 `dist` binary is built leave speed on the table on the bench's 64-vCPU machine
(the README table)? notes/perf-build-level.md answered it on an 18-vCPU Intel host at 1-8 checkers; this repeats the
useful parts on Depot `depot-ubuntu-24.04-64` (AMD EPYC 9R45, KVM, kernel 6.12, THP `madvise`) at the default
checker count (32) and at 4.

Answer: nothing beyond what release.yml already does is worth its build time.

- The training runs of `tsrs` had been writing empty profiles since #143 (`_exit`). #153 fixed PGO. BOLT was still
  broken (release's BOLT step would have failed: #155). With the PGO profiles back, the binary is 1-3% faster in
  wall time than main's between #143 and #153.
- BOLT, as release.yml ships it, is -1% to -3% wall against the PGO-only binary the bench measured, with identical
  output. That is consistent in sign but inside the run-to-run spread. bench.yml now BOLTs its binary too, so it
  measures what ships and runs release's BOLT path on every push to main (+21 s build job).
- A larger PGO and/or BOLT training set (vscode, t3code-server, formbricks-web) helps only the projects it trained
  on, adds 2-4 minutes of instrumented runs, and was not adopted. `-hugify`: nothing. Codegen settings: nothing left
  to try.

## Method

`.depot/workflows/perf-layout-probe.yml` (run with `depot ci run`) builds every variant in one job from one
instrumented build (`tools/perf/layout-build.sh`), then measures them interleaved (`tools/perf/layout-measure.py`):
every bench project (`bench/run.py` `project_path`), `-p <project> --noEmit --incremental false --pretty false
--extendedDiagnostics`, default checker count (32 here) and `--checkers 4`, 7 rounds per run with the binary order
rotating each round. Wall time and peak RSS come from `wait4`. Then the full stdout without `--extendedDiagnostics`
at 1, 4 and 32 checkers is compared byte for byte against the baseline. Finally, one `--singleThreaded` run per
project and binary goes under `perf stat -e instructions,cycles,iTLB-load-misses,L1-icache-load-misses`. Tables give
the median of the per-round paired ratios against A, pooled over two runs (14 rounds), with the interquartile range
in brackets; "spread" is half the min-max range of A's 14 runs, as a percentage of its median.

Noise floor: two binaries built from the same profile (run `rnw36c4n82`, 7 rounds) differed by -1.8% to +1.9% per
cell, median of paired ratios.

Variants, all built from the same source:

| name | how |
| --- | --- |
| A0 | main between #143 and #153: the PGO profile has only the suite and fourslash counts (the tsrs runs wrote 0-byte profiles) |
| A | today's bench binary: `pgo-train.sh` with the tsrs profiles written, `cargo build --profile dist` |
| B | A's profile plus tsrs on vscode, t3code-server and formbricks-web at 4 and 32 checkers |
| R | A linked with `--emit-relocs`, then BOLT exactly as `bolt.sh`: instrumented, trained on xstate-main + webpack |
| Rv | R with the BOLT training extended by vscode at 4 checkers |
| Rx | R with the BOLT training extended by B's three projects at 4 and 32 checkers |
| RB | B linked with `--emit-relocs`, BOLT trained on xstate-main + webpack + B's projects |
| RBH | RB with `-hugify` (hot text remapped onto 2 MiB pages at startup) |

## 1. The training runs of tsrs wrote nothing (fixed in #153 and #155)

Status (2026-10-10): the `_exit` exit path was reverted in #191 (docs/STATUS.md). `crates/tsrs_cli/src/main.rs`
ends with `std::process::exit`, so the exit handlers that write PGO and BOLT profiles always run; `bolt.sh` and
`tools/perf/layout-build.sh` keep `MIMALLOC_SHOW_STATS=1` as a leftover. The mechanism below is history.

`tsrs` ends a command-line run with `libc::_exit` since #143. Both the LLVM profile runtime and BOLT's
instrumentation runtime write their data from exit handlers, which `_exit` skips; the LLVM runtime does create the
file at startup, so `pgo-train.sh`'s existence check passed. On main's bench build of 8b3e4f4,
`xstate-main-46772.profraw` and `webpack-46807.profraw` were 0 bytes, and the first probe (`rnw36c4n82`) got
identical profiles with and without three more training projects. #153 (another agent, landed while this ran) makes
tsrs call `exit` when `LLVM_PROFILE_FILE` is set. `bolt.sh` sets no such variable: its instrumented tsrs still left
no profile (`merge-fdata: '.../prof*': No such file or directory`), so release's "BOLT and gates" step would stop
at `no BOLT profile from tsrs`. #155 sets `MIMALLOC_SHOW_STATS=1` on those runs (tsrs then calls `exit`) and
requires non-empty profiles.

Effect of having the tsrs counts (run `bdm2hx5c9p`, A vs A0, 7 rounds, medians): -0.0% to -3.3% wall on every cell
but one (t3code-server at 32 checkers, +0.3%); vscode -3.2% / -2.7% (32 / 4 checkers). Coverage: A executes
427 `tsrs_compiler` functions in training, A0 366 (the CLI's own driver and the parallel orchestration).

## 2. BOLT (R vs A)

LBR is not available on this VM (`perf record -j any,u`: "PMU Hardware or event type doesn't support branch stack
sampling"; no `amd_lbr_v2` CPU flag), so this uses instrumentation mode, as `bolt.sh` does. BOLT reorders 2,315-2,333
functions (74% of the profiled ones); the hot text is 2.74 MB and the cold 2.42 MB, next to the original 13.45 MB
`.text`; the unstripped file goes from 107.8 to 116.2 MB.

| project | A wall s (32) | spread | R (32) | A wall s (4) | spread | R (4) |
| --- | ---: | ---: | --- | ---: | ---: | --- |
| vscode | 0.590 | ±4.7% | -1.4% [-3.7, +0.1] | 2.460 | ±4.7% | -1.9% [-2.7, -1.1] |
| xstate-main | 0.133 | ±5.7% | -1.3% [-4.0, +0.2] | 0.188 | ±3.9% | -0.9% [-3.1, +1.1] |
| webpack | 0.135 | ±10.5% | -1.6% [-5.5, +2.2] | 0.306 | ±4.0% | -0.3% [-4.3, +3.0] |
| mui-docs | 0.817 | ±4.8% | +0.4% [-2.4, +3.2] | 1.292 | ±3.1% | -0.2% [-0.9, +3.6] |
| Compiler | 0.076 | ±7.9% | -1.1% [-2.3, +2.4] | 0.076 | ±4.8% | +0.7% [-2.7, +3.6] |
| Compiler-Unions | 0.130 | ±13.2% | -1.5% [-3.5, +1.3] | 0.130 | ±12.4% | +1.2% [-1.0, +2.1] |
| cal-diy | 0.562 | ±2.2% | -1.3% [-3.0, -0.6] | 1.207 | ±5.1% | -3.3% [-5.5, -2.0] |
| formbricks-web | 0.650 | ±3.5% | -1.8% [-4.8, -0.6] | 1.290 | ±3.4% | -1.7% [-2.7, -0.5] |
| supabase-studio | 0.571 | ±3.2% | -1.2% [-2.1, -0.5] | 1.498 | ±4.7% | -2.1% [-3.1, -0.6] |
| t3code-server | 1.383 | ±7.5% | -2.8% [-5.7, +0.7] | 1.881 | ±10.9% | -1.1% [-2.5, +2.7] |

Peak RSS within ±1% except the two suite projects (-2% to -3.5% of a few MB). Instructions -0.2% to -0.3% (perf stat,
single-threaded), as on Intel: the gain is instruction fetch, not work. Smaller than the 18-vCPU Intel host's -3% to
-4% (notes/perf-build-level.md), and inside the spread here on every big project. It is what release ships, though,
so `bench.yml` now builds it too: link with `--emit-relocs`, then `bolt.sh` with `BOLT_TSRS_ONLY=1` (tsrs only:
same instrumentation, training and byte comparison; the suite gates stay in release.yml). Bench run `tq2x4vs94n`
from the branch: build job 7 min 42 s vs 7 min 21 s on main (0d3b6c08), build start to merge end 24 min 35 s vs
24 min 27 s.

## 3. A larger training set (B, Rv, Rx, RB)

Training vscode, t3code-server and formbricks-web at 4 and 32 checkers moves `tsrs_checker`'s share of the PGO
counts from 48.6% to 58.5% (executed functions barely change: 2,153 / 3,079 in both). It costs a lot: an
instrumented `tsrs` is ~10x slower (vscode at 4 checkers 27 s, t3code-server at 32 checkers 93 s; PGO counters are
shared memory that 32 threads write), 236 s for the six PGO runs and 130-153 s for the BOLT-instrumented ones, on
64 vCPUs. The release runners have 4.

- B alone (PGO only) was at the noise floor on wall time in run `bdm2hx5c9p` (-2.6% to +2.3% per cell vs A), even though RB
  retires 2-6.5% fewer instructions single-threaded on every project.
- RB vs A, pooled: -3.0% / -3.8% on vscode (32 / 4 checkers), -3.8% / -4.7% on t3code-server, -1.6% / -2.6% on
  formbricks-web: all three are training projects. On the projects it did not train on, it equals R: mui-docs
  +0.8% / -1.0%, cal-diy -1.6% / -2.8%, supabase-studio -1.3% / -1.8%.
- Rv and Rx (only BOLT's training extended, run `rpfkgj3rdr`) are R within noise on every held-out project.

Training on the bench's own projects makes the bench faster, not the binary in general. Not adopted.

## 4. Huge pages for the hot text (RBH vs RB)

`iTLB-load-misses` are 0.01-0.05 M per single-threaded run (vscode: 0.03 M against 48 G cycles), so there are no
iTLB misses for huge pages to remove. `-hugify` is RB within noise (paired, 7 rounds: -2.3% to +3.2% at 32
checkers, -0.9% to +2.0% at 4), as on Intel. The `max-page-size=0x200000` + `MADV_COLLAPSE` variant would remap the
same pages less directly, so it was not built.

## 5. Codegen settings

- `panic = "abort"`: not built. `catch_unwind` is on the CLI's own path (`tsrs_compiler/src/checkerpool.rs`,
  `tsrs_cli/src/build/orchestrator.rs`) as well as in the LSP, API and fourslash crates, so it changes behavior,
  not only codegen (notes/perf-build-level.md section 3).
- `opt-level`: `dist` inherits `release`'s 3. `debug = "line-tables-only"` / `strip` change the file size, not the
  code; BOLT keeps the line tables (`-update-debug-sections`), which panic backtraces use.
- `-C target-cpu` above the baseline is out (portable binary).

## 6. Where the cycles go (A, vscode, 32 checkers, `perf record -F 1999`, flat)

No single hot spot: `check_expression_ex` 4.0%, `SymbolMap::search` 3.5%, `NameResolver::resolve` 3.0%,
`instantiate_type_with_alias_worker` 2.7%, `get_type_at_flow_node` 2.1%, `is_related_to_ex` 1.7%, `Scanner::scan`
1.7%, `assign_symbol_id` 1.6% + `assign_node_id` 1.2%. mimalloc ~3% (`_mi_page_malloc_zero` 1.2%, `mi_free` 1.1%,
`mi_theap_malloc_aligned` 0.7%), kernel `clear_page_erms` 0.9% and `__pv_queued_spin_lock_slowpath` 0.9% (page
faults and a contended lock under KVM), libc 0.9%. Nothing here is a build-level lever; `assign_*_id` and the
symbol-map search are candidates for the source-level work.

## Build time on 64 vCPUs (probe, for scale)

| step | time |
| --- | ---: |
| instrumented build (tsrs, tsrs-test, tsrs-fourslash) | 230-239 s |
| `pgo-train.sh` | 20 s |
| extra PGO training (B) | 236 s |
| final dist builds, 5 in parallel | 142-156 s |
| BOLT: instrument tsrs / train on xstate-main + webpack / optimize | 9 s / 5 s / 6 s |
| extra BOLT training (Rx, RB) | 130-153 s |

## Runs

- `rnw36c4n82`: A vs A with identical profiles (the empty-profile discovery; noise floor).
- `bdm2hx5c9p`: A0, A, B, R, RB, RBH; 7 rounds; 150 byte-identical comparisons.
- `dpc1dkttvw`: same variants; timings lost (perf stat output path bug), profile and coverage kept.
- `rpfkgj3rdr`: A, R, Rv, Rx, RB, RBH; 7 rounds; perf stat; 150 byte-identical comparisons.
- `tq2x4vs94n`: bench.yml from `perf/bench-bolt` (BOLT in the build job), end to end.
