# perf-serial-program-diagnostics: program diagnostics beside checker creation

**Reverted in https://github.com/maschwenk/tsrs/pull/190 (2026-10-07):** the change bought 3-4 ms at 32 checkers on the 64-vCPU
runner by adding a thread or an overlap around checker creation, which is below the bar in AGENTS.md ("A performance
change must pay for its complexity"). The measurements below stand; do not re-land the mechanism for this gain.

Goal: shorten the serial steps before the check pass of a CLI run on the 64-vCPU runner (`depot-ubuntu-24.04-64`),
from the "what remains" list of notes/perf-front-end-fixed-costs.md. Base: main 025b496, vscode `-p src` (10,427
files), the default 32 checkers.

## Change

`get_diagnostics_of_any_program` ran "Diagnostics: program" (2 ms on the runner: the processing diagnostics and a walk
over the ~110k module and type reference resolutions in `includeProcessor::get_diagnostics`) and then "Diagnostics:
global (first)" (15 ms: checker creation and file assignment, notes/perf-serial-assign-overlap.md). The program
diagnostics read only the loaded program, so a scoped helper thread now computes them while the program thread binds
(nothing to do in the CLI) and creates the checkers. They are still added to the result before the global ones, as in
Go. `--listFilesOnly` (which creates no checkers) and single-threaded runs keep the old sequence.

The two rows are recorded after the join, so "Diagnostics: program" now prints after the "Checkers: create" and
"Checkers: assign files" rows that the first global sweep records.

## Numbers (runner, run jc50scgs6q)

Base and new built in the same directory with `--profile dist`, 11 interleaved runs each. The serial time is
"Diagnostics: program" plus "Diagnostics: global (first)" before, and "Diagnostics: global (first)" after (the
program diagnostics finish well within it).

| row | base median (min-max) | new median (min-max) |
| --- | ---: | ---: |
| Diagnostics: program | 2 ms (2-2) | 2 ms (2-2), on the helper |
| Diagnostics: global (first) | 16 ms (15-19) | 15 ms (15-17) |
| serial: program + global (first) / global (first) | 18 ms (17-21) | 15 ms (15-17) |
| Check time | 454 ms (429-476) | 456 ms (448-468) |
| wall minus Total time | 16.9 ms | 16.7 ms |
| peak RSS | 2809 MiB | 2808 MiB |

Single-threaded user instructions (`bench/count.py`, the code path does not change): 102.558 G vs 102.557 G.
`perf stat` at 32 checkers: 131.5 G vs 130.9 G user instructions (stealing makes this vary run to run), page faults
34.5k vs 34.2k. Output of `--pretty false`, `--listFiles` and `--explainFiles` identical on vscode on the runner and on
the ten older bench projects locally.

With notes/perf-serial-assign-overlap.md the first sweep is 11 ms, still far longer than the program diagnostics.
