# mem-free-leaf-files: free a checked leaf file's tree and binder output (CLI `--noEmit`)

The one exact, double-digit lever from the Bun study (notes/bun-check-memory.md): Bun's `bun check` marks a module
`is_leaf` when no file refers to it and it adds nothing globally visible, and frees its tree and binder output at the
end of the task that checked it (`free_tree`). tsrs never freed any of the front end. This change does the same for
the CLI's `--noEmit` check, for the files whose path predicts a leaf (tests, specs, stories, mocks), by default only
when the program gets at most 16 checkers. On vscode that is 2,289 of its 2,337 leaves, 321 MB of parse and bind
output; the peak goes down 11% on Linux at 16 checkers for 1.6-2% more wall time, and 12-14% on macOS at 4 and 16, with
byte-identical output. At 32 checkers it costs 2.5-5% wall time, so it is off there unless asked for.

## Mechanism

- **Placement by prediction** (crates/tsrs_compiler/src/fileregions.rs). Whether a file is a leaf is known only once
  the whole program is loaded, but where its tree goes is decided when it is parsed. A first version gave every
  TypeScript root file a region of its own: every tree moved from the thread arenas, whose large chunks are huge pages
  on Linux, to 4 KiB-page regions, and vscode at 32 checkers lost 6-9% wall time. Now the host gives a region only to
  a TypeScript source file (`.ts`, `.tsx`, `.mts`, `.cts`, not a declaration file) that is loaded for a root file of
  the program and whose path, relative to the current directory, contains one of `PREDICTED_LEAF_PATTERNS`: `.test.`,
  `.spec.`, `/test/`, `/tests/`, `/__tests__/`, `.stories.`, `/__mocks__/`. Every other file is parsed into the thread
  arena exactly as on main. The prediction only places trees; the leaf test below stays exact: a predicted file that
  is not a leaf keeps its region for the life of the process (correct, slightly less dense), and a leaf that was not
  predicted is simply not freed. A root file that nothing imports cannot be recognized at parse time (its importers
  may not be parsed yet). `TSRS_FREE_LEAVES=all` restores the every-file placement for measurement.
- **What stays outside a region.** The text (leaked and registered, as before: `parse_source_file_keep_text`), the
  `SourceFile` node itself (the parser escapes the scratch region for it), and every diagnostic (diagnostic.rs already
  escapes scratch regions). Lazily filled per-file data (line map, JSDoc caches) is routed by `enter_owner` of the
  `SourceFile`, which is outside every region, so it lands in the thread arena. A freed file keeps its name, path,
  text, line map, counters and collected diagnostics. Each region is trimmed after parse and after bind; every bind
  site binds in the file's region.
- **Leaf marks.** `classify` runs at the start of the type-check pass (`get_semantic_diagnostics(None)`), after load
  and bind and before any checking, and sets `SourceFile::is_check_leaf` on the leaves that have a region. Regions that
  will never be freed forget their drop lists (all `SymbolTable`s).
- **Free.** In the pass's callback, after `get_semantic_diagnostics_with_checker` has collected the leaf's bind,
  checker and include-processor diagnostics and applied the `@ts-ignore` / `@ts-expect-error` and no-emit filters,
  the checker thread that checked it drops its region with `Region::retire_on_free`. A region is `Send + Sync`; the
  `OwnerLock` only matters while a scope is entered, and none is once binding is done.
- **Retire** (crates/tsrs_core/src/arena.rs, reserve.rs). A plain region free could not give a file region's memory
  back: file regions share 1 MiB slabs, and a slab was only cached or decommitted once all its chunks were gone. A
  retired region's chunks join a set of coalesced retired spans; every 16 MiB (and once after the pass) the pages that
  became wholly retired go back to the system (`reserve::discard`: Linux `MADV_DONTNEED`, macOS a no-access mapping
  over them). Coalescing frees the page two neighbouring leaves share (vscode, every-file placement: 4,691 leaf chunks
  formed 924 runs; with 16 KiB pages the shared pages were a tenth of the leaves' bytes) and cuts the system calls from
  one per chunk to about one per run. A retired range is never handed out again, so no later object takes the address
  of a freed one that an address-keyed cache may still name.
- **Default: at most 16 checkers** (`MAX_DEFAULT_CHECKERS`). The CLI turns file regions on only when the program can
  get at most 16 checkers (`checker_count_upper_bound`: `--checkers`, one when single-threaded, else the machine's
  default before the file-count cap: half the threads, so machines up to 33 threads). Above that the measured cost was
  2.5-5% wall time on vscode (below). `TSRS_FREE_LEAVES=1` (and `keep`, `all`) turns it on at any count.
- **Platforms.** Only with compressed pointers on unix, where retiring gives pages back and never reuses the range.
  Elsewhere (`plain-ptrs`, non-unix) a retired region's memory stays mapped with its old contents while the heap
  buffers its values owned are freed and reused, so a stale read would see a live object, and nothing is saved: file
  regions are off there.

## The leaf test, clause by clause

A file is a leaf only if all hold:

| clause | why |
| --- | --- |
| it has a file region | only predicted TS files loaded for a root file get one |
| type-checked (`!skip_type_checking`) | otherwise nothing checks it and nothing frees it |
| TypeScript (`.ts`, `.tsx`, `.mts`, `.cts`), not a declaration file | JavaScript: CommonJS exports and expando assignments can create global symbols; declaration files are mostly not checked (`skipLibCheck`) and may declare globals |
| an external module | a script's top-level declarations are merged into the globals by every checker |
| no module augmentation (this includes `declare global`), no ambient module, no pattern ambient module | merged into other modules or the globals at checker creation |
| no UMD global (`export as namespace`) | merged into the globals at checker creation |
| no `// @ts-check` / `// @ts-nocheck` directive | `skip_type_checking` reads it through the file, also after the pass (the checker cost cache) |
| its export table holds no alias, no `export *`, no `export =` | the checker asks every module of the program whether it re-exports a symbol (below) |
| no other file refers to it | no include reason but root file (this covers imports, `/// <reference path>`, type reference and lib reference directives, automatic type directives); and not the target of another file's resolved module (imports, module augmentations, the synthetic JSX and helper imports) or type reference directive |

Edges the program records are all covered by the include reasons and the resolutions; project references only reach
the build through declaration outputs (not checked, so not leaves) or redirected sources (their importers' resolutions
point at them). `globalThis` assignments do not declare globals in TypeScript; JSX factories are resolved from the
importing file's scope (an import edge); `--types` roots are automatic type directives; a file in tsconfig `files`
that another file imports has an import reason.

## Accesses to a file after its check, and how each is settled

| access | settled by |
| --- | --- |
| semantic diagnostics of the file (bind, checker, include processor; directives; no-emit filter) | collected in the same callback, before the free |
| syntactic diagnostics | collected before the pass; the pass only runs when there are none |
| include-processor diagnostics and their locations (import nodes of the importing file) | computed once (a `OnceLock`) in the program-diagnostics phase, before the pass |
| global diagnostics after the pass | checker data only |
| diagnostic sorting, dedupe, plain and `--pretty` reports, error summary, exit code | the `SourceFile` (name, path), its text and line map, all outside the region; the line map is built lazily in the thread arena |
| `--listFiles` | file names |
| `--extendedDiagnostics` | Lines from the text, Identifiers and Symbols from `SourceFile` counters, Types and Instantiations from the checkers |
| `program.emit` under `--noEmit` | returns before it touches a file |
| declaration diagnostics (`--noEmit` with `declaration` or `composite`) | not settled: they run the declaration transform over every file after the pass, so leaf freeing is off |
| `--explainFiles` | not settled: it reads each importer's import nodes, so leaf freeing is off |
| `--checkerCostCache` | reads `SourceFile` fields only (leaves have no ts-check directive) |
| `TSRS_FILE_TIMES`, `TSRS_ASSIGNMENT_STATS`, the work, heap and reachability censuses | walk trees or checker tables after the pass: leaf freeing is off |
| checker creation (globals, augmentations, UMD globals, pattern ambient modules) | runs when the pool is created, before the pass |
| deferred diagnostics | produced at the end of the file's own check |
| getAlternativeContainingModules' loops over every module (printer.rs) | skip other checkers' leaves, see below |
| checker caches keyed by types and symbols | keyed by ids, not structure: another file's structurally identical type is a different type and never reaches the leaf's (testdata/regressions/leaf-structural-instantiation) |
| `--checkerAssignment go` | Go's per-name `x?: undefined` cache keeps another file's declarations: leaf freeing is off |
| one file's diagnostics after the pass (`get_semantic_diagnostics(Some(file))` and the other single-file entry points) | would read the freed tree, bind diagnostics and comment directives: they panic for a freed leaf (`fileregions::assert_not_freed`). The CLI never calls them after the pass, and the pass returns all files' diagnostics together rather than keeping each file's, so keeping them for a caller that does not exist was not worth the memory |
| a second type-check pass | asserted not to happen (the CLI runs one) |

All of this is verified by `TSRS_ARENA_POISON=1` runs (freed regions filled with `0xA5` and kept, so any read
crashes): the seven projects at 1, 4 and 16 checkers, the five application projects at 32 on Linux, and the
conformance suite.

## The one checker change

`Checker::is_unreadable_check_leaf`: the fallback loop of getAlternativeContainingModules and `modules_exporting` (the
tsrs index that answers it) pass over a check leaf other than the file this checker is checking (`checking_file`, set
in `check_source_file`). Asking it would give nil: nothing outside a leaf refers to it and it declares nothing global,
so a symbol this checker names while checking another file is declared outside the leaf and the leaf is not its
parent; the leaf's export table holds only its own declarations (no alias, `export *` or `export =`), so no entry
resolves to that symbol and `getAliasForSymbolInContainer(leaf, symbol)` is nil; and asking has no effect that reaches
output (no `export *` to merge, the only part of `getExportsOfModule` that reports; the index entries it would add are
keyed by the leaf's own symbols). The checker that checks the leaf asks it as before. The loop runs whenever a printed
type reuses a type node written in another file and asks whether its name is accessible, also for errors a
`@ts-expect-error` then suppresses (formbricks-web reports none and runs it): without the guard the poison run crashes
on formbricks-web, and testdata/regressions/leaf-alternative-containers (tsgo-ref's output) has a freed leaf in that
loop (`cargo test -p tsrs_cli --test free_leaf_files`, which fails with the guard removed).

## Address space

A retired range is never reused, so a run uses as much of the 32 GiB reservation as if nothing were freed, which is
what the batch compiler did before. Regions take about a third more reservation than they use (unused chunk tails).
With the predicted placement only the predicted files' regions count (vscode: 444 MB reserved for 342 MB used, against
2.1 GB of reservation in use at 4 checkers). The every-file placement bounds it from above: 1,050 MB for 794 MB on
vscode; the 38k-file codebase has 27.5k TypeScript files with 954 MiB of parse and bind output
(notes/design-persisted-frontend.md), so at most about 0.3 GiB more, against a worst measured peak of 11.4 GiB.

## Numbers

Leaves, predicted placement (`TSRS_FREE_LEAVES=stats`, 4 checkers, macOS; bytes used by the regions):

| project | checked files | leaves freed | leaves not predicted (share of the leaves' nodes) | freed leaves' bytes | file regions (bytes) |
| --- | --- | --- | --- | --- | --- |
| vscode | 9,399 | 2,289 | 48 (0.8%) | 321.0 MB | 2,514 (342.3 MB) |
| formbricks-web | 3,477 | 1,029 | 107 (2.3%) | 61.6 MB | 1,059 (62.0 MB) |
| supabase-studio | 5,318 | 703 | 24 (2.8%) | 28.9 MB | 715 (29.1 MB) |
| t3code-server | 1,518 | 499 | 19 (2.5%) | 67.2 MB | 499 (67.2 MB) |
| cal-diy | 3,136 | 61 | 145 (29.1%) | 4.0 MB | 72 (4.1 MB) |
| xstate-main | 248 | 109 | 20 (0.3%) | 12.7 MB | 113 (12.8 MB) |
| webpack | 1,551 | 0 (JavaScript) | | | |

The every-file placement frees 2,337 / 1,136 / 727 / 518 / 206 / 129 leaves (vscode: 319 MB of 794 MB in 9,399 regions).

Linux, 64-vCPU Depot runner, `--noEmit --incremental false --extendedDiagnostics`, main 249561e (the branch merged
with it), freeing at every checker count (as before the 16-checker default), median of 5 interleaved runs (`depot ci
run` 230c9j763w of .depot/workflows/perf-probe.yml with a probe script that is not committed):

| project | checkers | maxrss main | predicted | every file (`all`) | wall main | predicted | every file | Check time main | predicted | every file |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| vscode | 32 | 2,861 MB | 2,564 MB (-10.4%) | 2,595 MB (-9.3%) | 0.68 s | 0.68 s | 0.70 s | 0.482 s | 0.479 s | 0.485 s |
| vscode | 16 | 2,582 MB | 2,287 MB (-11.4%) | 2,323 MB (-10.0%) | 0.97 s | 0.98 s | 0.99 s | 0.775 s | 0.782 s | 0.789 s |
| formbricks-web | 32 | 3,033 MB | 2,987 MB (-1.5%) | 2,989 MB | 0.74 s | 0.75 s | 0.76 s | 0.546 s | 0.557 s | 0.564 s |
| supabase-studio | 32 | 2,461 MB | 2,436 MB (-1.0%) | 2,456 MB | 0.65 s | 0.66 s | 0.66 s | 0.473 s | 0.473 s | 0.480 s |
| t3code-server | 32 | 2,889 MB | 2,842 MB (-1.6%) | 2,847 MB | 1.59 s | 1.67 s | 1.61 s | 1.460 s | 1.539 s | 1.493 s |
| t3code-server | 16 | 2,348 MB | 2,293 MB (-2.3%) | 2,303 MB | 1.73 s | 1.72 s | 1.73 s | 1.629 s | 1.608 s | 1.616 s |
| cal-diy | 32 | 2,766 MB | 2,764 MB | 2,777 MB | 0.63 s | 0.63 s | 0.63 s | 0.487 s | 0.487 s | 0.486 s |

t3code-server's check is a tail of a few heavy files and its wall time is bimodal run to run (1.56 or about 1.68 s,
main included). More interleaved runs where the 5-run medians were noisy or near the bar (k41qm6nz17: 10 runs, 12 for
t3code-server; wall time median, mean in brackets):

| project | checkers | wall main | predicted | change | maxrss change |
| --- | --- | --- | --- | --- | --- |
| vscode | 32 | 0.70 s | 0.70 s | +0.0% (+0.3%) | -10.0% |
| vscode | 16 | 0.99 s | 1.00 s | +1.0% (+1.5%) | -11.4% |
| formbricks-web | 32 | 0.73 s | 0.74 s | +2.1% (+1.8%) | -1.9% |
| supabase-studio | 32 | 0.65 s | 0.66 s | +1.5% (+1.4%) | -0.7% |
| t3code-server | 32 | 1.65 s | 1.60 s | -3.0% (-1.1%) | -2.1% |

After merging main again (#138-#141, main d94c544), vscode at 32 checkers no longer came out even. A pr-verify run (3
runs) reported +5.5%; four probes of interleaved runs, both binaries built in the same job, settle it (always freeing,
`TSRS_FREE_LEAVES=1`):

| probe | checkers | runs | wall change, median (mean) | Check time change, mean | maxrss change |
| --- | --- | --- | --- | --- | --- |
| 818kt6d663 | 32 | 20 | +4.5% (+4.2%) | +5.4% | -10.3% |
| 818kt6d663 | 16 | 20 | +1.6% (+1.7%) | +1.9% | -11.4% |
| g8t4jk2q6s | 32 | 15 | +1.5% (+2.3%) | +2.8% | -10.2% |
| 4q7p39slvf | 32 | 15 | +6.3% (+4.9%) | +5.5% | -10.5% |

Decomposed at 32 checkers (means against main; g8t4jk2q6s and 4q7p39slvf): file regions off in the branch -0.7%;
regions for the predicted files, nothing freed (`keep`) +1.3%; freed with the pages kept (a throwaway switch) +1.2% and
+3.1%; freed and given back +2.3% and +4.9%. Giving pages back in 64 MiB or 256 MiB batches instead of 16 MiB does not
help (+4.9%, +4.5%; 256 MiB also gives back 110 MB less by the peak). So at 32 checkers both the predicted files' trees
in 4 KiB pages and the TLB shootdowns of each `madvise` (which interrupt every core running a checker) cost about 1-3%,
above the 1-2% bar; at 16 the total is 1.6-2.1%. Hence the default: on up to 16 checkers. The confirmation run
(0p5njbn70r, default switch): 16 checkers, 20 runs, wall +2.1% (+2.0%), Check time +1.8% (+2.0%), maxrss -11.4%; 32
checkers, 12 runs (off by default), wall +0.0% (-0.3%), maxrss +0.0%.

Where the remaining time goes at 16 checkers and below (0rnsp0p9cv, means of 12 / 10 runs against main):
formbricks-web at 32 checkers is +1.7% with file regions off (`TSRS_FREE_LEAVES=0`, the same instructions as main: a
different binary's layout or the run order), +2.1% with the regions and nothing freed (`keep`), +2.4% freeing; giving
pages back in 64 MiB or 4 MiB batches or not at all changes nothing measurable there. vscode at 16: off -0.1%, keep
+1.2%, on +1.1%. madvise(MADV_COLLAPSE) of the regions was not tried: the trees in them are each read by one checker,
and a later discard would split the huge page again.

For comparison, the every-file placement against main 1cbf079 (29kcl2x5zp, 5 runs, 32 checkers): vscode wall 0.68 ->
0.74 s (+8.8%), maxrss -8.7%; formbricks-web +4.1% / -1.2%; supabase-studio +3.1% / -0.6%; t3code-server +2.5% /
-1.5%; cal-diy +1.6% / +0.5%. Its dTLB load misses on vscode at 32 checkers went from 5.4 M to 7.6 M.

macOS, M5 Max (18 cores, shared: load average 23-49, so times are not comparable), peak memory footprint
(`/usr/bin/time -l`), main 249561e against the branch merged with it, median of 3 interleaved runs:

| project | checkers | main | predicted | change |
| --- | --- | --- | --- | --- |
| vscode | 4 | 2,119 MB | 1,825 MB | -13.8% |
| vscode | 16 | 2,459 MB | 2,162 MB | -12.1% |
| formbricks-web | 4 | 1,694 MB | 1,644 MB | -2.9% |
| formbricks-web | 16 | 2,283 MB | 2,234 MB | -2.1% |
| supabase-studio | 4 | 1,258 MB | 1,236 MB | -1.8% |
| supabase-studio | 16 | 1,827 MB | 1,810 MB | -1.0% |
| t3code-server | 4 | 1,270 MB | 1,211 MB | -4.7% |
| t3code-server | 16 | 2,276 MB | 2,195 MB | -3.5% |
| cal-diy | 4 | 1,202 MB | 1,220 MB | +1.5% (ranges overlap) |
| cal-diy | 16 | 1,974 MB | 1,955 MB | -1.0% |

Single-threaded instructions (Linux, bench/count.py, two runs each): vscode 113.949 G -> 114.189 G (+0.21%);
t3code-server 57.767 G -> 57.865 G (+0.17%). Peak RSS single-threaded: vscode 2,036 -> 1,720 MB, t3code-server 891 ->
827 MB.

Alloc-profile build, vscode, 4 checkers: arena requested 1,581.0 -> 1,580.6 MB (the same objects); thread-arena chunks
1,903 -> 1,503 MB plus 444 MB of file regions (338 MB used); heap peak 445.3 -> 438.2 MB.

## Gates

- Output: stdout byte-identical to main (`cmp`, `--pretty false`) on vscode (371 errors), webpack (840), xstate-main
  (0), cal-diy (136), formbricks-web (0), supabase-studio (9), t3code-server (6) at 1, 4 and 16 checkers: 21 runs with
  the default switch and 21 with `TSRS_ARENA_POISON=1 TSRS_FREE_LEAVES=stats`, all identical, no crash, for both
  placements and again on the final head (merged with main d94c544; at 1, 4 and 16 checkers the default frees). On Linux at 32 checkers: predicted, poisoned and main identical for the five
  application projects. vscode `--pretty true`, `--listFiles` and the `--extendedDiagnostics` counters (single-threaded)
  identical with poisoned freed leaves.
- Conformance (`tsrs-test run --suite all`, 15,197 variants): 13,458 pass on main and the branch; with leaf freeing
  forced on in the runner (a temporary override, not committed: fresh, region-parsed files in programs whose trees
  nothing reads after the pass, canonical history), with every-file placement (2,339 leaves freed) and with the
  prediction, 13,458 pass at one and four checkers, poisoned and not; `comm -23` of the pass lists empty in every case.
  `.github/scripts/conformance-gate.sh` in both CI modes: errors 13,458, `.types` / `.symbols` 12,779 and 12,778.
- `cargo check --workspace` (no warnings), `tools/lint/ratchet.py`, `tools/lint/source.py`, `cargo test -p tsrs_core`
  (new: retired ranges are never reused; neighbouring retired chunks give back their shared page),
  `cargo test -p tsrs_compiler` (new: one file's diagnostics after its free panic), `cargo test -p tsrs_cli --test
  free_leaf_files --test derived_variance --test default_emit --test flow_memo`, tools/regressions.sh (21 cases). The
  tsrs_cli binary's own unit tests do not link on macOS (`malloc_trim`, before this change too).

## Not covered

- Runs that emit (JavaScript or declarations), `--explainFiles`, `--incremental` / `composite`, `--build`, the
  language server, the native API and the test harnesses: the trees are read after the check, or programs are reused.
- JavaScript, declaration and JSON files: never leaves. Builds without compressed pointers: off.
- Programs with more than 16 checkers by default (the default on machines with more than 33 threads):
  `TSRS_FREE_LEAVES=1` turns it on there.
- Leaves whose paths the prediction misses (cal-diy: 145 of its 206 leaves, 29% of the leaves' nodes, but 5.6 MB in
  all): not freed. `TSRS_FREE_LEAVES=all` frees them at the every-file placement's time cost.
- Files that re-export (barrel files) or that another file imports: by definition not leaves.
- The reachability census cannot check this: checker caches keep stale entries keyed by a freed leaf's nodes and
  symbols (never read), which it reports. File regions are off under `TSRS_CENSUS=1`; poison runs are the gate.
- On Linux a stray read of a freed leaf reads zeros (`MADV_DONTNEED`) instead of faulting.
- The pages a leaf shares with a live neighbouring chunk stay.
