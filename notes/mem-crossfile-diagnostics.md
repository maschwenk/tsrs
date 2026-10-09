# crossfile-diagnostics: work a checker spends on diagnostics for files it does not check

Question: #212 (notes/perf-excalidraw-typefest.md) found that on excalidraw every checker repeated one
type-argument constraint check whose error elaboration was thrown away in all but the owning checker. Does the same
pattern, generalized, cost real memory or instructions on the six projects where bun check beats tsrs on peak memory?
That is: a checker files a diagnostic against file X while checking file Y, only X's own checker's copy is collected,
so the other copies, and the elaboration and printing behind them, are wasted.

**Answer: no.** On every loss project the upper bound is below 0.25% of peak at 8 checkers and below 0.6% of the
instructions at 1 checker. In the A/B runs, skipping the reporting does not change output, and peak moves within
noise (webpack, a control, -1.9%). Nothing clears the gain bar. The excalidraw case was an outlier, and #212 already
fixed its site.

Branch: `exp/crossfile-diagnostics` (c3e47532, on main 872027a4), pushed, not for merge. Mac (M5 Max, shared, load
5-15), 8 checkers unless stated. No Linux runs were made: no saving is claimed, and the Mac upper bound is about 20
times smaller than the bar.

## Method

The instrumentation is compiled in with `--features tsrs_cli/xfile-stats` and enabled at run time by
`TSRS_XFILE=<out.tsv>`.

**Counters, per thread:**

- arena bytes allocated, counted in `Arena::alloc_layout`;
- heap bytes allocated, counted by a wrapper around mimalloc;
- instructions retired, read with `thread_selfcounts`.

**Hooks:**

- **Diagnostic entry points.** `add_diagnostic` and `add_suggestion_diagnostic` record each diagnostic whose file is
  not `checking_file`. All entry points funnel into these two: `error`, `error_or_suggestion`,
  `lookup_or_issue_error`, `report_diagnostic`, and the relater's `diagnostic_output`. They are `#[track_caller]`, so
  the site recorded is the code that reported, along with the message code. Related information was counted as part
  of its parent diagnostic.
- **Reporting relations.** These are `check_type_related_to_ex` and `..._and_optionally_elaborate` with an error
  node. When the error node lies in another file, the hook records the work done inside the outermost such relation:
  total, and the share spent printing. It records the caller site and whether a diagnostic came out.
- **Printing.** The hook covers `type_to_string` / `symbol_to_string` / `signature_to_string` /
  `type_predicate_to_string`, outermost call only. A print inside a reporting relation is charged to that
  relation's file. Any other print is charged to `current_node`'s file.
- **Collection.** `get_diagnostics` records the files each checker collects and the final diagnostics.

**Fate of each cross-file item:**

- **kept:** this checker checks X later, so the diagnostic reaches output.
- **lost-other:** another checker checks X.
- **lost-already:** this checker already collected X.
- **lost-unchecked:** no checker checks X, for example a `node_modules` `.d.ts` under skipLibCheck.

**Reproduced:** the same text at the same position appears in the final output.

**Upper bound saved:** all work in lost reporting relations, plus lost prints outside them. It includes the relation
work itself, which reporting-off would not save.

**Real test:** `TSRS_XFILE_SKIP=lost`, in any build, runs those relations with reporting off. A static assignment
(`--checkerAssignment locality`, no stealing) tells each checker which files it owns. `TSRS_XFILE_SKIP=all` skips
every cross-file report; it is the upper bound and is not exact.

## Table

The peak is the clean binary's (median of 3 at 8 checkers with the default stealing; one run at 1 checker). The bound
is from the instrumented run. Retained memory is the arena column. The "+ heap" column assumes every heap byte
allocated was live at peak, which overstates it: those bytes are message strings, freed right away. The A/B pair is
base vs skip=lost under locality, median of 3.

| project | checkers | peak MiB | bound: arena / + all heap MiB (% of peak) | instructions G (% of run) | of which printing G | A/B peak MiB |
|---|---:|---:|---|---|---:|---|
| t3code-server | 1 | 772 | 0.0 / 0.1 (0.00% / 0.0%) | 0.001 (0.00%) | 0.000 | 772 -> 774 |
| t3code-server | 8 | 1800 | 1.4 / 4.2 (0.08% / 0.2%) | 0.258 (0.19%) | 0.000 | 1342 -> 1341 |
| supabase-studio | 1 | 913 | 0.0 / 0.0 (0.00% / 0.0%) | 0.001 (0.00%) | 0.001 | 913 -> 909 |
| supabase-studio | 8 | 1381 | 0.6 / 2.5 (0.04% / 0.2%) | 0.058 (0.06%) | 0.005 | 1378 -> 1379 |
| mikro-orm | 1 | 1138 | 0.2 / 0.6 (0.02% / 0.1%) | 0.011 (0.01%) | 0.011 | 1138 -> 1142 |
| mikro-orm | 8 | 1629 | 0.3 / 1.1 (0.02% / 0.1%) | 0.013 (0.01%) | 0.013 | 1626 -> 1641 (noise) |
| cal-diy | 1 | 796 | 0.4 / 33.8 (0.06% / 4.2%) | 0.232 (0.55%) | 0.232 | 796 -> 795 |
| cal-diy | 8 | 1456 | 1.0 / 4.2 (0.07% / 0.3%) | 0.105 (0.13%) | 0.017 | 1311 -> 1311 |
| formbricks-web | 1 | 1129 | 0.1 / 0.1 (0.00% / 0.0%) | 0.002 (0.00%) | 0.001 | 1129 -> 1139 (noise) |
| formbricks-web | 8 | 1693 | 3.9 / 28.3 (0.23% / 1.7%) | 0.599 (0.78%) | 0.128 | 1624 -> 1622 |
| vscode | 1 | 1647 | 0.0 / 0.1 (0.00% / 0.0%) | 0.001 (0.00%) | 0.000 | 1647 -> 1633 |
| vscode | 8 | 1952 | 0.2 / 0.5 (0.01% / 0.0%) | 0.011 (0.01%) | 0.001 | 1941 -> 1939 |
| drizzle-orm (control) | 1 | 412 | 0.7 / 1.7 (0.17% / 0.4%) | 0.016 (0.10%) | 0.016 | 412 -> 413 |
| drizzle-orm (control) | 8 | 575 | 0.8 / 2.7 (0.13% / 0.5%) | 0.018 (0.09%) | 0.018 | 526 -> 527 |
| webpack (control) | 1 | 323 | 0.1 / 0.1 (0.02% / 0.0%) | 0.001 (0.01%) | 0.001 | 323 -> 321 |
| webpack (control) | 8 | 438 | 4.1 / 15.9 (0.93% / 3.6%) | 0.175 (1.02%) | 0.099 | 422 -> 414 |

How to read the instruction columns:

- **The 8-checker share is of the whole multi-checker run.** The bar's instruction criterion is single-threaded, and
  there the bound is at most 0.55% (cal-diy). On the Mac, `/usr/bin/time -l` single-threaded counts vary by 1-2%
  from run to run, so the A/B cannot resolve this; the attributed numbers are the estimate. The paired cal-diy runs
  agree with them: -0.28 and -0.25 G against 0.23 G attributed.
- **Walls (locality, 3 reps) do not move:**
  - formbricks-web: 1.03 vs 0.99 s
  - webpack: 0.30 vs 0.24 s
  - t3code-server: 1.35 vs 1.36 s

**Output:**

- `TSRS_XFILE_SKIP=lost`: diagnostics byte-identical to base on all 8 projects, at 1 checker and at 8 (locality).
- `TSRS_XFILE_SKIP=all`: identical on 7 projects. supabase-studio gains 2 lines (details in the next section).

## Top sites

Pooled over the 8 projects at 8 checkers, the lost work totals 12.3 MiB of arena, 47 MiB of heap allocated and
1.24 G instructions. By site, ranked by instructions:

| site | share of lost instructions | where | printing? | what it is |
|---|---:|---|---|---|
| `check_satisfies_expression` (checker_06.rs:114), relation | 56% (0.70 G) | 0.46 G formbricks-web, 0.14 G t3code | none | The relation itself. An `x satisfies T` initializer in X is checked when Y needs X's type. The check is report-only: its result is unused. |
| `check_property_assignment` with a type annotation (checker_07.rs:1100), relation | 14% (0.17 G) | webpack only | 0.10 G | JSDoc-typed property assignments. 112 TS2322 per run, all reproduced by the owner. |
| `get_fully_qualified_name` (checker_08.rs:1063), print outside a relation | 10% (0.13 G) | one call in formbricks-web | yes | The diagnostic is filed against a `node_modules` file nobody checks. |
| `get_iterated_type_or_element_type` (checker_03.rs:2284), relation | 8.5% | 54K relations | none | |
| `check_variable_like_declaration` (checker_03.rs:2045, 2014), relation | 4% | | none | |
| `report_nonexistent_property` (checker_06.rs:1105), print | 3% | | yes | The diagnostic is filed against unchecked `.d.ts` files. |
| `check_type_arguments` (checker_05.rs:798), relation | 1.6% | | | |
| `is_signature_applicable` (checker_05.rs:861), relation | 1.3% | | | Call resolution. |

The bulk of it is relation work that runs with reporting off as well. **Printing is 0.30 G and 4.6 MiB of arena in
total.**

**Cross-file diagnostics actually filed (D rows, errors only):**

| project | lost and reproduced by the owner | lost-unchecked (nobody outputs them) | kept |
|---|---:|---:|---:|
| webpack | 944 | | 183 |
| drizzle-orm | 875 | 150 | 1,216 |
| formbricks-web | | 1,005 (TS2694 `resolve_qualified_name` in `node_modules`) | |
| cal-diy | 52 | 175 | 9 |
| supabase-studio | 11 | 138 | 4 |
| vscode | 29 | | 18 |
| mikro-orm | 16 | | 13 |
| t3code-server | | 15 | |

Every lost-other error was reproduced by X's owner. Every kept error is in the output.

**Suggestion diagnostics are never collected by the CLI.** These are TS6385 deprecations (`add_deprecated_suggestion`),
a few hundred per project, all cheap.

The **backtraces** that lead to the cross-file adds are:

- return-expression inference (`check_and_aggregate_return_expression_types` / `..._yield_operand_types`), which checks
  a function body in X when Y needs its return type;
- module and qualified-name resolution (`get_exports_of_module_worker`, `get_type_arguments_from_node`);
- call resolution (`report_call_resolution_errors`).

## Exact and not exact

**Exact (the owning checker reaches the same check on its own, so skipping or deferring changes only the relation
cache's history):**

- **The report-only relations.** These are `check_satisfies_expression`, the typed property assignment,
  `check_variable_like_declaration` and `check_type_predicate`/`check_type_parameter` defaults. Each is a check of a
  node in X that X's own statement walk visits.
- **Every lost-other error diagnostic seen.** All of them were reproduced at the same position, with the same text.
- **Diagnostics filed into files no checker checks** (lost-unchecked). No output can contain them.

**Not exact (history channel, leave alone):**

- **The call-resolution path.** These are `report_call_resolution_errors` (checker_05.rs:1343) via
  `is_signature_applicable`, and anything else reached through a cached resolution (`resolve_call`, return-type
  inference).
  - When the same checker files such a diagnostic while checking Y and checks X later, the diagnostic is kept. X's own
    check does not reproduce it, because the resolution is cached.
  - `TSRS_XFILE_SKIP=all` shows this. On supabase-studio it drops two kept call-resolution errors (TS2353 in
    `pages/api/generate-attachment-url.ts`, TS2322 in `data/content/sql-folders-delete-mutation.ts`). Two
    `TS2578 Unused '@ts-expect-error' directive` errors appear instead.
  - Only deferring the reporting to X's check, as #212 does, keeps these. Deferral re-runs the elaboration against a
    different cache state, which is the rxjs elaboration-text channel in notes/open-history-dependence.md.
- **Skipping requires knowing ownership.** With stealing on, a checker cannot know at report time that it will never
  check X. Only the already-collected case is safe without a queue.

## Best candidate and why it is not worth landing

Generalize #212's deferral queue to the report-only relations: run the check in the checker that checks the node's
file, and drop it elsewhere. It is exact by #212's argument for those sites.

**Expected gain on formbricks-web, the largest:** at most 0.6 G of 77 G instructions at 8 checkers (0.8%) and 3.9 MiB
of 1,693 MiB peak (0.23%). Single-threaded it is about 0.

For comparison, turning reporting off where X is lost saves only the printing, 0.13 G and 0.3 MiB there. Neither
reaches 5% of peak at 8 checkers or 1% of single-threaded instructions, and both would add an invariant (a per-file
queue per site). **Recommendation: do not implement; record as measured and rejected.**

## Side findings (outside the question)

- **Checker-construction merge errors.** Every checker's constructor merges globals (`merge_global_symbol` ->
  `report_merge_symbol_error` / `add_duplicate_declaration_error`: TS2300, TS2451, TS2649) and prints the
  duplicate-declaration errors with no file being checked. Only one copy is collected. On drizzle-orm this is 49K adds
  and 11.2 MiB arena / 0.18 G at 8 checkers (2% of peak). It is 0.1-0.3 MiB on the loss projects. Go does the same.
- **The owning checker's own discarded elaboration is larger than the cross-file channel.** These are same-file
  reporting relations that produce a diagnostic into a `diagnostic_output` or restore their error state. At 8 checkers
  they printed:

  | project | arena | heap allocated | instructions |
  |---|---:|---:|---:|
  | supabase-studio | 8.5 MiB | 188 MiB | 1.6 G |
  | formbricks-web | 2.9 MiB | 79 MiB | 0.68 G |
  | t3code-server | 0.7 MiB | 63 MiB | 0.43 G |

  This is #212's candidate 3 (lazy formatting) for the owner. The arena part is still under 1% of peak; most of it is
  transient heap.

## Reproduce

```sh
git -C ~/Developer/tsrs-work/tsrs worktree add ~/Developer/tsrs-work/wt/crossfile origin/exp/crossfile-diagnostics
cd ~/Developer/tsrs-work/wt/crossfile
CARGO_BUILD_JOBS=8 CARGO_TARGET_DIR=target/xfile cargo build --release --locked -p tsrs_cli --features xfile-stats
CARGO_BUILD_JOBS=8 cargo build --release --locked -p tsrs_cli
F="--noEmit --incremental false --pretty false"
TSRS_XFILE=/tmp/x.tsv target/xfile/release/tsrs -p <project> $F --checkers 8 && python3 tools/perf/xfile_analyze.py /tmp/x.tsv
TSRS_XFILE_SKIP=lost target/release/tsrs -p <project> $F --checkers 8 --checkerAssignment locality   # vs without the env
TSRS_XFILE_SKIP=all  target/release/tsrs -p <project> $F --checkers 8                               # upper bound, not exact
# mikro-orm install needs YARN_NM_MODE=classic on this Mac (hardlink mode fails linking wrap-ansi)
```

Raw data is in this directory: `<project>.tsv` (8 checkers), `<project>.c1.tsv` (1 checker), `ab.tsv`, `table.md`
and `sites.txt`.
