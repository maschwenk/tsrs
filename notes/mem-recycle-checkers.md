# mem-recycle-checkers: retiring checkers at a memory budget on the 38k-file codebase (2026-10-09)

Question: the 38k-file codebase (now 118k program files, 90k checked) peaks at 18.0 GB with 8 checkers on a Mac.
Where do the gigabytes go, what is different about it from the bench projects, and what lowers it?

Answer: almost all of it is what each checker keeps of the work it has done. A checker on this program holds
~1.7 GB (arena ~1.2 GB, heap ~0.5 GB), against 30-150 MB per extra checker on the bench projects, and duplication
between checkers is only 1.31x (`TSRS_ASSIGNMENT_STATS`: 75% of each checker's link work is its own files). Peak memory
is linear in the work, not in the checker count: 4 / 8 / 16 checkers peak at 15.9 / 18.0 / 21.3 GB, so about 4.3 GB of
front end + ~9.5 GB that grows with the files checked + ~0.45 GB per checker. Nothing a checker made for files it
has finished is ever freed, and on this program that is most of the peak.

`--maxMemory <size>` (opt-in; or `TSRS_MAX_MEMORY`): in the `--noEmit` type-check pass each checker allocates in a
region of its own; between two files, while the process's memory (physical footprint on macOS, RSS on Linux) is above
the target, the checker with the largest region is retired (its global diagnostics and counters kept) and the rest of
its queue goes to a fresh checker in a fresh region. In this mode each checker checks its leaf files first, so their
trees are freed early, and empties its relation caches at 2M entries. Peaks land within 0.5% of the target: `12G`
12.9 GB (+11% instructions), `10G` 10.8 GB (+16%), `9G` 9.7 GB (+22%), `8G` 8.7 GB (+35%, wall +25%) on this Mac (8
checkers on 14 cores), against 18.0 GB without it. Diagnostics byte-identical in every run, also in poison mode.

The first version (section 3) was a per-checker budget (`--checkerMemoryBudget <MiB>`: retire any checker whose
region exceeds it); the process-wide target retires only when the process is over it and always the largest checker,
which costs less for the same peak (10.8 GB for +16% against 10.4 GB for +25%).

Base: origin/main be05ad5. Mac: Apple M-series, 14 cores, 96 GB, 16 KiB pages, shared with other work (1-minute load
up to 10 during some runs: the first baseline took 42 s, the same binary 25.7 s later; wall times below are from
interleaved runs only). `tsrs -p tsconfig.typecheck.json --noEmit --incremental false --pretty false --checkers 8`,
release build (fat LTO, no PGO). Peak = `/usr/bin/time -l` peak memory footprint.

## 1. Where the memory is (8 checkers, base)

`TSRS_MEM_SPLIT=1` and the alloc-profile heap sampler (`TSRS_HEAP_PROFILE=1`):

| | GB |
| --- | ---: |
| peak footprint | 18.0 |
| parse end: thread arenas used / leaf file regions / heap live | 2.9 / 1.55 / 1.4 |
| check end: thread arenas used (front end + checkers) | 10.5 |
| check end: heap live (front end 1.4, checkers 4.6 = 510-650 MB each) | 6.4 |

Front-end heap: 0.82 GB is source text (848 MB of program text: 630 MB in 90.8k project files, 218 MB in 27.3k
node_modules files), ~0.15 GB binder symbol tables, ~0.15 GB loader and resolution data. Checker heap by allocating
site (sampled): relation caches 1.06 GB (23%; `strict_subtype` 36 MB per checker, larger than on any bench project),
cross-product intersections 0.36 GB, conditional instantiation maps 0.30 GB, object type instantiations 0.21 GB,
spread types 0.28 GB. Arena by type: `Symbol` 1.70 GB (55.6M), identifiers 0.92 GB, `TypeMapper` 0.89 GB (58M),
`ValueSymbolLinks` pages 0.71 GB, type lists 0.64 GB, `IntersectionType` 0.51 GB (11.2M), `ObjectType` 0.48 GB.

What every checker builds again (assignment-stats feature build, types and symbols by declaring package, excess
over the largest checker): the default libs (4.8M types, 7.6M symbols), the project's `src/shared` (1.9M / 4.6M),
`@types/react` + `csstype` (2.8M symbols), jest's types (1.1M symbols). About 1.2-1.5M types per fresh checker.

One source pattern stands out: 31% of all intersection types (sampled at creation) are `"<css property>" &
Exclude<"<css property>", keyof CSS>`, all made while building the generic form of the codebase's
style-prop alias `StyleXStyles<Omit<CSSPropertiesWithExtras, keyof CSS>>`: StyleX's `_GenStylePropType`
computes `Exclude<keyof CSSPropertiesWithExtras, keyof X>`, and with `X` an `Omit<..., keyof CSS>` that compares
~550 property names against ~550 deferred conditional types (~300k types, 0.17 s, once per checker and alias). It is
the codebase's own type, and tsgo pays it as well; the fix belongs in the codebase (a deferred
`[CSS] extends [unknown] ? Omit<T, keyof CSS> : never` keeps the same types and messages and drops tsgo's type count
by 10.6%), not in tsrs.

## 2. The change

- `crates/tsrs_compiler/src/checkerpool.rs`: a pool slot is a `SlotChecker` (the checker and, when recycling applies,
  its `Region::new_scratch`). The region is entered as a scratch region whenever the checker runs (`CheckerHandle`,
  `for_each_checker_parallel`, the group loop), so everything the checker allocates lands in it, while what must
  outlive it escapes as it already does for per-file emit regions: diagnostics (`diagnostic.rs`), lazily filled data
  of shared objects (`enter_owner`, which escapes a scratch region unless the object is inside it), lazy member lists
  of declaration files (the parser escapes), process-wide statics (`enter_thread_arena`). In the type-check pass a
  checker whose region's chunks exceed the budget is retired between two files when work remains
  (`retire_checker`): its global diagnostics go to `RetiredCheckers` (added in `get_global_diagnostics`), its Types /
  Symbols / Instantiations / lazy-member counters too (added in `Program::type_count` etc.), and it is dropped, then
  its region (which runs the drops of the values in it and gives its chunks back).
- Applies where leaf freeing is allowed (`ProgramOptions::checker_recycling`: the CLI's `--noEmit` check without
  declaration diagnostics, `--explainFiles` or Go's check history), but independent of the checker count, and only
  with more than one checker. No later pass runs a checker over a checked file there, which is what makes dropping it
  safe; the pass already drops the diagnostics a checker files in files it does not check.
- With recycling, each checker's queue is heavy files first, then its leaves, then the rest in order: the leaves'
  regions (1.55 GB here) go early, while the checkers are young.
- `Checkers: retired` in `--extendedDiagnostics`. `--maxMemory` sizes take K, M or G (binary); a bare number is MiB.
  A checker smaller than 128 MiB is never retired (`TSRS_RETIRE_MIN`), so a target below the front end plus fresh
  checkers costs retirements but does not thrash.
- `crates/tsrs_cli/tests/recycle_checkers.rs`: every testdata/regressions case with `--maxMemory 1M` and
  `TSRS_RETIRE_MIN=0` (a retirement after nearly every file) at 2 checkers in poison mode and at 3, against tsgo-ref's expected output.

## 3. Numbers of the first version, a per-checker budget (8 checkers)

Budget sweep (one run each; leaves first and mapped text off unless named; diagnostics identical in all):

| budget | peak GB | instructions T | retired |
| --- | ---: | ---: | ---: |
| off | 18.00 | 2.274 | 0 |
| 1024 MiB | 17.63 | 2.311 | |
| 768 MiB | 14.66 | 2.538 | |
| 512 MiB | 11.68 | 2.748 | |
| 384 MiB | 10.70 | 2.883 | |
| 256 MiB | 9.70 | 3.421 | |
| 512 MiB, leaves first | 10.01-10.59 | 2.786-2.807 | 22 |

Below 512 MiB the cost grows fast: a lifetime is then mostly the ~1.2M types every checker rebuilds.

Interleaved, 3 rounds (medians): base 25.73 s / 18.00 GB / 2.274 T; `--checkerMemoryBudget 512` with leaves first
31.2 s / 10.0-10.6 GB / 2.80 T (the 10.0 GB runs also mapped source files, section 4).

Poison mode (`TSRS_ARENA_POISON=1`, retired regions filled and kept) at 512 and 256 MiB on the 38k-file codebase:
same 379 diagnostics, no crash.

## 3b. Relation caches, and what a partial eviction could reach

With a memory target the relation caches (yes/no memos; 23% of a checker's heap here) are also emptied between
two files once they hold 2M entries. Alone (no retirement): peak 18.02 -> 17.00 GB (-5.7%) for +2.2% instructions;
with retirement at 512 / 768 MiB: 10.60 -> 10.38-10.45 GB and 13.39 -> 12.91 GB. Final, 2 interleaved rounds:
off 18.00 GB / 2.27 T / 25.9 s; 512 MiB 10.43 GB (-42%) / 2.84 T (+25%) / 30.4 s; 768 MiB 12.92 GB (-28%) /
2.59 T (+14%) / 28.7 s. Diagnostics identical. Not on by default: an emptier cache leaves the relater more of its
complexity budget, which could move a TS2859 boundary against tsgo.

Why replacing a whole checker works this well, measured: an epoch census (branch `exp/usecensus-epochs`: the use
census of mem/use-census-apps merged onto this change, plus per-16-byte maps of the allocating checker thread and its
epoch, files finished, and of the latest epoch each granule was read). Of the arena bytes a checker has allocated by
the time it has checked a fraction t of its files, never read again after t: 68% at t = 0.1, 74% at 0.5 (4.1 of 5.6
GB over the 8 checkers), 84% at 0.9; never read at all: 3%. At t = 0.5 by site: transient symbols 1.1 GB (76% dead),
union and intersection member lists and `IntersectionType` (79-93%), member symbol tables (81%), signatures (68%),
`TypeMapper` (98-99%), inference contexts (100%); value-symbol link pages are mostly live (17% dead). The dead part is
the instantiated type graph each file needed, kept reachable by the identity caches (instantiations, unions,
intersections) and the link stores; only ~3-7% is unreachable garbage. Freeing it in place needs a collector that
treats the identity caches as weak (section 5); a retirement drops all of it at once and rebuilds the live quarter.

## 4. Tried and rejected

- **Retiring only where the queue changes directory** (`src/<a>/<b>` groups, depths 2-4, capped at 1.5x the
  budget): 2.730-2.745 T against 2.748 T. The rebuild cost is the shared base, not the neighbouring files.
- **Staggered first lifetimes** (checker i's first budget `B * (i+1)/n`, so the checkers' sawtooths are out of phase):
  modeled as -16% peak at the same retirements for linear growth; measured -7% at 768 MiB, nothing at 512, and
  -2.2% for +2.5% instructions with leaves first. Growth is not linear and the front end dominates. Removed.
- **Mapping source files instead of reading them** (`mmap` for files of at least 16 KiB: 10.5k files, 465 MB): -0.61
  GB footprint (-3.4%), RSS unchanged (mapped pages are counted there), +1% instructions and +4% wall (page faults:
  +6-10 s system time). Below the 5% bar, and RSS-based limits (Linux) see nothing. The patch is not kept.
- **Sharing a seed between lifetimes** (spike/shared-graph's frozen seed + forks) would remove most of the rebuild
  cost; not tried here (2,770 lines, 55 commits behind main).

## 5. What is left

- A collector for checker data (measured since: notes/mem-checker-gc.md, below the bar): identity caches weak, link stores and declared-symbol state strong, run between two
  files. The epoch census bounds what it could free (about 3/4 of a checker's arena at mid-life) without the 14-25%
  rebuild cost of retirements. Next step before building it: run the reachability census (`TSRS_CENSUS=1`, which
  already traces the checker) with the identity caches removed from its roots, to measure what such a collector
  would actually free (some dead objects stay reachable from live ones).

- The ~1.2M types a fresh checker rebuilds are the cost floor of a retirement; a seed shared by the lifetimes
  (spike/shared-graph) is the way to lower it.
- At 512 MiB the front end is half of the peak: 2.9 GB of trees and binder output of non-leaf files, 0.8 GB of source
  text, 0.8 GB of other front-end heap.
- Not measured: Linux (THP: checker regions are carved from slabs without huge pages, unlike thread-arena chunks), the
  bench projects (the option is off by default), PGO builds.
