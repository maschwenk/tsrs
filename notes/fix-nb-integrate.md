# fix-nb-integrate: node builder first light and landing

## What happened

- Took over `nb-integrate` at add8ff4 (six `body/nb-*` branches merged, SIG markers resolved). The uncommitted edit made
  `is_value_symbol_accessible` take `Option<P<Symbol>>` (Go `getMergedSymbol(nil parent symbol)` is nil in
  `shouldWriteTypeOfFunctionSymbol`) and made `should_{emit,write}_type_of_*symbol` return `Option<P<Symbol>>`;
  committed as is. A `TSRS_DBG` backtrace in `get_resolved_base_constraint` was a debugging leftover (removed).
- Merged origin/main (parallel checking, shared `Symbol.declarations` slice): only fallout was
  `declarations().clone()` in the node-builder files (now `&'static [P<Node>]`).
- First full run: **13,398 pass / 2 codes / 62 fail / 0 timeout / 0 crash** (main: 10,519 / 2,876 / 67 / 0 / 0), no
  previous pass lost. Landed as a merge on main (657bc02).
- Project counters now equal tsgo-ref exactly (single-threaded and 4 checkers), 0 errors; peak memory is unchanged
  versus main (19.5 GB vs 19.4 GB single-threaded, same machine, same hour).

## Nil children audit

Every node-builder entry point the checker reaches is in printer.go (`typeToString`, `symbolToString`,
`signatureToString`, `typePredicateToString`, …) and all of them add `FlagsIgnoreErrors`, which contains
`AllowEmptyUnionOrIntersection`. Under that flag `typeToTypeNode` returns nil only for a union/intersection whose
`mapToTypeNodes` is empty, which cannot happen for a real union. So the `.unwrap()`s on `typeToTypeNode` results that
feed non-optional factory children (conditional/tuple/indexed-access/template/keyof/string-mapping children,
`appendReferenceToType` arguments, `mapToTypeNodes` list elements, signature type arguments) are unreachable without
declaration emit / hover flags. The existing-node visitor (nodecopy) returns nil only to drop a member declaration
from a list, so its `visit_node(..).unwrap()`s on required children do not fire either. Left as is; revisit if
declaration emit is ported (then those AST fields must become `Option`, as in Go).

Fixed: `ArrowFunction.equals_greater_than_token` is nilable (Go's `signatureToSignatureDeclarationHelper` passes nil
and the printer then emits no `=>`); regenerated generated.rs, AST oracle 113/113 libs, 17318/17319 tests (the one
known non-UTF-8 case).

## Remaining clusters (`python3 tools/cluster-diffs.py [--class fail]`)

- codes 2: `incorrectRecursiveMappedTypeConstraint`, `typeParameterWithInvalidConstraintType` — missing
  `related TS2751 Circularity originates in type at this location`. Not a checker bug: the Go harness
  (harnessutil.go:675) runs `program.Emit` before collecting diagnostics; the const-enum inliner
  (transformers/inliners/constenum.go) calls `GetConstantValue` -> `checkExpressionCached` on every property/element
  access, so the circular constraint is first hit with `currentNode` = `x.foo`, and the later check adds the related
  info to the already-collected diagnostic (`DiagnosticsCollection.Add` returns the existing one). `tsgo-ref` CLI
  output equals ours. Same cause for fails `mutuallyRecursiveInference` and `recursiveMappedTypes` (error reported at
  the property access that emit touches first).
- fail 58: declaration-emit diagnostics (TS2883 10, TS9xxx isolatedDeclarations 20+, TS4xxx 15+, TS7056 3, TS2527 3,
  TS7080 2, TS5088 1, TS6424 1): the declarations transformer / emit resolver pipeline is not ported.
- fail 2: see codes above (harness emit order).

## Other observations

- Suite with `TS_TEST_PROGRAM_SINGLE_THREADED=false` equals the single-threaded run; one run hit the testrunner's 6 GB
  per-worker memory limit on `verifyDefaultLib_dom` (4 checkers over lib.dom after ~200 tests in the same worker);
  not reproducible on rerun, and main shows the same headroom.
- `Symbol.declarations().to_vec()` copies remain in node-builder/symbol-accessibility code (12 sites) where the loop
  body needs a `Vec`; cheap, not hot.
