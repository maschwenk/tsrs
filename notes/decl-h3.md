# decl-h3: transform_3.rs (transform.go 2328–3002), util.rs, supplementalreferences.rs

All functions ported; no `todo!()` left in the three files.

## Signature changes

- `DeclarationTransformer::ensure_type_params(node, params: P<NodeList>)` -> `params: Option<P<NodeList>>`.
  Go `*ast.TypeParameterList` is nil for declarations without type parameters (`VisitNodes(nil)` returns nil), so
  callers in transform_1/2.rs pass `node.type_parameter_list()` (Option) directly.

## Shared-file edits

- `crates/tsrs_ast/src/utilities_3.rs` (end): `is_contextual_keyword(Kind)`, `is_non_contextual_keyword(Kind)`
  (Go `ast.IsContextualKeyword` / `ast.IsNonContextualKeyword`, missing), and
  `impl SourceFile { fn supplemental_source_files(&self) -> &'static [P<SourceFile>] }` returning `&[]`
  (Go `SourceFile.SupplementalSourceFiles()`; content mappers are not ported, same as `is_content_mapped()` = false).

## Needs from others

- transform_2.rs `transform_common_js_export(&self, input, name) -> P<Node>` should return `Option<P<Node>>`
  (Go returns nil when `transformCommonJSExportWorker` does). My call sites (`visit_nested_expression`) use
  `let r: Option<P<Node>> = self.transform_common_js_export(..).into();`, which compiles with either signature.
- transform_1.rs `transform_export_assignment(..) -> P<Node>`: Go checks the result for nil in
  `visitCJSExportAssignments`; all Go returns look non-nil, but my call site uses the same `.into()` pattern so an
  `Option` return also compiles.
- transform_2.rs `rewrite_module_specifier(parent, input: P<Node>) -> P<Node>`: Go handles `input == nil`. All my
  call sites pass non-nil (ImportDeclaration.module_specifier, ExternalModuleReference.expression unwrapped).
- transform_2.rs `check_entity_name_visibility(entity_name, enclosing_declaration: P<Node>)`: I pass
  `self.enclosing_declaration.get().unwrap()` (Go never has it nil during the transform).

## Doubts

- `ensure_type_params` builds the `TypeParameterList` directly (`P::new(NodeList { loc: node.loc(), nodes })`),
  like Go's struct literal, bypassing factory hooks.
- `unwrap_parenthesized_expression` keeps the generated `-> Option<P<Node>>` signature though Go never returns nil
  (always `Some`); callers unwrap.
- `SupplementalReferencesTransformer::transform_source_file` writes `source_file.referenced_files` (an `OwnedCell`
  of a shared file, as Go appends to `ReferencedFiles`); unreachable today because the supplemental list is empty.
- `create_full_expando_block` iterates a SyntaxList with `Node::iter_children()` (Go `n.AsSyntaxList().IterChildren()`
  is the promoted `Node.IterChildren`, i.e. ForEachChild order).
