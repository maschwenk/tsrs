# Rayon construction of processed-file maps

Decision: keep the measurement, remove the implementation. The screening run did not demonstrate a gain worth
the staging buffers and extra map-building pass.

Base: `15ecffbe9daabfa0078314c32bbfb01939452fa2`. The independent prototype is retained in
[commit b0f6295c](https://github.com/maschwenk/tsrs/commit/b0f6295c). It targets successful, non-incremental
`--noEmit` checks; diagnostic processing is unchanged.

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

Local validation of the prototype (macOS arm64, release): the focused unit case passed; all 29 regression cases
passed; 26 package-deduplication and type-reference conformance cases matched the base for errors, `.types` and
`.symbols` in parallel canonical mode. On pinned xstate-main, output and initial assignment/import-edge dumps
matched at 4, 16 and 32 checkers, and `--listFiles` / `--explainFiles` output matched at the default count.
`cargo check --workspace`, `tools/lint/ratchet.py` and `tools/lint/source.py` passed. Local wall time is not used
for the performance decision because other compiler jobs were running on the machine.

## Linux measurements

[PR verification report](https://github.com/maschwenk/tsrs/pull/292#issuecomment-6099611952), Depot run
`967l1qlnxb`: base `15ecffbe9daa`, tested PR merge `358108b260eb`, release builds on a 32-vCPU AMD EPYC 9R45
machine with 126 GB RAM. The two PRs used the same base and were measured independently. Three interleaved runs
per binary at 1, 4, 16 and 32 checkers, plus a single-threaded instruction count and a poisoned-arena run per
project. All 102 comparison cells had identical diagnostics and exit codes; the full CI check-and-test gate passed.

The successful-check projects at **16 explicit checkers** (not a PGO headline/default-mode publish):

| project | wall base -> new | wall delta | peak RSS base -> new | RSS delta |
| --- | ---: | ---: | ---: | ---: |
| xstate-main | 0.11 -> 0.11 s | +1.8% | 374 -> 373 MiB | -0.2% |
| mui-docs | 0.99 -> 1.01 s | +2.1% | 1.61 -> 1.62 GiB | +0.3% |
| formbricks-web | 0.70 -> 0.72 s | +1.8% | 1.99 -> 1.99 GiB | +0.2% |

| project | single-threaded instructions base -> new | delta | single-threaded peak RSS base -> new |
| --- | ---: | ---: | ---: |
| xstate-main | 6.799 -> 6.799 G | +0.002% | 183 -> 183 MiB |
| mui-docs | 46.341 -> 46.342 G | +0.003% | 740 -> 740 MiB |
| formbricks-web | 46.722 -> 46.723 G | +0.002% | 1.07 -> 1.07 GiB |

Values use the report's display rounding; its deltas use the underlying measurements. Across all 17 projects,
instruction changes were +0.000% to +0.003%. The largest absolute peak-RSS change among the timed cells was a
2.3% increase (xstate-main, 4 checkers). At 32 checkers mui-docs was 3.3% faster, but formbricks-web was 2.4%
slower; the report notes 2-4% wall-time noise between identical runs.

There is no 1% instruction saving or 5% memory saving in this screen, and no convincing successful-check wall-time
gain to advance to the two required same-hardware headline publishes. No headline confirmation was run and no
headline speedup is claimed. The code is removed from the final PR diff. Revisit only if a fresh profile shows
substantially more cost in map construction, or a design avoids buffering and the additional traversal.
