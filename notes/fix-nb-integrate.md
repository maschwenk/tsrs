# fix-nb-integrate: node builder first light and landing

The six `body/nb-*` node-builder branches landed as a merge on main (657bc02). First full run: 13,398 pass / 2 codes /
62 fail / 0 crash (main before: 10,519 / 2,876 / 67). The other 60 fails (58 of them clustered as
declaration-emit diagnostics in this run) passed once declaration diagnostics landed (notes/fix-decl-diagnostics.md:
13,458 / 2 / 2). Private-monorepo counters equal tsgo-ref exactly (single-threaded and
4 checkers), and peak memory was unchanged versus main (19.5 GB vs 19.4 GB single-threaded, same machine, same hour).

## The 2 codes and 2 fails that remain (harness emit order, not a checker bug)

- codes 2: `incorrectRecursiveMappedTypeConstraint`, `typeParameterWithInvalidConstraintType`: missing
  `related TS2751 Circularity originates in type at this location`. The Go harness (harnessutil.go:675) runs
  `program.Emit` before collecting diagnostics; the const-enum inliner (transformers/inliners/constenum.go) calls
  `GetConstantValue` -> `checkExpressionCached` on every property/element access, so the circular constraint is first
  hit with `currentNode` = `x.foo`, and the later check adds the related info to the already-collected diagnostic
  (`DiagnosticsCollection.Add` returns the existing one). `tsgo-ref` CLI output equals ours.
- fail 2: `mutuallyRecursiveInference` and `recursiveMappedTypes`, same cause (the error is reported at the property
  access that emit touches first).

Clustering tool for such diffs: `python3 tools/cluster-diffs.py [--class fail]`.

## Nil children audit

Every node-builder entry point the checker reaches is in printer.go (`typeToString`, `symbolToString`,
`signatureToString`, `typePredicateToString`, …) and all of them add `FlagsIgnoreErrors`, which contains
`AllowEmptyUnionOrIntersection`. Under that flag `typeToTypeNode` returns nil only for a union/intersection whose
`mapToTypeNodes` is empty, which cannot happen for a real union. So the `.unwrap()`s on `typeToTypeNode` results that
feed non-optional factory children are unreachable from the checker's printing paths. The existing-node visitor
(nodecopy) returns nil only to drop a member declaration from a list, so its `visit_node(..).unwrap()`s on required
children do not fire either. (Declaration emit has since been ported; the one crash it hit is handled by
`ast::required_child`, notes/fix-decl-diagnostics.md.)

`ArrowFunction.equals_greater_than_token` is nilable (Go's `signatureToSignatureDeclarationHelper` passes nil and the
printer then emits no `=>`).
