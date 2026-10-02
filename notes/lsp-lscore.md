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
| `ls/utilities.go` (whole) | `utilities.rs` |
| `ls/lsutil/symbol_display.go` | `lsutil/symbol_display.rs` |
| `ls/displaypartswriter.go`, `ls/hovericon.go`, `ls/jsdoc.go`, `ls/hover.go` | `displaypartswriter.rs`, `hovericon.rs`, `jsdoc.rs`, `hover.rs` |
| `ls/definition.go`, `ls/sourcedefinition.go` | `definition.rs`, `sourcedefinition.rs` |
| `ls/diagnostics.go`, `ls/source_map.go` | `diagnostics.rs`, `source_map.rs` |
| `ls/constants.go`, `ls/api.go` | `constants.rs`, `api.rs` |
| PARTIAL `ls/findallreferences.go` (`refInfo`, `getContextNode`, `getRangeOfNode`), `ls/completions.go` (`ErrNeedsAutoImports`, `getSwitchedType`, `isEqualityOperatorKind`), `ls/format.go` (`getRangeOfEnclosingComment`) | `findallreferences.rs`, `completions.rs`, `lsformat.rs` (renamed: `format` is Go `internal/format`) |

Not ported: `ls/crossproject.go` functions (`handleCrossProject`, `combine*`): only references / rename /
implementations / call hierarchy use them (phase 3); hover, definition, diagnostics do not (brief step 4 n/a).

### Tests

- `cargo test -p tsrs_ls --lib`: 87 pass. New: `lsconv/converters_test.rs` (Go tests: URI <-> file name both ways,
  `TestConvertersAgainstJSReference` via node), `ls_smoke_test.rs` (not a Go test): builds an in-memory program with
  the bundled libs and compares the JSON of `provide_hover` / `provide_definition` / `provide_type_definition` /
  `provide_diagnostics` byte for byte with responses recorded from `tsgo-ref --lsp -stdio` (markdown hover with
  JSDoc `@param`/`@returns`/`@example`/`@see`/`@deprecated`/`{@link}`, constructors, methods, enum members,
  namespaces, overloads, type aliases, type parameters, import aliases, `this`, definition links, pull
  diagnostics incl. suggestion/deprecated; a Visual Studio client's `_vs_rawContent` classified runs). 23 responses,
  all identical. The recorder is
  `target/scratch/lscore/drive.py` (scratch).

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
- `crates/tsrs_ast/src/utilities_3.rs` (appended, origin markers): `is_deprecated_declaration` (utilities.go:1240),
  `is_let` (:3043), `has_initializer` (:3086), `is_string_text_containing_node` (:3274),
  `is_right_side_of_property_access` (:3646).
- `crates/tsrs_checker/src/printer.rs`: `signature_to_string_ex`, `symbol_to_string_ex` `pub(crate)` -> `pub` (Go
  exported `SignatureToStringEx` / `SymbolToStringEx`, called by hover).
- `docs/LSP.md` "Known gaps": autoimport and spanmap placeholders.

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

- hover.go: the closures of `getQuickInfoAndDeclarationAtLocation` share mutable state (`dpw`, `aliasLevel`,
  `firstDeclaration`, ...); they are methods of a private `QuickInfoWriter` that receive the checker first
  (PORTING.md callback rule). `vc` is never nil there (Go replaces nil by an empty context), so the `vc == nil`
  checks are gone. The node builder's shared `idToSymbol` map is copied into the printer after building (the
  printer only reads it). `documentationLocationMapper` is `&dyn Fn(P<SourceFile>, TextRange) -> (Location,
  Fidelity)`.
- sourcedefinition.go: `module.NewResolver(Host: program.Host())` needs an adapter from `CompilerHost` to
  `module::ResolutionHost` (Go's interfaces match structurally); it is arena-allocated per request (leaked, like
  the resolver's resolution data, per the phase-1 memory plan). `SourceDefResolver<'a>` borrows the
  `LanguageService` (Go keeps a pointer); `parsedFiles` is a `RefCell`. `ast.IsDefaultImport` is a private copy in
  sourcedefinition.rs (only user).
- `CaseClauseTracker` (utilities.go, Go `any` values): enum `TrackerValue`; numbers compare by `f64 ==` (Go map key
  semantics). Only completions use it.
- Language-service request functions return `Result<_, lsproto::Error>`; Go's sentinel errors of package ls are
  message constants (`ERR_NO_SOURCE_FILE`, `ERR_NEEDS_AUTO_IMPORTS`) compared by message.
- `getMappedLocation`: Go passes a nil `*script` to the converters when the file cannot be read (nil dereference);
  the port panics with "nil script" at the same point.

## Needs from others

## Doubts

- `impl Script for P<SourceFile>`: with `lsconv::Script` in scope, `file.text()` on a `P<SourceFile>` resolves to the
  trait method (borrow-tied `&str`) before deref to `SourceFile::text` (`&'static str`). Files that only need the
  trait as a type name it by path (`&dyn crate::lsconv::Script`).
