# lsp-api (helper: tsrs_checker language-service API)

Branch `lsp-api-checker`. Scope: every function of `checker/services.go` and `checker/exports.go`, every checker API
that `internal/{ls,lsp,project,fourslash}` call, and the language-service-only behavior the batch port dropped.

Naming: an exported Go method whose snake_case name collides with an existing (private-origin) Rust method gets the
`_exported` suffix (`GetTypeOfSymbol` -> `get_type_of_symbol_exported`). The same rule is applied to exported
package-level functions that collide with a private one (`IsTupleType` -> `is_tuple_type_exported`). Look names up
by origin marker in `docs/sigs/checker.txt`. Many private targets are already `pub` (the batch made them pub for
the baseline writer), so LS code may call either; the `_exported` wrapper is the Go-faithful entry point.

Inventory (`target/scratch/checker/inv.py` over `ls-used-methods.txt`): 67 MISSING before, 0 after. Every exported
Go function and method of the checker package now has a Rust counterpart with an origin marker; exported
package-level functions are `pub use`d from the crate root (lib.rs).

## services.go and exports.go

| Go | Rust | file | status |
| --- | --- | --- | --- |
| `GetSymbolsInScope` (services.go:17) | `get_symbols_in_scope_exported` | services.rs | ported |
| `getSymbolsInScope` (services.go:21) | `get_symbols_in_scope` | services.rs | ported |
| `GetExportsOfModule` (services.go:132) | `get_exports_of_module_exported` | services.rs | ported |
| `ForEachExportAndPropertyOfModule` (services.go:136) | `for_each_export_and_property_of_module` | services.rs | ported |
| `IsValidPropertyAccess` (services.go:165) | `is_valid_property_access_exported` | services.rs | ported |
| `isValidPropertyAccess` (services.go:169) | `is_valid_property_access` | services.rs | ported |
| `isValidPropertyAccessWithType` (services.go:181) | `is_valid_property_access_with_type` | services.rs | ported |
| `IsValidPropertyAccessForCompletions` (services.go:199) | `is_valid_property_access_for_completions_exported` | services.rs | ported |
| `GetAllPossiblePropertiesOfTypes` (services.go:210) | `get_all_possible_properties_of_types` | services.rs | ported |
| `IsUnknownSymbol` (services.go:232) | `is_unknown_symbol` | services.rs | ported |
| `IsUndefinedSymbol` (services.go:236) | `is_undefined_symbol` | services.rs | ported |
| `IsArgumentsSymbol` (services.go:240) | `is_arguments_symbol` | services.rs | ported |
| `GetNonOptionalType` (services.go:245) | `get_non_optional_type` | services.rs | ported |
| `GetStringIndexType` (services.go:249) | `get_string_index_type` | services.rs | ported |
| `GetNumberIndexType` (services.go:253) | `get_number_index_type` | services.rs | ported |
| `GetElementTypeOfArrayType` (services.go:257) | `get_element_type_of_array_type_exported` | services.rs | ported |
| `GetCallSignatures` (services.go:261) | `get_call_signatures` | services.rs | ported |
| `GetConstructSignatures` (services.go:265) | `get_construct_signatures` | services.rs | ported |
| `GetApparentProperties` (services.go:269) | `get_apparent_properties` | services.rs | ported |
| `getAugmentedPropertiesOfType` (services.go:273) | `get_augmented_properties_of_type` | services.rs | ported |
| `TryGetMemberInModuleExportsAndProperties` (services.go:296) | `try_get_member_in_module_exports_and_properties` | services.rs | ported |
| `TryGetMemberInModuleExports` (services.go:314) | `try_get_member_in_module_exports` | services.rs | ported |
| `shouldTreatPropertiesOfExternalModuleAsExports` (services.go:319) | `should_treat_properties_of_external_module_as_exports` | services.rs | ported |
| `GetContextualType` (services.go:327) | `get_contextual_type_exported` | services.rs | ported |
| `runWithInferenceBlockedFromSourceNode` (services.go:334) | `run_with_inference_blocked_from_source_node` | services.rs | ported |
| `GetResolvedSignatureForSignatureHelp` (services.go:355) | `get_resolved_signature_for_signature_help` | services.rs | ported |
| `runWithoutResolvedSignatureCaching` (services.go:367) | `run_without_resolved_signature_caching` | services.rs | ported |
| `SkipAlias` (services.go:396) | `skip_alias` | services.rs | already existed |
| `GetRootSymbols` (services.go:403) | `get_root_symbols` | services.rs | already existed |
| `GetMappedTypeSymbolOfProperty` (services.go:415) | `get_mapped_type_symbol_of_property` | services.rs | already existed |
| `getImmediateRootSymbols` (services.go:422) | `get_immediate_root_symbols` | services.rs | already existed |
| `tryGetTarget` (services.go:453) | `try_get_target` | services.rs | already existed |
| `GetExportSymbolOfSymbol` (services.go:472) | `get_export_symbol_of_symbol` | services.rs | already existed |
| `GetExportSpecifierLocalTargetSymbol` (services.go:476) | `get_export_specifier_local_target_symbol` | services.rs | already existed |
| `GetShorthandAssignmentValueSymbol` (services.go:495) | `get_shorthand_assignment_value_symbol` | services.rs | already existed |
| `GetSymbolsOfParameterPropertyDeclaration` (services.go:508) | `get_symbols_of_parameter_property_declaration` | services.rs | ported |
| `IsDeclarationUsed` (services.go:524) | `is_declaration_used` | services.rs | ported |
| `IsSymbolReferencedInFile` (services.go:552) | `is_symbol_referenced_in_file` | services.rs | ported |
| `GetReferencesToSymbolInFile` (services.go:587) | `get_references_to_symbol_in_file` | services.rs | ported |
| `getLocalSymbolForExportSpecifier` (services.go:624) | `get_local_symbol_for_export_specifier` | services.rs | ported |
| `isExportSpecifierAlias` (services.go:633) | `is_export_specifier_alias` | services.rs | ported |
| `getPossibleSymbolReferenceNodes` (services.go:646) | `get_possible_symbol_reference_nodes` | services.rs | ported |
| `getPossibleSymbolReferencePositions` (services.go:655) | `get_possible_symbol_reference_positions` | services.rs | ported |
| `GetTypeArgumentConstraint` (services.go:700) | `get_type_argument_constraint_exported` | services.rs | ported |
| `getUninstantiatedSignatures` (services.go:708) | `get_uninstantiated_signatures` | services.rs | ported |
| `getTypeParameterConstraintForPositionAcrossSignatures` (services.go:727) | `get_type_parameter_constraint_for_position_across_signatures` | services.rs | ported |
| `getTypeArgumentConstraint` (services.go:742) | `get_type_argument_constraint` | services.rs | ported |
| `IsTypeInvalidDueToUnionDiscriminant` (services.go:816) | `is_type_invalid_due_to_union_discriminant` | services.rs | ported |
| `GetExportsAndPropertiesOfModule` (services.go:841) | `get_exports_and_properties_of_module` | services.rs | ported |
| `getExportsOfModuleAsArray` (services.go:853) | `get_exports_of_module_as_array` | services.rs | ported |
| `GetJsxIntrinsicTagNamesAt` (services.go:858) | `get_jsx_intrinsic_tag_names_at` | services.rs | ported |
| `GetContextualTypeForJsxAttribute` (services.go:866) | `get_contextual_type_for_jsx_attribute_exported` | services.rs | ported |
| `GetConstantValue` (services.go:870) | `get_constant_value` | services.rs | already existed |
| `getResolvedSignatureWorker` (services.go:899) | `get_resolved_signature_worker` | services.rs | ported |
| `GetCandidateSignaturesForStringLiteralCompletions` (services.go:911) | `get_candidate_signatures_for_string_literal_completions` | services.rs | ported |
| `GetTypeAtPosition` (services.go:936) | `get_type_at_position_exported` | services.rs | ported |
| `GetTypeParameterAtPosition` (services.go:940) | `get_type_parameter_at_position` | services.rs | ported |
| `GetContextualTypeForArrayLiteralAtPosition` (services.go:953) | `get_contextual_type_for_array_literal_at_position` | services.rs | ported |
| `knownGenericTypeNames` (services.go:981) | `knownGenericTypeNames (static)` | services.rs | ported |
| `isKnownGenericTypeName` (services.go:1004) | `is_known_generic_type_name` | services.rs | ported |
| `GetFirstTypeArgumentFromKnownType` (services.go:1009) | `get_first_type_argument_from_known_type` | services.rs | ported |
| `GetPropertySymbolsFromContextualType` (services.go:1026) | `get_property_symbols_from_contextual_type` | services.rs | ported |
| `GetPropertySymbolOfDestructuringAssignment` (services.go:1071) | `get_property_symbol_of_destructuring_assignment` | services.rs | ported |
| `getTypeOfAssignmentPattern` (services.go:1090) | `get_type_of_assignment_pattern` | services.rs | ported |
| `GetSignatureFromDeclaration` (services.go:1120) | `get_signature_from_declaration_exported` | services.rs | ported |
| `IsLibSymbolForHoverVerbosity` (services.go:1125) | `is_lib_symbol_for_hover_verbosity` | services.rs | already existed |
| `IsLibTypeForHoverVerbosity` (services.go:1140) | `is_lib_type_for_hover_verbosity` | services.rs | already existed |
| `GetStringType` (exports.go:8) | `get_string_type` | exports.rs | already existed |
| `GetNumberType` (exports.go:12) | `get_number_type` | exports.rs | already existed |
| `GetBooleanType` (exports.go:16) | `get_boolean_type` | exports.rs | already existed |
| `GetVoidType` (exports.go:20) | `get_void_type` | exports.rs | already existed |
| `GetUndefinedType` (exports.go:24) | `get_undefined_type` | exports.rs | already existed |
| `GetNullType` (exports.go:28) | `get_null_type` | exports.rs | already existed |
| `GetAnyType` (exports.go:32) | `get_any_type` | exports.rs | already existed |
| `GetErrorType` (exports.go:36) | `get_error_type` | exports.rs | already existed |
| `GetNeverType` (exports.go:40) | `get_never_type` | exports.rs | already existed |
| `GetUnknownType` (exports.go:44) | `get_unknown_type` | exports.rs | already existed |
| `GetBigIntType` (exports.go:48) | `get_big_int_type` | exports.rs | already existed |
| `GetESSymbolType` (exports.go:52) | `get_es_symbol_type` | exports.rs | already existed |
| `GetNonPrimitiveType` (exports.go:56) | `get_non_primitive_type` | exports.rs | already existed |
| `GetBaseTypeOfLiteralType` (exports.go:60) | `get_base_type_of_literal_type_exported` | exports.rs | ported |
| `GetUnknownSymbol` (exports.go:64) | `get_unknown_symbol` | exports.rs | already existed |
| `GetUndefinedSymbol` (exports.go:68) | `get_undefined_symbol` | exports.rs | already existed |
| `GetArgumentsSymbol` (exports.go:72) | `get_arguments_symbol` | exports.rs | already existed |
| `GetUnknownSignature` (exports.go:76) | `get_unknown_signature` | exports.rs | already existed |
| `GetUnionType` (exports.go:80) | `get_union_type_exported` | exports.rs | ported |
| `GetNameTypeOfSymbol` (exports.go:84) | `get_name_type_of_symbol` | exports.rs | already existed |
| `IsTypeUsableAsPropertyName` (exports.go:91) | `is_type_usable_as_property_name_exported` | exports.rs | ported |
| `GetPropertyNameFromType` (exports.go:95) | `get_property_name_from_type_exported` | exports.rs | ported |
| `GetGlobalSymbol` (exports.go:99) | `get_global_symbol_exported` | exports.rs | existed; `diagnostic` now `Option` (LS passes nil) |
| `GetMergedSymbol` (exports.go:103) | `get_merged_symbol_exported` | exports.rs | ported |
| `TryFindAmbientModule` (exports.go:107) | `try_find_ambient_module_exported` | exports.rs | already existed |
| `GetImmediateAliasedSymbol` (exports.go:111) | `get_immediate_aliased_symbol_exported` | exports.rs | ported |
| `GetTargetSymbol` (exports.go:115) | `get_target_symbol_exported` | exports.rs | already existed |
| `GetTypeOnlyAliasDeclaration` (exports.go:119) | `get_type_only_alias_declaration_exported` | exports.rs | ported |
| `ResolveExternalModuleName` (exports.go:123) | `resolve_external_module_name_exported` | exports.rs | existed; `import_attributes_type` now `Option` (Go nil-able) |
| `ResolveExternalModuleSymbol` (exports.go:127) | `resolve_external_module_symbol_exported` | exports.rs | already existed |
| `GetTypeFromTypeNode` (exports.go:131) | `get_type_from_type_node_exported` | exports.rs | ported |
| `IsArrayLikeType` (exports.go:135) | `is_array_like_type_exported` | exports.rs | ported |
| `GetPropertiesOfType` (exports.go:139) | `get_properties_of_type_exported` | exports.rs | ported |
| `GetPropertyOfType` (exports.go:143) | `get_property_of_type_exported` | exports.rs | ported |
| `TypeHasCallOrConstructSignatures` (exports.go:147) | `type_has_call_or_construct_signatures_exported` | exports.rs | ported |
| `IsPropertyAccessible` (exports.go:159) | `is_property_accessible_exported` | exports.rs | ported |
| `GetTypeOfPropertyOfContextualType` (exports.go:163) | `get_type_of_property_of_contextual_type_exported` | exports.rs | ported |
| `GetDeclarationModifierFlagsFromSymbol` (exports.go:167) | `get_declaration_modifier_flags_from_symbol_exported` | exports.rs | ported |
| `WasCanceled` (exports.go:171) | `was_canceled` | exports.rs | already existed |
| `GetSignaturesOfType` (exports.go:175) | `get_signatures_of_type_exported` | exports.rs | ported |
| `GetDeclaredTypeOfSymbol` (exports.go:179) | `get_declared_type_of_symbol_exported` | exports.rs | ported |
| `GetTypeOfSymbol` (exports.go:183) | `get_type_of_symbol_exported` | exports.rs | ported |
| `GetNonMissingTypeOfSymbol` (exports.go:187) | `get_non_missing_type_of_symbol_exported` | exports.rs | ported |
| `GetConstraintOfTypeParameter` (exports.go:191) | `get_constraint_of_type_parameter_exported` | exports.rs | ported |
| `GetTrueTypeOfConditionalType` (exports.go:195) | `get_true_type_of_conditional_type` | exports.rs | already existed |
| `GetFalseTypeOfConditionalType` (exports.go:199) | `get_false_type_of_conditional_type` | exports.rs | already existed |
| `GetDefaultFromTypeParameter` (exports.go:203) | `get_default_from_type_parameter_exported` | exports.rs | already existed |
| `GetEffectiveDeclarationFlags` (exports.go:207) | `get_effective_declaration_flags_exported` | exports.rs | ported |
| `GetBaseConstraintOfType` (exports.go:211) | `get_base_constraint_of_type_exported` | exports.rs | ported |
| `GetTypePredicateOfSignature` (exports.go:215) | `get_type_predicate_of_signature_exported` | exports.rs | ported |
| `IsTupleType` (exports.go:219) | `is_tuple_type_exported` | exports.rs | ported |
| `IsTupleTypeTarget` (exports.go:223) | `is_tuple_type_target` | exports.rs | existed; now `pub use` from the crate root |
| `IsArrayType` (exports.go:227) | `is_array_type_exported` | exports.rs | ported |
| `IsReadonlySymbol` (exports.go:231) | `is_readonly_symbol_exported` | exports.rs | ported |
| `GetReturnTypeOfSignature` (exports.go:235) | `get_return_type_of_signature_exported` | exports.rs | ported |
| `HasEffectiveRestParameter` (exports.go:239) | `has_effective_rest_parameter_exported` | exports.rs | ported |
| `GetLocalTypeParametersOfClassOrInterfaceOrTypeAlias` (exports.go:243) | `get_local_type_parameters_of_class_or_interface_or_type_alias_exported` | exports.rs | ported |
| `GetContextualTypeForObjectLiteralElement` (exports.go:247) | `get_contextual_type_for_object_literal_element_exported` | exports.rs | ported |
| `TypePredicateToString` (exports.go:251) | `type_predicate_to_string_exported` | exports.rs | already existed |
| `GetExpandedParameters` (exports.go:255) | `get_expanded_parameters_exported` | exports.rs | existed as `unimplemented!`; now calls `get_expanded_parameters` |
| `GetResolvedSignature` (exports.go:259) | `get_resolved_signature_exported` | exports.rs | already existed |
| `GetTypeOfPropertyOfType` (exports.go:264) | `get_type_of_property_of_type_exported` | exports.rs | ported |
| `GetContextualTypeForArgumentAtIndex` (exports.go:268) | `get_contextual_type_for_argument_at_index_exported` | exports.rs | already existed |
| `GetAwaitedType` (exports.go:272) | `get_awaited_type_exported` | exports.rs | ported |
| `GetIndexSignaturesAtLocation` (exports.go:276) | `get_index_signatures_at_location_exported` | exports.rs | ported |
| `GetResolvedSymbol` (exports.go:280) | `get_resolved_symbol_exported` | exports.rs | ported |
| `GetJsxNamespace` (exports.go:284) | `get_jsx_namespace_exported` | exports.rs | already existed |
| `GetJsxFragmentFactory` (exports.go:288) | `get_jsx_fragment_factory` | exports.rs | already existed |
| `ResolveName` (exports.go:296) | `resolve_name_exported` | exports.rs | already existed |
| `GetSymbolFlags` (exports.go:300) | `get_symbol_flags_exported` | exports.rs | ported |
| `GetBaseTypes` (exports.go:304) | `get_base_types_exported` | exports.rs | ported |
| `GetApparentType` (exports.go:308) | `get_apparent_type_exported` | exports.rs | ported |
| `GetReducedType` (exports.go:312) | `get_reduced_type_exported` | exports.rs | ported |
| `GetFullyQualifiedName` (exports.go:318) | `get_fully_qualified_name_exported` | exports.rs | already existed |
| `GetBaseConstructorTypeOfClass` (exports.go:322) | `get_base_constructor_type_of_class_exported` | exports.rs | ported |
| `GetMemberOverrideModifierStatus` (exports.go:326) | `get_member_override_modifier_status_exported` | exports.rs | existed; `member_symbol` now `Option` |
| `GetRestTypeOfSignature` (exports.go:330) | `get_rest_type_of_signature_exported` | exports.rs | already existed |
| `GetTypeArguments` (exports.go:334) | `get_type_arguments_exported` | exports.rs | ported |
| `GetIndexInfoOfType` (exports.go:338) | `get_index_info_of_type_exported` | exports.rs | ported |
| `GetIndexTypeOfType` (exports.go:342) | `get_index_type_of_type_exported` | exports.rs | ported |
| `GetIndexInfosOfType` (exports.go:346) | `get_index_infos_of_type_exported` | exports.rs | ported |
| `IsContextSensitive` (exports.go:350) | `is_context_sensitive_exported` | exports.rs | ported |
| `FillMissingTypeArguments` (exports.go:354) | `fill_missing_type_arguments_exported` | exports.rs | ported |
| `GetMinTypeArgumentCount` (exports.go:358) | `get_min_type_argument_count_exported` | exports.rs | ported |
| `GetWidenedLiteralType` (exports.go:362) | `get_widened_literal_type_exported` | exports.rs | ported |
| `IsTypeAssignableTo` (exports.go:366) | `is_type_assignable_to_exported` | exports.rs | ported |
| `GetUnionTypeEx` (exports.go:370) | `get_union_type_ex_exported` | exports.rs | already existed |
| `RequiresAddingImplicitUndefined` (exports.go:374) | `requires_adding_implicit_undefined` | exports.rs | existed; now calls `EmitResolver::requires_adding_implicit_undefined_exported` instead of an inlined copy |
| `RemoveMissingOrUndefinedType` (exports.go:386) | `remove_missing_or_undefined_type_exported` | exports.rs | ported |
| `GetWidenedType` (exports.go:390) | `get_widened_type_exported` | exports.rs | ported |
| `CompareSymbols` (exports.go:394) | `compare_symbols_exported` | exports.rs | existed; params now `Option` (Go nil-able, matches `compare_symbols`) |
| `IsDistributedTypeParameter` (exports.go:398) | `is_distributed_type_parameter` | exports.rs | existed; now `pub use` from the crate root |


## Other exported APIs the language service calls

| Go | Rust | file | status |
| --- | --- | --- | --- |
| `Checker.TypeToStringEx` (printer.go:55) | `type_to_string_ex_exported` | printer.rs | ported (the private `type_to_string_ex` was already `pub`) |
| `Checker.GetAccessibleSymbolChain` (symbolaccessibility.go:382) | `get_accessible_symbol_chain_exported` | printer.rs | ported |
| `EmitResolver.IsSymbolAccessible` (emitresolver.go:685) | `EmitResolver::is_symbol_accessible_exported` | emitresolver.rs | ported |
| `NewChecker` (checker.go:911) | `new_checker` | checker.rs | already existed; origin marker added |
| `NewNodeBuilderEx`, `TryGetModuleSpecifierFromDeclaration`, `NewSymbolTrackerImpl`, `IsInTypeQuery`, `IsKnownSymbol`, `IsPrivateIdentifierSymbol`, `SkipAlias`, `IsExternalModuleSymbol`, `ValueToString`, `GetSetAccessorValueParameter`, `CompareTypes`, `NewDiagnosticChainForNode`, `GetSingleVariableOfVariableStatement`, `CreateModuleNotFoundChain`, `CreateModeMismatchDetails`, exports.go/services.go package functions | same snake_case names | lib.rs | made pub (crate-root `pub use`; the modules are private) |
| `NodeBuilder.*`, `EmitResolver.*`, `Type`/`Signature`/`TypePredicate`/`IndexInfo` accessors | (unchanged) | | already `pub` |

## Restored language-service-only behavior

- `runWithInferenceBlockedFromSourceNode`, `runWithoutResolvedSignatureCaching`, `getResolvedSignatureWorker`
  (services.go) ported; they drive the existing `skip_direct_inference_nodes`, `is_inference_partially_blocked`,
  `apparent_argument_count` and `candidates_out_array` / `CheckMode::IsForSignatureHelp` paths, which were already
  present in the call-resolution and inference code.
- Node builder `idToSymbol` map: Go shares the caller's map (inlay hints, missing-member and missing-type-annotation
  code actions, auto-import read it back after `TypeToTypeNode`). The batch port cloned it. `new_node_builder_ex`,
  `Checker::type_to_type_node[_ex]`, `type_predicate_to_type_predicate_node` and `NodeBuilderImpl.id_to_symbol` now
  take/hold `P<RefCell<FxHashMap<P<Node>, P<Symbol>>>>` (None -> a fresh map, as Go's `make`).
- Cancellation: Go stores the `checkSourceFile` context in `c.ctx` and `isCanceled` checks `c.ctx.Err()`. Restored
  (`Checker.ctx: Option<Context>`, set in `check_source_file`; `is_canceled` reads it). The batch path passes a
  background context, so it never cancels.
- `GetExpandedParameters` (signature help) was `unimplemented!`; it now calls the ported `get_expanded_parameters`.

No other LS-only branch was found missing: `ContextFlags::IgnoreNodeInferences` (jsx.go:230), `IsForSignatureHelp`
trailing-comma handling, `isFromInferenceBlockedSource`, hover expansion (`maxExpansionDepth`,
`canIncreaseExpansionDepth`) and `candidatesOutArray` aliasing in `resolveCall` were already ported. Checked by
counting every LS flag's uses in Go vs Rust and by a parameter-count comparison of all Go vs Rust checker functions
(no dropped parameters).

## Shared-file edits

- `crates/tsrs_ast/src/utilities_3.rs`: cherry-picked lsp-lsfound's `4d8e2ef` (token cache `GetOrCreateToken` + LS
  utilities in tsrs_ast; identical patch, so landing both branches merges cleanly), then appended
  `is_call_like_or_function_like_expression` (ast utilities.go:3017) and `has_type_arguments` (utilities.go:3063).
- `crates/tsrs_checker/src/astnav.rs` (new, `pub mod astnav`): verbatim copy of lsp-lsfound's
  `tsrs_ls::astnav::tokens` (d0f3d71). Go's checker imports `astnav` (services.go `getPossibleSymbolReferenceNodes`
  -> `astnav.GetTouchingPropertyName`), and tsrs_checker cannot depend on tsrs_ls. One copy should go (see Doubts).
- `crates/tsrs_checker/src/checker.rs`: `ctx` field + init, origin marker on `new_checker`. `checker_02.rs`:
  `check_source_file` stores the context (the line the lead touched in 24e76d2). Not touched: tsrs_compiler,
  tsrs_core/src/context.rs, printer_types.rs.

## Deviations

- `getResolvedSignatureWorker`: Go calls `printer.NewEmitContext().ParseNode(node)`. A fresh EmitContext has no
  original-node links, so this is `is_parse_tree_node(node) ? node : nil`; written that way to avoid allocating an
  emit context (leak arena) per signature-help request.
- `runWithoutResolvedSignatureCaching` keeps the saved links in `Vec`s instead of Go maps keyed by link pointer
  (ancestors are distinct nodes, so the keys never repeat; restore order is unobservable).
- `getSymbolsInScope` / `GetAllPossiblePropertiesOfTypes` collect into a `SymbolTable` (insertion order) where Go
  uses a map (random order); `symbolsToArray` / `maps.Values` order is unspecified in Go.
- `getPossibleSymbolReferencePositions` searches bytes (Go `strings.Index` on byte offsets that need not be char
  boundaries); Go's quirk of using the index relative to `container.Pos()` as an absolute position is kept.
- `GetJsxIntrinsicTagNamesAt`: Go checks `getJsxType(...) == nil`; the Rust `get_jsx_type` returns `P<Type>`
  (errorType, never nil), so the check is gone.
- `NewChecker` still returns only the checker (no tracer argument, no `*sync.Mutex`), as before.
- Callbacks (`ForEachExportAndPropertyOfModule`) receive `&mut Checker` first (PORTING.md callback rule).

## Doubts

- astnav home: Go's dependency is checker -> astnav, but docs/LSP.md puts astnav in tsrs_ls. Suggest moving
  `tokens.rs` below tsrs_checker (e.g. keep `tsrs_checker::astnav` and make `tsrs_ls::astnav` re-export it, or a
  small `tsrs_astnav` crate) and deleting the other copy.
- `_exported` wrappers duplicate many already-`pub` private methods (`get_type_of_symbol` vs
  `get_type_of_symbol_exported`). Both are callable; LS ports should prefer the `_exported` (Go-exported) one.
- Exported wrappers keep Go's return shapes as the private Rust functions have them (`&'static [..]` for stored
  slices, `Cow` for `GetPropertyNameFromType`, `i32` for `GetMinTypeArgumentCount`).
