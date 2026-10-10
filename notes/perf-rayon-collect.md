# Rayon construction of processed-file maps

Base: `15ecffbe9daabfa0078314c32bbfb01939452fa2`. This experiment is independent of the Rayon checker-assignment
experiment. It targets successful, non-incremental `--noEmit` checks; diagnostic processing is unchanged.

## Prior evidence

`notes/perf-front-end-fixed-costs.md` lists moving the collect walk's per-file maps after the walk as untried.
`notes/perf-serial-collect.md` reduced the whole phase from 10 to 8 ms on vscode at 32 checkers on a 64-vCPU
runner. That is historical evidence of a small ceiling, not a prediction for today's default mode.

## Experiment

The traversal still decides file order, package deduplication, redirects and include reasons. In threaded mode,
four maps buffer their owned entries: files by path, resolved modules, type-reference resolutions and source-file
metadata. After the traversal and the library-resolution overrides, four jobs on the existing worker pool build
the maps. Each map consumes its entries in the original insertion order, retaining last-write-wins behavior.

Single-threaded mode inserts directly into maps. No new pool, helper thread, arena allocation or unsafe code is
introduced. The parallel path pays for temporary vectors and an additional pass over the entries; peak RSS is
part of the acceptance decision. `Program: collect maps` measures final construction and is nested in the existing
`Program: collect files` phase.

## Validation and decision

The focused unit case covers non-adjacent overwrites and empty maps. Full validation compares conformance,
regressions, project outputs and poisoned-arena runs against the same base. Happy-path measurements start with
xstate-main, mui-docs and formbricks-web, using the benchmark's exact flags.

Status: experimental; measurements pending. Before retaining the implementation, record base/new single-threaded
instructions, peak RSS and default-mode wall time. The repository's landing thresholds apply: 1% fewer
single-threaded instructions, 2% headline wall time across two same-hardware publishes, or 5% lower peak RSS at the
default checker count, with enough gain to pay for the complexity. If it falls short, keep this note and remove
the implementation.
