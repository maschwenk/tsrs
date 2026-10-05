# perf-checker-stealing: dynamic scheduling of files over checkers

With a static assignment the slowest checker decides the check time. notes/perf-checker-scaling.md measured it 19%
above the mean at 8 checkers and 28% at 12 on the 38k-file codebase. Since output no longer depends on which checker
checks a file (notes/perf-order-independence.md), files can move at run time. This note compares two back-ends: checker
threads with tail work stealing, and forked checker processes pulling from shared-memory queues
(notes/perf-checker-processes.md). Threads with stealing are now the default.

## Design (threads, landed)

- Each checker keeps its locality-assigned files in program order and takes them from the front of its queue. A
  checker that runs out takes not-yet-started files from the back of the queue with the most estimated work left
  (`checked_file_weights`). This is tsrslint's scheduler (`tsrslint-claude/crates/tsrslint/src/sched.rs`). One `AtomicU64`
  per queue packs the front and back positions; a compare-exchange claims a position.
- A stolen file's owner changes. Later passes over it (declaration diagnostics, emit) then run on the checker that
  checked it, as before.
- Only the full type-check pass (`get_semantic_diagnostics` for all files) steals. Declaration diagnostics, emit and
  the incremental pass do not: the incremental pass attributes a newly found global diagnostic to the first file of
  that checker that produced it.
- Off, so the run is fully deterministic, when an assignment is named (`--checkerAssignment` /
  `TSRS_CHECKER_ASSIGNMENT`: `locality`, `go`, `file:`, `random:`) and under Go's check history. The tsgo-baseline
  harnesses are therefore unchanged.

What varies from run to run in the default mode: which checker checks a stolen file, and with it the
`--extendedDiagnostics` Types / Symbols / Instantiations counters and peak memory. Diagnostics and emitted files do not
vary (proofs below). `--checkers N --checkerAssignment locality` gives today's fully deterministic behaviour, counters
included.

## Results

Mac (18 cores, M5 Max, load ~16; medians of 3 for the 38k-file codebase, 2 for the rest). `/usr/bin/time` total
seconds, instructions and peak footprint:

| project | N | static s | stealing s | change | static instr | stealing instr | static GiB | stealing GiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 38k-file codebase | 4 | 8.49 | 7.57 | -10.9% | 370 G | 371 G | 5.69 | 5.73 |
| | 8 | 6.41 | 5.62 | -12.3% | 471 G | 483 G | 7.27 | 7.48 |
| | 12 | 5.15 | 4.90 | -4.9% | 533 G | 570 G | 8.46 | 8.93 |
| | 16 | 4.84 | 4.72 | -2.4% | 594 G | 648 G | 9.47 | 10.23 |
| vscode | 4 | 2.41 | 2.06 | -14.7% | 121 G | 121 G | 2.26 | 2.27 |
| | 8 | 1.83 | 1.83 | 0% (check -4.3%) | 126 G | 126 G | 2.51 | 2.52 |
| | 12 | 1.34 | 1.16 | -13.5% | 130 G | 130 G | 2.73 | 2.79 |
| | 16 | 1.18 | 1.02 | -14.1% | 132 G | 133 G | 2.90 | 2.98 |
| webpack | 4 / 8 / 12 / 16 | 0.27 / 0.23 / 0.19 / 0.18 | 0.25 / 0.18 / 0.17 / 0.16 | -7 / -20 / -12 / -14% | 16.4-20.5 G | +0-3% | 0.39-0.58 | +0-0.03 |
| xstate | 4 / 8 / 12 / 16 | 0.17 / 0.13 / 0.10 / 0.11 | 0.14 / 0.12 / 0.10 / 0.10 | -15 / -13 / -2 / -4% | 9-12 G | +0% | 0.24-0.36 | same |

Linux (x86-64, 18 vCPU, main with transparent huge pages, the 40k-error corpus, medians of 3, PSS):

| N | static s | stealing s | change | static instr | stealing instr | static GiB | stealing GiB |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 4 | 20.29 | 17.50 | -13.8% | 350 G | 354 G | 5.91 | 5.95 |
| 8 | 15.68 | 13.02 | -17.0% | 450 G | 475 G | 7.47 | 7.92 |
| 16 | 11.21 | 10.94 | -2.4% | 539 G | 634 G | 9.59 | 10.81 |

Per-checker thread CPU at 8 checkers on the 38k-file codebase goes from 3.3-5.1 s to 4.22-4.49 s: balanced. The cost is
first touches. A stolen file is checked by a checker that has not built its neighbourhood, so instructions grow with N
(+0-1% at 4, +2-5% at 8, +9-18% at 16), and so does memory (+0.04 to +1.2 GiB). At the default count (4-8) the gain is
11-17%. At 16 most of it goes into duplicated work. That is one reason the default stays `clamp(cores/2, 4, 8)`.

mui-docs is left out: its program reports option diagnostics, so the CLI never type-checks it.

## Processes with shared queues (not landed)

The parked fork spike (notes/perf-checker-processes.md) was extended with the same queues. The ranges live in a
`MAP_SHARED` page and the file lists are inherited copy-on-write. The warm-up resolves the exports of the 100
most-imported modules, and ids are pre-assigned. Mac, threads with stealing vs processes with stealing, total seconds,
medians:

| project | N | threads | processes | change | threads instr | processes instr |
| --- | --- | --- | --- | --- | --- | --- |
| 38k-file codebase | 4 / 8 / 12 | 6.98 / 4.72 / 4.46 | 6.46 / 4.85 / 4.49 | -7.5 / +2.7 / +0.7% | 373 / 483 / 573 G | 346 / 403 / 433 G |
| vscode | 4 / 8 / 16 | 2.28 / 1.40 / 0.98 | 2.15 / 1.40 / 1.08 | -6 / 0 / +9% | 122-133 G | +5% |
| webpack | 4 / 8 / 16 | 0.25 / 0.19 / 0.16 | 0.28 / 0.22 / 0.21 | +9 / +13 / +32% | | |
| xstate | 4 / 8 / 16 | 0.17 / 0.12 / 0.10 | 0.18 / 0.16 / 0.18 | +6 / +27 / +74% | | |

Processes balance just as well and do 15-27% fewer instructions on the 38k-file codebase, but they are not faster. On
small and medium programs the warm-up, id pre-assignment and forks cost more than they save. They also cannot run in
build mode, the language server, the API or on Windows, and they need a diagnostics wire format and the fork-safety
rules. Threads with stealing win; the process back-end stays parked on `perf/checker-processes-queue`.

## Proofs

- The 40k-error corpus: stealing vs static at N = 4, 8, 16 gives identical diagnostics (43,859 lines).
- vscode and webpack identical.
- Conformance with 4 checkers per test, stealing active (`TSRS_HISTORY=canonical`): result trees identical to
  canonical single-threaded in 3 runs of types / symbols and 1 of js / jsmap / sourcemap.
- Random assignments (notes/perf-order-independence.md) already cover arbitrary partitions and visit orders, of which
  stealing produces one.
- Unit tests: every queue position is handed out exactly once under concurrent owners and thieves (zero-weight
  positions included); without stealing a checker keeps its own files in order.

## Reproduce

```sh
tsrs -p . --noEmit --incremental false --extendedDiagnostics --pretty false --checkers $N                    # stealing
TSRS_CHECKER_ASSIGNMENT=locality tsrs -p . --noEmit --incremental false --extendedDiagnostics --pretty false --checkers $N   # static
TSRS_ASSIGNMENT_STATS=times tsrs ...    # per-checker wall and thread CPU seconds
```
