# mem-finish-checkers: free checker regions as their queues finish

Question (2026-10-11): `--maxMemory` retires a checker between files while work remains, but a checker whose queue is
empty stays in the pool until process exit. Peak memory is near the end of the type-check pass. Can a finished worker
preserve its global diagnostics and counters, then drop its whole checker region while the slower workers continue?

Answer: exact, but no measurable peak reduction. Work stealing already finishes the checker queues within about
70 ms of one another on a large private monorepo, and the process has reached its maximum RSS before that tail. The
prototype changed mean peak RSS by -0.03% at `--maxMemory 4G`, far below the 5% memory bar, and was reverted.

## Prototype

`SlotChecker::checker` became an `Option<Box<Checker>>`. At the end of a recycling worker's queue, the prototype:

1. produced and preserved the checker's deferred global diagnostics;
2. added its Symbols, Types, Instantiations and lazy-member counters to `RetiredCheckers` without incrementing the
   replacement-retirement count;
3. dropped the checker and then its region; and
4. left an empty pool slot, which the post-pass global-diagnostic and extended-diagnostic walks skipped.

Unlike an ordinary mid-queue retirement, finalization did not construct a fresh checker. This was limited to
`ProgramOptions::checker_recycling`, whose CLI no-emit mode guarantees that no later pass asks for the checker that
checked a file.

Correctness checks:

- `cargo test -p tsrs_cli --test recycle_checkers`: passed, including poison-mode retirement and extended counters;
- sorted plain diagnostics on the measured workload: byte-identical SHA-256 output.

## Measurements

Release build with symbols and debug information, macOS. `hyperfine 2.0.0`, one warmup and five measured runs. The
workload was a large private monorepo with the same corrected no-emit flags as the `--maxMemory` investigation.

| `--maxMemory 4G` | baseline | finalize finished checkers | change |
| --- | ---: | ---: | ---: |
| wall | 6.562 +/- 0.069 s | 6.558 +/- 0.086 s | -0.1% (noise) |
| peak RSS | 4.0372 GiB | 4.0360 GiB | -0.03% |

The memory-triggered retirement count makes multi-checker hardware instructions noisy at 4G. To isolate the code
change, a second five-run measurement used static checker assignment and `--maxMemory 100G`: recycling and final
checker disposal stayed enabled, but no whole checker was retired for memory pressure.

| stable-work variant | baseline | finalize finished checkers | change |
| --- | ---: | ---: | ---: |
| instructions retired | 295.036 +/- 0.538 B | 294.823 +/- 0.483 B | -0.07% (noise) |
| peak RSS | 5.3904 GiB | 5.3843 GiB | -0.11% |
| wall | 6.955 s | 6.981 s | +0.4% (noise) |

`TSRS_ASSIGNMENT_STATS=times` put the first and last checker completion at 2.18 and 2.25 seconds: a 70 ms spread.
The existing work stealing leaves too little tail for finished-checker disposal to lower the historical maximum.
The code is not kept; reconsider only for a workload with measured checker-tail imbalance large enough that one or
more checker regions remain live across the RSS peak.
