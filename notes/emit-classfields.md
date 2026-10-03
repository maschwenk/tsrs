# emit/classfields: port of estransforms/classfields.go (complete)

`crates/tsrs_transformers/src/estransforms/classfields.rs` (gate stub) is replaced by:

- `classfields_1.rs`: Go lines 1-1888, **complete**. Covers `classFacts`, `privateIdentifierInfo`,
  `privateEnvironment(Data)`, `classLexicalEnvironment`, `classLexicalEnv`, the transformer struct,
  `new_class_fields_transformer` (with all 9 secondary visitors), and every method from `requiresBlockScopedVar`
  through `visitInNewClassLexicalEnvironment`.
- `classfields_2.rs`: Go lines 1889-3618, **complete** (every Go `func` in that range has a `// classfields.go:LINE`
  marker; no `unimplemented!` left).

## Part 2 (done)

Ported: `visitClassDeclarationInNewClassLexicalEnvironment`, `visitClassExpressionInNewClassLexicalEnvironment`,
`transformClassMembers`, `createBrandCheckWeakSetForPrivateMethods`, `transformConstructor`,
`transformConstructorBodyWorker`, `transformConstructorBody`, `addPropertyOrClassStaticBlockStatements`,
`transformPropertyOrClassStaticBlock`, `generateInitializedPropertyExpressionsOrClassStaticBlock`, `transformProperty`,
`transformPropertyWorker`, `addInstanceMethodStatements`, `addPrivateIdentifier{PropertyDeclaration,Method,GetAccessor,
SetAccessor,AutoAccessor}ToEnvironment`, `addPrivateIdentifierToEnvironment`.

Part-2 decisions:
- `transform_constructor` takes `Option<P<Node>>` (Go passes nil for the synthetic constructor); part 1's call site
  passes `Some(node)`.
- Go `&privateIdentifierInfo{kind: Untransformed}` is `new_private_identifier_info(kind)` (other fields zero).
- `tx.pendingStatements = tx.addPropertyOrClassStaticBlockStatements(tx.pendingStatements, ...)` is `mem::take` +
  store back (RefCell).
- `statementsIn[a:b]` slices are `&'static [P<Node>]` sub-slices passed to `visit_slice`.

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
- No changes to tsrs_ast / tsrs_printer (part 2 included).
- `crates/tsrs_ast/src/utilities_1.rs`: AST class helpers the transformer needs (Go `ast/utilities.go`).
- `crates/tsrs_transformers/src/utilities.rs`: `is_super_call` made `pub` (used by classfields); the rest of
  `transformers/utilities.go` already comes from main.

## Review layer

Clean review branch `mfs-cx/emit-classfields`, one layer on top of the clean JSX branch (`mfs-cx/emit-jsx-decorators`
07be672). The code is the helper branch `emit/classfields` d949ed9 applied as one patch (algorithmic files byte-identical
to the helper); only `transformers/utilities.rs` was resolved against main's version (main already ports the rest of
transformers/utilities.go). Results for this layer: see the layer's commit message. On this base the TypeScript
transformers are still gate stubs (typeeraser), so most class-field tests still stop there; the combined local
integration build (all helper layers) is the deeper check.
