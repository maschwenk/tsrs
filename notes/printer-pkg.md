# printer-pkg notes

New crate `crates/tsrs_printer` (Go `internal/printer`). Public API is documented at the top of `src/lib.rs`.

## Validation

- Oracle `tools/oracle/printer/` (Go `main.go` + `build.sh` + `run.py`; Rust side `crates/tsrs_printer/examples/printer_oracle.rs`):
  parse a file, print it, compare hashes. Modes: `default` (PrinterOptions{}), `nocomments` (RemoveComments),
  `synth` / `synthflags` / `synthmulti` (every statement deep-cloned through an EmitContext factory, printed without a
  source file; `synthflags` adds EFSingleLine|EFNoAsciiEscaping and the single-line writer, `synthmulti` adds
  EFMultiLine|EFStartOnNewLine|EFIndented), `omitsemi` (RemoveComments+OmitTrailingSemicolon+NeverAsciiEscape),
  `preserve` (CRLF, PreserveSourceNewlines, NeverAsciiEscape, TerminateUnterminatedLiterals, ES2021).
- Result (2026-09-30): test units (17319, 838 skipped for parse errors, 30 files where both sides panic on
  JSImportDeclaration/JSDoc like Go): 16480/16481 identical in every mode (also with jsx+force flags); libs 113/113 in
  every mode. The one mismatch is `regexInvalidUtf8WithUnicodeFlag.ts`, a non-UTF-8 file that the Rust vfs decodes with
  U+FFFD (same known divergence as the AST oracle).
- `src/printer_test.rs`: Go TestEmit table (553 cases, extracted mechanically), parenthesization tests on synthesized
  nodes, TestNameGeneration, TestOmitTrailingSemicolon, synthetic comments (checked against Go ad hoc).
  `src/utilities_test.rs`: utilities_test.go.

## Design decisions (deviations in shape, not behavior)

- `Printer` is a `&mut self` state machine returned by value from `new_printer` (Go `*Printer`). The checker's
  generated stubs `create_printer_*(emit_context: P<EmitContext>) -> P<Printer>` should return `Printer` and pass
  `Some(emit_context)`.
- `Printer::write(node, source_file, writer: &mut (dyn EmitTextWriter + 'static), source_map_generator: Option<&mut SourceMapGenerator>)`.
  The printer keeps the caller's writer for the duration of the call through a raw-pointer adapter (`borrowedWriter`,
  printer_3.rs, one `unsafe` with a SAFETY comment) because a field cannot hold the borrow without a lifetime.
- `EmitContext` is `P<EmitContext>` with `RefCell` side tables; `factory: RefCell<NodeFactory>`. Its hooks capture the
  `P<EmitContext>` (created in two steps in `new_emit_context`). Callers must not hold `factory.borrow_mut()` across
  another factory borrow (e.g. nested `f.new_x(f.new_y())` through two `borrow_mut()` calls panics).
- Name clashes from snake_casing: Go `write` (private) -> `write_`, Go `emitSourceFile` (private) -> `emit_source_file_`
  (the exported `Write`/`EmitSourceFile` keep the plain names).
- `NameGenerator` callbacks: `get_text_of_node: Rc<dyn Fn(&mut NameGenerator, P<Node>) -> String>` (receiver passed back,
  PORTING callback convention, because the printer's implementation re-enters `generate_name`),
  `is_file_level_unique_name_in_current_file: Rc<dyn Fn(&str, bool) -> bool>`. The printer's current source file and
  target live in a shared `printerTextState` so these callbacks can read them (Go closes over the printer).
- `PrinterOptions.target` is snapshotted into that state at `new_printer` and at the start of every `write`.
- `textWriter`/`singleLineStringWriter` keep only "is the last written rune white space" instead of the last string.
- `get_single_line_string_writer()` returns a fresh writer and a no-op release closure (Go pools them).

## Reduced / skipped

- Source maps: `SourceMapGenerator` is uninhabited, `source_maps_disabled` is always set during `write`; the guards of
  `emitPos`/`setSourceMapSource`/… are kept and the generator paths are `unreachable!`. `lineCharacterCache` and
  `PrintHandlers.MapSourcePosition` not ported.
- EmitContext: variable/lexical environments and visitor hooks (transform-only) not ported.
- factory.go: only generated names (`new_temp_variable`, `new_loop_variable`, `new_unique_name(_ex)`,
  `new_generated_name_for_node(_ex)`, private variants) and `new_string_literal_from_node`.
- helpers.go: only `EmitHelper`/`Priority`/`compare_emit_helpers` (the printer still emits whatever helpers a context holds).
- Not ported: changetrackerwriter.go, syntheticfile.go, emithost.go, emitresolver.go (the checker keeps its own
  `SymbolAccessibility*` types in printer_types.rs), namegenerator_test.go.
- `emitJSDocNode` panics like Go.

## Shared-file edits

- Root `Cargo.toml`: `tsrs_printer = { path = "crates/tsrs_printer" }` in `[workspace.dependencies]`.
- `crates/tsrs_ast/src/utilities_1.rs`: added `is_member_name`, `is_type_element`, `is_jsx_child`, `is_jsdoc_kind`,
  `is_var_await_using`, `is_var_using`, `is_var_let` (Go ast/utilities.go, missing in the port).
- `crates/tsrs_ast/src/utilities_2.rs`: added `is_parse_tree_node`.
- Oracle: `ts-ref/tsc/cmd/tsrs-oracle-printer/main.go` (copy of tools/oracle/printer/main.go), binary in `bin/`.

## Needs from others

- Checker (`printer_types.rs`): replace the placeholder `EmitContext`/`Printer` with `pub use tsrs_printer::{EmitContext, Printer}`
  (add `tsrs_printer.workspace = true` to tsrs_checker), and change `create_printer_*` to return `Printer`.

## Doubts

- None found by the oracle; paths it cannot reach (unique helper names / external helpers, PrintHandlers hooks,
  `id_to_symbol`, helper emission) were ported by reading only.
