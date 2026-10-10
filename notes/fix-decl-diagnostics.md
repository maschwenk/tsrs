# decl-diagnostics: declaration diagnostics (TS4xxx / TS9xxx / TS2883 / TS7056 / TS2527 / TS5088 …)

The declarations transformer (`crates/tsrs_declarations`, transform.go split into transform_{1,2,3}.rs) and the emit
resolver were ported so that `Program::get_declaration_diagnostics` runs per checker group on the checker threads,
with a per-file cache like Go's `declarationDiagnosticCache`. Result: conformance 13,458 pass / 2 codes / 2 fail /
0 crash (from 13,398 / 2 / 62); every declaration-diagnostic test passed on the first full run after merging.

## Design decisions worth knowing

- The checker reaches the transformer through `Resolver` (now `crates/tsrs_transformers/src/resolver.rs`, re-exported
  by tsrs_declarations): the caller lends the file's checker to a `CheckerSlot` (`tsrs_checker`,
  nodebuilder_types.rs) for the whole transform; Go's lock-taking resolver methods borrow it per call (`slot.with`),
  the lock-free ones (`IsSymbolAccessible`, `*Unsafe`, `GetPropertiesOfContainerFunction`) take `c`.
  `SymbolTracker::track_symbol` and `report_inference_fallback` take `c: &mut Checker` (the only tracker methods
  whose declaration-emit implementation needs the checker; they run inside node-builder calls).
- `ast::required_child` (visitor.rs): when a visitor drops a non-optional child, Go stores nil and Rust keeps the
  original. This only happens in the transformer's side-effect traversals (`visitThisPropertyAssignments` returns nil
  outside its `this` container), whose results are discarded. Before this, 11 JS tests crashed ("visitor removed a
  required child").
- `NodeVisitor` is `Clone`; transformers hand out visitor copies (`tx.visitor()`), which is equivalent to Go's shared
  pointer because visitors carry no state.

## Open doubts (none observed in the suite)

- Factory calls where Go would store a nil child still `unwrap()` in a few places (heritage clause list, binding
  name); unreachable in practice.
- `create_type_parameters_of_signature_declaration` maps an empty list to `None` (Go returns nil exactly then).
- Two emit-resolver loops iterate symbol tables in insertion order where Go's map order is random (one yes/no answer,
  one mirrors existing checker code).
