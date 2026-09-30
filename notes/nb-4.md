# nb-4 notes

Ported: `nodecopy.rs` (nodecopy.go, all 29), `emitresolver_subset.rs` (14), `services_subset.rs` (4). No `todo!()` left.

## Signature changes

All in `emitresolver_subset.rs` (Go passes nil for these; the node builder calls them with `b.ctx.enclosingDeclaration`,
which is nil for context-free `typeToString`):

- `EmitResolver::is_entity_name_visible`: `enclosing_declaration: P<Node>` -> `Option<P<Node>>` (nodebuilderimpl.go:2156
  passes `b.ctx.enclosingDeclaration`; it only reaches `resolveName`, which takes a nil location).
- `EmitResolver::requires_adding_implicit_undefined` and `requires_adding_implicit_undefined_worker`:
  `enclosing_declaration: P<Node>` -> `Option<P<Node>>` (nodebuilderimpl.go:2288 passes `b.ctx.enclosingDeclaration`;
  `isRequiredInitializedParameter` tests `enclosingDeclaration != nil`, its generated signature already took `Option`).
- `EmitResolver::is_symbol_accessible`: `symbol: P<Symbol>` -> `Option<P<Symbol>>`, `enclosing_declaration: P<Node>` ->
  `Option<P<Node>>` (its only caller, `isEntityNameVisible`, passes the possibly-nil `this`-container symbol and the
  possibly-nil enclosing declaration).

## Shared-file edits

- `crates/tsrs_ast/src/utilities_3.rs`: added `is_late_visibility_painted_statement` (Go ast/utilities.go:3590), used
  by `hasVisibleDeclarations`.
- `crates/tsrs_checker/src/exports.rs` (`requires_adding_implicit_undefined`, exports.go:374): call site passes
  `Some(enclosing_declaration)` after the signature change above.

## Needs from others

- `Checker::is_symbol_accessible` (printer.rs, symbolaccessibility.go:839) should take `symbol: Option<P<Symbol>>`,
  `enclosing_declaration: Option<P<Node>>`: Go's `IsSymbolAccessible` accepts nil for both and nodecopy.go passes
  `b.ctx.enclosingDeclaration` / a possibly-nil `this`-container symbol. Adapted with `// SIG:`: nodecopy.rs has a
  private `is_symbol_accessible_nilable` and `EmitResolver::is_symbol_accessible` calls
  `is_symbol_accessible_worker(.., true /*allowModules*/)` directly (that is Go's body of `IsSymbolAccessible`). When the
  signature changes, replace both with `c.is_symbol_accessible(..)`.
- `NodeBuilderImpl::serialize_type_name` (nodebuilderimpl_1.rs, nodebuilderimpl.go:442): `type_arguments` should be
  `Option<P<NodeList>>` (nodecopy.go passes the nil-able result of `visitor.VisitNodes(...)`, and serializeTypeName hands
  it to `symbolToTypeNode`, which already takes `Option`). Adapted with `// SIG:` in nodecopy.rs
  (`serialize_type_name_nilable`: `Some` calls theirs; `None` runs the Go body inline). When fixed, call
  `b.serialize_type_name(c, node, is_type_of, type_arguments)` directly and delete the helper.
- `NodeBuilderImpl::enter_new_scope` (nodebuilderscopes.go:59) takes `original_parameters: &[P<Symbol>]`; nodecopy.go
  passes nil, and Go's `originalParameters != nil && originalParam != param` distinguishes nil from empty. I pass `&[]`,
  so its port must treat an empty slice as nil there.
- nb-1/nb-2: call `r.is_entity_name_visible(c, name, b.ctx().enclosing_declaration.get(), false)` and
  `r.requires_adding_implicit_undefined(c, declaration, symbol, b.ctx().enclosing_declaration.get())` (Option now).

## Doubts

- `hasVisibleDeclarations`: Go returns `AliasesToMakeVisible` in Go map-iteration (random) order; the port returns them
  in first-insertion order (a repeated declaration id overwrites the value in place, like the map assignment). Only
  declaration emit consumes the order.
- nodecopy.go places where Go would carry a nil through into a factory call (e.g. `visitor.VisitNode(...)` returning nil
  for the index type of an indexed access, the inner type of JSDoc nullable/optional/variadic types, the specifier of an
  import type, the branches of a conditional type, a JSDoc property tag name) are `.unwrap()`s, because the factory
  signatures take `P<Node>`. They would panic where Go would build a node with a nil child.
- `getExistingNodeTreeVisitor`'s closures are methods of a private `Copy` struct `ExistingNodeTree { b, bound }`, with
  `v: &mut NodeVisitor` standing for Go's captured `visitor`; every re-entry into the visitor (and into the inner
  `attachSymbolToLeftmostIdentifier` visitor) goes through `b.checker_slot.lend`. Go's `nonLocalNode` is an
  `Rc<Cell<bool>>` shared by the two hooks.
- `createRecoveryBoundary` calls `c.check_not_canceled()` (it only panics if the checker was cancelled before).
- `finalizeBoundary` / `getEnclosingDeclarationIgnoringFakeScope` keep the generated `c` parameter unused (`let _ = c`).
