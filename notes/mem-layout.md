# mem-layout: data-layout and allocation changes for peak memory

Follow-up to `notes/fix-perf-memory.md`, after mimalloc (c1c1488) and lazy members on by default. Only layout /
allocation changes: no checker semantics, evaluation order, or type/symbol creation order change (counters below
must stay identical in both modes).

Measurement: `/usr/bin/time -l tsrs -p tsconfig.json [--singleThreaded] --extendedDiagnostics` on the pristine
Project checkout, one heavy process at a time, 18-core machine shared with other agents. "GB" in the tables below
is GiB of `peak memory footprint` (same convention as the earlier notes: 11,586,069,456 B = 10.79 GB).

Profile build: `CARGO_TARGET_DIR=$PWD/target/prof cargo build --release -p tsrs_cli --features alloc-profile`
(the binaries' `alloc-profile` feature forwards to `tsrs_core/alloc-profile` and swaps the mimalloc global
allocator for the counting one, which now allocates through mimalloc too, so the heap rows reflect the production
allocator). `--features tsrs_core/alloc-profile` alone no longer links (two global allocators).

## Baseline (main at c1c1488, default mode)

| run | symbols | types | instantiations | check s | total s | peak GB |
| --- | --- | --- | --- | --- | --- | --- |
| single | 15,331,397 | 9,630,120 | 44,820,708 | 19.47 | 22.99 | 10.79 |
| 4 checkers | 22,813,093 | 16,186,849 | 89,882,712 | 9.46 | 11.16 | 16.18 |

## Profile (step 1), default mode

Single: arena 7,811 MB requested (8,223 MB in chunks), Rust heap outside the arena 3,003 MB live at exit, heap peak
11,243 MB. 4 checkers: arena 11,277 MB requested (16,822 MB in 27 arenas' chunks; untouched chunk tails are not
resident), heap outside the arena 4,711 MB live.

Arena by type (MB, count, bytes each):

| type | single MB | single count | 4-ch MB | 4-ch count | B/each |
| --- | --- | --- | --- | --- | --- |
| Symbol | 1286.7 | 15,331,398 | 1914.6 | 22,813,097 | 88 |
| Node | 1052.2 | 22,985,698 | 1059.3 | 23,139,986 | 48 |
| [ValueSymbolLinks] chunks | 724.7 | 3,313 | 1150.8 | 5,261 | 229,376 (4096 x 56) |
| TypeMapper | 660.3 | 21,635,081 | 1215.1 | 39,815,815 | 32 |
| Type | 440.8 | 9,630,121 | 741.0 | 16,186,853 | 48 |
| str | 318.4 | 8,134,562 | 371.0 | 13,791,579 | 41 / 28 (213.5 MB source text) |
| ObjectType | 298.2 | 3,006,176 | 450.0 | 4,537,406 | 104 |
| TypeReference | 257.2 | 2,106,919 | 457.0 | 3,743,571 | 128 |
| [P<Type>] | 227.2 | 13,235,777 | 420.4 | 24,368,238 | 17 |
| SymbolTable | 208.0 | 3,894,884 | 271.3 | 5,079,739 | 56 |
| Identifier | 179.5 | 7,844,376 | 180.4 | 7,880,391 | 24 |
| InferenceContext | 174.6 | 1,430,116 | 266.0 | 2,179,182 | 128 |
| UnionType | 171.3 | 1,020,584 | 290.1 | 1,728,264 | 176 |
| Signature | 168.1 | 1,574,006 | 279.6 | 2,617,554 | 112 |
| InferenceInfo | 162.5 | 1,775,160 | 252.0 | 2,752,092 | 96 |
| IntersectionType | 155.7 | 1,133,593 | 269.6 | 1,963,135 | 144 |
| [P<Symbol>] | 138.7 | 2,864,411 | 222.3 | 4,375,649 | 50 |
| [P<Node>] | 102.8 | 8,759,426 | 114.5 | 9,494,771 | 12 |
| NodeList | 90.5 | 3,953,989 | 91.1 | 3,981,027 | 24 |
| other AST payloads (PropertyAssignment, CallExpression, ...) | ~450 | | ~450 | | |

Rust heap outside the arena, live at exit, by allocating function (sampled, MB):

| function (top frame) | single | 4 checkers | what |
| --- | --- | --- | --- |
| `get_union_or_intersection_property` | 319 | 482 | `SymbolTable`/cache maps of union/intersection properties |
| `resolve_object_type_members` | 303 | 428 | resolved member `SymbolTable` index maps |
| `GoMap::set` | 227 | 364 | per-type `instantiations` maps (type references, conditional, object types) |
| `Relation::set` | 227 | 151 | relation caches |
| `IdLinkStore::get` | 216 | 360 | value/node link id -> slot tables |
| `Relater::reset_maybe_stack` | 25 | 313 | relater maybe-key sets (capacity retained by pooled relaters) |
| `Binder::declare_symbol_ex` | 194 | 186 | binder symbol tables |
| `check_expression_worker` | 190 | 171 | |
| `get_ready_lazy_member_table_worker` + `resolve_lazy_members` | 251 | 442 | lazy member tables |
| `LinkStore::get` (pointer-keyed) | 104 | 154 | |
| `SymbolTable::with_capacity` | 76 | 117 | |
