# lsp-actions: change tracker, organize imports, auto-import registry, code actions (`tsrs_ls`)

Wave agent `actions`, branch `lsp-actions`.

## Ported

| Go (`ts-ref/tsc/internal/…`) | Rust |
| --- | --- |
| `ls/change/tracker.go`, `trackerimpl.go`, `delete.go` (+ `trackerimpl_test.go`) | `crates/tsrs_ls/src/change/{tracker,trackerimpl,delete,trackerimpl_test}.rs` |
| `printer/changetrackerwriter.go`, `printer/syntheticfile.go` | `crates/tsrs_printer/src/{changetrackerwriter,syntheticfile}.rs` |
| `ls/organizeimports.go` | `crates/tsrs_ls/src/organizeimports.rs` |
| `ls/codeactions.go` | `crates/tsrs_ls/src/codeactions.rs`; server `handleCodeAction` wired |
| fourslash `VerifyOrganizeImports(WithRequestKind)`, `verifyOrganizeImports` | `crates/tsrs_fourslash/src/fourslash.rs` |

## Shared-file edits

- `crates/tsrs_printer`: new modules `changetrackerwriter`, `syntheticfile` (Go files listed as not ported in its lib docs);
  `textWriter::set_new_line_and_indent_size` (crate-private helper for Go's struct literal).
- `crates/tsrs_ast/src/utilities_3.rs`: `is_class_or_type_element` (utilities.go:3127).
- `crates/tsrs_ls/src/diagnostics.rs`: `get_all_diagnostics` -> `pub(crate)`.
- `crates/tsrs_lsp/src/server.rs`: `handle_code_action` calls `provide_code_actions`.

## Deviations

- Tracker: Go's `MultiMap[*SourceFile]`, `nodesWithInsertionsAtStart` and `deletedNodesInLists` maps iterate in random
  order; the port uses insertion order (edits are sorted per file before they are returned, so output is the same).
  `GetChanges` returns an `OrderedMap<String, Vec<TextEdit>>` (Go map).
- `ChangeTrackerWriter`: the print handlers and the writer share the position maps through `Rc<RefCell<…>>` (Go closes
  over the writer); map keys are node / node-list addresses (Go's interface-of-pointer keys).
- Tracker `ctx`: Go stores the formatter settings in the request context; the port keeps a `format::FormatContext`.
- codeactions: Go's `fixIdSeen` map iterates randomly; insertion order here.

## Needs from others

## Doubts
