# checker-06 notes (checker.go 10857–13068)

## Signature changes
- `is_property_in_class_derived_from(prop, base_class: P<Type>)` -> `base_class: Option<P<Type>>`: Go passes the nil-able result of `getDeclaringClass` (from `isValidOverrideOf`).

## Shared-file edits
(none)

## Needs from others
- `has_base_type(t, check_base: P<Type>)` (checker_09): Go passes nil `checkBase` from `isPropertyInClassDerivedFrom` / `isClassDerivedFromDeclaringClasses` (`getDeclaringClass` can be nil). Should be `Option<P<Type>>`. Worked around locally with `has_base_type_or_nil` (a nil checkBase never matches, so it only recurses).
- `get_export_symbol_of_value_symbol_if_exported(Option<P<Symbol>>) -> P<Symbol>`: Go returns nil for nil input; result should be `Option<P<Symbol>>`. `check_delete_expression` guards the nil case before calling.
- `check_type_assignable_to_and_optionally_elaborate(.., expr: P<Node>, ..)`: Go passes nil `expr` (checkYieldExpression with no operand). Should be `Option<P<Node>>`. Expanded inline at that call site (isTypeRelatedTo + checkTypeRelatedToEx, equivalent since elaborateError(nil) is false).
- `get_this_type_of_object_literal_from_contextual_type(.., contextual_type: P<Type>)`: Go passes a nil-able type; should be `Option<P<Type>>`. Guarded at the call site.
- `get_mapper_from_context(n: P<InferenceContext>)`: Go accepts nil (returns nil). Guarded at the call site.

## Doubts
- `check_synthetic_expression`: `SyntheticExpression.type_` is `&'static dyn Any`; the port downcasts to `Type` or `P<Type>`. Whoever builds synthetic expressions must store one of those.
- `get_instantiation_expression_type`: Go's nested closures were turned into free helper fns with a shared state struct; `checkTypeArguments` returning nil is modelled as an empty Vec (check that its port returns empty exactly when Go returns nil).
- Messages use `type_to_string_exported` for Go `c.TypeToString(t)` (same as `typeToString(t, nil)`).
