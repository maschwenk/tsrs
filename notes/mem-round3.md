# mem-round3: peak memory, third pass (representation and lazy creation)

Follow-up to notes/mem-layout.md, notes/mem-lazy.md and notes/mem-round2.md. Track A changes only the
representation (no checker semantics); track B prototypes Go-portable "created and never used" candidates on the
type side behind `TSRS_LAZY_*` switches.

Gates for every step: the suite (errors and `--baselines types,symbols`) byte-identical to the base commit, as whole
`target/test-results` trees, in the default mode, with `TSRS_LAZY_MEMBERS=0`, and with
`TS_TEST_PROGRAM_SINGLE_THREADED=false`; opt-out counters 25,973,354 / 9,639,962 / 44,884,281 single and
39,704,001 / 16,200,921 / 89,981,648 with `--checkers 4 --checkerAssignment go`; default counters unchanged
(12,811,032 / 9,630,120 / 44,820,708 single, 16,549,988 / 13,788,912 / 76,888,800 on 4 checkers); Project 0 errors.

Measurement: `/usr/bin/time -l` on the pristine Project checkout, rounds interleaved, medians of 3, "GiB" = peak
memory footprint / 2^30. The machine was shared with two other agents (load 8 to 60), so wall and check times are
noise; instructions retired are the CPU-work measure.

## Occupancy at the start (main at dc59d8e, default mode, single)

Counted once with throwaway instrumentation (all types registered at creation, all value-symbol link slots walked
at exit, relation caches inspected):

| structured types | count | any resolved member set |
| --- | --- | --- |
| object (anonymous, instantiated) | 3,006,176 | 2,194,820 (73%) |
| type references | 2,106,919 | 197,751 (9%) |
| intersections | 1,133,593 | 25,399 (2%) |
| unions | 1,020,584 | 660 (0.06%) |
| mapped | 272,161 | 153,702 (56%) |
| interfaces, tuples, reverse mapped, ... | 40,545 | 25,343 |

`ValueSymbolLinks` slots: 11.05M, of which 0.98M are never written (all four words empty), 1.14M have only
`resolved_type`, 1.67M only `target` + `mapper`, 3.05M `resolved_type` + `target` + `mapper`, and 2.50M a rare tail.
No two-tier split pays: `target`/`mapper` are set on 61% of the slots.

Relation caches: 5.10M assignable, 0.61M identity, 0.34M strict-subtype, 0.27M subtype, 0.04M comparable entries
(hit rates 47%, 61%, 36%, 23%, 64%). Of 16.06M keys computed, 16.02M have the simple form
(`'s'`, source id, target id, intersection state) and 43.7K the generic-reference form (`'g'`).

## Track A

### A1. Resolved members of structured types in a record allocated on first write

`StructuredType` (every object, reference, union and intersection type) held Go's five resolved-member fields
inline (48 bytes). They now live in `StructuredMembers`, allocated by the first setter call; the type keeps one
pointer. Getters return the zero values (nil members, empty slices, count 0) while it is absent, exactly what the
unset fields read; after the first write every getter returns exactly what was last set (any write allocates, so
an empty slice that was set is returned as set). 4.9M of 7.6M structured types are never resolved. Call sites
changed mechanically from `.members.get()` / `.members.set(..)` & co. to `members()` / `set_members(..)`.
TypeAlloc<ObjectType> 96 -> 56 bytes, <TypeReference> 120 -> 80, <UnionType> 160 -> 120, <IntersectionType>
136 -> 96; resolved types pay 8 bytes more (pointer + 48-byte record).

| run (3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, before | 7.020-7.024 (median 7.021) | 323-325 G |
| single, after | 6.851-6.855 (6.851, -0.17) | 323-326 G |
| 4 checkers, before | 9.39-9.48 (9.410) | 435-439 G |
| 4 checkers, after | 9.13-9.15 (9.151, -0.26) | 435-439 G |
| opt-out single / 4 checkers (go assignment), after | 9.35 / 14.30 | 348 / 523 G |

### A2. Relation cache keys: type-id pairs packed into 8-byte slots

Go keys the relation caches by the 128-bit xxh3 hash of the `getRelationKey` bytes. The simple form
(`'s'`, source id, target id, intersection state; 99.7% of keys) is a triple of small numbers, so
`get_relation_key` now returns `RelationKey::Pair` (source id, target id below 2^28, state below 2^2, packed into
58 bits; an injective mapping) without building and hashing the key bytes, and `RelationKey::Hashed` (the same
128-bit hash as before) for everything else. `Relation` keeps the pairs in a `hashbrown::HashTable<u64>` whose slot
is `key << 6 | result bits` (8 bytes + 1 control byte per slot instead of 17 + 1) and the hashed keys in the
previous map; `size()` counts both. The relater's maybe-key stack and set hold `RelationKey`s. The forms never
alias each other: Go's `'s'` and `'g'` byte strings differ, and a simple key is `Pair` exactly when it fits.

| run (3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, before (A1) | 6.852-6.858 (6.852) | 321-323 G |
| single, after | 6.766-6.770 (6.768, -0.08) | 320-321 G |
| 4 checkers, before (A1) | 9.13-9.18 (9.148) | 435-436 G |
| 4 checkers, after | 8.965-8.991 (8.971, -0.18) | 432-434 G (-0.8%) |
| opt-out single / 4 checkers (go assignment), after | 9.26 / 14.12 | 344 / 515 G (-1.2%, -1.6%) |

### A3. AST node header 32 -> 24 bytes

23.0M nodes on Project (the AST is shared by all checkers). The header held the kind (`u16`), the data tag (`u8`),
flags, the range, the parent pointer and a 64-bit id, 32 bytes with padding. Now the kind (9 bits), the data tag
(8 bits) and the parent (address / 8 in 45 bits: nodes are 8-aligned and user-space addresses are below 2^48;
asserted when a parent is set) share one word, and the id is stored in 32 bits like symbol ids (a 64-bit counter,
`NodeId` stays `u64`, panic past `u32::MAX`). The parent's provenance is exposed on store and recovered with
`with_exposed_provenance`. Only the parent part is written after creation, under the same owner-only contract as
before (`OwnedCell`). `node.kind` became `node.kind()` and `node.parent.get()` / `.set(..)` `parent()` /
`set_parent(..)` (mechanical, 1,950 sites; `tools/gen-ast/gen-ast.ts` emits the new forms, `generated.rs`
regenerated). AST oracle: 113/113 libs and 17,318/17,319 test units identical (the one is the known non-UTF-8
file), as before.

| run (3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, before (A2) | 6.759-6.763 (6.760) | 315-316 G |
| single, after | 6.587-6.592 (6.592, -0.17) | 317-318 G (+0.5%) |
| 4 checkers, before (A2) | 8.96-9.01 (8.966) | 429-431 G |
| 4 checkers, after | 8.79-8.82 (8.812, -0.15) | 431-432 G (+0.4%) |
| opt-out single / 4 checkers (go assignment), after | 9.08 / 13.95 | 339 / 517 G |

### A4. Symbol table entries 16 -> 8 bytes

Counted once (capacity deltas of every `SymbolMap`): 465 MB of entry capacity and 74 MB of hash index on Project
single, 19.7M inserts. An entry was (symbol pointer, 32-bit key hash, 32-bit key length). It is now one word: the
symbol's address / 8 in 45 bits (same platform assumption and provenance handling as A3), an odd-key flag, the
key length capped at 63 (6 bits) and 12 bits of the key hash. Linear scans (tables up to 16 entries) still compare
the length first and hash the probe only when a length matches, then the 12 hash bits, then the text; the hash
index (larger tables) recomputes an entry's full hash from its key when it rehashes (on growth, and when it is
built at 17 entries) or removes. Same order, same `set`-keeps-key and `delete` behavior; a unit test covers keys
longer than 63 bytes in linear and indexed tables.

| run (3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, before (A3) | 6.589-6.594 (6.589) | 318-322 G |
| single, after | 6.372-6.375 (6.373, -0.22) | 319-322 G (+0.3%) |
| 4 checkers, before (A3) | 8.79-8.82 (8.818) | 432-436 G |
| 4 checkers, after | 8.53-8.57 (8.551, -0.27) | 433-434 G (+0.2%) |
| opt-out single, before / after | 9.08 / 8.51 (-0.58) | 340 / 340 G |
| opt-out 4 checkers (go assignment), after | 13.09 (was 13.95) | 524 G |

### A5. No links slot for a read of an object literal member's `nameType`

Walking the value-symbol link slots at exit with their creating call sites: 0.96M of the 0.98M never-written slots
come from one read, `getContextualTypeForObjectLiteralElement` asking for the `nameType` of the element's
declaration symbol. It now uses `try_get` (the store still assigns the symbol its id at the same point, so ids are
unchanged; a missing slot reads as nil like a fresh one). mem-lazy noted the same site; in Go the paged store
allocates the slot with its page either way, so this is tsrs-only.

| run (3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, before (A4) | 6.374-6.378 (6.376) | 316-318 G |
| single, after | 6.344-6.353 (6.345, -0.03) | 316-318 G |
| 4 checkers, before (A4) | 8.51-8.55 (8.541) | 431-432 G |
| 4 checkers, after | 8.48-8.53 (8.525, -0.02) | 432 G |
| opt-out single / 4 checkers (go assignment), after | 8.48 / 13.07 | 341 / 518 G |

### A6. Instantiated type aliases allocated only when a type is created with them

Of the 2.34M `TypeAlias` records `instantiateTypeAlias` made on Project single, 0.55M ended up on a type: the
union/intersection (1.12M made, 0.17M kept), indexed-access (0.11M, 5K kept) and object-instantiation (0.83M,
0.10M kept) paths build the alias before the constructor's cache lookup, and a hit drops it. The constructors that
look up a cache first (`get_union_type_ex` and its worker / sorted-list part, `get_intersection_type_ex`,
`get_indexed_access_type_ex` / `_or_undefined`, `get_object_type_instantiation`) now take an `AliasArg`: none, an
existing alias, or a `PendingTypeAlias` (symbol + instantiated type arguments, instantiated at the same point as
before) that the cache keys read directly and that is allocated, at most once, when a type is created with it.
Alias identity (`typesAreSameReference` compares `TypeAlias` pointers) is unchanged: every created type gets the
one record its call made, as before. Call sites that pass `None` or an existing alias changed mechanically.

| run (3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, before (A5) | 6.341-6.346 (6.346) | 315-316 G |
| single, after | 6.288-6.290 (6.290, -0.06) | 316-317 G |
| 4 checkers, before (A5) | 8.516-8.522 (8.517) | 431 G |
| 4 checkers, after | 8.42-8.47 (8.442, -0.08) | 431-432 G |
| opt-out single / 4 checkers (go assignment), after | 8.42 / 12.98 | 341 / 518 G |

## Track B: lazy creation on the type side

Same conventions as notes/mem-lazy.md: every candidate has a `TSRS_LAZY_*` switch in `tsrs_core::lazymembers`
that only acts when the master switch is on, so `TSRS_LAZY_MEMBERS=0` stays reference-identical.

### Attribution (Project single, default mode, `--features site-counts`)

Types (9.63M) by creating call, with `track_caller` added (locally, not committed) through the union, intersection
and reference constructors so each row names the code that asked:

| types | kind | asked for by |
| --- | --- | --- |
| 1,735,719 | instantiated anonymous | `instantiateAnonymousType` (every instantiation cache miss of a declared anonymous type) |
| 1,046,391 | reference | `instantiateType` of a non-deferred reference (new type arguments) |
| 663,066 | anonymous | `checkObjectLiteral` |
| 632,811 | conditional | `getConditionalType` |
| 598,399 | reference | `getTypeWithThisArgument` of base types when lazy member tables are prepared (mem-lazy: 97.6% needed later) |
| 538,952 | intersection | `getCrossProductIntersections` (distributing an intersection over unions; all go into the result union) |
| 533,607 / 177,373 | union / intersection | `instantiateType` of a union / intersection |
| 307,284 | intersection | `getUnionOrIntersectionProperty` -> `createUnionOrIntersectionProperty`: the type of a synthetic property of an intersection, computed eagerly when it has at most two constituent property types (candidate B1) |

Mappers (17.3M): simple 5.06M, array 4.09M, inference 2.86M, merged 2.77M, composite 2.25M. Largest sites: the
conditional-type instantiation mapper on a cache miss (1.52M), the two mappers of every inference context (1.43M
each, candidate B2), the object-instantiation mapper on a miss (2.07M), lazy member table mappers (1.02M), and the
composite mappers of conditional types with `infer` (0.65M + 0.65M). Type lists (13.7M, 230 MB): union and
intersection constituent lists, reference type arguments and the mapper target lists of the sites above, each
belonging to an object created on a cache miss.

Most rows are objects created on a cache miss and kept by the type they built, so they are "used" by construction.
Two had a measurable never-used share:

### B1 (rejected): defer the type of two-constituent union/intersection properties (`TSRS_LAZY_PROP_TYPES`, removed)

`createUnionOrIntersectionProperty` computes the property type eagerly unless there are more than two constituent
types (then it sets `CheckFlags::DeferredType` and keeps the constituents in `deferredSymbolLinks`). Counted with
throwaway instrumentation: of the eagerly typed properties, 159K are ever read through `getTypeOfSymbol`, and the
unread ones made 339K new intersection types (541K properties) and 25K new unions (67K properties). Deferring the
two-constituent case as well (`len > 2 || len == 2`) gave 9,630,120 -> 9,275,217 types (-3.7%) but peak only
6.289 -> 6.275 GiB (the deferred-symbol links and constituent lists cost nearly what the types did), and it changed
results: Project reports TS2578 (unused `@ts-expect-error`) in `handler.test.ts`, because code that
reads `links.resolvedType` directly (e.g. `isSymbolUnaffectedByInstantiation`) and the deferred-type paths behave
differently. Not pursued.

### B2 (landed, default on): inference context mappers on first use (`TSRS_LAZY_INFERENCE_MAPPERS`)

Go's `newInferenceContextWorker` creates `context.mapper` and `context.nonFixingMapper` with every context. Now
`InferenceContext::mapper()` / `non_fixing_mapper()` create them on the first call (once, so the identity that
`findActiveMapper` and `compareTypeMappers` see is stable; nothing else about a mapper is observable). Of 1.43M
contexts on Project, 0.93M ever use the non-fixing mapper and 0.80M the fixing one: inference mappers 2.86M ->
1.73M. Upstream this saves the same 1.13M allocations per Project check (Go: allocation/GC work, not retained
memory).

```go
 func (c *Checker) newInferenceContextWorker(inferences []*InferenceInfo, signature *Signature, flags InferenceFlags, compareTypes TypeComparer) *InferenceContext {
 	n := &InferenceContext{inferences: inferences, signature: signature, flags: flags, compareTypes: compareTypes}
-	n.mapper = c.newInferenceTypeMapper(n, true /*fixing*/)
-	n.nonFixingMapper = c.newInferenceTypeMapper(n, false /*fixing*/)
 	return n
 }
+
+func (c *Checker) getFixingMapper(n *InferenceContext) *TypeMapper {
+	if n.mapper == nil {
+		n.mapper = c.newInferenceTypeMapper(n, true /*fixing*/)
+	}
+	return n.mapper
+}
```
(and `getNonFixingMapper` likewise; the ~15 reads of `n.mapper` / `n.nonFixingMapper` go through them.)

| run (3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, before (A6) | 6.284-6.286 (6.286) | 316-317 G |
| single, after | 6.267-6.268 (6.268, -0.02) | 316-318 G |
| 4 checkers, before (A6) | 8.39-8.43 (8.430) | 432-433 G |
| 4 checkers, after | 8.39-8.41 (8.405, -0.03) | 432 G |
| opt-out single / 4 checkers (go assignment), after | 8.43 / 12.98 | 342 / 519 G |

Gates: suite trees identical in all three modes, counters unchanged (mappers are not counted).

### A7. Reference instantiation tables store only the references

After A1-A6 the instantiation caches are the largest heap item (Project 4 checkers: conditional 135 MB, references
112 MB, object types 99 MB). For generic class, interface and tuple targets the map is keyed by
`getTypeListKey(typeArguments)` and every value is a non-deferred reference whose `resolved_type_arguments` are
exactly that list (the target itself for its type parameters) and never change. `InterfaceType.instantiations` is
now a `ReferenceInstantiations`: a `hashbrown::HashTable<P<Type>>` hashed over the type ids and compared by list
equality, one word per slot instead of a 128-bit key plus the value, and no xxh3 over the key bytes per lookup.
List equality is the injective version of Go's key (Go relies on the 128-bit hash having no collisions). The
conditional and object-type caches map keys to arbitrary result types, so they keep their keys.

| run (3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, before (B2) | 6.266-6.275 (6.269) | 317-319 G |
| single, after | 6.208-6.212 (6.209, -0.06) | 316-318 G |
| 4 checkers, before (B2) | 8.375-8.416 (8.398) | 433-434 G |
| 4 checkers, after | 8.331-8.353 (8.336, -0.06) | 430-433 G |
| opt-out single / 4 checkers (go assignment), after | 8.37 / 12.91 | 344 / 520 G |

### A8. Synthetic symbols keep containing and name types in the words of target and mapper

Walking the rare tails of the value-symbol links at exit: 2.15M of the 2.50M tails belong to records that set
`containing_type` (and often `name_type`) but never `target` or `mapper`: union/intersection properties (0.88M,
containing type only) and mapped type members (1.24M, both). Such a record now switches to "synthetic" mode
(bit 0 of the tail word): the words that hold `target` and `mapper` hold `containing_type` and `name_type`, and it
needs no tail. A later non-nil `target` / `mapper` write moves the two into the tail and switches back; a record
that already has a target, a mapper or those fields in its tail keeps the old layout. `target` and `mapper` became
accessors (`target()`, `set_target(..)`, 18 call sites). A unit test walks the mode changes.

| run (3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, before (A7) | 6.211-6.222 (6.218) | 316-317 G |
| single, after | 6.142-6.155 (6.153, -0.07) | 316-317 G |
| 4 checkers, before (A7) | 8.28-8.34 (8.321) | 432-433 G |
| 4 checkers, after | 8.20-8.24 (8.240, -0.08) | 431-432 G |
| opt-out single / 4 checkers (go assignment), after | 8.25 / 12.71 | 341 / 517 G |

### A9. Value-symbol links in three words

After A8 only 0.35M of the ~10M link records have a tail, so the tail word is almost always nil. The record is
now `resolved_type` plus two mode-dependent words, the mode in the low two bits of the second (stored pointers are
8-aligned): plain (`target`, `mapper`), synthetic (`containing_type`, `name_type`; A8) or tail (a pointer to a
`ValueSymbolLinksTail` holding all six other fields; the second word is just the mode). A record moves to synthetic
or tail mode on the first write its mode cannot hold and never moves back. 32 -> 24 bytes per record; tail records
pay 48 bytes for the tail instead of 32.

| run (3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, before (A8) | 6.148-6.158 (6.152) | 315-317 G |
| single, after | 6.083-6.085 (6.083, -0.07) | 316-317 G |
| 4 checkers, before (A8) | 8.247-8.256 (8.251) | 430-431 G |
| 4 checkers, after | 8.127-8.149 (8.139, -0.11) | 430-431 G |
| opt-out single / 4 checkers (go assignment), after | 8.08 / 12.45 | 341 / 519 G |

### A10. Pointer-keyed link stores: 12-byte slots, values in chunks

Go's `core.LinkStore` is `map[K]*V`; ours was `FxHashMap<P<K>, P<V>>` with each value a separate arena
allocation. Sizes at exit on Project single (entries / table slots): signature links 1.47M / 1.84M, mapped-symbol
links 1.14M / 1.84M, type-node links 0.87M / 0.92M, symbol-reference links 0.68M / 0.92M, members-and-exports
0.27M, node links 0.24M, alias links 0.20M, 19 smaller stores; 5.1M links. Ids are not an option here (Go assigns
none for these keys, and node ids are observable). A slot is now the key address plus the value's `u32` index
(12 bytes, 4-aligned, in a `hashbrown::HashTable`) and the values live in 1024-value arena chunks in first-access
order (stable addresses, as before); `get` finds or inserts with one hash. A first version that looked up twice on
a miss and bounds-checked the chunk access retired +0.6% instructions; this one +0.3%.

| run (3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, before (A9) | 6.079-6.082 (6.079) | 316 G |
| single, after | 6.045-6.057 (6.053, -0.03) | 317 G (+0.3%) |
| 4 checkers, before (A9) | 8.10-8.14 (8.102) | 430-431 G |
| 4 checkers, after | 8.05-8.10 (8.096, -0.01) | 431-432 G |
| opt-out single / 4 checkers (go assignment), after | 8.04 / 12.43 | 341 / 518 G |

### A11. Symbol table header 32 -> 24 bytes

3.49M `SymbolTable`s on Project single (arena, 32 bytes each: a `Vec` of entries plus the boxed index/odd-key
extra). The entries are now an `EntryVec`: pointer plus `u32` length and capacity, same growth as `Vec` (`push`
doubles from 4, `reserve_exact` adds exactly, `clone` allocates the length). `SymbolMap` 24 bytes.

| run (3 interleaved rounds) | peak GiB | instructions |
| --- | --- | --- |
| single, before (A10) | 6.048-6.054 (6.050) | 316 G |
| single, after | 6.013-6.029 (6.013, -0.04) | 316-317 G |
| 4 checkers, before (A10) | 8.097-8.111 (8.109) | 431-433 G |
| 4 checkers, after | 8.036-8.068 (8.046, -0.06) | 431-432 G |
| opt-out single / 4 checkers (go assignment), after | 8.01 / 12.37 | 340 / 518 G |
