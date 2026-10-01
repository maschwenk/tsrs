# perf-memory: peak memory on Project

Targets (Go reference at the same checker count): 4 checkers <= 25 GB, `--singleThreaded` <= 17 GB.
Acceptance per change: `--extendedDiagnostics` counters identical, conformance pass lists identical (errors,
`.types`, `.symbols`), no wall-time regression.

## Allocation profile (opt-in)

Build: `CARGO_TARGET_DIR=$PWD/target/prof cargo build --release -p tsrs_cli --features alloc-profile` (was `--features tsrs_core/alloc-profile` before mimalloc; see notes/mem-layout.md).
Compiled out otherwise (`#[cfg_attr(feature, track_caller)]`, the counters are `#[cfg]`).

- Every arena allocation (`P::new`, `alloc`, `alloc_slice`, `alloc_vec`, `alloc_str`) is recorded per (call site,
  element type); the per-type table doubles as the `size_of` x count table (`B/each` = `size_of::<T>()` for
  `P::new`). Call sites are one level deep: allocations through a wrapper (`Symbol::new`, `new_node`, link stores)
  show the wrapper's line; the type column tells them apart.
- A counting global allocator reports the Rust heap (which includes the arena's chunks).
- `TSRS_HEAP_PROFILE=1`: sampling heap profiler (one raw stack per ~256 KB allocated, live bytes per stack,
  resolved with `atos` at exit; arena chunk allocations are reported as one `<arena chunks>` row). Slow (~45 s
  check instead of 27 s) but fine for a profile run.
- `TSRS_ALLOC_PROFILE_TOP` / `TSRS_HEAP_PROFILE_TOP` set N for the top-N tables. Output goes to stderr at exit.

## Project, single-threaded, before (main at 5ca48f3)

Peak footprint 19.53 GB. Arena: 13.1 GB requested (16.4 GB in chunks: bumpalo doubles chunk sizes, so the last
8 GB chunk is partly untouched and not resident). Rust heap outside the arena: ~2.5 GB live (link-store hash maps,
`SymbolTable` index maps, relation caches).

| MB | count | B/each | type |
| --- | --- | --- | --- |
| 3516.9 | 31,444,026 | 117 | str (3,242.6 MB of it at `parser_1.rs:380`, 48,706 calls) |
| 2179.8 | 25,973,355 | 88 | Symbol (Go: 96) |
| 1293.0 | 24,210,251 | 56 | ValueSymbolLinks |
| 1052.2 | 22,985,698 | 48 | Node |
| 826.5 | 21,666,011 | 40 | TypeMapper |
| 441.3 | 9,639,963 | 48 | Type |
| 337.7 | 3,643,896 | 97 | [P<Symbol>] |
| 298.3 | 3,007,846 | 104 | ObjectType |
| 257.3 | 2,108,158 | 128 | TypeReference |
| 250.6 | 4,691,624 | 56 | SymbolTable |
| 247.0 | 14,136,278 | 18 | [P<Type>] |
| 179.5 | 7,844,376 | 24 | Identifier |
| 174.7 | 1,430,757 | 128 | InferenceContext |
| 171.3 | 1,020,843 | 176 | UnionType |
| 168.4 | 1,576,178 | 112 | Signature |
| 162.6 | 1,775,832 | 96 | InferenceInfo |
| 161.4 | 8,894,854 | 19 | [P<Node>] |
| 155.9 | 1,135,037 | 144 | IntersectionType |
| 106.4 | 104,512 | 1067 | [&str] |
| 90.5 | 3,953,989 | 24 | NodeList |
| 56.2 | 1,841,212 | 32 | FlowNode |
| 42.4 | 2,779,226 | 16 | MappedSymbolLinks |
| 33.7 | 1,472,738 | 24 | SignatureLinks |
| 28.5 | 3,739,315 | 8 | SymbolNodeLinks |
| 17.2 | 1,288,083 | 14 | relater `type_comparer` closures |

Top live heap stacks (sampled, outside the arena): `SymbolArenaLinkStore::get` hash map 544 MB, `Relation::set`
225 MB, `NodeLinkStore::get` 136 MB, `SymbolTable` (IndexMap) growth in `resolve_object_type_members` /
`get_union_or_intersection_property` / mapped-type members ~600 MB over many stacks, other `LinkStore`s ~120 MB.

## Fixes

| change | 1 checker peak | 1 checker wall | 4 checkers peak | 4 checkers wall |
| --- | --- | --- | --- | --- |
| before (5ca48f3) | 19.53 GB | 31.5-32.9 s | 28.31 GB | 17.5 s |
| lazy JSDoc parse shares the source text | 16.35 GB | 31.4-34.5 s | 25.14 GB | 17.0 s |
| + module references appended once, transient symbol names not copied | 15.98 GB | 31.2 s | 24.63 GB | 16.9 s |
| + 32-byte `TypeMapper`, relater comparers built once per relater | 15.78 GB | 35.7 s (load 6-7; user 27.6 s) | 24.30 GB | 14.4 s |
| + id-keyed link stores map u32 id -> slot in value chunks | 15.35 GB | 29.2-30.6 s (prev 30.8-35.0) | 23.35 GB | best 14.5 s, check 12.0 s (prev 14.4 s, 11.9 s) |

1. **Lazy JSDoc parsing copied the whole file text into the arena per node** (`parse_jsdoc_for_node` ->
   `Parser::initialize_state` -> `alloc_str(source_text)`): ~9.8k calls, 3.0 GB. Go assigns the string (shared
   backing array). Now `initialize_state_static` takes the `&'static` text of the `SourceFile`; the main parse
   still copies the freshly read file once. Pure memory change, nothing observable.
2. **`collect_module_references` copied the whole imports / ambient-module-names / module-augmentations slice
   into the arena on every append** (Go appends with amortized growth; the old arrays are garbage): 166 MB. The
   appends now go to local vectors stored once at the end of `collect_external_module_references`; nothing reads
   these fields in between.
3. **`Checker::new_symbol` copied the name** (`alloc_str(name)`) for every transient symbol, 22M x 7 B on one
   checker, 36M on four (269 MB). Go stores the caller's string. `new_symbol`/`new_symbol_ex`/`new_parameter`/
   `new_property` now take `&'static str`; the 14 callers that pass a freshly built `String` copy it there.
4. **`TypeMapper` was 40 bytes** (21.7M on one checker, 39.9M on four) because `Array` held two slice headers
   and `Deferred` a slice plus a `Vec`. `Array` now holds a `tsrs_core::SlicePair` (two pointers, two u32
   lengths: 24 bytes) and the rare `Deferred` mapper lives out of line, so the enum is 32 bytes (compile-time
   assert in mapper.rs): -173 MB / -320 MB. Same lengths and elements, so mapping and `compare_type_mappers` are
   unchanged.
5. **relater `type_comparer` leaked one closure per call** (`signatures_related_to`, template-literal matching,
   infer-type-parameter contexts; 2.2M closures on four checkers, 30 MB, `notes/relater-2.md`). The closures only
   capture the relater handle (and the intersection state), so each relater now builds them once
   (`Relater::worker_comparer`, `signature_comparers`). Relaters are pooled and their handles never change, so a
   cached comparer calls exactly what a fresh one would.
6. **`valueSymbolLinks` / `symbolNodeLinks` were `FxHashMap<P<K>, P<V>>` plus one arena allocation per value**
   (24.2M / 3.7M entries on one checker; the value-links table alone was a 570 MB hash table with a 285 MB old
   table alive during its last resize). Go keys these two stores by symbol/node id (`PagedLinkStore`). Go's paging
   by id does not fit here (ids are process-wide atomics shared by the checkers, so every checker would touch
   nearly every page), so `IdLinkStore` maps `u32 id -> u32 slot` (9 bytes per table slot) and keeps the values
   in 4096-entry chunks allocated in the arena (stable addresses, `P<V>` handed out as before; ids >= 2^32 go to a
   second map). Ids are still assigned by `get_node_id`/`get_symbol_id` on every access, in the same order.
   -460 MB single, -940 MB on four checkers.

## Project, single-threaded, after (3fe4280)

Peak footprint 15.35 GB (Go 16.7 GB); 4 checkers 23.35 GB (Go 24.4 GB). Arena requested 9.57 GB (was 13.1 GB),
heap outside the arena ~4.27 GB live (was ~4.7 GB).

| MB | count | B/each | type |
| --- | --- | --- | --- |
| 2179.8 | 25,973,355 | 88 | Symbol |
| 1293.0 | 5,911 chunks | 229,376 | [ValueSymbolLinks] (24.2M values, 56 B) |
| 1052.2 | 22,985,698 | 48 | Node |
| 661.2 | 21,666,011 | 32 | TypeMapper |
| 441.3 | 9,639,963 | 48 | Type |
| 348.4 | 13,050,244 | 27 | str (213 MB of it source text, read once) |
| 337.7 | 3,643,896 | 97 | [P<Symbol>] |
| 298.3 | 3,007,846 | 104 | ObjectType |

Top live heap stacks now: `IdLinkStore` table for value links 288 MB, `Relation::set` 200 MB, `SymbolTable`
index maps in `resolve_object_type_members` & co. ~400 MB spread over many stacks, `symbolNodeLinks` table 72 MB,
other pointer-keyed `LinkStore`s ~100 MB.

## Not done (each < 2% or not behavior-safe)

- `SymbolTable` is an `IndexMap` (insertion-ordered: ~45-60 B per entry incl. stored hash and Vec slack vs ~30 B
  for Go's map). Replacing it changes iteration order, which ported code depends on for determinism.
- `Symbol` (88 B) is already smaller than Go's (96 B). `ValueSymbolLinks` values (56 B) are the same fields as Go.
- Pointer-keyed `LinkStore`s (`mappedSymbolLinks`, `signatureLinks`, `typeNodeLinks`, ...) could move to slot
  chunks too, but they must stay keyed by pointer (Go does not assign ids there; ids are observable): ~100 MB.
- `get_union_or_intersection_property` copies `name` for the property cache key (and again for the augmented
  cache): ~65 MB; the symbol name is not provably the same string, so left as is.
- ~1.1 GB of the footprint is neither arena nor live heap: malloc retention after ~23 GB of transient heap churn
  (Vec/HashMap temporaries). Reducing churn is a wall-time topic as much as a memory one.
