# Language-server port log (2026-10-01–02)

These phase gates and feature-level counts record the initial language-server port. They are historical
measurements, not a report of the current checkout. See the [current language-server guide](../LSP.md)
for editor setup and validation, and the [compatibility table](../COMPATIBILITY.md#what-it-does-and-doesnt-do)
for the current capability summary.

## Progress

| phase | content | state |
| --- | --- | --- |
| 1 | transport, protocol types, session skeleton, document sync, project discovery, program update, push + pull diagnostics, hover, definition; LSP oracle | done (2026-10-02, below) |
| 2 | fourslash harness + generated tests | done: all 4,546 Go tests generated and run on the in-process server (below) |
| 3 | references, rename, completions, signature help, symbols, semantic tokens, folding, selection ranges, inlay hints, code actions, formatting | done: fourslash 4,066 / 4,546 pass; the 63 failures are 55 content-mapper tests (out of scope) and 8 `@tsc` tests (harness helper not ported yet), see Known gaps |
| 4 | watchers, multi-project, program reuse, cancellation, memory regions, editor setup | cancellation, builtin watcher, watched-file invalidation and state baselines done (robust wave, notes/lsp-robust.md); memory regions done (mem + memfix waves: RSS flat under edits, census 0 violations); watched-file / multi-project oracle sessions identical; exit behavior identical (`exit_check.py`); editor setup documented, Neovim verified headless |

### Phase 1 gates (2026-10-02, `lsp` 3c95d59+)

`tools/oracle/lsp/lsp_oracle.py`, `tsgo-ref --lsp -stdio` vs `tsrs --lsp -stdio`, sequential sessions with an edit round
(insert a character in an identifier, re-request, revert, re-request):

| session | files | diagnostic | hover | definition | typeDefinition | publishDiagnostics | initialize |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 20 conformance cases (generics, async, abstract classes, enums, mapped/conditional/template literal types, JSX, control flow, decorators, module resolution, overloads) | 20 | 87/87 | 2334/2334 | 2334/2334 | 2334/2334 | 20/20 | 20/20 |
| xstate (bench), 30 files of `packages/core` + `xstate-react` | 30 | 120/120 | 3308/3308 | 3308/3308 | 3308/3308 | 1/1 | 1/1 |

16,875 responses, all JSON-identical (1,440 + 2,308 non-null hovers; 67 + 12 diagnostic reports with items). Conformance
suite unchanged in both lazy modes (13,458 pass / 2 codes / 2 fail; `.types` / `.symbols` 12,779 / 12,779, pass lists
identical). Unit tests: `tsrs_lsproto` 33 (+ 6,869-case Go codec oracle), `tsrs_ls` 87, `tsrs_project` 105, `tsrs_lsp` 26.

### Watched files (phase 4, robust wave)

`lsp_oracle.py --closed GLOB --disk-edits N [--disk-edit-at REGEX] [--watch-client]`: after the request rounds, a
closed file is edited on disk, the client sends `workspace/didChangeWatchedFiles` (`--watch-client`, dynamic
registration) or the servers' builtin watchers (FSEvents) pick the change up, every open file is re-requested, the
edit is reverted and everything requested again. Fixtures in `tools/oracle/lsp/fixtures`:

| session | client watching | builtin watcher |
| --- | --- | --- |
| `multi` (3 files; edit `a.ts`, `b.ts` gets TS2459) | 128/128 equal | 128/128 equal |
| `refs` (app references lib; edit `lib/src/math.ts`, `app/src/main.ts` gets TS2305) | 65/65 equal | 65/65 equal |

### Fourslash

`tools/gen-fourslash` (Go, `go/types`) converts every Go fourslash test that `go test` builds
(`internal/fourslash/tests/*_test.go`, 4,546 `Test*` functions; files excluded by Go build constraints, such as the 13
tests in `*_js_test.go` files that only build for GOOS=js, are skipped like `go test` skips them) into a Rust
function in `crates/tsrs_fourslash/src/tests/gen/` plus a registry (name, file, fn, skip reason for the 386
`t.Skip("Known failing…")` tests). Regenerate with `cd tools/gen-fourslash && GOTOOLCHAIN=auto go run .`; nothing
is left untranslated. The harness (`tsrs_fourslash::fourslash`) mirrors Go's `FourslashTest` method for method and
drives the real server in-process (`tsrs_lsp::lsptestutil`, Go's `testutil/lsptestutil`: framed JSON over byte
pipes, in-memory `vfstest` file system wrapped with the bundled libs, shared parse cache, compiler options from the
test's `// @option` lines via `harnessutil.SetOptionsFromTestConfig`). Methods for language-service features that are
not ported yet fail with `feature not ported: <feature> (<Method>)`. The test-data parser is a full port (checked
against Go's `ParseTestData`, `tools/oracle/fourslash-parser`). Run with
`tsrs-fourslash run [--filter <regex>] [--include-skipped] [-j N] [-v]` (results in
`target/fourslash-results/{pass,fail,skip}.txt`, full failure messages in `failures/<name>.txt`, baseline actuals in `target/fourslash-results/local/`). Tests run
in worker processes (`tsrs-fourslash worker`, replaced after a crash, a 120 s timeout or 200 tests): the server ends
the process on an unrecovered panic in one of its threads, like a Go program, and programs are never freed. Mapping
rules and deviations: notes/lsp-fsgen.md.

| date | total | pass | fail | skip |
| --- | --- | --- | --- | --- |
| 2026-10-01 (no server yet) | 4,559 | 0 | 4,173 | 386 |
| 2026-10-02 (in-process server; hover, definition, diagnostics, formatting, edits) | 4,546 | 1,172 | 2,957 | 417 |
| 2026-10-02 (+ refs wave; completions, signature help, linked editing, closing tags) | 4,546 | 3,037 | 1,092 | 417 |
| 2026-10-02 (+ fix1: compiler project references) | 4,546 | 3,040 | 1,089 | 417 |
| 2026-10-02 (+ document/workspace symbols, semantic tokens, folding, selection ranges, inlay hints, code lens) | 4,546 | 3,321 | 808 | 417 |
| 2026-10-02 (merged: refs + compl + misc + fix1) | 4,546 | 3,325 | 804 | 417 |
| 2026-10-02 (+ change tracker, organize imports, auto-import registry, code actions, file rename) | 4,546 | 4,048 | 81 | 417 |
| 2026-10-02 (+ robust: state baselines; notes/lsp-robust.md; alone on the 3,325 base) | 4,546 | 3,342 | 787 | 417 |
| 2026-10-02 (merged: + actions + robust) | 4,546 | 4,066 | 63 | 417 |

Skips: 386 known failing in Go (registry) + 31 `SkipUnsupportedCompilerOptions` (module UMD/System, moduleResolution
node10/classic, `esModuleInterop`/`allowSyntheticDefaultImports` false, `baseUrl`, ES5 target, `alwaysStrict` false).
Failures (63, the CI gate's maximum in `.github/scripts/fourslash-gate.sh`): 55 content-mapper tests (out of scope)
and 8 tests with `@tsc` command lines (not ported yet), see Known gaps. These 63 fail before the session starts, so
with the gate green no failure is a divergence in a ported feature.

Passing tests per verify family (tests that call the method and pass / tests that call it):

| method | pass / calling tests |
| --- | --- |
| VerifyBaselineGoToDefinition | 226 / 249 |
| VerifyQuickInfoAt | 190 / 325 |
| FormatDocument | 157 / 177 |
| VerifyBaselineHover | 142 / 148 |
| VerifyCurrentLineContent | 133 / 136 |
| Insert (typing + on-type formatting) | 112 / 226 |
| VerifyCurrentFileContent | 94 / 112 |
| VerifyBaselineGoToSourceDefinition | 89 / 89 |
| VerifyBaselineHoverWithVerbosity | 56 / 56 |
| VerifyNoErrors | 48 / 109 |
| VerifyNumberOfErrorsInCurrentFile | 41 / 49 |
| VerifySuggestionDiagnostics | 39 / 42 |
| DeleteAtCaret | 25 / 26 |
| VerifyBaselineNonSuggestionDiagnostics | 24 / 30 |
| FormatSelection | 21 / 36 |
| VerifyBaselineGoToTypeDefinition | 20 / 24 |
| VerifyQuickInfoExists | 19 / 22 |
| InsertLine | 18 / 25 |
| VerifyQuickInfoIs | 15 / 41 |
| VerifyErrorExistsBetweenMarkers | 11 / 15 |
| VerifyNonSuggestionDiagnostics | 9 / 16 |
| VerifyNotQuickInfoExists | 6 / 9 |
| VerifyBaselineVSHover | 5 / 5 |
| Backspace / Paste / ReplaceLine | 4 / 18, 4 / 4, 2 / 11 |
| VerifyBaselineDocumentSymbol | 83 / 85 |
| VerifyBaselineInlayHints | 64 / 65 |
| VerifySemanticTokens | 46 / 46 |
| VerifyBaselineSelectionRanges | 36 / 37 |
| VerifyOutliningSpans / VerifyFoldingRangeLines | 32 / 33, 2 / 6 |
| VerifyWorkspaceSymbol / VerifyBaselineWorkspaceSymbol | 14 / 15, 0 / 1 |
| VerifyBaselineCodeLens | 7 / 13 |
| VerifyCompletions / GetCompletions | 986 / 1,143, 8 / 9 |
| VerifyOrganizeImports / WithRequestKind | 92 / 92, 1 / 1 |
| VerifyCodeFix | 136 / 152 |
| VerifyCodeFixAvailable / NotAvailable / AvailableExact | 18 / 25, 85 / 89, 1 / 1 |
| VerifyCodeFixAll / VerifySourceFixAll | 17 / 22, 1 / 1 |
| VerifyImportFixAtPosition | 143 / 171 |
| VerifyImportFixModuleSpecifiers | 52 / 64 |
| VerifyRangeAfterCodeFix | 4 / 70 (66 skipped in Go) |
| BaselineAutoImportsCompletions | 36 / 39 |
| VerifyApplyCodeActionFromCompletion | 56 / 70 |
| VerifyWillRenameFilesEdits / VerifyRename | 31 / 33, 4 / 7 |

(These counts are historical and understate the current run, in which every calling test that does not pass is
skipped, a content-mapper test or an `@tsc` test.) Earlier divergences in ported features:
`TestRewriteRelativeImportExtensionsProjectReferences{1,2,3}` (diagnostic baselines) were tsrs_compiler not resolving
project references; fixed by porting them (fix1).

After the completions wave (fix1, 2026-10-02): of the 1,089 failures, 153 do not stop at an unported feature, and
none of them is a divergence in ported code: 152 expect auto-import data that the placeholder registry cannot
produce (auto-import completion items, `AdditionalTextEdits` from the import adder in class member / override /
exhaustive case snippets, `VerifyApplyCodeActionFromCompletion` import fixes, `BaselineAutoImportsCompletions`; in
JS files a missing auto-import entry also lets the name-table entry of the same name through), and 1 reaches
`workspace/willRenameFiles` from rename (`TestGetEditsForFileRename_cssImport4`). `TestCodeLensOnFunctionAcrossProjects1` (code lens, misc wave) had the same root cause and passes after the merge.
Full failure messages: `target/fourslash-results/failures/<test>.txt`. The symbol / folding / selection / inlay hint /
code lens failures not counted as passing are content-mapper, state-baseline or Go-skipped tests.

After the actions wave (2026-10-02): every one of the 81 failures stops at an out-of-scope
feature (55 content mappers, 20 state baselines, 6 `@tsc` command lines); every non-passing test of the code action,
organize imports, auto-import and file rename families above is skipped in Go or a content-mapper test. The real
auto-import registry also fixed the 152 auto-import divergences listed above, and `Program.comparePathsOptions` is now
Go's zero value (tsrs_compiler set a current directory, which made `IsGlobalTypingsFile` true for every `.d.ts` when no
typings location is configured and hid them from auto-imports); the conformance suite is unchanged (13,458 pass both
before and after; lazy-off `.types` / `.symbols` 12,779 / 12,779).

