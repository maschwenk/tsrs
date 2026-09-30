// STAND-IN for tsrs_printer until merge.
//
// The node builder and printer.rs are written against the `tsrs_printer` crate (Go `internal/printer`), which agent
// `printer-pkg` ports concurrently. Until it merges, this module declares the part of its API the checker uses, with
// the names and signatures the checker assumes (snake_case of the Go API). It is only reached through the seam in
// printer_types.rs (`pub use crate::printer_standin::{...}`). At merge: delete this file, point the seam at
// `tsrs_printer`, add the dependency. Bodies here are placeholders, never run before the merge.

use std::cell::RefCell;
use std::ops::Deref;
use std::rc::Rc;

use bitflags::bitflags;
use rustc_hash::FxHashMap;
use tsrs_ast::{Kind, Node, NodeFactoryHooks, NodeFlags, SourceFile};
use tsrs_core::P;

bitflags! {
    /// Go `printer.EmitFlags` (`printer.EFSingleLine` -> `EmitFlags::SingleLine`).
    #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
    pub struct EmitFlags: u32 {
        const None = 0;
        const SingleLine = 1 << 0;
        const MultiLine = 1 << 1;
        const NoLeadingSourceMap = 1 << 2;
        const NoTrailingSourceMap = 1 << 3;
        const NoNestedSourceMaps = 1 << 4;
        const NoTokenLeadingSourceMaps = 1 << 5;
        const NoTokenTrailingSourceMaps = 1 << 6;
        const NoLeadingComments = 1 << 7;
        const NoTrailingComments = 1 << 8;
        const NoNestedComments = 1 << 9;
        const HelperName = 1 << 10;
        const ExportName = 1 << 11;
        const LocalName = 1 << 12;
        const Indented = 1 << 13;
        const NoIndentation = 1 << 14;
        const ReuseTempVariableScope = 1 << 15;
        const CustomPrologue = 1 << 16;
        const NoAsciiEscaping = 1 << 17;
        const ExternalHelpers = 1 << 18;
        const StartOnNewLine = 1 << 19;
        const IndirectCall = 1 << 20;
        const AsyncFunctionBody = 1 << 21;
        const NoLexicalArguments = 1 << 22;
        const TransformPrivateStaticElements = 1 << 23;
        const NoLexicalThis = 1 << 24;
        const NoSourceMap = Self::NoLeadingSourceMap.bits() | Self::NoTrailingSourceMap.bits();
        const NoTokenSourceMaps = Self::NoTokenLeadingSourceMaps.bits() | Self::NoTokenTrailingSourceMaps.bits();
        const NoComments = Self::NoLeadingComments.bits() | Self::NoTrailingComments.bits();
    }
}

/// Go `printer.NodeFactory`: embeds `ast.NodeFactory` (created with the emit context's hooks), so it derefs to it.
pub struct NodeFactory {
    pub node_factory: tsrs_ast::NodeFactory,
}

impl Deref for NodeFactory {
    type Target = tsrs_ast::NodeFactory;
    fn deref(&self) -> &tsrs_ast::NodeFactory {
        &self.node_factory
    }
}

/// Go `printer.EmitContext`, handled as `P<EmitContext>`; every method takes `&self` (interior mutability), because
/// the factory hooks, the node builder and the printer all share one context.
pub struct EmitContext {
    pub factory: NodeFactory,
    emit_flags: RefCell<FxHashMap<P<Node>, EmitFlags>>,
    original: RefCell<FxHashMap<P<Node>, P<Node>>>,
}

/// Go `printer.NewEmitContext`.
pub fn new_emit_context() -> P<EmitContext> {
    let hooks = NodeFactoryHooks {
        on_create: Some(Rc::new(|node: P<Node>| node.flags.set(node.flags.get() | NodeFlags::Synthesized))),
        ..Default::default()
    };
    P::new(EmitContext {
        factory: NodeFactory { node_factory: tsrs_ast::NodeFactory::new(hooks) },
        emit_flags: RefCell::default(),
        original: RefCell::default(),
    })
}

impl EmitContext {
    pub fn emit_flags(&self, node: P<Node>) -> EmitFlags {
        self.emit_flags.borrow().get(&node).copied().unwrap_or(EmitFlags::None)
    }

    pub fn set_emit_flags(&self, node: P<Node>, flags: EmitFlags) {
        self.emit_flags.borrow_mut().insert(node, flags);
    }

    pub fn add_emit_flags(&self, node: P<Node>, flags: EmitFlags) {
        let old = self.emit_flags(node);
        self.set_emit_flags(node, old | flags);
    }

    pub fn set_original(&self, node: P<Node>, original: P<Node>) {
        self.set_original_ex(node, original, false)
    }

    pub fn set_original_ex(&self, node: P<Node>, original: P<Node>, allow_overwrite: bool) {
        let _ = (node, original, allow_overwrite);
        todo!("tsrs_printer stand-in")
    }

    pub fn original(&self, node: P<Node>) -> Option<P<Node>> {
        self.original.borrow().get(&node).copied()
    }

    /// Go `MostOriginal`: nil in, nil out.
    pub fn most_original(&self, node: Option<P<Node>>) -> Option<P<Node>> {
        let _ = node;
        todo!("tsrs_printer stand-in")
    }

    pub fn assign_comment_range(&self, to: P<Node>, from: P<Node>) {
        let _ = (to, from);
        todo!("tsrs_printer stand-in")
    }

    pub fn add_synthetic_leading_comment(&self, node: P<Node>, kind: Kind, text: &str, has_trailing_new_line: bool) -> P<Node> {
        let _ = (node, kind, text, has_trailing_new_line);
        todo!("tsrs_printer stand-in")
    }

    pub fn add_synthetic_trailing_comment(&self, node: P<Node>, kind: Kind, text: &str, has_trailing_new_line: bool) -> P<Node> {
        let _ = (node, kind, text, has_trailing_new_line);
        todo!("tsrs_printer stand-in")
    }
}

/// Go `printer.PrinterOptions` (only the fields the checker sets are listed here; the real struct has all of them).
#[derive(Clone, Copy, Debug, Default)]
pub struct PrinterOptions {
    pub remove_comments: bool,
    pub omit_trailing_semicolon: bool,
    pub never_ascii_escape: bool,
}

/// Go `printer.PrintHandlers` (the checker always passes the zero value).
#[derive(Default)]
pub struct PrintHandlers {}

/// Go `printer.EmitTextWriter` (only what the checker calls is listed here).
pub trait EmitTextWriter {
    /// Go `String()`.
    fn string(&self) -> String;
}

/// Go `printer.Printer`, used by value (`&mut self` methods).
pub struct Printer {}

/// Go `printer.NewPrinter`.
pub fn new_printer(options: PrinterOptions, handlers: PrintHandlers, emit_context: Option<P<EmitContext>>) -> Printer {
    let _ = (options, handlers, emit_context);
    todo!("tsrs_printer stand-in")
}

impl Printer {
    /// Go `Write(node, sourceFile, writer, sourceMapGenerator)`; source maps are not ported, so the last parameter is dropped.
    pub fn write(&mut self, node: P<Node>, source_file: Option<P<SourceFile>>, writer: &mut dyn EmitTextWriter) {
        let _ = (node, source_file, writer);
        todo!("tsrs_printer stand-in")
    }

    pub fn emit(&mut self, node: P<Node>, source_file: Option<P<SourceFile>>) -> String {
        let _ = (node, source_file);
        todo!("tsrs_printer stand-in")
    }
}

/// Go `printer.NewTextWriter`.
pub fn new_text_writer(new_line: &str, indent_size: i32) -> Box<dyn EmitTextWriter> {
    let _ = (new_line, indent_size);
    todo!("tsrs_printer stand-in")
}

/// Go `printer.GetSingleLineStringWriter`; the Go pool (and its release func) is dropped.
pub fn get_single_line_string_writer() -> Box<dyn EmitTextWriter> {
    todo!("tsrs_printer stand-in")
}
