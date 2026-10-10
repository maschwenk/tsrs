# Rayon preparation of checker assignment

Base: `15ecffbe9daabfa0078314c32bbfb01939452fa2`. This experiment is independent of the Rayon processed-file-map
experiment. It targets successful, non-incremental `--noEmit` checks; diagnostic processing is unchanged.

## Prior evidence

`notes/perf-front-end-fixed-costs.md` measured assignment at about 10 ms on vscode at 32 checkers on a 64-vCPU
runner. Import-target extraction and path sorting already run on Rayon. The checker-creation overlap in
`notes/perf-serial-assign-overlap.md` was reverted for insufficient benefit; this experiment does not restore it.
The current base defaults to up to 16 checkers on a 16-core machine, so historical 8-checker results are not the
default-mode baseline for this experiment.

## Experiment

Directory-subtree weights are accumulated in chunks of 512 checked files, with a private map per chunk. Rayon
reduces those maps by integer addition; only keyed lookups consume the result. Inputs of at most one chunk and
single-threaded mode retain serial accumulation.

After the existing deterministic group formation, files are collected by group in path order. Each group builds
its adjacency list independently on the existing worker pool. Edge order and multiplicity are preserved. The
undirected file graph, group IDs, FENNEL placement and affinity refinement are unchanged. Single-threaded mode
retains the original adjacency loop.

`Checkers: subtree weights` and `Checkers: group adjacency` measure the two substeps inside `Checkers: assign
files`. Partial maps, their reduction, and temporary group-member lists are costs to include in the comparison.

## Validation and decision

Focused unit cases compare subtree totals across several chunks and pin adjacency order with duplicate edges,
unchecked files, intra-group edges, interleaved members and empty groups. Full validation compares assignments,
conformance, regressions, project outputs and poisoned-arena runs against the same base. Happy-path measurements
start with xstate-main, mui-docs and formbricks-web, using the benchmark's exact flags.

Status: experimental; measurements pending. Before retaining the implementation, record base/new single-threaded
instructions, peak RSS and default-mode wall time. The repository's landing thresholds apply: 1% fewer
single-threaded instructions, 2% headline wall time across two same-hardware publishes, or 5% lower peak RSS at the
default checker count, with enough gain to pay for the complexity. If it falls short, keep this note and remove
the implementation.
