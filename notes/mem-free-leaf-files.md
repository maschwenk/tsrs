# mem-free-leaf-files: free a checked leaf file's tree and binder output (CLI `--noEmit`)

The one exact, double-digit lever from the Bun study (notes/bun-check-memory.md): Bun's `bun check` marks a module
`is_leaf` when no file refers to it and it adds nothing globally visible, and frees its tree and binder output at the
end of the task that checked it (`free_tree`). tsrs never freed any of the front end. This change does the same for
the CLI's `--noEmit` check. On vscode, 2,337 of 9,399 checked files are leaves, holding 319 MB of the 794 MB of parse
and bind output of its TypeScript sources; the peak goes down 9-13% with byte-identical output.

## Mechanism

- **File regions** (crates/tsrs_compiler/src/fileregions.rs). When the CLI turns them on, the host parses each
  TypeScript source file (`.ts`, `.tsx`, `.mts`, `.cts`, not declaration files) that is loaded for a root file of the
  program into a scratch region of its own (`Region::new_scratch`, first chunk 10x the text plus 4 KiB, trimmed after
  parse and again after bind). A file loaded for an import or a reference has a referrer and is never a leaf, so it
  is parsed as before (cal-diy: 907 regions instead of 3,243). Every bind site binds in the file's region.
- **What stays outside the region.** The text (leaked and registered, as before: `parse_source_file_keep_text`), the
  `SourceFile` node itself (the parser escapes the scratch region for it), and every diagnostic (diagnostic.rs already
  escapes scratch regions). Lazily filled per-file data (line map, JSDoc caches) is routed by `enter_owner` of the
  `SourceFile`, which is outside every region, so it lands in the thread arena. A freed file keeps its name, path,
  text, line map, counters and collected diagnostics.
- **Leaf marks.** `classify` runs at the start of the type-check pass (`get_semantic_diagnostics(None)`), after load
  and bind and before any checking, and sets `SourceFile::is_check_leaf` (the test is below). Regions that will never
  be freed forget their drop lists (551k entries on vscode, all `SymbolTable`s).
- **Free.** In the pass's callback, after `get_semantic_diagnostics_with_checker` has collected the leaf's bind,
  checker and include-processor diagnostics and applied the `@ts-ignore` / `@ts-expect-error` and no-emit filters,
  the checker thread that checked it drops its region with `Region::retire_on_free`. A region is `Send + Sync`; the
  `OwnerLock` only matters while a scope is entered, and none is once binding is done.
- **Retire** (crates/tsrs_core/src/arena.rs, reserve.rs). A plain region free could not give a file region's memory
  back: file regions share 1 MiB slabs, and a slab was only cached or decommitted once all its chunks were gone. A
  retired region's chunks join a set of coalesced retired spans; every 16 MiB (and once after the pass) the pages that
  became wholly retired go back to the system (`reserve::discard`: Linux `MADV_DONTNEED`, macOS a no-access mapping
  over them). Coalescing frees the page two neighbouring leaves share (vscode: 4,691 leaf chunks form 924 runs; with
  16 KiB pages the shared pages were a tenth of the leaves' bytes) and cuts the system calls from one per chunk to
  about 950. A retired range is never handed out again, so no later object takes the address of a freed one that an
  address-keyed cache may still name.

## The leaf test, clause by clause

A file is a leaf only if all hold:

| clause | why |
| --- | --- |
| it has a file region | only TS files loaded for a root file get one |
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

On vscode, 2,354 checked files have no importer (`TSRS_ASSIGNMENT_DUMP`, the Bun study); the other clauses exclude 17
of them.

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
| `--checkerAssignment go` | Go's per-name `x?: undefined` cache keeps another file's declarations: leaf freeing is off |
| a second type-check pass | would read the freed bind diagnostics and comment directives; asserted not to happen (the CLI runs one) |

All of this is verified by `TSRS_ARENA_POISON=1` runs (freed regions filled with `0xA5` and kept, so any read
crashes): the seven projects at 1, 4 and 16 checkers and the conformance suite.

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
on formbricks-web, and
testdata/regressions/leaf-alternative-containers (tsgo-ref's output) has a freed leaf in that loop
(`cargo test -p tsrs_cli --test free_leaf_files`, which fails with the guard removed). The conformance suite frees
2,339 leaves but never reaches the loop with one.

## Address space

A retired range is never reused, so a run uses as much of the 32 GiB reservation as if nothing were freed, which is
what the batch compiler did before. File regions take about a third more reservation than they use (unused chunk
tails: vscode 1,050 MB for 794 MB). vscode's peak reservation in use is 2.0 GB at 4 checkers (the same with and
without freeing) and 3.1 GB at 32. The 38k-file codebase has 27.5k TypeScript files with 954 MiB of parse and bind
output (notes/design-persisted-frontend.md), so about 0.3 GiB more, against a worst measured peak of 11.4 GiB.

## Numbers

Leaves (`TSRS_FREE_LEAVES=stats`, 4 checkers, macOS; bytes used by the regions):

| project | checked files | file regions | their bytes | leaves | leaves' bytes |
| --- | --- | --- | --- | --- | --- |
| vscode | 9,399 | 9,399 | 794 MB | 2,337 | 319 MB |
| formbricks-web | 3,477 | 3,376 | 122 MB | 1,136 | 62 MB |
| supabase-studio | 5,318 | 4,823 | 131 MB | 727 | 30 MB |
| t3code-server | 1,518 | 1,347 | 128 MB | 518 | 68 MB |
| cal-diy | 3,136 | 907 | 25 MB | 206 | 6 MB |
| xstate-main | 248 | 248 | 17 MB | 129 | 13 MB |
| webpack | 1,551 | 0 (JavaScript) | | | |

Linux, 64-vCPU Depot runner, 32 checkers (`--noEmit --incremental false --extendedDiagnostics`), median of 5
interleaved runs (`depot ci run` of .depot/workflows/perf-probe.yml with a probe script that is not committed). `off`
is `TSRS_FREE_LEAVES=0`; `keep` makes the regions and frees nothing:

| project | maxrss main | maxrss branch | change | wall main | wall branch | Check time main | branch |
| --- | --- | --- | --- | --- | --- | --- | --- |
| vscode | 2,939 MB | 2,682 MB | -8.7% | 0.68 s | 0.74 s | 0.481 s | 0.517 s |
| formbricks-web | 3,222 MB | 3,183 MB | -1.2% | 0.73 s | 0.76 s | 0.546 s | 0.565 s |
| supabase-studio | 2,535 MB | 2,520 MB | -0.6% | 0.65 s | 0.67 s | 0.465 s | 0.484 s |
| t3code-server | 2,945 MB | 2,900 MB | -1.5% | 1.57 s | 1.61 s | 1.454 s | 1.491 s |
| cal-diy | 2,855 MB | 2,868 MB | +0.5% (noise) | 0.63 s | 0.64 s | 0.487 s | 0.499 s |

vscode by variant (medians): main 0.481 s check / 0.68 s wall; off 0.498 / 0.70; keep 0.509 / 0.73; freed but pages
kept (a throwaway switch) 0.511 / 0.72; on 0.517 / 0.74. The `off` row is main's code path (the same instruction count)
and shows the run-to-run spread. Giving pages back costs about 6 ms of check time (`on` against the switch that frees
but keeps the pages; each call flushes the TLB of every core running the process). The rest comes with the regions
themselves (`keep`): their memory is in 4 KiB pages where the AST used to be in the thread arenas' transparent huge
pages, which fits what was measured (dTLB load misses at 32 checkers 5.4 M without regions, 7.6 M with them; the time
from the end of the run to the process exit, mostly tearing down the address space, 28 ms on main and 43-47 ms with
regions) but was not isolated further. Two earlier costs are gone: the leaf test's 110k import lookups ran on one
thread inside the check time (24 ms on vscode; now on the worker pool), and one `madvise` per chunk (4,691 on vscode)
cost 2.6% of the check at 32 checkers with 4x the dTLB misses (now about 950 calls, coalesced).

macOS, M5 Max (18 cores; the machine was shared, load average 6-74, so the times are not comparable), peak memory
footprint (`/usr/bin/time -l`), median of 3 interleaved runs:

| project | checkers | main | branch | change |
| --- | --- | --- | --- | --- |
| vscode | 4 | 2,186 MB | 1,908 MB | -12.7% |
| vscode | 16 | 2,519 MB | 2,255 MB | -10.5% |
| formbricks-web | 4 | 1,740 MB | 1,700 MB | -2.3% |
| formbricks-web | 16 | 2,399 MB | 2,357 MB | -1.8% |
| supabase-studio | 4 | 1,275 MB | 1,268 MB | -0.6% |
| supabase-studio | 16 | 1,872 MB | 1,869 MB | -0.2% |
| t3code-server | 4 | 1,288 MB | 1,239 MB | -3.8% |
| t3code-server | 16 | 2,302 MB | 2,254 MB | -2.1% |
| cal-diy | 4 | 1,212 MB | 1,222 MB | +0.8% (ranges overlap) |
| cal-diy | 16 | 2,042 MB | 2,045 MB | +0.2% |

Single-threaded instructions (Linux, bench/count.py, deterministic, two runs each): vscode main 114.611 G, branch
114.856 G (+0.21%), `TSRS_FREE_LEAVES=0` 114.604 G; t3code-server 57.914 G, 57.976 G (+0.11%), 57.908 G. Peak RSS
single-threaded: vscode 2,088 -> 1,801 MB, t3code-server 901 -> 837 MB. On macOS (`/usr/bin/time -l` instructions
retired, `--singleThreaded`, three runs) vscode main 111.27 / 111.81 / 112.72 G, branch 111.87 / 112.43 / 112.96 G;
footprint 1,961 -> 1,693 MB.

Alloc-profile build, vscode, 4 checkers: arena requested 1,615.6 -> 1,615.3 MB (the same objects); thread-arena chunks
1,903 -> 871 MB plus 1,050 MB of file regions (794 MB used); heap peak 462.4 -> 461.9 MB.

What the regions alone cost (`keep` against `off`): about 1 KB of bookkeeping per region, and on macOS 15 MB of
footprint on vscode at 4 checkers. Projects whose leaves are a small share of their TypeScript (cal-diy, supabase-studio)
end up within noise of main in memory and pay 2-4% of check time at 32 checkers.

## Gates

- Output: stdout byte-identical to main (`cmp`, `--pretty false`) on vscode (371 errors), webpack (840), xstate-main
  (0), cal-diy (136), formbricks-web (0), supabase-studio (9), t3code-server (6) at 1, 4 and 16 checkers: 21 runs with
  the default switch and 21 with `TSRS_ARENA_POISON=1 TSRS_FREE_LEAVES=stats`, all identical, no crash. On Linux at 32
  checkers: on, off, poisoned and main identical for the five application projects. vscode `--pretty true`,
  `--listFiles` and the `--extendedDiagnostics` counters (single-threaded) identical with poisoned freed leaves.
- Conformance (`tsrs-test run --suite all`, 15,197 variants): 13,458 pass on main and the branch; with leaf freeing
  forced on in the runner (a temporary override, not committed: fresh, region-parsed files in programs whose trees
  nothing reads after the pass, canonical history) 13,458 pass in each of one checker, one checker poisoned, four
  checkers, four checkers poisoned; `comm -23` of the pass lists empty in every case; 2,339 leaves freed per run.
  `.github/scripts/conformance-gate.sh` in both CI modes (Go's history; default checker mode): errors 13,458,
  `.types` / `.symbols` 12,779 / 12,779 and 12,778 / 12,778.
- `cargo check --workspace` (no warnings), `tools/lint/ratchet.py`, `tools/lint/source.py`, `cargo test -p tsrs_core`
  (new: retired ranges are never reused; neighbouring retired chunks give back their shared page),
  `cargo test -p tsrs_compiler`, `cargo test -p tsrs_cli --test free_leaf_files --test derived_variance --test
  default_emit --test flow_memo`, tools/regressions.sh (20 cases). The tsrs_cli binary's own unit tests do not link
  on macOS (`malloc_trim`, before this change too).

## Not covered

- Runs that emit (JavaScript or declarations), `--explainFiles`, `--incremental` / `composite`, `--build`, the
  language server, the native API and the test harnesses: the trees are read after the check, or programs are reused.
- JavaScript, declaration and JSON files: never leaves.
- Files that re-export (barrel files) or that another file imports: by definition not leaves. Freeing a file after
  its last importer is checked would need the importers' checkers to be done with it, which they never are.
- The reachability census cannot check this: checker caches keep stale entries keyed by a freed leaf's nodes and
  symbols (never read), which it reports. File regions are off under `TSRS_CENSUS=1`; poison runs are the gate.
- On Linux a stray read of a freed leaf reads zeros (`MADV_DONTNEED`) instead of faulting.
- The pages a leaf shares with a live neighbouring chunk stay.
- The regions' 4 KiB pages: putting region slabs in huge pages would split them on every partial discard, and a
  split huge page is only freed when the kernel's deferred split runs.
