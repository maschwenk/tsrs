# perf-memory: peak memory on the private monorepo

First memory round (main 5ca48f3 -> 3fe4280). Acceptance per change: `--extendedDiagnostics` counters identical,
conformance pass lists identical (errors, `.types`, `.symbols`), no wall-time regression. Later rounds changed most of
the data structures named here (see notes/mem-layout.md and the `mem-*` notes); this note keeps the result, the
fixes that still stand, and the profiler.

## Allocation profile (opt-in)

Build: `CARGO_TARGET_DIR=$PWD/target/prof cargo build --release -p tsrs_cli --features alloc-profile` (replaces the
mimalloc global allocator with a counting one). Compiled out otherwise (`#[cfg_attr(feature, track_caller)]`, the
counters are `#[cfg]`). docs/DEBUGGING.md has the current heap-sampler options.

- Every arena allocation (`P::new`, `alloc`, `alloc_slice`, `alloc_vec`, `alloc_str`) is recorded per (call site,
  element type); the per-type table doubles as the `size_of` x count table (`B/each` = `size_of::<T>()` for
  `P::new`). Call sites are one level deep: allocations through a wrapper (`Symbol::new`, `new_node`, link stores)
  show the wrapper's line; the type column tells them apart.
- The counting global allocator reports the Rust heap (which includes the arena's chunks).
- `TSRS_HEAP_PROFILE=1`: sampling heap profiler (one raw stack per ~256 KB allocated, live bytes per stack, resolved
  at exit with `atos` on macOS or `addr2line` on Linux; arena chunk allocations are reported as one `<arena chunks>`
  row). Slow (~45 s check instead of 27 s at the time) but fine for a profile run.
- `TSRS_ALLOC_PROFILE_TOP` / `TSRS_HEAP_PROFILE_TOP` set N for the top-N tables. Output goes to stderr at exit.

## Result

Single-threaded peak footprint 19.53 GB -> 15.35 GB (Go 16.7 GB); 4 checkers 28.31 GB -> 23.35 GB (Go 24.4 GB). Arena
requested 13.1 GB -> 9.57 GB. Before, the largest arena item was `str`: 3,516.9 MB, 3,242.6 MB of it at one parser
call site (48,706 calls).

| change | 1 checker peak | 1 checker wall | 4 checkers peak | 4 checkers wall |
| --- | --- | --- | --- | --- |
| before (5ca48f3) | 19.53 GB | 31.5-32.9 s | 28.31 GB | 17.5 s |
| lazy JSDoc parse shares the source text | 16.35 GB | 31.4-34.5 s | 25.14 GB | 17.0 s |
| + module references appended once, transient symbol names not copied | 15.98 GB | 31.2 s | 24.63 GB | 16.9 s |
| + 32-byte `TypeMapper`, relater comparers built once per relater | 15.78 GB | 35.7 s (load 6-7; user 27.6 s) | 24.30 GB | 14.4 s |
| + id-keyed link stores map u32 id -> slot in value chunks | 15.35 GB | 29.2-30.6 s (prev 30.8-35.0) | 23.35 GB | best 14.5 s, check 12.0 s (prev 14.4 s, 11.9 s) |

The fixes, all pure memory changes with nothing observable:

1. Lazy JSDoc parsing copied the whole file text into the arena per node (`parse_jsdoc_for_node` ->
   `Parser::initialize_state` -> `alloc_str(source_text)`): ~9.8k calls, 3.0 GB. Go assigns the string (shared
   backing array). Now `initialize_state_static` takes the `&'static` text of the `SourceFile`.
2. `collect_module_references` copied the whole imports / ambient-module-names / module-augmentations slice into the
   arena on every append (166 MB). The appends go to local vectors stored once at the end of
   `collect_external_module_references`; nothing reads these fields in between.
3. `Checker::new_symbol` copied the name for every transient symbol (269 MB on four checkers). Go stores the
   caller's string; `new_symbol` and its siblings take `&'static str`.
4. `TypeMapper` 40 -> 32 bytes (-173 MB / -320 MB). It has shrunk further since (16 bytes, assert in mapper.rs).
5. The relater's `type_comparer` leaked one closure per call (2.2M closures on four checkers, 30 MB).
   Each relater now builds them once (`Relater::worker_comparer`, `signature_comparers`);
   relaters are pooled and their handles never change, so a cached comparer calls exactly what a fresh one would.
6. `valueSymbolLinks` / `symbolNodeLinks` became id-keyed (`IdLinkStore`): -460 MB single, -940 MB on four checkers.
   Go's paging by id did not fit, because ids are process-wide atomics shared by the checkers. The store has been
   redesigned since (128-id groups, links.rs; notes/mem-64.md); ids are still assigned on every access, in Go's order.

## The private monorepo, single-threaded, before (main at 5ca48f3)

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

## The private monorepo, single-threaded, after (3fe4280)

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

## Not done, still applies

- Pointer-keyed `LinkStore`s (`mappedSymbolLinks`, `signatureLinks`, `typeNodeLinks`, ...) stay keyed by pointer: Go
  assigns no ids there, and ids are observable (~100 MB at the time). Their values have since moved to arena chunks
  (links.rs `LinkStore`).
- ~1.1 GB of the footprint was neither arena nor live heap: malloc retention after ~23 GB of transient heap churn
  (Vec/HashMap temporaries). Reducing churn is a wall-time topic as much as a memory one.

Status (2026-10-10): three points of the original "Not done" list are obsolete: `SymbolTable` is no longer an
`IndexMap` but a `SymbolMap` with one-word entries that keeps insertion order (crates/tsrs_ast/src/symbol.rs);
`Symbol` is 32-40 B now, not 88 B (symbol.rs:51); `get_union_or_intersection_property` (~65 MB of copied cache-key
names at the time) now reuses the property's name when it equals the key (checker_11.rs). The arena no longer uses
bumpalo (crates/tsrs_core/src/arena.rs).
