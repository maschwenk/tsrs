# lsp-fswire: fourslash harness on the in-process server

Wave agent `fswire`, branch `lsp-fswire`.

## Ported

| Go (`ts-ref/tsc/…`) | Rust |
| --- | --- |
| `internal/fourslash/fourslash.go`: `newFourslash` after the client (vfs, `lsp.ServerOptions`, shared `parseCache`, `NewLSPClient`, `SetCompilerOptionsForInferredProjects`, `initialize`, opening files, `done`), `handleServerRequest`, `sendRequest` / `sendRequestAndBaselineWorker` / `sendNotification` / `updateState`, `Configure`, `CloseFileOfMarker`, `openFile`, `FormatDocument`, `FormatSelection`, `goToMarkerInput`, `verifyBaselineDefinitions` + `VerifyBaselineGoTo{Definition,TypeDefinition,SourceDefinition}`, `VerifyBaselineHover`, `VerifyBaselineVSHover`, `renderVSContainerElement`, `appendLinesForMarkedStringWithLanguage`, `hoverContentString`, `VerifyBaselineHoverWithVerbosity`, `lookupMarkersOrGetRanges`, `Insert`, `InsertLine`, `Backspace`, `DeleteAtCaret`, `Paste`, `ReplaceLine`, `selectLine`, `selectRange`, `getSelection`, `applyTextEdits`, `Replace`, `replaceWorker`, `typeText`, `editScriptAndUpdateMarkers(Worker)`, `updatePosition`, `fromLSPRange`, `editScript`, `getOrLoadScriptInfo`, `VerifyQuickInfoAt`, `getQuickInfoAtCurrentPosition`, `verifyHoverContent`, `verifyHoverMarkdown`, `VerifyQuickInfoExists`, `VerifyNotQuickInfoExists`, `quickInfoIsEmpty`, `VerifyQuickInfoIs`, `getCurrentPositionPrefix`, `VerifyDiagnostics`, `VerifyNonSuggestionDiagnostics`, `VerifySuggestionDiagnostics`, `verifyDiagnostics`, `getDiagnostics`, `isSuggestionDiagnostic`, `VerifyBaselineNonSuggestionDiagnostics`, `fourslashDiagnostic(File)`, `toDiagnostic`, `compareDiagnostics`, `compareRelatedDiagnostics`, `VerifyNumberOfErrorsInCurrentFile`, `VerifyNoErrors`, `VerifyErrorExists{AtRange,BetweenMarkers,AfterMarker,BeforeMarker}`, `assertDeepEqual` | `crates/tsrs_fourslash/src/fourslash.rs` |
| `fourslash/baselineutil.go`: `getBaselineFor{Locations,Spans,GroupedSpans}WithFileContents`, `getAccessibleFilePaths`, `textOfFile` (ready for references / rename / implementation baselines) | `baselineutil.rs` |
| `fourslash/statebaseline.go`: `baselineRequestOrNotification`, `baselineProjectsAfterNotification`, `baselineState` (entry points only, see Deviations) | `statebaseline.rs` |
| `testutil/harnessutil/harnessutil.go`: `HarnessOptions`, `SetOptionsFromTestConfig`, `compilerOptions`, `harnessCommandLineOptions`, `getHarnessOption`, `parseHarnessOption`, `getOptionValue`, `getCommandLineOption`, `SkipUnsupportedCompilerOptions`, `failOnUnsupportedCompilerOptions` | `harnessutil.rs` |
| `testutil/tsbaseline/error_baseline.go` `GetErrorBaseline` / `iterateErrorBaseline` (pretty=false) + the diagnosticwriter functions it calls, `util.go` `removeTestPathPrefixes` | `tsbaseline.rs` |
| `testutil/lsptestutil/lspclient.go` `SetCompilerOptionsForInferredProjects` | `crates/tsrs_lsp/src/lsptestutil.rs` (module made public) |
| `ls/format.go` (whole: `toLSProtoTextEdits`, `ProvideFormatDocument`, `getFormattingEditsForMappedRange`, `ProvideFormatDocumentRange`, `ProvideFormatDocumentOnType`, `getFormattingEditsFor{Range,Document}`, `getFormattingEditsAfterKeystroke`) | `crates/tsrs_ls/src/lsformat.rs`; server handlers `handleDocument{Format,RangeFormat,OnTypeFormat}` wired in `crates/tsrs_lsp/src/server.rs` |

Runner: `tsrs-fourslash run` now runs tests in worker processes (`tsrs-fourslash worker` reads test names on stdin,
prints `name\tPASS|FAIL|SKIP\tmessage`); a worker is replaced after a crash (reported as `crash: worker exited with
status N: <last panic line>`), a 120 s timeout, or 200 tests.

Still `feature not ported: <feature> (<Method>)` (server or `ls` side missing): completions (+ JSDoc completions,
auto-import completions, `VerifyApplyCodeActionFromCompletion`), code actions / organize imports / import fixes,
references, implementation, rename, willRenameFiles, document highlights, signature help, document symbols,
workspace symbols, folding ranges, selection ranges, call hierarchy, inlay hints, linked editing, code lens, semantic
tokens, `_vs_onAutoInsert` (JSX closing tags), state baselines, content mappers (out of scope), `@tsc` command lines.

## Gates (2026-10-02)

- `tsrs-fourslash run`: 4,546 tests, 1,172 pass, 2,957 fail, 417 skip (2.2 s). 2,940 failures stop at an unported
  feature; 14 content-mapper tests (out of scope); 3 divergences (below). Per-family counts in docs/LSP.md.
- `cargo check --workspace --tests`: 0 errors, 0 warnings. `cargo test -p tsrs_lsp`: 26 pass; `cargo test -p
  tsrs_fourslash --lib`: 2 pass.

Failures among tests that use only ported features, clustered:
- harness slips found and fixed: content-mapper tests reached the server with a placeholder spawner (now fail
  early); 13 `*_js_test.go` tests that Go never builds (GOOS=js file name suffix) were generated and run (generator
  now applies `go/build` `MatchFile`).
- server/ls divergences fixed: `configFileRegistryBuilder.updateRootFilesWatch` unwrapped a nil command line (Go
  calls the nil-safe `*ParsedCommandLine` methods) and ended the server process (TestWorkspaceSymbolMultiProjectNonExistentRef).
- open: `TestRewriteRelativeImportExtensionsProjectReferences{1,2,3}`: program construction does not redirect a
  referenced project's sources (tsrs reports TS6059 / TS6307 where Go reports TS2878 / nothing).

## Shared-file edits

- `crates/tsrs_lsp/src/lib.rs`, `lsptestutil.rs`: `pub mod lsptestutil` (was `#[cfg(test)]`); `LSPClient`,
  `closeClient`, `new_lsp_client`, `send_request`, `send_notification`, `write_msg`, the handler types public; test-only
  helpers (`acknowledge_registrations`, `test_server_options`, null reader/writer) `#[cfg(test)]`; new
  `set_compiler_options_for_inferred_projects`.
- `crates/tsrs_lsp/src/server.rs`: formatting handlers call the new `tsrs_ls` functions (were `not_yet_ported`).
- `crates/tsrs_ls/src/lsformat.rs`: was the PARTIAL format.go (only `getRangeOfEnclosingComment`).
- `crates/tsrs_project/src/configfileregistrybuilder.rs`: `update_root_files_watch` nil command line (see above).
- `tools/gen-fourslash/load.go`: skip test files `go test` does not build; regenerated `tests/gen` (4,559 -> 4,546).
- `crates/tsrs_fourslash/Cargo.toml` (+ `Cargo.lock`): depends on tsrs_lsp, tsrs_project, tsrs_tsoptions, tsrs_vfs,
  tsrs_diagnostics.
- `docs/LSP.md`: Progress row 2, Fourslash section. (notes/lsp-server.md still lists the three formatting handlers
  as not ported; they are now.)

## Deviations

- Markers are shared `Arc<Marker>` / `Arc<RangeMarker>`; Go edits the shared objects in place on every edit.
  `editScriptAndUpdateMarkersWorker` replaces the edited markers by updated copies in `markers`, `marker_positions`,
  `ranges` and the ranges' `marker` links; a marker a test took out before an edit keeps the old position.
- `handleServerRequest` reads `f.userPreferences` from the client's router goroutine; the Rust handler reads a
  `Arc<Mutex<UserPreferences>>` copy that `Configure` keeps in sync.
- `scriptInfo` sharing: Go mutates the shared `*scriptInfo` and later conversions in the same function see the edit;
  the Rust methods re-read the script info after each edit at the same points.
- `assertDeepEqual` prints both values' Debug forms instead of `cmp.Diff`; `diagnosticsIgnoreOpts` is applied by
  clearing `severity`, `source`, `related_information` on both sides.
- State baselines (`// @stateBaseline: true`, 7 tests) fail in NewFourslash: `newStateBaseline` needs
  `fsbaselineutil.FSDiffer` and the project / open-file / config-registry printers (not ported).
  `baselineRequestOrNotification` is ported; `baselineState` fails the test when reached with state baselining on.
- `@tsc` command lines (`tsctests.GetFileMapWithBuild`) fail the test (needs emit).
- Content-mapper tests fail in NewFourslash (Go passes the spawner to the server; out of scope).
- `VerifyBaselineNonSuggestionDiagnostics` iterates the script infos sorted by name (Go: random map order; the
  output is sorted either way). `tsbaseline.rs` is the pretty=false subset over fourslash diagnostics; the global
  error / library / tsconfig count assertions of `iterateErrorBaseline` are not ported (fourslash diagnostics always
  have a file).
- ls/format.go `getFormattingEditsForMappedRange` / `nonOverlappingFormattingRanges`: content-mapped branch kept to
  the span-map lookup, which is statically unreachable (`SpanMap` is uninhabited).
- `toLSProtoTextEdits` returns an empty list where Go returns a nil slice (the response is a list either way).

## Needs from others

- completions / references agents: fill `verify_completions`, `get_completions`, `verify_baseline_find_all_references`,
  … in `fourslash.rs` (bodies are `server_unavailable("feature not ported: …")`); `send_request`,
  `go_to_marker_input`, `lookup_markers_or_get_ranges`, `get_baseline_for_{locations,spans,grouped_spans}_with_file_contents`,
  `annotate_content_with_tooltips`, `marker_and_item_to_json`, `assert_deep_equal` are ready.
- compiler: project-reference source redirects (the 3 open divergences).

## Doubts

- Go's test-binary panics: an unrecovered server-thread panic ends the whole Go test binary; the worker processes
  turn it into one failing test, so a crash no longer hides later results (but also cannot fail the run as a whole).
