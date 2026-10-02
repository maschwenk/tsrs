# lsp-lscore: language-service core (`tsrs_ls`)

Wave agent `lscore`, branch `lsp-lscore`. Go `internal/ls`, `ls/lsconv`, `ls/lsutil` -> crate `tsrs_ls`.

## Ported

| Go (`ts-ref/tsc/internal/…`) | Rust (`crates/tsrs_ls/src/…`) |
| --- | --- |
| `ls/lsconv/converters.go` (+ `converters_test.go`) | `lsconv/converters.rs` (+ `converters_test.rs`) |
| `spanmap/spanmap.go` (types only, PLACEHOLDER) | `spanmap.rs` |
| `ls/host.go` | `host.rs` (trait `Host`) |
| `ls/languageservice.go` | `languageservice.rs` |
| `ls/crossproject.go` (interfaces `Project`, `CrossProjectOrchestrator` only) | `crossproject.rs` |
| `ls/autoimport/{registry,view}.go` (PLACEHOLDER subset) | `autoimport/{mod,registry,view}.rs` |
| `ls/lsutil/formatcodeoptions.go` `FromLSFormatOptions`, `ToLSFormatOptions` | `lsutil/formatcodeoptions.rs` |

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

- `Host` (trait, `Send + Sync`): Go's methods; `ReadFile` -> `Option<String>`, `Converters()` -> `Arc<Converters>`,
  `AutoImportRegistry()` -> `Option<Arc<autoimport::Registry>>` (Go nil-able), `GetECMALineInfo` ->
  `Option<Arc<ECMALineInfo>>`, `ReadDirectory(..., depth usize)`.
- `new_language_service(ProjectID, &'static Program, Arc<dyn Host>, active_file) -> LanguageService` (Send + Sync;
  wrap in `Arc` where Go shares the pointer). `get_program() -> &'static Program`.
- `ls::Project` trait: `id() -> String`, `get_program() -> Option<&'static Program>`, `has_file`.
  `CrossProjectOrchestrator` trait: projects are `Arc<dyn Project>` (Go compares them by identity: `Arc::ptr_eq`),
  `get_language_service_for_project_with_file -> Option<Arc<LanguageService>>`, `get_projects_for_file ->
  Result<Vec<_>, lsproto::Error>`, `get_projects_loading_project_tree -> Box<dyn Iterator>` (Go `iter.Seq`).
- autoimport placeholder: `ProjectID(pub String)` (Go `fmt.Stringer` interface whose only implementation is
  `project.ID`); `new_registry(to_path: Arc<dyn Fn(&str) -> Path>, prefs) -> Arc<Registry>`;
  `Registry::clone_registry(ctx, RegistryChange, &dyn RegistryCloneHost, logger: L)` (`clone` is taken by Rust;
  the logger is generic because Go's `project/logging` sits in `tsrs_project`, above this crate);
  `is_prepared_for_importing_file` (also on `Option<Arc<Registry>>` via `RegistryExt`, Go's nil receiver);
  `node_modules_directories`; `RegistryCloneHost: tsrs_module::ResolutionHost`; `new_view`. `GetCacheStats`
  (telemetry only) is not provided.
- `ErrNeedsAutoImports` (completions.go:35): `err_needs_auto_imports()` / `is_err_needs_auto_imports(&Error)`
  (`lsproto::Error` message comparison; Go `errors.Is`).

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

- `LanguageService.documentPositionMappers` is behind a `Mutex` (Go: plain map, single goroutine) so the service is
  `Sync`; the cached value is `Option<Arc<DocumentPositionMapper>>` (Go caches nil too).
- `autoimport::View` does not keep the checker (no lifetime parameters; nothing reads it until phase 3).
- crossproject.go functions (`handleCrossProject`, `combine*`) are not ported: only references, rename,
  implementations and call hierarchy use them (phase 3); hover/definition/diagnostics do not.

## Needs from others

## Doubts
