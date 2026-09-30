//! Port of `internal/printer` (TypeScript 7): the AST -> text emitter the checker uses to render types, symbols and
//! signatures in diagnostics (via its node builder), plus the writers and the EmitContext side tables it reads.
//!
//! Public API (Go name -> Rust):
//! - `PrinterOptions` (all Go fields, snake_case; `Default`), `PrintHandlers` (`Default`; hooks are
//!   `Option<Box<dyn FnMut(Option<P<Node>>)>>` / `Option<Box<dyn FnMut(Option<P<NodeList>>)>>`, `has_global_name:
//!   Option<Rc<dyn Fn(&str) -> bool>>`; Go's `MapSourcePosition` is not ported).
//! - `new_printer(options: PrinterOptions, handlers: PrintHandlers, emit_context: Option<P<EmitContext>>) -> Printer`
//!   (Go `NewPrinter`; `None` creates a fresh context). `Printer` is a `&mut self` state machine, returned by value.
//!   - `Printer::write(&mut self, node: P<Node>, source_file: Option<P<SourceFile>>, writer: &mut (dyn EmitTextWriter + 'static),
//!     source_map_generator: Option<&mut SourceMapGenerator>)` (pass `None`; `SourceMapGenerator` is uninhabited).
//!     A `Box<dyn EmitTextWriter>` can be passed as `&mut writer` or `&mut *writer`.
//!   - `Printer::emit(&mut self, node: P<Node>, source_file: Option<P<SourceFile>>) -> String`,
//!     `Printer::emit_source_file(&mut self, source_file: P<SourceFile>) -> String`.
//!   - pub fields `options: PrinterOptions`, `print_handlers: PrintHandlers` (Go's embedded `PrintHandlers`),
//!     `id_to_symbol: Option<FxHashMap<P<Node>, P<Symbol>>>` (Go `IdToSymbol`).
//! - Writers: `trait EmitTextWriter` (Go interface, snake_case methods: `write`, `write_keyword`, …, `string() -> String`,
//!   `clear`, `get_text_pos/get_line/get_indent -> i32`, `get_column -> UTF16Offset`, …; `Box<W>` implements it too),
//!   `new_text_writer(new_line: &str, indent_size: usize) -> Box<dyn EmitTextWriter>` (0 = default 4),
//!   `get_single_line_string_writer() -> (Box<dyn EmitTextWriter>, impl FnOnce())` (the closure is Go's release func),
//!   `get_default_indent_size() -> usize`.
//! - `EmitContext`, always handled as `P<EmitContext>` (interior mutability, `&self` methods):
//!   `new_emit_context() -> P<EmitContext>` / `EmitContext::new()`, `get_emit_context() -> (P<EmitContext>, impl FnOnce())`,
//!   field `factory: RefCell<NodeFactory>` (Go `Factory`; `NodeFactory` derefs to `tsrs_ast::NodeFactory`, and
//!   `as_node_factory()` returns it; created nodes get `NodeFlags::Synthesized`, updates/clones record `original`.
//!   Do not hold a `borrow_mut()` across another factory call), `emit_flags(node) -> EmitFlags`,
//!   `set_emit_flags(node, EmitFlags)`, `add_emit_flags(node, EmitFlags)`, `set_original(node, original)`,
//!   `set_original_ex(node, original, allow_overwrite)`, `unset_original`, `original(node) -> Option<P<Node>>`,
//!   `most_original(node) -> P<Node>`, `parse_node(Option<P<Node>>) -> Option<P<Node>>`, `comment_range`,
//!   `set_comment_range(node, TextRange)`, `assign_comment_range(to, from)`, `source_map_range`/`set_source_map_range`/
//!   `assign_source_map_range`/`assign_comment_and_source_map_ranges`, `add_synthetic_leading_comment(node, kind: Kind,
//!   text: &str, has_trailing_new_line: bool) -> P<Node>`, `add_synthetic_trailing_comment(..)`, `set_/get_synthetic_*_comments`
//!   (`Vec<SynthesizedComment>`), `set_type_node`/`get_type_node`, `snippet_element`/`set_snippet_element`, `text_source`,
//!   auto-generated names (`has_auto_generate_info`, `get_auto_generate_info`, `get_node_for_generated_name`),
//!   `is_file_level_unique_name`, helpers bookkeeping (`get_emit_helpers`, `has_recorded_external_helpers`, …),
//!   `new_not_emitted_statement`, `reset`.
//! - `NodeFactory` extras: `new_temp_variable(_ex)`, `new_loop_variable(_ex)`, `new_unique_name(_ex)`,
//!   `new_generated_name_for_node(_ex)`, private-name variants, `new_string_literal_from_node`; `AutoGenerateOptions`.
//! - Flags/enums: `EmitFlags::SingleLine`, `EmitFlags::NoAsciiEscaping`, … (Go `EF*`), `ListFormat::*` (Go `LF*`),
//!   `GeneratedIdentifierFlags::*`, `WriteKind`, `QuoteChar::{SingleQuote, DoubleQuote, Backtick}`.
//! - Utilities: `escape_string(s: &str, quote_char: QuoteChar) -> String`, `range_is_on_single_line`,
//!   `range_start_positions_are_on_same_line`, `positions_are_on_same_line`, `get_lines_between_positions`,
//!   `is_recognized_triple_slash_comment`, `is_pinned_comment`, `format_generated_name`, `NameGenerator`.
//!
//! Not ported: source map emit (guards kept, generator paths unreachable), the transform-only parts of EmitContext
//! (variable/lexical environments, visitor hooks), the transform helpers of `factory.go`, the helper definitions of
//! `helpers.go` (only `EmitHelper` and its ordering), `changetrackerwriter.go`, `syntheticfile.go`, `emithost.go`,
//! `emitresolver.go`, JSDoc emit (`emitJSDocNode` panics like Go).

mod emitcontext;
mod emitflags;
mod emittextwriter;
mod factory;
mod generatedidentifierflags;
mod helpers;
mod namegenerator;
mod printer_1;
mod printer_2;
mod printer_3;
mod semicolon_writer;
mod singlelinestringwriter;
mod textwriter;
mod utilities;

pub use emitcontext::*;
pub use emitflags::*;
pub use emittextwriter::*;
pub use factory::*;
pub use generatedidentifierflags::*;
pub use helpers::*;
pub use namegenerator::*;
pub use printer_1::*;
pub use printer_2::*;
pub use printer_3::*;
pub(crate) use semicolon_writer::*;
pub use singlelinestringwriter::*;
pub use textwriter::*;
pub use utilities::*;
