# dod-semantic-tables: transient symbols as dense per-checker rows (yuku-style tables for the semantic graph)

Experiment asked for by the owner (2026-10-10): rework tsrs's semantic graph in the data-oriented style of yuku
(https://github.com/yuku-toolchain/yuku, commit df7b067), id-indexed tables with no per-object headers, aimed at the
per-checker state that is tsrs's memory gap to `bun check`. A sibling branch (dod/ast-tables) does the syntax tree.
This note covers what yuku does, what was built (step 1 of the brief: symbols as rows), every number base vs new,
the fidelity evidence, what did not work, and what a follow-up would do.

Base: origin/main c71ed097. Branch `dod/semantic-tables`. Mac (Apple M5 Max, 18 cores, 128 GB), shared with
another agent's builds, so wall times are reported with their spread; instructions retired (`/usr/bin/time -l`) are
deterministic and are the primary CPU metric. MiB = 2^20 bytes.

## 1. Prior evidence, and what is different now

- notes/mem-compact-ast-sizing.md counted the real trees: the whole tree is 6-21% of the peak, layout-only designs
  save 1-4% of it, and it recommended against a compact-tree project for memory. It counted bytes only and never
  measured the CPU side of a column layout.
- notes/bun-check-memory.md and notes/mem-per-checker-duplication.md: the gap to `bun check` is per-checker state.
  Per extra checker on t3code-server (reachable at exit): `Symbol` 11.5 MiB, value-symbol link slots 7.9,
  `TypeReference` 7.6, `[P<Type>]` 7.0, `LazyMemberTable` 5.6, `Signature` 5.2, `TypeMapper` 3.8. 62-69% of an extra
  checker's memory is its own copy of the type graph; the objects are already header-less arena rows.
- notes/mem-dense-link-tables.md: Bun-style dense link pages keyed by id cost 23-632 MB more than the landed 128-id
  groups at 4-32 checkers on vscode, because ids a checker touches in other checkers' blocks are sparse. That is why
  this note gives rows only to the symbols a checker creates itself (always dense for that checker) and leaves binder
  symbols on the id-keyed store.
- notes/perf-round2-followups.md "Measured and rejected", docs/RUST.md "Techniques", notes/mem-round4.md: read; no
  entry covers dense creation-order rows for transient symbols.

Different now: the owner asked for the full data-oriented rework as an experiment, with the CPU side measured
(cache behaviour of columns, dense indices instead of id-keyed groups), not only bytes.

## 2. What yuku does (src/parser/semantic/, read, not run)

- `Semantic` (binder.zig) is a set of flat tables per tree: `symbols: []const Symbol` ("every symbol, in
  declaration order, indexed by `SymbolId`"), `references: []const Reference`, `scopes: ScopeTree`, and side columns
  keyed by the same dense ids: `decl_nodes` + a `Range` per symbol, `use_ids` / `use_ranges` per symbol,
  `node_symbols`, `node_references`, `node_scopes`, `node_parents` keyed by `NodeIndex`, `scope_maps` (one string map
  per scope). `SymbolId`, `ScopeId`, `ReferenceId` are `enum(u32)` with a `none` sentinel.
- A `Symbol` is 16 bytes of plain data (name slice, `Flags` packed u32, `scope: ScopeId`, `decls: Range`): no
  header, no id field (the index is the id), no lazily assigned state. Symbols are an array of structs; the tree's
  nodes are the struct-of-arrays `MultiArrayList` (ast.zig `Tree`: `{tag, data, span}` columns, children as
  `NodeIndex`, lists as `IndexRange` into one `extras` array).
- The tables are built by one pass (`SymbolTracker`, `std.ArrayList(Symbol)`) and then frozen as slices; nothing is
  allocated per symbol after that. There is no checker-created symbol: yuku's checker (checker.zig) is a
  resolver over these tables, so "transient symbols" and per-checker link stores have no counterpart.

The part of this that maps onto tsrs: a dense index assigned at creation as the handle of every side table, and
side data as columns of that index rather than hash maps or id-keyed groups.

## 3. What tsrs had (base c71ed097)

- `Symbol` (crates/tsrs_ast/src/symbol.rs) is a 32-byte arena row: `flags` 4, `check_flags` 4 (non-zero only in
  the checker's transient symbols), `name` 8 (tagged), `declarations` 8, `id` 4 (assigned lazily by
  `get_symbol_id`, process-wide counter in per-thread blocks of 1,024), `parent_or_tables` 4; a `SymbolTables` tail
  (24 B) for the ~5% with members / exports / export symbol. `P<Symbol>` is the arena offset (32-bit handle).
- The checker's per-symbol state is 17 link stores keyed by the symbol's identity or id. `value_symbol_links`
  (resolved type, target, mapper: 16 B records) is the large one, keyed by the lazily assigned id through 128-id
  groups (`IdLinkStore`: group index -> group -> 16-bit slot offset -> chunk -> record).
- A transient symbol's check flags are read on most property paths (`get_type_of_symbol` reads them first).

## 4. What was built (step 1: transient symbols as rows)

Code: crates/tsrs_ast/src/symbol.rs (`TransientRow`, `TransientSymbols`, `TransientColumns`, the registry),
crates/tsrs_checker/src/links.rs (`RowLinks`, `ValueSymbolLinkStore`), crates/tsrs_checker/src/checker.rs (the
table per checker), `Checker::new_symbol` (checker_07.rs). All 1,515 `P<Symbol>` sites are unchanged; the 8 writers
and ~40 readers of the `check_flags` field use the accessors.

- Every symbol a checker creates (`Checker::new_symbol`, Go `newSymbol`) gets the next row of that checker's
  `TransientSymbols` table at creation. The row word replaces the `check_flags` field in the 32-byte `Symbol`:
  owner (the table's registry slot, 7 bits), a "has value links" bit, row (24 bits). Binder symbols keep 0. The
  symbol id is untouched: still lazily assigned by the same calls in the same order (ids are observable:
  `__@name@id` internal names, `compare_symbols` ties, the API codec), so every output that depends on ids is
  byte-identical by construction.
- `check_flags` is a column of the owning table: arena chunks of 4,096 `OwnedCell<CheckFlags>` (16 KiB), one
  installed when the first row of the chunk is handed out. A read goes registry slot -> columns -> chunk -> cell (3
  dependent loads where the field was 1); a binder symbol answers `None` from the row word alone. The registry is a
  static array of 127 atomics so that a shared-graph fork, the language service and the API can read another
  checker's column through a `&Symbol` without a checker in hand (the "zero-sized view" idea of the brief, applied to
  one column).
- `value_symbol_links` became `ValueSymbolLinkStore`: for the checker's own rows a `RowLinks` column (arena chunks of
  4,096 16-byte records, allocated when the first row of the chunk is linked; no key, no index, no slot offsets),
  and for every other symbol (binder symbols; the frozen seed's symbols in a `--features shared-graph` fork) the
  id-keyed `IdLinkStore` as before. `get` / `try_get` / `has` still call `get_symbol_id` first, as Go's
  `symbolArenaLinkStore` does, so id assignment order is unchanged. Presence ("were links created for this symbol")
  is the bit in the row word, which keeps `has` exact.
- Shared graph: a fork's store reads a seed symbol's record from the seed's row column on first access and copies it
  into the fork's own id-keyed store, as the fork copied from the seed's id store before. `--maxMemory`: the table and
  both columns live in the checker's region and heap and go with the retired checker; the registry slot is freed in
  `Drop` and reused by the replacement.
- Not done, with the count that decides it: the `SymbolTables` tail as a column. The tail is 24 B and ~5% of symbols
  have one, mostly binder symbols (classes, modules, exported locals); a dense 24-byte column over every transient
  row would cost ~20x what the tails cost. The `declarations` and `name` words were not moved either: a transient
  symbol's declarations are usually the target's slice (`set_declarations_static`), already one word.

Arithmetic before measuring (per transient symbol): the row stays 32 B; check flags +4 B dense (they were free in the
row); value links 16 B inline for every row of a chunk with at least one linked row, against 16 B + 2 B group slot +
~0.1 B index per *linked* symbol before. Net about +2 B to +4 B per transient symbol if nearly every one is linked
(they are: `new_property` / `new_parameter` / `create_symbol_with_type` link at creation), i.e. no memory saving is
possible from this layout; the experiment's memory question is whether the dense columns cost measurable peak. The
CPU question is whether 2 fewer dependent loads on every `value_symbol_links.get` of a transient symbol (row word ->
chunk -> record, instead of id -> group index -> group -> slot -> chunk -> record) outweigh 2 more on every
`check_flags` read (registry -> columns -> chunk -> cell).

## 5. Numbers

(filled in below)

## 6. Fidelity

(filled in below)

## 7. What did not work / what a follow-up would do

(filled in below)
