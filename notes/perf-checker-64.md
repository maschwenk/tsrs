# perf-checker-64: the 16 -> 64 checker range on a 64-core machine

notes/perf-checker-scaling.md and notes/perf-checker-stealing.md cover 1 to 16 checkers on an 18-core Mac. On a
64-vCPU machine the check phase of vscode stopped improving past 16 checkers: 0.79 s at 16, 0.65 s at 32, 0.66 s at 64,
while bun check's check phase takes 0.51 s on its 64 threads. This note measures 8 to 64 checkers on that machine,
splits the gap into its causes, lands the one fix that pays, and re-derives the default checker count.

Machine: Depot `depot-ubuntu-24.04-64` (64 vCPU AMD EPYC 9R45, one thread per core, 8 x 32 MiB L3, one NUMA node,
252 GB), `cargo build --release` (not the PGO dist build), vscode `src` (10,427 files, 9,399 type checked, 371
errors), `--noEmit --incremental false --extendedDiagnostics --pretty false`. Every measured run has
`TSRS_ASSIGNMENT_STATS=times` (per-checker wall, thread CPU and stolen files; the `stolen files` line is new) and
runs under `bench/count.py` (user-space instructions, peak RSS). Medians of 3 unless stated. The three probe runs were
`depot ci run --workflow .depot/workflows/perf-probe.yml` with three versions of tools/perf/probe.sh (the committed
one is the default curve); their artifacts are not committed.

## 1. Where the gap to linear goes at 24-64 checkers

Slowest checker CPU = (one-checker CPU / k) x (b) x (c) x (a), as in notes/perf-checker-scaling.md: (b) checker
instructions at k over k = 1 (duplicated first-touch work), (c) CPU per checker instruction at k over k = 1 (slower
cores: cache and memory contention, clocks), (a) slowest / mean checker CPU (imbalance). One checker: check 10.37 s,
118.6 G instructions of which 21.5 G are the front end (`--listFilesOnly`), 2.80 GiB. Program order (main):

| checkers | check s | Total s | slowest checker CPU s | mean checker CPU s | (a) slowest / mean | CPU sum s | instructions G | (b) checker instructions | (c) CPU per instruction | stolen files | peak GiB |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 8 | 1.42 | 1.73 | 1.42 | 1.42 | 1.01 | 11.3 | 129.1 | +11% | -1% | 326 | 3.24 |
| 16 | 0.79 | 1.10 | 0.79 | 0.78 | 1.02 | 12.4 | 136.6 | +19% | +1% | 563 | 3.59 |
| 24 | 0.78 | 1.10 | 0.78 | 0.56 | 1.40 | 13.4 | 142.0 | +24% | +5% | 287 | 3.88 |
| 32 | 0.65 | 0.97 | 0.65 | 0.47 | 1.39 | 15.0 | 148.2 | +30% | +11% | 299 | 4.15 |
| 48 | 0.66 | 0.98 | 0.65 | 0.36 | 1.80 | 17.3 | 155.4 | +38% | +22% | 505 | 4.60 |
| 64 | 0.66 | 0.99 | 0.66 | 0.32 | 2.10 | 20.2 | 160.8 | +43% | +36% | 553 | 4.99 |

The check phase ends within 5 ms of the slowest checker at every count (the `Check time` minus the slowest
checker's wall).

- **(a) dominates from 24 up, and it is one file.** From 24 to 64 checkers the slowest checker's CPU is a constant
  0.65-0.66 s while the mean falls from 0.56 to 0.32 s. Per-file CPU (`TSRS_FILE_TIMES`) shows why:
  `src/vs/platform/agentHost/test/node/mapSessionEvents.test.ts` costs **0.341 s on one checker** (3.3% of the whole
  check; 15,102 nodes, 11 imports, 609 calls: 0.56 ms per `event(...)`/`assert.deepStrictEqual` call, a type-level
  cost that no syntactic measure predicts: the next most expensive file, `agentService.test.ts`, has 11x the nodes and
  11x the calls and costs 0.10 s) and 0.35-0.40 s in a checker that has not seen its neighbourhood. At 64 checkers the
  thief that took it (it sits at position 122 of 154 in its owner's queue, so a thief got it from the back) had done
  0.30 s of its own files first: 0.30 + 0.35 = 0.65. Stealing moves not-yet-started files; it cannot split this one. The
  top 5 files cost 0.73 s together, the 26 files above 20 ms 1.39 s; the rest of the 9,399 files average 1 ms.
- **(b) and (c) together double the mean.** At 64 checkers the checker instructions are +43% (every checker builds the
  shared service graph, as on the Mac) and each instruction costs +36% more CPU (64 threads sharing the caches and
  memory system; at 8 and 16 checkers there is no per-instruction loss on this machine, unlike the loaded Mac). The mean
  checker CPU, 0.32 s, is 2.0x the ideal 10.37 / 64 = 0.16 s, and it is the floor the fix below runs into.
- **(d) serial sections are small.** Checker creation 3 -> 7 ms, file assignment 9 -> 12 ms, global diagnostics
  13 -> 22 ms from 8 to 64 checkers; parse 0.27 s and config 0.025 s at every count; `Total` minus config, parse and
  check is 18 -> 29 ms. Nothing to take.
- Stealing itself: with the static locality assignment (`TSRS_CHECKER_ASSIGNMENT=locality`) the check phase is 0.80 s
  at 32 and 0.69 s at 64 checkers (slowest / mean 1.8 and 2.3): stealing recovers the rest of the imbalance, not the
  heavy file.

## 2. Fixes tried: the owner's visiting order and the thieves' end

With one file as the tail, the lever is when that file starts: if its checker begins with it, it overlaps the other
checkers' work instead of following it. Within a checker, output does not depend on the visiting order
(notes/perf-order-independence.md; gates below), so the order is free in the default (stealing) mode; `--checkerAssignment
go` / `locality` and the oracles keep program order and stay deterministic. Variants (vscode, check s / slowest
checker CPU s / mean checker CPU s / CPU sum s / stolen files / instructions G / peak GiB, medians of 3):

| owner visits its queue ... / thieves take from ... | 16 | 24 | 32 | 48 | 64 |
| --- | --- | --- | --- | --- | --- |
| program order / the back (main) | 0.79 / 0.79 / 0.78 / 12.5 / 561 / 136.5 / 3.58 | 0.78 / 0.78 / 0.57 / 13.7 / 298 / 142.0 / 3.90 | 0.65 / 0.65 / 0.47 / 15.0 / 299 / 148.2 / 4.15 | 0.66 / 0.65 / 0.36 / 17.3 / 505 / 155.4 / 4.60 | 0.66 / 0.66 / 0.32 / 20.2 / 553 / 160.8 / 4.99 |
| whole queue by static weight, heaviest first / the back | 0.81 / 0.81 / 0.81 / 13.0 / 2907 / 136.9 / 3.64 | 0.60 / 0.60 / 0.58 / 13.9 / 1861 / 142.9 / 3.96 | 0.50 / 0.50 / 0.48 / 15.4 / 1117 / 147.8 / 4.17 | 0.64 / 0.64 / 0.38 / 18.1 / 2638 / 156.9 / 4.70 | 0.48 / 0.48 / 0.32 / 20.6 / 2703 / 161.2 / 5.01 |
| whole queue by weight / the front (the heaviest left) | 0.82 / 0.82 / 0.81 / 13.0 / 1828 / 137.2 / 3.64 | 0.62 / 0.61 / 0.60 / 14.4 / 946 / 143.2 / 3.94 | 0.52 / 0.52 / 0.49 / 15.7 / 954 / 147.9 / 4.20 | 0.66 / 0.66 / 0.40 / 19.4 / 2063 / 157.7 / 4.70 | 0.50 / 0.50 / 0.34 / 21.5 / 1127 / 162.1 / 5.03 |
| files above 4x the queue's mean weight first, rest program order / the back | 0.81 / 0.81 / 0.81 / 13.0 / 865 / 137.0 / 3.63 | 0.86 / 0.85 / 0.59 / 14.2 / 470 / 142.2 / 3.92 | 0.50 / 0.49 / 0.46 / 14.9 / 515 / 147.7 / 4.18 | 0.66 / 0.65 / 0.39 / 18.6 / 1554 / 156.8 / 4.65 | 0.49 / 0.48 / 0.32 / 20.3 / 1701 / 160.9 / 5.00 |
| same / the front while the victim's front is heavy, then the back | 0.81 / 0.81 / 0.80 / 12.8 / 847 / 137.0 / 3.62 | 0.85 / 0.84 / 0.60 / 14.3 / 483 / 142.2 / 3.93 | 0.52 / 0.52 / 0.49 / 15.7 / 544 / 147.6 / 4.17 | 0.66 / 0.65 / 0.39 / 18.5 / 1614 / 156.3 / 4.66 | 0.50 / 0.49 / 0.33 / 21.3 / 1272 / 161.6 / 5.02 |
| **files above 1% of a checker's share first, heaviest first, rest program order / the back (landed)** | 0.78 / 0.78 / 0.78 / 12.4 / 632 / 136.6 / 3.60 | 0.57 / 0.57 / 0.56 / 13.5 / 551 / 142.5 / 3.91 | 0.50 / 0.50 / 0.46 / 14.8 / 584 / 147.1 / 4.19 | 0.64 / 0.64 / 0.37 / 17.7 / 1799 / 156.7 / 4.65 | 0.49 / 0.49 / 0.32 / 20.7 / 1146 / 161.7 / 5.03 |

What the rows say:

- Sorting a whole queue by weight (longest-processing-time-first) balances 24-64 but costs at 16 and below: +3.6% CPU
  at 16 for the same instructions, and on the Mac with the static assignment (same file sets, no stealing) each checker
  took 5-8% more CPU for the same instructions (119.4 G both ways) in weight order than in program order: program order
  (dependencies first, directory-local) keeps the checker's caches warm from one file to the next. It also makes thieves
  take 3-5x more files (the lightest). Rejected for small k; the README bench (4 checkers) would lose 2%.
- Thieves taking from the front (the heaviest remaining file) is worse than taking from the back at every count
  (+4-6% CPU: big files moved to a cold checker pay their first touch again). Rejected.
- A per-queue threshold (4x the queue's mean) is a lottery: the queue that holds the biggest test files has a mean so
  high that the 0.34 s file does not count as heavy, so it stays late in program order (24 checkers: 0.86 s, worse than
  main).
- **Landed:** a file is heavy when its static weight (`checked_file_weights`) exceeds 1/100 of an average checker's
  share of the pass (`HEAVY_SHARE_DIVISOR`); each owner visits its heavy files first, heaviest first, then the rest
  in program order; thieves take from the back as before (`heavy_files_first` in checkerpool.rs). On vscode the
  heavy set is 6 files at 4 checkers (weight above 359k: the biggest test files), 28 at 8, 125 at 16 (above 90k, 1.3%
  of the files, 8 per checker), 552 at 32 (above 45k, 17 per checker) and 1,784 at 64 (above 22k, 19% of the files, 28
  per checker); the 0.34 s file (weight 71k) is heavy from 24 checkers on, which is where it matters. Neutral at 8 and
  16 (check 1.42 -> 1.41 s, 0.79 -> 0.78 s; CPU and instructions within 0.5%),
  0.78 -> 0.57 s at 24, 0.65 -> 0.50 s at 32, 0.66 -> 0.49 s at 64. Stolen files 563 -> 632 at 16 and 553 -> 1146 at
  64; the stolen files are light, so instructions move by +0.5% at most.
- What remains at 32-64 is still that one file: at 64 checkers its owner first runs the 3 files of its queue that
  are heavier by weight (`agentHostProtocolClient.test.ts`, 0.054 s, then two of 0.02 s), then the heavy file from
  0.091 s to 0.488 s; the mean checker is done at 0.32 s. At 48 checkers the same queue holds 6 heavier-by-weight
  files (0.27 s) before it, so 48 gains nothing (0.64 s). A cost proxy that ranked this file first would need the
  previous run's measurements (`--checkerCostCache`, opt-in) or a cheaper checker for it; the latter is the next
  lever: it costs 0.34 s alone, which is now the check-phase floor at any count.

Head-to-head walls (no `--extendedDiagnostics`, no stats, medians of 5 interleaved runs; bun `check -p src --no-pretty
--all`, same session):

| | 16 checkers | 24 | 32 | 48 | 64 | bun --threads 32 | bun --threads 64 (its default here) |
| --- | --- | --- | --- | --- | --- | --- | --- |
| main (program order) | 1.09 s / 3.59 GiB | 1.10 / 3.88 | 0.97 / 4.15 | 0.97 / 4.58 | 0.97 / 4.96 | 0.95 s / 2.21 GiB | 0.82 s / 2.85 GiB |
| this change | 1.10 / 3.61 | 0.88 / 3.94 | **0.82 / 4.19** | 0.96 / 4.67 | **0.81 / 5.03** | | |

bun's `--timing` split: 0.25-0.26 s load, 0.51-0.55 s check at 64 threads; tsrs: 0.27 s parse, 0.49-0.50 s check
plus 0.05 s config, global diagnostics and reporting.

## 3. The default checker count

`default_checker_count` was `clamp(available_parallelism / 2, 4, 8)`, with at most one checker per 32 type-checked
files. On the 64-vCPU machine that chose 8: 1.73 s total where 32 checkers give 0.81 s. Measured with this change
(section 2 tables): 8 -> 16 saves 0.63 s, 16 -> 32 another 0.28 s, 32 -> 64 nothing (0.81-0.82 s, the one-file floor)
for +0.84 GiB and +40% summed CPU. The cap is now **32**: `clamp(available_parallelism / 2, 4, 32)`, files floor
unchanged. On 8 vCPUs that is still 4 (the README bench is unchanged); on the 18-core Mac 9 instead of 8; on 64 or more
vCPUs 32.

Memory cost of the new default against the old one (peak RSS; one checker adds the checker's own type graph):

| project | per checker | 8 -> 32 checkers (64-vCPU machine) | 8 -> 9 (18-core Mac) |
| --- | --- | --- | --- |
| vscode | ~37 MiB (3.24 -> 4.15 GiB measured, 8 -> 32) | +0.9 GiB | +0.04 GiB |
| webpack | ~14 MiB (notes/perf-checker-scaling.md: 0.45 -> 0.56 GiB, 8 -> 16) | ~+0.3 GiB | - |
| the 38k-file codebase | ~0.4 GiB (notes/perf-checker-scaling.md: 7.4 -> 9.4 GiB, 8 -> 16) | ~+10 GiB (to ~17 GiB), extrapolated, not measured here | ~+0.4 GiB |

The last row is the reason not to go to 64 by default: the 32-checker gain is real on vscode-sized programs and the
memory is a few GiB; past 32 the check phase does not move and the memory keeps growing. Small programs are unaffected:
the files floor gives xstate (248 checked files) 7 checkers and a 5-file program 4.

bench/compare.py and bench/README.md still describe the default as "half the cores between 4 and 8" (not in this
change's write set); bench/run.py's mode title now derives the count from the machine.

## 4. Result: the branch head on the 64-vCPU machine

The committed tools/perf/probe.sh, branch head (both changes), one run. `default` passes no `--checkers` and chose
32. Medians of 3 (stats runs) and of 5 interleaved (walls):

| | check s | Total s | instructions G | peak GiB | wall s, no stats |
| --- | --- | --- | --- | --- | --- |
| tsrs default (32 checkers; main chose 8: 1.42 / 1.73 / 129.1 / 3.24) | 0.52 | 0.84 | 147.6 | 4.17 | **0.82** |
| tsrs --checkers 4 | 2.76 | 3.07 | 123.4 | 3.03 | |
| tsrs --checkers 16 | 0.80 | 1.12 | 136.5 | 3.62 | 1.10 |
| tsrs --checkers 64 | 0.50 | 0.82 | 161.9 | 5.02 | 0.81 |
| bun check default (64 threads) | 0.52 (its "checked in") | | | 2.85 | 0.83 |

vscode without flags: 1.73 s -> 0.82 s on this machine, at the same output, for +0.9 GiB; bun check at its default is
0.83 s. The remaining difference to bun is memory (4.2 vs 2.9 GiB), not time.

## Gates

- Output byte-identical to the base binary (origin/main fd2ffc1) for vscode, webpack and xstate-main at 1, 4 and 16
  checkers (`--pretty false`, full output), and vscode at 64 and at the new default (9 on the Mac).
- `cargo test --release -p tsrs_compiler` green, including a new test that `heavy_files_first` moves only the heavy
  files and keeps the others' order.
- `tools/lint/ratchet.py` ok (11 findings, none new); `tools/lint/source.py` ok.
- `--checkerAssignment go` and `locality` do not reorder (the reorder runs only when stealing does); the incremental
  pass, declaration diagnostics and emit never steal, so they are unchanged.

## Reproduce

```sh
# the 64-thread curve, per-checker CPU and stolen files, instructions, peak RSS, default count, walls against bun
depot ci run --workflow .depot/workflows/perf-probe.yml      # tools/perf/probe.sh; artifact perf-probe
# locally: per-checker report and per-file times
TSRS_ASSIGNMENT_STATS=times tsrs -p src --noEmit --incremental false --extendedDiagnostics --pretty false --checkers 64
TSRS_FILE_TIMES=ft64.tsv tsrs -p src --noEmit --incremental false --checkers 64   # checker, wall, cpu, nodes, text, imports, file
```
