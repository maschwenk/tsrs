# mem-assignment: assigning files to checkers by locality

With 4 checkers every checker re-creates the types, symbols and instantiations that its files need from shared
declarations (libs, `node_modules`, workspace packages, shared project modules). This note quantifies that
duplication on the private monorepo and replaces the file-to-checker assignment to reduce it. Checker code is unchanged; only
`tsrs_compiler::checkerpool` decides differently which checker gets which file.

## Go's assignment is already import-graph based

Go's `createCheckers` (ts-ref `compiler/checkerpool.go`, b85298b6) is not a plain split: it runs a weighted
FENNEL streaming graph partition over the undirected import graph (one vertex per program file, base weight =
node count + text length / 100 plus normalized import fanout, 4x source weight and penalty 16 for
declaration-heavy projects with >= 4 checkers, a 101% load cap) in program order. On the private monorepo this regime applies
(declaration files are 27% of the base weight). Two properties hurt here:

- Every file carries weight, including the 10,414 declaration files that `skipLibCheck` never checks. They take
  part of each checker's budget, so the checked work per checker is balanced only by accident.
- The stream follows program order (dependency DFS order) one file at a time; the result interleaves heavily
  (11,286 runs of equal checker among the 27,451 checked files in program order) and 41% of checked->checked
  import edges cross checkers.

## Instrumentation (all off by default)

- `TSRS_ASSIGNMENT_STATS=times`: per-checker wall time and file count of each `for_each_checker_group_do` pass,
  printed after `--extendedDiagnostics`. `TSRS_ASSIGNMENT_STATS=1` adds the touched-file report
  (`checkerpool_stats.rs`): per checker, the files whose nodes have node links or whose declaration symbols have
  value/declared-type links in that checker, split into own / foreign source / declaration / lib, and the
  per-category histogram of files touched by exactly n checkers. It reads the checkers' link stores after
  checking; no checker code runs.
- Feature `assignment-stats` (`cargo build -p tsrs_cli --features assignment-stats`): the checker records every
  type/symbol it creates (`Checker::stats_created`, compiled out otherwise), and the report attributes them to the
  file of the first declaration of the type's (alias) symbol, per checker and per package.
- `TSRS_ASSIGNMENT_DUMP=<prefix>` writes the assignment inputs (`<prefix>.files.tsv`: checked, declaration, node
  count, text length, import count, association, name; `<prefix>.edges.tsv`: resolved in-program imports) for
  offline partition experiments; `TSRS_CHECKER_ASSIGNMENT=file:<path>` runs with an assignment from a file (one
  checker index per program file).

## Where the duplication is (the private monorepo, default mode, Go assignment, 4 checkers vs 1)

Totals: symbols 22.81M vs 15.33M (binder symbols 3.78M are shared, so checker-created 19.03M vs 11.55M = 1.65x),
types 16.19M vs 9.63M (1.68x), instantiations 89.9M vs 44.8M (2.0x).

Checker-created types and symbols by the file that declares their (alias) symbol, summed over the checkers (M):

| category | types 1 checker | types 4 checkers | symbols 1 checker | symbols 4 checkers |
| --- | --- | --- | --- | --- |
| lib (`bundled:///libs`) | 1.43 | 2.08 | 4.51 | 6.65 |
| `node_modules` | 3.32 | 6.36 | 3.67 | 7.33 |
| project `.ts`, own files | 1.47 | 1.41 | 2.82 | 2.55 |
| project `.ts`, files of other checkers | - | 0.51 | - | 1.51 |
| workspace `.d.ts` | 0.03 | 0.08 | 0.15 | 0.25 |
| no declaring file (unions, literals, ...) | 3.38 | 5.73 | 0.40 | 0.75 |

Largest excess (sum over checkers minus per-file maximum) by package: zod (3.1M types / 3.3M symbols), lib (1.5M /
4.7M), @mikro-orm/core (0.6M / 0.7M), vitest (0.3M / 0.2M), project `src/services` (0.2M / 0.6M), `src/router`
(0.1M / 0.5M), mongoose (0.1M / 0.3M). Of the excess (4 checkers minus 1), library declarations (lib +
`node_modules`) account for 56% of the types (3.7M of 6.6M; another 36% have no declaring symbol: unions,
literals, ..., mostly built from library types) and 78% of the checker-created symbols (5.8M of 7.5M): the same
instantiations of zod schemas, MikroORM entities and `Array`/`Promise`/... members created in several checkers.
Project declarations resolved by a checker that does not own the declaring file are 7% / 17%.
Single-checker node_modules types (3.32M) exceed any one 4-checker share (1.2-1.9M): most library types are
per-use instantiations, so they follow the files that use them, which is what makes locality pay off.

Touched-file view (node/symbol links, Go assignment): each checker touches 11.6k-15.6k files; 13-19% of its links
are in project files owned by another checker; 3,757 project files and 714 `node_modules` files are touched by all
4 checkers (the hubs: a test-database helper module is imported by 3,215 files,
the shared router module by 3,117, the ORM setup module by 1,870, zod by 1,523).

The import graph cannot be clustered by transitive closure: there is a strongly connected component of 10,878
files, and the median checked file's transitive import closure is 24.6k files (15.1k checked files).

## Experiments (4 checkers, the private monorepo, peak = `peak memory footprint`)

Peak memory is reproducible to +-0.02 GB run to run; check times move by 10-30% with machine load (other agents),
so time comparisons below are same-round only. "cut" = share of checked->checked import edges across checkers.

| assignment | cut | peak GB | symbols | types | check s |
| --- | --- | --- | --- | --- | --- |
| Go (FENNEL, program order) | 0.41 | 17.30 | 22,813,093 | 16,186,849 | 9.96 |
| checked files by path, 4 equal-weight runs | 0.36 | 15.97 | 20,754,410 | 14,403,244 | 9.41 |
| same, cuts moved to the shallowest directory boundary within +-2% | 0.35 | 15.88 | 20,650,193 | 14,323,681 | 9.19 |
| path runs + label-propagation refinement (cap 2%) | 0.22 | 16.02 | 20,802,528 | 14,584,107 | 8.93 |
| FENNEL over `src/services/<feature>`-level directory groups, size order | 0.26 | 15.57 | 20,302,576 | 13,866,654 | 8.40 |
| **adaptive directory groups (<= 1/4 checker load), FENNEL penalty 1, size order** | 0.27 | **15.48** | 20,167,888 | 13,791,774 | 8.36 |

Lower edge cut alone does not help (the refinement halves the cut and gains nothing); keeping directory subtrees
together does. Group-size / penalty sweep for the adaptive scheme (peak GB): 4 checkers 15.05-15.59 across
groups of <= 1, 1/2, 1/4, 1/8 checker loads and penalty 1 or 16 (1/4 with penalty 1: 15.10); 3 checkers
13.87-14.41. No setting is consistently better; 1/4 + penalty 1 is used for every checker count.

## What landed

`--checkerAssignment locality` (default; `TSRS_CHECKER_ASSIGNMENT` for any binary) in `checkerpool.rs`
(`locality_associations`):

1. Weights are Go's (base + 4x source + normalized imports), but declaration and JSON files that are not type
   checked get 0 (no checker does work for them).
2. Checked files are sorted by path; each file belongs to the shallowest directory subtree whose checked weight is
   at most 1/4 of an average checker load (a file directly inside a larger directory is its own group).
3. The groups are placed with Go's own `getCheckerAssociationsInOrder` (FENNEL, 101% cap) in descending weight
   order, affinity = file-level import edges between groups, penalty multiplier 1.

Deterministic (sorted paths, groups numbered in path order, index tie-breaks; hash maps only for lookups).
`--checkerAssignment go` keeps Go's scheme. The checker count default stays Go's 4.

Diagnostics do not depend on the assignment: each file's diagnostics are computed by the checker that owns the
file and stored by file index; output is concatenated in file order and then sorted/deduplicated. Verified:
conformance with `TS_TEST_PROGRAM_SINGLE_THREADED=false` (4 checkers per test program) under both assignments,
both lazy modes, is identical to the single-threaded run (13,458 pass, `.types` 12,779, `.symbols` 12,779, all
artifacts byte-identical); a synthetic 180-file program with 898 errors prints identical output single-threaded,
with `go` (4) and `locality` (4 and 7).

## Memory / time trade-off (final code, commit 7d96945 + this change)

The private monorepo, default mode, 0 errors in every run; medians of 3 interleaved rounds (load ~8-12 from other agents).
Peak varies by <= 0.03 GB between rounds; check times are noisier (one round per row was 20-30% slower).

| checkers | assignment | peak GB | check s | wall s | symbols | types | instantiations |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | - | 11.21 | 20.9 | 24.6 | 15,331,397 | 9,630,120 | 44,820,708 |
| 2 | go | 13.70 | 12.8 | 14.8 | 18,450,594 | 12,348,464 | 62,892,108 |
| 2 | locality | 12.94 (-5.5%) | 12.0 | 14.0 | 17,320,024 | 11,402,638 | 57,345,545 |
| 3 | go | 14.81 | 10.6 | 12.7 | 20,103,591 | 13,708,706 | 72,944,572 |
| 3 | locality | 14.25 (-3.8%) | 9.9 | 11.9 | 19,239,670 | 13,013,981 | 69,527,167 |
| 4 | go | 16.93 | 10.2 | 13.9 | 22,813,093 | 16,186,849 | 89,882,712 |
| **4** | **locality (default)** | **15.15 (-10.5%)** | **8.5** | **10.6** | 20,165,269 | 13,788,912 | 76,888,800 |
| 6 | go | 19.70 | 7.7 | 9.8 | 26,307,213 | 18,973,675 | 111,177,458 |
| 6 | locality | 17.38 (-11.8%) | 7.3 | 9.4 | 23,287,394 | 16,492,339 | 96,763,265 |
| 8 | go | 21.75 | 7.5 | 9.6 | 29,392,429 | 21,724,296 | 130,570,576 |
| 8 | locality | 18.91 (-13.1%) | 6.5 | 8.7 | 25,487,708 | 18,497,338 | 112,240,065 |

Re-measured after rebasing onto 52ef5a9 (lazy tuple member tables, SymbolTable and ValueSymbolLinks layout
changes; 4 checkers median of 3, 3 checkers one run): 4 checkers go 14.99 GB / 9.5 s / 19,555,571 symbols ->
locality 13.37 GB (-10.8%) / 8.0 s / 17,297,363; 3 checkers go 13.08 GB / 10.4 s -> locality 12.61 GB / 9.3 s;
reference mode 4 checkers 20.08 -> 17.70 GB (-11.9%); single-threaded 9.98 GB (13,297,831 symbols; reference mode
25,973,354).

Each extra checker costs ~1.9 GB with Go's assignment and ~1.3 GB with locality (4 -> 8). Locality with 3
checkers (14.25 GB, 9.9 s) beats Go with 4 (16.93 GB, 10.2 s) on both axes. Reference mode
(`TSRS_LAZY_MEMBERS=0`, 4 checkers): go 22.42 GB / 39,704,001 symbols, locality 19.70 GB (-12.1%) / 34,474,225.
Single-threaded counters are unchanged (one checker: no assignment), in both modes (15,331,397 and 25,973,354
symbols).

## Remaining ideas

- Most of the remaining excess is library instantiations every checker needs (zod, MikroORM, lib). Assignment
  cannot remove it; sharing frozen library-level types across checkers or fewer checkers can.
- Per-checker check times are still uneven (e.g. 6.7 / 7.7 / 8.1 / 7.3 s): the weight proxy (nodes + text +
  imports) does not see semantic cost (zod-heavy routers are expensive). A cost model from a previous run, or
  work stealing of whole groups between checkers, would shorten the critical path.
- The adaptive grouping uses directory structure as the locality signal; projects with flat layouts fall back to
  per-file groups (FENNEL with penalty 1 in weight order), which is close to Go's scheme.
