# checker-14 notes (checker_14.rs = checker.go 28300–30509, 97 functions)

## Signature changes

- `mark_decorator_medata_data_type_node_as_referenced(node: P<Node>)` -> `node: Option<P<Node>>`: Go passes
  `getParameterTypeNodeForDecoratorCheck(...)` / `node.Type()` results, which can be nil.
- `get_entity_name_for_decorator_metadata(node: P<Node>)` -> `node: Option<P<Node>>`: Go begins with `if node == nil`.
- `get_contextual_type_for_element_expression(t: P<Type>, ...)` -> `t: Option<P<Type>>`: Go begins with `if t == nil`
  (getContextualType passes `getApparentTypeOfContextualType`'s result, which can be nil). Callers in other files must
  pass `Option`.

## Shared-file edits

None. Private helpers added in checker_14.rs only:
- `is_const_enum_or_const_enum_only_module` (Go `emitresolver.go:693`, emitresolver.go is not ported; used by the alias
  marking functions here).
- `synthetic_expression_type(arg)`: Go `arg.AsSyntheticExpression().Type.(*Type)` = `downcast_ref::<P<Type>>()`.
- `TemplateSpansState` + `add_template_spans`: the recursive `addSpans` closure of getTemplateLiteralType.

## Needs from others

- Whoever creates `SyntheticExpression` nodes (getEffectiveCallArguments etc.) must store the checker type as a
  `P<Type>` value behind `&'static dyn Any` (e.g. `alloc(t)`), so the `downcast_ref::<P<Type>>()` above works.
- `resolve_symbol(symbol: P<Symbol>)`: Go's `resolveSymbol(nil)` returns nil and Go passes a possibly nil
  `getSymbol(...)` result in checkExternalEmitHelpers; adapted at the call site with `.map(...)` (`// SIG:` comment).
  Consider making it `Option<P<Symbol>> -> Option<P<Symbol>>`.

## Doubts

- getRegularTypeOfObjectLiteral: Go `regular.flags = resolved.flags` / `resolved.objectFlags` read the header of
  `resolved` (the StructuredType embedded in t, whose TypeBase embeds t's Type header), ported as `t.flags()` /
  `t.object_flags()`.
- markExportSpecifierAliasReferenced: `ast.IsGlobalSourceFile(ast.GetDeclarationContainer(decl))` - Rust
  `get_declaration_container` returns Option; None is treated as "not global" (Go would dereference nil).
- getTemplateLiteralType: the length limit uses UTF-8 byte lengths of the builder, like Go's `sb.Len()`.
- markLinkedReferences / markDecoratorAliasReferenced unwrap parents where Go dereferences `Parent.Parent` directly.
