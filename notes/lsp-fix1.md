# lsp-fix1: divergences in ported LS features (phase 3 fix wave)

Wave agent `fix1`, branch `lsp-fix1` (from `lsp` at c1ab60d, 3,037 / 1,092 / 417).

## Result

`tsrs-fourslash run`: 4,546 tests, 3,040 pass, 1,089 fail, 417 skip; no test of the starting pass list fails.

The 156 failures that did not stop at `feature not ported` clustered as:

| cluster | tests | cause | state |
| --- | --- | --- | --- |
| project references | 3 | tsrs_compiler never resolved `references` (mapper, parser, dts-faking host were stubs) | fixed (ported) |
| auto-import placeholder | 152 | expected auto-import items / import-adder edits / import fixes | not fixable here (autoimport port) |
| willRenameFiles | 1 | `TestGetEditsForFileRename_cssImport4`: rename reaches `workspace/willRenameFiles` (ls/file_rename.go) | `actions` wave |

The 152 auto-import tests were checked one by one against the Go test source: every one expects an item with
`Data.AutoImport` / `SortTextAutoImportSuggestions`, non-empty `AdditionalTextEdits` from the import adder, an
import-fix code action, or an auto-imports baseline. In every `Expected N exact completion items but got M` case the
difference is the number of auto-import items in the expected list (in JS files, minus a name-table entry the
missing auto-import would have shadowed, e.g. `TestCompletionsImport_default_reExport`).

## Ported

| Go (`ts-ref/tsc/internal/compiler/…`) | Rust (`crates/tsrs_compiler/src/…`) |
| --- | --- |
| `projectreferencefilemapper.go` (whole; was a no-reference stub) | `projectreferencefilemapper.rs` |
| `projectreferenceparser.go` | `projectreferenceparser.rs` (new) |
| `projectreferencedtsfakinghost.go` | `projectreferencedtsfakinghost.rs` (new) |
| `fileloader.go` `addProjectReferenceTasks` | `fileloader.rs` `add_project_reference_tasks` (free function, runs before the resolver is created, as in Go) |
| `host.go` `GetResolvedProjectReference` | `host.rs` |
| `program.go` `verifyProjectReferences`, `GetResolvedProjectReferenceFor`, `RangeResolvedProjectReferenceInChildConfig`, the redirect getters | `program.rs` |

Runner: the worker writes each failing test's full message to `target/fourslash-results/failures/<test>.txt` (and
removes it when the test no longer fails).

## Shared-file edits

- `tsrs_checker`: depends on `tsrs_tsoptions`; `program.rs` drops the placeholder `ProjectReferenceCommandLine` /
  `SourceOutputAndProjectReference` and re-exports the tsoptions types; the `Program` trait returns
  `Option<P<SourceOutputAndProjectReference>>` / `Option<P<ParsedCommandLine>>`; `checker_04.rs` (2) and
  `checker_08.rs` (1) unwrap `compiler_options()` (Go dereferences it). docs/CHECKER.md updated.
- `tsrs_tsoptions/src/parsedcommandline.rs`: `impl tsrs_module::ResolvedProjectReference for ParsedCommandLine`.
- `tsrs_compiler/src/checker_program.rs`: forwards the three project-reference methods (were `None` / `todo!()`).
- `tsrs_project/src/projectcollectionbuilder.rs` `ensure_project_tree`: Go's `if childConfig == nil { continue }`
  restored (`GetResolvedProjectReferences` entries can be nil); child config passed as `Option`.
- `tsrs_fourslash/src/runner.rs`: failure files.
- docs/LSP.md: totals row, divergence paragraph.

## Deviations

- `projectReferenceParser`: tasks are parsed sequentially in queue order on the calling thread (Go: a work group);
  deduplication and the resulting mapper are the same. Tasks live in a `Vec` and are referenced by index (Go shares
  task pointers between `subTasks` lists).
- `projectReferenceFileMapper.resolutionHost`: Go builds a new dts-faking host per call; the mapper (immutable once
  built) caches one in a `OnceLock`, so its `cachedvfs` / known-symlink caches persist across calls.
- The mapper is leaked once built (`&'static`), so the faking host and the program share it (Go: pointers).
- `rangeResolvedReferenceWorker` does not recurse below an unresolved reference (Go recurses with a nil parent over
  an empty list; same visits).
- `compilerHost.GetResolvedProjectReference` passes a leaked `ParseConfigHost` handle (same FS and directory) because
  tsoptions keeps the host for the command line's lifetime.

## Gates

- `cargo check --workspace --tests`: 0 errors, 0 warnings.
- Conformance: `tsrs-test run --suite all` 13,458 pass / 2 codes / 2 fail; `TSRS_LAZY_MEMBERS=0 … --baselines
  types,symbols` 12,779 / 12,779 (unchanged).
- `cargo test --release -p tsrs_compiler -p tsrs_project -p tsrs_lsp`: all pass.

## Remaining non-"not ported" failures (153)

- auto-import-data: TestCompletionsImport_require_addNew TestCompletionsImport_require_addToExisting
- auto-imports-baseline: TestAutoImportCrossProjectNodeModules TestAutoImportErrorMixedExportKinds TestAutoImportExportEqualsOfImportStar TestAutoImportModuleAugmentation TestAutoImportNewLine TestAutoImportNewLineWithHeaderComment TestAutoImportPackageJsonImportsHashSlashNode16 TestAutoImportPackageJsonImportsHashSlashNodenext TestAutoImportReexportOfCrossPackageAugmentation TestAutoImportSymlinkedMonorepo TestAutoImportSymlinkedMonorepoGranularUpdate TestAutoImportSymlinkedMonorepoProjectReferences TestAutoImportSymlinkedMonorepoProjectReferencesNoPkgExports TestAutoImportSymlinkedMonorepoSourceUpdate TestAutoImportTypedefMissingName TestPreferTypeOnlyAutoImports
- code-action-import-fix: TestAutoImportQuoteDetection TestCompletionsImport_fromAmbientModule TestCompletionsImport_quoteStyle TestCompletionsImport_typeOnly TestCompletionsImportFromJSXTag TestCompletionsImportModuleAugmentationWithJS TestCompletionsImportYieldExpression TestImportFixAfterIndentedImport TestImportFixBeforeIndentedImport TestImportFixBeforeIndentedImportWithCarriageReturns TestImportNameCodeFixDefaultExport6 TestImportNameCodeFixExportAsDefault
- import-adder-edits: TestAutoImportCompletionAmbientMergedModule1 TestAutoImportCompletionExportListAugmentation1 TestAutoImportCompletionExportListAugmentation2 TestAutoImportCompletionExportListAugmentation3 TestAutoImportCompletionExportListAugmentation4 TestCompletionClassMemberSnippetCrossFileNodeReuse1 TestCompletionsOverridingMethod22 TestCompletionsOverridingMethod8 TestCompletionsOverridingMethodCrash2 TestCompletionsOverridingMethodDefaultExported TestExhaustiveCaseCompletions2
- missing-auto-import-items: TestAutoImportAutomaticJsxRuntimeCrash TestAutoImportAutomaticJsxRuntimeProjectReferenceCrash TestAutoImportCompletion1 TestAutoImportCompletion2 TestAutoImportCompletion3 TestAutoImportCompletionsForArbitraryNonIdentifierExports TestAutoImportCssModule TestAutoImportDefaultPascalCase TestAutoImportDefaultPascalCaseAliasCaseInsensitive TestAutoImportDefaultPascalCaseAnonymous TestAutoImportDefaultPascalCaseAnonymousCaseInsensitive TestAutoImportDefaultPascalCaseCaseInsensitive TestAutoImportDefaultPascalCaseReexportCaseInsensitive TestAutoImportFileExcludePatterns TestAutoImportFileExcludePatterns2 TestAutoImportNodeBuiltinNodenext TestAutoImportPathsAliasesAndBarrels TestAutoImportProvider_exportMap1 TestAutoImportProvider_exportMap3 TestAutoImportProvider_exportMap4 TestAutoImportProvider_exportMap5 TestAutoImportProvider_exportMap6 TestAutoImportProvider_exportMap7 TestAutoImportProvider_exportMap8 TestAutoImportProvider_namespaceSameNameAsIntrinsic TestAutoImportProvider_wildcardExports1 TestAutoImportProvider_wildcardExports2 TestAutoImportProvider_wildcardExports3 TestAutoImportProvider6 TestAutoImportProvider7 TestAutoImportProvider8 TestAutoImportReExportFromAmbientModule TestAutoImportSameNameDefaultExported TestAutoImportSortCaseSensitivity2 TestAutoImportSpecifierExcludeRegexes TestAutoImportTypeOnlyPreferred1 TestCompletionPropertyShorthandForObjectLiteral5 TestCompletionsImport_46332 TestCompletionsImport_addToNamedWithDifferentCacheValue TestCompletionsImport_ambient TestCompletionsImport_augmentation TestCompletionsImport_compilerOptionsModule TestCompletionsImport_default_addToNamedImports TestCompletionsImport_default_addToNamespaceImport TestCompletionsImport_default_alreadyExistedWithRename TestCompletionsImport_default_didNotExistBefore TestCompletionsImport_default_exportDefaultIdentifier TestCompletionsImport_default_fromMergedDeclarations TestCompletionsImport_default_reExport TestCompletionsImport_defaultAndNamedConflict TestCompletionsImport_defaultFalsePositive TestCompletionsImport_duplicatePackages_scoped TestCompletionsImport_duplicatePackages_scopedTypes TestCompletionsImport_duplicatePackages_scopedTypesAndNotTypes TestCompletionsImport_duplicatePackages_types TestCompletionsImport_duplicatePackages_typesAndNotTypes TestCompletionsImport_exportEqualsNamespace_noDuplicate TestCompletionsImport_filteredByPackageJson_ambient TestCompletionsImport_importType TestCompletionsImport_jsModuleExportsAssignment TestCompletionsImport_jsxOpeningTagImportDefault TestCompletionsImport_mergedReExport TestCompletionsImport_multipleWithSameName TestCompletionsImport_named_addToNamedImports TestCompletionsImport_named_exportEqualsNamespace TestCompletionsImport_named_exportEqualsNamespace_merged TestCompletionsImport_named_fromMergedDeclarations TestCompletionsImport_named_namespaceImportExists TestCompletionsImport_ofAlias_preferShortPath TestCompletionsImport_packageJsonImportsPreference TestCompletionsImport_preferUpdatingExistingImport TestCompletionsImport_previousTokenIsSemicolon TestCompletionsImport_reExport_wrongName TestCompletionsImport_reExportDefault TestCompletionsImport_reExportDefault2 TestCompletionsImport_reexportTransient TestCompletionsImport_require TestCompletionsImport_sortingModuleSpecifiers TestCompletionsImport_tsx TestCompletionsImport_umdModules2_moduleExports TestCompletionsImport_uriStyleNodeCoreModules1 TestCompletionsImportBaseUrl TestCompletionsImportDefaultExportCrash2 TestCompletionsImportPathsConflict TestCompletionsImportTypeKeyword TestImportModuleSpecifierEndingAuto TestImportModuleSpecifierEndingIndex TestImportModuleSpecifierEndingJs TestImportModuleSpecifierEndingMinimal TestImportModuleSpecifierPreferenceNonRelative TestImportModuleSpecifierPreferenceProjectRelative TestImportModuleSpecifierPreferenceProjectRelativeWithPaths TestImportModuleSpecifierPreferenceRelative TestImportModuleSpecifierPreferenceShortest TestImportStatementCompletions_bareBrace TestImportStatementCompletions_bracePrefix TestImportStatementCompletions_esModuleInterop2 TestImportStatementCompletions_noSnippet TestImportStatementCompletions_pnpm1 TestImportStatementCompletions_quotes TestImportStatementCompletions_semicolons TestImportStatementCompletionUsesNamedImport TestImportSuggestionsCache_exportUndefined TestImportTypeCompletions1 TestImportTypeCompletions3 TestImportTypeCompletions4 TestImportTypeCompletions6 TestImportTypeCompletions7 TestImportTypeCompletions8 TestImportTypeCompletions9 TestResolveImportStatementCompletion
- willRenameFiles: TestGetEditsForFileRename_cssImport4

## Needs from others

- autoimport port: the 152 tests above.
- `actions` wave (ls/file_rename.go): `TestGetEditsForFileRename_cssImport4`.

## Doubts

- Project references now resolve everywhere (CLI, conformance runner, LSP). The conformance suite is unchanged; the
  LSP multi-project paths that depend on them (state-baseline tests, `@tsc` tests) still fail early in the harness,
  so the project-system side of references is only exercised by the three diagnostics tests.
