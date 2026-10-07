# perf-heavy-first-threshold: the heavy-first rule between 10 and 20 checkers

notes/perf-checker-64.md landed `heavy_files_first`: each checker starts the type-check pass with the files of its
queue whose static weight exceeds 1/`HEAVY_SHARE_DIVISOR` of an average checker's share, heaviest first, and keeps
program order for the rest. The divisor was 100, measured at 8, 16 and 24-64 checkers on the 64-vCPU runner ("neutral
at 8 and 16"). This note measures the counts in between on an 18-core Mac, finds that the check phase is not monotonic
in the checker count because the same file is the tail at 13, 14, 17 and 18 checkers, and moves the divisor to 200.
Base: main dc8bfa8 (PR 120). Diagnostics byte-identical in every run (vscode 371, webpack 840, xstate-main 0 errors).

## 1. The curve on the Mac is a lottery

Apple M5 Max (6 P + 12 E cores, 18 threads, no transparent huge pages), vscode (`-p src --noEmit --incremental false
--extendedDiagnostics --pretty false --checkers k`), `cargo build --release` of main, 3 reps, medians of `Check time`.
The machine was shared with another agent's builds, so single runs move a few percent; the pattern did not.

| checkers | 9 | 10 | 12 | 13 | 14 | 15 | 16 | 17 | 18 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| check s, main | 1.00 | 1.01 | 0.83 | 0.97 | 0.95 | 0.81 | 0.68 | 0.93 | 0.87 |

`TSRS_ASSIGNMENT_STATS=1` at 17 checkers: sixteen checkers finish at 0.63 s, one at 0.92 s. `TSRS_FILE_TIMES` for that
checker: 521 files, file wall 0.917 s, thread CPU 0.876 s (busy the whole time, not descheduled), and its longest file
is `src/vs/platform/agentHost/test/node/mapSessionEvents.test.ts` at 0.302 s, the file perf-checker-64.md found at
24-64 checkers (0.34 s on one checker; 15,102 nodes, 609 calls of a 155-member union inference). The tail is exactly
one such file started at about 0.62 s: stealing moves unstarted files, it cannot split one. At 16 checkers the file's
owner holds 184 files and reaches it early; at 17 it is reached last. Same at 13 and 14 against 12 and 15.

## 2. Why the 1/100 rule misses it below 24 checkers

The static weight of a checked file is `4 * (nodes + text / 100)` plus `imports * (total base weight / total imports)`
(`checked_file_weights`); vscode's total is 143.7 M over 9,399 checked files, and this file weighs 71.4 k, rank 199.
A file is heavy when its weight exceeds `total / (checkers * divisor)`:

| divisor | threshold at 4 / 8 / 16 / 17 / 32 checkers (k) | heavy files at 4 / 8 / 16 / 17 / 32 | this file heavy from |
| --- | --- | --- | --- | --- |
| 100 (was) | 359 / 180 / 90 / 85 / 45 | 6 / 28 / 125 / 138 / 552 | 21 checkers |
| 200 (now) | 180 / 90 / 45 / 42 / 22 | 28 / 125 / 552 / 614 / 1,784 | 11 checkers |
| 300 | 120 / 60 / 30 / 28 / 15 | 65 / 304 / 1,159 / 1,298 / 2,978 | 7 checkers |
| 1000 | 36 / 18 / 9 / 8 / 4 | 858 / 2,427 / 4,607 / 4,792 / 6,670 | 3 checkers |

So at 13-18 checkers the file is "light" under 1/100 and sits wherever program order put it in its owner's queue
(position 122 of 154 at 64 checkers, per perf-checker-64.md); thieves take from the back, so a late position is a late
start. Nothing syntactic predicts its cost (the next most expensive file has 11x the nodes and costs a third), so the
threshold has to be low enough to catch a weight in the top 2% of files at the checker counts where a 0.3 s file is a
large share of the pass.

## 3. The divisor sweep on the Mac

`TSRS_HEAVY_SHARE_DIVISOR` (new; the constant when unset), same binary, medians of 3 interleaved reps per divisor;
check seconds and the summed user CPU seconds (`/usr/bin/time -l`), which is where lost locality would show. Divisors
200 and 500 were a second sweep an hour later, so compare them with their own 100-column neighbours loosely.

| checkers | 100 (was) | 200 (now) | 300 | 500 | 1000 |
| --- | --- | --- | --- | --- | --- |
| 4 | 1.918 (8.93) | 1.893 (8.84) | 1.924 (9.00) | 1.941 (8.98) | 1.933 (9.03) |
| 8 | | 1.096 (9.95) | | 1.112 (10.13) | |
| 9 (the default here) | 1.018 (10.31) | | 1.011 (10.32) | | 1.043 (10.56) |
| 12 | 0.833 (11.15) | | 0.823 (11.03) | | 0.852 (11.30) |
| 13 | **0.966** (11.11) | **0.768** (11.07) | 0.775 (11.21) | 0.777 (11.23) | 0.792 (11.38) |
| 14 | **0.935** (11.08) | **0.707** (11.03) | 0.734 (11.31) | 0.730 (11.35) | 0.747 (11.42) |
| 16 | 0.668 (11.56) | | 0.682 (11.71) | | 0.678 (11.79) |
| 17 | **0.917** (11.63) | **0.726** (11.60) | 0.734 (11.90) | 0.724 (11.76) | 0.737 (11.93) |
| 18 | **0.857** (11.70) | **0.762** (11.65) | 0.760 (11.92) | 0.781 (11.74) | 0.778 (12.04) |

- 200 removes the four tails (-21%, -24%, -21%, -11%) and costs no CPU: its user seconds equal the 100 column's within
  the run-to-run spread. Peak memory is unchanged (within 15 MiB at every count).
- 300 removes them too but costs 1-3% CPU at 13-18 (more files leave program order); 500 and 1000 cost the same or
  more and gain nothing: once the one file starts first, the rest of the imbalance is the stealing floor.
- The rebuilt binary with the constant at 200 reproduces the sweep: 0.677 s at 16, 0.720 at 17, 0.791 at 13.

## 4. Linux, 64 vCPUs (Depot `depot-ubuntu-24.04-64`, perf-probe.yml, `cargo build --release`)

`tools/perf/probe.sh` variants (not committed) ran vscode at the counts below, 3 interleaved reps per divisor, each run
under `perf stat -e instructions:u` with `TSRS_ASSIGNMENT_STATS=1`. Medians: check s / user CPU s / instructions G /
slowest checker over mean checker. Probe runs 2g1j4vv090 (100, 300, 500) and kr3gzc4f07 (100, 200).

| checkers | 100 (was) | 200 (now) | 300 | 500 |
| --- | --- | --- | --- | --- |
| 4 | 2.665 / 14.7 / 131.8 / 1.00 and 2.643 / 14.4 / 131.9 / 1.00 | 2.668 / 14.4 / 131.7 / 1.00 | 2.707 / 14.9 / 131.8 / 1.00 | 2.693 / 14.7 / 131.7 / 1.00 |
| 8 | 1.425 / 16.2 / 145.9 and 1.379 / 15.9 / 146.2 | 1.403 / 16.1 / 146.1 / 1.00 | 1.415 / 16.4 / 145.9 | 1.424 / 16.6 / 146.0 |
| 12 | 1.010 / 18.3 / 158.8 and 0.981 / 17.9 / 158.8 | 0.989 / 18.0 / 159.0 / 1.01 | 1.015 / 18.6 / 158.9 | 1.005 / 18.5 / 158.9 |
| 16 | 0.768 / 20.2 / 171.0 and 0.762 / 19.0 / 171.1 | 0.763 / 19.9 / 171.4 / 1.00 | 0.789 / 20.3 / 171.9 | 0.789 / 20.4 / 171.5 |
| 20 | 0.650 / 21.9 / 182.7 and 0.637 / 21.4 / 182.7 | 0.639 / 21.4 / 182.5 / 1.01 | 0.668 / 22.5 / 182.8 | 0.656 / 21.9 / 182.5 |
| 24 | 0.570 / 24.1 / 193.7 and 0.547 / 23.1 / 193.8 | 0.555 / 23.3 / 193.7 / 1.00 | 0.583 / 24.3 / 194.1 | 0.618 / 24.3 / 194.1 |
| 32 (the default there) | 0.504 / 28.6 / 218.4 / 1.08 and 0.470 / 26.6 / 218.2 / 1.08 | 0.458 / 26.9 / 218.4 / 1.07 | 0.498 / 28.6 / 218.1 / 1.06 | 0.514 / 29.5 / 217.5 / 1.08 |

At these counts main has no tail (slowest / mean 1.00-1.08; the two 100 columns are the two probes and show the
runner's own spread: 0.03 s and 5% CPU), and 200 is inside that spread everywhere: instructions identical to 0.2%,
check time within 0.02 s, peak RSS within 0.01 GiB. 300 and 500 are 0.01-0.05 s slower at 16-24. The counts that
matter are the ones the first two probes did not run; see section 5.

## 5. Linux at the tail-prone counts

Probe 7l4nj0j464, same setup as section 4, the counts the first two probes skipped. Medians of 3: check s / user CPU s /
instructions G / slowest checker over mean checker.

| checkers | 100 (was) | 200 (now) | check |
| --- | --- | --- | --- |
| 10 | 1.229 / 17.8 / 152.4 / 1.00 | 1.268 / 18.5 / 152.4 / 1.01 | +0.04 |
| 11 | 1.186 / 18.7 / 155.4 / 1.01 | 1.152 / 19.0 / 155.7 / 1.00 | -0.03 |
| 13 | 1.187 / 19.7 / 162.2 / **1.22** | 1.002 / 19.7 / 162.8 / 1.00 | **-16%** |
| 14 | 1.135 / 20.7 / 164.9 / **1.22** | 0.955 / 20.6 / 165.1 / 1.01 | **-16%** |
| 15 | 0.911 / 21.0 / 168.4 / 1.03 | 0.889 / 21.2 / 168.6 / 1.01 | -0.02 |
| 17 | 1.081 / 22.1 / 174.5 / **1.37** | 0.824 / 22.3 / 175.1 / 1.01 | **-24%** |
| 18 | 0.924 / 22.1 / 176.7 / **1.22** | 0.805 / 22.1 / 176.9 / 1.04 | **-13%** |
| 19 | 1.002 / 23.1 / 180.1 / **1.39** | 0.767 / 23.1 / 180.1 / 1.01 | **-23%** |
| 22 | 0.658 / 24.5 / 187.7 / 1.01 | 0.675 / 24.6 / 187.7 / 1.01 | +0.02 |
| 28 | 0.606 / 28.1 / 206.7 / 1.09 | 0.607 / 29.0 / 207.5 / 1.09 | 0 |

The same counts as on the Mac (13, 14, 17, 18, plus 19) have the tail on Linux: the slowest checker runs 22-39% longer
than the mean, and on main 13 or 17 checkers were no faster than 11. The file-to-checker assignment is deterministic and
does not depend on the machine, so this is where the file sits in its owner's queue at each count, not scheduling. With
1/200 every count is balanced (1.00-1.04 apart from the 1.09 at 28 that both columns share: at 28 the file is heavy under
both rules and the floor is the file itself), instructions are identical to 0.4%, user CPU within the spread, peak RSS
unchanged, 371 errors in every run.

## 6. Gates

- Diagnostics: full `--pretty false` output identical between the main binary and this one at 200 on vscode (4, 13, 17
  checkers), webpack (4, 16) and xstate-main (4, 16); Linux runs report 371 errors in every cell.
- `cargo check --workspace` clean; `cargo test --release -p tsrs_compiler checkerpool` (3 tests, including the
  `heavy_files_first` ordering test); `tools/lint/ratchet.py` ok, none new; `tools/lint/source.py` ok (the override is
  a `OnceLock`, no atomics).
- The 8-vCPU README bench runs 4 checkers, where the heavy set grows from 6 to 28 files: check 2.64-2.67 s either way
  on the 64-vCPU runner, instructions identical. Single-threaded runs (the regression counter) have no pass order.

## 7. Not done

- A cost proxy that ranks this kind of file first at 4-8 checkers would need the previous run's measurements
  (`--checkerCostCache`) or a type-level estimate; at 4 checkers the file is 13% of one checker's share and the Mac
  shows no tail there, so the divisor is not pushed further (300+ costs CPU).
- The remaining lever for this file is the one perf-checker-64.md names: a cheaper checker for a 155-member union
  inference, which is the check-phase floor (0.30-0.34 s) at any count above about 30.
