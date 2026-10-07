# perf-serial-assign-overlap: assign files while the checkers are created

**Reverted in https://github.com/maschwenk/tsrs/pull/190 (2026-10-07):** the change bought 3-4 ms at 32 checkers on the 64-vCPU
runner by adding a thread or an overlap around checker creation, which is below the bar in AGENTS.md ("A performance
change must pay for its complexity"). The measurements below stand; do not re-land the mechanism for this gain.

Goal: shorten the serial step before the check pass of a CLI run on the 64-vCPU runner (`depot-ubuntu-24.04-64`),
from the "what remains" list of notes/perf-front-end-fixed-costs.md. Base: main 025b496, vscode `-p src` (10,427
files), the default 32 checkers.

## What "Diagnostics: global (first)" was

The row is 15 ms on the runner, but the sweep itself (`get_global_diagnostics` on each checker, which runs no
deferred diagnostics yet) is under 1 ms. The rest is `checkerPool::create_checkers`, which the first sweep triggers:
"Checkers: create" (4-5 ms, 32 checkers built on the checker threads, each merging the globals) followed by
"Checkers: assign files" (10 ms, `compute_associations` on the program thread). A frame-pointer profile of the
program thread on the runner (run dkp6ztszxg) shows the same split.

The assignment reads only the loaded program (file sizes, imports, resolutions; the import graph on the worker
pool) and no checker. The leaf classification (`fileregions::prepare`) already runs beside both for the same reason.

## Change

`create_checkers` creates the checkers from a scoped helper thread (it only waits on the checker threads'
broadcast) while the program thread computes the assignment. The two rows are recorded after both ended so they
keep their order. Single-threaded runs keep the old sequence. The assignment's code (`compute_associations`, the
weights, stealing, `plan_splits`) is unchanged.

## Numbers (runner, run dkp6ztszxg)

Base and new built in the same directory with `--profile dist`, 11 interleaved runs each.

| row | base median (min-max) | new median (min-max) |
| --- | ---: | ---: |
| Checkers: create | 4 ms (4-6) | 4 ms (4-6) |
| Checkers: assign files | 10 ms (10-11) | 10 ms (10-10) |
| Diagnostics: global (first) | 15 ms (14-16) | 11 ms (11-11) |
| Check time | 456 ms (438-470) | 463 ms (437-470) |
| wall minus Total time | 17.1 ms | 17.1 ms |
| peak RSS | 2805 MiB | 2807 MiB |

Single-threaded user instructions (`bench/count.py`, the code path does not change): 102.558 G before and after.
`perf stat` at 32 checkers: 131.3 G vs 131.7 G user instructions (stealing makes this vary run to run), page faults
34.4k vs 34.6k. Output of `--pretty false`, `--listFiles` and `--explainFiles` identical on vscode on the runner and
on the ten older bench projects locally.

## Interaction with the assignment work

If the assignment becomes more parallel (it already uses the worker pool for the import graph), it competes with
checker creation for cores only on machines with fewer cores than checkers plus worker threads; on the 64-vCPU
runner the 32 checker threads and the worker pool fit.
