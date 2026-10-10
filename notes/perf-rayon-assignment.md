# Rayon preparation of checker assignment

Decision: keep the measurement, remove the implementation. The screening run did not demonstrate a gain worth
the partial maps, reduction and group-member staging.

Base: `15ecffbe9daabfa0078314c32bbfb01939452fa2`. The independent prototype is retained in
[commit bb407521](https://github.com/maschwenk/tsrs/commit/bb407521). It targets successful, non-incremental
`--noEmit` checks; diagnostic processing is unchanged.

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

Local validation of the prototype (macOS arm64, release): both focused unit cases passed; all 29 regression cases
passed; 26 package-deduplication and type-reference conformance cases matched the base for errors, `.types` and
`.symbols` in parallel canonical mode. On pinned xstate-main, output and initial assignment/import-edge dumps
matched at 4, 16 and 32 checkers, and `--listFiles` / `--explainFiles` output matched at the default count.
`cargo check --workspace`, `tools/lint/ratchet.py` and `tools/lint/source.py` passed. Local wall time is not used
for the performance decision because other compiler jobs were running on the machine.

## Linux measurements

[PR verification report](https://github.com/maschwenk/tsrs/pull/293#issuecomment-6099610922), Depot run
`019znptrpf`: base `15ecffbe9daa`, tested PR merge `d537ab9009be`, release builds on a 32-vCPU AMD EPYC 9R45
machine with 126 GB RAM. The two PRs used the same base and were measured independently. Three interleaved runs
per binary at 1, 4, 16 and 32 checkers, plus a single-threaded instruction count and a poisoned-arena run per
project. All 102 comparison cells had identical diagnostics and exit codes; the full CI check-and-test gate passed.

The successful-check projects at **16 explicit checkers** (not a PGO headline/default-mode publish):

| project | wall base -> new | wall delta | peak RSS base -> new | RSS delta |
| --- | ---: | ---: | ---: | ---: |
| xstate-main | 0.12 -> 0.12 s | +0.8% | 372 -> 376 MiB | +1.1% |
| mui-docs | 1.00 -> 1.01 s | +0.9% | 1.62 -> 1.61 GiB | -0.3% |
| formbricks-web | 0.72 -> 0.74 s | +3.2% | 2.00 -> 2.00 GiB | -0.1% |

| project | single-threaded instructions base -> new | delta | single-threaded peak RSS base -> new |
| --- | ---: | ---: | ---: |
| xstate-main | 6.799 -> 6.799 G | +0.000% | 183 -> 183 MiB |
| mui-docs | 46.341 -> 46.341 G | +0.000% | 740 -> 740 MiB |
| formbricks-web | 46.722 -> 46.722 G | +0.000% | 1.07 -> 1.07 GiB |

Values use the report's display rounding; its deltas use the underlying measurements. All 17 projects rounded
to +0.000% instruction change. The largest absolute peak-RSS change among the timed cells was a 2.5% increase
(Compiler, 1 checker). Mui-docs was 6.9% faster at 32 checkers, but that isolated result does not establish a
headline/default-mode gain; its 16-checker row was 0.9% slower. The report notes 2-4% wall-time noise between
identical runs.

There is no 1% instruction saving or 5% memory saving in this screen, and no convincing successful-check wall-time
gain to advance to the two required same-hardware headline publishes. No headline confirmation was run and no
headline speedup is claimed. The code is removed from the final PR diff. Revisit only if a fresh profile shows
subtree accumulation or group adjacency dominating assignment, or a representation removes the temporary maps
and group-member lists.
