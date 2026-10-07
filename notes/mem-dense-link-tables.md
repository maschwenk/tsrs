# mem-dense-link-tables: dense id-indexed link tables (Bun's `ByNode` / `ById`), design and verified estimate

notes/bun-check-memory.md listed "dense link tables" as a layout-only technique worth 60-90 MB on vscode at one
checker, with no fidelity risk in principle, and never verified it. Bun keeps what it computes per node, symbol and
type in arrays indexed by dense ids whose zero pages are committed on first touch (Bun `src/sema/table.rs:1-11`,
`414-575`; `util/memory.rs:61-74` and `336-388`; used from `check/mod.rs:193-229`, e.g. `expr_types`,
`type_node_types`, `effects_signatures`, each a `ByNode` of 4-byte ids). This note measures what tsrs's link stores
hold, how dense their keys are in every id space they have or could get, and prototypes the layouts that pay.

Base: origin/main 1cbf079. Machine: Apple M5 Max, 18 cores, 16 KiB pages, macOS 26.6. Projects: vscode
(`-p src`) and t3code-server (`-p apps/server`) from the bench checkouts, `--noEmit --incremental false --pretty
false`. Counts come from a temporary density report printed next to `TSRS_HEAP_CENSUS=1` (end of checking; link
stores never shrink, so the end is their peak). Peaks are `/usr/bin/time -l` peak memory footprint. The
instrumentation and the prototypes were temporary and are not part of this change. MB = 10^6 bytes in the
arithmetic, MiB = 2^20 bytes for peaks; byte counts are summed over all checkers.

**Result.** The 60-90 MB does not hold for Bun's technique as such. Only one store has a dense key that pays:
`symbol_node_links` with its 4-byte value stored in the id-indexed table itself (26 MB at one checker, 36 MB at 32).
Two hash-layout changes in the same spirit (signature and type-node links keep a 4-byte index per bucket and find
the key in the value's padding; reference kinds live in the slot) bring the prototype to **-46 MiB at one checker
and -75 MiB at 32 checkers on vscode** (-2.3% / -2.7% of peak; census arithmetic: 47 / 62 MiB), -12 / -47 MiB on
t3code-server, instructions unchanged within the noise, diagnostics byte-identical on eight projects. Bun-style
per-file, per-kind node numbering would add at most 4-5 MB on top and needs an AST change.

## 1. Inventory

### 1a. The 27 link stores (crates/tsrs_checker/src/checker.rs:1033-1059)

Two layouts (crates/tsrs_checker/src/links.rs):

- `LinkStore<K, V>` (links.rs:10-110), 25 stores: a hashbrown table of 8-byte slots (the key's `PKey` handle and the
  value's index, links.rs:19-24) and the values in 1,024-value arena chunks in first-access order (links.rs:26,
  89-110). A value is a `PSlot<V>`, 8-aligned with compressed pointers (crates/tsrs_core/src/ptr.rs:553-556)
  because a `P` handle counts 8-byte units (crates/tsrs_core/src/reserve.rs:17-20): a 4-byte value costs 8 bytes,
  a 12-byte one 16.
- `IdLinkStore<V>` (links.rs:120-290) behind `NodeLinkStore` (node id, links.rs:301) and `SymbolArenaLinkStore`
  (symbol id, links.rs:346): `index[id / 128]` points at a 264-byte `IdGroup` of 16-bit slot offsets
  (links.rs:131-137), the values sit in 4,096-value chunks.

vscode. "id at insert": keys that already had a node / symbol id when their links were created. "index": slot table,
or group index plus groups. "values": arena chunks. Sites: calls of `get` / `try_get` / `has` /
`try_get_if_id_assigned` (grep), in parentheses those bound to a local.

| store | key | B/value | sites | entries at 1 / 4 / 16 / 32 checkers | id at insert, 1 / 32 | index MB, 1 / 32 | values MB, 1 / 32 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| value_symbol_links | symbol id | 16 | 91 (47) | 3,667,277 / 4,266,151 / 5,422,355 / 6,213,095 | 100% / 100% | 8.87 / 42.13 | 58.72 / 100.47 |
| symbol_node_links | node id | 8 (4 used) | 24 (11) | 4,420,883 / 4,549,884 / 4,833,574 / 5,033,511 | 100% / 100% | 9.49 / 30.32 | 35.39 / 40.73 |
| signature_links | P<Node> | 16 (12 used) | 9 (7) | 1,405,767 / 1,457,052 / 1,565,186 / 1,645,464 | 0% / 1% | 18.87 / 20.35 | 22.50 / 26.59 |
| type_node_links | P<Node> | 24 (20 used) | 22 (21) | 987,797 / 1,157,838 / 1,516,437 / 1,754,599 | 1% / 27% | 18.87 / 25.07 | 23.72 / 42.49 |
| symbol_reference_links | P<Symbol> | 8 (4 used) | 3 (2) | 809,859 / 857,586 / 955,239 / 1,022,182 | 61% / 53% | 9.44 / 17.25 | 6.48 / 8.31 |
| members_and_exports_links | P<Symbol> | 8 | 1 (1) | 230,545 / 260,132 / 323,736 / 365,018 | 80% / 61% | 4.72 / 4.94 | 1.85 / 3.04 |
| alias_symbol_links | P<Symbol> | 16 | 14 (9) | 185,404 / 213,452 / 265,248 / 296,229 | 0% / 8% | 2.36 / 4.46 | 2.98 / 5.00 |
| node_links | P<Node> | 8 | 22 (12) | 194,667 / 196,699 / 200,761 / 203,525 | 2% / 2% | 2.36 / 2.77 | 1.56 / 1.76 |
| mapped_symbol_links | P<Symbol> | 8 | 9 (5) | 160,894 / 191,654 / 250,949 / 293,237 | 100% / 100% | 2.36 / 4.31 | 1.29 / 2.48 |
| array_literal_links | P<Node> | 16 | 1 (1) | 89,198 / 89,586 / 90,730 / 92,094 | 0% / 0% | 1.18 / 1.36 | 1.44 / 1.70 |
| marked_assignment_symbol_links | P<Symbol> | 8 | 5 (3) | 61,056 / 61,236 / 61,657 / 62,076 | 88% / 88% | 1.18 / 1.00 | 0.49 / 0.60 |
| declared_type_links | P<Symbol> | 8 | 12 (11) | 48,385 / 81,307 / 160,966 / 229,785 | 5% / 33% | 0.59 / 3.58 | 0.39 / 1.97 |
| enum_member_links | P<Node> | 32 | 4 (0) | 9,210 / 18,796 / 40,616 / 61,955 | 0% / 0% | 0.15 / 0.95 | 0.29 / 2.49 |
| module_symbol_links | P<Symbol> | 16 | 5 (3) | 10,372 / 15,931 / 26,794 / 36,138 | 6% / 10% | 0.15 / 0.56 | 0.18 / 0.84 |
| assertion_links | P<Node> | 8 (4 used) | 2 (2) | 18,888 / 19,022 / 19,404 / 19,924 | 0% / 0% | 0.29 / 0.30 | 0.16 / 0.29 |
| type_alias_links | P<Symbol> | 32 | 13 (9) | 5,310 / 8,251 / 14,489 / 19,748 | 31% / 51% | 0.07 / 0.30 | 0.20 / 1.11 |
| deferred_symbol_links | P<Symbol> | 40 | 3 (3) | 5,381 / 7,645 / 12,171 / 15,980 | 100% / 100% | 0.07 / 0.25 | 0.25 / 1.35 |
| source_file_links | P<SourceFile> | 152 | 11 (10) | 9,415 / 9,766 / 10,540 / 11,101 | - | 0.15 / 0.16 | 1.56 / 5.14 |
| late_bound_links | P<Symbol> | 8 (4 used) | 2 (1) | 5,704 / 6,260 / 7,885 / 9,568 | 0% / 0% | 0.07 / 0.14 | 0.05 / 0.26 |
| spread_links | P<Symbol> | 8 | 5 (2) | 8,417 / 8,417 / 8,446 / 8,446 | 100% / 100% | 0.15 / 0.12 | 0.07 / 0.29 |
| computed_name_links | P<Node> | 24 | 1 (1) | 5,352 / 5,359 / 5,668 / 5,884 | 0% / 0% | 0.07 / 0.09 | 0.15 / 0.79 |
| variance_links | P<Symbol> | 16 | 1 (1) | 1,634 / 2,576 / 4,307 / 5,734 | 72% / 48% | 0.02 / 0.08 | 0.03 / 0.52 |
| reverse_mapped_symbol_links | P<Symbol> | 16 | 10 (5) | 1,182 / 1,204 / 1,204 / 1,528 | 100% / 100% | 0.02 / 0.02 | 0.03 / 0.43 |
| switch_statement_links | P<Node> | 40 | 3 (3) | 1,035 / 1,040 / 1,051 / 1,051 | 0% / 0% | 0.02 / 0.02 | 0.08 / 1.31 |
| export_type_links | P<Symbol> | 8 | 7 (4) | 571 / 614 / 681 / 720 | 0% / 0% | 0.01 / 0.01 | 0.01 / 0.26 |
| symbol_container_links | P<Symbol> | 96 | 3 (3) | 5 / 15 / 25 / 25 | 60% / 92% | 0.00 / 0.00 | 0.10 / 0.49 |
| jsx_element_links | P<Node> | 24 | 6 (5) | 0 (vscode has no JSX) | - | 0 | 0 |
| **total** | | | 289 (182) | | | **81.5 / 160.6** | **160.0 / 250.7** |

The link stores hold 241 MB at one checker and 411 MB at 32. Four fifths of the index bytes at one checker are in
five stores (signature, type-node, symbol-reference and the two id-keyed ones); the values are dominated by the two
id-keyed stores and the signature and type-node links. t3code-server at 32 checkers has the same shape with
`value_symbol_links` (8.88M entries, 45.2 + 143.2 MB) and `mapped_symbol_links` (2.13M entries, 28.9 + 17.2 MB) on
top.

### 1b. How callers use the values

- **Values never move, and callers rely on it.** Both stores hand out `P<V>` into chunks that are never
  reallocated. `get_resolved_symbol` holds its symbol-node links across `resolve_name`, which creates more
  (checker_07.rs:1356-1362); `run_without_resolved_signature_caching` keeps `P<SignatureLinks>` and
  `P<ValueSymbolLinks>` in a vector across a nested check (services.rs:403-421). 182 of 289 sites bind the links to
  a local. A layout in which a value can move (a value stored in a hashbrown slot moves on every rehash) is safe only
  for a store none of whose sites keeps the value: of the large stores, only `symbol_reference_links`
  (checker_01.rs:566-567 and checker_13.rs:2435-2436 set the flags on the spot, checker_01.rs:574 reads them).
- **No iteration.** Neither store has an iteration API (links.rs), so slot order and first-access order are internal
  and no output depends on table order. The handle-keyed caches in 1c are iterated only by the census
  (checker_09.rs:2629, sums).
- **Ids are observable.** Node ids appear "in cache keys and internal names" (links.rs:298-300); symbol ids in the
  internal names of unique-symbol-keyed properties, whose length the node builder counts toward truncation
  (links.rs:341-345). A store may use an id where Go's code already assigns one; it must not assign one Go does not.

### 1c. Other handle-keyed tables in the checker

| table | key | entries, vscode 1 / 32 | MB, vscode 1 / 32 | t3code-server 32 |
| --- | --- | --- | --- | --- |
| lazy_member_tables | P<Type> | 167,308 / 268,043 | 2.34 / 4.37 | 1.70M entries, 27.4 MB |
| object_type_instantiations (outer) | P<Type> | 11,634 / 51,960 | 0.67 / 3.61 | 3.45 MB |
| structured_type_base_constraints | P<Type> | 30,655 / 65,316 | 0.59 / 1.07 | 3.13 MB |
| flow_node_reachable | P<FlowNode> | 344,671 / 350,777 | 4.5 / 3.7 MiB (census rows, rounded per checker) | |
| flow_type_cache, context_free_types | P<Node> | 41,766, 20,232 at 1 | 0.6, 0.3 at 1 | |

Everything else is below 0.1 MB at one checker. `object_type_instantiations`' inner maps (14 MB at one checker) are
keyed by argument-list hashes, not ids. `FlowNode` has no id (crates/tsrs_ast/src/flow.rs:39-46). The node
builder's and emit resolver's stores (`tsrs_core::LinkStore`, crates/tsrs_core/src/linkstore.rs:8-50: a
`FxHashMap<P<K>, P<V>>` with one arena value per entry) live per node builder or per emit resolver; they are not
in the census and were not measured.

## 2. Id density

### 2a. The id-keyed stores

Share of the ids in a touched group (or page) that have links, vscode:

| store | grouping | 1 | 4 | 16 | 32 checkers |
| --- | --- | --- | --- | --- | --- |
| symbol_node_links | 32 ids | 0.98 | 0.87 | 0.73 | 0.67 |
| | 128 ids (today) | 0.98 | 0.72 | 0.49 | 0.41 |
| | 1,024 ids (a 4 KiB page of 4-byte cells) | 0.98 | 0.40 | 0.19 | 0.14 |
| | 4,096 ids (a 16 KiB page) | 0.97 | 0.27 | 0.10 | 0.07 |
| value_symbol_links | 32 ids | 0.87 | 0.76 | 0.64 | 0.59 |
| | 128 ids (today) | 0.87 | 0.62 | 0.42 | 0.36 |
| | 1,024 ids | 0.87 | 0.35 | 0.17 | 0.13 |
| | 4,096 ids | 0.87 | 0.25 | 0.09 | 0.06 |

Ids come from per-thread blocks of 1,024 (crates/tsrs_ast/src/utilities_1.rs:14, 33-50): a checker's own blocks
are full, the other checkers' blocks thin. Small groups follow that; OS pages (Bun's unit of lazy commit) do not.
Node ids go almost only to nodes that get symbol-node links: 4,420,883 of the ids up to 4,538,174 at one checker.

### 2b. The hashed stores: is there a dense key?

- **Node ids: no.** Keys of the node-keyed stores almost never have a node id when their links are created: 0.3% of
  signature-link keys, 0.9% of type-node-link keys, 1.6% of node-link keys at one checker (type-node links 27% at 32
  checkers, where other checkers assigned ids first). Assigning one would change later ids (1b).
- **Symbol ids: too thin.** Keys of mapped, deferred, spread and reverse-mapped links (transient symbols) all have
  ids; members-and-exports 80% -> 61%, symbol-reference 61% -> 53%, alias 0% -> 8%, declared-type 5% -> 33% (1 ->
  32 checkers). Where they have ids, each store fills 1-21% of the 128-id groups it touches (symbol-reference 0.19 /
  0.08 at 1 / 32, mapped 0.18 / 0.21, all others below 0.11): the symbol id space is shared by all 4.2M symbols a
  checker numbers. A 128-id group costs 264 bytes and a hashed entry ~14 (8 + 1 bytes per bucket at the measured
  loads), so groups pay above ~0.15 and only if every key has an id. Modeled per store with its own groups and index
  (keys without an id at insert left in a hash), the symbol stores above 20,000 entries lose 2.8-6.0 MB each at one
  checker and 6.5-18.6 MB at 32; only `mapped_symbol_links` gains (0.2-0.3 MB on vscode at 1-4 checkers, 2.1 / 8.9
  MB on t3code-server at 1 / 32), and more by sharing the value-symbol groups (section 3, E).
- **Handles: no.** Signature-link keys lie in 547,463 distinct 1-KiB ranges of the arena at one checker (2.6 keys
  per KiB); a table indexed by handle (8-byte units, reserve.rs:17-20) would spend ~200 bytes per key.
- **Bun's numbering (per file, per node kind): dense for call-like keys, not available.** If every node had an
  ordinal among the nodes of its kind in its file, the cells a checker would need for the (file, kind) pairs its keys
  touch: signature links 1.64M cells for 1.41M keys (0.86) at one checker, 3.20M for 1.65M (0.51) at 32; node links
  0.39 / 0.37; array-literal links 0.70 / 0.67; type-node links 0.10 / 0.11 (this store also caches expression
  types: 29,576 of its keys are identifiers, out of 4.76M identifiers in the files they touch). Per kind at one
  checker, signature links cover 997,612 of 997,842 call expressions, all 143,911 arrow functions and all 87,187 new
  expressions; at 32 checkers call expressions fill 0.60 of their cells. tsrs nodes carry no such ordinal: `Node` is
  24 bytes with no spare field (crates/tsrs_ast/src/ast.rs:390-397), the vscode program has 17.1M nodes, so a
  4-byte ordinal costs 8 bytes per node after alignment (137 MB, shared) unless a kind has padding (a
  `CallExpression` allocation is 24 + 12 -> 40 bytes, 4 spare; other kinds not checked), and a node does not know
  its file (Bun's key is `(FileId, index)`, table.rs:266-333).
- **Type ids: dense per checker, but the caches are sparse in them.** `lazy_member_tables` keys fill 0.08 / 0.07 of
  the 128-type-id groups they touch (vscode 1 / 32), `structured_type_base_constraints` 0.03,
  `object_type_instantiations` 0.02-0.04; t3code-server at 32 checkers 0.24 / 0.05 / 0.03. 4-byte cells in 128-id
  groups would cost 6.3 / 10.2 MB more than the hash for lazy member tables on vscode and 0.6 MB more on
  t3code-server.

## 3. Design

### What the 128-id groups already captured

PR 120's `IdGroup` made the id-to-slot index 2 bytes per id slot in small groups. Of the layouts below it is the best
for `value_symbol_links` at one checker (tied with Bun's) and within 0.6 MB of the best at 4; at 16-32 checkers
32-id groups behind a two-level index (G) would save 6-12 MB more. Bun-style tables over 4 KiB pages cost 110-632 MB
more than today for `value_symbol_links` at 4-32 checkers, and for `symbol_node_links` match A at one checker but
cost 23-106 MB more than A at 4-32, because the pages thin with the id blocks. What PR 120 left: a 4-byte value
still costs a 2-byte offset, 8 bytes in a chunk (the value padded to the handle unit) and its share of the group
header, 10.2 bytes per entry for `symbol_node_links` where Bun stores 4.

Modeled bytes (MB, vscode; t3code-server in the last two columns), from the measured groups, pages and entries:

| layout | 1 | 4 | 16 | 32 | t3 1 | t3 32 |
| --- | --- | --- | --- | --- | --- | --- |
| symbol_node_links today (128-id offset groups + chunks) | 44.9 | 50.2 | 62.0 | 71.1 | 9.0 | 30.8 |
| Bun `ById`, value inline in 4 KiB pages (1,024 ids) | 18.1 | 45.8 | 100.0 | 140.7 | 3.6 | 43.1 |
| Bun `ByNodeIndirect`-like, 4-byte handle in 4 KiB pages + chunks | 53.5 | 82.2 | 139.0 | 181.5 | 10.7 | 63.3 |
| **A: value inline in 32-id groups, two-level index** | **18.7** | **22.5** | **30.0** | **35.0** | **3.7** | **14.6** |
| value_symbol_links today | 67.6 | 83.2 | 117.1 | 142.6 | 31.9 | 188.4 |
| Bun `ById`, 16-byte value inline in 1,024-id pages | 67.6 | 193.3 | 506.8 | 774.4 | 30.7 | 696.6 |
| 16-byte value inline in 32-id groups, two-level index | 68.1 | 91.3 | 140.7 | 174.9 | 30.9 | 207.8 |
| G: 32-id offset groups (72 bytes) + chunks, two-level index | 68.8 | 82.6 | 110.9 | 130.9 | 32.5 | 177.8 |

### A. `symbol_node_links`: the resolved symbol inline in 32-id groups (the one dense table that pays)

`SymbolNodeLinks` is one `Cell<Option<P<Symbol>>>` (types.rs:690), 4 bytes. Store it in the table:

- Level 1: a vector indexed by `id >> 10` (one entry per id block, 4 bytes; 18 KB per checker at 32 checkers on
  vscode). Level 2: a 128-byte arena table of 32 group pointers per touched block. Groups: `[SymbolNodeLinks; 32]`,
  128 bytes, in the checker's arena, zero (None) until set. Ids at or above 2^32 keep a hash map, as today.
- A lookup is the node's id, the level-1 entry, the level-2 entry and the cell: three dependent loads after the id,
  one fewer than today (index entry, group, chunk pointer, value), and no 16-bit offset arithmetic or dense-form
  branch.
- Handles and the reserve: the cell is 4-aligned, not 8, so `get` returns `&'static SymbolNodeLinks` instead of a
  `P` handle (the groups never move or get freed, so the reference is as long-lived as the handle was). No code
  names `P<SymbolNodeLinks>`; the prototype compiled with all 24 call sites unchanged. Groups and level-2 tables
  are ordinary arena objects inside the 32 GiB reservation; nothing needs lazily committed pages, which the
  reservation commits per chunk anyway (reserve.rs:101-117) and huge-page-advises on Linux.
- Semantics: `get` and `try_get` still call `get_node_id` exactly where they do today, so ids are assigned in the
  same order. `try_get` answers `Some` for any id of an allocated group even if that id never had `get`; the two
  `try_get` callers read `resolved_symbol` only (nodebuilderimpl_1.rs:493-494, flowmemo.rs:544), which is None in
  both cases. No caller uses `has`.
- A flat 32-id index (one 4-byte entry per 32 ids) is not enough: it is 21 MB at 32 checkers (summed) and it is
  reallocated as it grows by a quarter; that prototype measured +7 / +8 MiB peak at 32 checkers (two rounds) against
  -16 / -24 MiB at one. The two-level index fixed it.

### B. `signature_links`, `type_node_links`: 4-byte index per bucket, the key in the value's padding

No dense key exists (2b). What a dense table buys, a 4-byte cell per entry pointing at a stable value (Bun's
`ByNodeIndirect`, table.rs:503-517), a hash table can have too: store only the value's index in the bucket and keep
the key in the value. `SignatureLinks` is 12 bytes in a 16-byte slot (types.rs:740), `TypeNodeLinks` 20 bytes in 24
(types.rs:695), so the key fits the padding and the values do not grow. The bucket drops from 8 + 1 to 4 + 1 bytes:
4/9 of the table, 16.8 MB at one checker on vscode (both 18.0 -> 10.0 MiB in the prototype's census) and 20.2 MB at
32. Values stay in their chunks, so the pointers of 1b stay valid. Probing compares the key in the value: on a hit
the value is the one the caller dereferences next, so the dependent loads are the same; a 7-bit tag false positive
costs one extra load; a rehash reads every value's key once (random reads into the chunks, once per doubling).
Plain-pointer builds (`PKey` is 8 bytes) have no room: keep `LinkStore` there.

### C. `symbol_reference_links`: the flags in the slot

`SymbolReferenceLinks` is one 4-byte `SymbolFlags` (types.rs:256) in an 8-byte arena slot, and its three sites never
keep it (1b). A hash table of (key, flags) pairs has the same 8-byte bucket as today and no value chunk: -6.5 MB at
one checker, -8.3 MB at 32 (vscode). Two dependent loads fewer per lookup. API: `add(symbol, meaning)` and
`kinds(symbol)` replace `get(..).reference_kinds`. `late_bound_links` and `assertion_links` have the same shape but
hold 0.1-0.3 MB.

### D. Smaller value chunks for small stores (modeled)

The unused part of each store's last 1,024-value chunk is 0.5 MB at one checker and 12.8 MB at 32 on vscode (12.8 MB
on t3code-server), mostly stores with a few hundred entries per checker and large values: `source_file_links` 3.5
MB (152-byte values), `switch_statement_links` 1.3 MB, `value_symbol_links` 1.1 MB, `deferred_symbol_links` 0.7 MB
at 32. Chunks that start at 64 values and double to 1,024 waste 6.2 MB at 32 checkers (-6.7 MB); a per-store
constant chunk size (64 values for the stores with a few thousand entries per checker) keeps `at` free of extra
instructions.

### E. `mapped_symbol_links` through the value-symbol groups (modeled)

Mapped symbols always have ids at insert (2b). Giving `IdGroup` a 4-byte pointer to a lazily allocated 256-byte
array of mapped-link offsets replaces the mapped slot table: -0.4 / -0.6 / -0.7 / -1.0 MB at 1 / 4 / 16 / 32
checkers on vscode, but -2.2 / -15.4 MB at 1 / 32 on t3code-server (2.13M mapped links at 32). Couples two stores.

### Not proposed

- **Bun-style node numbering** for the node-keyed stores: after B, a signature-link entry costs ~7.5 bytes; a 4-byte
  cell at the measured occupancy (0.86 -> 0.51) saves at most 4-5 MB at one checker and 1-2 MB at 32, for a per-kind
  ordinal in the parser, the generated AST and a node-to-file map.
- **Symbol-id groups for the hashed symbol stores** (2b): 1-21% occupancy, keys without ids at insert, assigning ids
  is observable.
- **Type-id tables** for the type-keyed caches (2b).
- **Lazily committed zero pages** (Bun's `Cells`): page-granular tables cost 23-632 MB more than the chosen layouts
  at 4-32 checkers (table above). Bun's tables are program-wide and shared by its tasks; tsrs's are per checker
  over a shared id space.
- **Inline 16-byte values for `value_symbol_links`**: worse at every checker count (table). G, 32-id offset groups
  behind A's two-level index, is modeled at -11.7 MB at 32 checkers but +1.2 MB at one; not prototyped.

## 4. Numbers

### 4a. Per change, modeled from the census (MB, summed over checkers)

| change | vscode 1 | 4 | 16 | 32 | t3 1 | t3 32 | arithmetic |
| --- | --- | --- | --- | --- | --- | --- | --- |
| A symbol-node links inline | 26.2 | 27.7 | 32.0 | 36.1 | 5.2 | 16.2 | today's groups + index + chunks - (level 1 + 128 B x blocks + 128 B x 32-id groups) |
| B index-only buckets | 16.8 | 16.8 | 17.3 | 20.2 | 3.1 | 11.4 | 4/9 x (signature + type-node slot tables) |
| C reference kinds in the slot | 6.5 | 6.9 | 7.7 | 8.3 | 1.1 | 3.9 | the value chunks |
| D small chunks | 0.1 | 0.5 | 2.2 | 6.7 | 0.1 | 7.2 | last-chunk waste, 1,024 vs 64-doubling |
| E mapped via value groups | 0.4 | 0.6 | 0.7 | 1.0 | 2.2 | 15.4 | mapped slot table - (256 B x mapped groups + 4 B x value groups) |
| **A + B + C** | **49.5** | **51.4** | **57.0** | **64.6** | **9.4** | **31.5** | |
| A-E | 50.0 | 52.5 | 59.9 | 72.3 | 11.7 | 54.1 | |

### 4b. Prototypes, measured (macOS, peak footprint MiB, median of 3 interleaved rounds, range)

A = the two-level variant; A + B + C on top. Full `--pretty false` output identical to the base in every run.

| project, checkers | base | A | A + B + C | change A + B + C | model A + B + C in MiB |
| --- | --- | --- | --- | --- | --- |
| vscode, 1 | 1,983.7 (1,980.5-1,986.1) | 1,960.7 (1,960.5-1,962.2) | 1,937.3 (1,935.3-1,940.5) | -46.4 (-2.3%) | -47.2 |
| vscode, 4 | 2,181.9 (2,177.5-2,183.1) | 2 rounds: -14, -35 | 2,128.7 (2,121.4-2,130.6) | -53.2 (-2.4%) | -49.0 |
| vscode, 16 | 2,520.9 (2,519.1-2,523.7) | 2 rounds: -32, -22 | 2,471.0 (2,470.1-2,475.0) | -49.9 (-2.0%) | -54.4 |
| vscode, 32 | 2,793.9 (2,791.9-2,801.1) | 2,763.0 (2,755.6-2,763.5) | 2,718.6 (2,717.1-2,724.9) | -75.3 (-2.7%) | -61.6 |
| t3code-server, 1 | 869.4 (867.3-872.7) | 2 rounds: -4, -4 | 857.9 (854.8-858.5) | -11.5 (-1.3%) | -9.0 |
| t3code-server, 32 | 2,828.0 (2,813.9-2,852.4) | 2 rounds: -21, +37 | 2,780.6 (2,770.7-2,783.1) | -47.4 (-1.7%) | -30.0 |

The prototype census agrees with the model: symbol-node links 44.9 -> 18.7 MB at one checker, and 32.7 MiB of groups
and tables at 32 checkers against 70.8 MiB; signature and type-node buckets 18.0 -> 10.0 MiB each at one checker.
Measured and modeled differ by -4.5 to +17.4 MiB; the larger measured drops at 32 checkers are presumably heap pages
left unused per checker thread (inferred). The model is the conservative figure there. t3code-server at 32 checkers
varies by 40 MiB between runs of the same binary.

One round on the other bench projects (1 / 8 checkers, peak MiB, base -> A + B + C), diagnostics identical: webpack
321.0 -> 315.4 / 437.4 -> 429.0 (840 errors), xstate-main 197.9 -> 196.8 / 270.9 -> 268.9, cal-diy 848.8 -> 846.3
/ 1,563.2 -> 1,517.6 (136), supabase-studio 986.1 -> 985.4 / 1,495.9 -> 1,475.3 (9), formbricks-web 1,398.6 ->
1,385.0 / 2,022.2 -> 2,010.7, mui-docs 788.6 -> 785.3 / 1,334.5 -> 1,332.9.

### 4c. Instructions

| run (vscode) | base | A | A + B + C |
| --- | --- | --- | --- |
| `--singleThreaded`, 2 runs | 111.50, 111.02 G | 111.17, 110.35 G | 111.24, 111.20 G |
| 1 checker, 3 runs | 112.25, 110.65, 110.75 G | 111.75, 111.36, 111.53 G | 111.44, 111.37, 111.49 G |
| 32 checkers, 3 runs | 140.40, 139.75, 139.86 G | 138.81, 138.48, 139.27 G | 138.18, 138.97, 138.89 G |

No change outside the noise (macOS's count includes page-fault work and moves by up to 1.4% between identical
runs). Per lookup: A is three dependent loads after the node's id instead of four (notes/mem-64.md 2a counts "two
dependent loads" for today's id-to-slot part, then the chunk pointer and the value); B the same as today on a hit;
C two fewer. Not the "one dependent load instead of a hash probe" of Bun's tables: tsrs reaches the id through the
key object and the groups through a level-1 entry.

## 5. Effort, risk, tests, order

| change | effort | hazards | tests |
| --- | --- | --- | --- |
| A | 2-3 days: one store type (prototype ~110 lines), the field type, census rows | `&'static` cells must never move (arena groups only, never freed or recycled); `try_get` semantics (above); `get_node_id` calls stay where they are; wide ids | links.rs tests rewritten (today's at links.rs:389-439 cover the offset groups): scrambled ids over many blocks map to their own cells, `try_get_if_id_assigned` never assigns, ids >= 2^32 |
| B | 3-4 days: store type, a key cell in two value types with size assertions, compressed-pointer builds only | the key must be written before the index is inserted (handle 0 is never an object, reserve.rs:5-7, so a default key never matches); rehash reads keys from values that must not move; services.rs:403-421 keeps `P<SignatureLinks>` across a check | growth through many rehashes with scrambled keys, every key finds its own value; a forced 7-bit tag collision |
| C | 1 day | none beyond the API change at three sites | absent symbol reads None; flags accumulate |
| D | 1 day | chunk arithmetic per store | first-chunk boundary |
| E | 3-5 days | couples the mapped store to the value-symbol groups; every mapped key must already have an id (assert) | mapped keys in groups with and without value links |
| Bun-style numbering | weeks | parser and generated AST, a node-to-file map | |

Every change keeps slots, ids and first-access order, so no checker result can change; gates as in
notes/mem-64.md section 5: full `--pretty false` diagnostics on vscode, webpack and xstate-main at 1, 4 and 16
checkers, the conformance suite (`.github/scripts/conformance-gate.sh`, no lost entries in `pass.txt`), the census
free-gate for the new arena types (docs/DEBUGGING.md "The census free-gate"), `tools/lint/ratchet.py`, and the
bench's peak-memory flag.

Recommended order:

1. **A** (largest, one store, call sites unchanged in the prototype).
2. **B and C** together (both replace a hashed `LinkStore` with a narrower table; 23-29 MB on vscode).
3. **D** if the 32-checker numbers matter (7 MB there, nothing at one checker).
4. **E** only for projects shaped like t3code-server (15 MB there at 32 checkers, 1 MB on vscode).

## 6. Not measured, inferred

- Linux: no probe run. The arena is huge-page backed there (reserve.rs:9-11), so A's arena savings should show in
  RSS as measured here (C removes arena chunks too); B shrinks mimalloc tables (built with `no_thp`, docs/RUST.md).
- The conformance suite was not run on the prototypes (no `ts-ref` checkout); diagnostics were compared on eight
  bench projects only.
- D, E and G are modeled from the census, not prototyped. The 4- and 16-checker A columns and the t3code-server A
  column have two rounds each.
- Instructions: macOS counts only; the user-space share of the change is not separable here.
- Store sizes per kind of key (2b) come from a walk of every file's AST with `for_each_child` (no JSDoc), at the
  end of checking.
- The node builder's and emit resolver's link stores (1c) were not measured; `--noEmit` runs create few node
  builders.
- Bun's own per-table byte counts (its `Footprint`, table.rs:363-412) were not obtained; the comparison with Bun is
  by design, not by measurement.
