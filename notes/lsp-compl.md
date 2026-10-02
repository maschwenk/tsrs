# lsp-compl: completions (`tsrs_ls`)

Wave agent `compl`, branch `lsp-compl`.

## Ported

| Go (`ts-ref/tsc/internal/…`) | Rust (`crates/…`) |
| --- | --- |
| `ls/completions.go` (whole, incl. `ResolveCompletionItem`, the snippet printer, exhaustive case snippets) | `tsrs_ls/src/completions.rs` (Go lines 1-1818), `completions_2.rs` (1820-3466), `completions_3.rs` (3468-5487), `completions_4.rs` (5489-6905) |
| `ls/string_completions.go` (+ `string_completions_test.go`) | `tsrs_ls/src/string_completions.rs` (+ `string_completions_test.rs`) |
| `ls/jsdoc_snippet.go` | `tsrs_ls/src/jsdoc_snippet.rs` |
| `ls/autoinsert.go`, `ls/linkedediting.go` | `tsrs_ls/src/autoinsert.rs`, `linkedediting.rs` |
| `ls/codeactions_missingmemberfixer.go` (whole; class member snippets use it) | `tsrs_ls/src/codeactions_missingmemberfixer.rs` |
| PARTIAL `ls/signaturehelp.go` (invocation types, `getImmediatelyContainingArgumentInfo` and what it calls) | `tsrs_ls/src/signaturehelp.rs` |
| PARTIAL `ls/change` (`Tracker` fields `NewTracker` sets, `NewTracker`, `GetFormatCodeSettingsForWriting`) | `tsrs_ls/src/change/{mod,tracker,trackerimpl}.rs` |
| autoimport PLACEHOLDER additions: `Export`/`ExportID`/`ExportSyntax`, `Fix` (+ `Edits` stub), `FixAndExport`, `View.GetCompletions` (empty), `ImportAdder` (interface + inert adder), `GetImportKindForImportStatement`/`getImportKind`, `TypeToAutoImportableTypeNode`, `TypeNodeToAutoImportableTypeNode`, `TryGetAutoImportableReferenceFromTypeNode`, `getNameForExportedSymbol`, `replaceFirstIdentifierOfEntityName`, `getDefaultLikeExportNameFromDeclaration` | `tsrs_ls/src/autoimport/{export,fix,import_adder,view}.rs` |
| `printer/changetrackerwriter.go`, `printer/syntheticfile.go` | `tsrs_printer/src/changetrackerwriter.rs`, `syntheticfile.rs` |

Public API: `LanguageService::{provide_completion, get_completions_at_position, resolve_completion_item,
provide_on_auto_insert, provide_linked_editing_range}`, `tsrs_ls::{CompletionItem, CompletionList,
compare_completion_entries, SortText, SORT_TEXT_*, deprecate_sort_text, object_literal_property_sort_text, sort_below,
SOURCE_*, COMPLETION_TRIGGER_CHARACTERS}`. `tsrs_fourslash` now uses these (`ls_shim.rs` deleted).

Auto-imports: the registry placeholder is never prepared, so every completion that collects auto-imports
(`collectAutoImports`: global completions and import-statement completions unless
`includeCompletionsForModuleExports` is false or the file name is dynamic), class member snippets
(`createImportAdder`) and exhaustive switch-case snippets (non-dynamic files) return `ErrNeedsAutoImports`, exactly
Go's "registry not prepared" path. The server must then do what Go does (`GetLanguageServiceWithAutoImports` and
retry); with the placeholder the retry returns the same error, where Go panics.

## Tests

- `string_completions_test.rs`: the Go unit test (1/1).
- `completions_smoke_test.rs` (not a Go test): in-memory program from `testdata/completions_smoke/proj`, 44
  responses compared with `tsgo-ref --lsp -stdio` recordings (`tools/oracle/completions/record.py`): rich client
  (snippets, insert/replace, item defaults, label details), minimal client, non-default preferences (object literal
  method snippets formatted through the snippet printer + formatter, JSX brace style), member / optional chain / enum
  / namespace / lib members (deprecated), object literal members, string literal (union, element access, call
  argument), type position, `this.` members, class body (keywords + members), labels, JSDoc tags, JSDoc snippet
  (`/**`), import paths, import specifiers, JSX tag / attribute / closing tag, JS name-table entries, resolve
  (member doc + deprecated tag, literal, keyword, snippet), linked editing, VS auto insert; plus the
  registry-not-prepared errors. All identical (item lists sorted with `compare_completion_entries`, as clients and
  fourslash do, because Go returns some lists in map order).

## Shared-file edits

- `tsrs_ast/src/ast.rs`: `SourceFile.name_table` (`OnceLock`) + `get_name_table` (ast.go:2857; insertion-ordered
  map where Go's is random).
- `tsrs_ast/src/utilities_3.rs` (appended, origin markers): `node_has_kind`, `get_type_annotation_node`,
  `is_object_type_declaration`, `is_class_or_type_element`, `is_type_keyword_token`,
  `try_get_import_from_module_specifier`, `is_template_literal_token`.
- `tsrs_checker/src/checker_06.rs`: `try_get_this_type_at_ex_exported` takes `container: Option<P<Node>>` (Go passes
  nil; completions do).
- `tsrs_printer`: new `changetrackerwriter.rs`, `syntheticfile.rs`; `textWriter::new_with` (crate-internal
  constructor); lib.rs module list and doc line.
- `tsrs_ls/src/hover.rs`: `get_quick_info_and_documentation_for_symbol` `pub(crate)` (completion resolve calls it).
- `tsrs_ls/src/lib.rs`: modules and re-exports.
- `tsrs_fourslash`: `tests/prelude.rs`, `tests/util.rs` use `tsrs_ls` as `ls`; `ls_shim.rs` removed.

## Deviations

- Go maps iterated by completions keep insertion order (`uniques`, `moduleCompletionNameAndKindSet.names`, the name
  table, `localsContainer.Locals()`); Go's order is random and clients sort.
- `CompletionItem` holds the `lsproto::CompletionItem` by value (Go embeds a never-nil pointer); `toLSP`'s nil checks
  are gone.
- getCompletionData's closures share mutable locals: they are methods of a private `getCompletionDataState` that
  receive the checker (and the language service) as parameters.
- `collections.Set` passed by value (`symbolCanBeReferencedAtTypeLocation`'s `seenModules`) is
  `Option<Rc<RefCell<FxHashSet>>>`, reproducing Go's semantics (copies share the map once it exists; adding to a
  nil-map copy allocates a map only that copy sees).
- `missingMemberFixer<'a>` borrows the tracker, checker and import adder (no lifetime-free way to hold `&mut
  Checker`), like lscore's `SourceDefResolver<'a>`.
- The snippet printer's base writer lives inside its escaping writer (Go keeps two pointers to one
  `ChangeTrackerWriter`). `ChangeTrackerWriter`'s position maps are shared with its print handlers through an `Rc`.
- Format settings: Go carries them as `context.Context` values (`format.WithFormatCodeSettings`); here a
  `format::FormatContext` is stored as a value of the request `Context` (`completions::with_format_code_settings`,
  `format_context`).
- `quotePropertyName`'s `unicode.IsDigit` (Nd): ASCII exact; non-ASCII uses `char::is_numeric` (Nd+Nl+No; no Nd
  table is ported).
- `core.StringifyJson(number)` is `tsrs_core::json::marshal(Value::Number)` (Go's float formatting; NaN/Inf give "").
- autoimport placeholder: `Fix::edits` returns no edits / not ok (auto-import data is never produced);
  `new_import_adder` returns an inert adder whose `add_*` methods are unreachable while no prepared view exists.
- `ImportAttributes.attributes`, `JSDoc.comment`, `UnionTypeNode.types` are non-optional in tsrs_ast; Go's nil
  checks on them are dropped.

## Needs from others

- autoimport port: registry/view/fix/import adder (the placeholder decides the `ErrNeedsAutoImports` path above).

## Doubts

- Go's `getCompletionEntriesFromSymbols` returns `uniqueNames` built from a map; later uniqueness checks only use
  membership, so order is irrelevant.
