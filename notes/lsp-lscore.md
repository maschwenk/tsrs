# lsp-lscore: language-service core (`tsrs_ls`)

Wave agent `lscore`, branch `lsp-lscore`. Go `internal/ls`, `ls/lsconv`, `ls/lsutil` -> crate `tsrs_ls`.

## Ported

| Go (`ts-ref/tsc/internal/…`) | Rust (`crates/tsrs_ls/src/…`) |
| --- | --- |
| `ls/lsconv/converters.go` (+ `converters_test.go`) | `lsconv/converters.rs` (+ `converters_test.rs`) |
| `spanmap/spanmap.go` (types only, PLACEHOLDER) | `spanmap.rs` |

### API notes for callers

- `lsconv::new_converters(encoding, get_line_map) -> Arc<Converters>`; `get_line_map: Fn(&str) ->
  Option<Arc<LSPLineMap>> + Send + Sync` (Go's func may return nil, e.g. `Snapshot.LSPLineMap`; the converters
  panic on nil like Go's nil dereference).
- `lsconv::Script` trait (Go interface): `file_name`, `original_file_name`, `text`, `span_map -> Option<&SpanMap>`,
  `original_text`. Implemented for `SourceFile` and `P<SourceFile>`. Overlays (project) implement it themselves
  with `span_map() -> None`.
- Methods that take Go's `Script` interface take `&dyn Script`; the generic ones (`from_lsp_range[T]`,
  `from_lsp_position[T]`) take `T: Script + Clone` by value. `MappedSpan<T>` / `MappedPosition<T>` embed the
  spanmap value (`mapped_span` / `mapped_position` field + `Deref`, so `positions[0].position` works).
- `diagnostic_to_lsp_push(ctx, &Converters, P<Diagnostic>)`, `diagnostic_to_lsp_pull(ctx, &Converters,
  P<Diagnostic>, report_style_checks_as_warnings)` return `lsproto::Diagnostic` (Go `*lsproto.Diagnostic`, never nil).

## Shared-file edits

- `Cargo.lock`: was missing `tsrs_astnav` in `tsrs_ls`'s dependency list (already in Cargo.toml).

## Deviations

- `spanmap`: content mappers are out of scope. `tsrs_ls::spanmap` ports the value types (`Feature`, `Fidelity`,
  `MappedPosition`, `MappedSpan`) and declares `SpanMap` as an uninhabited enum whose methods have Go's signatures
  and `match *self {}` bodies, so every "content-mapped" branch keeps Go's structure and is statically unreachable.
- `diagnosticToLSP`: Go reads `locale.FromContext(ctx)`; only English is ported (`Diagnostic::localize()`).
- `diagnosticScriptAndRange` takes `P<SourceFile>` (Go accepts nil and returns it, after which both callers
  dereference it); the related-information loop panics on a related diagnostic without a file, as Go does.
- `FileNameToDocumentURI`: Go `url.PathEscape` is a local `url_path_escape` (net/url `shouldEscape` for
  `encodePathSegment`, uppercase hex); `strings.NewReplacer` with single-byte keys is a byte-wise replace.
- Tests: `TestConvertersSourceFileProjectionExpansion` (content mappers) and `TestConvertersInvalidUTF8` (Rust text
  is always valid UTF-8, the invalid-byte path is unreachable) are not ported. `TestConvertersAgainstJSReference`
  runs node like Go and is skipped (passes) when node is missing.

## Needs from others

## Doubts
