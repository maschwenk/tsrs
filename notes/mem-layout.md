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

## Step 2: one allocation per AST node

`Node` was a 48-byte header holding `NodeData`, an enum of `&'static` pointers to a separately allocated data
struct (two arena allocations per node, 16 bytes of the header for the enum). Now the generator
(`tools/gen-ast/gen-ast.ts`) allocates `NodeAlloc<T> { node: Node, data: T }` (`repr(C)`) in one `P::new`; the
header is 32 bytes (`kind`, a `u8` `data_tag` in the former padding, `flags`, `loc`, `parent`, `id`;
compile-time assert), and the data struct sits at `offset_of!(NodeAlloc<T>, data)` = 32. Field-less data
structs (`Token`, `KeywordTypeNode`, ...) allocate only the header. `as_*()` and the generated dispatchers
(`for_each_child`, `visit_each_child`, `clone_node`, `name()`, `modifiers()`, `*_data()`) match on `data_tag`
and read the data in place (one pointer chase less); `node.data()` returns the old `NodeData` view for the four
hand-written matches (subtreefacts.rs, ast.rs, the AST oracle). Public accessors unchanged.

Checks: AST oracle 113/113 libs and 17,318/17,319 test units identical (the one is the known non-UTF-8 file),
`cargo test -p tsrs_ast -p tsrs_parser`, suite pass lists identical in both modes, counters identical (opt-out:
25,973,354 / 9,639,962 / 44,884,281 single, 39,704,001 / 16,200,921 / 89,981,648 on 4 checkers).

| run (3 interleaved rounds, median) | check s | instructions | peak GB |
| --- | --- | --- | --- |
| single, before | 22.03 | 320 G | 10.79 |
| single, after | 21.92 | 320 G | 10.46 (-0.34) |
| 4 checkers, before | 10.03 | 480 G | 16.09 |
| 4 checkers, after | 10.53 | 479 G | 15.81 (-0.29) |

Wall/check times on this shared machine vary by +-20% between rounds (load ~8 from other agents);
instructions retired (from `/usr/bin/time -l`) are the stable CPU-work measure and did not change.

## Step 3: link records

Occupancy (one-off count over all link chunks at exit, Project single, default mode): 13,568,293
`ValueSymbolLinks` records; `target` set in 68%, `resolved_type` 59%, `mapper` 56%, `containing_type` 17%,
`name_type` 11%, `write_type` 1.6%, `function_or_constructor_checked` 0.25%; 81% set none of the last four.
`write_type`, `name_type`, `containing_type` and `function_or_constructor_checked` moved into a tail
(`ValueSymbolLinksRare`, 32 bytes) allocated on the first non-default write; reads of an absent tail return the
zero value. Record 56 -> 32 bytes (compile-time assert). Callers use `x()` / `set_x(..)` for the four moved fields
(54 sites, mechanical). The other link records are too small to matter here: `SignatureLinks` 34 MB,
`TypeNodeLinks` 20 MB, `MappedSymbolLinks` 18 MB, `NodeLinks` 2 MB (single).

| run (2 interleaved rounds) | check s | instructions | peak GB |
| --- | --- | --- | --- |
| single, before | 22.43-24.88 | 318-320 G | 10.46 |
| single, after | 19.82-22.59 | 318-320 G | 10.22 (-0.23) |
| 4 checkers, before | 9.27-10.16 | 478-479 G | 15.76 |
| 4 checkers, after | 10.24-10.36 | 478 G | 15.43 (-0.33) |

Opt-out counters identical; suite pass lists identical in both modes.

(After this step `mem-lazy` landed lazy member tables for tuple references, bd93422: default-mode symbols drop to
13,297,831 single / 19,555,571 on 4 checkers and the peaks to 9.75 / 14.69 GB; the rows below start from there.)

## Step 4: map overhead

Counts at exit (Project single, default mode, one-off instrumentation):

| map | count | entries | size distribution |
| --- | --- | --- | --- |
| `SymbolTable` | 3,894,884 | 25,056,786 (capacity 34.6M) | 1.24M with 1 entry, 1.01M with 2, 0.63M with 3-4, 0.31M with 5-8; 382K with 17-64 hold 14.2M entries |
| `instantiations` (`GoMap<CacheHashKey, P<Type>>`, type aliases, interfaces, conditional roots, object types) | 27,868 | 6,016,332 | 863 maps hold 5.78M entries; 23K maps with <= 16 |
| `Relation.results` | 5 relations | (one large table per relation) | |
| other `GoMap`s (widening contexts, constituent maps, node-builder caches) | < 45K | < 30K | |

So the tiny-map problem is `SymbolTable`, not the instantiation caches (few, large maps: a shared map keyed by
(owner, key) would hold the same entries) or the relation caches.

**`SymbolTable`** was an `IndexMap<&str, P<Symbol>, Fx>`: 56-byte header, 32 bytes per entry slot (stored hash +
key + value) plus an 8-byte index per hash slot, even for one entry. Now (`tsrs_ast::symbol`): the entries in
insertion order in a `Vec<(&str, P<Symbol>)>` (24 bytes per slot), searched linearly up to 8 entries; past that a
boxed `hashbrown::HashTable<u32>` of positions (5 bytes per slot). Header 32 bytes. Same observable behavior as
`IndexMap` (insertion-order iteration, re-`set` keeps position and stored key, `delete` = `shift_remove`); unit
test `symbol_table_indexed_order_and_delete`.

| run (2 interleaved rounds) | check s | instructions | peak GB |
| --- | --- | --- | --- |
| single, before | 21.2-31.1 | 316-318 G | 9.75 |
| single, after | 19.4-29.8 | 315 G | 9.30 (-0.45) |
| 4 checkers, before | 9.34-9.60 | 474 G | 14.69 |
| 4 checkers, after | 9.47-9.67 | 471 G | 13.96 (-0.73) |
| opt-out single / 4 checkers, before | | | 13.19 / 20.22 |
| opt-out single / 4 checkers, after | | | 12.23 / 18.73 |

## Step 5: other layouts

### `Symbol` 88 -> 72 bytes

Occupancy at exit (Project single, default mode): 3.78M binder symbols, of which 617K have `members`, 56K
`exports`, 115K `export_symbol`; 11.55M transient symbols, of which 301 have `members` and 144 `exports`. The three
fields moved into a tail (`SymbolTables`) allocated on the first non-nil write; `members()` / `exports()` /
`export_symbol()` read through it (nil when absent) and `set_members()` / `set_exports()` / `set_export_symbol()`
write (23 call sites changed from `.x.get()` / `.x.set(..)`; `get_members` / `get_exports` keep Go's
create-on-demand behavior). Same `OwnedCell` ownership contract as before (only the owning thread writes, and the
tail is allocated by that writer).

| run (2-4 interleaved rounds) | check s | instructions | peak GB |
| --- | --- | --- | --- |
| single, before | 19.3-23.1 | 313-314 G | 9.27-9.30 |
| single, after | 22.2-23.8 | 313-314 G | 9.12 (-0.15) |
| 4 checkers, before | 9.93-10.63 | 471 G | 13.98 |
| 4 checkers, after | 9.88-10.29 | 471 G | 13.69 (-0.29) |
| opt-out single / 4 checkers, before | | | 12.23 / 18.73 |
| opt-out single / 4 checkers, after | | | 11.86 / 18.12 |
