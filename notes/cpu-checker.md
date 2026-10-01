# cpu-checker: CPU work of the checking phase

Goal: make checking itself cheaper without changing semantics. Gates for every change: the suite (errors and
`--baselines types,symbols`) byte-identical to the base in the default mode and with `TSRS_LAZY_MEMBERS=0`
(whole `target/test-results` trees compared); counters opt-out 25,973,354 / 9,639,962 / 44,884,281 single and
39,704,001 / 16,200,921 / 89,981,648 on 4 checkers (`--checkerAssignment go`), default 12,811,032 / 9,630,120 /
44,820,708 single and 16,549,988 / 13,788,912 / 76,888,800 on 4 checkers; Project 0 errors.

Measurement: the machine was shared with agents running Go test suites (load 7 to 90), so wall time was not
usable; the metric is instructions retired from `/usr/bin/time -l` on Project `--singleThreaded` (default mode),
medians of 3 (5 where the difference was small), interleaved with the previous binary. Run-to-run spread is
about +-0.5% (rayon parse threads spin). Scripts used (not committed): `samply record --unstable-presymbolicate`
(installed with `cargo install samply`; works on macOS without sudo) plus a small reader of the Firefox profile
JSON for exclusive/inclusive/caller tables and per-address samples (`objdump -d` around the hot addresses).

## Result

| | base (dc59d8e) | after (8f54a25) |
| --- | --- | --- |
| single, instructions | 324-326 G | 293-295 G (-9.5%) |
| 4 checkers, instructions | 434.3 G | 401.2 G (-7.6%) |
| single / 4 checkers, peak | 7.02 / 9.38 GiB | 7.02 / 9.38 GiB |
| single, cycles (noisy) | 97-100 G | 90-97 G |

Wall: not measured on a quiet machine (load never fell below ~7 during this pass); check time single-threaded
went 17.5-19 s -> 16.8-18 s at load 10-15, 4 checkers 7.9-8.5 s -> 7.6 s, both within the noise of the load.

## Changes (each landed separately, numbers in the commit messages)

1. Inference states: `putInferenceState` clears the pooled `visited` map after every `inferTypes`; hashbrown's
   clear (like Go's) touches every bucket, so one deep inference made every later one memset its capacity
   (2.2% of check samples). The map is reallocated at its last use's size when its capacity is >4x that and >64
   (`GoMap::clear_scratch`). 325 -> 320 G.
2. `keyBuilder` writers `#[inline]`, `spill` cold. `write_uint32` alone was 1.3% of check samples as an
   out-of-line call from every instantiation / relation / union key. 320 -> 312.5 G.
3. `IdLinkStore` finds a page through a `Vec` indexed by page number instead of an `FxHashMap` (two stores per
   checker, ~0.2 MB each for 26M ids); `get_node_id` / `get_symbol_id` inline their fast path (they were
   out-of-line calls from every crate). 312.5 -> 308.1 G.
4. Active type-mapper caches: Go's `popActiveMapper` clears the map and keeps it for the next push; the port
   dropped it and started from an empty map each push. Popped maps now go to a pool (cleared, or dropped when one
   instantiation grew them past 4x). 308.1 -> 304.9 G.
5. `TSRS_HEAP_PROFILE=count` (alloc-profile feature): samples every 1024th heap allocation by stack. Project
   single: 218M heap allocations at the start of the pass, 204M after change 6.
6. `getStringLiteralValue` returned a `String` copy (Go returns the string); ~10M allocations, mostly from
   template-literal matching against string literal sources. 304.9 -> 301.9 G.
7. `getTypeArguments` / `getElementTypes` return the stored slice (Go does). 301.9 -> 301.1 G.
8. `getPropertiesOfType` / `getPropertiesOfObjectType` / `getPropertiesOfUnionOrIntersectionType` /
   `getSignaturesOfType` / `getSignaturesOfStructuredType` / `getIndexInfosOfType` /
   `getIndexInfosOfStructuredType` return the stored slices. Counted: 15.5M property-list copies of 89.5M symbols
   (6M from `getUnmatchedProperties` alone), 21.8M signature-list and 16.2M index-info-list copies. Iterating an
   arena slice keeps the copy's snapshot semantics (stored slices are replaced, never mutated). 301.1 -> 296.3 G.
9. site-counts counters for the relation caches and the active-mapper instantiation cache (below).
10. Property-name helpers (`getPropertyNameFromType`, `getPropertyNameFromIndex`,
    `getPropertyNameForPropertyNameNode`) return `Cow<'static, str>` instead of copies; `GoMap::get` takes a
    borrowed key (the type-only export-star lookup built a `String` per property lookup on module types).
    295.8 -> 295.5 G (medians of 5, within noise; kept because it matches Go and removes the 0.5% of samples in
    `memmove` of long literal keys).
11. CompareTypes sort / binary searches use Go's algorithms (`tsrs_core::goslices`, below). 296.2 -> 294.2 G.

## Port divergences found (Go vs tsrs call counts)

Entry counters were added to ~110 hot functions in both a copy of the Go reference (`TypeScript/tsc` copied
without testdata, built with the local go1.27 toolchain; the shared checkout untouched) and in tsrs (site-counts),
and compared on Project single-threaded in the opt-out mode (where counters equal Go's). Nearly every function is
called exactly as often (getPropertyOfTypeEx 67,608,533 both, recursiveTypeRelatedTo 12,925,359, getTypeOfSymbol
54,653,970, inferFromTypes 14,513,973, isRelatedTo 22,397,251 vs 22,397,228, ...). Differences:

- **CompareTypes 183.1M in tsrs vs 163.5M in Go (+12%)**: Rust's `sort_by` / `binary_search_by` compare different
  pairs than Go's `slices.SortStableFunc` (insertion sort in blocks of 20 + SymMerge) and `slices.BinarySearchFunc`;
  the extra top-level comparisons (+2.7M) recurse into type-argument and mapper comparisons. The comparison
  sequence is observable in principle: `compareSymbols` assigns symbol ids as a last resort (`GetSymbolId` for
  same-named declaration-less symbols). Fixed for the four CompareTypes sites (change 11): 163,508,167 vs
  163,512,190; the remaining 4K come from `sortSymbols` and `compareTypesAndDepth`, which Go sorts with
  `slices.SortFunc` (pdqsort; insertion sort only up to 12 elements) and tsrs with `sort_by`. Not ported (pdqsort
  is ~300 lines; these sites are cold).
- `getSignaturesOfStructuredType` 77.5M vs 21.8M and `getIndexInfosOfStructuredType` 21.2M vs 16.2M: where Go
  reads `resolved.CallSignatures()` after `resolveStructuredTypeMembers`, tsrs calls the function again (it has to
  in the lazy-member mode, where the type may be unresolved). Same answers; in the default mode each call costs a
  `lazy_member_tables` probe for references (the probe is ~1% of check samples, mostly cache misses).
- Fewer calls in tsrs (fine): getReducedType -2.1%, getApparentType -1.6%, getPropertiesOfType -18%,
  getPropertiesOfObjectType -14%, getMergedSymbol -5%, mapType -2.7%.

No case of Go doing less work than tsrs at the function level other than the sort/search above.

## Relation and instantiation caches (site-counts)

Project single, default mode: `instantiateTypeWithAlias` 44.82M misses (= the instantiation counter; 20.0M with a
new mapper, 24.8M with an active one), 6.64M hits; `recursiveTypeRelatedTo` 6.12M cache hits, 6.82M misses, 0.41M
maybe-stack hits; `isTypeRelatedTo`'s object fast path 1.77M hits, 1.36M misses. In the opt-out mode the number of
`recursiveTypeRelatedTo` / `structuredTypeRelatedTo` / `isRelatedTo` calls equals Go's exactly, so the hit rates do
too (same keys, same sequence). Key construction is Go's byte layout through `keyBuilder` + xxh3-128, now inlined;
the remaining key cost is `hash_128` (~1.1%), as in Go.

## Profile (single-threaded check, exclusive samples)

Before: `SymbolMap::position` 5.7%, `instantiate_type_with_alias` 2.6%, `memset` 2.4% (inferTypes' visited clear),
`memcmp` 1.9%, `compare_types` 1.9%, hashbrown rehash/insert 3.5%, `get_reduced_type` 1.7%, `get_apparent_type`
1.5%, `recursive_type_related_to` 1.5%, malloc+free 2.5%, `keyBuilder::write_uint32` 1.3%, `get_object_type_instantiation`
1.3%, `hash_128` 1.2%, `IdLinkStore::get`+`slot` 1.5%.

After: `SymbolMap::position` 6.4%, `instantiate_type_with_alias` 3.2%, `memcmp` 2.2%, `compare_types` 2.2%,
`get_apparent_type` 2.2%, `get_reduced_type` 2.2%, `recursive_type_related_to` 1.9%, hashbrown rehash/insert 3.7%,
`get_object_type_instantiation` 1.2%, malloc+free 2.0%, `hash_128` 1.1%, `get_ready_lazy_member_table_worker` 1.1%;
`memset`, `write_uint32` and `IdLinkStore::slot` are gone from the list. The profile is flat; most of the top
entries spend their samples on the first load of a type, symbol table entry or hash bucket (cache misses), not on
instructions.

## Tried and rejected

- Uninitialized `keyBuilder` inline buffer (no zeroing of 192 bytes per key): -0.2%, within noise; needs unsafe.
- `#[inline]` on all `Type` accessors / casts (`target()`, `as_object_type()`, ...): no change.
- Remembering "no lazy member table" for references whose type arguments are their own type parameters: no
  change (that path is not hot).

## What remains

- `SymbolMap::position` (6.4%): 144M lookups single (incl. binding), 54% of them misses in linear tables (scan of
  4.3 entries on average; the 7-entry `Object` members table after every property miss), 18%/13% hash index
  hits/misses. Instruction cost is ~2% of the total; the rest is memory latency on cold tables. A per-table filter
  would cost 8 bytes per table (~28 MB).
- `get_ready_lazy_member_table` probes a 1M-entry map per lazy reference access (cache misses); storing the table
  pointer in the type would cost 8 bytes per reference (~17 MB).
- Allocation churn: 204M heap allocations single; the long tail is temporary `Vec`s that Go also allocates
  (instantiation type-argument lists, `SymbolTable::entries` snapshots in `getNamedMembers` / lazy members).
- `sortSymbols` / `compareTypesAndDepth` still use Rust's sort (comparison sequence differs from Go's pdqsort).
- Wall-time confirmation on a quiet machine.
