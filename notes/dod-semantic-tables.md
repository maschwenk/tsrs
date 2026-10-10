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

Interleaved base and new runs, same Mac, same session (`scripts/measure.sh 5`: 5 reps each, the rep order alternating
base, new). Median, the spread (min-max), and the change new vs base. Instructions are the deterministic metric; the
peak is the `peak memory footprint` line of `/usr/bin/time -l`; wall is `real`, noisy (it moves by more than the
effect on every project).

| project | mode | metric | base (median, spread) | new (median, spread) | delta |
| --- | --- | --- | ---: | ---: | ---: |
| vscode | single | instructions (G) | 97.173 (96.651-98.536) | 97.549 (96.898-97.917) | +0.39% |
| vscode | single | peak (MiB) | 1602.3 (1597.5-1611.0) | 1609.5 (1606.7-1619.0) | +0.45% |
| vscode | single | wall (s) | 7.36 (7.19-8.63) | 7.61 (7.16-7.73) | +3.4% (noise) |
| vscode | default | peak (MiB) | 2096.9 (2087.8-2101.2) | 2113.4 (2110.1-2116.8) | +0.79% |
| vscode | default | wall (s) | 0.89 (0.84-1.39) | 0.89 (0.87-0.90) | 0.0% |
| t3code-server | single | instructions (G) | 46.560 (46.482-47.032) | 47.256 (47.099-47.397) | +1.50% |
| t3code-server | single | peak (MiB) | 731.5 (728.8-734.3) | 739.0 (732.2-741.6) | +1.03% |
| t3code-server | single | wall (s) | 3.32 (3.09-4.77) | 3.46 (3.26-5.31) | +4.2% (noise) |
| t3code-server | default | peak (MiB) | 2151.0 (2143.4-2203.7) | 2172.1 (2139.3-2212.6) | +0.99% |
| t3code-server | default | wall (s) | 1.50 (1.34-1.67) | 1.75 (1.38-1.92) | +16.7% (noise) |
| webpack | single | instructions (G) | 13.041 (13.036-13.449) | 13.134 (13.131-13.312) | +0.72% |
| webpack | single | peak (MiB) | 303.0 (302.4-303.3) | 304.4 (303.5-305.4) | +0.46% |
| webpack | single | wall (s) | 0.92 (0.83-1.14) | 0.85 (0.83-1.05) | -7.6% (noise) |
| webpack | default | peak (MiB) | 495.0 (490.5-507.0) | 502.8 (499.6-509.1) | +1.57% |
| webpack | default | wall (s) | 0.14 (0.13-0.14) | 0.14 (0.13-0.14) | 0.0% |
| formbricks-web | single | instructions (G) | 48.400 (48.283-50.708) | 48.862 (48.487-49.687) | +0.96% |
| formbricks-web | single | peak (MiB) | 1093.9 (1091.9-1096.9) | 1099.2 (1095.9-1103.8) | +0.49% |
| formbricks-web | single | wall (s) | 3.92 (3.36-5.35) | 3.74 (3.35-4.26) | -4.6% (noise) |
| formbricks-web | default | peak (MiB) | 1986.0 (1962.1-2007.3) | 1998.5 (1990.6-2007.9) | +0.63% |
| formbricks-web | default | wall (s) | 0.68 (0.65-1.01) | 0.68 (0.66-0.73) | 0.0% |
| cal-diy | single | instructions (G) | 39.228 (38.769-40.420) | 39.121 (38.764-39.657) | -0.27% |
| cal-diy | single | peak (MiB) | 760.6 (755.4-768.1) | 766.0 (764.1-767.0) | +0.72% |
| cal-diy | single | wall (s) | 2.84 (2.55-4.18) | 2.86 (2.57-3.64) | +0.7% (noise) |
| cal-diy | default | peak (MiB) | 1892.7 (1864.1-1899.0) | 1916.5 (1854.5-1921.4) | +1.26% |
| cal-diy | default | wall (s) | 0.72 (0.71-0.78) | 0.71 (0.70-1.05) | -1.4% (noise) |

Default-count instructions are not reported (the brief asks for single-threaded instructions, and default-count
instructions vary with work stealing). Errors: 371 / 6 / 840 / 0 / 136 identical base and new in every run.

Against the bar (AGENTS.md: 1% of single-threaded instructions on one project, 2% of wall, or 5% of peak at the
default checker count): nothing clears it, and nothing is a win. The step-1 deltas are small but all positive for
peak at the default count (+0.6% to +1.6%) and for instructions on four of five projects (+0.4% to +1.5%). The
brief's stop rule (within +-0.5% everywhere) is not met, so the answer is a measured small loss, not a neutral result.

## 6. Fidelity

- Diagnostics: byte-identical base vs new (`--pretty false`) for vscode, t3code-server, webpack, formbricks-web and
  cal-diy in single-threaded, `--checkers 4`, default, and `--checkerAssignment go` (20 pairs; /tmp/dod-sem/ref).
- `--extendedDiagnostics` counters (Files, Lines, Identifiers, Symbols, Types, Instantiations): identical in single
  mode on all five projects.
- Under `--checkerAssignment go`, Identifiers, Types, Instantiations and Lines are identical, but the Symbols counter
  is not reproducible even on the base binary: repeated base runs of cal-diy give 5211353-5211723, and of t3code-server
  give 6122987-6123008. New runs fall inside those spreads (cal-diy 5211403-5211546; t3code-server 6122987-6123012).
  So the go-mode Symbols mismatch in the saved refs is base nondeterminism, not a change. The check cannot be exact
  for that counter, and the note records it as such.
- Determinism gate (tools/ci/determinism.sh, xstate-main, webpack, nuxt, drizzle-orm, 594 runs at 2 and 4 checkers
  and random assignments): every run identical to the single-threaded output. Self-test passes.
- Regressions (tools/regressions.sh): 29 of 29 pass.
- Conformance: `.github/scripts/conformance-gate.sh` passes with pass=13458 (minimum 13458), .types 12779,
  .symbols 12779, 0 crashes, 0 timeouts. The base pass list is a subset of the new pass list (0 tests lost).
- Fourslash (`.github/scripts/fourslash-gate.sh`): 4066 pass, 63 fail, the documented main values.
- `--maxMemory` (t3code-server at 1500 MB, webpack at 300 MB): diagnostics identical to base, exit status identical.

A bug found and fixed during the gate run, in the record for the follow-up: an earlier build of this branch (before
the transient-table blocks became process-wide, commit 0342f8ed has the final design) used a 127-slot static registry
for the owner tables. The conformance run then panicked in 3110 tests with "more than 127 live transient symbol
tables", because the pool leaks its checkers and the slots were never freed. The final design numbers blocks
process-wide (`BLOCKS`, 20 bits), so no table needs a slot. The fidelity refs and measurements above were all taken
on the final binary, rebuilt from HEAD.

Free lists: the brief asks that the free lists recycling transient symbols keep working. The current source has no
such recycling of `Symbol` objects: `free!` / `free_slice!` are called on mappers, inference lists, type lists and
`LazyVec` cells only (grep of crates/tsrs_checker/src). So no row word is reused for a new symbol.

Not run here: `cargo test --features shared-graph` (compiled with `cargo check --features shared-graph`, and the
shared-graph fork path is covered only by the source reading in links.rs), and the wasm/plain-ptrs builds were checked
with `cargo check` only.

## 7. What did not work / what a follow-up would do

- The layout does not save memory (section 4, the arithmetic), and it did not pay for its CPU either: step 1 is
  +0.4% to +1.5% instructions on four projects and +0.6% to +1.6% peak at the default count. The likely cost (not profiled)
  is the extra dependent loads on a check-flags read (row word -> block -> cell instead of one field), and the arena
  chunks, which hold 16 KiB of check flags for each 4,096 rows whether the rows use them or not.
- Step 2 (TypeReference, `[P<Type>]`, Signature, TypeMapper as columns) is not done. The stop rule says to do it only
  with a concrete reason it would change the result. Step 1 did not reduce the per-checker state (its columns are
  per checker too), and the same loads would apply to types. No concrete reason to expect a different result, so it
  was not started.
- Untested idea for a follow-up: store a transient symbol's check flags next to its link record in one per-row record,
  so both come from one cache line. Not measured, and it would need a new peak-memory estimate first.
- The lazily assigned id column is still there (ids decide output, section 3). A table that dropped it would need the
  audit of every id-sorted path (the API codec encoder orders handles by id) first.
