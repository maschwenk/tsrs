# utilities agent notes (utilities.rs, exports.rs, jsdoc.rs)

## Signature changes

- `find_in_map<K, V>` (utilities.go:38): added bound `V: Copy + Default` (Go returns the value by copy and
  `*new(V)` = zero value when nothing matches). No other change.
- `min_and_max<T>` (utilities.go:1316): added bound `T: Copy` (Go passes elements by value to `getValue`).

## Shared-file edits

None.

## Needs from others

- `Checker::compare_symbols` (checker.rs) takes `P<Symbol>`, but Go's `compareSymbols` field is called with
  possibly-nil symbols (`CompareTypes` passes `t1.symbol`/`t2.symbol`). `compare_types` therefore calls
  `compare_symbols_worker(Option, Option)` directly (marked `// SIG:`). Consider making `compare_symbols` take
  `Option<P<Symbol>>`.
- `new_diagnostic_for_node` / `new_diagnostic_chain_for_node`: the generated `message: Option<&'static Message>`
  is kept (callers must pass `Some(&diagnostics::X)`); Go would nil-deref a nil message, so the body unwraps.
- `printer.EscapeString` (printer/utilities.go) is not ported anywhere; `value_to_string` needs it, so a
  private copy (`escape_string` + `escape_string_worker`, all flag branches) lives at the bottom of
  utilities.rs. Replace it with the real printer port when one exists (private, so no name clash).
- `get_expanded_parameters` (nodebuilderimpl.go) is only an advisory signature; `get_expanded_parameters_exported`
  is `unimplemented!("node builder")` (only the language service / API calls it).
- `requires_adding_implicit_undefined` (exports.go:374) ports the prefix, then `unimplemented!("emit resolver")`
  at the `GetEmitResolver().RequiresAddingImplicitUndefined` call (only declaration emit / LS call it).

## Doubts

- `new_diagnostic_for_node`: `scanner.GetErrorRangeForNode(file, node)` with a nil file is unwrapped (a node
  without a source file would panic here; Go would pass nil through and likely also crash).
- `compare_nodes`: Go indexes `fileIndexMap` with a possibly nil `*SourceFile` (yields 0); ported as
  `unwrap_or(0)`.
- `compare_symbols_worker` / `compare_types` id fallback: Go `int(a) - int(b)` computed in i64 then cast to i32.
- `is_valid_big_int_string`: Go's error callback is replaced by the buffered scanner errors
  (`set_on_error(true)` + `has_errors()` after both scans); the augmented text is arena-allocated per call.
- `symbols_to_array`, `find_in_map`: Go iterates maps in random order; Rust uses insertion / hash order.
- `get_packages_map` returns a clone of the cached map (Go returns the map itself; it is only read).
- jsdoc.go:42 `lastJSDocParam == nil` check dropped: `AsJSDocParameterOrPropertyTag()` never returns nil.
- `sort_symbols` uses a stable sort where Go's `slices.SortFunc` is unstable; the comparator is total except for
  identical symbols, so the result is the same.
