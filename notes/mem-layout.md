# mem-layout: data-layout and allocation changes for peak memory

Follow-up to `notes/fix-perf-memory.md`, after mimalloc (c1c1488) and lazy members on by default. Only layout /
allocation changes: no checker semantics, evaluation order, or type/symbol creation order change (the
`--extendedDiagnostics` counters stayed identical in both modes after every step).

Status (2026-10-10): the oldest layout log. Nearly every size it reports has changed since (notes/mem-round2.md,
mem-round3.md, mem-layout3.md, mem-small.md, mem-pointer-compression.md): the node header is 24 bytes, not 32, and
`Symbol` 32 with compressed pointers, not 72. The id-keyed link stores no longer use 1,024-id pages but 128-id
groups (`IdLinkStore`) and 32-id inline groups (`InlineIdStore`, notes/mem-dense-link-tables.md). What still holds
is the per-step record and the "Not done" reasons. The step-by-step tables and the before/after profiles were removed (git history has them).

Measurement: `/usr/bin/time -l tsrs -p tsconfig.json [--singleThreaded] --extendedDiagnostics` on the pristine
private monorepo checkout, one heavy process at a time, 18-core machine shared with other agents. "GB" is GiB of
`peak memory footprint` (11,586,069,456 B = 10.79 GB). Wall times on that machine varied by +-20% between rounds;
instructions retired were the stable CPU-work measure.

Profile build: `CARGO_TARGET_DIR=$PWD/target/prof cargo build --release -p tsrs_cli --features alloc-profile`
(the binaries' `alloc-profile` feature forwards to `tsrs_core/alloc-profile` and swaps the mimalloc global
allocator for the counting one, which allocates through mimalloc too). `--features tsrs_core/alloc-profile` alone no
longer links (two global allocators).

## Steps (single-threaded / 4-checker peak, GB, default mode)

| step | change | single | 4 checkers |
| --- | --- | --- | --- |
| 2 | one allocation per AST node (`NodeAlloc<T>`, header 48 -> 32 bytes) | 10.79 -> 10.46 | 16.09 -> 15.81 |
| 3 | `ValueSymbolLinks` 56 -> 32 bytes, four rare fields in a tail | 10.46 -> 10.22 | 15.76 -> 15.43 |
| 4 | `SymbolTable` as an insertion-ordered `Vec`, linear up to 8 entries, then a `HashTable<u32>` index (was `IndexMap`) | 9.75 -> 9.30 | 14.69 -> 13.96 |
| 5 | `Symbol` 88 -> 72 bytes (members / exports / export symbol in a tail) | 9.30 -> 9.12 | 13.98 -> 13.69 |
| 5 | `TypeMapper` 32 -> 24 bytes | 9.12 -> 8.93 | 13.70 -> 13.38 |
| 5 | one allocation per type (`TypeAlloc<T>`, header 48 -> 32) | 8.96 -> 8.81 | 13.39 -> 13.14 |
| 5 | packed 12-byte slice cells in `StructuredType` and `Signature` | 8.78 -> 8.68 | 11.76 -> 11.58 |
| 5 | relation cache slots 24 -> 20 bytes | 8.68 -> 8.64 | 11.61 -> 11.50 |
| 5 | `SymbolTable` entries grow by half past 8 | 8.64 -> 8.59-8.62 | 11.54 -> 11.46 |
| 5 | union/intersection property cache keyed by the property's own name | 8.59 -> 8.55 | 11.48 -> 11.41 |
| 5 | id-keyed link stores paged by id (below) | 8.55 -> 8.39 | 11.42 -> 11.33 |

Between steps 3 and 4 lazy tuple tables landed (bd93422), and before the slice-cell step locality checker assignment
(c9c52b2); each table row starts from the binary of its step. Instructions did not change by more than the noise in
any step (the `SymbolTable` growth step +0.1%).

Occupancy counts that drove the tails: of 13.57M `ValueSymbolLinks` records, `write_type` was set on 1.6%,
`name_type` 11%, `containing_type` 17%, `function_or_constructor_checked` 0.25%; of 3.78M binder symbols 617K had
`members`, 56K `exports`; of 11.55M transient symbols 301 had `members`. 3.89M `SymbolTable`s held 25.1M entries,
1.24M of them with one entry; the instantiation caches were few, large maps (863 maps held 5.78M of 6.0M entries).

### Id-keyed link stores paged by id

`IdLinkStore` (value-symbol links, symbol-node links) mapped `u32 id -> u32 slot` in one hash table per store
(9 bytes per table slot, a 150 MB table for the 11.5M value links single, and a cache miss per lookup). It became,
like Go's `PagedLinkStore`, a page of 1,024 consecutive ids (4 bytes per id) reached through a small map keyed by page
number; ids >= 2^32 kept a side map. Values and their first-access order were unchanged; check time single 18.1-18.5
-> 17.0-17.1 s, cycles 94.5-95.2 -> 88.9-89.3 G. The pages were later replaced by 128-id groups and, for symbol-node
links, 32-id inline groups (status line above); ids still have to stay dense for those stores to pay.

## Result

Interleaved, 3 rounds each (base = c1c1488). Reference mode isolates this pass (no lazy tables,
`--checkerAssignment go`); default mode also includes lazy tuple tables (bd93422) and locality assignment (c9c52b2).

| run | base peak GB | final peak GB | base check s | final check s | instructions |
| --- | --- | --- | --- | --- | --- |
| reference, single | 13.94 | 11.08 (-20.5%) | 20.9-21.4 | 18.1-18.2 | 338 -> 334 G |
| reference, 4 checkers | 21.21 | 17.06 (-19.6%) | 10.06-10.11 | 8.64-8.78 | 507 -> 501 G |
| default, single | 10.80 | 8.39 (-22%) | 18.9-20.3 | 16.7-18.0 | |
| default, 4 checkers | 16.09 | 11.33 (-30%) | 9.10-9.26 | 6.67-6.69 | |

## Not done (reasons as of this pass)

- `TypeMapper` count (21.6M single) and `[P<Type>]` lists: creation is semantics (mapper identity is used in
  caches); interning one-element lists would change slice identity.
- `InferenceContext` / `InferenceInfo` (~340 MB single): garbage after inference in Go, never freed in the arena
  then (notes/mem-recycle.md later recycled them).
- Instantiation maps: few large maps, so a shared map would hold the same entries.
- Pointer-keyed `LinkStore`s (~100 MB single): keyed by pointer by design (Go assigns no ids there).
