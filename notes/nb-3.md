# nb-3 notes (printer.go, symbolaccessibility.go, symboltracker.go -> printer.rs; nodebuilder.go -> nodebuilder.rs)

All functions ported; no `todo!()` or `TEMPORARY placeholder` left in printer.rs / nodebuilder.rs.
`value_to_string` (utilities.rs) now uses `tsrs_printer::escape_string`; the private `printer.EscapeString` copy is gone.

## Signature changes

Go `enclosingDeclaration` is nil for every plain `typeToString`/`symbolToString` call (and `b.ctx.enclosingDeclaration`
is `Option` in the node builder), so every `enclosing_declaration: P<Node>` below became `Option<P<Node>>`:

- printer.rs, Checker: `is_type_symbol_accessible`, `is_value_symbol_accessible`, `is_symbol_accessible_by_flags`,
  `is_any_symbol_accessible`, `get_containers_of_symbol`, `get_accessible_symbol_chain`, `needs_qualification`,
  `some_symbol_table_in_scope`, `is_symbol_accessible`, `symbol_to_string_ex_exported`, `signature_to_string_ex_exported`,
  `type_to_type_node`, `type_to_type_node_ex`, `type_predicate_to_type_predicate_node`, `signature_to_signature_declaration`.
- printer.rs: `some_symbol_table_in_scope` callback table parameter `P<SymbolTable>` -> `Option<P<SymbolTable>>` (Go passes
  `sym.Exports`, which may be nil).
- printer.rs: `vc: P<VerbosityContext>` -> `Option<P<VerbosityContext>>` in `signature_to_string_ex_exported`,
  `expand_symbol_for_hover`, `type_parameter_to_string_ex` (nil for non-hover callers).
- printer.rs: `id_to_symbol: &FxHashMap<..>` -> `Option<&FxHashMap<..>>` in `type_to_type_node`, `type_to_type_node_ex`,
  `type_predicate_to_type_predicate_node` (Go map may be nil; matches `get_node_builder_ex`).
- printer.rs: `ctx: accessibleSymbolChainContext` -> `ctx: &accessibleSymbolChainContext` in `get_accessible_symbol_chain_ex`,
  `get_accessible_symbol_chain_from_symbol_table`, `try_symbol_table`, `get_candidate_list_for_symbol`, `is_accessible`,
  `can_qualify_symbol` (see shared-file edit: the visited map must be shared like Go's map).
- nodebuilder.rs, every `NodeBuilder` entry point: `enclosing_declaration: Option<P<Node>>` and
  `tracker: Option<&'static dyn SymbolTracker>` (Go's printer.go passes nil for both); `exit_context(result: Option<P<Node>>)`
  (several impl helpers return `Option`); `exit_context_slice(result: Vec<P<Node>>)` (was `&[P<Node>]`).

`docs/sigs/checker.txt` regenerated with tools/sigs-from-rust.py.

## Shared-file edits

- printer_types.rs: seam re-exports `escape_string`, `QuoteChar` from `tsrs_printer`.
- printer_types.rs: `accessibleSymbolChainContext` is `Clone`, `symbol: Option<P<Symbol>>` (canQualifySymbol recurses
  with `symbolFromSymbolTable.Parent`, which may be nil), `visited_symbol_tables_map: Rc<RefCell<FxHashMap<..>>>` (Go
  copies the struct but shares the map).
- utilities.rs: removed the private EscapeString copy (`QuoteChar`, `escape_string_worker`, …); `value_to_string` calls
  `escape_string(value, QuoteChar::DoubleQuote)`.
- `stKind*` constants are defined in printer.rs (`stKindShift`, `stKindLocals`, …, `stKindMask`).

## Needs from others

- nb-1/nb-2 (nodebuilderimpl): pass `b.ctx().enclosing_declaration.get()` (an `Option`) straight to
  `c.get_accessible_symbol_chain`, `c.needs_qualification`, `c.get_containers_of_symbol`, `c.is_symbol_accessible`,
  `c.is_value_symbol_accessible`, `c.is_type_symbol_accessible`, `c.is_symbol_accessible_by_flags` (do not unwrap: it is
  nil for every diagnostic `typeToString`).
- nb-4: `EmitResolver::is_symbol_accessible` (emitresolver_subset.rs) must call `c.is_symbol_accessible(symbol,
  Some(enclosing_declaration), ..)` (or take `Option` itself); nodecopy.go's `b.ch.IsSymbolAccessible(..)` calls pass `Option`.
- Anyone calling `NodeBuilder` entry points: `Option` enclosing declaration / tracker.
- docs/CHECKER.md "Node builder" still says the String entry points keep TEMPORARY placeholders; they are real now.

## Doubts

- `type_to_string_ex` truncation: Go slices the byte string (`result[0:maxLength-3]`), possibly inside a multi-byte
  character; Rust uses `String::from_utf8_lossy` on the byte prefix, so such a cut shows U+FFFD instead of raw bytes.
- `stKindMask` is `(iota - 1) << stKindShift` with iota = 5 in Go, i.e. `4 << 61` (only the top bit). Ported literally:
  `get_symbol_table_aliases` therefore caches only resolved-exports tables and does not skip members tables (members
  have no aliases, so results are unchanged). Looks like a Go bug; kept for fidelity.
- `slices.SortStableFunc(candidateChains, c.compareSymbolChains)` -> `sort_by` (stable); Rust's sort may panic on a
  comparator that is not a total order where Go would not.
- `get_symbol_table_aliases` / cached chains return copies (`to_vec`) of the cached `&'static` slices.
- `is_any_symbol_accessible` returns `P::new(..)` results (leaked, as the generated `Option<P<..>>` return requires).
