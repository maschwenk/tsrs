# mem-per-checker-duplication: what each extra checker repeats on the Zod and Effect projects

Question (2026-10-07): at each tool's default on a 64-vCPU machine tsrs peaks at 1.37x / 1.43x / 1.17x / 1.91x of `bun
check`'s memory on cal-diy, formbricks-web, supabase-studio and t3code-server, but 1.01x on vscode. Single-threaded the
gap is small (t3code 871 vs 802 MiB), so the excess is per-checker growth. This note measures what an extra checker
holds, who declared it (lib, node_modules packages, workspace packages, the project), what sharing only the read-mostly
library part would save and risk, and what a per-project default checker count would trade. Measurement only: no
production code changes; the instrumentation below lived in an uncommitted worktree.

Base: origin/main e4f82c0 (tsrs 1cbf079, `0.5.0`), release build unless named. macOS, Apple M5 Max (18 cores), 16 KiB
pages. Flags `--noEmit --incremental false --pretty false --extendedDiagnostics --checkers N`, run from each checkout
root under bench/.work/solutions (t3code-server `-p apps/server`, formbricks-web `-p apps/web/tsconfig.typecheck.json`,
supabase-studio `-p apps/studio`, cal-diy `-p apps/web`, vscode `-p src`). Peak = `/usr/bin/time -l` peak memory
footprint. MB and MiB are 2^20 bytes throughout (the alloc profile and heap census print 2^20 as "MB"). The machine was
shared: 1-minute load 13-31 during every run, so Mac wall times are only used qualitatively; memory, instruction and
object counts are not affected by load.

Result in one paragraph: an extra checker adds 33-42 MiB at 16 -> 32 checkers on the four app projects (17 on vscode)
and 73-145 MiB at 1 -> 8. About 62-69% of it is the checker's own type graph, still reachable at exit (types, checker
symbols and their link slots, signatures, member tables); the rest is inference scratch that goes back to free lists and
8-14% garbage, more than half of it on formbricks one dead scratch structure (`ExportCollision`, an exact fix, spawned
as a separate task). By declaration: on t3code 54% of the extra types and 50% of the extra symbols are Effect (and lib)
generics instantiated with project types, which no read-mostly share can hold; on the Zod/React projects 35-40% of the
extra types and 56-68% of the extra symbols are library-only (mostly zod and @types/react declarations). A perfect share
of the read-mostly part (lib and node_modules types and members built only from library declarations) would save 267-347
MiB at 32 checkers on the app projects (9.5-12.9% of the peak; 83 MiB, 3.0%, on vscode), cannot be exact (type ids and
lazy state), and is a months-long change with high fidelity risk. The cheap lever is the checker count: on t3code the
check is bound by one ~1.2 s file at 16+ checkers, so `min(32, checked files / 100)` gives it 15 checkers for -0.55 GiB
(-19%) at a modeled +0.00 s and changes nothing on the other four; 16 checkers everywhere saves 0.5-0.66 GiB per app
project but costs a modeled +0.02 s (formbricks) to +0.17 s (supabase).

## 1. Peak memory by checker count (macOS)

Peak footprint, MiB, median of 2-8 runs per cell (1: 4, 2: 2, 4: 7, 8: 8, 12: 5, 16: 8, 24: 5, 32: 8, 64: 2 runs); the
widest cell spread is 76 MiB (cal-diy at 32; stealing changes which files share a checker), 34 of the 45 cells are under
30.

| project | `--noCheck` | 1 | 2 | 4 | 8 | 12 | 16 | 24 | 32 | 64 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| t3code-server | 442 | 871 | 1,042 | 1,286 | 1,887 | 2,121 | 2,301 | 2,609 | 2,824 | 3,621 |
| formbricks-web | 989 | 1,397 | 1,525 | 1,742 | 2,020 | 2,191 | 2,396 | 2,724 | 3,067 | 4,131 |
| supabase-studio | 626 | 990 | 1,139 | 1,275 | 1,499 | 1,708 | 1,875 | 2,159 | 2,396 | 3,446 |
| cal-diy | 489 | 848 | 1,005 | 1,226 | 1,535 | 1,781 | 2,024 | 2,444 | 2,685 | 3,750 |
| vscode | 1,205 | 1,982 | 2,075 | 2,178 | 2,322 | 2,451 | 2,523 | 2,678 | 2,800 | 3,192 |

MiB per extra checker on each segment:

| project | 1 -> 4 | 4 -> 8 | 8 -> 16 | 16 -> 32 | 32 -> 64 | 1 -> 32 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| t3code-server | 139 | 150 | 52 | 33 | 25 | 63.0 |
| formbricks-web | 115 | 70 | 47 | 42 | 33 | 53.9 |
| supabase-studio | 95 | 56 | 47 | 33 | 33 | 45.3 |
| cal-diy | 126 | 77 | 61 | 41 | 33 | 59.3 |
| vscode | 65 | 36 | 25 | 17 | 12 | 26.4 |

The slope falls with the count: a new checker re-resolves the library working set its files need, and at small counts
each checker's files need most of it. The first checker costs 359-429 MiB on the app projects (777 on vscode) over a
front end of 442-989 MiB.

The same in work: checker instructions (process instructions minus the `--noCheck` run; G, Mac) grow by S = 5.32 / 2.22
/ 2.76 / 2.72 / 0.87 G per extra checker (1 -> 32) on t3code / formbricks / supabase / cal-diy / vscode against a
single-checker check of 46.9 / 34.4 / 49.9 / 30.6 / 85.9 G. At 32 checkers 78% / 67% / 63% / 73% / 24% of all checker
instructions repeat work another checker also did.

## 2. What an extra checker's memory is

### 2a. Arena, heap, rest (`--features alloc-profile`, 1 vs 16 checkers)

"Arena used" is the arena chunks minus their untouched tails (resident on macOS); "requested" counts every allocation,
including blocks handed out again from free lists (mappers, inference contexts, type lists; notes/mem-recycle.md), so it
overstates. Heap = the counting allocator's live heap. MiB per extra checker, (16-checker value - 1-checker value) / 15:

| project | arena used | heap | rest (footprint - both) | footprint (alloc-profile build) | arena requested | of which reissued from free lists |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| t3code-server | 67.7 | 22.1 | 4.3 | 94.1 | 95.6 | 27.9 |
| formbricks-web | 36.2 | 22.7 | 7.2 | 66.1 | 43.1 | 6.9 |
| supabase-studio | 32.6 | 20.1 | 6.6 | 59.3 | 35.3 | 2.7 |
| cal-diy | 43.5 | 28.1 | 7.3 | 78.9 | 47.0 | 3.5 |
| vscode | 19.9 | 10.2 | 5.8 | 35.9 | 20.3 | 0.3 |

### 2b. Arena by object kind (requested MiB per extra checker, 1 -> 16)

| arena rows | t3code-server | formbricks-web | supabase-studio | cal-diy | vscode |
| --- | ---: | ---: | ---: | ---: | ---: |
| `Symbol` (checker-created) | 9.5 | 4.6 | 5.1 | 7.1 | 2.3 |
| value-symbol link slots + `IdGroup` | 7.5 | 3.6 | 4.1 | 5.0 | 4.3 |
| `TypeMapper` (reissued in part) | 11.5 | 6.5 | 3.8 | 5.8 | 0.9 |
| `[P<Type>]` lists (reissued in part) | 6.5 | 2.0 | 2.0 | 2.7 | 0.7 |
| inference contexts, infos, candidate lists (mostly reissued) | 23.7 | 3.5 | 0.9 | 1.0 | 0.3 |
| `TypeReference` | 5.6 | 1.8 | 1.0 | 1.3 | 0.6 |
| `ObjectType` | 2.0 | 1.3 | 1.3 | 1.4 | 1.0 |
| `LazyMemberTable` | 5.9 | 1.5 | 0.7 | 0.8 | 0.5 |
| resolved members (`StructuredMembers`, `SymbolTable`, `[P<Symbol>]`, `[&str]`) | 7.0 | 3.0 | 4.1 | 5.1 | 2.2 |
| `Signature` | 4.4 | 0.9 | 1.1 | 1.4 | 1.0 |
| mapped, conditional, type-parameter types | 3.6 | 3.0 | 1.8 | 4.2 | 0.6 |
| union, intersection, literal types | 2.4 | 2.3 | 2.8 | 3.3 | 1.6 |
| other link slot pages | 2.6 | 1.4 | 1.9 | 1.4 | 1.3 |
| `ExportCollision` (dead scratch, 2e) | 0.4 | 4.3 | 1.0 | 2.5 | 0.0 |
| rest | 1.2 | 1.5 | 1.7 | 2.4 | 0.9 |
| total of the listed (top-60) rows | 93.8 | 41.1 | 33.2 | 45.5 | 18.2 |

Every AST row is flat or slightly negative (shared). t3code is the outlier in inference: 6.2M `InferenceInfo` and 1.8M
`InferenceContext` allocations at 16 checkers (1.3M and 0.5M at 1), Effect's generic pipelines inferred again in every
checker.

### 2c. Reachable, reissued, garbage (reachability census, t3code and formbricks)

`--features alloc-profile,tsrs_core/plain-ptrs`, `TSRS_CENSUS=1` at 1 and 16 checkers. In this build nothing is reissued
and pointers are 8 bytes, so bytes are larger than in 2a; read the shares. Per extra checker:

| project | allocated | reachable at exit | freed to free lists (reissued in a normal run) | unreachable, never freed |
| --- | ---: | ---: | ---: | ---: |
| t3code-server | 125.5 MiB | 78.2 (62%) | 37.5 (30%) | 9.9 (8%) |
| formbricks-web | 55.9 MiB | 38.7 (69%) | 9.4 (17%) | 7.8 (14%) |

Reachable growth on t3code by row (MiB per extra checker): `Symbol` 11.5, value-symbol link slots 7.9, `TypeReference`
7.6, `[P<Type>]` 7.0, `LazyMemberTable` 5.6, `Signature` 5.2, `TypeMapper` 3.8, `[P<Symbol>]` 3.1, `TypeParameter` 2.7,
`MappedType` 2.4, `ObjectType` 2.0, `StructuredMembers` 2.0. The inference rows are 95-97% unreachable and most of that
is on free lists (`InferenceInfo` 15.0 allocated, 0.5 reachable). So what an extra checker keeps is its own copy of the
type graph; freeing more scratch would not change the picture.

### 2d. Heap containers each checker owns (`TSRS_HEAP_CENSUS=1`, release)

Total over the checkers, 1 -> 16: t3code 70 -> 307 MiB (15.8 per extra checker), formbricks 77 -> 215 (9.2), supabase 68
-> 205 (9.2), cal-diy 60 -> 201 (9.4), vscode 160 -> 261 (6.7). The rows that grow (MiB per extra checker, t3code /
formbricks / supabase / cal-diy / vscode): object-type instantiation maps 1.96 / 1.49 / 1.09 / 1.45 / 0.67, relation
assignable 1.36 / 0.66 / 0.57 / 1.05 / 0.50, value-symbol link groups 1.35 / - / 0.41 / 0.64 / 1.13, mapped-symbol links
1.27 / 0.59 / 0.87 / 0.43 / - (- = below the census's 1 MiB row threshold). The instantiation caches as containers are
1-2 MiB per checker; their cost is the types they point to (2b). The rest of the per-checker heap in 2a (6-12 MiB) is
containers owned by arena objects (symbol tables of resolved members, inference candidate vectors, property caches).

### 2e. `ExportCollision`: dead scratch on every `export *` (an exact fix)

`extend_export_symbols` (crates/tsrs_checker/src/checker_08.rs:1407-1430) allocates an arena `ExportCollision` (64 B)
plus a heap `String` of the module specifier for every name re-exported through `export *`, into a function-local table
(checker_08.rs:1328) that is dropped when the export resolution returns; arena objects are never dropped, so both leak.
formbricks: 115,688 objects (7.1 MiB) at 1 checker, 1,161,656 (70.9 MiB) at 16, 100% unreachable, plus 1.23M specifier
strings (24.0 MiB at 16, 99.9% unreachable, all from `extend_export_symbols`); cal-diy 63,768 -> 679,752 (3.9 -> 41.5
MiB), supabase 81,384 -> 327,216 (5.0 -> 20.0 MiB). Estimated at 32 checkers: ~187 MiB on formbricks (6% of its peak),
~109 on cal-diy, ~53 on supabase (objects per checker at 16 x 32 x 84 B: 64 B arena + formbricks' mean specifier string
of 20 B). Keeping the scratch local frees it with no output change. Spawned as its own task; not done here.

## 3. Who declared it

Temporary instrumentation (not committed): `TSRS_ASSIGNMENT_STATS=classes` in
crates/tsrs_compiler/src/checkerpool_stats.rs (`--features assignment-stats`, which records every type and symbol a
checker creates). Each created type is classed by the declaration of its alias symbol or symbol (lib, node_modules,
workspace = outside the tsconfig's directory and not in node_modules, project = inside it, none) and by whether
everything it is built from is declared in lib or node_modules or has no declaration ("library-only"): alias type
arguments, reference type arguments (or the deferred node's file), mapper targets (inference and deferred mappers count
as project-involved: 23K such mapper visits at 1 checker and 55K summed over 16 on t3code, 10-12K on vscode, under 3.3K
on the others), union / intersection / template constituents, index / indexed-access / substitution / string-mapping
components, conditional roots, reverse-mapped sources. The walk is memoized, cycles optimistic. A checker-created symbol
is classed by its declaration and, if instantiated, by its mapper or containing type. Totals match
`--extendedDiagnostics` (t3code at 1 checker: 1,443,271 types). Per extra checker = (sum over 16 checkers - single
checker) / 15, so a class's growth is what the checkers create again.

Types per extra checker, thousands (share):

| class | t3code-server | formbricks-web | supabase-studio | cal-diy | vscode |
| --- | ---: | ---: | ---: | ---: | ---: |
| lib declared | 1.1 (0%) | 2.0 (1%) | 1.9 (1%) | 2.1 (1%) | 3.1 (3%) |
| lib instantiated, library-only | 13.5 (5%) | 10.5 (6%) | 10.0 (7%) | 8.6 (4%) | 10.9 (12%) |
| node_modules declared | 9.3 (3%) | 5.9 (3%) | 6.0 (4%) | 4.4 (2%) | 1.2 (1%) |
| node_modules instantiated, library-only | 33.5 (12%) | 50.6 (29%) | 33.0 (23%) | 57.6 (28%) | 2.8 (3%) |
| no declaration, library-only (unions, literals, tuples, ...) | 31.2 (11%) | 39.8 (23%) | 39.2 (27%) | 70.3 (34%) | 29.4 (32%) |
| lib / node_modules generic, project arguments | 156.3 (54%) | 31.3 (18%) | 27.6 (19%) | 38.0 (18%) | 7.5 (8%) |
| no declaration, project-involved | 23.2 (8%) | 20.4 (12%) | 15.2 (11%) | 14.9 (7%) | 10.6 (11%) |
| workspace packages | 8.0 (3%) | 11.5 (7%) | 5.8 (4%) | 10.1 (5%) | 0.0 |
| project | 11.2 (4%) | 1.3 (1%) | 3.9 (3%) | 0.3 (0%) | 26.9 (29%) |
| total | 287.3 | 173.3 | 142.5 | 206.1 | 92.5 |

Checker-created symbols per extra checker, thousands (share):

| class | t3code-server | formbricks-web | supabase-studio | cal-diy | vscode |
| --- | ---: | ---: | ---: | ---: | ---: |
| lib declared or instantiated, library-only | 10.4 (3%) | 13.4 (9%) | 12.3 (7%) | 15.3 (7%) | 16.7 (22%) |
| node_modules declared or instantiated, library-only | 46.2 (15%) | 90.0 (59%) | 92.0 (55%) | 111.1 (49%) | 5.2 (7%) |
| no declaration, library-only | 1.3 (0%) | 2.1 (1%) | 1.6 (1%) | 2.0 (1%) | 0.1 (0%) |
| lib / node_modules generic, project arguments | 157.2 (50%) | 17.6 (12%) | 40.5 (24%) | 57.3 (25%) | 10.0 (13%) |
| no declaration, project-involved | 5.4 (2%) | 2.5 (2%) | 2.7 (2%) | 10.0 (4%) | 7.3 (10%) |
| workspace packages | 71.6 (23%) | 20.4 (13%) | 11.6 (7%) | 29.8 (13%) | 0.0 |
| project | 23.1 (7%) | 5.2 (3%) | 6.0 (4%) | 0.2 (0%) | 35.8 (48%) |
| total | 315.2 | 151.3 | 166.7 | 225.6 | 75.2 |

By declaring package (objects with a declaration, types + symbols per extra checker; library-only / project-involved):

| project | top packages |
| --- | --- |
| t3code-server | effect 350K (65%: 83K / 268K), packages/contracts 71K (13%, workspace), lib 71K (13%: 25K / 46K), project 34K |
| formbricks-web | zod 92K (35%: 77K / 15K), lib 37K (14%), @types/react 36K (14%: 34K / 2K), @prisma/client 24K (9%: 7K / 17K), packages/database 23K (workspace) |
| supabase-studio | @types/react 77K (31%: 64K / 14K), zod 43K (17%: 32K / 11K), lib 42K (17%), @tanstack/query-core 15K |
| cal-diy | @types/react 118K (35%: 84K / 34K), zod 59K (18%: 29K / 29K), lib 37K (11%), react-hook-form 22K, @prisma/client 21K |
| vscode | project 63K (52%), lib 48K (40%), zod 5K |

So the Effect project duplicates mostly Effect-declared generics instantiated with its own types (the services and
schemas of the server, judging by the declaring packages; individual types were not listed), while the Zod/React
projects duplicate mostly zod and @types/react instantiations whose arguments are library types too.

The existing per-file report (`TSRS_ASSIGNMENT_STATS=1`, 16 checkers; work = linked nodes + symbols per file) agrees:
work duplication 3.04x / 2.36x / 2.74x / 3.38x / 1.56x; the work on lib files is repeated 9.4-12.3x over the 16
checkers, on node_modules files 3.7-7.0x, on the checkout's own files 1.2-2.4x (that report's "project" is the current
directory, so it includes workspace packages).

## 4. What sharing only the read-mostly part would save

The read-mostly candidate (notes/perf-shared-checker.md's "library declared" plus "library instantiated, library
arguments"): types and symbols declared in lib or node_modules and built only from library declarations, i.e. the lib
and node_modules library-only rows of the tables above. Bytes are estimated: the type-side arena growth (types and
members + signatures + 42% of mappers and type lists, the reachable fraction the census found on t3code and formbricks)
divided by the type growth, and the symbol-side growth (symbols + value link slots) by the symbol growth. t3code: (27.7
+ 4.7 + 0.42 x 18.0) MiB / 287,315 types = 146 B per type, 17.6 MiB / 315,245 symbols = 59 B per symbol. A perfect share
keeps one copy, so it saves the class's growth over the single checker; at 32 checkers on t3code that is 1,376,752 types
x 146 B + 1,350,395 symbols x 59 B = 268 MiB.

| project | B / type, B / symbol | read-mostly share of extra types / symbols | saved at 16 (of Mac peak) | saved at 32 (of Mac peak) | also sharing library-only unions, literals (16 / 32) | not read-mostly: library generics with project arguments (16 / 32) |
| --- | --- | --- | ---: | ---: | --- | --- |
| t3code-server | 146 / 59 | 20% / 18% | 168 MiB (7.3%) | 267 MiB (9.5%) | +66 / +101 MiB | 459 / 597 MiB |
| formbricks-web | 113 / 59 | 40% / 68% | 199 MiB (8.3%) | 347 MiB (11.3%) | +66 / +113 | 66 / 98 |
| supabase-studio | 117 / 61 | 36% / 63% | 176 MiB (9.4%) | 289 MiB (12.1%) | +67 / +113 | 81 / 114 |
| cal-diy | 117 / 58 | 35% / 56% | 226 MiB (11.0%) | 346 MiB (12.9%) | +119 / +153 | 111 / 166 |
| vscode | 106 / 92 | 20% / 29% | 56 MiB (2.2%) | 83 MiB (3.0%) | +45 / +65 | 25 / 31 |

These are ceilings: one copy is assumed to cover every checker (the single checker's count as the distinct set; not
checked with fingerprints here), sharing is assumed free, and the declared part alone ("lib / node_modules declared", no
instantiations) is 3-6% of the extra types. On t3code the 54%-of-types class that Bun's design does share (library
generics with project arguments) is out of reach for a read-mostly layer: it is project-specific by construction.

Why it cannot be exact (notes/mem-shared-base.md section 2b, notes/perf-shared-checker.md section 3; line numbers at
e4f82c0):

- Type ids are per checker: `TypeId(self.type_count)` (crates/tsrs_checker/src/checker_12.rs:1899). Every instantiation
  cache key is built from argument ids: `get_type_list_key` (checker_09.rs:550) for `InterfaceType.instantiations`
  (types.rs:2189), `Checker::object_type_instantiations` (checker.rs:1069), `TypeAliasLinks.instantiations`
  (types.rs:533) and `ConditionalRoot.instantiations` (types.rs:2703). Sharing a cache entry means sharing its argument
  and result types, so a library-only instantiation cache is not separable from a shared type identity.
- Ids decide output in two places: `compare_types` falls back to the id when flags, symbols and payload tie
  (utilities.rs:768-769; union constituent order), and `is_deeply_nested_type` counts only occurrences with increasing
  ids (relater_1.rs:1080-1091; the recursion cutoff, relation results, TS2589-style errors). Ids taken from a shared
  counter would depend on which thread created a type first. Deterministic ids (Bun numbers new records at a barrier,
  section 6) fix run-to-run variation but put every shared type before every local one, an order a single tsgo checker
  never produces, so the two id-dependent paths can still differ from tsgo. tsrs output is already independent of the
  file order inside a checker (random-seed assignments print the same text, docs/DEBUGGING.md "Checker assignment"),
  which is evidence the exposure is small, not that it is zero.
- Library types are not frozen after creation: members, base constraints, apparent types, property caches, lazy member
  tables and the instantiation tables of generic targets are filled by whoever asks first, and every checker writes the
  same targets' tables with project arguments. A share needs either synchronized mutation of all of it (297
  interior-mutable arena fields and 860 `Cell` writes counted in perf-shared-checker.md section 4) or an eagerly
  resolved frozen layer, which is extra work and, built before checking, sits on the critical path.
- Lazy resolution belongs to a checker: circularity stacks, instantiation depth counters and `current_node` decide which
  diagnostic is reported where; a shared resolution would report once where tsgo reports per checker.

Effort and risk, my estimate: months, high fidelity risk (the type identity and lazy-state model of the checker change),
for at most 9.5-12.9% of the 32-checker peak on these projects, 13-19% if union and literal interning is shared too
(that is the full design of perf-shared-checker.md section 4). The checker count (section 5) moves as much memory on the
app projects for a few lines of code, at a speed cost on some of them.

## 5. The cheap alternative: a per-project default checker count

The default is `clamp(cores / 2, 4, 32)` with at most one checker per 32 checked files
(crates/tsrs_compiler/src/checkerpool.rs:1135-1148); on a 64-vCPU machine it is 32 on all five projects.

### 5a. Speed: what is measured

64-vCPU Linux (Depot `depot-ubuntu-24.04-64`), median of the three bench results files at 1cbf079, bd86c6d, ce6a7dc
(bench/results/2026-10-07-*.json, modes `wide` = default 32 and `checkers64`); 8-vCPU `default` = 4 checkers on an
8-vCPU machine of the same CPU; vscode at 4 / 8 / 16 from the head-to-head
(bench/results/compare/2026-10-07-b05f05bc6e3f-64t.md, mean of 20: 2.49 s 2.23 GiB / 1.39 s 2.37 GiB / 0.86 s 2.57 GiB):

| project | tsrs 32: wall, check, RSS | tsrs 64: wall, check, RSS | bun check (64 threads): wall, RSS | tsrs 4 on 8 vCPU: check |
| --- | --- | --- | --- | --- |
| t3code-server | 1.36 s, 1.25 s, 2.89 GiB | 1.33 s, 1.21 s, 3.71 GiB | 2.40 s, 1.52 GiB | 1.86 s |
| formbricks-web | 0.66, 0.48, 3.15 | 0.61, 0.42, 4.24 | 0.92, 2.20 | 1.24 |
| supabase-studio | 0.56, 0.40, 2.47 | 0.52, 0.34, 3.54 | 1.23, 2.10 | 1.49 |
| cal-diy | 0.56, 0.42, 2.75 | 0.52, 0.37, 3.84 | 1.32, 2.02 | 1.16 |
| vscode | 0.60, 0.41, 2.87 | 0.59, 0.38, 3.31 | 0.84, 2.87 | 2.52 |

t3code does not get faster past ~12 checkers. Its heaviest files cost 1.15 s (`apps/server/src/server.ts`) and 1.04 s
(`src/bincli.ts`) of checker CPU at 16 checkers on the Mac (`--checkerCostCache`), and a file is never split, so the
check is bound by one file; 32 -> 64 checkers on Linux buys 0.04 s for +0.82 GiB. On the loaded Mac the t3code check
time was flat from 4 to 24 checkers (1.52 / 1.49 / 1.46 / 1.41 / 1.44 s at 4 / 8 / 12 / 16 / 24, medians of 5).

### 5b. Speed at 8-24 checkers on 64 vCPU: modeled

No Linux runs at 8-24 checkers exist for the app projects. Model: check(N) = max(L, k x A(N)) + d, where A(N) = Mac
checker instructions per checker = (I(N) - I(`--noCheck`)) / N, L = the measured 64-checker check time (the single-file
floor), k = 0.913 x (8-vCPU 4-checker check) / A(4) (0.913 = vscode's 64-vCPU / 8-vCPU ratio at 4 checkers), and d =
measured check(32) - max(L, k x A(32)) (0.03-0.06 s). Validation on vscode, the only project with measured 8 and 16: 16
checkers 0.68 s modeled vs 0.67 measured, 8 checkers 1.23 vs 1.20. Memory: Linux RSS(N) = RSS(32) - (Mac(32) - Mac(N));
on vscode this gives 2.26 / 2.40 / 2.60 GiB at 4 / 8 / 16 against 2.23 / 2.37 / 2.57 measured. All cells below are
modeled except the vscode rows marked measured.

Change against the 64-vCPU default of 32 (check time, wall change in % of the measured 32-checker wall; RSS):

| checkers | t3code-server | formbricks-web | supabase-studio | cal-diy | vscode |
| --- | --- | --- | --- | --- | --- |
| 24 | +0.00 s, -0.21 GiB | +0.00 s, -0.34 GiB | +0.04 s (+8%), -0.23 GiB | +0.00 s, -0.24 GiB | +0.07 s (+12%), -0.12 GiB |
| 16 | +0.00 s, -0.51 GiB (2.38) | +0.02 s (+3%), -0.66 GiB (2.49) | +0.17 s (+30%), -0.51 GiB (1.96) | +0.10 s (+17%), -0.65 GiB (2.10) | +0.27 s (+44%), -0.27 GiB; measured +0.26 s, -0.30 GiB |
| 12 | +0.00 s, -0.69 GiB | +0.10 s (+15%), -0.86 GiB | +0.26 s (+47%), -0.67 GiB | +0.17 s (+30%), -0.88 GiB | +0.46 s (+76%), -0.34 GiB |
| 8 | +0.24 s (+18%), -0.91 GiB | +0.28 s (+43%), -1.02 GiB | +0.45 s (+80%), -0.88 GiB | +0.34 s (+60%), -1.12 GiB | +0.82 s (+137%), -0.47 GiB; measured +0.79 s, -0.50 GiB |

### 5c. Rules

`clamp(cores / 2, 4, min(32, checked_files / K))`, checked source files 1,522 / 3,547 / 5,324 / 3,346 / 9,415 (the sum
of the checkers' own source files in `TSRS_ASSIGNMENT_STATS=1`; skipLibCheck declaration files carry no weight), 64
vCPUs:

| K | t3code-server | formbricks-web | supabase-studio | cal-diy | vscode |
| --- | --- | --- | --- | --- | --- |
| 32 (today) | 32 | 32 | 32 | 32 | 32 |
| 100 | 15: +0.00 s, 2.34 GiB (-0.55, -19%) | 32 | 32 | 32 | 32 |
| 150 | 10: +0.03 s (+2%), 2.09 GiB (-0.80, -28%) | 23: +0.00 s, 2.77 GiB (-0.38) | 32 | 22: +0.02 s (+3%), 2.41 GiB (-0.34) | 32 |
| 200 | 7: +0.28 s (+20%), 1.83 GiB (-1.06) | 17: +0.01 s (+1%), 2.53 GiB (-0.62) | 26: +0.02 s (+4%), 2.30 GiB (-0.17) | 16: +0.10 s (+17%), 2.10 GiB (-0.65) | 32 |

Against bun at its default, K = 100 leaves t3code 1.76x faster at 1.54x its memory (today 1.76x at 1.90x); K = 150 gives
t3code 1.73x / 1.37x, formbricks 1.39x / 1.26x (today 1.39x / 1.43x), cal-diy 2.29x / 1.19x (2.36x / 1.36x).

The file-count rule works on t3code because its files are few and one of them is the floor, not because of anything it
measures. Two rules that measure the cause, not evaluated: cap the count at total checked weight / heaviest file weight
(the pool already computes static weights; whether they find server.ts was not checked), or, with `--checkerCostCache`,
at total measured cost / heaviest file cost. A duplication break-even N = (single-checker work) / (S per extra checker),
where an extra checker repeats as much as it adds, is 8.8 / 15.5 / 18.1 / 11.2 / 98 on the five projects; using it as
the cap gives speed away (cal-diy at 11: about +0.2 s), so it is not proposed.

Within this note's evidence, the trade that costs no measured or modeled speed is K = 100 (t3code only, -0.55 GiB).
Every step further trades tenths of a second on formbricks, cal-diy and supabase for 0.2-0.7 GiB each.

## 6. What Bun does

From Bun's source (oven-sh/bun PR 44361, `src/sema`, read for this note; paths relative to it):

- **Sharing: all of it, not just the read-mostly part.** One type store per program, shared by all threads, read-only
  during a step and written only at the barrier (check/mod.rs:4-5, 189; types.rs:1554-1556). Types, signatures and
  mappers are hash-consed, "equal ones have equal ids" (types.rs:1-4); a task looks in its own records, then the
  published shards without locks, else creates a task-local record (types.rs:1493-1529). Instantiations are one shared
  table keyed by (type, mapper) (check/mod.rs:270) behind a 16K-entry per-checker cache (check/instantiate.rs:7-60).
  Members are a shared shape plus a mapper; no per-instantiation symbols (check/shape.rs:142-191, types.rs:515-557), so
  the per-instantiation `Symbol` and value-link rows of 2b have no counterpart there.
- **Determinism by barrier numbering, not tsgo identity.** New records are numbered at the barrier by (task, index in
  the task) (types.rs:1560-1562) after merging equal content hashes (types.rs:3352, 3703-3793). Union ordering falls
  back to that creation order and unions that compared a task-local id are re-sorted at the barrier
  (check/unions.rs:2124-2206, types.rs:4034-4065); `isDeeplyNestedType` uses the same order, which can differ from a
  single tsc checker (check/relate.rs:3956-3992); diagnostic text can differ from tsc's (check/sink.rs:10-12). This is
  the "deterministic ids" option of section 4, accepted with its tsc divergence.
- **Library resolution is still repeated, briefly.** "Every task resolves nodes of the library for itself" until it is
  published (check/mod.rs:1716-1719); warm-up steps of 1, 8 and 64 single-file tasks exist to publish it early
  (driver/lib.rs:173-188, 237-250).
- **Scratch is per task and freed.** A task's own records and buffers are dropped when it finishes; the last step
  publishes nothing but diagnostics (check/task.rs:95-108, 182-237).
- **No per-project thread count.** One thread per core by default (driver/lib.rs:837-840), independent of project size;
  the work plan is a function of the program, not of the thread count (driver/lib.rs:191-195). `--checkers=N` gives
  tsgo's share-nothing pool and is off by default (driver/lib.rs:166, 203-222).

So of the three options here Bun does the full share (more than section 4's read-mostly part, with determinism that is
not tsgo's) and frees scratch; it does not size its thread count per project.

## 7. Not measured / inferred

- No Linux run at 8-24 checkers for the app projects: every 64-vCPU cell in 5b and 5c except vscode's is the model
  (validated on vscode only) or the memory difference method (validated on vscode within 0.04 GiB). The single-file
  floor L is a measured 64-checker check time; that it is one file on t3code rests on the Mac cost cache.
- Mac wall times ran at load 13-31 on 18 cores and are not used for any number in the tables; instructions, memory and
  object counts are load-independent.
- Bytes per class (section 4) are estimates: counts x mean bytes per object of the arena rows assigned to types or
  symbols, with mappers and type lists at the census's 42% reachable fraction; heap containers that follow the objects
  (instantiation and relation maps, 1-3 MiB per checker) are not added.
- The library-only test treats inference and deferred mappers as project-involved and function mappers as library-only;
  the read-mostly ceiling assumes the single-checker set is the distinct set. The fingerprint census of
  notes/mem-shared-base.md (commit 9bed25d, branch mem/shared-base) was not ported to current main, so no
  structural-identity duplication check backs the ceiling.
- The census runs (2c) use plain 8-byte pointers and no reuse; only their shares are used.
- `ExportCollision` at 32 checkers is extrapolated from 16 (2e).
- Bun's per-thread memory on these projects was not measured here; section 6 is a source reading.
- None of the rules in 5c was implemented or run; their cells are the model of 5b.

## Reproduce

```sh
cargo build --release --locked -p tsrs_cli
CARGO_TARGET_DIR=target/alloc-profile cargo build --release --locked -p tsrs_cli --features alloc-profile
CARGO_TARGET_DIR=target/census cargo build --release --locked -p tsrs_cli --features alloc-profile,tsrs_core/plain-ptrs
CARGO_TARGET_DIR=target/astats cargo build --release --locked -p tsrs_cli --features assignment-stats
cd bench/.work/solutions/<project>
/usr/bin/time -l tsrs -p <tsconfig> --noEmit --incremental false --pretty false --extendedDiagnostics --checkers N
TSRS_ALLOC_PROFILE_TOP=60 target/alloc-profile/release/tsrs ... --checkers {1,16}   # 2a, 2b
TSRS_CENSUS=1 TSRS_CENSUS_TSV=census.tsv target/census/release/tsrs ... --checkers {1,16}   # 2c
TSRS_HEAP_CENSUS=1 tsrs ... --checkers {1,16}   # 2d
TSRS_ASSIGNMENT_STATS=1 target/astats/release/tsrs ... --checkers 16   # section 3, per-file report
tsrs ... --checkers 16 --checkerCostCache costs.txt   # per-file checker CPU, 5a
```

Section 3's class tables need the temporary `TSRS_ASSIGNMENT_STATS=classes` report: a memoized walk over `stats_created`
in checkerpool_stats.rs as described in section 3 (about 420 lines, kept out of this PR).
