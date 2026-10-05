# perf-build-level: speed from how the binary is built and laid out (Linux x86-64)

Work in progress (branch `perf/build-level`). Every experiment is measured on a PGO + fat-LTO build, the way
`.github/workflows/release.yml` builds the shipped binary.

## Host

Cloud sandbox microVM, Intel Xeon Platinum 8375C @ 2.90 GHz (Ice Lake), 18 vCPUs, 47 GiB, no swap, kernel 7.2.9,
THP `enabled` = `madvise`. rustc 1.99.0 (LLVM 23.1.1).

## Reproducing the release build

origin/main 68636c6, `cargo build --profile dist` with the workflow's commands, clean target directories:

| step | time |
| --- | --- |
| instrumented build (tsrs, tsrs-test, tsrs-fourslash) | 415 s |
| training (`.github/scripts/pgo-train.sh`, bench projects already cloned) | 39 s |
| `llvm-profdata merge` | < 1 s |
| final build (tsrs) | 246 s |

Results of the experiments follow as they finish.

## Raw results (interim)

Big corpus, PGO-only vs PGO+BOLT (instrumentation mode) vs PGO+BOLT with `-hugify`. Two sessions (5 and 7 interleaved rounds); diagnostics identical (md5 of the 40,542 error lines) in every run.

## bolt
| checkers | bin | n | wall s (range) | cycles G | instr G | max RSS GiB | br-miss M | iTLB miss M | L1i miss M | iTLB walk cyc % | icache stall cyc % | md5s | size MB |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | bolt | 5 | 42.43 (41.75-44.23) | 160.0 | 216.2 | 4.49 | 736 | 38.5 | 4687 | 1.54 | 4.00 | fac13d | 34.1 |
| 1 | pgo | 5 | 44.09 (40.21-45.25) | 164.9 | 216.6 | 4.49 | 748 | 30.5 | 5869 | 1.44 | 5.03 | fac13d | 101.7 |
| 1 | relocs | 5 | 44.43 (41.90-44.74) | 167.1 | 216.7 | 4.48 | 745 | 31.0 | 5880 | 1.40 | 4.98 | fac13d | 196.3 |
| 4 | bolt | 5 | 15.01 (14.58-15.77) | 206.7 | 296.9 | 5.88 | 918 | 44.7 | 5942 | 1.41 | 3.87 | fac13d | 34.1 |
| 4 | pgo | 5 | 15.77 (15.07-16.35) | 215.7 | 297.9 | 5.87 | 929 | 39.5 | 7475 | 1.33 | 5.07 | fac13d | 101.7 |
| 4 | relocs | 5 | 15.47 (15.09-15.84) | 213.6 | 297.9 | 5.87 | 926 | 36.0 | 7463 | 1.24 | 4.98 | fac13d | 196.3 |
| 8 | bolt | 5 | 11.20 (10.85-11.57) | 276.1 | 399.0 | 7.57 | 1159 | 77.2 | 7543 | 1.76 | 3.44 | fac13d | 34.1 |
| 8 | pgo | 5 | 11.24 (10.97-11.51) | 274.2 | 398.3 | 7.53 | 1159 | 43.1 | 9364 | 1.18 | 4.31 | fac13d | 101.7 |
| 8 | relocs | 5 | 11.31 (10.95-11.73) | 276.5 | 399.8 | 7.57 | 1162 | 43.7 | 9408 | 1.14 | 4.25 | fac13d | 196.3 |

paired vs pgo (median of per-round ratios, range)
| checkers | bin | wall | cycles | instructions | max RSS |
|---|---|---|---|---|---|
| 1 | bolt | -2.4% (-6.2..+7.0) | -2.2% (-6.4..+5.5) | -0.21% | -0.0% (-0.2..+0.5) |
| 1 | relocs | -1.5% (-5.0..+8.7) | -1.0% (-4.0..+7.2) | +0.00% | -0.2% (-0.4..+0.4) |
| 4 | bolt | -5.1% (-7.9..-0.4) | -4.4% (-6.1..+0.2) | -0.31% | +0.1% (-0.8..+0.4) |
| 4 | relocs | -0.6% (-6.7..+2.7) | -0.6% (-4.2..+3.3) | +0.07% | -0.1% (-0.5..+0.6) |
| 8 | bolt | -1.0% (-1.2..+4.3) | +0.7% (-1.0..+3.0) | +0.06% | +0.6% (-0.3..+1.3) |
| 8 | relocs | +1.9% (-0.3..+3.0) | +1.6% (-0.4..+4.1) | +0.33% | +0.6% (-0.2..+0.8) |

## bolt2
| checkers | bin | n | wall s (range) | cycles G | instr G | max RSS GiB | br-miss M | iTLB miss M | L1i miss M | iTLB walk cyc % | icache stall cyc % | md5s | size MB |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 1 | bolt | 7 | 42.43 (38.00-43.63) | 159.4 | 216.3 | 4.49 | 736 | 38.4 | 4672 | 1.53 | 3.95 | fac13d | 34.1 |
| 1 | bolt-huge | 7 | 40.23 (38.96-43.56) | 152.4 | 216.2 | 4.48 | 737 | 38.2 | 4676 | 1.38 | 3.81 | fac13d | 35.4 |
| 1 | pgo | 7 | 42.26 (41.27-45.78) | 160.3 | 216.6 | 4.49 | 745 | 31.5 | 5859 | 1.47 | 5.07 | fac13d | 101.7 |
| 4 | bolt | 7 | 14.47 (13.70-15.87) | 198.5 | 297.1 | 5.88 | 918 | 44.2 | 5914 | 1.41 | 3.72 | fac13d | 34.1 |
| 4 | bolt-huge | 7 | 14.25 (13.73-16.43) | 197.7 | 296.7 | 5.88 | 919 | 45.4 | 5926 | 1.30 | 3.63 | fac13d | 35.4 |
| 4 | pgo | 7 | 14.67 (14.20-16.42) | 201.9 | 297.3 | 5.88 | 927 | 36.2 | 7422 | 1.33 | 4.71 | fac13d | 101.7 |
| 8 | bolt | 7 | 11.35 (10.35-11.64) | 269.7 | 398.8 | 7.56 | 1155 | 52.9 | 7511 | 1.27 | 3.45 | fac13d | 34.1 |
| 8 | bolt-huge | 7 | 11.22 (9.94-12.82) | 272.8 | 398.2 | 7.57 | 1152 | 53.6 | 7498 | 1.14 | 3.31 | fac13d | 35.4 |
| 8 | pgo | 7 | 11.09 (10.95-11.70) | 271.3 | 399.1 | 7.56 | 1161 | 43.5 | 9356 | 1.20 | 4.35 | fac13d | 101.7 |

paired vs pgo (median of per-round ratios, range)
| checkers | bin | wall | cycles | instructions | max RSS |
|---|---|---|---|---|---|
| 1 | bolt | -3.1% (-7.9..+0.4) | -3.1% (-6.5..-0.3) | -0.19% | -0.0% (-0.4..+0.4) |
| 1 | bolt-huge | -4.9% (-9.5..-2.4) | -4.7% (-7.6..-2.8) | -0.20% | -0.1% (-0.5..+0.5) |
| 4 | bolt | -3.0% (-10.4..+0.4) | -2.5% (-10.0..-0.4) | -0.24% | +0.0% (-0.3..+0.4) |
| 4 | bolt-huge | -2.4% (-5.1..+0.4) | -3.4% (-5.4..+0.6) | -0.16% | -0.0% (-0.4..+0.3) |
| 8 | bolt | -2.2% (-5.6..+6.1) | -2.2% (-5.5..+5.8) | -0.37% | +0.1% (-1.0..+0.9) |
| 8 | bolt-huge | -2.7% (-9.2..+16.9) | -2.4% (-6.5..+8.4) | -0.25% | -0.1% (-0.8..+0.8) |
