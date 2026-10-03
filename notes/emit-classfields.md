# emit/classfields: port of estransforms/classfields.go (part 1 of 2)

`crates/tsrs_transformers/src/estransforms/classfields.rs` (gate stub) is replaced by:

- `classfields_1.rs`: Go lines 1-1888, **complete**. Covers `classFacts`, `privateIdentifierInfo`,
  `privateEnvironment(Data)`, `classLexicalEnvironment`, `classLexicalEnv`, the transformer struct,
  `new_class_fields_transformer` (with all 9 secondary visitors), and every method from `requiresBlockScopedVar`
  through `visitInNewClassLexicalEnvironment`.
- `classfields_2.rs`: Go lines 1889-3618. Partially ported; the rest are signatures with
  `unimplemented!("classfields part 2")`.

## Part-2 stubs still to fill (in Go order)

| Go line | Rust fn |
| --- | --- |
| 1893 | `visit_class_declaration_in_new_class_lexical_environment(node, facts)` |
| 2007 | `visit_class_expression_in_new_class_lexical_environment(node, facts)` |
| 2365 | `transform_constructor(constructor, container)` |
| 2650 | `transform_property_or_class_static_block(property, receiver)` |

Part 1 calls only these four, plus the functions already ported in part 2. Part 2 still has to add the
functions that only part 2 calls, in Go order: `transformClassMembers` (2223), `createBrandCheckWeakSetForPrivateMethods`,
`transformConstructorBodyWorker`, `transformConstructorBody`, `addPropertyOrClassStaticBlockStatements`,
`generateInitializedPropertyExpressionsOrClassStaticBlock`, `transformProperty`, `transformPropertyWorker`,
`addInstanceMethodStatements`, `addPrivateIdentifier*ToEnvironment` (2940-3101).

Already fully ported in part 2: `visitClassDeclaration`, `visitClassExpression`, `visitClassStaticBlockDeclaration`,
`visitThisExpression`, `visitInvalidSuperProperty`, `getPropertyNameExpressionIfNeeded`, `start/end/getClassLexicalEnvironment`,
`getPrivateIdentifierEnvironment`, `addPendingExpressions`, `set/getPrivateIdentifier`, `createHoistedVariableForClass(FromNode)`,
`createHoistedVariableForPrivateName`, `accessPrivateIdentifier`, `wrapPrivateIdentifierForDestructuringTarget`,
all destructuring visitors (3220-3364), `createPrivate{Static,Instance}FieldInitializer`, `createPrivateInstanceMethodInitializer`,
`isReservedPrivateName`, `isStaticPropertyDeclarationOrClassStaticBlock`, `getProperties`,
`getStaticPropertiesAndClassStaticBlock`, `classHasClassThisAssignment`, `isNonStaticMethodOrAccessorWithPrivateName`,
`createMemberAccessForPropertyName`, `createCallBinding`, `shouldBeCapturedInTempVariable`, the accessor
get/set redirectors, `flattenCommaList(Worker)`, `findComputedPropertyNameCacheAssignment`,
`expandPreOrPostfixIncrementOrDecrementExpression`.

## Decisions

- Go pointer-shared structs are arena handles: `P<classLexicalEnv>` (`data`, `private_env` are `Cell` because Go
  fills them lazily), `P<classLexicalEnvironment>` (all fields `Cell`), `P<privateEnvironment>` (`data` fields `Cell`,
  maps in `RefCell`), `P<privateIdentifierInfo>` (`getter_name`/`setter_name` are `Cell`: Go mutates
  `previousInfo.getterName/setterName`; the other fields are set at construction).
- `privateEnvironment.generated_identifiers` is always allocated (Go allocates lazily; nil map reads equal empty).
  `class_aliases` is a plain `FxHashMap` (Go's nil map behaves as empty; reset to empty where Go sets nil).
- `pending_expressions` / `pending_statements` are `RefCell<Vec>`; Go's save/nil/restore is `mem::take` + restore.
- Visit functions return `Option<P<Node>>` (Go may return nil). `visit_identifier` and
  `visit_property_access_expression_for_substitution` return `P<Node>`.
- Go method-value callbacks (`(*classFieldsTransformer).visitX`) are `fn(&Self, P<Node>, ...) -> Option<P<Node>>`.
  `tx.isAnonymousClassNeedingAssignedName` (pre-bound method value) is a closure built at each call site.
- Go `findComputedPropertyNameCacheAssignment` returns `*ast.BinaryExpression`; Rust returns the `P<Node>`.
- `flattenCommaList` returns a `Vec` (every Go caller ranges over the whole sequence).
- `createCopiableReceiverExpr` / `createCallBinding` return tuples in Go result order.
- `memberContainsConstructorReference`'s recursive closure is a nested `fn check(tx, ...)`.
- The Go `visit` defer (`popNode`) is an explicit `visit` -> `visit_worker` split.

## Shared-file edits

- `crates/tsrs_transformers/Cargo.toml`: added `bitflags.workspace = true` (for `classFacts`).
- `crates/tsrs_transformers/src/estransforms/mod.rs`: `classfields` -> `classfields_1` + `classfields_2`.
- No changes to tsrs_ast / tsrs_printer.
