# decl-h2: transform_2.rs (transform.go 1340–2327)

All 25 functions ported; no `todo!()` left in transform_2.rs.

## Signature changes

- `transform_common_js_export(input, name) -> P<Node>` -> `-> Option<P<Node>>`: Go returns nil when the worker
  returns nil (export name already witnessed). Callers (transform_3 `visit_cjs_export_assignments`, Go 2716/2723)
  must handle `None`.
- `rewrite_module_specifier(parent, input: P<Node>) -> P<Node>` -> `(parent, input: Option<P<Node>>) -> Option<P<Node>>`:
  Go returns nil for a nil input, and transform.go:1177 passes `input.ModuleSpecifier()` of an export declaration
  (nil for `export { x }`). Callers with a non-nil specifier pass `Some(x)` and `.unwrap()` the result.
- `check_entity_name_visibility(entity_name, enclosing_declaration: P<Node>)` -> `enclosing_declaration: Option<P<Node>>`:
  its caller `checkName` passes `tx.enclosingDeclaration`, which is nil-able; `Resolver::is_entity_name_visible` takes
  an `Option` too. Callers pass `self.enclosing_declaration.get()`.

## Shared-file edits

- `crates/tsrs_ast/src/utilities_3.rs` (end): `pub fn clone_as_import_declaration(node, f: &NodeFactory) -> P<Node>`,
  Go `res := node.Clone(f); res.Kind = ast.KindImportDeclaration` (Rust `Node::kind` is immutable, so the clone is
  created as an ImportDeclaration with the same data and then goes through the factory's clone hooks, like Go's `Clone`).

## Needs from others

- transform_3 `ensure_type_params(&self, node, params: P<NodeList>)` must take `params: Option<P<NodeList>>`: Go
  passes `TypeParameters`, nil for every declaration without type parameters (and `VisitNodes(nil)` = nil is what
  makes it fall through to the resolver). The three call sites here (`transform_class_expression_to_declaration`,
  `transform_function_declaration`, `transform_class_declaration`) currently `.unwrap()` the list with a `// SIG:`
  comment; they panic on a nil list until the signature is fixed. After the merge: drop the `.unwrap()`.
- transform_3 `update_param_list(node, params: P<NodeList>)`: called with `input.parameter_list().unwrap()` (Go reads
  `len(params.Nodes)`, so it never gets nil there). Fine as is.
- util `unwrap_parenthesized_expression(o) -> Option<P<Node>>`: Go never returns nil; unwrapped at the call site.
- The visitor callbacks keep their generated signatures: `strip_export_modifiers(&self, P<Node>) -> P<Node>` (install
  as `Rc::new(move |_, n| Some(tx.strip_export_modifiers(n)))`; Go's `statement == nil` guard is unreachable from a
  visitor) and `visit_this_property_assignments(&self, P<Node>) -> Option<P<Node>>`.

## Doubts

- `transform_common_js_export_worker`: Go's `defer` resetting `tracker.watchedClassSymbol`/`classSymbolTracked` is
  registered inside the `hasExprName` branch; every path of that branch returns, so it is run right after the branch
  body (closure) computes its result. Equivalent unless the body panics.
- `collect_this_property_assignments`: Go assigns the fresh set to `tx.seenProperties` and `Clear()`s it on exit
  (not a save/restore); ported the same way (replace, then clear), so a nested call would also leave the outer set
  empty, as in Go.
- `transform_top_level_declaration`: the late-marked-statement filter uses `retain` instead of building a new slice
  (no one holds the old slice, so it is equivalent).
- `ensure_type` / `strip_export_modifiers` unwrap `emit_context().parse_node(..)` where Go passes it straight to
  `GetEffectiveDeclarationFlags` (which would crash on nil in Go too); `strip_export_modifiers` keeps Go's nil check.
