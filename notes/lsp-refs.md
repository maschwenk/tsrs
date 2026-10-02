# lsp-refs: references, rename, highlights, implementations, call hierarchy (`tsrs_ls`)

Wave agent `refs`, branch `lsp-refs` (merged `lsp` at 88c3089 to build on tsrs_lsp/tsrs_project).

## Ported

| Go (`ts-ref/tsc/internal/ls/…`) | Rust (`crates/tsrs_ls/src/…`) |
| --- | --- |
| `findallreferences.go` (whole; the lscore helpers kept) | `findallreferences.rs` |
| `importTracker.go` | `import_tracker.rs` |
| `crossproject.go` (`handleCrossProject`, `combine*`; interfaces were already there) | `crossproject.rs` |
| `rename.go` | `rename.rs` |
| `documenthighlights.go` | `documenthighlights.rs` |
| `callhierarchy.go` | `callhierarchy.rs` |
| PARTIAL `symbols.go`: `isInsideNodeModules`, `getSymbolKindFromNode` | `symbols.rs` (the symbols wave completes it) |
| `findallreferences_test.go` | `findallreferences_test.rs` (passes) |

Entry points for the LSP handlers (all `Result<_, lsproto::Error>`; `orchestrator: Option<&dyn CrossProjectOrchestrator>`,
Go's nil interface = `None`):

- `references`: `ls.provide_references(ctx, &ReferenceParams, orch)`; `_vs_references`: `provide_vs_references`.
- `implementation`: `provide_implementations(ctx, &ImplementationParams, orch)`.
- `rename`: `provide_rename(ctx, &RenameParams, orch)`; `prepareRename` and the file-rename branch of `handleRename`:
  `get_rename_info(ctx, new_name, &uri, position) -> RenameInfo` (pub fields `can_rename`, `localized_error_message`,
  `display_name`, `trigger_span`, `file_to_rename`, `new_file_name`); `client_supports_will_rename_files`,
  `client_supports_document_changes`, `client_supports_rename_resource_operations` (crate root).
- `documentHighlight`: `provide_document_highlights(ctx, &uri, position)`; multi: `provide_multi_document_highlights(ctx,
  &uri, position, &files_to_search)`.
- `prepareCallHierarchy`: `provide_prepare_call_hierarchy(ctx, &uri, position)`; incoming:
  `provide_call_hierarchy_incoming_calls(ctx, &item, orch)`; outgoing: `provide_call_hierarchy_outgoing_calls(ctx, &item)`.
- Exported for other packages: `get_referenced_symbols_for_node_exported` (Go `GetReferencedSymbolsForNode`),
  `get_signature_usages`, `SymbolAndEntries` / `ReferenceEntry` / `Definition` / `SignatureUsage` / `SymbolAndEntriesData`.

The server (tsrs_lsp, not edited here) still answers these methods with `not_yet_ported`; wiring is e.g.
`register_multi_project_reference_request_handler(TEXT_DOCUMENT_REFERENCES_INFO, |ls, ctx, p, o| ls.provide_references(ctx, &p, Some(&*o)))`.

### Tests

- `cargo test -p tsrs_ls`: all pass (91 lib tests). New: `findallreferences_test.rs` (Go
  `TestImplementationsWorklistDoesNotBlowUp`), `refs_smoke_test.rs` (not a Go test): in-memory two-file programs with the
  bundled libs, 73 requests (references, `_vs_references`, rename, prepareRename, documentHighlight,
  multiDocumentHighlight, implementation as locations and as links, prepareCallHierarchy, incoming/outgoing calls)
  compared byte for byte with `tsgo-ref --lsp -stdio`; all identical. Data and recorder: `testdata/refs_smoke*/`
  (`drive.py`: `CAPS='<client caps json>' python3 drive.py <dir> reqs.json > expected.txt`).
- The smoke test gives the program a small test checker pool (`ProgramOptions.create_checker_pool`): call hierarchy
  acquires a checker while holding one, as Go does; the compiler pool locks one checker per acquisition, so a nested
  acquisition on it would deadlock (see Doubts).

## Shared-file edits

- `crates/tsrs_ast/src/utilities_3.rs` (appended, origin markers): `get_super_container` (utilities.go:1864),
  `is_default_import` (:2592), `is_argument_expression_of_element_access` (:3650), `climb_past_property_access` (:3654),
  `climb_past_property_or_element_access`, the callee selectors and `is_call_expression_target`,
  `is_new_expression_target`, `is_call_or_new_expression_target`, `is_tagged_template_tag`, `is_decorator_target`,
  `is_jsx_opening_like_element_tag_name`, `is_callee_worker` (:3661-3713), `import_from_module_specifier` (:4195),
  `try_get_import_from_module_specifier` (:4203).
- `crates/tsrs_ast/src/ast.rs`: `SourceFile.name_table` (`OnceLock`) + `SourceFile::get_name_table` (ast.go:2857).
- `crates/tsrs_checker/src/checker_04.rs`: two calls qualified as `crate::utilities::get_super_container` (the glob
  import of tsrs_ast made the name ambiguous; no behavior change).
- `crates/tsrs_ls`: `hover.rs` (`SymbolDisplayInfo`, its `display_parts`, `get_quick_info_and_declaration_at_location` ->
  `pub(crate)`), `definition.rs` (`lsp_range_contains` -> `pub(crate)`), `languageservice.rs` (`project_id` ->
  `pub(crate)`), `lib.rs` (modules, re-exports). `sourcedefinition.rs` keeps its private `is_default_import` copy.

## Deviations

- `ReferenceEntry`: Go shares `*ReferenceEntry` and fills `sourceFile`/`textRange`/`lspRange` lazily; the port shares
  `Rc<ReferenceEntry>` with those fields in cells. `Definition` is a `Copy` value (Go never compares the pointer).
- `refState.referenceAdder` returns the group's index in `result` (Go: a closure appending to a `*SymbolAndEntries`
  kept in `symbolToReferences`); `addImplementationReferences` / `findOwnConstructorReferences` /
  `findSuperConstructorAccesses` collect nodes and the caller adds them afterwards (nothing in between reads the
  result). The state holds `&mut Checker`; `forEachRelatedSymbol`'s `fromRoot` closure is a method; callbacks get the
  state first (PORTING.md callback rule). `ImportTracker` (a Go closure) is a struct with `call(checker, ...)`;
  `getImportersForExport` / `getSearchesFromDirectImports` closures are methods of private structs.
- `handleCrossProject`: the per-project searches run sequentially on the calling thread, last-queued first (tsrs_core
  `WorkGroup` order), not as goroutines (the request already runs on a worker thread; no thread per request). A panic
  in a search is caught and re-raised after the queue drains, like Go's recover + panic. `iter.Seq[Resp]` is a `Vec`
  (`combineImplementations`' restart of the iteration is kept by passing all results again). `sync.Map`'s random
  `Range` order and Go's random map orders (rename `changes`, call-site grouping, combined rename edits) are
  insertion order here; results that Go sorts are sorted the same way.
- `nonLocalDefinition.GetSourcePosition/GetGeneratedPosition` (`sync.OnceValue` closures over the creating service)
  are `OnceCell`s whose getters take that service (always the default service in `handleCrossProject`).
- `(T, bool)` results are `Option<T>` (`provideSymbolsAndEntries`, `getLocationOfEntryForFeature`,
  `getRenameInfoForNode`, `deduplicateRenameEdits`); nil-vs-empty slices that callers distinguish are
  `Option<Vec<_>>` (`getReferencedSymbolsSpecial`, `getReferencedSymbolsForModuleIfDeclaredBySourceFile`).
- `getPossibleSymbolReferencePositions`: Go's quirk (index relative to `container.Pos()` used as absolute) kept;
  `strings.Index` from a non-boundary byte offset searches from the next char boundary (same result: a match never
  starts at a continuation byte).
- `isDefinitionVisible` takes the checker too (Rust `EmitResolver` methods take it).
- `getRenameInfoError`: English only (`Message::localize`). Context errors become `lsproto::Error` via
  `From<ContextError>` (tag `ContextCanceled`).

## Needs from others

- Server wiring (tsrs_lsp): replace the `not_yet_ported` handlers for references, `_vs_references`, implementation,
  rename (incl. the `FileToRename` path, which needs `GetEditsForFileRename` from file_rename.go), prepareRename,
  documentHighlight, multiDocumentHighlight, prepareCallHierarchy, incoming/outgoing calls.
- symbols wave: `symbols.rs` currently holds only `is_inside_node_modules` and `get_symbol_kind_from_node`.

## Doubts

- Nested checker acquisition: call hierarchy (`getOutgoingCalls` -> `resolveCallHierarchyDeclaration`,
  `getCallHierarchyItemName`) acquires a checker while holding one, exactly like Go. Go's compiler pool hands out the
  same checker without locking; the Rust compiler pool locks it (std `Mutex`, so a nested acquisition on one thread
  deadlocks). The project pool (separate query checkers) is fine. Any caller that runs these on a compiler-pool
  program (CLI/tests) needs a pool like the smoke test's.
- `combine_vs_references` maps an unknown definition id to 0 like Go's map zero value.
