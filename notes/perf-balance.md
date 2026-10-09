# perf-balance: balancing the checkers

> Removed on 2026-10-09: the opt-in cost cache (`--checkerCostCache`, `TSRS_CHECKER_COST_CACHE`, the measured-cost
> placement and its refinement moves) was deleted. It stayed opt-in, so no published number used it, and work stealing
> (notes/perf-checker-stealing.md) balances the checkers at run time. `TSRS_FILE_TIMES` still records per-file CPU
> seconds for experiments. The measurements below stand as they were.

With 4 checkers the locality assignment (notes/mem-assignment.md) leaves the slowest checker ~10% above the mean on
the private monorepo, 22% on vscode and 35% on mui-docs. Wall time is the slowest checker's total, so the imbalance
is lost wall time. This note tries ordering, static cost models and a cost cache from a previous run.

## Measuring on a shared machine

The machine ran other agents' builds throughout (load average 11-54). Wall times moved 30-100% between rounds, so:

- `TSRS_ASSIGNMENT_STATS=times` now also prints `checker group cpu seconds` (thread CPU time per checker,
  `clock_gettime(CLOCK_THREAD_CPUTIME_ID)`). CPU time does not grow when another process takes the core, so the
  imbalance in CPU time (slowest checker / mean - 1) and the CPU sum (total work, i.e. duplication) are comparable
  across rounds. Wall comparisons are medians of interleaved rounds.
- Every run's error lines are hashed; all numbers below come from runs with identical output.

## 1. Ordering: no gain, and in-checker order is not free

With a static assignment there is no tail effect: each checker runs its fixed set and the wall time is the largest
set's total. The only free variable is the order inside a checker. `TSRS_CHECKER_FILE_ORDER=reverse|groupdesc|groupasc`
(experiment, not landed), the private monorepo, median of 3: CPU imbalance 9.3% / 9.2% / 7.9% vs 9.4% in program
order, and the slowest checker stays the slowest. The checker totals do not depend on the order; first-touch costs
only move between files of the same checker.

Order is also not semantically free. Reversing the order changes one diagnostic on the private monorepo: the
property order of a printed object-literal union in a TS2345 message (`available?: undefined` first or last).
Program order (dependencies first) is load-bearing. Checker membership is not: the output is byte-identical with
`--checkers 1, 2, 3, 4, 6, 8` and with the `go` and `locality` assignments. So a better assignment may change which
files a checker gets, but each checker must keep visiting its files in program order (it still does).

## 2. Static cost models: rejected

Per-file cost per weight unit (nodes + text/100, x4 for source files, plus normalized import fanout) differs by 40x
between kinds of files: 170 ns per unit on the private monorepo, 43 ns for mui-docs' 10,752 icon files
(`packages/mui-icons-material/lib`, 57% of mui's static weight but 0.23 s of 8.4 s summed checker time in that run) and 1.9 us for
mui's docs demos. The cost follows the type graphs that the file's imports reach for the first time in that
checker, which syntax does not show.

| variant (CPU imbalance, median of 3) | private monorepo | vscode | mui-docs |
| --- | --- | --- | --- |
| locality (current) | 9.4-10.2% | 22% | 36% |
| + 0.25 x weight of distinct files the group imports from outside itself | 17.1% | - | - |
| + 1 x the same | 20.7% | 27% | 77% |
| + 4 x the same | 9.4% | 26% | 98% |
| import fanout part x0 (syntax only) | 8.9% (CPU sum +8%) | 12% | 13% (but slowest checker 1.93 vs 1.81 s) |
| import fanout part x0.5 | 10.1% | 17% | 30% |
| import fanout part x2 | 7.3% | - | - |

The import-closure weight (what task 2 suggested; the earlier attempt used named imports and dynamic imports) is
worse everywhere: the import graph is one big component and direct imports say nothing about how expensive the
imported types are. Dropping the import fanout term balances vscode and mui better but adds work: on mui the fourth
checker then gets docs demos and pays the whole `@mui/material` first touch again, so its slowest checker is
slower. Nothing is better on all three projects, so the default stays unchanged.

## 3. Cost cache from a previous run: landed, opt-in

`--checkerCostCache <file>` / `TSRS_CHECKER_COST_CACHE=<file>` (locality assignment only; ignored with one checker):

- The run records each checked file's thread CPU seconds (two `clock_gettime` calls per file) and writes
  `# tsrs checker cost cache v2 checkers=<n>` plus `<seconds>\t<checker>\t<path>` per type-checked file
  (write-then-rename; the private monorepo: 28,338 entries, 5.5 MB). Unwritable paths are ignored.
- The next run reads it. Files without an entry cost their static weight converted at the cache's average seconds
  per weight unit; a missing, foreign or garbled file reads as empty and the run is the plain locality assignment.
- Groups are formed by the same directory-subtree rule but on measured cost, so a subtree that is cheap by syntax
  but expensive in fact is split (mui: `docs/src/components`, 2 s, a quarter of the total).
- Each group starts on the checker that ran most of its cost in the previous run (if the cache has the same checker
  count; otherwise FENNEL on measured costs), new groups go to the least loaded checker, and then single groups move
  off the slowest checker while that lowers the predicted maximum: only if the slowest checker is >= 3% above the
  mean, until it is within 1%, at most 64 moves. A moved group is predicted to cost 1.3x at the destination (measured:
  moving two groups worth 0.545 s off the slowest checker took 0.54 s off it and added 0.72 s to the destination).

Why it is built this way (each measured on the private monorepo, vscode and mui-docs):

- FENNEL re-placement on measured costs every run oscillates (the private monorepo: imbalance 5.6%, 6.1%, 12% on
  runs 2-4; symbols 17.19 M -> 17.48 M): moving groups moves first-touch costs, so the costs that justified a move
  are wrong after it.
- Refining from the static placement each run reached 2.4-4% on the private monorepo and vscode, but mui stayed at
  12-15%: a group moved to the idle checker is measured with the full first-touch cost and looks too expensive to
  move again on the next run, which starts from the static placement.
- Starting from the previous run's placement uses every cost in the context it was measured in, and the next run
  corrects a mispredicted move. Converges in 1-3 runs (CPU imbalance, runs 1-5 after an empty cache): the private
  monorepo 10.3 -> 3.6 / 2.8 / 3.0 / 1.5%, vscode 21 -> 4.8 / 3.5 / 2.9 / 1.0%, mui 37 -> 15 / 13 / 8.3 / 9.0%.
- Preferring the move with the strongest import affinity among near-best moves (to reduce duplication): within noise,
  not landed.

## Results (warm cache vs plain locality, interleaved, same binary)

The private monorepo, 8 rounds (load 11-30):

| | wall s | check s | per-checker wall s | CPU slowest / sum s | CPU imbalance | peak GB |
| --- | --- | --- | --- | --- | --- | --- |
| locality | 8.59 | 7.42 | 6.18 / 7.00 / 7.42 / 6.52 | 7.17 / 26.12 | 10.4% | 8.49 |
| + cost cache | **8.27 (-3.7%)** | 7.06 (-4.9%) | 6.96 / 7.06 / 7.05 / 6.69 | 6.88 / 27.02 (+3.4%) | 2.1% | 8.53 |

Cache wins 5 of the 8 pairs. Balancing costs 3.4% more total work (more duplicated first touches), which eats about
half of the theoretical gain.

vscode and mui-docs (bench/, 5 rounds, load ~15):

| | wall s | check s | per-checker wall s | CPU sum s | CPU imbalance | peak GB |
| --- | --- | --- | --- | --- | --- | --- |
| vscode, locality | 2.63 | 2.23 | 1.62 / 1.79 / 2.23 / 1.68 | 7.29 | 22.0% | 3.22 |
| vscode, + cost cache | **2.30 (-12.5%)** | 1.89 | 1.82 / 1.86 / 1.89 / 1.85 | 7.41 | 1.6% | 3.25 |
| mui-docs, locality | 1.79 | 1.34 | 0.11 / 1.34 / 1.28 / 1.23 | 3.96 | 35.1% | 1.40 |
| mui-docs, + cost cache | **1.52 (-15%)** | 1.09 | 1.05 / 1.03 / 1.09 / 1.09 | 4.24 | 2.6% | 1.48 |

Reading and writing the cache adds ~0.05-0.1 s outside checking on the private monorepo and nothing measurable on
the bench projects. Output was identical in every cache run (the private monorepo 19 errors, vscode 371, mui 0).

## Gates

Conformance suite (`--suite all --baselines types,symbols`) byte-identical to the base binary in both lazy modes,
with `TS_TEST_PROGRAM_SINGLE_THREADED=false` and single-threaded (whole `target/test-results` trees; 13,458 pass).
The private monorepo: output identical with 1 and 4 checkers and to the base binary; default counters unchanged
(17,187,182 symbols / 14,365,514 types / 79,522,288 instantiations); `--checkerAssignment go` with
`TSRS_LAZY_MEMBERS=0` identical to the base binary (40,743,802 / 16,630,033 / 90,406,448 on the current checkout;
the 39,704,001 of notes/mem-assignment.md was an older checkout). Without the option nothing is read or written and
the assignment code path is the old one; `--checkers 1` never reaches it.

## Remaining ideas

- A static model good enough for the default would need semantic cost (how expensive the types reached for the
  first time are). Sharing library instantiations between checkers would remove most first-touch duplication and
  with it most of the imbalance.
- The cache could live next to the tsbuildinfo by default when `--incremental` is on; tsrs writes no files today, so
  this stays opt-in.
