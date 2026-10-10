# lsp-lsfound: language-service foundation (astnav, lsutil, format, sourcemap, lsconv/linemap)

Wave `lsfound` of the language-server port. astnav is now crate `tsrs_astnav`, sourcemap is crate `tsrs_sourcemap`,
the rest is in `tsrs_ls` (docs/LSP.md, crate map). This note keeps the formatter oracle result and the deviations
from Go that still hold.

## Format oracle

`crates/tsrs_ls/src/format/oracle_test.rs` compares the formatter edit for edit with Go's (`tools/oracle/format`,
opt-in via `TSRS_FORMAT_ORACLE` / `TSRS_FORMAT_ORACLE_PRESET`). Result at the time of the port: all 12,750
compiler + conformance test files match Go for FormatDocument with default settings, with two non-default presets
(`alt`: tabs, all optional spaces flipped, braces on new lines, semicolons=remove; `insert`: same + semicolons=insert,
CRLF) and for `GetIndentation` at every line start (`indent`); checker.ts with the api_test settings matches. The
astnav "go baseline json" tests reproduce `testdata/baselines/reference/astnav/*.baseline.json` byte for byte; the
three TS-comparison baselines also match when `TSRS_TYPESCRIPT_JS=<path to typescript.js>` is set (skipped otherwise).

## Deviations that still hold

- `format` takes a `&format::FormatContext` (format/api.rs) for every Go `ctx` parameter: Go stores two values in
  the request context (`formatOptionsKey`, `formatNewlineKey`); callers that hold a `tsrs_core::context::Context`
  carry the `FormatContext` as one value of it (`completions::format_context`).
- `format/rules.go`: Go's `tokenRangeFromEx(allTokens, …)` appends into `allTokens`' one spare slot twice, so
  `anyTokenIncludingMultilineComments` ends with EndOfFile, not MultiLineCommentTrivia; reproduced (commented in
  format/rules.rs).
- In `format/span.go` the visitor's callbacks re-enter the worker, which Rust closures cannot borrow, so
  `execute_process_node_visitor` first collects the children in `VisitEachChild` order (single nodes via `visit`,
  lists via the `VisitNodes` hook, exactly Go's dispatch) and then processes them in that order; processing never
  changes the node's children.
- `contextPredicate` closures are leaked `&'static dyn Fn + Send + Sync` (the rules map is a process-wide
  `OnceLock`, like Go's `sync.OnceValue`); rule structs are leaked once.
- `norm_nfd_string` (tsrs_core stringutil/norm.rs, for organizeimports' collation; tables generated from Go by
  `tools/oracle/unicodenorm`) does not reproduce x/text's Stream-Safe Text Format (U+034F inserted after 30
  consecutive non-starters); irrelevant for module specifiers / import names.
- `tsrs_core::goslices::sort_func` is Go's `slices.SortFunc` pdqsort with the same order of equal elements:
  `source_mapper.go` sorts mappings with it and then deduplicates and binary-searches, so the order of mappings with
  equal positions decides which one `GetSourcePosition` / `GetGeneratedPosition` return.
- userpreferences: Go reads the `raw` / `config` / `fallbackConfig` struct tags by reflection; Rust has a static
  table `USER_PREFERENCES_FIELD_TAGS` (one entry per tagged field in Go declaration order, nested settings structs
  flattened in place) with `FieldRef` / `FieldMut` accessors. A test destructures every struct exhaustively, so a new
  field without a table entry fails to compile.
- userpreferences types: `IndentStyle` is a newtype over `i32` (`parseIndentStyle` stores any number),
  `WorkspaceSymbolsScope` a newtype over a string, `SemicolonPreference` has a `None` variant for Go's `""`, `[]string`
  fields are `Option<Vec<String>>` (Go serializes nil and empty differently). JSON numbers are f64 only, so Go's
  `case int` / `case float64` branches are one branch.
- sourcemap: Go's `tryParseBase64Url` slices `url[:6]` after `charset=` without a length check; the port panics the
  same way. `RawSourceMap.version`: Go rejects a non-integer literal like `3.0` / `3e0` for an `int`; the port
  accepts any integral number. Decoder accumulators are `i32` (Go `int`, 64-bit); results differ only past 2^31.
