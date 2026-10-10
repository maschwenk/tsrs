# mem-dense-link-tables: dense id-indexed link tables (Bun's `ByNode` / `ById`), design and verified estimate

Status (2026-10-10): changes A, B and C below landed (notes/mem-link-tables-landed.md): `symbol_node_links` is an
`InlineIdStore`, `signature_links` and `type_node_links` are `KeyedLinkStore`s, `symbol_reference_links` is a
`SymbolReferenceLinkStore` (crates/tsrs_checker/src/links.rs). D and E were not built. This note keeps the density
measurements, the rejected designs and the unbuilt candidates; the per-store inventory and the prototype step log
were removed (git history has them).

notes/bun-check-memory.md listed "dense link tables" as a layout-only technique worth 60-90 MB on vscode at one
checker. Bun keeps what it computes per node, symbol and type in arrays indexed by dense ids whose zero pages are
committed on first touch (Bun `src/sema/table.rs`, `util/memory.rs`). This note measured what tsrs's link stores
held, how dense their keys are in every id space they have or could get, and prototyped the layouts that pay.

Base: origin/main 1cbf079. Machine: Apple M5 Max, 18 cores, 16 KiB pages, macOS 26.6. Projects: vscode (`-p src`)
and t3code-server (`-p apps/server`). MB = 10^6 bytes in the arithmetic, MiB = 2^20 bytes for peaks; byte counts are
summed over all checkers.

**Result.** The 60-90 MB does not hold for Bun's technique as such. Only one store has a dense key that pays:
`symbol_node_links` with its 4-byte value stored in the id-indexed table itself (26 MB at one checker, 36 MB at 32).
Two hash-layout changes in the same spirit (signature and type-node links keep a 4-byte index per bucket and find
the key in the value's padding; reference kinds live in the slot) brought the prototype to **-46 MiB at one checker
and -75 MiB at 32 checkers on vscode** (-2.3% / -2.7% of peak), -12 / -47 MiB on t3code-server, instructions
unchanged within the noise. Bun-style per-file, per-kind node numbering would add at most 4-5 MB on top and needs an
AST change.

At 1cbf079 the link stores held 241 MB on vscode at one checker and 411 MB at 32.

## Constraints from the callers (still true)

- **Values never move, and callers rely on it.** `get_resolved_symbol` holds its symbol-node links across
  `resolve_name`, which creates more; `run_without_resolved_signature_caching` keeps `P<SignatureLinks>` and
  `P<ValueSymbolLinks>` in a vector across a nested check. A layout in which a value can move (a value in a hashbrown
  slot moves on every rehash) is safe only for a store none of whose sites keeps the value: of the large stores, only
  `symbol_reference_links`.
- **No iteration**, so slot order and first-access order are internal.
- **Ids are observable.** Node ids appear in cache keys and internal names; symbol ids in the internal names of
  unique-symbol-keyed properties, whose length the node builder counts toward truncation. A store may use an id where
  Go's code already assigns one; it must not assign one Go does not.

## Id density

Share of the ids in a touched group (or page) that have links, vscode:

| store | grouping | 1 | 4 | 16 | 32 checkers |
| --- | --- | --- | --- | --- | --- |
| symbol_node_links | 32 ids | 0.98 | 0.87 | 0.73 | 0.67 |
| | 128 ids | 0.98 | 0.72 | 0.49 | 0.41 |
| | 1,024 ids (a 4 KiB page of 4-byte cells) | 0.98 | 0.40 | 0.19 | 0.14 |
| | 4,096 ids (a 16 KiB page) | 0.97 | 0.27 | 0.10 | 0.07 |
| value_symbol_links | 32 ids | 0.87 | 0.76 | 0.64 | 0.59 |
| | 128 ids | 0.87 | 0.62 | 0.42 | 0.36 |
| | 1,024 ids | 0.87 | 0.35 | 0.17 | 0.13 |
| | 4,096 ids | 0.87 | 0.25 | 0.09 | 0.06 |

Ids come from per-thread blocks of 1,024 (`use_id_blocks`, crates/tsrs_ast/src/utilities_1.rs): a checker's own
blocks are full, the other checkers' blocks thin. Small groups follow that; OS pages (Bun's unit of lazy commit) do
not.

The hashed stores have no usable dense key:

- **Node ids: no.** Keys of the node-keyed stores almost never have a node id when their links are created (0.3% of
  signature-link keys, 0.9% of type-node-link keys, 1.6% of node-link keys at one checker). Assigning one would change
  later ids.
- **Symbol ids: too thin.** Where keys have ids, each store fills 1-21% of the 128-id groups it touches. A 128-id
  group costs 264 bytes and a hashed entry ~14, so groups pay above ~0.15 occupancy and only if every key has an id.
  Modeled, the symbol stores above 20,000 entries lose 2.8-6.0 MB each at one checker and 6.5-18.6 MB at 32; only
  `mapped_symbol_links` gains (E below).
- **Handles: no.** Signature-link keys lie in 547,463 distinct 1-KiB ranges of the arena at one checker; a table
  indexed by handle would spend ~200 bytes per key.
- **Bun's numbering (per file, per node kind): dense for call-like keys, not available.** Signature links would fill
  0.86 of the cells at one checker and 0.51 at 32, but tsrs nodes carry no per-kind ordinal (`Node` has no spare field;
  a 4-byte ordinal costs 8 bytes per node after alignment, 137 MB on vscode's 17.1M nodes) and a node does not know its
  file.
- **Type ids: dense per checker, but the caches are sparse in them** (`lazy_member_tables` 0.08 / 0.07 of the 128-id
  groups they touch at 1 / 32 checkers): 4-byte cells would cost 6.3 / 10.2 MB more than the hash on vscode.

## The changes (A-C landed, D-E not built)

Modeled savings from the census (MB, summed over checkers):

| change | vscode 1 | 4 | 16 | 32 | t3 1 | t3 32 |
| --- | --- | --- | --- | --- | --- | --- |
| A symbol-node links inline in 32-id groups, two-level index | 26.2 | 27.7 | 32.0 | 36.1 | 5.2 | 16.2 |
| B index-only buckets, key in the value's padding | 16.8 | 16.8 | 17.3 | 20.2 | 3.1 | 11.4 |
| C reference kinds in the slot | 6.5 | 6.9 | 7.7 | 8.3 | 1.1 | 3.9 |
| D small value chunks for small stores | 0.1 | 0.5 | 2.2 | 6.7 | 0.1 | 7.2 |
| E mapped links via the value-symbol groups | 0.4 | 0.6 | 0.7 | 1.0 | 2.2 | 15.4 |

- A: a flat 32-id index (one 4-byte entry per 32 ids) is not enough: it is 21 MB at 32 checkers and is reallocated as
  it grows; that prototype measured +7 / +8 MiB peak at 32 checkers against -16 / -24 MiB at one. The two-level index
  fixed it.
- B: both bucket tables 18.0 -> 10.0 MiB in the prototype's census on vscode at one checker. Plain-pointer builds have
  no padding room.
- C: -6.5 MB at one checker, -8.3 MB at 32 (vscode).
- D (not built): the unused part of each store's last 1,024-value chunk is 0.5 MB at one checker and 12.8 MB at 32 on
  vscode, mostly `source_file_links` (152-byte values), `switch_statement_links`, `value_symbol_links`,
  `deferred_symbol_links`. Chunks that start at 64 values and double to 1,024 save 6.7 MB at 32 checkers; a per-store
  constant chunk size keeps `at` free of extra instructions.
- E (not built): `IdGroup` gets a 4-byte pointer to a lazily allocated 256-byte array of mapped-link offsets,
  replacing the mapped slot table. Couples two stores; worth it only on projects shaped like t3code-server.

Prototype A + B + C, measured (macOS peak footprint, median of 3, full `--pretty false` output identical): vscode
1,983.7 -> 1,937.3 MiB at 1 checker (-2.3%), 2,793.9 -> 2,718.6 at 32 (-2.7%); t3code-server 869.4 -> 857.9 at 1,
2,828.0 -> 2,780.6 at 32. vscode instructions unchanged within macOS's noise (up to 1.4% between identical runs).

## Not proposed

- **Bun-style node numbering** for the node-keyed stores: after B, a signature-link entry costs ~7.5 bytes; a 4-byte
  cell at the measured occupancy (0.86 -> 0.51) saves at most 4-5 MB at one checker and 1-2 MB at 32, for a per-kind
  ordinal in the parser, the generated AST and a node-to-file map.
- **Symbol-id groups for the hashed symbol stores**: 1-21% occupancy, keys without ids at insert, assigning ids is
  observable.
- **Type-id tables** for the type-keyed caches.
- **Lazily committed zero pages** (Bun's `Cells`): page-granular tables cost 23-632 MB more than the chosen layouts at
  4-32 checkers on vscode (`value_symbol_links` as 1,024-id pages: 193-774 MB against 83-143 MB for 128-id groups).
  Bun's tables are program-wide and shared by its tasks; tsrs's are per checker over a shared id space.
- **Inline 16-byte values for `value_symbol_links`**: worse at every checker count. G, 32-id offset groups behind A's
  two-level index, is modeled at -11.7 MB at 32 checkers but +1.2 MB at one; not prototyped.
