# lsp-misc: symbols, semantic tokens, folding, selection ranges, inlay hints, code lens

Wave agent `misc` (phase 3), branch `lsp-misc`.

## Ported

| Go (`ts-ref/tsc/internal/…`) | Rust |
| --- | --- |
| `ls/symbols.go` (whole: document symbols, workspace symbols) | `crates/tsrs_ls/src/symbols.rs` (replaces the refs wave's partial file) |
| `ls/folding.go` | `crates/tsrs_ls/src/folding.rs` |
| `ls/selectionranges.go` | `crates/tsrs_ls/src/selectionranges.rs` |
| `ls/semantictokens.go` (incl. `SemanticTokensLegend`, moved out of `tsrs_lsp/src/lsconsts.rs`) | `crates/tsrs_ls/src/semantictokens.rs` |
| `ls/inlay_hints.go` | `crates/tsrs_ls/src/inlay_hints.rs` |
| `ls/codelens.go` (+ `ResolveCodeLens` through the refs wave's `provideSymbolsAndEntriesAtPosition` / `provideReferencesFromData` / `provideImplementationsFromData`) | `crates/tsrs_ls/src/codelens.rs` |
| `ast/ast.go` `SourceFile.GetDeclarationMap`, `computeDeclarationMap`, `GetDeclarationName` | `crates/tsrs_ast/src/ast.rs` |
| `lsp/server.go` `handleDocumentSymbol`, `handleWorkspaceSymbol`, `handleFoldingRange`, `handleSelectionRange`, `handleSemanticTokensFull`, `handleSemanticTokensRange`, `handleInlayHint`, `handleCodeLens`, `handleCodeLensResolve` | `crates/tsrs_lsp/src/server.rs` (were `not_yet_ported`) |
| `fourslash/fourslash.go` `VerifyBaselineDocumentSymbol` (+ `writeDocumentSymbolDetails`, `collectDocumentSymbolSpans`, `documentSpanKey`), `VerifyBaselineWorkspaceSymbol`, `VerifyWorkspaceSymbol` (+ `verifyExactSymbols`, `verifyIncludesSymbols`), `VerifyOutliningSpans`, `VerifyFoldingRangeLines`, `VerifyBaselineSelectionRanges`, `VerifyBaselineInlayHints`, `VerifyBaselineCodeLens`; `fourslash/semantictokens.go` `VerifySemanticTokens` | `crates/tsrs_fourslash/src/fourslash.rs`, `semantictokens.rs` |

Not ported: `ls/organizeimports.go` (scope change: the `actions` wave owns `ls/change`, organize imports, code
actions and auto-imports; nothing was started here).

## Gates (2026-10-02, after merging `lsp` with the refs wave)

- `tsrs-fourslash run`: 4,546 tests, 2,238 pass (lsp: 1,954), 1,891 fail, 417 skip; no test of the `lsp` pass list
  fails.
- Per family (pass / calling tests; every other failure is a content-mapper, state-baseline or Go-skipped test,
  except the one divergence below): VerifyBaselineDocumentSymbol 83 / 85, VerifySemanticTokens 46 / 46,
  VerifyOutliningSpans 32 / 33, VerifyFoldingRangeLines 2 / 6, VerifyBaselineSelectionRanges 36 / 37,
  VerifyBaselineInlayHints 64 / 65, VerifyBaselineCodeLens 7 / 13, VerifyWorkspaceSymbol 14 / 15,
  VerifyBaselineWorkspaceSymbol 0 / 1 (state baseline).
- Divergence: `TestCodeLensOnFunctionAcrossProjects1` (1 reference instead of 2): project b imports
  `../../a/dist/foo.js`, which tsrs does not redirect to the referenced project's source `a/src/foo.ts`
  ("Cannot find module '../../a/dist/foo.js'" in b), so b's reference is never found. Same compiler gap as the
  `TestRewriteRelativeImportExtensionsProjectReferences{1,2,3}` divergences (project-reference output -> source
  redirects); not in this wave's files.
- `cargo check --workspace --tests`: 0 errors, 0 warnings.

## Shared-file edits

- `crates/tsrs_ast/src/ast.rs`: `SourceFile.declaration_map` (`OnceLock`, Go `declarationMapMu` + `declarationMap`),
  `get_declaration_map`, `compute_declaration_map`, `get_declaration_name` (ast.go:2973/2982/3092).
- `crates/tsrs_lsp/src/lsconsts.rs`: semantic tokens legend removed (now `tsrs_ls::semantic_tokens_legend`);
  `server.rs` calls the `tsrs_ls` one.
- `crates/tsrs_fourslash/src/runner.rs`: the full message of each failure is written to
  `target/fourslash-results/failures/<name>.txt` (fail.txt keeps the first line); passing / skipped tests remove
  their stale file.

## Deviations

- `mergeExpandos` / `mergeChildren` mutate `*lsproto.DocumentSymbol` values that can be shared between several
  parents' child lists; the port builds an `Rc<RefCell<…>>` tree with the same sharing (`DocSym`) and converts it to
  `lsproto::DocumentSymbol` values when the response is built.
- Workspace symbols: Go iterates a map of source files and each file's declaration map in random order; tsrs uses
  `FxHashMap`s. The matches are sorted by `compareDeclarationInfos` (a total order except for equal name, file and
  position), so the response is the same.
- `getMatchScore`: Go `unicode.IsUpper` is approximated by `char::is_uppercase` outside ASCII (Rust's Uppercase
  property also includes `Other_Uppercase`, e.g. circled letters); `unicode.ToLower` is `stringutil::unicode_to_lower`.
- `getLineEndOfPosition` (folding), `supplementalFileIndex` / `sourceFileForSupplementalFileIndex` (code lens) and
  `strPtrTo` (workspace symbols) live in `completions.go`; this wave keeps private copies (`folding.rs`,
  `codelens.rs`, inline in `symbols.rs`) so it does not touch `completions.rs`. Fold them into `completions.rs` when
  merging.
- Selection ranges: Go walks `current.VisitEachChild(tempVisitor)` with closures over the builder state; the Rust
  visitor's callbacks are `'static`, so the children are collected in VisitEachChild order (nodes through `Visit`,
  lists through the `VisitNodes` hook) and processed afterwards (same pattern as `format/span.rs`).
- Semantic tokens: only the token types / modifier bits this file produces are named constants (the rest are
  positions in `TOKEN_TYPES` / `TOKEN_MODIFIERS`), to keep the crate free of dead-code warnings.
- `handleWorkspaceSymbol` / `handleCodeLensResolve`: after a recovered panic Go returns the zero response (null);
  workspace symbols return the default union (null), code lens resolve returns an empty `CodeLens` (the Rust
  response type is not nullable). The request already got the recover's error response in both cases.
- Code lens titles use English only (`locale.FromContext` is not ported).
- `VerifyOutliningSpans` sorts the test data's range list in place, as Go's `slices.SortFunc(f.Ranges(), …)` does.
- `VerifyBaselineSelectionRanges` compares rune indices with byte offsets, as Go does.

## Needs from others

- compiler: project-reference output -> source redirects (`TestCodeLensOnFunctionAcrossProjects1`, and the 3
  RewriteRelativeImportExtensions divergences).
- completions wave: `getLineEndOfPosition`, `supplementalFileIndex`, `sourceFileForSupplementalFileIndex`,
  `strPtrTo` from completions.rs can replace the private copies listed above.

## Doubts

- The `DocSym` sharing reproduces Go's pointer aliasing in `mergeExpandos`; no test exercises a child shared by
  two expando targets (same-named class/function/variable in one scope), so that path is unverified against Go.
