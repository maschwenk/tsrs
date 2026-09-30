# printer-foundation notes

Foundation for the node builder (type/symbol/signature -> text). Data model described in docs/CHECKER.md "Node builder".

## Generated stubs

- `tools/gosig/checker.json`: added `nodebuilder.go` -> `nodebuilder.rs`, `nodebuilderimpl.go` -> `nodebuilderimpl_1.rs`
  (1–1850) / `nodebuilderimpl_2.rs` (1851–end), `nodebuilderscopes.go` -> `nodebuilderscopes.rs`;
  `arenaTypes` += `NodeBuilderImpl`, `NodeBuilder`, `NodeBuilderContext`, `SymbolTrackerImpl` (`&self` receivers);
  `droppedCheckerFields` += `NodeBuilderImpl.ch` (NodeBuilder entry points get `c: &mut Checker`);
  `typeMap` += `*printer.Printer` -> `Printer` (by value), `*ast.NodeFactory` -> `&NodeFactory`;
  overrides `SymbolTrackerImpl.TrackSymbol.p1` / `PushErrorFallbackNode.p0` -> `Option` (match `trait SymbolTracker`).
- `tools/gosig/emit.go`: arena receivers win over Go value receivers (`NodeBuilder.SymbolToParameterDeclaration` is
  `&self`, the only such case); the hand-written-function scan ignores `impl Trait for T` blocks, so in-place
  regeneration no longer drops the `len/name/matches` stubs of `TypeDiscriminator`/`ObjectLiteralDiscriminator`
  (checker_15.rs, relater_1.rs are reproduced byte-identical).
- Regeneration changed only `printer.rs` among existing files: `create_printer_*` return `Printer` (was `P<Printer>`),
  `SymbolTrackerImpl` methods `&self` (were `&mut self`), `track_symbol`/`push_error_fallback_node` take `Option<P<Node>>`.
- Stub counts (functions / still `todo!()`): nodebuilder.rs 29/26, nodebuilderimpl_1.rs 73/72, nodebuilderimpl_2.rs 41/41,
  nodebuilderscopes.rs 4/4, printer.rs 76/75.
- Ported (constructors / trivial): `NodeBuilder::emit_context`, `new_node_builder`, `new_node_builder_ex`,
  `new_node_builder_impl`, `new_symbol_tracker_impl`.

## Signature changes

- none by hand (all via generator config, above).

## Shared-file edits

- `crates/tsrs_ast` (+ `tools/gen-ast/gen-ast.ts`, regenerated `generated.rs`): `NodeFactory` is a handle — all methods
  `&self`, hooks `Rc<dyn Fn>` (`NodeFactoryHooks: Clone`), counters `Rc<Cell<usize>>`, `NodeFactory: Clone` shares
  state (Go pointer copy). `clone_node`/`clone_list` take `&NodeFactory`; `deep_clone_*` take `&self` and hand the
  visitor `self.clone()` (no more `mem::take`). Widening only: every existing caller compiles unchanged, ast/parser
  tests pass. Why: the node builder nests factory calls ~60 times (`b.f.NewX(b.f.NewY())`, `b.f.NewX(b.typeToTypeNode(t))`)
  and shares the emit context's factory; a `RefCell<NodeFactory>` would turn each into a runtime borrow panic.
- `crates/tsrs_ast/src/utilities_3.rs`: added `create_modifiers_from_modifier_flags`, `replace_modifiers`,
  `has_inferred_type` (Go ast/utilities.go). All other `ast.*` functions and factory methods the node builder files
  call already existed (checked mechanically).
- `crates/tsrs_checker/src/checker.rs`: `Checker.type_to_string_nodebuilder: Option<P<NodeBuilder>>` (Go
  `typeToStringNodebuilder`, previously listed as dropped).
- `crates/tsrs_checker/src/printer_types.rs`: removed placeholders `NodeBuilderContext`/`EmitContext`/`Printer`;
  `VerbosityContext` fields are `Cell`s; `SymbolTrackerImpl.disable_track_symbol: Cell<bool>`; `trait SymbolTracker`
  gained default `as_symbol_tracker_impl()`; `impl SymbolTracker for SymbolTrackerImpl` (forwards to the inherent
  methods in printer.rs); `tsrs_printer` seam re-exports.
- New: `nodebuilder_types.rs` (data model), `printer_standin.rs` (STAND-IN for tsrs_printer), lib.rs module lines.
- docs/CHECKER.md ("Node builder" section, file table, dropped-fields and printer_types bullets), docs/AST.md (NodeFactory).

## Wave 2 (coordinator: port all unported dependencies)

- nodecopy.go -> `nodecopy.rs`, pseudotypenodebuilder.go -> `pseudotypenodebuilder.rs`, nodebuilder_hover.go ->
  `nodebuilder_hover.rs` (stubs); emitresolver.go -> `emitresolver_subset.rs` and services.go -> `services_subset.rs`
  (only reachable functions; the others are `skipFuncs`). Un-notPorting these files changed no existing signature
  (verified: docs/sigs/checker.txt has only additions, all existing stub files byte-identical).
- New crates: `tsrs_pseudochecker` (no checker dependency; tools/gosig/pseudochecker.json), `tsrs_modulespecifiers`
  (depends on tsrs_module, tsrs_tsoptions, regex; tools/gosig/modulespecifiers.json). `ProcessEntrypointEnding` not
  ported (LS auto-imports only; needs unported `module.ResolvedEntrypoint`).
- gosig changes: tolerate packages without a state-machine type; skipped functions reserve no names (a skipped
  exported forwarder in services.go had made `get_element_type_of_array_type` `pub`); `paramTypes` `"Recv.fn.r0"` result
  override (`GetConstantValue -> Option<LiteralValue>`); `*ast.NodeVisitor` -> `NodeVisitor` by value; `EmitResolver`,
  `recoveryBoundary`, `wrappingTracker` arena types.
- Shared-file edits (additive): `Program::as_module_specifier_generation_host()` (program.rs) + `todo!()` impl in
  tsrs_compiler/src/checker_program.rs (compiler must implement `ModuleSpecifierGenerationHost` + `OutputPathsHost` for
  its Program); `Checker.emit_resolver`; tsrs_ast `LiteralLikeNodeBase.token_flags` is now a `Cell` (nodecopy.go writes
  it; one `.token_flags` field read in utilities_3.rs became `.token_flags()`); workspace Cargo.toml entries.
- cloneBindingName / existing-node visitors: `CheckerSlot` (the one `unsafe` block, dynamically checked like RefCell);
  see CHECKER.md "Visitors that need the checker".

## Merge notes (main has moved on)

- main's `checker_15.rs` `get_emit_resolver` returns `P::new(EmitResolver {})`: replace with the memoized Go port
  (`if self.emit_resolver.is_none() { self.emit_resolver = Some(new_emit_resolver(self)); } self.emit_resolver.unwrap()`),
  and drop main's placeholder `EmitResolver {}` in printer_types.rs (now in nodebuilder_types.rs).
- main's exports.rs `unimplemented!("emit resolver")` (exports.go:383): replace with Go's `RequiresAddingImplicitUndefined`
  wrapper: `if !ast::is_parse_tree_node(node) { return false }` then
  `let r = self.get_emit_resolver(); r.requires_adding_implicit_undefined(self, node, symbol, enclosing_declaration)`.
- main regenerates docs/sigs/checker.txt from the Rust sources (tools/sigs-from-rust.py): rerun it after merging instead
  of merging this branch's generated sigs file.
- Stand-ins to replace at merge: printer_standin.rs (tsrs_printer), compiler `as_module_specifier_generation_host`.

## Needs from others

- `tsrs_printer` (printer-pkg): API assumed as in CHECKER.md "The tsrs_printer seam"; at merge delete printer_standin.rs
  and re-export from `tsrs_printer` in printer_types.rs. `SymbolAccessibility`/`SymbolAccessibilityResult` live in
  printer_types.rs; if tsrs_printer also exports them, keep one.
- compiler: implement `tsrs_modulespecifiers::ModuleSpecifierGenerationHost` (+ `OutputPathsHost`) for its Program.

## Doubts

- `CheckerSlot` uses one `unsafe` deref (NonNull from the lent `&mut Checker`); soundness relies on the dynamic
  `in_use` check and on callbacks re-lending before re-entering a visitor.
- `id_to_symbol` is copied from the caller's map (Go shares it; only language-service inlay hints read it back).
- tsrs_compiler does not compile at the base commit (`get_diagnostics` is private, program.rs:1233) — unrelated,
  pre-existing.

## Merged into main (nb-merge, 2026-09-30)

- Conflicts: checker.rs fields (kept main's `unassigned_type` + the two node-builder fields), Cargo.lock (main's, then
  cargo re-resolved), docs/sigs/checker.txt (regenerated). `generated.rs` merged cleanly; re-running `tools/gen-ast`
  reproduces it byte-identical (HeritageClause.types Cell, nilable ImportAttribute.name intact).
- printer_standin.rs deleted; the seam in printer_types.rs re-exports `tsrs_printer`. Reconciled in `tsrs_printer`:
  `EmitContext.factory` is a plain `NodeFactory` handle (was `RefCell`; the hooks reach the context through a shared
  slot filled by `new_emit_context`), printer `NodeFactory` methods take `&self`, `most_original` is
  `Option -> Option` (Go nil in, nil out). Kept `tsrs_printer`'s shapes where they match Go: `new_text_writer(&str,
  usize)`, `get_single_line_string_writer() -> (writer, release)`, `Printer::write(.., None)` source-map argument.
  Printer oracle unchanged (16480/16481 in all 7 modes, libs 113/113).
- `get_emit_resolver` memoized, exports.rs `requires_adding_implicit_undefined` is the Go wrapper. checker_04's
  duplicate `try_get_module_specifier_from_declaration[_worker]` moved into the nodebuilderimpl_1.rs stubs (same
  bodies), so no working path hits `todo!()`.
- tsrs_compiler implements `OutputPathsHost` + `ModuleSpecifierGenerationHost` for `Program`; `get_symlink_cache`
  (Go program.go:2309) is `todo!()`, `get_project_reference_from_source` panics only if a project reference exists
  (not ported).
- `tools/sigs-from-rust.py` also writes docs/sigs/pseudochecker.txt and modulespecifiers.txt.
