# decl-diagnostics: declaration diagnostics (TS4xxx / TS9xxx / TS2883 / TS7056 / TS2527 / TS5088 …)

## How it was done

- Foundation (this agent): crate `tsrs_declarations` with the data model hand-ported in `types.rs` and stubs from
  `tools/gosig/declarations.json` (transform.go split into transform_{1,2,3}.rs at 1339/2327; `diagnostics.go` ->
  `diagnostics_.rs` because `diagnostics` is the messages alias); `emitresolver_subset.rs` -> `emitresolver.rs` with all
  of emitresolver.go generated (checker.json no longer skips `EmitResolver.*`; the 14 already-ported bodies were kept);
  `tsrs_binder::ReferenceResolver` (referenceresolver.go, hooks are `fn(&mut H, …)` like `NameResolver`).
- Bodies: six helpers in parallel (notes/decl-h1..h6.md): transform_1/2/3, diagnostics+tracker, emitresolver,
  printer EmitContext environments + visitor hooks. Brief: notes/decl-helper-brief.md.
- Glue: `Program::get_declaration_diagnostics` (per checker group on the checker threads, per-file cache like Go's
  `declarationDiagnosticCache`), `emithost.rs` (Go emitHost.go), `emitter::get_declaration_diagnostics`. Harness and
  CLI already called `get_declaration_diagnostics` at Go's points.

## Design decisions worth knowing

- The checker reaches the transformer through `Resolver` (tsrs_declarations/src/resolver.rs): the caller lends the
  file's checker to a `CheckerSlot` for the whole transform; Go's lock-taking resolver methods borrow it per call
  (`slot.with`), the lock-free ones (`IsSymbolAccessible`, `*Unsafe`, `GetPropertiesOfContainerFunction`) take `c`.
  `SymbolTracker::track_symbol` and `report_inference_fallback` now take `c: &mut Checker` (the only tracker methods
  whose declaration-emit implementation needs the checker; they run inside node-builder calls).
- `ast::required_child`: when a visitor drops a non-optional child, Go stores nil and Rust keeps the original. This
  only happens in the transformer's side-effect traversals (`visitThisPropertyAssignments` returns nil outside its
  `this` container), whose results are discarded. Before this, 11 JS tests crashed ("visitor removed a required
  child").
- `NodeVisitor` is `Clone`; transformers hand out visitor copies (`tx.visitor()`), which is equivalent to Go's shared
  pointer because visitors carry no state.

## Results

13,458 pass / 2 codes / 2 fail / 0 crash (from 13,398 / 2 / 62). Every declaration-diagnostic test passes on the
first full run after merging; the only follow-up was the required-child crash above.

## Open / doubts (from the helper notes, none observed in the suite)

- Factory calls where Go would store a nil child still `unwrap()` in a few places (heritage clause list, binding
  name); unreachable in practice.
- `create_type_parameters_of_signature_declaration` maps an empty list to `None` (Go returns nil exactly then).
- Two emit-resolver loops iterate symbol tables in insertion order where Go's map order is random (one yes/no answer,
  one mirrors existing checker code).
