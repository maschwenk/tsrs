# perf-parallel: parallel checking

## Shared-mutable audit (what checkers can reach)

- (a) Written by parser/binder/config parsing only, read-only afterwards: all `Node`/`NodeList` fields, generated
  node data cells, `SourceFile` fields, `Symbol` fields and `SymbolTable`s of binder symbols, flow nodes,
  parse/bind diagnostics, `TsConfigSourceFile`. These are `OwnedCell` / `FrozenCell` (no runtime borrow flag).
- (b) Lazily initialized during checking in Go too: node/symbol ids (atomics), `SourceFile` line map / position
  map / identifiers (`OnceLock`), binding (`Once`, Go `bindOnce`), lazy JSDoc cache (`jsdoc_mu` RwLock, Go
  `jsdocMu`), program-level include-reason diagnostics (`Mutex`es).
- (c) Written by our checker although Go keeps it checker-owned: none found. Checked dynamically: a
  `checked-cells` build (feature of tsrs_core) with `TSRS_CHECK_SHARED=1` records every allocation up to the end of
  binding and panics on any later `OwnedCell::set` / `FrozenCell::borrow_mut` to it (crates/tsrs_core/src/ptr.rs);
  the conformance suite and the private monorepo with 4 checkers were clean. Static review of every symbol-field
  write in the checker: all on symbols the checker created (`mergeSymbol` clones non-transient targets as Go does).
- Statics shared across checkers: `primitive_type_alias_suggestions` (Go shares it too), read-only maps.

## Other changes

- `Symbol.declarations` is an arena slice (Go slice semantics: copies share, append replaces). Removed a Vec
  clone per instantiated/cloned symbol and at many read sites: -0.6 GB (1 checker) / -0.9 GB (4 checkers).
- `TSRS_TEST_BACKTRACE=<file>` makes test workers append panic backtraces to a file.

## Limitations

- Paths not exercised by the suite or the private monorepo are covered only by the static review.
- A panic inside a checker thread is re-raised after the threads are joined (`resume_unwind` in
  crates/tsrs_compiler/src/checkerpool.rs), but its location is recorded on the checker thread, so crash reports
  under the test runner lack the location; only matters with parallel test programs.
