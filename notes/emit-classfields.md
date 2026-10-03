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

## Test results

Scratch worktree `/root/wt-cf-int`, branch `scratch/cf-int` = `int/local` + `emit/classfields` (conflict in
`tsrs_transformers/src/utilities.rs`: kept emit/classfields' `find_super_statement_index_path` /
`get_super_call_from_statement` / `pub is_super_call`, dropped `todo_core.rs::is_super_call`).
`./target/release/tsrs-test run --baselines js --list ...` (pass / fail / crash / skip):

| list | int/local + classfields | + scratch/small-int + scratch/async-int |
| --- | --- | --- |
| class tests (`/tmp/cf-list.txt`, 325: classStaticBlock/privateName/propertyMember/autoAccessor/classField/defineProperty/useDefine) | 208 / 0 / 88 / 29 | 267 / 0 / 29 / 29 |
| `/root/list-dec.txt` (213) | 81 / 0 / 63 / 69 | 101 / 0 / 43 / 69 |
| `/root/list-jsx.txt` (230) | 93 / 0 / 134 / 3 | 111 / 0 / 116 / 3 |

No js `fail` and no crash in classfields/namedevaluation. Remaining crashes are other waves' stubs: forawait (int/local
only), esdecorator, commonjsmodule, using, esmodule `visit_source_file`, optionalchain/objectrestspread (int/local only),
emitter `print_source_file` (es2022/esnext `classStaticBlock25`, `privateNameEmitHelpers`-style).
The second column is branch `scratch/cf-int-all` (scratch/cf-int + `scratch/small-int` + `scratch/async-int`, to get
forawait/optionalchain/objectrestspread so es2015-target class tests run; conflict in
`estransforms/utilities.rs` resolved by combining the header functions with async's `superAccessState`).

Monorepo oracle (`tools/oracle/emit/monorepo.sh`, 7 es2024 decorator packages, `--sourceMap false
--declarationMap false`, scratch/cf-int-all build): 7/7 packages fully identical, 146 files identical, 0 different,
0 not emitted, 0 extra; monorepo git status unchanged.

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
