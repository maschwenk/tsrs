# lsp-actions: change tracker, organize imports, auto-import registry, code actions, file rename (`tsrs_ls`)

Wave agent `actions`, branch `lsp-actions` (merged `lsp` twice on the lead's request: compl wave, then misc + fix1).

## Ported

| Go (`ts-ref/tsc/internal/…`) | Rust |
| --- | --- |
| `ls/change/tracker.go`, `trackerimpl.go`, `delete.go` (+ `trackerimpl_test.go`) | `crates/tsrs_ls/src/change/{tracker,trackerimpl,delete,trackerimpl_test}.rs` (replaces compl's partial tracker; compl's callers compile unchanged) |
| `printer/changetrackerwriter.go`, `printer/syntheticfile.go` | `crates/tsrs_printer/src/{changetrackerwriter,syntheticfile}.rs` (identical to lsp-compl's copies) |
| `ls/organizeimports.go` | `crates/tsrs_ls/src/organizeimports.rs` |
| `ls/autoimport/{registry,index,extract,export,specifiers,aliasresolver,fix,import_adder,view,util}.go` | `crates/tsrs_ls/src/autoimport/*.rs` (replaces the placeholder; same public API as the placeholder and compl's additions) |
| `ls/autoimport/{index,util,aliasresolver_crash}_test.go` | `autoimport/{index_test,util_test,aliasresolver_crash_test}.rs` (9 tests; `registry_test.go` needs a project session: not ported) |
| `ls/codeactions.go` | `crates/tsrs_ls/src/codeactions.rs`; server `textDocument/codeAction` wired (Go has no `codeAction/resolve`) |
| `ls/codeactions_importfixes.go` | `codeactions_importfixes.rs` |
| `ls/codeactions_fixclassincorrectlyimplementsinterface.go` | `codeactions_fixclassincorrectlyimplementsinterface.rs` (uses compl's `codeactions_missingmemberfixer.rs`) |
| `ls/codeactions_fixmissingtypeannotation.go` | `codeactions_fixmissingtypeannotation.rs` |
| `ls/file_rename.go` | `file_rename.rs`; server `workspace/willRenameFiles` worker completed (also reached from rename's file branch) |
| `module/resolver.go` entrypoints (`Ending`, `ResolvedEntrypoint`, `GetEntrypointsFromPackageJsonInfo`, `createResolvedEntrypointHandlingSymlink`, `loadEntrypointsFromExportMap`, `getMatchedStarForPatternEntrypoint`) | appended to `crates/tsrs_module/src/resolver.rs` |
| `modulespecifiers/util.go` `ProcessEntrypointEnding` | appended to `crates/tsrs_modulespecifiers/src/util.rs` |
| `vfs/wrapvfs` | `crates/tsrs_vfs/src/wrapvfs.rs` |
| `project/session.go` `logCacheStats` auto-import section | `crates/tsrs_project/src/session.rs` |
| fourslash: `VerifyOrganizeImports(WithRequestKind)`, `VerifyCodeFix`, `VerifyRangeAfterCodeFix` (+ `getCodeActionEditsForActiveFile`), `VerifyCodeFixAvailable`, `VerifyCodeFixNotAvailable`, `VerifyCodeFixAvailableExact`, `VerifyCodeFixAll`, `VerifySourceFixAll`, `getCodeFixActions`, `getAllQuickFixActions`, `updateTextRangeForTextEdits`, `applyEditsToContent`, `VerifyImportFixAtPosition`, `VerifyImportFixModuleSpecifiers`, `extractModuleSpecifier`, `WillRenameFiles`, `willRenameFilesWorker`, `VerifyWillRenameFilesEdits`, helpers `updatePositionForTextEdit`, `removeWhitespace`, `assertValidTextRange`, `selectCodeFixDiagnostic` | `crates/tsrs_fourslash/src/fourslash.rs` |

`BaselineAutoImportsCompletions` and `VerifyApplyCodeActionFromCompletion` were already ported by compl; with the real
registry they pass (36 / 39 and 56 / 70; the rest are content-mapper or Go-skipped tests).

## Gates (2026-10-02)

- `tsrs-fourslash run`: 4,546 tests, 4,048 pass, 81 fail, 417 skip (was 3,325 on `lsp`); no test of the `lsp` pass list
  fails. All 81 failures stop at out-of-scope features (55 content mappers, 20 state baselines, 6 `@tsc`).
  Per family: see docs/LSP.md (organize imports 93 / 93, code fix 136 / 152, import fix 143 / 171, module specifiers
  52 / 64, willRenameFiles 31 / 33, completions 986 / 1,143; every non-passing test in these families is Go-skipped or
  a content-mapper test).
- `cargo check --workspace --tests`: 0 errors, 0 warnings. `cargo test` for tsrs_ls, tsrs_project, tsrs_lsp,
  tsrs_printer, tsrs_projectutil, tsrs_module, tsrs_modulespecifiers, tsrs_vfs: all pass.
- Conformance (`tsrs-test run --suite all`): 13,458 pass, pass list identical to `lsp`'s; lazy-off `.types` /
  `.symbols` 12,779 / 12,779.

## Shared-file edits

- New crate `crates/tsrs_projectutil` (Go `project/dirty` + `project/logging`, moved from `tsrs_project` with
  `git mv`); `tsrs_project/src/lib.rs` re-exports them (`pub use tsrs_projectutil::{dirty, logging}`), root
  `Cargo.toml` workspace dependency, `tsrs_project` / `tsrs_ls` `Cargo.toml`.
- `crates/tsrs_printer`: `changetrackerwriter.rs`, `syntheticfile.rs`, `lib.rs`, `textwriter.rs` copied from
  lsp-compl (identical content, so the branches merge cleanly).
- `crates/tsrs_ast/src/utilities_3.rs`: lsp-compl's version (contains `is_class_or_type_element`);
  `utilities_2.rs`: `is_require_variable_statement` (utilities.go:2811).
- `crates/tsrs_compiler/src/program.rs`: `Program.compare_paths_options` is `ComparePathsOptions::default()` (Go's
  `NewProgram` never sets `comparePathsOptions`; the port set the host's case sensitivity and current directory, so
  `IsGlobalTypingsFile("x.d.ts")` was true whenever no typings location is configured: every `.d.ts` was dropped from
  auto-import project buckets). Conformance unchanged.
- `crates/tsrs_module/src/resolver.rs`, `crates/tsrs_modulespecifiers/src/util.rs`, `crates/tsrs_vfs/src/{lib,wrapvfs}.rs`:
  appended ports (above).
- `crates/tsrs_ls/src/completions.rs`, `completions_4.rs`: `autoimport::Fix { auto_import_fix: x, ..Default::default() }`
  (the real `Fix` has Go's other fields; Go's literal omits them).
- `crates/tsrs_ls/src/languageservice.rs`: `host` field `pub(crate)` (file_rename reads `host.FileExists`);
  `diagnostics.rs`: `get_all_diagnostics` `pub(crate)`.
- `crates/tsrs_lsp/src/server.rs`: `handle_code_action`, `handle_will_rename_files_worker`.
- `crates/tsrs_project/src/session.rs`: auto-import cache statistics in `log_cache_stats`.
- `docs/LSP.md`: crate map, progress row 3, fourslash table and families, known gaps.

## Deviations

- Go maps iterated in random order are insertion-ordered here (tracker change map, deleted nodes in lists, nodes with
  insertions at start, code-fix `fixIdSeen`, import adder `addToExisting` / `newImports`, `GetCompletions` groups,
  willRenameFiles change map). Edits are sorted per file before they are returned, so responses match.
- `ChangeTrackerWriter` (compl's copy): print handlers share the position maps with the writer through
  `Rc<RefCell<…>>` and key them by address.
- `change::Tracker` keeps Go's `*ast.NodeFactory` as `node_factory` (Deref target) and the formatter settings as a
  `format::FormatContext` value in its `Context`, as compl's partial tracker did.
- autoimport registry: Go's `wg.Go` parallel phases (discovery, extraction, bucket build, project bucket extraction)
  run sequentially; the project bucket's checker pool holds one checker. `*collections.Set` nil semantics are
  `Option<Set<String>>` with Go's nil rules (`Equals`, `IsSubsetOf`, `Coalesce`). The clone host is turned into a
  `'static` reference for the duration of `clone_registry` (`assume_static`, one `unsafe`): module resolvers require a
  `'static` host and are leaked with the alias resolvers and checkers built on them; nothing in the finished registry
  points at them. `clone_registry` takes the concrete `LogTree` (was generic in the placeholder).
- `autoimport::View` keeps the request's checker as `NonNull<Checker>` (Go shares the `*checker.Checker` between the
  caller, the view and the import adder; callers such as exhaustive-case snippets keep using the checker while the
  adder calls into it). `View::checker()` is the one `unsafe` deref, used one call at a time on the request thread.
  View caches (`allowedEndings`, `existingImports`, `shouldUseRequire`) are `OnceCell`s so `get_completions(&self)`
  keeps compl's signature.
- `Index` entries are `Arc<Export>`; `ModuleID` is a newtype; `FixAndExport` holds `Arc<Fix>` / `Arc<Export>`.
- `extractFromSymbol`'s `slices.Delete(allExports, -1, 0)` panic for an ignored symbol that is not in the export list is
  kept as an explicit panic; Go's nil-map lookups that are then dereferenced (`Members[name]`, `specifierCache[path]`,
  `UpdateNodeModulesBucket` on a nil index) panic with messages.
- `unicode.IsLower` uses Rust's `char::is_lowercase` outside ASCII (Lowercase vs Go's Ll: differs only for a few
  modifier letters); `typeToStringForDiag` truncation replaces each byte of a cut multi-byte character with U+FFFD
  (json/v2's behavior when the description is serialized).
- fixclass `addChanges`: the missing member fixer borrows the tracker shared, so a fixer is created per member
  creation (Go keeps one); the fixer has no state, so creations and insertions happen in Go's order.
- `tryCodeAction` (isolated declarations) never sets an import adder, like Go.
- fourslash: `VerifyCodeFixAvailable(t, nil)` is generated as `&[]`; no Go test passes a non-nil empty slice, so an
  empty slice takes Go's nil branch. `VerifyImportFixAtPosition` re-reads the range marker and the script length after
  each edit (Go mutates the shared `*RangeMarker` / `*scriptInfo`).

## Needs from others

- Lead: when landing, `crates/tsrs_ls/src/autoimport/*` and `crates/tsrs_ls/src/change/*` replace the placeholder /
  partial versions on `lsp` (this branch already merged them and compiles every caller).

## Doubts

- `View::checker()` and `assume_static` are the two `unsafe` uses; both mirror Go pointer sharing and are scoped as
  their SAFETY comments say. A future change that keeps a `View` past its request's checker lease would be unsound.
- `registry_test.go` (lifecycle / granular node_modules updates) is not ported (needs the project session fixture
  `autoimporttestutil`); node_modules granular updates are only exercised by fourslash.
