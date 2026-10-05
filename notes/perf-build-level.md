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
