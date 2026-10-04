# mem-scoped-arenas: scoped reclamation of the checker's dead memory

Branch `mem/scoped-arenas` (from main ff92b7f). The task: give back, per checker scope (an inference context, a
`chooseOverload` attempt, a speculative expression check), the memory notes/mem-census.md measured as unreachable
at exit, without a dangling `P<T>` and without any change in results.

Result: 89 MB less garbage at exit on the private monorepo (single checker), peak -0.077 GiB with one checker and
-0.109 GiB with four, instructions unchanged within noise. That is far below the brief's "-0.3 to -0.5 GiB",
because notes/mem-recycle.md had already taken that: the 743 MB of mem-census.md were measured before it, and it
reclaimed 351 MB of them (-0.30 / -0.46 GiB). What was left for this pass is about 318 MB, and most of that is
either live-adjacent (parser speculation, flow nodes, cached types) or the expression re-check class that cannot be
made exact. A bump region per scope turned out to be the wrong tool for what remains (measured below); the classes
that are exact were reclaimed with the existing per-object escape bit, freed at scope exit.

## What was left at the start

New census switch `TSRS_CENSUS_SKIP_FREED=1`: the tables leave out the blocks the arena already frees or rewinds
(reused in normal builds), so they show the garbage that remains. Main ff92b7f, private monorepo, one checker,
default mode, grouped by cause from `TSRS_CENSUS_TSV` (script below):

| cause | main, MB at exit | this branch |
| --- | --- | --- |
| expression re-checks: object literal types, property symbols, members, member tables, spread types | 98.5 | 81.3 |
| parser speculation that is not rolled back, the 983 dropped duplicate-package parses | 43.1 | 41.2 |
| inference: contexts, infos, info lists, candidate cells, inference mappers | 36.1 | 6.3 |
| scratch mappers outside inference | 27.4 | 3.4 |
| type lists (instantiation keys, unions that die) | 20.8 | 20.5 |
| types and signatures built during instantiation that die | 11.9 | 12.3 |
| binder flow nodes | 7.2 | 7.3 |
| other arena (node builder contexts, `ExportCollision`, declaration-array growth, ...) | 22.4 | 22.3 |
| heap: symbol-table entry buffers of dead tables | 33.6 | 18.7 |
| heap: other (inference candidate buffers 1.6 -> 0.3, symlink path strings, diagnostics text) | 15.8 | 14.6 |
| **total** | **317.6** (arena 268.1, heap 49.5) | **228.4** (arena 195.0, heap 33.4) |

Freed or rewound by the arena: 334.0 MB (14.93M blocks) on main, 388.1 MB (17.25M blocks) here.

## Why not a bump region per scope

A region per scope can be released only when nothing allocated in it is still referenced; if one object escapes,
the region is pinned (or the escapee is copied out, which changes its identity: mappers, contexts, types and symbols
are all compared by pointer). Before building one, I measured how often a region would be pinned, with a
measurement-only census mode (local patch, not landed: `census_scope_begin/end(kind)` around the scope, the census
then reports per outermost scope the arena blocks allocated in it that are still reachable at exit, freed blocks left
out). Reachable at exit is a lower bound for "escaped at scope exit", so the clean share is an upper bound. Private
monorepo, one checker, this branch:

| scope | scopes | clean (no block reachable at exit) | allocated in scopes | reachable at exit | garbage in clean scopes | garbage in pinned scopes |
| --- | --- | --- | --- | --- | --- | --- |
| one argument of `isSignatureApplicable` (`checkExpressionWithContextualType` + relation) | 1,315,638 | 75.0% | 683.4 MB | 614.5 MB | 24.1 MB | 44.8 MB |
| one argument of `inferTypeArguments` (check + `inferTypes`) | 522,093 | 59.2% | 745.7 MB | 697.5 MB | 4.2 MB | 44.0 MB |
| one `chooseOverload` candidate | 564,922 | 50.5% | 1,348.7 MB | 1,268.2 MB | 2.8 MB | 77.6 MB |

Most of what these scopes allocate is live: they are where types, members, signatures and symbols are resolved for
the first time, and those land in caches. A scope region would have to route every such allocation outside the
region from the start, which means knowing at allocation time which objects a cache will keep; and even with a
perfect escape oracle, wholesale release reaches only the garbage in clean scopes (24 + 4 + 3 MB, overlapping),
while 2-25x more garbage sits next to live objects. For the classes that are exact (inference contexts, scratch
mappers), the escape bit of notes/mem-recycle.md already decides per object, which is strictly more precise than
per scope. So the scope exit became the recycling point, and allocation stayed where it was.

## What changed

Each item frees only what it made, only if the escape bit is clear, at the end of the scope that made it.

1. **`inferTypeArguments`' own contexts.** The snapshot of the outer context (`cloneInferenceContext(outerContext,
   NoDefault)`, used for one instantiation of the contextual type and one `inferTypes`) is recycled after that
   `inferTypes`; the return context (`returnContext`, whose inferred part `cloneInferredPartOfContext` copies) after the
   copy. Both are referenced only by locals of the function unless one of their mappers was stored (escape bit).
2. **Return mappers live and die with their context.** `context.returnMapper` (the mapper of a clone made by
   `inferTypeArguments`) and `outerReturnMapper` (`createOuterReturnMapper`: the mapper of another clone, merged after
   the return mapper of that time) used to be marked escaped when stored, which kept the clones forever. Now the
   setters escape them only if the context has escaped (`InferenceContext::escape` already walks both), and
   `InferenceContext::recycle` frees them with the context: the merged mapper, and the clones behind the mappers
   (with their infos, lists and cells), unless a mapper escaped on its own (an instantiation stored it). A replaced
   return mapper (second inference round) is freed when the outer return mapper was its last holder, else left alone.
   Go reads the fields only in `instantiateContextualType` and `createOuterReturnMapper`, both inside the scope.
3. **`getTailRecursionRoot`.** The composite type-parameter mapper (used only for the `map` calls) is freed right
   away; the root mapper and its type-argument list (`alloc_slice_recycled`) are freed at once when there is no tail
   recursion, and otherwise when the enclosing `getConditionalType` returns (`scratch_mapper_lists`, next to
   `scratch_mappers`). A one-type list is not referenced by the simple mapper and is always freed.
4. **One-instantiation mappers**: `getConstraintOfDistributiveConditionalType` (the `prependTypeMapping` pair),
   `isGenericMappedType` (`nameType` instantiation), `substituteIndexedMappedType` (simple + composite).
5. **Scratch symbol tables off the arena.** `checkObjectLiteral`'s `allPropertiesTable` and the JSX
   `allAttributesTable` (strict null checks only, read by `checkSpreadPropOverrides`, never stored) are owned locals:
   heap only, dropped at return, nothing to prove. `checkObjectLiteral`'s `propertiesTable` is created on first use:
   Go makes a new one after every spread, and the last one is usually never used (an object literal ending in a
   spread). Symbol tables 3.49M -> 2.80M; their garbage 26.2 -> 10.3 MB arena, entry buffers 28.7 -> 14.1 MB heap.

Results are unchanged by construction: none of these objects has an id (mappers, contexts, infos, lists and tables
are identified by address only, and nothing orders by address), reuse only recycles memory, and an empty table that
is created later is unobservable.

## Census proof per class

Private monorepo, alloc-profile build, `TSRS_CENSUS=1 TSRS_CENSUS_VERIFY=1 TSRS_CENSUS_ASSERT=1`: every freed block
must be unreachable from the program's roots by the strong mark (pointer words the program stores, from blocks
allocated before the free), and the precise walk (every node, parent, JSDoc node, diagnostic, flow node, flow list,
declaration) must find no freed block.

New frees (one checker, default mode, by allocation site; main 334.0 MB -> 388.1 MB):

| class | blocks freed (new) | MB |
| --- | --- | --- |
| inference contexts (snapshot, return context, return-mapper clones) | +176,644 | +10.8 |
| their infos (`newInferenceInfo` +114,011, `cloneInferenceInfo` +149,989) and info lists | +440,644 | +14.2 |
| candidate cells (`LazyVec`, push and copy) | +115,831 | +3.5 |
| simple mappers (one-instantiation sites, tail-recursion roots) | +526,016 | +8.0 |
| array mappers (tail-recursion roots) | +291,409 | +4.4 |
| merged / composite / inference mappers (outer return mappers, tail-recursion composites, clone mappers) | +409,841 | +6.3 |
| tail-recursion type-argument lists | +357,018 | +7.0 |

| run | freed or rewound | strong mark: freed blocks reachable | precise walk: references to freed blocks |
| --- | --- | --- | --- |
| 1 checker, default | 17.25M blocks / 388.1 MB | 0 | 0 of 48.2M |
| 1 checker, `TSRS_LAZY_MEMBERS=0` | 18.33M / 404.7 MB | 0 | 0 |
| 4 checkers, default | 26.14M / 559.5 MB | 0 | 0 |
| 4 checkers, `TSRS_LAZY_MEMBERS=0` | 27.30M / 577.2 MB | 0 | 0 |

The scratch tables of item 5 are not freed blocks (they are never allocated in the arena).

Conformance corpus, 12,758 files through `tsrs --strict --target esnext`: in every file 0 freed blocks strongly
reachable and 0 precise-walk references to freed blocks (4,006 files without and 8,752 with diagnostics; no assert
exit).

## Poison and debug checks

- `TSRS_ARENA_POISON=1` fills freed blocks with 0xA5 and never reuses them. New: alloc-profile builds then check
  every `P` dereference (`Deref`, `P::get`) of an 8-byte-aligned target of 8 bytes or more against the fill and panic
  at the use ("dereference of a freed block"), instead of a changed result somewhere later.
- Private monorepo with the poison-checking profile build: output and counters identical to main in all four modes
  (1 / 4 checkers, default / `TSRS_LAZY_MEMBERS=0`).
- Conformance corpus through the poison-checking profile build: 12,758 files, output identical to main's release
  binary, no panic.
- Suite with `TSRS_ARENA_POISON=1` (release): trees identical to main. Suite with the dev-profile `tsrs-test` (debug
  assertions: freed free-list blocks are poisoned and checked when reused, frees assert that the block belongs to
  the arena that frees it): trees identical in default and `TSRS_LAZY_MEMBERS=0`.

## Census tooling changes

- `TSRS_CENSUS_SKIP_FREED=1` (above).
- The strong mark scans the root ranges (data segments, stack) in 8-byte steps. With 4-byte steps it read the high
  half of one word and the low half of the next as a pointer: in `SOURCE_TEXTS` (pointer, length) slots, a heap
  address's high half 0x200 next to a text length of 0x7c01 is the arena address 0x7c01_0000_0200, which happened to be
  a freed mapper (one false violation on the private monorepo). Statics and stack slots keep pointers 8-aligned; the
  packed 4-byte slots the 4-byte steps exist for live in arena and heap blocks, which keep the 4-byte scan.
- The strong mark (and the conservative mark) scan the stack from the frame of `census::run` up, not from the
  census's innermost frame. A first conformance-corpus pass reported one to ten violations in 8 of 12,758 files, at
  random (none reproduced in 200+ reruns of those files): every captured one was a parser node rolled back by a
  speculative parse (main's step 1 of notes/mem-recycle.md, not this branch's frees) referenced from `root stack
  +0x58` .. `+0x5d8`, dead slots of the census's own frames that kept block addresses the census itself had just
  sorted and indexed (freed ones included). Scrubbing the stack before the census (as the LSP census does; kept in
  the CLI too) did not remove them, starting the scan above the census frames did.

## Measurements

Private monorepo, `--noEmit --extendedDiagnostics --incremental false`, main ff92b7f vs this branch, interleaved,
6 rounds (two sets of 3), medians and ranges; peak = peak memory footprint (`/usr/bin/time -l`); machine load 18-53.

| run | main | this branch | change |
| --- | --- | --- | --- |
| 1 checker, peak | 5.034 GiB (5.027-5.038) | 4.957 GiB (4.948-4.962) | -0.077 GiB (-1.5%) |
| 4 checkers, peak | 6.684 GiB (6.669-6.690) | 6.575 GiB (6.566-6.587) | -0.109 GiB (-1.6%) |
| 1 checker, instructions | 303.6 G (296.3-308.5) | 303.2 G (296.7-305.5) | -0.1%, within noise |
| 4 checkers, instructions | 409.4 G (402.0-411.0) | 404.6 G (402.5-410.9) | -1.2%, within noise |

Diagnostics and counters (Types, Symbols, Instantiations) identical in every run. (Another session was editing
`src/debugWithOlympusContext.ts` in the corpus during the runs, which moved `Lines`/`Identifiers`/`Symbols` by 1-2 in one
run; main and this branch agree whenever they saw the same file.)

## Gates

- Suite `--baselines types,symbols`, whole `target/test-results` trees against main's binary (only the `ms` timings
  of `summary.json` differ): identical in default and `TSRS_LAZY_MEMBERS=0`, 13,458 / 12,779 / 12,779 as on main.
- `--baselines js,jsmap,sourcemap`: identical trees, 13,392 `.js` pass / 0 fail.
- Fourslash: 4,066 pass / 63 fail, same pass list as main.
- `cargo test -p tsrs_cli`: green (tsctests, default_emit). On macOS the test binary links only with `api.rs`'s
  `memory_tests` (glibc `malloc_trim`, `/proc`) compiled out, which is the same on main; not changed here.
- `RUSTFLAGS="-D warnings" cargo +1.99.0 check --workspace --locked`, and `-p tsrs_cli --features alloc-profile`: clean.

## Not done / rejected

- **Regions per scope**: measured above; pinned in 25-50% of scopes, and the garbage sits mostly in pinned scopes.
- **Promotion (copy escapees out at scope exit)**: changes identities; every class here is compared by pointer.
- **Expression re-checks (81 MB + their entry buffers)**: object literal types, their property symbols, members and
  tables from `checkExpressionWithContextualType` under `isSignatureApplicable` / `inferTypeArguments`. They escape
  through value-keyed caches (unions, intersections, instantiations, literal types first created in the scope),
  link stores and the relation machinery (notes/mem-recycle.md "Item 3"); the scope measurement shows the same from
  the other side: 90% of what an argument check allocates is still reachable at exit. Not exact without a barrier on
  every cache insert and link write; stopped here as the brief says.
- Smaller exact sites left (each < 2 MB now): `inferFromSignatures` mappers (1.0), `instantiateSignatureInContextOf`
  contexts (1.0 + 0.7), `chooseOverload` signature-instantiation type lists (1.9, 58% garbage: the rest is cached).

## Reproduce

```sh
CARGO_TARGET_DIR=$PWD/target/prof cargo build --release -p tsrs_cli --features alloc-profile
cd <private monorepo>/apps/olympus
TSRS_CENSUS=1 TSRS_CENSUS_VERIFY=1 TSRS_CENSUS_ASSERT=1 TSRS_CENSUS_SKIP_FREED=1 TSRS_CENSUS_TSV=/tmp/c.tsv \
  <wt>/target/prof/release/tsrs -p . --noEmit --extendedDiagnostics --pretty false --incremental false --checkers 1
TSRS_ARENA_POISON=1 <wt>/target/prof/release/tsrs -p . --noEmit ...   # poison + dereference check
```

The scope measurement was a local patch: `tsrs_core::census_scope_begin(kind) -> u32` (the thread's census block
count) and `census_scope_end(begin, kind)` (records the outermost scope of each kind), called around the argument
loop bodies of `isSignatureApplicable` (check + relation) and `inferTypeArguments` (check + `inferTypes`) and around
`choose_overload_candidate`; `run_frozen` then looks up each scope's blocks after the mark (freed ones skipped) and
prints, per kind, scopes, clean scopes, allocated / reachable bytes and the garbage in clean and in pinned scopes.

The cause grouping applies these rules to the `arena by type and allocating function <- callers` rows of the TSV:
parser/binder frames first, then inference types (contexts, infos, info lists, candidate cells, rare tails, mappers
made by `infer_type_arguments` or inference code), other mappers and `getTailRecursionRoot` lists, object literal /
spread / `checkExpressionWorker` allocations, remaining type lists, remaining types and signatures; heap rows by
`EntryVec` / `SymbolMap::insert`, `LazyVec` / `clone_inference_info`, rest.
