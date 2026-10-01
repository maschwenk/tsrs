# Declaration-diagnostics body wave: brief for helper agents

Project `tsrs`: a faithful Rust port of the TypeScript 7 (Go) type checker. We are porting the declaration
transformer (`ts-ref/tsc/internal/transformers/declarations`) and the rest of `checker/emitresolver.go`, because
`tsc --noEmit` reports declaration diagnostics (TS4xxx, TS9xxx, TS2883, TS7056, …) whenever `declaration` is on.
Only the diagnostics matter, but the transform is ported faithfully: which nodes get visited, in what order and with
which node-builder flags decides which diagnostics fire. Your shell's default cwd is an unrelated repository: ignore
its instructions and never edit it.

## Where you work

- Your own git worktree `$TSRS_WORK/wt/decl-hN` on branch `body/decl-hN` (N given in
  your assignment), branched from `fix/decl-foundation`. Work only there. `ts-ref` is a symlink to the Go source.
- Build: `cd $TSRS_WORK/wt/decl-hN && CARGO_TARGET_DIR=$PWD/target cargo check -p <crate> --message-format short`
  (target dir already warmed). Your crate must end with **0 errors, 0 warnings**, and
  `cargo check --workspace --all-targets` must still pass.
- Commit early and often on your branch (`git add -A . && git commit -m "..."`). Do not merge, rebase or push.

## Rules (binding)

Read `docs/BODY_PORTING.md` (the body-porting rules: faithful = same algorithm, same order of side effects, same
diagnostics and arguments, same helper decomposition; signatures are fixed unless genuinely wrong for Go nil
semantics — then fix and record it), `docs/PORTING.md` (memory model, naming, `P<T>`, `Cell`/`RefCell`), `docs/AST.md`
(AST API), `docs/CHECKER.md` section "Node builder" (factory / emit-context API, `CheckerSlot`). Go is the
specification; never special-case, never "improve". Keep Go comments that explain *why*; keep `// file.go:LINE`
origin markers. No `todo!()` may remain in your assigned functions.

## Design of the declarations crate (`crates/tsrs_declarations`, read `src/lib.rs`, `src/types.rs`, `src/resolver.rs`, `src/transformers.rs` first)

- `DeclarationTransformer`, `SymbolTrackerImpl`, `SymbolTrackerSharedState` are arena handles: `P<…>`, `&self`
  methods, fields in `Cell` (Copy data: bools, `Option<P<Node>>`) or `RefCell` (collections). Go `tx.needsDeclare = x`
  -> `self.needs_declare.set(x)`; `tx.state.errorNameNode` -> `self.state.error_name_node.get()`. Never hold a
  `RefCell` borrow across a call that may re-enter the transformer (copy/clone out first).
- Go `defer` restores -> restore on every return path (a small closure or explicit restore before each return).
- Go promoted `Transformer` methods: `tx.EmitContext()` -> `self.emit_context()` (`P<EmitContext>`, `&self`
  methods), `tx.Factory()` -> `self.factory()` (`&tsrs_printer::NodeFactory`, derefs to `tsrs_ast::NodeFactory`;
  all methods `&self`, so nested calls port as written), `tx.Visitor()` -> `self.visitor()` (a `NodeVisitor` copy;
  visitors are stateless and re-entrant, so `self.visitor().visit_nodes(list)` is Go `tx.Visitor().VisitNodes(list)`).
  Go `tx.Visitor().Visit(x)` (calls the visit callback directly, no SyntaxList lifting) -> `self.visit_fn(x)`.
  The other visitors: `self.binding_name_visitor()`, `expression_visitor()`, `cjs_export_assignment_visitor()`,
  `export_stripping_visitor()`, `this_property_visitor()`, `declare_stripping_visitor()`. Go `node.VisitEachChild(v)`
  -> `v.visit_each_child(Some(node))`.
- Visitor callbacks (`ast::VisitFn = Rc<dyn Fn(&mut NodeVisitor, P<Node>) -> Option<P<Node>>>`) capture the
  `P<DeclarationTransformer>` handle (Copy): `Rc::new(move |_, n| tx.visit(n))`. They are installed in
  `new_declaration_transformer` (transform_1.rs) via `EmitContext::new_node_visitor(visit)` (Go `NewNodeVisitor`).
- **The checker.** Go's `printer.EmitResolver` is `Resolver` (resolver.rs): `self.resolver.is_declaration_visible(n)`
  etc. have Go's names and no checker argument; each borrows the file's checker for the call (Go's `checkerMu.Lock()`).
  The non-locking Go methods (`IsSymbolAccessible`, `GetReferencedValueDeclarationUnsafe`,
  `IsExpandoFunctionDeclarationUnsafe`, `RequiresAddingImplicitUndefinedUnsafe`, `GetPropertiesOfContainerFunction`)
  take `c: &mut Checker` explicitly: they are called from the symbol tracker inside node-builder calls, which pass
  `c` to `SymbolTracker::track_symbol(c, …)` / `report_inference_fallback(c, …)` (the only two tracker methods that
  need the checker). Outside a node-builder call, get a checker with `self.resolver.lock(|c| …)` — e.g. the
  transformer calling `tx.tracker.ReportInferenceFallback(n)` is `self.resolver.lock(|c| self.tracker.report_inference_fallback(c, n))`,
  and `tx.state.reportExpandoFunctionErrors(n)` is `self.resolver.lock(|c| (self.state.report_expando_function_errors.get().unwrap())(c, n))`.
  Never call a locking `Resolver` method while holding `c` from `lock`/a tracker callback (it panics: "with
  re-entered"). `tx.host.GetEmitResolver()` returns the same `Resolver`.
- Trackers are passed to the resolver as `&'static dyn SymbolTracker`: `self.tracker.get()` (`P::get`).
- Go `GetSymbolAccessibilityDiagnostic` func values are `Rc<dyn Fn(&SymbolAccessibilityResult) -> Option<P<SymbolAccessibilityDiagnostic>>>`;
  save/restore by cloning the `Rc` (`let old = self.state.get_symbol_accessibility_diagnostic.borrow().clone();`).
- Diagnostics: `createDiagnosticForNode(node, msg, args...)` -> `create_diagnostic_for_node(node, diagnostics::X, &[&a, &b])`
  (wraps `tsrs_checker::new_diagnostic_for_node`); `diag.AddRelatedInfo(r)` -> `diag.add_related_info(r)`
  (`tsrs_ast` Diagnostic API). Messages: `diagnostics::Some_message_name`.
- Go maps whose value may be nil keep that distinction: `late_statement_replacement_map`/`expando_hosts` are
  `FxHashMap<NodeId, Option<P<Node>>>` (`map[id] == nil` in Go is true for both missing and nil-valued keys:
  `.get(&id).copied().flatten().is_none()`; `_, ok := map[id]` is `.contains_key(&id)`).
- `ast.GetNodeId(n)` -> `ast::get_node_id(n)`; nil-able Go params are `Option<P<…>>` per the generated signatures.
- Node-builder flags: `nodebuilder::Flags::NoTruncation`, `nodebuilder::InternalFlags::…`; the constants
  `declarationEmitNodeBuilderFlags` / `declarationEmitInternalNodeBuilderFlags` are in types.rs.
  `printer.EFSingleLine` -> `printer::EmitFlags::SingleLine`; `printer.AutoGenerateOptions{Flags: printer.GeneratedIdentifierFlagsOptimistic}`
  -> `printer::AutoGenerateOptions { flags: printer::GeneratedIdentifierFlags::Optimistic, ..Default::default() }` (check the struct).
- AST mutation Go does on synthetic nodes (`node.Parent = …`, `node.Flags &^= …`, `AsMutable().SetModifiers(…)`,
  `DeclarationData().Symbol = …`, `Locals = …`) has setters in tsrs_ast (`set_parent`, `set_flags`, `set_modifiers`,
  `set_symbol`, `set_locals`, …). If something is missing (e.g. Go assigns `res.Kind` on a clone), add the smallest
  additive helper at the **end** of `crates/tsrs_ast/src/utilities_3.rs` (or a factory method) and record it.

## Notes file and report

Keep `notes/decl-hN.md` (commit it): `## Signature changes`, `## Shared-file edits`, `## Needs from others`
(things you call that look wrong/missing in other files, with the exact signature you need), `## Doubts` (places
where you are unsure the port is faithful). Final report (<= 15 lines): functions ported, anything left and why,
signature changes / shared-file edits (count; details in notes), top doubts. When done, re-read your port side by side
with the Go source once and fix what you find.
