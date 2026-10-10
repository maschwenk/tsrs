# mem-overload-rollback: can the speculative work of overload resolution be rolled back?

Experiment 2 of the 2026-10-02 checker-memory round. Question: can the memory allocated by the argument checks of
rejected overload candidates (and other speculative argument checks whose results are thrown away) be given back,
without changing any output byte? Result: **measured, not implemented.** The upper bound for the scope the brief
names (argument checks in `isSignatureApplicable` from `chooseOverload`) is 175-201 MB, below the 300 MB threshold;
the broadest scope (all of `chooseOverload`) reaches 351-409 MB, but only as an at-exit upper bound, and getting any
of it needs a store barrier on every checker write. The branch lands the measurement only (compiled out of normal
builds).

## Why ids decide the design

Type and symbol ids are observable: union constituent order, relation cache keys, `.types` / `.symbols` baselines and
the `--extendedDiagnostics` counters all depend on them, and most caches are keyed by them.

- **(a) Undo the region**: restore the id counters and remove every cache entry, link and list member the region
  created, so the state after the rollback is the state before it. Work the region did that is needed later (a
  declared type first resolved inside a rejected candidate, a library instantiation) is then redone later with other
  ids. Go never undoes anything, so its ids differ from that point on: union order and printed types change. This
  cannot pass the byte-identical gates, in either lazy mode. Rejected.
- **(b) Tombstones**: keep every id consumed and every observable order; free only the payload of region objects
  that nothing surviving refers to. Two levels: (b1) keep every cache entry, so only objects no cache holds can go;
  (b2) also drop cache entries whose key involves a dead region object. (b2) is exact as well: ids are never reused,
  so a key that contains the id of an object nobody holds can never be built again. Value-keyed caches (literal
  types by text, undefined properties by name) and node-keyed links must stay: their keys can be rebuilt at any time.

So the measurement asks, per variant of (b), how much of the region's memory is unreachable.

## Method (alloc-profile build, `TSRS_CENSUS=1`)

- **Regions.** `TSRS_CENSUS_REGION=applicable` (default) wraps every `isSignatureApplicable` call from
  `chooseOverload` (the three calls without error reporting: each candidate's argument checks and relation checks,
  rejected and chosen, including nested calls inside the arguments). `overload` wraps all of `chooseOverload`
  (adds `inferTypeArguments`, the inference contexts and the signature instantiations). Every arena and heap block
  allocated while a region is entered on the thread is tagged (bit 31 of its site index / stack id).
- **Mark 1** is the census's ordinary conservative mark: region blocks it does not reach are unreachable even with
  every cache kept (variant b1).
- **Mark 2** treats the identity-keyed caches as weak tables (`crates/tsrs_checker/src/census_weak.rs`): their
  blocks are not scanned; each entry is an ephemeron whose value words are scanned once all its key objects are
  reached (iterated to a fixpoint: 19 passes, 22.9M entries). Keys are read off the entry where possible: union /
  intersection / template / indexed-access / string-mapping / substitution types (their own fields), signature and
  anonymous-type instantiations (generic + mapper targets), reference instantiations of generic classes and
  interfaces (type arguments), alias and conditional instantiations, type-keyed tables, every link store keyed by
  symbol, value-symbol links of checker-created symbols (binder symbols' slots stay strong), and the tables keyed by
  type ids (an id -> type map built from noted types; one checker, so ids are unique). Entries whose key cannot be
  recovered (`subtype_reduction_cache`, `flow_loop_cache`, `error_types`, `marker_types`, a few undecodable values)
  are either fully weak (default: over-approximates) or strong (`TSRS_CENSUS_WEAK=decoded`): the two runs bracket
  variant b2.
- **Upper bound.** Both marks run at exit. A block unreachable at the end of its region stays unreachable (any later
  lookup goes through a table that holds the pointer), but a block reachable at region end may die later (a scratch
  structure that held it is dropped), so the at-exit number bounds what a perfect rollback at region end could free.
  Blocks already freed by the mem-recycle sites are counted separately and excluded.

## Numbers (the private monorepo, `--checkers 1`, default mode)

Region allocations still held (after recycling), and how much of that is unreachable at exit:

| region | allocated in region | already recycled | kept | b1: no cache entry dropped | b2: entries with dead keys dropped |
| --- | --- | --- | --- | --- | --- |
| `isSignatureApplicable` from `chooseOverload` | 1,134 MB | 73 MB | 1,061 MB (768 arena + 293 heap) | **127 MB** (98 + 30) | **175-201 MB** (137-159 arena + 38-42 heap) |
| all of `chooseOverload` | 2,836 MB | 249 MB | 2,587 MB (1,776 + 811) | **240 MB** (197 + 44) | **351-409 MB** (284-333 + 68-77) |

(MB = 2^20 bytes. The b2 range is decoded-only weak tables / undecodable entries also weak.)

What the b2 garbage is (`applicable`, MB unreachable in b2 of MB kept): property `Symbol`s 39 of 136, object-literal
`ObjectType`s 25 of 57, their `StructuredMembers` 23 of 51 and member `SymbolTable`s 21 of 27 (plus 26 MB of heap
entry buffers from `SymbolMap::insert` in `checkExpressionWorker`, 98% dead), `[P<Symbol>]` property arrays 13,
type lists 9, unions 8, leftover mappers 6. This is the census's item 3 (object literals of re-checks). The region's
type references (47 of 51 MB still reachable), signatures (42 of 42), type parameters, conditional and mapped types
stay reachable even with weak caches: that work is reused (declared types, library instantiations, signature
instantiations cached by live keys).

Which weak tables keep the b2-only part (first hop, `applicable`): widened types of `getWidenedTypeWithContext`
(4.7 MB), value-symbol links of dead transient symbols (4.5), anonymous-type instantiations (3.6), mapped-type member
links (2.7), reference instantiations with dead type arguments (1.9), unions with a dead constituent (1.2).

The census run with both marks takes 1-2 minutes and ~13.6 GB peak (the normal run: ~22 s, 6.2 GB).

## Why the journal was not implemented

- The scope the brief asks about (argument checks of overload candidates) is bounded by **175-201 MB**, under the
  300 MB threshold, and that is an at-exit upper bound.
- The broadest scope crosses 300 MB only with every identity-keyed cache entry dropped and undecodable entries
  counted as weak, and two thirds of it (240 MB) is memory that a journal of cache inserts does not help with at
  all: it is already unreachable, and freeing it needs the same thing as everything else here, a proof at region end
  that nothing outside the region points to an object.
- That proof is the real cost. A journal records what the region inserted into caches, but region objects also
  escape through ordinary field writes: node links (`resolvedSignature`, context-free types), links of binder
  symbols (declared types resolved inside the region), fields of older types (`resolvedBaseConstraint`, regular /
  widened / apparent types, lazy member tables, union `constituentMap`), deferred diagnostics, inference contexts of
  outer calls. Knowing at region end which region objects are still referenced needs either a write barrier on every
  pointer store from an older object into the region (a generational remembered set over hundreds of `Cell` fields,
  link stores and caches, not the ~60 mapper sites of mem-recycle) plus a typed tracer over the region's objects, or
  a mark from all roots per region (a full mark takes 20-40 s here; regions run once per call resolution). Either is
  a garbage collector for the checker's object graph, for at most ~0.2 GB (3% of the single-checker peak) in the
  named scope.
- Go's GC does none of this either: the caches keep the same objects alive in tsgo, so this would be a tsrs-only
  divergence with a large invariant surface.

If someone revisits it: the cheap, exact subset is the census's item 3 restricted to object-literal types whose
property symbols never got links and that never entered a cache (a per-type "escaped" bit set by every cache insert
and link write that takes a type). The b1 column bounds it: at most 127 MB in the named scope.

## Instrumentation landed on the branch

Status (2026-10-10): none of this is on main. `tsrs_core::census_hooks`, `crates/tsrs_checker/src/census_weak.rs`
and the `TSRS_CENSUS_REGION` / `TSRS_CENSUS_WEAK` switches are absent from crates/; rerunning the measurement needs
the experiment branch.

- `tsrs_core::census_hooks` (no-ops without the alloc-profile feature): `region_enter` / `region_exit`, `weak`,
  `ephemeron`, `note` / `noted`. `census.rs` tags region blocks and prints the region report (types, weak tables
  holding the b2 part, heap blocks by allocating function, `region` rows in `TSRS_CENSUS_TSV`).
- `crates/tsrs_checker/src/census_weak.rs`: the weak-table registration; `CensusRegion` guards in
  `chooseOverload` / `isSignatureApplicable` (checker_05.rs); notes in `newType`, `newSymbol` and the instantiation
  tables' `make`; `census_entries` / `census_slots` on the link stores; `Symbol::peek_id`.
- Env: `TSRS_CENSUS_REGION=applicable|overload|none`, `TSRS_CENSUS_WEAK=decoded`.

## Gates (normal build vs the branch point de3beaf)

- Conformance suite with types/symbols baselines, default / `TSRS_LAZY_MEMBERS=0`, single-threaded and
  `TS_TEST_PROGRAM_SINGLE_THREADED=false`: result trees identical except timings in `summary.json` (13,458 error
  baselines, 12,779 types/symbols baselines pass).
- Fourslash: 4,066 pass / 63 fail.
- The private monorepo: diagnostics and `--extendedDiagnostics` counters identical with 1 and 4 checkers and in the
  opt-out mode.
- `RUSTFLAGS="-D warnings" cargo +1.99.0 check --workspace --locked` clean (also `-p tsrs_cli --features
  alloc-profile`).
- No memory is freed, so the census free-gate does not apply to this change. Side finding: the would-free check on
  the branch point itself (no change of ours) now reports 25-27 strongly reachable freed blocks on the private
  monorepo (referrers: `InferenceInfo` +40, `LiteralType` +16, relater maybe-key tables); mem-recycle recorded 0.
  Not investigated here; whoever owns the recycling sites should look.

Peak footprint and instructions (3 interleaved rounds, medians): single 6.189 GB base / 6.178 GB branch,
311.1 G / 312.0 G instructions; 4 checkers 8.166 / 8.167 GB, 423.6 G / 424.7 G. All within run-to-run noise (the
hooks compile to nothing).
