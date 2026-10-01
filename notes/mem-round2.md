# mem-round2: representation changes for peak memory (after mem-layout and mem-lazy)

Same rules as notes/mem-layout.md: only layout / representation changes, no checker semantics. Gates for every
step: the suite (errors and `--baselines types,symbols`) byte-identical to the base commit in the default mode and
with `TSRS_LAZY_MEMBERS=0` (whole `target/test-results` trees compared); opt-out counters 25,973,354 / 9,639,962 /
44,884,281 single and 39,704,001 / 16,200,921 / 89,981,648 with `--checkers 4 --checkerAssignment go`; default
counters unchanged (12,811,032 / 9,630,120 / 44,820,708 single, 16,549,988 / 13,788,912 / 76,888,800 on 4
checkers); Project 0 errors.

Measurement: `/usr/bin/time -l` on the pristine Project checkout, interleaved rounds, "GiB" = peak memory
footprint / 2^30. The machine was shared with other agents (load 8 to 70), so check times are noisy; instructions
retired are the stable CPU-work measure.

## Profile at the start (main at e8d4196, default mode, single)

Arena 6,027 MB requested, heap outside the arena 2,094 MB live, peak 8.13 GiB.

| arena type | MB | count | B/each |
| --- | --- | --- | --- |
| Symbol | 880 | 12,811,033 | 72 |
| NodeAlloc<Identifier> | 419 | 7,844,376 | 56 |
| TypeMapper | 396 | 17,294,526 | 24 |
| TypeAlloc<ObjectType> | 344 | 3,006,176 | 120 |
| [ValueSymbolLinks] chunks | 337 | 10.5M values | 32 |
| TypeAlloc<TypeReference> | 289 | 2,106,919 | 144 |
| str (213 MB source text) | 274 | 3,006,153 | |
| [P<Type>] | 224 | 13,080,622 | 17 |
| TypeAlloc<UnionType> / <IntersectionType> | 187 / 173 | 1.02M / 1.13M | 192 / 160 |
| InferenceContext / InferenceInfo | 175 / 163 | 1.43M / 1.78M | 128 / 96 |
| Signature | 156 | 1,574,006 | 104 |

Heap live by allocating function: `SymbolMap::insert` 493 MB, instantiation `GoMap`s 226, `Relation::set` 190,
lazy member tables (`get_ready_lazy_member_table_worker` + `get_lazy_declared_member` + `resolve_lazy_members`)
271, pointer-keyed `LinkStore`s 104.

## Steps

### 1. `Symbol` 72 -> 64 bytes

`name` and `declarations` are now `OwnedStrCell` / `OwnedSliceCell<P<Node>>` (tsrs_core: an `OwnedCell` of a
`&'static str` / `&'static [T]` stored as a 4-byte-aligned pointer + `u32` length, 12 bytes, like `SliceCell`), so
they pack with the two `u32` flag words. Same `get` / `set`, `get` returns exactly what was set; no call site
changed.

| run (3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, before | 8.133 | 317-319 G |
| single, after | 8.040 (-0.09) | 315-318 G |
| 4 checkers, before | 10.92-10.97 | 424-429 G |
| 4 checkers, after | 10.79-10.84 (-0.12) | 424-427 G |

### 2. `SymbolTable` entries 24 -> 16 bytes

Instrumented once: of 16.7M new symbol-table entries on Project single, 16,684,348 store the symbol's own name
string as the key, 4,399 an equal string at another address, 1 a different text. Symbol names never change after
`Symbol::new`. So an entry is now (symbol, `u32` hash of the key, `u32` key length) and the key is read from the
symbol; a key with other text than its symbol's name (an insert, or a `set` that replaces the symbol of an
existing key) goes to a side list `SymbolMapExtra::odd_keys` (boxed together with the hash index; the table header
stays 32 bytes). Lookups compare length and hash before reading the symbol (linear tables hash the probe only once
an entry of the same length turns up). Iteration order, `set`-keeps-key and `delete` shifting are unchanged; the
unit test covers odd keys across deletes. A variant that stored the key's first four bytes instead of a hash
(no hashing in linear tables) retired more instructions (more symbols read and compared), so the hash stayed.

| run (2-3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, before | 8.037-8.040 | 315-319 G |
| single, after | 7.819 (-0.22) | 320-325 G (+1.5%) |
| 4 checkers, before | 10.80-10.85 | 424-426 G |
| 4 checkers, after | 10.52-10.56 (-0.30) | 431-433 G |
| opt-out single / 4 checkers (go assignment), before | 10.89 / 16.80 | |
| opt-out single / 4 checkers (go assignment), after | 10.31 / 15.92 | |

Check times did not move outside the noise of the shared machine (cycles 97-112 G for both binaries single).

### 3. `TypeMapper` 24 -> 16 bytes

17.3M mappers single (Composite, Simple, Array, Inference, Merged make up nearly all). Mapper identity is
observable (`find_active_mapper` compares pointers, and `compare_type_mappers` short-circuits on identity), so
interning them (candidate 3 of the brief) is out; their size is not. A mapper is now two words: the kind in the
low three bits of the first (all payload pointers are 8-aligned, compile-time asserted), the array kinds' list
lengths in the top 16 bits of the two slice pointers (checked at construction: a pointer above 2^48 or a list
longer than `u16::MAX` makes the mapper out of line), and the rare kinds (function mappers, deferred mappers, long
lists) behind a pointer to a `RareTypeMapper`. `TypeMapper::data()` decodes into the `TypeMapperData` view
(`Array` and `ArrayToSingle` now carry full slices), so `map`, `kind`, `maps_this_only` and
`compare_type_mappers` read the same sources, targets and lengths as before. Pointers are tagged with
`map_addr` (strict provenance).

| run (2 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, before | 7.818-7.819 | 321-324 G |
| single, after | 7.690-7.693 (-0.13) | 321-323 G |
| 4 checkers, before | 10.52-10.57 | 431-432 G |
| 4 checkers, after | 10.35-10.36 (-0.19) | 432 G |
| opt-out single / 4 checkers (go assignment), after | 10.15 / 15.57 | |

### 4. Lazy member tables: shared type-argument list, slices, a `SymbolTable` for instantiated members

1.04M lazy member tables single (1.49M on 4 checkers), most alive until exit. Each held its type arguments
twice (a `Vec` and the arena copy given to its mapper), its base types and unaffected names in `Vec`s with growth
slack, and its instantiated members in an `FxHashMap<&str, P<Symbol>>`. Now the table keeps the mapper's arena
slice, `base_types` is an arena slice, `unaffected` a boxed slice, and `declared` a `SymbolTable` (16-byte
entries, the key is the member's name). Only lookups by name and inserts touch `declared`, so nothing depends on
its order.

| run (2-3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, before | 7.693 | 321-323 G |
| single, after | 7.603 (-0.09) | 319-322 G |
| 4 checkers, before | 10.32-10.35 | 431-432 G |
| 4 checkers, after | 10.19-10.21 (-0.14) | 431-432 G |

With `declared` left as the `FxHashMap` (the rest as above) the peak is 0.02 GiB higher; check times of the three
binaries were indistinguishable on the shared machine (load 10-40). The opt-out mode has no lazy tables (10.15 /
15.59 GiB, unchanged).

### 5. Inference contexts 128 -> 64 bytes, inference infos 96 -> 48 bytes

The brief's candidate 4 (recycling contexts through a free list) is out: a context escapes through its fixing /
non-fixing mappers (`TypeMapperData::Inference` holds the context), and those mappers end up in instantiated
types (`ObjectType.mapper`, symbol links), so a context cannot be proven dead after the call that made it.
Its layout can shrink instead. Counted once on Project single (default mode): of 1.43M contexts, 45.5K set a
return mapper, 24.6K an outer return mapper, 8.7K collect intra-expression sites, and none gets inferred type
parameters (only higher-order generic-function inference adds them); of 1.78M inference infos 635K ever get a covariant
candidate and 40K a contravariant one.

- `InferenceContext`: the four rare fields moved into a tail (`InferenceContextRare`, allocated on the first
  non-default write; accessors `return_mapper()` / `set_return_mapper()` & co. return the zero value when it is
  absent, 14 call sites), and `inferences` is a `SliceCell` packed with `flags`.
- `InferenceInfo`: `candidates` / `contra_candidates` are `LazyVec`s (8 bytes; the `RefCell<Vec>` is allocated in
  the arena by the first push), with `push` / `contains` / `clear` / `is_empty` / `to_vec` in place of the
  `borrow()` forms (16 call sites).

| run (2 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, before | 7.603 | 320 G |
| single, after | 7.447 (-0.16) | 320-321 G |
| 4 checkers, before | 10.19 | 432-433 G |
| 4 checkers, after | 9.98-10.00 (-0.20) | 432 G |
| opt-out single / 4 checkers (go assignment), before | 10.15 / 15.59 | |
| opt-out single / 4 checkers (go assignment), after | 10.00 / 15.38 | |

### 6. `StructuredType` 64 -> 56 bytes: the node builder's abstract-construct-signature cache moves to the checker

`objectTypeWithoutAbstractConstructSignatures` is a cache that only `getResolvedTypeWithoutAbstractConstructSignatures`
(node builder) reads and writes, for few types, but every object, reference, union and intersection type (7.6M
single) carried it. It is now a checker map keyed by the type (types belong to one checker; same lookups, same
values). TypeAlloc<ObjectType> 120 -> 112, <TypeReference> 144 -> 136, <UnionType> 192 -> 184.

| run (2 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, before | 7.447 | 320-321 G |
| single, after | 7.390 (-0.06) | 320-322 G |
| 4 checkers, before | 9.98-9.99 | 431-433 G |
| 4 checkers, after | 9.91-9.92 (-0.07) | 432-433 G |
| opt-out single / 4 checkers (go assignment), after | 9.94 / 15.30 | |

### 7. `StructuredType` 56 -> 48 bytes: structured types' base constraints in a checker map

`getResolvedBaseConstraint` stores its result in the type (`ConstrainedType.resolvedBaseConstraint`, embedded in
every structured type in Go), but only 321K of the 7.6M structured types single ever get one (counted once).
Structured types no longer embed `ConstrainedType`; their base constraint lives in
`Checker::structured_type_base_constraints`, read and written through `resolved_base_constraint_of` (also used by
the circularity check in `has_type_resolution`). Type parameters, indexed-access, conditional and the other
constrained types keep the field. Same reads, same writes, same order. TypeAlloc<ObjectType> 112 -> 104.

| run (3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, before | 7.390-7.391 | 320-322 G |
| single, after | 7.344 (-0.05) | 320-322 G |
| 4 checkers, before | 9.90-9.91 | 432-435 G |
| 4 checkers, after | 9.83-9.85 (-0.06) | 432-433 G |
| opt-out single / 4 checkers (go assignment), after | 9.89 / 15.18 | |

### 8. Symbol tables searched linearly up to 16 entries

With step 2 a linear scan compares only the stored lengths and hashes (no symbol is read until one matches), so
the hash index pays off later. Tables with 9-16 entries no longer get one (a boxed `HashTable<u32>`).

| run (2 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, 8 (before) | 7.344 | 321-323 G |
| single, 16 | 7.316 (-0.03) | 322-323 G |
| single, 32 | 7.296 (-0.05) | 322-324 G |
| 4 checkers, 8 (before) | 9.84-9.85 | 431-432 G |
| 4 checkers, 16 | 9.81 (-0.04) | 432-434 G |
| 4 checkers, 32 | 9.77-9.78 (-0.07) | 434-435 G |

16 landed (32 retired +0.6% instructions on 4 checkers for the extra 0.03 GiB). Opt-out after: 9.87 / 15.18 GiB.
