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

## Needs from others

- `tsrs_printer` (printer-pkg) API assumed — see CHECKER.md "The tsrs_printer seam"; at merge delete
  printer_standin.rs and re-export from `tsrs_printer` in printer_types.rs. `SymbolAccessibility`/
  `SymbolAccessibilityResult` (Go printer/emitresolver.go) live in printer_types.rs today; if tsrs_printer also
  exports them, keep one.
- Not ported but called by the node builder / symbolaccessibility (lead decision needed): EmitResolver
  `hasVisibleDeclarations` (affects every `isSymbolAccessible` -> symbol chains in messages), `isEntityNameVisible`,
  `requiresAddingImplicitUndefined`; nodecopy.go `tryReuseExistingNodeHelper`, `reuseNode`, `tryJSTypeNodeToTypeNode`;
  pseudotypenodebuilder.go + `internal/pseudochecker`; `internal/modulespecifiers` (`GetModuleSpecifiers`,
  `CountPathComponents`) for `import("…")` type names.
- `Program` trait: Go `Host` = `modulespecifiers.ModuleSpecifierGenerationHost`; `NodeBuilder.host` /
  `NodeBuilderContext.host` are `&'static dyn Program`, which may need those host methods once modulespecifiers is ported.

## Doubts

- `clone_binding_name_visitor` left `None`: Go's reusable visitor calls `b.cloneBindingName` (needs the checker); a
  `'static` `VisitFn` cannot capture `&mut Checker`. The body port must pick a strategy (e.g. a per-call visitor with a
  documented raw checker pointer, or an equivalent manual child rebuild).
- `id_to_symbol` is copied from the caller's map (Go shares it; only language-service inlay hints read it back).
- tsrs_compiler does not compile at the base commit (`get_diagnostics` is private, program.rs:1233) — unrelated,
  pre-existing.
