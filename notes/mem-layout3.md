# mem-layout3: what is left in pure layout

Follow-up to notes/mem-layout.md, mem-round2.md, mem-round3.md and mem-small.md. Representation only: no checker
semantics, no change in creation counts. Question: how much of the private monorepo's peak is still pure layout
(padding, 16-byte slices, fields that are almost never set), and what is cheap to take.

Gates for every step, against the binary of the previous step (and the final head against the base): the suite with
`--baselines types,symbols`, whole `target/test-results` trees compared (timings in `summary.json` ignored), in the
default mode, with `TSRS_LAZY_MEMBERS=0` and with `TS_TEST_PROGRAM_SINGLE_THREADED=false` (13,458 error and 12,779
types/symbols baselines pass, unchanged); fourslash 4,066 pass / 63 fail with identical result trees; the private
monorepo's output including the `--extendedDiagnostics` counters identical in default / opt-out x 1 / 4 checkers
(opt-out 4 checkers with `--checkerAssignment go`; the project reports 19 errors from stale workspace builds before
and after). `RUSTFLAGS="-D warnings" cargo +1.99.0 check --workspace --locked` clean.

Measurement: `/usr/bin/time -l`, base and candidate interleaved, medians of 3, GiB = peak memory footprint / 2^30.
The machine was shared with four other experiments (load 30-75), so instructions retired vary by about +-1% between
identical runs (the per-step instruction deltas below are within that noise unless noted).

## Profile at the start (branch point de3beaf, alloc-profile, single checker, default mode)

Arena 4,326 MB requested, Rust heap outside the arena 1,570 MB live, peak 5.74 GiB. Every row above 20 MB:

| MB | count | B/each | what | layout slack found |
| --- | --- | --- | --- | --- |
| 506 | 13.26M | 40 | `Symbol` | 4 bytes of padding; 40 -> 32 needs a field out (see floor) |
| 256 | 16.78M | 16 | `TypeMapper` | none (two words) |
| 241 | 7.90M | 32 | `NodeAlloc<Identifier>` | none (header + one packed word) |
| 239 | 2,550 chunks | 24/value | value-symbol links | none (three words, mem-round3 A9) |
| 218 | 12.32M | 18 | `[P<Type>]` type lists | none (count is semantics) |
| 149 | 2.17M | 72 | `TypeAlloc<TypeReference>` | `resolved_type_arguments` a 16-byte `Option<&[T]>` |
| 144 | 3.14M | 48 | `TypeAlloc<ObjectType>` | none |
| 136 | 1.62M | 88 | `Signature` | two 12-byte packed slices; `resolvedTypePredicate` is the `noTypePredicate` sentinel or nil for all but a few |
| 126 | 2.74M | 48 | `StructuredMembers` | three 12-byte slices; index infos on 2% |
| 114 / 98 | 1.07M / 1.17M | 112 / 88 | `TypeAlloc<UnionType>` / `<IntersectionType>` | 7 / 5 fields after `types` that are set on 0.02-21% (counted below) |
| 107 | 9.13M | 12 | `[P<Node>]` | none |
| 98 | 2.82M | 36 | `[P<Symbol>]` | none |
| 95 | 4.15M | 24 | `NodeList` | 16-byte slice + range |
| 92 / 85 | 1.50M / 1.85M | 64 / 48 | `InferenceContext` / `InferenceInfo` | 6 bytes of padding in the info; mostly recycled already |
| 84 | 3.65M | 24 | `SymbolTable` | none |
| 76 / 71 / 68 | | 64 / 72 / 56 | `NodeAlloc<CallExpression>` / `<PropertyAssignment>` / `<PropertyAccessExpression>` | rarely set optional children (main's front-end pass moved them to a tail) |
| 71 | 0.66M | 112 | `TypeAlloc<ConditionalType>` | 3 fields set on 18-35% (3-5 MB if moved out; not done) |
| 59 | 1.93M | 32 | `FlowNode` | `antecedent` / `antecedents` never both set |
| 28 | 203K | 144 | `FileIncludeReason` | 88-byte rarely set inline option (main's front-end pass boxed it) |
| 24 | 0.79M | 32 | `RefCell<Vec<P<Type>>>` candidate lists | (inference, recycled) |

Heap (sampled, live at exit): file texts 214 MB, symbol table entries 207 MB (of which ~56 MB unused capacity in
tables of at most 16 entries: 1.36M one-entry and 0.93M two-entry tables got 4-entry buffers), lazy member tables
132 MB, pointer-keyed link stores 93 MB, relation caches 91 MB, object / conditional instantiation maps 74 + 72 MB.
On 4 checkers the same rows scale with the checker-owned data (arena 5,531 MB, `Symbol` 661 MB, mappers 401 MB,
type lists 345 MB).

Occupancy counted once (throwaway instrumentation, single checker): of 7.86M structured types 2.74M are resolved;
of those 1.18M have signatures and 55K index infos. Unions (1.07M): origin 12%, key property name 9.5%, property
cache / resolved properties / reduced / regular type 5-6% each, constituent map 211. Intersections (1.17M):
non-augmented property cache 21%, apparent type 14%, resolved properties 4.4%, unique-literal instantiation 3.4%.
Signatures (1.62M): `target` / `mapper` 83%, a rare tail 3.5%. Type references: `node` 2.9%.

## Steps

Per step against the previous step's binary (3 interleaved rounds each; base of step 1 = de3beaf):

| step | single GiB | 4 checkers GiB | instructions single / 4 |
| --- | --- | --- | --- |
| 1. `ThinSlice`: slice cells and node lists in one word | -0.110 (-1.9%) | -0.135 (-1.8%) | +0.4% / +0.4% |
| 2. `Signature`: `noTypePredicate` as a bit | -0.022 (-0.4%) | -0.021 (-0.3%) | +0.1% / 0.0% |
| 3. union / intersection rare fields in a kind-specific tail | -0.073 (-1.3%) | -0.094 (-1.3%) | +0.5% / +0.3% |
| 4. `StructuredMembers` 40 -> 32 | -0.011 (-0.2%) | -0.008 (-0.1%) | noise |
| 5. symbol table entries grow from 1 (superseded by main) | -0.051 (-0.9%) | -0.048 (-0.7%) | +0.4% / +0.1% |
| 6. `FileIncludeReason` boxed rare data (superseded by main) | -0.014 (-0.3%) | -0.017 (-0.2%) | noise |
| 7. `FlowNode` 32 -> 24 | 0.000 | -0.016 (-0.2%) | +0.2% / +0.3% |

### 1. `ThinSlice` (`tsrs_core::ptr`)

A `&'static [T]` in one word: the data pointer in the low 48 bits and the length in the high 16, like `PackedStr`.
A list of 2^16 elements or more (or at an address above 2^48) is stored as a pointer, tagged with bit 0, to an
arena copy of the `&[T]` itself, so `get` returns exactly the slice that was stored (same pointer, same length) in
both forms; `T` must be at least 2-aligned (compile-time check). The word is never zero, so `Option<ThinSlice<T>>`
is one word too (`OptionThinSliceCell`). Used for `NodeList.nodes` (24 -> 16 bytes; `nodes` became private,
`NodeList::new(loc, nodes)` builds one, ~310 `.nodes` field reads across the workspace became `.nodes()`), `Signature`'s two lists
(88 -> 80), `StructuredMembers`' three lists (48 -> 40), union/intersection `types` and `resolvedProperties`,
`TypeReference.resolvedTypeArguments` (16-byte `Option<&[T]>` -> 8), `TypeAlias.typeArguments`.

The 12-byte packed `SliceCell` stays where it packs with 4-byte fields for free (`Symbol.declarations`,
`InferenceContext.inferences`). A first version replaced it everywhere, including `Symbol`: no gain there (40 bytes
with 4 bytes of padding either way) and +1.6% / +1.2% instructions (single / 4 checkers), from `get_type_arguments` and the other hot
readers decoding a length that `ThinSlice` read with a compare-and-branch for the long form; the landed `get` tests
bit 0 (one predictable branch, the long form out of line), and that version measured +0.4%.

### 2. `Signature` 80 -> 72 bytes

`resolvedTypePredicate` is nil until resolved, then the checker's `noTypePredicate` sentinel for every signature
without a predicate. The sentinel is now bit 0 of the rare-tail word; a real predicate lives in the tail
(`SignatureRare`, which already held `thisParameter`, `isolatedSignatureType`, `composite`). Accessors take the
sentinel: `resolved_type_predicate(no_type_predicate)` returns `Some(no_type_predicate)` when the bit is set, and
`set_resolved_type_predicate(p, no_type_predicate)` sets the bit exactly when `p` is the sentinel. All 13 uses are in
`getTypePredicateOfSignature` and `newSignature`.

### 3. Unions and intersections: 112 -> 48 and 88 -> 48 bytes

Everything after `types` moved into a tail allocated on the first non-nil write: `UnionOrIntersectionRare`
(resolved properties, both property caches) at the start of `UnionRare` (reduced, regular, origin, key property
name, constituent map) or `IntersectionRare` (apparent type, unique-literal instantiation), `repr(C)`. The kind is
bit 0 of the tail word, set when an intersection's data is constructed, so the shared accessors
(`resolved_properties`, `property_cache(skip_augment)`, `property_cache_for_write`) allocate the right tail. Getters
return the zero value without a tail (nil, `""`, a nil map), like the unset Go fields; 44 call sites changed
mechanically from field access to accessors. Unions with any rare field: 103K of 1.07M; intersections 265K of
1.17M.

### 4. `StructuredMembers` 40 -> 32 bytes

The call-signature count and the index infos share a word: `count << 1 | 1` while no non-empty index-info list was
set, else a pointer to `{index_infos, call_signature_count}`. One difference from "returns exactly what was set": an
empty index-info list set before any non-empty one reads back as `&[]` (Go: a nil or empty slice, never compared by
identity). Small (22 MB expected, within noise on the peak); kept because it costs nothing measurable.

### 5 and 6 (superseded)

Symbol table buffers grew 4, 8, ... from the first insert; growing 1, 2, 4, ... saved 45-56 MB (most tables hold one
or two entries). `FileIncludeReason` carried an 88-byte `Option<automaticTypeDirectiveFileData>` inline (203K
reasons, 144 -> 64 bytes). Main's front-end pass (278b139) landed both independently (same capacities; the data
behind a `P`), so the merge keeps main's versions and these two steps drop out of the branch's diff.

### 7. `FlowNode` 32 -> 24 bytes

Go sets `Antecedent` on every flow node except labels and `Antecedents` only on labels (created without an
antecedent), so they share one word: the antecedent, or the antecedent list with bit 0 set. `FlowNode::new(flags,
node, antecedent, text_index)`, `antecedent()`, `antecedents()`, `set_antecedents()` (asserts the node has no
antecedent). 1.93M flow nodes, 15 MB expected; the peak delta is inside the noise (the census was running beside
this measurement).

## Census free-gate

Main's census now takes field layouts registered with `tsrs_core::census_layout` (b0602ae, docs/DEBUGGING.md "census
free-gate"); the merge replaced this branch's earlier hard-coded census edits. The branch registers its encodings
instead, with two new `CensusField` kinds: `Thin { off }` (a `ThinSlice` word: a reference only while the length bits
are non-zero, or with bit 0 the long list's `&[T]` record) and `LowTag { off, mask }` (an address with low tag bits).
Registered: `NodeList` / `ModifierList`, the slices of `Signature`, `StructuredMembers` (and its index-info tail),
unions / intersections and their tails, `TypeReference` / `InterfaceType` / `TupleType` type arguments, `TypeAlias`,
and the tagged words of the union / intersection tail, the signature tail and the flow node link. All from
`offset_of!`.

Final head, the private monorepo, `TSRS_CENSUS=1 TSRS_CENSUS_VERIFY=1`, default and `TSRS_LAZY_MEMBERS=0`, 1 and 4
checkers: precise walk 50,362,689 references, 0 to freed or rewound blocks; strong mark **0 violations** in all four
runs (27 registered layouts, 208 arena classes covered). Positive control: with the escape barrier removed from
`MapperCell::set`, single checker reports 835,737 violations (freed mappers held by `ConditionalType.mapper` /
`combinedMapper`), so the registrations do not hide real pointer fields. Conformance corpus (12,758 files through `tsrs --strict --target esnext`, final census binary, 10 in parallel):
precise walk 0 and strong mark 0 in every file. 9 files did not report 0 on the first run under the parallel load
(a missing or non-zero line) and reported 0 on the retry and on three more single runs each.

## Result

Interleaved, medians of 3 (opt-out: 1 run), base = current main (b0602ae; 0.2.2 differs only in version strings),
final = this branch merged with main. The machine was otherwise idle for this measurement.

| run | main peak GiB | final peak GiB | main instructions | final instructions |
| --- | --- | --- | --- | --- |
| default, single | 5.474 | 5.272 (-0.203, -3.7%) | 309.0 G | 311.0 G (+0.6%) |
| default, 4 checkers | 7.304 | 7.022 (-0.282, -3.9%) | 422.8 G | 425.3 G (+0.6%) |
| opt-out, single | 7.312 | 7.094 (-0.218, -3.0%) | 332.9 G | 335.2 G (+0.7%) |
| opt-out, 4 checkers (go assignment) | 11.166 | 10.813 (-0.353, -3.2%) | 502.2 G | 505.8 G (+0.7%) |

Against the branch point de3beaf (before main's front-end pass), the same comparison was -0.270 GiB (-4.7%) single
and -0.359 GiB (-4.7%) on 4 checkers; the difference is the two steps main landed independently (5 and 6) and
overlap with main's NodeList-adjacent front-end savings. Cost: ~0.16% instructions per 1% of peak overall; steps 3
(+0.5% single / +0.3% on 4 checkers for 1.3%) and 7 (single-threaded instruction medians 310.3 G vs 310.0 G, i.e.
within noise, for 0-0.2%) are the ones closest to the stop rule.

## Not done, and the floor

Rejected or left after counting:

- `ConditionalType` rare fields (resolved inferred-true type, default constraint, distributive constraint; set on
  18-35% of 665K): 3-5 MB.
- `TypeParameter` (`target` / `mapper` on 89%), `ObjectType` (`target` / `mapper` on 57%), `IndexedAccessType`:
  nothing rare enough.
- `InferenceInfo` padding (6 bytes): most infos are recycled (mem-recycle), so the live share is small.
- `Signature.resolvedMinArgumentCount` / `minArgumentCount` as `i16`: would need range checks on every write for
  ~6 MB.

The floor in pure layout is close. What is left above 20 MB is either already minimal for what it holds (mappers,
type lists, link records, identifiers, symbol tables) or needs a semantic change. The next 5% (~0.27 GiB single,
~0.36 GiB on 4 checkers) is not available from layout alone; candidates, each a project rather than a step:

1. `Symbol` 40 -> 32 bytes (-100 MB single, -130 MB on 4 checkers): every word is used (flags, check flags, name,
   declarations, id, parent/tail) plus 4 bytes of padding. Needs one 4-byte field out: the lazily assigned id
   (written atomically by any checker; a side table keyed by address costs a hash per `getSymbolId`), or the check
   flags of binder symbols (always 0 there, but transient symbols use them on every access).
2. Type lists and mappers (~470 MB single): their count is checker semantics (created on instantiation-cache misses);
   only fewer instantiations shrink them (the use-census / laziness experiments).
3. `[ValueSymbolLinks]` (239 MB): three words per record already; fewer records needs lazier link creation.
4. Source text (214 MB heap) and identifiers (241 MB): the text is needed for diagnostics and identifiers; a
   compact AST (32-bit child indices instead of 8-byte pointers, mem-frontend's floor) would halve every node, but
   that is a rewrite of `P<Node>`.
