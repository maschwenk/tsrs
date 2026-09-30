# nb-5 notes

Ported: `crates/tsrs_pseudochecker/src/lookup.rs` (lookup.go, 37 functions; checker.go/type.go were already complete),
`crates/tsrs_checker/src/pseudotypenodebuilder.rs` (10), `crates/tsrs_checker/src/nodebuilder_hover.rs` (19, all of
nodebuilder_hover.go — every function only needs checker/node-builder infrastructure that has generated signatures).

## Signature changes

- none.

## Shared-file edits

- `crates/tsrs_ast/src/utilities_3.rs`: added `is_variable_parameter_or_property` and `is_primitive_literal_value`
  (Go ast/utilities.go:3097 and :4119), placed right before `has_inferred_type`. Needed by lookup.go. If another agent
  (nodebuilderimpl / nodecopy) adds the same functions, keep one copy at merge.

## Needs from others

- Callees used by generated signature only (bodies are other agents'): `reuse_type_node`, `reuse_node`, `reuse_name`
  (nodecopy), `type_to_type_node`, `can_reuse_existing_js_type_node`, `serialize_return_type_for_signature`,
  `serialize_type_for_declaration`, `enter_new_scope`, `save_restore_flags`, `set_comment_range`,
  `parameter_to_parameter_declaration_name`, `add_property_to_element_list`, `check_truncation_length_if_expanding`,
  `type_parameter_to_declaration`, `signature_to_signature_declaration_helper`,
  `index_info_to_index_signature_declaration_helper`, `symbol_to_node` (nodebuilderimpl).
- `reuse_node`, `reuse_name`, `parameter_to_parameter_declaration_name` return `Option`; Go's call sites here put the
  result straight into a node list / non-nil factory field, so I `unwrap()` (Go would build a list holding nil or a
  node with a nil name; neither happens for these inputs).

## Doubts

- `pseudo_type_to_node` (MaybeConstLocation): `instantiate_contextual_type` is only called when `pseudo_type_to_type`
  returned non-nil (Go's `&&` short-circuit), after `get_contextual_type` — same order as Go.
- `expand_module_decl`: Go's `nodebuilder.Flags(SymbolFormatFlagsUseOnlyExternalAliasing)` reinterprets the
  SymbolFormatFlags bit (1<<1) as a node-builder flag; ported literally with `Flags::from_bits_retain(..bits())`.
  The deferred `b.ctx.flags = oldFlags` is an explicit restore just before returning.
- `expand_class_decl`: deferred `enclosingDeclaration` restore is done after building the ClassDeclaration node (same
  point as Go's defer). Empty type-parameter / heritage lists are passed as `Some(empty list)` like Go's
  `NewNodeList(nil)` (non-nil list).
- `is_in_const_context` unwraps `find_ancestor` (Go's `IsConstAssertion(nil)` would panic too; only reachable for a
  node with no parent).
- lookup.go `canGetTypeFromObjectLiteral`/`canGetTypeFromArrayLiteral` return nil-or-nonempty in Go, so an empty
  `Vec` stands for nil.
- `add_class_modifiers`, `type_elements_to_class_elements` take `&mut [P<Node>]` (generated): they modify in place and
  return a copy, matching Go's in-place slice mutation + return.
