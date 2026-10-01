use std::cell::Cell;
use std::fmt::Display;
use std::sync::LazyLock;

use bitflags::bitflags;
use rustc_hash::{FxHashMap, FxHashSet};
use tsrs_ast::{self as ast, CommentRange, Diagnostic, DiagnosticExt, Kind, ModifierFlags, ModifierList, Node, NodeFactory, NodeFlags, NodeList, SourceFile, SourceFileParseOptions, TokenFlags};
use tsrs_core::{alloc_slice, alloc_str, alloc_vec, new_text_range, tspath, LanguageVariant, ScriptKind, TextRange, P};
use tsrs_diagnostics::{self as diagnostics, Message};
use tsrs_scanner::{self as scanner, Scanner, ScannerState};

use crate::parser_3::{attach_file_to_diagnostics, get_comment_pragmas};
use crate::references::collect_external_module_references;
use crate::types::ParseFlags;
use crate::utilities::{get_language_variant, is_keyword_or_punctuation, token_is_identifier_or_keyword};

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum ParsingContext {
    SourceElements,          // Elements in source file
    BlockStatements,         // Statements in block
    SwitchClauses,           // Clauses in switch statement
    SwitchClauseStatements,  // Statements in switch clause
    TypeMembers,             // Members in interface or type literal
    ClassMembers,            // Members in class declaration
    EnumMembers,             // Members in enum declaration
    HeritageClauseElement,   // Elements in a heritage clause
    VariableDeclarations,    // Variable declarations in variable statement
    ObjectBindingElements,   // Binding elements in object binding list
    ArrayBindingElements,    // Binding elements in array binding list
    ArgumentExpressions,     // Expressions in argument list
    ObjectLiteralMembers,    // Members in object literal
    JsxAttributes,           // Attributes in jsx element
    JsxChildren,             // Things between opening and closing JSX tags
    ArrayLiteralMembers,     // Members in array literal
    Parameters,              // Parameters in parameter list
    JSDocParameters,         // JSDoc parameters in parameter list of JSDoc function type
    RestProperties,          // Property names in a rest type list
    TypeParameters,          // Type parameters in type parameter list
    TypeArguments,           // Type arguments in type argument list
    TupleElementTypes,       // Element types in tuple element type list
    HeritageClauses,         // Heritage clauses for a class or interface declaration.
    ImportOrExportSpecifiers, // Named import clause's import specifier list
    ImportAttributes,        // Import attributes
    JSDocComment,            // Parsing via JSDocParser
    Count,                   // Number of parsing contexts
}

impl ParsingContext {
    pub(crate) const ALL: [ParsingContext; ParsingContext::Count as usize] = [
        ParsingContext::SourceElements,
        ParsingContext::BlockStatements,
        ParsingContext::SwitchClauses,
        ParsingContext::SwitchClauseStatements,
        ParsingContext::TypeMembers,
        ParsingContext::ClassMembers,
        ParsingContext::EnumMembers,
        ParsingContext::HeritageClauseElement,
        ParsingContext::VariableDeclarations,
        ParsingContext::ObjectBindingElements,
        ParsingContext::ArrayBindingElements,
        ParsingContext::ArgumentExpressions,
        ParsingContext::ObjectLiteralMembers,
        ParsingContext::JsxAttributes,
        ParsingContext::JsxChildren,
        ParsingContext::ArrayLiteralMembers,
        ParsingContext::Parameters,
        ParsingContext::JSDocParameters,
        ParsingContext::RestProperties,
        ParsingContext::TypeParameters,
        ParsingContext::TypeArguments,
        ParsingContext::TupleElementTypes,
        ParsingContext::HeritageClauses,
        ParsingContext::ImportOrExportSpecifiers,
        ParsingContext::ImportAttributes,
        ParsingContext::JSDocComment,
    ];
}

/// Bit set of `1 << ParsingContext`.
pub type ParsingContexts = i32;

pub struct JSDocInfo {
    pub(crate) parent: P<Node>,
    pub(crate) js_docs: &'static [P<Node>],
}

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
    pub(crate) struct JsdocScannerInfo: u8 {
        const HasJSDoc = 1 << 0;
        const HasDeprecated = 1 << 1;
        const HasSeeOrLink = 1 << 2;
    }
}

pub struct Parser {
    pub(crate) scanner: Scanner,
    pub(crate) factory: NodeFactory,

    pub(crate) opts: SourceFileParseOptions,
    pub(crate) source_text: &'static str,

    pub(crate) script_kind: ScriptKind,
    pub(crate) language_variant: LanguageVariant,
    pub(crate) diagnostics: Vec<P<Diagnostic>>,
    pub(crate) js_diagnostics: Vec<P<Diagnostic>>,
    pub(crate) jsdoc_diagnostics: Vec<P<Diagnostic>>,

    pub(crate) token: Kind,
    pub(crate) source_flags: NodeFlags,
    pub(crate) context_flags: NodeFlags,
    pub(crate) parsing_contexts: ParsingContexts,
    pub(crate) statement_has_await_identifier: bool,
    pub(crate) has_deprecated_tag: bool,
    pub(crate) has_parse_error: bool,

    pub(crate) identifier_count: usize,
    pub(crate) not_parenthesized_arrow: FxHashSet<i32>,
    pub(crate) jsdoc_infos: Vec<JSDocInfo>,
    pub(crate) possible_await_spans: Vec<usize>,
    pub(crate) jsdoc_comments_space: Vec<&'static str>,
    pub(crate) jsdoc_comment_ranges_space: Vec<CommentRange>,
    pub(crate) jsdoc_tag_comments_space: Vec<&'static str>,
    pub(crate) jsdoc_tag_comments_parts_space: Vec<P<Node>>,
    pub(crate) reparse_list: Vec<P<Node>>,

    pub(crate) current_parent: Option<P<Node>>,
    pub(crate) reparsed_clones: Vec<P<Node>>,

    // Go builds node lists in `make([]*ast.Node, 0, 16)` slices that escape analysis keeps on the goroutine stack,
    // and clones the finished list into the node slice arena. The port builds every list on this one stack
    // instead (a nested list pushes above its parent's elements and pops them before returning), so building a
    // list costs no heap allocation; `finish_node_list` copies the elements into the arena.
    pub(crate) node_stack: Vec<P<Node>>,
}

pub(crate) fn new_parser() -> Parser {
    Parser {
        scanner: Scanner::new(),
        factory: NodeFactory::default(),
        opts: SourceFileParseOptions::default(),
        source_text: "",
        script_kind: ScriptKind::Unknown,
        language_variant: LanguageVariant::Standard,
        diagnostics: Vec::new(),
        js_diagnostics: Vec::new(),
        jsdoc_diagnostics: Vec::new(),
        token: Kind::Unknown,
        source_flags: NodeFlags::None,
        context_flags: NodeFlags::None,
        parsing_contexts: 0,
        statement_has_await_identifier: false,
        has_deprecated_tag: false,
        has_parse_error: false,
        identifier_count: 0,
        not_parenthesized_arrow: FxHashSet::default(),
        jsdoc_infos: Vec::new(),
        possible_await_spans: Vec::new(),
        jsdoc_comments_space: Vec::new(),
        jsdoc_comment_ranges_space: Vec::new(),
        jsdoc_tag_comments_space: Vec::new(),
        jsdoc_tag_comments_parts_space: Vec::new(),
        reparse_list: Vec::new(),
        current_parent: None,
        reparsed_clones: Vec::new(),
        node_stack: Vec::new(),
    }
}

pub(crate) static viable_keyword_suggestions: LazyLock<Vec<&'static str>> = LazyLock::new(scanner::get_viable_keyword_suggestions);

// MISSING_LIST_NODES is a sentinel backing array used to distinguish "missing" node lists
// (where the expected opening token was not found) from ordinary empty node lists.
// A pointer-sized static gives a unique, suitably aligned address for a zero-length slice.
static MISSING_LIST_NODES_BACKING: usize = 0;

pub(crate) fn missing_list_nodes() -> &'static [P<Node>] {
    // SAFETY: a zero-length slice needs only a non-null, aligned pointer; P<Node> has the size
    // and alignment of a pointer, and nothing is ever read through this slice.
    unsafe { std::slice::from_raw_parts(&MISSING_LIST_NODES_BACKING as *const usize as *const P<Node>, 0) }
}

pub(crate) fn is_missing_node_list(list: Option<P<NodeList>>) -> bool {
    match list {
        Some(list) => std::ptr::eq(list.nodes.as_ptr(), missing_list_nodes().as_ptr()),
        None => false,
    }
}

pub fn parse_source_file(opts: SourceFileParseOptions, source_text: &str, script_kind: ScriptKind) -> P<SourceFile> {
    parse_source_file_static(opts, alloc_str(source_text), script_kind)
}

/// `parse_source_file` for text the caller hands over (a file just read): it becomes the file's text without a
/// copy into the arena (Go shares the string). Like the arena, the text is never freed.
pub fn parse_source_file_owned(opts: SourceFileParseOptions, source_text: String, script_kind: ScriptKind) -> P<SourceFile> {
    parse_source_file_static(opts, source_text.leak(), script_kind)
}

fn parse_source_file_static(opts: SourceFileParseOptions, source_text: &'static str, script_kind: ScriptKind) -> P<SourceFile> {
    crate::jsdoc::init();
    let mut p = new_parser();
    p.initialize_state_static(opts, source_text, script_kind);
    p.next_token();
    if p.script_kind == ScriptKind::JSON {
        return p.parse_json_text();
    }
    p.parse_source_file_worker()
}

impl Parser {
    pub(crate) fn is_java_script(&self) -> bool {
        self.script_kind == ScriptKind::JS || self.script_kind == ScriptKind::JSX
    }

    pub(crate) fn parse_json_text(&mut self) -> P<SourceFile> {
        let pos = self.node_pos();
        let statements: P<NodeList>;
        let eof: P<Node>;

        if self.token == Kind::EndOfFile {
            let end = self.node_pos();
            statements = self.new_node_list(new_text_range(pos, end), &[]);
            eof = self.parse_token_node();
        } else {
            // Go keeps a single expression until a second one shows up ([]*ast.Expression | *ast.Expression).
            let mut expressions: Vec<P<Node>> = Vec::new();

            while self.token != Kind::EndOfFile {
                let expression: P<Node> = match self.token {
                    Kind::OpenBracketToken => self.parse_array_literal_expression(),
                    Kind::TrueKeyword | Kind::FalseKeyword | Kind::NullKeyword => self.parse_token_node(),
                    Kind::MinusToken => {
                        if self.look_ahead(|p| p.next_token() == Kind::NumericLiteral && p.next_token() != Kind::ColonToken) {
                            self.parse_prefix_unary_expression()
                        } else {
                            self.parse_object_literal_expression()
                        }
                    }
                    Kind::NumericLiteral | Kind::StringLiteral => {
                        if self.look_ahead(|p| p.next_token() != Kind::ColonToken) {
                            self.parse_literal_expression()
                        } else {
                            self.parse_object_literal_expression()
                        }
                    }
                    _ => self.parse_object_literal_expression(),
                };

                // Error recovery: collect multiple top-level expressions
                if !expressions.is_empty() {
                    expressions.push(expression);
                } else {
                    expressions.push(expression);
                    if self.token != Kind::EndOfFile {
                        self.parse_error_at_current_token(&diagnostics::Unexpected_token, &[]);
                    }
                }
            }

            let expression = if expressions.len() > 1 {
                let end = self.node_pos();
                let list = self.new_node_list(new_text_range(pos, end), &expressions);
                let array = self.factory.new_array_literal_expression(list, false);
                self.finish_node(array, pos)
            } else {
                expressions[0]
            };
            let statement = self.factory.new_expression_statement(expression);
            let statement = self.finish_node(statement, pos);
            let end = self.node_pos();
            statements = self.new_node_list(new_text_range(pos, end), &[statement]);
            eof = self.parse_expected_token(Kind::EndOfFile);
        }
        let node = self.factory.new_source_file(self.opts.clone(), self.source_text, statements, eof);
        let node = self.finish_node(node, pos);
        let result = P::from_static(node.as_source_file());
        if !result.statements.nodes.is_empty() {
            let value = result.statements.nodes[0].expression();
            self.validate_json_value(result, value);
        }
        self.finish_source_file(result, false);
        result
    }
}

pub(crate) fn get_error_span_for_node(source_text: &str, node: P<Node>) -> TextRange {
    let mut pos = node.pos();
    if !ast::node_is_missing(Some(node)) {
        pos = scanner::skip_trivia(source_text, pos);
    }
    new_text_range(pos, node.end())
}

impl Parser {
    pub(crate) fn validate_json_value(&mut self, source_file: P<SourceFile>, value_expression: Option<P<Node>>) {
        let Some(value_expression) = value_expression else {
            return;
        };
        match value_expression.kind() {
            Kind::TrueKeyword | Kind::FalseKeyword | Kind::NullKeyword | Kind::NumericLiteral => {
                return;
            }
            Kind::StringLiteral => {
                if !is_double_quoted_string(value_expression) {
                    self.diagnostics.push(ast::new_diagnostic(
                        Some(source_file),
                        get_error_span_for_node(self.source_text, value_expression),
                        &diagnostics::String_literal_with_double_quotes_expected,
                        &[],
                    ));
                }
                return;
            }
            Kind::PrefixUnaryExpression => {
                let prefix = value_expression.as_prefix_unary_expression();
                if !(prefix.operator() != Kind::MinusToken || prefix.operand().kind() != Kind::NumericLiteral) {
                    return;
                }
                // not valid JSON syntax
            }
            Kind::ObjectLiteralExpression => {
                self.validate_json_object_literal(source_file, value_expression);
                return;
            }
            Kind::ArrayLiteralExpression => {
                for element in value_expression.elements() {
                    self.validate_json_value(source_file, Some(*element));
                }
                return;
            }
            _ => {}
        }
        self.diagnostics.push(ast::new_diagnostic(
            Some(source_file),
            get_error_span_for_node(self.source_text, value_expression),
            &diagnostics::Property_value_can_only_be_string_literal_numeric_literal_true_false_null_object_literal_or_array_literal,
            &[],
        ));
    }
}

pub(crate) fn is_double_quoted_string(node: P<Node>) -> bool {
    ast::is_string_literal(node) && !node.as_string_literal().token_flags().intersects(TokenFlags::SingleQuote)
}

impl Parser {
    // validateJsonObjectLiteral validates properties of a JSON object literal.
    pub(crate) fn validate_json_object_literal(&mut self, source_file: P<SourceFile>, node: P<Node>) {
        for element in node.as_object_literal_expression().properties().nodes {
            let element = *element;
            if element.kind() != Kind::PropertyAssignment {
                self.diagnostics.push(ast::new_diagnostic(
                    Some(source_file),
                    get_error_span_for_node(self.source_text, element),
                    &diagnostics::Property_assignment_expected,
                    &[],
                ));
                continue;
            }
            if let Some(name) = element.name() {
                if !is_double_quoted_string(name) {
                    self.diagnostics.push(ast::new_diagnostic(
                        Some(source_file),
                        get_error_span_for_node(self.source_text, name),
                        &diagnostics::String_literal_with_double_quotes_expected,
                        &[],
                    ));
                }
            }
            self.validate_json_value(source_file, Some(element.as_property_assignment().initializer()));
        }
    }
}

pub fn parse_isolated_entity_name(text: &str) -> Option<P<Node>> {
    let mut p = new_parser();
    p.initialize_state(SourceFileParseOptions::default(), text, ScriptKind::JS);
    p.next_token();
    let entity_name = p.parse_entity_name(true, false, None);
    if p.token == Kind::EndOfFile && p.diagnostics.is_empty() {
        Some(entity_name)
    } else {
        None
    }
}

impl Parser {
    pub(crate) fn initialize_state(&mut self, opts: SourceFileParseOptions, source_text: &str, script_kind: ScriptKind) {
        self.initialize_state_static(opts, alloc_str(source_text), script_kind)
    }

    /// `initialize_state` for text that already lives in the arena (Go shares the string; no copy).
    pub(crate) fn initialize_state_static(
        &mut self,
        opts: SourceFileParseOptions,
        source_text: &'static str,
        script_kind: ScriptKind,
    ) {
        if script_kind == ScriptKind::Unknown {
            panic!("ScriptKind must be specified when parsing source file: {}", opts.file_name);
        }

        self.scanner.reset();
        self.opts = opts;
        self.source_text = source_text;
        self.script_kind = script_kind;
        self.language_variant = get_language_variant(self.script_kind);
        self.context_flags = match self.script_kind {
            ScriptKind::JS | ScriptKind::JSX => NodeFlags::JavaScriptFile,
            ScriptKind::JSON => NodeFlags::JavaScriptFile | NodeFlags::JsonFile,
            _ => NodeFlags::None,
        };
        self.scanner.set_text(self.source_text);
        self.scanner.set_on_error(true);
        self.scanner.set_language_variant(self.language_variant);
    }

    pub(crate) fn scan_error(&mut self, message: &'static Message, pos: i32, length: i32, args: &[&dyn Display]) {
        self.parse_error_at_range(new_text_range(pos, pos + length), message, args);
    }

    // The scanner buffers what Go reports through its synchronous error callback; this must run
    // right after every scanner call that can report (scan, re_scan_*, scan_jsx_*, scan_jsdoc_*)
    // so scanner errors interleave with parser errors exactly as in Go.
    #[inline]
    pub(crate) fn report_scan_errors(&mut self) {
        if self.scanner.has_errors() {
            self.report_buffered_scan_errors();
        }
    }

    #[cold]
    fn report_buffered_scan_errors(&mut self) {
        for error in self.scanner.take_errors() {
            let args: Vec<&dyn Display> = error.args.iter().map(|arg| arg as &dyn Display).collect();
            self.scan_error(error.message, error.start, error.length, &args);
        }
    }

    pub(crate) fn parse_error_at(&mut self, pos: i32, end: i32, message: &'static Message, args: &[&dyn Display]) -> Option<P<Diagnostic>> {
        self.parse_error_at_range(new_text_range(pos, end), message, args)
    }

    pub(crate) fn parse_error_at_current_token(&mut self, message: &'static Message, args: &[&dyn Display]) -> Option<P<Diagnostic>> {
        let loc = self.scanner.token_range();
        self.parse_error_at_range(loc, message, args)
    }

    pub(crate) fn parse_error_at_range(&mut self, loc: TextRange, message: &'static Message, args: &[&dyn Display]) -> Option<P<Diagnostic>> {
        // Don't report another error if it would just be at the same location as the last error
        let mut result = None;
        if self.diagnostics.is_empty() || self.diagnostics[self.diagnostics.len() - 1].pos() != loc.pos() {
            let diagnostic = ast::new_diagnostic(None, loc, message, args);
            self.diagnostics.push(diagnostic);
            result = Some(diagnostic);
        }
        self.has_parse_error = true;
        result
    }
}

pub struct ParserState {
    pub(crate) scanner_state: ScannerState,
    pub(crate) context_flags: NodeFlags,
    pub(crate) diagnostics_len: usize,
    pub(crate) js_diagnostics_len: usize,
    pub(crate) jsdoc_infos_len: usize,
    pub(crate) reparsed_clones_len: usize,
    pub(crate) statement_has_await_identifier: bool,
    pub(crate) has_parse_error: bool,
}

impl Parser {
    pub(crate) fn mark(&mut self) -> ParserState {
        ParserState {
            scanner_state: self.scanner.mark(),
            context_flags: self.context_flags,
            diagnostics_len: self.diagnostics.len(),
            js_diagnostics_len: self.js_diagnostics.len(),
            jsdoc_infos_len: self.jsdoc_infos.len(),
            reparsed_clones_len: self.reparsed_clones.len(),
            statement_has_await_identifier: self.statement_has_await_identifier,
            has_parse_error: self.has_parse_error,
        }
    }

    pub(crate) fn rewind(&mut self, state: ParserState) {
        self.scanner.rewind(state.scanner_state);
        self.token = self.scanner.token();
        self.context_flags = state.context_flags;
        self.diagnostics.truncate(state.diagnostics_len);
        self.js_diagnostics.truncate(state.js_diagnostics_len);
        self.jsdoc_infos.truncate(state.jsdoc_infos_len);
        self.reparsed_clones.truncate(state.reparsed_clones_len);
        self.statement_has_await_identifier = state.statement_has_await_identifier;
        self.has_parse_error = state.has_parse_error;
    }

    pub(crate) fn look_ahead(&mut self, callback: impl FnOnce(&mut Parser) -> bool) -> bool {
        let state = self.mark();
        let result = callback(self);
        self.rewind(state);
        result
    }

    pub(crate) fn next_token(&mut self) -> Kind {
        // if the keyword had an escape
        if ast::is_keyword(self.token) && (self.scanner.has_unicode_escape() || self.scanner.has_extended_unicode_escape()) {
            // issue a parse error for the escape
            self.parse_error_at_current_token(&diagnostics::Keywords_cannot_contain_escape_characters, &[]);
        }
        self.token = self.scanner.scan();
        self.report_scan_errors();
        self.token
    }

    pub(crate) fn next_token_without_check(&mut self) -> Kind {
        self.token = self.scanner.scan();
        self.report_scan_errors();
        self.token
    }

    pub(crate) fn next_token_jsdoc(&mut self) -> Kind {
        self.token = self.scanner.scan_jsdoc_token();
        self.report_scan_errors();
        self.token
    }

    pub(crate) fn next_jsdoc_comment_text_token(&mut self, in_backticks: bool) -> Kind {
        self.token = self.scanner.scan_jsdoc_comment_text_token(in_backticks);
        self.report_scan_errors();
        self.token
    }

    pub(crate) fn node_pos(&self) -> i32 {
        self.scanner.token_full_start()
    }

    pub(crate) fn has_preceding_line_break(&self) -> bool {
        self.scanner.has_preceding_line_break()
    }

    pub(crate) fn jsdoc_scanner_info(&self) -> JsdocScannerInfo {
        if !self.scanner.has_preceding_jsdoc_comment() {
            return JsdocScannerInfo::empty();
        }
        let mut info = JsdocScannerInfo::HasJSDoc;
        if self.scanner.has_preceding_jsdoc_with_deprecated_tag() {
            info |= JsdocScannerInfo::HasDeprecated;
        }
        if self.scanner.has_preceding_jsdoc_with_see_or_link() {
            info |= JsdocScannerInfo::HasSeeOrLink;
        }
        info
    }

    pub(crate) fn parse_source_file_worker(&mut self) -> P<SourceFile> {
        let is_declaration_file = tspath::is_declaration_file_name(&self.opts.file_name);
        if is_declaration_file {
            self.context_flags |= NodeFlags::Ambient;
        }
        let pos = self.node_pos();
        let mut statements = self.parse_list_index(ParsingContext::SourceElements, Parser::parse_toplevel_statement);
        let end = self.node_pos();
        let end_jsdoc = self.jsdoc_scanner_info();
        let eof = self.parse_token_node();
        self.with_jsdoc(eof, end_jsdoc);
        if eof.kind() != Kind::EndOfFile {
            panic!("Expected end of file token from scanner.");
        }
        if !self.reparse_list.is_empty() {
            statements.append(&mut self.reparse_list);
        }
        let statement_list = self.new_node_list(new_text_range(pos, end), &statements);
        let node = self.factory.new_source_file(self.opts.clone(), self.source_text, statement_list, eof);
        let node = self.finish_node(node, pos);
        let mut result = P::from_static(node.as_source_file());
        self.finish_source_file(result, is_declaration_file);
        if !result.is_declaration_file.get() && result.external_module_indicator.get().is_some() && !self.possible_await_spans.is_empty() {
            let reparse = self.reparse_top_level_await(result);
            let reparse = self.finish_node(reparse, pos);
            if node != reparse {
                result = P::from_static(reparse.as_source_file());
                self.finish_source_file(result, is_declaration_file);
            }
        }
        collect_external_module_references(result);
        if ast::is_in_js_file(Some(node)) {
            result.set_js_diagnostics(&attach_file_to_diagnostics(&self.js_diagnostics, result));
        }
        result
    }

    pub(crate) fn finish_source_file(&mut self, result: P<SourceFile>, is_declaration_file: bool) {
        result.comment_directives.set(self.scanner.comment_directives());
        result.pragmas.set(alloc_vec(get_comment_pragmas(&mut self.factory, self.source_text)));
        self.process_pragmas_into_fields(result);
        result.set_diagnostics(&attach_file_to_diagnostics(&self.diagnostics, result));
        result.set_jsdoc_diagnostics(&attach_file_to_diagnostics(&self.jsdoc_diagnostics, result));
        result.is_declaration_file.set(is_declaration_file);
        result.language_variant.set(self.language_variant);
        result.script_kind.set(self.script_kind);
        let node = result.as_node();
        node.set_flags(node.flags() | self.source_flags);
        result.node_count.set(self.factory.node_count());
        result.text_count.set(self.factory.text_count());
        result.identifier_count.set(self.identifier_count);
        result.set_jsdoc_cache(self.create_jsdoc_cache());
        // For non-JS files, enable lazy JSDoc parsing on demand
        if !self.is_java_script() {
            result.set_has_lazy_jsdoc(true);
        }
        self.reparsed_clones.sort_by(|a, b| ast::compare_node_positions(*a, *b).cmp(&0));
        result.reparsed_clones.set(alloc_slice(&self.reparsed_clones));
        ast::set_external_module_indicator(&result, self.opts.external_module_indicator_options);
    }

    pub(crate) fn create_jsdoc_cache(&self) -> FxHashMap<P<Node>, &'static [P<Node>]> {
        if self.jsdoc_infos.is_empty() {
            return FxHashMap::default();
        }
        let mut result = FxHashMap::with_capacity_and_hasher(self.jsdoc_infos.len(), Default::default());
        for info in &self.jsdoc_infos {
            result.insert(info.parent, info.js_docs);
        }
        result
    }

    pub(crate) fn parse_toplevel_statement(&mut self, i: usize) -> P<Node> {
        self.statement_has_await_identifier = false;
        let statement = self.parse_statement();
        // Reparsed nodes (e.g. JSDoc @typedef) produced while parsing this statement are inserted
        // into the statement list before this statement, so account for them when recording the
        // statement's index for possibleAwaitSpans.
        let i = i + self.reparse_list.len();
        if self.statement_has_await_identifier && !statement.flags().intersects(NodeFlags::AwaitContext) {
            if self.possible_await_spans.is_empty() || self.possible_await_spans[self.possible_await_spans.len() - 1] != i {
                self.possible_await_spans.push(i);
                self.possible_await_spans.push(i + 1);
            } else {
                let last = self.possible_await_spans.len() - 1;
                self.possible_await_spans[last] = i + 1;
            }
        }
        statement
    }

    pub(crate) fn reparse_top_level_await(&mut self, source_file: P<SourceFile>) -> P<Node> {
        if self.possible_await_spans.len() % 2 == 1 {
            panic!("possibleAwaitSpans malformed: odd number of indices, not paired into spans.");
        }
        let source_statements: &'static [P<Node>] = source_file.statements.nodes;
        let mut statements: Vec<P<Node>> = Vec::new();
        let saved_parse_diagnostics = std::mem::take(&mut self.diagnostics);

        let mut after_await_statement = 0;
        let mut i = 0;
        while i < self.possible_await_spans.len() {
            let next_await_statement = self.possible_await_spans[i];
            // append all non-await statements between afterAwaitStatement and nextAwaitStatement
            let prev_statement = source_statements[after_await_statement];
            let next_statement = source_statements[next_await_statement];
            statements.extend_from_slice(&source_statements[after_await_statement..next_await_statement]);

            // append all diagnostics associated with the copied range
            let diagnostic_start = saved_parse_diagnostics.iter().position(|diagnostic| diagnostic.pos() >= prev_statement.pos());
            let diagnostic_end = match diagnostic_start {
                Some(start) => saved_parse_diagnostics[start..].iter().position(|diagnostic| diagnostic.pos() >= next_statement.pos()),
                None => None,
            };
            if let Some(start) = diagnostic_start {
                let slice = match diagnostic_end {
                    Some(end) => &saved_parse_diagnostics[start..start + end],
                    None => &saved_parse_diagnostics[start..],
                };
                self.diagnostics.extend_from_slice(slice);
            }

            let mut state = self.mark();
            // reparse all statements between start and pos. We skip existing diagnostics for the same range and allow the parser to generate new ones.
            self.context_flags |= NodeFlags::AwaitContext;
            self.scanner.reset_pos(next_statement.pos());
            self.next_token();

            after_await_statement = self.possible_await_spans[i + 1];
            while self.token != Kind::EndOfFile {
                let start_pos = self.scanner.token_full_start();
                let statement = self.parse_statement();
                statements.push(statement);
                if start_pos == self.scanner.token_full_start() {
                    self.next_token();
                }
                if after_await_statement < source_statements.len() {
                    let last_await_statement = source_statements[after_await_statement - 1];
                    if statement.end() == last_await_statement.end() {
                        // done reparsing this section
                        break;
                    }
                    if statement.end() > last_await_statement.end() {
                        // we ate into the next statement, so we must continue reparsing the next span
                        i += 2;
                        if i < self.possible_await_spans.len() {
                            after_await_statement = self.possible_await_spans[i + 1];
                        } else {
                            after_await_statement = source_statements.len();
                        }
                    }
                }
            }

            // Keep diagnostics from the reparse
            state.diagnostics_len = self.diagnostics.len();
            self.rewind(state);
            i += 2;
        }

        // append all statements between pos and the end of the list
        if after_await_statement < source_statements.len() {
            let prev_statement = source_statements[after_await_statement];
            statements.extend_from_slice(&source_statements[after_await_statement..]);

            // append all diagnostics associated with the copied range
            let diagnostic_start = saved_parse_diagnostics.iter().position(|diagnostic| diagnostic.pos() >= prev_statement.pos());
            if let Some(start) = diagnostic_start {
                self.diagnostics.extend_from_slice(&saved_parse_diagnostics[start..]);
            }
        }

        let loc = source_file.statements.loc.get();
        let list = self.new_node_list(loc, &statements);
        let result = self.factory.new_source_file(source_file.parse_options().clone(), self.source_text, list, source_file.end_of_file_token);
        for s in &statements {
            s.set_parent(Some(result)); // force (re)set parent to reparsed source file
        }
        result
    }

    pub(crate) fn parse_list_index(&mut self, kind: ParsingContext, parse_element: impl FnMut(&mut Parser, usize) -> P<Node>) -> Vec<P<Node>> {
        let start = self.parse_list_index_on_stack(kind, parse_element);
        self.node_stack.split_off(start)
    }

    /// `parseListIndex` with the elements left on `node_stack` above the returned start index.
    pub(crate) fn parse_list_index_on_stack(&mut self, kind: ParsingContext, mut parse_element: impl FnMut(&mut Parser, usize) -> P<Node>) -> usize {
        let save_parsing_contexts = self.parsing_contexts;
        self.parsing_contexts |= 1 << kind as i32;
        let mut outer_reparse_list = std::mem::take(&mut self.reparse_list);
        let start = self.node_stack.len();
        while !self.is_list_terminator(kind) {
            if self.is_list_element(kind, false /*inErrorRecovery*/) {
                let elt = parse_element(self, self.node_stack.len() - start);
                if !self.reparse_list.is_empty() {
                    for e in std::mem::take(&mut self.reparse_list) {
                        // Propagate @typedef type alias declarations outwards to a context that permits them.
                        if (ast::is_js_type_alias_declaration(e) || ast::is_js_import_declaration(e))
                            && kind != ParsingContext::SourceElements
                            && kind != ParsingContext::BlockStatements
                        {
                            outer_reparse_list.push(e);
                        } else {
                            self.node_stack.push(e);
                        }
                    }
                }
                self.node_stack.push(elt);
                continue;
            }
            if self.abort_parsing_list_or_move_to_next_token(kind) {
                break;
            }
        }
        self.reparse_list = outer_reparse_list;
        self.parsing_contexts = save_parsing_contexts;
        start
    }

    pub(crate) fn parse_list(&mut self, kind: ParsingContext, mut parse_element: impl FnMut(&mut Parser) -> P<Node>) -> P<NodeList> {
        let pos = self.node_pos();
        let start = self.parse_list_index_on_stack(kind, |p, _| parse_element(p));
        let end = self.node_pos();
        self.finish_node_list(start, new_text_range(pos, end))
    }

    /// Moves the elements above `start` off `node_stack` into a new arena node list.
    pub(crate) fn finish_node_list(&mut self, start: usize, loc: TextRange) -> P<NodeList> {
        let list = self.new_node_list(loc, &self.node_stack[start..]);
        self.node_stack.truncate(start);
        list
    }

    // Return a non-nil (but possibly empty) slice if parsing was successful, or nil if parseElement returned nil
    pub(crate) fn parse_delimited_list<R: Into<Option<P<Node>>>>(
        &mut self,
        kind: ParsingContext,
        mut parse_element: impl FnMut(&mut Parser) -> R,
    ) -> Option<P<NodeList>> {
        let pos = self.node_pos();
        let save_parsing_contexts = self.parsing_contexts;
        self.parsing_contexts |= 1 << kind as i32;
        let start = self.node_stack.len();
        loop {
            if self.is_list_element(kind, false /*inErrorRecovery*/) {
                let start_pos = self.node_pos();
                let element: Option<P<Node>> = parse_element(self).into();
                let Some(element) = element else {
                    self.parsing_contexts = save_parsing_contexts;
                    self.node_stack.truncate(start);
                    // Return nil to indicate parseElement failed
                    return None;
                };
                self.node_stack.push(element);
                if self.parse_optional(Kind::CommaToken) {
                    // No need to check for a zero length node since we know we parsed a comma
                    continue;
                }
                if self.is_list_terminator(kind) {
                    break;
                }
                // We didn't get a comma, and the list wasn't terminated, explicitly parse
                // out a comma so we give a good error message.
                if self.token != Kind::CommaToken && kind == ParsingContext::EnumMembers {
                    self.parse_error_at_current_token(&diagnostics::An_enum_member_name_must_be_followed_by_a_or, &[]);
                } else {
                    self.parse_expected(Kind::CommaToken);
                }
                // If the token was a semicolon, and the caller allows that, then skip it and
                // continue.  This ensures we get back on track and don't result in tons of
                // parse errors.  For example, this can happen when people do things like use
                // a semicolon to delimit object literal members.   Note: we'll have already
                // reported an error when we called parseExpected above.
                if (kind == ParsingContext::ObjectLiteralMembers || kind == ParsingContext::ImportAttributes)
                    && self.token == Kind::SemicolonToken
                    && !self.has_preceding_line_break()
                {
                    self.next_token();
                }
                if start_pos == self.node_pos() {
                    // What we're parsing isn't actually remotely recognizable as a element and we've consumed no tokens whatsoever
                    // Consume a token to advance the parser in some way and avoid an infinite loop
                    // This can happen when we're speculatively parsing parenthesized expressions which we think may be arrow functions,
                    // or when a modifier keyword which is disallowed as a parameter name (ie, `static` in strict mode) is supplied
                    self.next_token();
                }
                continue;
            }
            if self.is_list_terminator(kind) {
                break;
            }
            if self.abort_parsing_list_or_move_to_next_token(kind) {
                break;
            }
        }
        self.parsing_contexts = save_parsing_contexts;
        let end = self.node_pos();
        Some(self.finish_node_list(start, new_text_range(pos, end)))
    }

    // Return a non-nil (but possibly empty) NodeList if parsing was successful, a missing NodeList if the opening
    // token wasn't found, or nil if parseElement returned nil.
    pub(crate) fn parse_bracketed_list<R: Into<Option<P<Node>>>>(
        &mut self,
        kind: ParsingContext,
        parse_element: impl FnMut(&mut Parser) -> R,
        opening: Kind,
        closing: Kind,
    ) -> Option<P<NodeList>> {
        if self.parse_expected(opening) {
            let result = self.parse_delimited_list(kind, parse_element);
            self.parse_expected(closing);
            return result;
        }
        Some(self.create_missing_list())
    }

    pub(crate) fn parse_empty_node_list(&mut self) -> P<NodeList> {
        let pos = self.node_pos();
        self.new_node_list(new_text_range(pos, pos), &[])
    }

    pub(crate) fn create_missing_list(&mut self) -> P<NodeList> {
        let pos = self.node_pos();
        let result = self.factory.new_node_list_from_static(missing_list_nodes());
        result.loc.set(new_text_range(pos, pos));
        result
    }

    // Returns true if we should abort parsing.
    pub(crate) fn abort_parsing_list_or_move_to_next_token(&mut self, kind: ParsingContext) -> bool {
        self.parsing_context_errors(kind);
        if self.is_in_some_parsing_context() {
            return true;
        }
        self.next_token();
        false
    }

    // True if positioned at element or terminator of the current list or any enclosing list
    pub(crate) fn is_in_some_parsing_context(&mut self) -> bool {
        // We should be in at least one parsing context, be it SourceElements while parsing
        // a SourceFile, or JSDocComment when lazily parsing JSDoc.
        assert!(self.parsing_contexts != 0, "Missing parsing context");
        for kind in ParsingContext::ALL {
            if self.parsing_contexts & (1 << kind as i32) != 0 {
                if self.is_list_element(kind, true /*inErrorRecovery*/) || self.is_list_terminator(kind) {
                    return true;
                }
            }
        }
        false
    }

    pub(crate) fn parsing_context_errors(&mut self, context: ParsingContext) {
        match context {
            ParsingContext::SourceElements => {
                if self.token == Kind::DefaultKeyword {
                    self.parse_error_at_current_token(&diagnostics::X_0_expected, &[&"export"]);
                } else {
                    self.parse_error_at_current_token(&diagnostics::Declaration_or_statement_expected, &[]);
                }
            }
            ParsingContext::BlockStatements => {
                self.parse_error_at_current_token(&diagnostics::Declaration_or_statement_expected, &[]);
            }
            ParsingContext::SwitchClauses => {
                self.parse_error_at_current_token(&diagnostics::X_case_or_default_expected, &[]);
            }
            ParsingContext::SwitchClauseStatements => {
                self.parse_error_at_current_token(&diagnostics::Statement_expected, &[]);
            }
            ParsingContext::RestProperties | ParsingContext::TypeMembers => {
                self.parse_error_at_current_token(&diagnostics::Property_or_signature_expected, &[]);
            }
            ParsingContext::ClassMembers => {
                self.parse_error_at_current_token(&diagnostics::Unexpected_token_A_constructor_method_accessor_or_property_was_expected, &[]);
            }
            ParsingContext::EnumMembers => {
                self.parse_error_at_current_token(&diagnostics::Enum_member_expected, &[]);
            }
            ParsingContext::HeritageClauseElement => {
                self.parse_error_at_current_token(&diagnostics::Expression_expected, &[]);
            }
            ParsingContext::VariableDeclarations => {
                if ast::is_keyword(self.token) {
                    let text = scanner::token_to_string(self.token);
                    self.parse_error_at_current_token(&diagnostics::X_0_is_not_allowed_as_a_variable_declaration_name, &[&text]);
                } else {
                    self.parse_error_at_current_token(&diagnostics::Variable_declaration_expected, &[]);
                }
            }
            ParsingContext::ObjectBindingElements => {
                self.parse_error_at_current_token(&diagnostics::Property_destructuring_pattern_expected, &[]);
            }
            ParsingContext::ArrayBindingElements => {
                self.parse_error_at_current_token(&diagnostics::Array_element_destructuring_pattern_expected, &[]);
            }
            ParsingContext::ArgumentExpressions => {
                self.parse_error_at_current_token(&diagnostics::Argument_expression_expected, &[]);
            }
            ParsingContext::ObjectLiteralMembers => {
                self.parse_error_at_current_token(&diagnostics::Property_assignment_expected, &[]);
            }
            ParsingContext::ArrayLiteralMembers => {
                self.parse_error_at_current_token(&diagnostics::Expression_or_comma_expected, &[]);
            }
            ParsingContext::JSDocParameters => {
                self.parse_error_at_current_token(&diagnostics::Parameter_declaration_expected, &[]);
            }
            ParsingContext::Parameters => {
                if ast::is_keyword(self.token) {
                    let text = scanner::token_to_string(self.token);
                    self.parse_error_at_current_token(&diagnostics::X_0_is_not_allowed_as_a_parameter_name, &[&text]);
                } else {
                    self.parse_error_at_current_token(&diagnostics::Parameter_declaration_expected, &[]);
                }
            }
            ParsingContext::TypeParameters => {
                self.parse_error_at_current_token(&diagnostics::Type_parameter_declaration_expected, &[]);
            }
            ParsingContext::TypeArguments => {
                self.parse_error_at_current_token(&diagnostics::Type_argument_expected, &[]);
            }
            ParsingContext::TupleElementTypes => {
                self.parse_error_at_current_token(&diagnostics::Type_expected, &[]);
            }
            ParsingContext::HeritageClauses => {
                self.parse_error_at_current_token(&diagnostics::Unexpected_token_expected, &[]);
            }
            ParsingContext::ImportOrExportSpecifiers => {
                if self.token == Kind::FromKeyword {
                    self.parse_error_at_current_token(&diagnostics::X_0_expected, &[&"}"]);
                } else {
                    self.parse_error_at_current_token(&diagnostics::Identifier_expected, &[]);
                }
            }
            ParsingContext::JsxAttributes | ParsingContext::JsxChildren | ParsingContext::JSDocComment => {
                self.parse_error_at_current_token(&diagnostics::Identifier_expected, &[]);
            }
            ParsingContext::ImportAttributes => {
                self.parse_error_at_current_token(&diagnostics::Identifier_or_string_literal_expected, &[]);
            }
            _ => panic!("Unhandled case in parsingContextErrors"),
        }
    }

    pub(crate) fn is_list_element(&mut self, parsing_context: ParsingContext, in_error_recovery: bool) -> bool {
        match parsing_context {
            ParsingContext::SourceElements | ParsingContext::BlockStatements | ParsingContext::SwitchClauseStatements => {
                // If we're in error recovery, then we don't want to treat ';' as an empty statement.
                // The problem is that ';' can show up in far too many contexts, and if we see one
                // and assume it's a statement, then we may bail out inappropriately from whatever
                // we're parsing.  For example, if we have a semicolon in the middle of a class, then
                // we really don't want to assume the class is over and we're on a statement in the
                // outer module.  We just want to consume and move on.
                !(self.token == Kind::SemicolonToken && in_error_recovery) && self.is_start_of_statement()
            }
            ParsingContext::SwitchClauses => self.token == Kind::CaseKeyword || self.token == Kind::DefaultKeyword,
            ParsingContext::TypeMembers => self.look_ahead(Parser::scan_type_member_start),
            ParsingContext::ClassMembers => {
                // We allow semicolons as class elements (as specified by ES6) as long as we're
                // not in error recovery.  If we're in error recovery, we don't want an errant
                // semicolon to be treated as a class member (since they're almost always used
                // for statements.
                self.look_ahead(Parser::scan_class_member_start) || self.token == Kind::SemicolonToken && !in_error_recovery
            }
            ParsingContext::EnumMembers => {
                // Include open bracket computed properties. This technically also lets in indexers,
                // which would be a candidate for improved error reporting.
                self.token == Kind::OpenBracketToken || self.is_literal_property_name()
            }
            ParsingContext::ObjectLiteralMembers => match self.token {
                // Not an object literal member, but don't want to close the object (see `tests/cases/fourslash/completionsDotInObjectLiteral.ts`)
                Kind::OpenBracketToken | Kind::AsteriskToken | Kind::DotDotDotToken | Kind::DotToken => true,
                _ => self.is_literal_property_name(),
            },
            ParsingContext::RestProperties => self.is_literal_property_name(),
            ParsingContext::ObjectBindingElements => {
                self.token == Kind::OpenBracketToken || self.token == Kind::DotDotDotToken || self.is_literal_property_name()
            }
            ParsingContext::ImportAttributes => self.is_import_attribute_name(),
            ParsingContext::HeritageClauseElement => {
                // If we see `{ ... }` then only consume it as an expression if it is followed by `,` or `{`
                // That way we won't consume the body of a class in its heritage clause.
                if self.token == Kind::OpenBraceToken {
                    return self.is_valid_heritage_clause_object_literal();
                }
                if !in_error_recovery {
                    return self.is_start_of_left_hand_side_expression() && !self.is_heritage_clause_extends_or_implements_keyword();
                }
                // If we're in error recovery we tighten up what we're willing to match.
                // That way we don't treat something like "this" as a valid heritage clause
                // element during recovery.
                self.is_identifier() && !self.is_heritage_clause_extends_or_implements_keyword()
            }
            ParsingContext::VariableDeclarations => self.is_binding_identifier_or_private_identifier_or_pattern(),
            ParsingContext::ArrayBindingElements => {
                self.token == Kind::CommaToken || self.token == Kind::DotDotDotToken || self.is_binding_identifier_or_private_identifier_or_pattern()
            }
            ParsingContext::TypeParameters => self.token == Kind::InKeyword || self.token == Kind::ConstKeyword || self.is_identifier(),
            ParsingContext::ArrayLiteralMembers | ParsingContext::ArgumentExpressions => {
                if parsing_context == ParsingContext::ArrayLiteralMembers {
                    // Not an array literal member, but don't want to close the array (see `tests/cases/fourslash/completionsDotInArrayLiteralInObjectLiteral.ts`)
                    if self.token == Kind::CommaToken || self.token == Kind::DotToken {
                        return true;
                    }
                }
                self.token == Kind::DotDotDotToken || self.is_start_of_expression()
            }
            ParsingContext::Parameters => self.is_start_of_parameter(false /*isJSDocParameter*/),
            ParsingContext::JSDocParameters => self.is_start_of_parameter(true /*isJSDocParameter*/),
            ParsingContext::TypeArguments | ParsingContext::TupleElementTypes => {
                self.token == Kind::CommaToken || self.is_start_of_type(false /*inStartOfParameter*/)
            }
            ParsingContext::HeritageClauses => self.is_heritage_clause(),
            ParsingContext::ImportOrExportSpecifiers => {
                // bail out if the next token is [FromKeyword StringLiteral].
                // That means we're in something like `import { from "mod"`. Stop here can give better error message.
                if self.token == Kind::FromKeyword && self.look_ahead(Parser::next_token_is_token_string_literal) {
                    return false;
                }
                if self.token == Kind::StringLiteral {
                    return true; // For "arbitrary module namespace identifiers"
                }
                token_is_identifier_or_keyword(self.token)
            }
            ParsingContext::JsxAttributes => token_is_identifier_or_keyword(self.token) || self.token == Kind::OpenBraceToken,
            ParsingContext::JsxChildren => true,
            ParsingContext::JSDocComment => true,
            ParsingContext::Count => panic!("Unhandled case in isListElement"),
        }
    }

    pub(crate) fn is_list_terminator(&mut self, kind: ParsingContext) -> bool {
        if self.token == Kind::EndOfFile {
            return true;
        }
        match kind {
            ParsingContext::BlockStatements
            | ParsingContext::SwitchClauses
            | ParsingContext::TypeMembers
            | ParsingContext::ClassMembers
            | ParsingContext::EnumMembers
            | ParsingContext::ObjectLiteralMembers
            | ParsingContext::ObjectBindingElements
            | ParsingContext::ImportOrExportSpecifiers
            | ParsingContext::ImportAttributes => self.token == Kind::CloseBraceToken,
            ParsingContext::SwitchClauseStatements => {
                self.token == Kind::CloseBraceToken || self.token == Kind::CaseKeyword || self.token == Kind::DefaultKeyword
            }
            ParsingContext::HeritageClauseElement => {
                self.token == Kind::OpenBraceToken || self.token == Kind::ExtendsKeyword || self.token == Kind::ImplementsKeyword
            }
            ParsingContext::VariableDeclarations => {
                // If we can consume a semicolon (either explicitly, or with ASI), then consider us done
                // with parsing the list of variable declarators.
                // In the case where we're parsing the variable declarator of a 'for-in' statement, we
                // are done if we see an 'in' keyword in front of us. Same with for-of
                // ERROR RECOVERY TWEAK:
                // For better error recovery, if we see an '=>' then we just stop immediately.  We've got an
                // arrow function here and it's going to be very unlikely that we'll resynchronize and get
                // another variable declaration.
                self.can_parse_semicolon()
                    || self.token == Kind::InKeyword
                    || self.token == Kind::OfKeyword
                    || self.token == Kind::EqualsGreaterThanToken
            }
            ParsingContext::TypeParameters => {
                // Tokens other than '>' are here for better error recovery
                self.token == Kind::GreaterThanToken
                    || self.token == Kind::OpenParenToken
                    || self.token == Kind::OpenBraceToken
                    || self.token == Kind::ExtendsKeyword
                    || self.token == Kind::ImplementsKeyword
            }
            ParsingContext::ArgumentExpressions => {
                // Tokens other than ')' are here for better error recovery
                self.token == Kind::CloseParenToken || self.token == Kind::SemicolonToken
            }
            ParsingContext::ArrayLiteralMembers | ParsingContext::TupleElementTypes | ParsingContext::ArrayBindingElements => {
                self.token == Kind::CloseBracketToken
            }
            ParsingContext::JSDocParameters | ParsingContext::Parameters | ParsingContext::RestProperties => {
                // Tokens other than ')' and ']' (the latter for index signatures) are here for better error recovery
                self.token == Kind::CloseParenToken || self.token == Kind::CloseBracketToken /*|| token == ast.KindOpenBraceToken*/
            }
            ParsingContext::TypeArguments => {
                // All other tokens should cause the type-argument to terminate except comma token
                self.token != Kind::CommaToken
            }
            ParsingContext::HeritageClauses => self.token == Kind::OpenBraceToken || self.token == Kind::CloseBraceToken,
            ParsingContext::JsxAttributes => self.token == Kind::GreaterThanToken || self.token == Kind::SlashToken,
            ParsingContext::JsxChildren => self.token == Kind::LessThanToken && self.look_ahead(Parser::next_token_is_slash),
            _ => false,
        }
    }

    pub(crate) fn parse_expected_jsdoc(&mut self, kind: Kind) -> bool {
        if self.token == kind {
            self.next_token_jsdoc();
            return true;
        }
        if !is_keyword_or_punctuation(kind) {
            panic!("Invalid JSDoc kind: expected keyword or punctuation");
        }
        self.parse_error_at_current_token(&diagnostics::X_0_expected, &[&scanner::token_to_string(kind)]);
        false
    }

    pub(crate) fn parse_expected_matching_brackets(&mut self, open_kind: Kind, close_kind: Kind, open_parsed: bool, open_position: i32) {
        if self.token == close_kind {
            self.next_token();
            return;
        }
        let last_error = self.parse_error_at_current_token(&diagnostics::X_0_expected, &[&scanner::token_to_string(close_kind)]);
        if !open_parsed {
            return;
        }
        if let Some(last_error) = last_error {
            let related = ast::new_diagnostic(
                None,
                new_text_range(open_position, open_position),
                &diagnostics::The_parser_expected_to_find_a_1_to_match_the_0_token_here,
                &[&scanner::token_to_string(open_kind), &scanner::token_to_string(close_kind)],
            );
            last_error.add_related_info(related);
        }
    }

    pub(crate) fn parse_optional(&mut self, token: Kind) -> bool {
        if self.token == token {
            self.next_token();
            return true;
        }
        false
    }

    pub(crate) fn parse_expected(&mut self, kind: Kind) -> bool {
        self.parse_expected_with_diagnostic(kind, None, true)
    }

    pub(crate) fn parse_expected_without_advancing(&mut self, kind: Kind) -> bool {
        self.parse_expected_with_diagnostic(kind, None, false)
    }

    pub(crate) fn parse_expected_with_diagnostic(&mut self, kind: Kind, message: Option<&'static Message>, should_advance: bool) -> bool {
        if self.token == kind {
            if should_advance {
                self.next_token();
            }
            return true;
        }
        // Report specific message if provided with one.  Otherwise, report generic fallback message.
        if let Some(message) = message {
            self.parse_error_at_current_token(message, &[]);
        } else {
            self.parse_error_at_current_token(&diagnostics::X_0_expected, &[&scanner::token_to_string(kind)]);
        }
        false
    }

    pub(crate) fn parse_token_node(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let kind = self.token;
        self.next_token();
        let token = self.factory.new_token(kind);
        self.finish_node(token, pos)
    }

    pub(crate) fn parse_expected_token(&mut self, kind: Kind) -> P<Node> {
        match self.parse_optional_token(kind) {
            Some(token) => token,
            None => {
                self.parse_error_at_current_token(&diagnostics::X_0_expected, &[&scanner::token_to_string(kind)]);
                let token = self.factory.new_token(kind);
                let pos = self.node_pos();
                self.finish_node(token, pos)
            }
        }
    }

    pub(crate) fn parse_optional_token(&mut self, kind: Kind) -> Option<P<Node>> {
        if self.token == kind {
            return Some(self.parse_token_node());
        }
        None
    }

    pub(crate) fn parse_expected_token_jsdoc(&mut self, kind: Kind) -> P<Node> {
        match self.parse_optional_token_jsdoc(kind) {
            Some(optional) => optional,
            None => {
                if !is_keyword_or_punctuation(kind) {
                    panic!("expected keyword or punctuation");
                }
                self.parse_error_at_current_token(&diagnostics::X_0_expected, &[&scanner::token_to_string(kind)]);
                let token = self.factory.new_token(kind);
                let pos = self.node_pos();
                self.finish_node(token, pos)
            }
        }
    }

    pub(crate) fn parse_optional_token_jsdoc(&mut self, kind: Kind) -> Option<P<Node>> {
        if self.token == kind {
            return Some(self.parse_token_node());
        }
        None
    }

    pub(crate) fn parse_statement(&mut self) -> P<Node> {
        match self.token {
            Kind::SemicolonToken => return self.parse_empty_statement(),
            Kind::OpenBraceToken => return self.parse_block(false /*ignoreMissingOpenBrace*/, None),
            Kind::VarKeyword => {
                let pos = self.node_pos();
                let jsdoc = self.jsdoc_scanner_info();
                return self.parse_variable_statement(pos, jsdoc, None /*modifiers*/);
            }
            Kind::LetKeyword => {
                if self.is_let_declaration() {
                    let pos = self.node_pos();
                    let jsdoc = self.jsdoc_scanner_info();
                    return self.parse_variable_statement(pos, jsdoc, None /*modifiers*/);
                }
            }
            Kind::AwaitKeyword => {
                if self.is_await_using_declaration() {
                    let pos = self.node_pos();
                    let jsdoc = self.jsdoc_scanner_info();
                    return self.parse_variable_statement(pos, jsdoc, None /*modifiers*/);
                }
            }
            Kind::UsingKeyword => {
                if self.is_using_declaration() {
                    let pos = self.node_pos();
                    let jsdoc = self.jsdoc_scanner_info();
                    return self.parse_variable_statement(pos, jsdoc, None /*modifiers*/);
                }
            }
            Kind::FunctionKeyword => {
                let pos = self.node_pos();
                let jsdoc = self.jsdoc_scanner_info();
                return self.parse_function_declaration(pos, jsdoc, None /*modifiers*/);
            }
            Kind::ClassKeyword => {
                let pos = self.node_pos();
                let jsdoc = self.jsdoc_scanner_info();
                return self.parse_class_declaration(pos, jsdoc, None /*modifiers*/);
            }
            Kind::IfKeyword => return self.parse_if_statement(),
            Kind::DoKeyword => return self.parse_do_statement(),
            Kind::WhileKeyword => return self.parse_while_statement(),
            Kind::ForKeyword => return self.parse_for_or_for_in_or_for_of_statement(),
            Kind::ContinueKeyword => return self.parse_continue_statement(),
            Kind::BreakKeyword => return self.parse_break_statement(),
            Kind::ReturnKeyword => return self.parse_return_statement(),
            Kind::WithKeyword => return self.parse_with_statement(),
            Kind::SwitchKeyword => return self.parse_switch_statement(),
            Kind::ThrowKeyword => return self.parse_throw_statement(),
            Kind::TryKeyword | Kind::CatchKeyword | Kind::FinallyKeyword => return self.parse_try_statement(),
            Kind::DebuggerKeyword => return self.parse_debugger_statement(),
            Kind::AtToken => return self.parse_declaration(),
            Kind::AsyncKeyword
            | Kind::InterfaceKeyword
            | Kind::TypeKeyword
            | Kind::ModuleKeyword
            | Kind::NamespaceKeyword
            | Kind::DeclareKeyword
            | Kind::ConstKeyword
            | Kind::EnumKeyword
            | Kind::ExportKeyword
            | Kind::ImportKeyword
            | Kind::PrivateKeyword
            | Kind::ProtectedKeyword
            | Kind::PublicKeyword
            | Kind::AbstractKeyword
            | Kind::AccessorKeyword
            | Kind::StaticKeyword
            | Kind::ReadonlyKeyword
            | Kind::GlobalKeyword => {
                if self.is_start_of_declaration() {
                    return self.parse_declaration();
                }
            }
            _ => {}
        }
        self.parse_expression_or_labeled_statement()
    }

    pub(crate) fn parse_declaration(&mut self) -> P<Node> {
        // `parseListElement` attempted to get the reused node at this position,
        // but the ambient context flag was not yet set, so the node appeared
        // not reusable in that context.
        let pos = self.node_pos();
        let jsdoc = self.jsdoc_scanner_info();
        let modifiers = self.parse_modifiers_ex(true /*allowDecorators*/, false /*permitConstAsModifier*/, false /*stopOnStartOfClassStaticBlock*/);
        let is_ambient = modifiers.is_some_and(|modifiers| modifiers.nodes().iter().any(|m| is_declare_modifier(*m)));
        if is_ambient {
            // !!! incremental parsing
            // node := p.tryReuseAmbientDeclaration(pos)
            // if node {
            // 	return node
            // }
            for m in modifiers.unwrap().nodes() {
                m.set_flags(m.flags() | NodeFlags::Ambient);
            }
            let save_context_flags = self.context_flags;
            self.set_context_flags(NodeFlags::Ambient, true);
            let result = self.parse_declaration_worker(pos, jsdoc, modifiers);
            self.context_flags = save_context_flags;
            result
        } else {
            self.parse_declaration_worker(pos, jsdoc, modifiers)
        }
    }

    pub(crate) fn parse_declaration_worker(&mut self, pos: i32, jsdoc: JsdocScannerInfo, modifiers: Option<P<ModifierList>>) -> P<Node> {
        match self.token {
            Kind::VarKeyword | Kind::LetKeyword | Kind::ConstKeyword | Kind::UsingKeyword => {
                return self.parse_variable_statement(pos, jsdoc, modifiers);
            }
            Kind::AwaitKeyword => {
                if self.is_await_using_declaration() {
                    return self.parse_variable_statement(pos, jsdoc, modifiers);
                }
            }
            Kind::FunctionKeyword => return self.parse_function_declaration(pos, jsdoc, modifiers),
            Kind::ClassKeyword => return self.parse_class_declaration(pos, jsdoc, modifiers),
            Kind::InterfaceKeyword => return self.parse_interface_declaration(pos, jsdoc, modifiers),
            Kind::TypeKeyword => return self.parse_type_alias_declaration(pos, jsdoc, modifiers),
            Kind::EnumKeyword => return self.parse_enum_declaration(pos, jsdoc, modifiers),
            Kind::GlobalKeyword | Kind::ModuleKeyword | Kind::NamespaceKeyword => {
                return self.parse_module_declaration(pos, jsdoc, modifiers);
            }
            Kind::ImportKeyword => return self.parse_import_declaration_or_import_equals_declaration(pos, jsdoc, modifiers),
            Kind::ExportKeyword => {
                self.next_token();
                return match self.token {
                    Kind::DefaultKeyword | Kind::EqualsToken => self.parse_export_assignment(pos, jsdoc, modifiers),
                    Kind::AsKeyword => self.parse_namespace_export_declaration(pos, jsdoc, modifiers),
                    _ => self.parse_export_declaration(pos, jsdoc, modifiers),
                };
            }
            _ => {}
        }
        if modifiers.is_some() {
            // We reached this point because we encountered decorators and/or modifiers and assumed a declaration
            // would follow. For recovery and error reporting purposes, return an incomplete declaration.
            let node_pos = self.node_pos();
            self.parse_error_at(node_pos, node_pos, &diagnostics::Declaration_expected, &[]);
            let result = self.factory.new_missing_declaration(modifiers);
            return self.finish_node(result, pos);
        }
        panic!("Unhandled case in parseDeclarationWorker");
    }
}

pub(crate) fn is_declare_modifier(modifier: P<Node>) -> bool {
    modifier.kind() == Kind::DeclareKeyword
}

impl Parser {
    pub(crate) fn is_let_declaration(&mut self) -> bool {
        // In ES6 'let' always starts a lexical declaration if followed by an identifier or {
        // or [.
        self.look_ahead(Parser::next_token_is_binding_identifier_or_start_of_destructuring)
    }

    pub(crate) fn next_token_is_binding_identifier_or_start_of_destructuring(&mut self) -> bool {
        self.next_token();
        self.is_binding_identifier() || self.token == Kind::OpenBraceToken || self.token == Kind::OpenBracketToken
    }

    pub(crate) fn parse_block(&mut self, ignore_missing_open_brace: bool, diagnostic_message: Option<&'static Message>) -> P<Node> {
        let pos = self.node_pos();
        let jsdoc = self.jsdoc_scanner_info();
        let open_brace_position = self.scanner.token_start();
        let open_brace_parsed = self.parse_expected_with_diagnostic(Kind::OpenBraceToken, diagnostic_message, true /*shouldAdvance*/);
        let mut multiline = false;
        if open_brace_parsed || ignore_missing_open_brace {
            multiline = self.has_preceding_line_break();
            let statements = self.parse_list(ParsingContext::BlockStatements, Parser::parse_statement);
            self.parse_expected_matching_brackets(Kind::OpenBraceToken, Kind::CloseBraceToken, open_brace_parsed, open_brace_position);
            let result = self.factory.new_block(statements, multiline);
            let result = self.finish_node(result, pos);
            self.with_jsdoc(result, jsdoc);
            if self.token == Kind::EqualsToken {
                self.parse_error_at_current_token(&diagnostics::Declaration_or_statement_expected_This_follows_a_block_of_statements_so_if_you_intended_to_write_a_destructuring_assignment_you_might_need_to_wrap_the_whole_assignment_in_parentheses, &[]);
                self.next_token();
            }
            return result;
        }
        let statements = self.create_missing_list();
        let result = self.factory.new_block(statements, multiline);
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        result
    }

    pub(crate) fn parse_empty_statement(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let jsdoc = self.jsdoc_scanner_info();
        self.parse_expected(Kind::SemicolonToken);
        let result = self.factory.new_empty_statement();
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        result
    }

    pub(crate) fn parse_if_statement(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let jsdoc = self.jsdoc_scanner_info();
        self.parse_expected(Kind::IfKeyword);
        let open_paren_position = self.scanner.token_start();
        let open_paren_parsed = self.parse_expected(Kind::OpenParenToken);
        let expression = self.parse_expression_allow_in();
        self.parse_expected_matching_brackets(Kind::OpenParenToken, Kind::CloseParenToken, open_paren_parsed, open_paren_position);
        let then_statement = self.parse_statement();
        let mut else_statement = None;
        if self.parse_optional(Kind::ElseKeyword) {
            else_statement = Some(self.parse_statement());
        }
        let result = self.factory.new_if_statement(expression, then_statement, else_statement);
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        result
    }

    pub(crate) fn parse_do_statement(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let jsdoc = self.jsdoc_scanner_info();
        self.parse_expected(Kind::DoKeyword);
        let statement = self.parse_statement();
        self.parse_expected(Kind::WhileKeyword);
        let open_paren_position = self.scanner.token_start();
        let open_paren_parsed = self.parse_expected(Kind::OpenParenToken);
        let expression = self.parse_expression_allow_in();
        self.parse_expected_matching_brackets(Kind::OpenParenToken, Kind::CloseParenToken, open_paren_parsed, open_paren_position);
        // From: https://mail.mozilla.org/pipermail/es-discuss/2011-August/016188.html
        // 157 min --- All allen at wirfs-brock.com CONF --- "do{;}while(false)false" prohibited in
        // spec but allowed in consensus reality. Approved -- this is the de-facto standard whereby
        //  do;while(0)x will have a semicolon inserted before x.
        self.parse_optional(Kind::SemicolonToken);
        let result = self.factory.new_do_statement(statement, expression);
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        result
    }

    pub(crate) fn parse_while_statement(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let jsdoc = self.jsdoc_scanner_info();
        self.parse_expected(Kind::WhileKeyword);
        let open_paren_position = self.scanner.token_start();
        let open_paren_parsed = self.parse_expected(Kind::OpenParenToken);
        let expression = self.parse_expression_allow_in();
        self.parse_expected_matching_brackets(Kind::OpenParenToken, Kind::CloseParenToken, open_paren_parsed, open_paren_position);
        let statement = self.parse_statement();
        let result = self.factory.new_while_statement(expression, statement);
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        result
    }

    pub(crate) fn parse_for_or_for_in_or_for_of_statement(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let jsdoc = self.jsdoc_scanner_info();
        self.parse_expected(Kind::ForKeyword);
        let await_token = self.parse_optional_token(Kind::AwaitKeyword);
        self.parse_expected(Kind::OpenParenToken);
        let mut initializer: Option<P<Node>> = None;
        if self.token != Kind::SemicolonToken {
            if self.token == Kind::VarKeyword
                || self.token == Kind::LetKeyword
                || self.token == Kind::ConstKeyword
                || self.token == Kind::UsingKeyword
                    && self.look_ahead(Parser::next_token_is_binding_identifier_or_start_of_destructuring_on_same_line_disallow_of)
                // this one is meant to allow of
                || self.token == Kind::AwaitKeyword
                    && self.look_ahead(Parser::next_is_using_keyword_then_binding_identifier_or_start_of_object_destructuring_on_same_line)
            {
                initializer = Some(self.parse_variable_declaration_list(true /*inForStatementInitializer*/));
            } else {
                initializer = Some(self.do_in_context(NodeFlags::DisallowInContext, true, Parser::parse_expression));
            }
        }
        let result: P<Node>;
        if await_token.is_some() && self.parse_expected(Kind::OfKeyword) || await_token.is_none() && self.parse_optional(Kind::OfKeyword) {
            let expression = self.do_in_context(NodeFlags::DisallowInContext, false, Parser::parse_assignment_expression_or_higher);
            self.parse_expected(Kind::CloseParenToken);
            let statement = self.parse_statement();
            result = self.factory.new_for_in_or_of_statement(Kind::ForOfStatement, await_token, initializer.unwrap(), expression, statement);
        } else if self.parse_optional(Kind::InKeyword) {
            let expression = self.parse_expression_allow_in();
            self.parse_expected(Kind::CloseParenToken);
            let statement = self.parse_statement();
            result = self.factory.new_for_in_or_of_statement(Kind::ForInStatement, None /*awaitToken*/, initializer.unwrap(), expression, statement);
        } else {
            self.parse_expected(Kind::SemicolonToken);
            let mut condition = None;
            if self.token != Kind::SemicolonToken && self.token != Kind::CloseParenToken {
                condition = Some(self.parse_expression_allow_in());
            }
            self.parse_expected(Kind::SemicolonToken);
            let mut incrementor = None;
            if self.token != Kind::CloseParenToken {
                incrementor = Some(self.parse_expression_allow_in());
            }
            self.parse_expected(Kind::CloseParenToken);
            let statement = self.parse_statement();
            result = self.factory.new_for_statement(initializer, condition, incrementor, statement);
        }
        self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        result
    }

    pub(crate) fn parse_break_statement(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let jsdoc = self.jsdoc_scanner_info();
        self.parse_expected(Kind::BreakKeyword);
        let label = self.parse_identifier_unless_at_semicolon();
        self.parse_semicolon();
        let result = self.factory.new_break_statement(label);
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        result
    }

    pub(crate) fn parse_continue_statement(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let jsdoc = self.jsdoc_scanner_info();
        self.parse_expected(Kind::ContinueKeyword);
        let label = self.parse_identifier_unless_at_semicolon();
        self.parse_semicolon();
        let result = self.factory.new_continue_statement(label);
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        result
    }

    pub(crate) fn parse_identifier_unless_at_semicolon(&mut self) -> Option<P<Node>> {
        if !self.can_parse_semicolon() {
            return Some(self.parse_identifier());
        }
        None
    }

    pub(crate) fn parse_return_statement(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let jsdoc = self.jsdoc_scanner_info();
        self.parse_expected(Kind::ReturnKeyword);
        let mut expression = None;
        if !self.can_parse_semicolon() {
            expression = Some(self.parse_expression_allow_in());
        }
        self.parse_semicolon();
        let result = self.factory.new_return_statement(expression);
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        result
    }

    pub(crate) fn parse_with_statement(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let jsdoc = self.jsdoc_scanner_info();
        self.parse_expected(Kind::WithKeyword);
        let open_paren_position = self.scanner.token_start();
        let open_paren_parsed = self.parse_expected(Kind::OpenParenToken);
        let expression = self.parse_expression_allow_in();
        self.parse_expected_matching_brackets(Kind::OpenParenToken, Kind::CloseParenToken, open_paren_parsed, open_paren_position);
        let statement = self.do_in_context(NodeFlags::InWithStatement, true, Parser::parse_statement);
        let result = self.factory.new_with_statement(expression, statement);
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        result
    }

    pub(crate) fn parse_case_clause(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let jsdoc = self.jsdoc_scanner_info();
        self.parse_expected(Kind::CaseKeyword);
        let expression = self.parse_expression_allow_in();
        self.parse_expected(Kind::ColonToken);
        let statements = self.parse_list(ParsingContext::SwitchClauseStatements, Parser::parse_statement);
        let result = self.factory.new_case_or_default_clause(Kind::CaseClause, Some(expression), statements);
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        result
    }

    pub(crate) fn parse_default_clause(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let jsdoc = self.jsdoc_scanner_info();
        self.parse_expected(Kind::DefaultKeyword);
        self.parse_expected(Kind::ColonToken);
        let statements = self.parse_list(ParsingContext::SwitchClauseStatements, Parser::parse_statement);
        let result = self.factory.new_case_or_default_clause(Kind::DefaultClause, None /*expression*/, statements);
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        result
    }

    pub(crate) fn parse_case_or_default_clause(&mut self) -> P<Node> {
        if self.token == Kind::CaseKeyword {
            return self.parse_case_clause();
        }
        self.parse_default_clause()
    }

    pub(crate) fn parse_case_block(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let jsdoc = self.jsdoc_scanner_info();
        self.parse_expected(Kind::OpenBraceToken);
        let clauses = self.parse_list(ParsingContext::SwitchClauses, Parser::parse_case_or_default_clause);
        self.parse_expected(Kind::CloseBraceToken);
        let result = self.factory.new_case_block(clauses);
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        result
    }

    pub(crate) fn parse_switch_statement(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let jsdoc = self.jsdoc_scanner_info();
        self.parse_expected(Kind::SwitchKeyword);
        self.parse_expected(Kind::OpenParenToken);
        let expression = self.parse_expression_allow_in();
        self.parse_expected(Kind::CloseParenToken);
        let case_block = self.parse_case_block();
        let result = self.factory.new_switch_statement(expression, case_block);
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        result
    }

    pub(crate) fn parse_throw_statement(&mut self) -> P<Node> {
        // ThrowStatement[Yield] :
        //      throw [no LineTerminator here]Expression[In, ?Yield];
        let pos = self.node_pos();
        let jsdoc = self.jsdoc_scanner_info();
        self.parse_expected(Kind::ThrowKeyword);
        // Because of automatic semicolon insertion, we need to report error if this
        // throw could be terminated with a semicolon.  Note: we can't call 'parseExpression'
        // directly as that might consume an expression on the following line.
        // Instead, we create a "missing" identifier, but don't report an error. The actual error
        // will be reported in the grammar walker.
        let expression = if !self.has_preceding_line_break() {
            self.parse_expression_allow_in()
        } else {
            self.create_missing_identifier()
        };
        if !self.try_parse_semicolon() {
            self.parse_error_for_missing_semicolon_after(expression);
        }
        let result = self.factory.new_throw_statement(expression);
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        result
    }

    // TODO: Review for error recovery
    pub(crate) fn parse_try_statement(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let jsdoc = self.jsdoc_scanner_info();
        self.parse_expected(Kind::TryKeyword);
        let try_block = self.parse_block(false /*ignoreMissingOpenBrace*/, None);
        let mut catch_clause = None;
        if self.token == Kind::CatchKeyword {
            catch_clause = Some(self.parse_catch_clause());
        }
        // If we don't have a catch clause, then we must have a finally clause.  Try to parse
        // one out no matter what.
        let mut finally_block = None;
        if catch_clause.is_none() || self.token == Kind::FinallyKeyword {
            self.parse_expected_with_diagnostic(Kind::FinallyKeyword, Some(&diagnostics::X_catch_or_finally_expected), true /*shouldAdvance*/);
            finally_block = Some(self.parse_block(false /*ignoreMissingOpenBrace*/, None));
        }
        let result = self.factory.new_try_statement(try_block, catch_clause, finally_block);
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        result
    }

    pub(crate) fn parse_catch_clause(&mut self) -> P<Node> {
        let pos = self.node_pos();
        self.parse_expected(Kind::CatchKeyword);
        let mut variable_declaration = None;
        if self.parse_optional(Kind::OpenParenToken) {
            variable_declaration = Some(self.parse_variable_declaration());
            self.parse_expected(Kind::CloseParenToken);
        }
        let block = self.parse_block(false /*ignoreMissingOpenBrace*/, None);
        let result = self.factory.new_catch_clause(variable_declaration, block);
        self.finish_node(result, pos)
    }

    pub(crate) fn parse_debugger_statement(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let jsdoc = self.jsdoc_scanner_info();
        self.parse_expected(Kind::DebuggerKeyword);
        self.parse_semicolon();
        let result = self.factory.new_debugger_statement();
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        result
    }

    pub(crate) fn parse_expression_or_labeled_statement(&mut self) -> P<Node> {
        // Avoiding having to do the lookahead for a labeled statement by just trying to parse
        // out an expression, seeing if it is identifier and then seeing if it is followed by
        // a colon.
        let pos = self.node_pos();
        let mut jsdoc = self.jsdoc_scanner_info();
        let has_paren = self.token == Kind::OpenParenToken;
        let expression = self.parse_expression();

        if expression.kind() == Kind::Identifier && self.parse_optional(Kind::ColonToken) {
            let statement = self.parse_statement();
            let result = self.factory.new_labeled_statement(expression, statement);
            let result = self.finish_node(result, pos);
            self.with_jsdoc(result, jsdoc);
            return result;
        }

        if !self.try_parse_semicolon() {
            self.parse_error_for_missing_semicolon_after(expression);
        }
        let result = self.factory.new_expression_statement(expression);
        let result = self.finish_node(result, pos);
        if has_paren {
            jsdoc.remove(JsdocScannerInfo::HasJSDoc);
        }
        self.with_jsdoc(result, jsdoc);
        result
    }

    pub(crate) fn parse_variable_statement(&mut self, pos: i32, jsdoc: JsdocScannerInfo, modifiers: Option<P<ModifierList>>) -> P<Node> {
        let declaration_list = self.parse_variable_declaration_list(false /*inForStatementInitializer*/);
        self.parse_semicolon();
        let result = self.factory.new_variable_statement(modifiers, declaration_list);
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        self.check_js_syntax(result);
        result
    }

    pub(crate) fn parse_variable_declaration_list(&mut self, in_for_statement_initializer: bool) -> P<Node> {
        let pos = self.node_pos();
        let mut flags = NodeFlags::None;
        match self.token {
            Kind::VarKeyword => flags = NodeFlags::None,
            Kind::LetKeyword => flags = NodeFlags::Let,
            Kind::ConstKeyword => flags = NodeFlags::Const,
            Kind::UsingKeyword => flags = NodeFlags::Using,
            Kind::AwaitKeyword => {
                if self.is_await_using_declaration() {
                    flags = NodeFlags::AwaitUsing;
                    self.next_token();
                }
            }
            _ => panic!("Unhandled case in parseVariableDeclarationList"),
        }
        self.next_token();
        // The user may have written the following:
        //
        //    for (let of X) { }
        //
        // In this case, we want to parse an empty declaration list, and then parse 'of'
        // as a keyword. The reason this is not automatic is that 'of' is a valid identifier.
        // So we need to look ahead to determine if 'of' should be treated as a keyword in
        // this context.
        // The checker will then give an error that there is an empty declaration list.
        let declarations: P<NodeList>;
        if self.token == Kind::OfKeyword && self.look_ahead(Parser::next_is_identifier_and_close_paren) {
            declarations = self.create_missing_list();
        } else {
            let save_context_flags = self.context_flags;
            self.set_context_flags(NodeFlags::DisallowInContext, in_for_statement_initializer);
            let parse_element: fn(&mut Parser) -> P<Node> = if in_for_statement_initializer {
                Parser::parse_variable_declaration
            } else {
                Parser::parse_variable_declaration_allow_exclamation
            };
            declarations = self.parse_delimited_list(ParsingContext::VariableDeclarations, parse_element).unwrap();
            self.context_flags = save_context_flags;
        }
        let result = self.factory.new_variable_declaration_list(declarations, flags);
        self.finish_node(result, pos)
    }

    pub(crate) fn next_is_identifier_and_close_paren(&mut self) -> bool {
        self.next_token_is_identifier() && self.next_token() == Kind::CloseParenToken
    }

    pub(crate) fn next_token_is_identifier(&mut self) -> bool {
        self.next_token();
        self.is_identifier()
    }

    pub(crate) fn parse_variable_declaration(&mut self) -> P<Node> {
        self.parse_variable_declaration_worker(false /*allowExclamation*/)
    }

    pub(crate) fn parse_variable_declaration_allow_exclamation(&mut self) -> P<Node> {
        self.parse_variable_declaration_worker(true /*allowExclamation*/)
    }

    pub(crate) fn parse_variable_declaration_worker(&mut self, allow_exclamation: bool) -> P<Node> {
        let pos = self.node_pos();
        let jsdoc = self.jsdoc_scanner_info();
        let name = self.parse_identifier_or_pattern_with_diagnostic(Some(&diagnostics::Private_identifiers_are_not_allowed_in_variable_declarations));
        let mut exclamation_token = None;
        if allow_exclamation && name.kind() == Kind::Identifier && self.token == Kind::ExclamationToken && !self.has_preceding_line_break() {
            exclamation_token = Some(self.parse_token_node());
        }
        let type_node = self.parse_type_annotation();
        let mut initializer = None;
        if self.token != Kind::InKeyword && self.token != Kind::OfKeyword {
            initializer = self.parse_initializer();
        }
        let result = self.factory.new_variable_declaration(name, exclamation_token, type_node, initializer);
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        self.check_js_syntax(result);
        result
    }

    pub(crate) fn parse_identifier_or_pattern(&mut self) -> P<Node> {
        self.parse_identifier_or_pattern_with_diagnostic(None)
    }

    pub(crate) fn parse_identifier_or_pattern_with_diagnostic(&mut self, private_identifier_diagnostic_message: Option<&'static Message>) -> P<Node> {
        if self.token == Kind::OpenBracketToken {
            return self.parse_array_binding_pattern();
        }
        if self.token == Kind::OpenBraceToken {
            return self.parse_object_binding_pattern();
        }
        self.parse_binding_identifier_with_diagnostic(private_identifier_diagnostic_message)
    }

    pub(crate) fn parse_array_binding_pattern(&mut self) -> P<Node> {
        let pos = self.node_pos();
        self.parse_expected(Kind::OpenBracketToken);
        let save_context_flags = self.context_flags;
        self.set_context_flags(NodeFlags::DisallowInContext, false);
        let elements = self.parse_delimited_list(ParsingContext::ArrayBindingElements, Parser::parse_array_binding_element).unwrap();
        self.context_flags = save_context_flags;
        self.parse_expected(Kind::CloseBracketToken);
        let result = self.factory.new_binding_pattern(Kind::ArrayBindingPattern, elements);
        self.finish_node(result, pos)
    }

    pub(crate) fn parse_array_binding_element(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let mut dot_dot_dot_token = None;
        let mut name = None;
        let mut initializer = None;
        if self.token != Kind::CommaToken {
            // These are all nil for a missing element
            dot_dot_dot_token = self.parse_optional_token(Kind::DotDotDotToken);
            name = Some(self.parse_identifier_or_pattern());
            initializer = self.parse_initializer();
        }
        let result = self.factory.new_binding_element(dot_dot_dot_token, None /*propertyName*/, name, initializer);
        self.finish_node(result, pos)
    }

    pub(crate) fn parse_object_binding_pattern(&mut self) -> P<Node> {
        let pos = self.node_pos();
        self.parse_expected(Kind::OpenBraceToken);
        let save_context_flags = self.context_flags;
        self.set_context_flags(NodeFlags::DisallowInContext, false);
        let elements = self.parse_delimited_list(ParsingContext::ObjectBindingElements, Parser::parse_object_binding_element).unwrap();
        self.context_flags = save_context_flags;
        self.parse_expected(Kind::CloseBraceToken);
        let result = self.factory.new_binding_pattern(Kind::ObjectBindingPattern, elements);
        self.finish_node(result, pos)
    }

    pub(crate) fn parse_object_binding_element(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let dot_dot_dot_token = self.parse_optional_token(Kind::DotDotDotToken);
        let token_is_identifier = self.is_binding_identifier();
        let mut property_name = Some(self.parse_property_name());
        let name;
        if token_is_identifier && self.token != Kind::ColonToken {
            name = property_name;
            property_name = None;
        } else {
            self.parse_expected(Kind::ColonToken);
            name = Some(self.parse_identifier_or_pattern());
        }
        let initializer = self.parse_initializer();
        let result = self.factory.new_binding_element(dot_dot_dot_token, property_name, name, initializer);
        self.finish_node(result, pos)
    }

    pub(crate) fn parse_initializer(&mut self) -> Option<P<Node>> {
        if self.parse_optional(Kind::EqualsToken) {
            return Some(self.parse_assignment_expression_or_higher());
        }
        None
    }

    pub(crate) fn parse_type_annotation(&mut self) -> Option<P<Node>> {
        if self.parse_optional(Kind::ColonToken) {
            return Some(self.parse_type());
        }
        None
    }

    pub(crate) fn parse_function_declaration(&mut self, pos: i32, jsdoc: JsdocScannerInfo, modifiers: Option<P<ModifierList>>) -> P<Node> {
        self.parse_expected(Kind::FunctionKeyword);
        let asterisk_token = self.parse_optional_token(Kind::AsteriskToken);
        // We don't parse the name here in await context, instead we will report a grammar error in the checker.
        let mut name = None;
        if modifiers.is_none_or(|modifiers| !modifiers.modifier_flags.intersects(ModifierFlags::Default)) || self.is_binding_identifier() {
            name = Some(self.parse_binding_identifier());
        }
        let signature_flags = (if asterisk_token.is_some() { ParseFlags::Yield } else { ParseFlags::None })
            | (if modifiers.is_some_and(|modifiers| modifiers.modifier_flags.intersects(ModifierFlags::Async)) {
                ParseFlags::Await
            } else {
                ParseFlags::None
            });
        let type_parameters = self.parse_type_parameters();
        let save_context_flags = self.context_flags;
        if modifiers.is_some_and(|modifiers| modifiers.modifier_flags.intersects(ModifierFlags::Export)) {
            self.set_context_flags(NodeFlags::AwaitContext, true);
        }
        let parameters = self.parse_parameters(signature_flags);
        let return_type = self.parse_return_type(Kind::ColonToken, false /*isType*/);
        let body = self.parse_function_block_or_semicolon(signature_flags, Some(&diagnostics::X_or_expected));
        self.context_flags = save_context_flags;
        let result = self.factory.new_function_declaration(modifiers, asterisk_token, name, type_parameters, Some(parameters), return_type, None /*fullSignature*/, body);
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        self.check_js_syntax(result);
        result
    }

    pub(crate) fn parse_class_declaration(&mut self, pos: i32, jsdoc: JsdocScannerInfo, modifiers: Option<P<ModifierList>>) -> P<Node> {
        self.parse_class_declaration_or_expression(pos, jsdoc, modifiers, Kind::ClassDeclaration)
    }

    pub(crate) fn parse_class_expression(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let jsdoc = self.jsdoc_scanner_info();
        self.parse_class_declaration_or_expression(pos, jsdoc, None /*modifiers*/, Kind::ClassExpression)
    }

    pub(crate) fn parse_class_declaration_or_expression(
        &mut self,
        pos: i32,
        jsdoc: JsdocScannerInfo,
        modifiers: Option<P<ModifierList>>,
        kind: Kind,
    ) -> P<Node> {
        let save_context_flags = self.context_flags;
        let save_has_await_identifier = self.statement_has_await_identifier;
        self.parse_expected(Kind::ClassKeyword);
        // We don't parse the name here in await context, instead we will report a grammar error in the checker.
        let name = self.parse_name_of_class_declaration_or_expression();
        let type_parameters = self.parse_type_parameters();
        if modifiers.is_some()
            && self.parsing_contexts & (1 << ParsingContext::SourceElements as i32) != 0
            && self.parsing_contexts & ((1 << ParsingContext::BlockStatements as i32) | (1 << ParsingContext::SwitchClauseStatements as i32)) == 0
            && modifiers.unwrap().nodes().iter().any(|m| is_export_modifier(*m))
        {
            self.set_context_flags(NodeFlags::AwaitContext, true /*value*/);
        }
        let heritage_clauses = self.parse_heritage_clauses(false /*isInterface*/);
        let members: P<NodeList>;
        if self.parse_expected(Kind::OpenBraceToken) {
            // ClassTail[Yield,Await] : (Modified) See 14.5
            //      ClassHeritage[?Yield,?Await]opt { ClassBody[?Yield,?Await]opt }
            members = self.parse_list(ParsingContext::ClassMembers, Parser::parse_class_element);
            self.parse_expected(Kind::CloseBraceToken);
        } else {
            members = self.create_missing_list();
        }
        self.context_flags = save_context_flags;
        if modifiers.is_some_and(|modifiers| ast::modifiers_to_flags(modifiers.nodes()).intersects(ModifierFlags::Ambient)) {
            self.statement_has_await_identifier = save_has_await_identifier;
        }
        let result = if kind == Kind::ClassDeclaration {
            self.factory.new_class_declaration(modifiers, name, type_parameters, heritage_clauses, members)
        } else {
            self.factory.new_class_expression(modifiers, name, type_parameters, heritage_clauses, members)
        };
        self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        if result.flags().intersects(NodeFlags::JavaScriptFile) {
            self.check_js_syntax(result);
            if let Some(heritage_clauses) = heritage_clauses {
                for clause in heritage_clauses.nodes {
                    let clause = clause.as_heritage_clause();
                    if clause.token() == Kind::ExtendsKeyword {
                        for expr in clause.types().nodes {
                            self.check_js_syntax(*expr);
                        }
                    }
                }
            }
        }
        result
    }

    pub(crate) fn parse_name_of_class_declaration_or_expression(&mut self) -> Option<P<Node>> {
        // implements is a future reserved word so
        // 'class implements' might mean either
        // - class expression with omitted name, 'implements' starts heritage clause
        // - class with name 'implements'
        // 'isImplementsClause' helps to disambiguate between these two cases
        if self.is_binding_identifier() && !self.is_implements_clause() {
            let save_has_await_identifier = self.statement_has_await_identifier;
            let is_binding_identifier = self.is_binding_identifier();
            let id = self.create_identifier(is_binding_identifier);
            self.statement_has_await_identifier = save_has_await_identifier;
            return Some(id);
        }
        None
    }

    pub(crate) fn is_implements_clause(&mut self) -> bool {
        self.token == Kind::ImplementsKeyword && self.look_ahead(Parser::next_token_is_identifier_or_keyword)
    }
}

pub(crate) fn is_export_modifier(modifier: P<Node>) -> bool {
    modifier.kind() == Kind::ExportKeyword
}

pub(crate) fn is_async_modifier(modifier: P<Node>) -> bool {
    modifier.kind() == Kind::AsyncKeyword
}

impl Parser {
    pub(crate) fn parse_heritage_clauses(&mut self, is_interface: bool) -> Option<P<NodeList>> {
        // ClassTail[Yield,Await] : (Modified) See 14.5
        //      ClassHeritage[?Yield,?Await]opt { ClassBody[?Yield,?Await]opt }
        if self.is_heritage_clause() {
            return Some(self.parse_list(ParsingContext::HeritageClauses, |p| p.parse_heritage_clause(is_interface)));
        }
        None
    }

    pub(crate) fn parse_heritage_clause(&mut self, is_interface: bool) -> P<Node> {
        let pos = self.node_pos();
        let kind = self.token;
        self.next_token();
        let mut parse_element: fn(&mut Parser) -> P<Node> = Parser::parse_expression_with_type_arguments;
        if is_type_heritage_clause(is_interface, kind) {
            parse_element = Parser::parse_type_heritage_clause_element;
        }
        let types = self.parse_delimited_list(ParsingContext::HeritageClauseElement, parse_element).unwrap();
        let result = self.factory.new_heritage_clause(kind, types);
        let result = self.finish_node(result, pos);
        self.check_js_syntax(result)
    }
}

pub(crate) fn is_type_heritage_clause(is_interface: bool, token: Kind) -> bool {
    is_interface && token == Kind::ExtendsKeyword || !is_interface && token == Kind::ImplementsKeyword
}

impl Parser {
    pub(crate) fn parse_type_heritage_clause_element(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let node = self.parse_expression_with_type_arguments();
        let expression_with_type_arguments = node.as_expression_with_type_arguments();
        if !is_valid_heritage_type_reference_expression(expression_with_type_arguments.expression()) {
            return node;
        }
        let type_name = self.convert_entity_name_expression_to_entity_name(expression_with_type_arguments.expression());
        let result = self.factory.new_type_reference_node(type_name, expression_with_type_arguments.type_arguments());
        self.finish_node(result, pos)
    }
}

pub(crate) fn is_valid_heritage_type_reference_expression(node: P<Node>) -> bool {
    if ast::is_identifier(node) {
        return ast::node_is_present(Some(node));
    }
    ast::is_property_access_expression(node)
        && !ast::is_optional_chain(node)
        && ast::node_is_present(node.name())
        && is_valid_heritage_type_reference_expression(node.expression().unwrap())
}

impl Parser {
    pub(crate) fn convert_entity_name_expression_to_entity_name(&mut self, node: P<Node>) -> P<Node> {
        if ast::is_identifier(node) {
            return node;
        }
        let property_access = node.as_property_access_expression();
        let left = self.convert_entity_name_expression_to_entity_name(property_access.expression());
        let result = self.factory.new_qualified_name(left, property_access.name());
        self.finish_node_with_end(result, node.pos(), node.end())
    }

    pub(crate) fn parse_expression_with_type_arguments(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let expression = self.parse_left_hand_side_expression_or_higher();
        if ast::is_expression_with_type_arguments(expression) {
            return expression;
        }
        let type_arguments = self.parse_type_arguments();
        let result = self.factory.new_expression_with_type_arguments(expression, type_arguments);
        self.finish_node(result, pos)
    }

    pub(crate) fn parse_class_element(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let jsdoc = self.jsdoc_scanner_info();
        if self.token == Kind::SemicolonToken {
            self.next_token();
            let result = self.factory.new_semicolon_class_element();
            let result = self.finish_node(result, pos);
            self.with_jsdoc(result, jsdoc);
            return result;
        }
        let modifiers = self.parse_modifiers_ex(true /*allowDecorators*/, true /*permitConstAsModifier*/, true /*stopOnStartOfClassStaticBlock*/);
        if self.token == Kind::StaticKeyword && self.look_ahead(Parser::next_token_is_open_brace) {
            return self.parse_class_static_block_declaration(pos, jsdoc, modifiers);
        }
        if self.parse_contextual_modifier(Kind::GetKeyword) {
            return self.parse_accessor_declaration(pos, jsdoc, modifiers, Kind::GetAccessor, ParseFlags::None);
        }
        if self.parse_contextual_modifier(Kind::SetKeyword) {
            return self.parse_accessor_declaration(pos, jsdoc, modifiers, Kind::SetAccessor, ParseFlags::None);
        }
        if self.token == Kind::ConstructorKeyword || self.token == Kind::StringLiteral {
            if let Some(constructor_declaration) = self.try_parse_constructor_declaration(pos, jsdoc, modifiers) {
                return constructor_declaration;
            }
        }
        if self.is_index_signature() {
            let result = self.parse_index_signature_declaration(pos, jsdoc, modifiers);
            return self.check_js_syntax(result);
        }
        // It is very important that we check this *after* checking indexers because
        // the [ token can start an index signature or a computed property name
        if token_is_identifier_or_keyword(self.token)
            || self.token == Kind::StringLiteral
            || self.token == Kind::NumericLiteral
            || self.token == Kind::BigIntLiteral
            || self.token == Kind::AsteriskToken
            || self.token == Kind::OpenBracketToken
        {
            let is_ambient = modifiers.is_some_and(|modifiers| modifiers.nodes().iter().any(|m| is_declare_modifier(*m)));
            if is_ambient {
                for m in modifiers.unwrap().nodes() {
                    m.set_flags(m.flags() | NodeFlags::Ambient);
                }
                let save_context_flags = self.context_flags;
                self.set_context_flags(NodeFlags::Ambient, true);
                let result = self.parse_property_or_method_declaration(pos, jsdoc, modifiers);
                self.context_flags = save_context_flags;
                return result;
            } else {
                return self.parse_property_or_method_declaration(pos, jsdoc, modifiers);
            }
        }
        if modifiers.is_some() {
            // treat this as a property declaration with a missing name.
            let node_pos = self.node_pos();
            self.parse_error_at(node_pos, node_pos, &diagnostics::Declaration_expected, &[]);
            let name = self.create_missing_identifier();
            return self.parse_property_declaration(pos, jsdoc, modifiers, name, None /*questionToken*/);
        }
        // 'isClassMemberStart' should have hinted not to attempt parsing.
        panic!("Should not have attempted to parse class member declaration.");
    }

    pub(crate) fn parse_class_static_block_declaration(&mut self, pos: i32, jsdoc: JsdocScannerInfo, modifiers: Option<P<ModifierList>>) -> P<Node> {
        self.parse_expected_token(Kind::StaticKeyword);
        let body = self.parse_class_static_block_body();
        let result = self.factory.new_class_static_block_declaration(modifiers, body);
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        result
    }

    pub(crate) fn parse_class_static_block_body(&mut self) -> P<Node> {
        let save_context_flags = self.context_flags;
        self.set_context_flags(NodeFlags::YieldContext, false);
        self.set_context_flags(NodeFlags::AwaitContext, true);
        let body = self.parse_block(false /*ignoreMissingOpenBrace*/, None /*diagnosticMessage*/);
        self.context_flags = save_context_flags;
        body
    }

    pub(crate) fn try_parse_constructor_declaration(&mut self, pos: i32, jsdoc: JsdocScannerInfo, modifiers: Option<P<ModifierList>>) -> Option<P<Node>> {
        let state = self.mark();
        if self.token == Kind::ConstructorKeyword
            || self.token == Kind::StringLiteral && self.scanner.token_value() == "constructor" && self.look_ahead(Parser::next_token_is_open_paren)
        {
            self.next_token();
            let type_parameters = self.parse_type_parameters();
            let parameters = self.parse_parameters(ParseFlags::None);
            let return_type = self.parse_return_type(Kind::ColonToken, false /*isType*/);
            let body = self.parse_function_block_or_semicolon(ParseFlags::None, Some(&diagnostics::X_or_expected));
            let result = self.factory.new_constructor_declaration(modifiers, type_parameters, Some(parameters), return_type, None /*fullSignature*/, body);
            let result = self.finish_node(result, pos);
            self.with_jsdoc(result, jsdoc);
            self.check_js_syntax(result);
            return Some(result);
        }
        self.rewind(state);
        None
    }

    pub(crate) fn next_token_is_open_paren(&mut self) -> bool {
        self.next_token() == Kind::OpenParenToken
    }

    pub(crate) fn parse_property_or_method_declaration(&mut self, pos: i32, jsdoc: JsdocScannerInfo, modifiers: Option<P<ModifierList>>) -> P<Node> {
        let asterisk_token = self.parse_optional_token(Kind::AsteriskToken);
        let name = self.parse_property_name();
        // Note: this is not legal as per the grammar.  But we allow it in the parser and
        // report an error in the grammar checker.
        let question_token = self.parse_optional_token(Kind::QuestionToken);
        if asterisk_token.is_some() || self.token == Kind::OpenParenToken || self.token == Kind::LessThanToken {
            return self.parse_method_declaration(pos, jsdoc, modifiers, asterisk_token, name, question_token, Some(&diagnostics::X_or_expected));
        }
        self.parse_property_declaration(pos, jsdoc, modifiers, name, question_token)
    }

    pub(crate) fn parse_method_declaration(
        &mut self,
        pos: i32,
        jsdoc: JsdocScannerInfo,
        modifiers: Option<P<ModifierList>>,
        asterisk_token: Option<P<Node>>,
        name: P<Node>,
        question_token: Option<P<Node>>,
        diagnostic_message: Option<&'static Message>,
    ) -> P<Node> {
        let signature_flags = (if asterisk_token.is_some() { ParseFlags::Yield } else { ParseFlags::None })
            | (if modifier_list_has_async(modifiers) { ParseFlags::Await } else { ParseFlags::None });
        let type_parameters = self.parse_type_parameters();
        let parameters = self.parse_parameters(signature_flags);
        let type_node = self.parse_return_type(Kind::ColonToken, false /*isType*/);
        let body = self.parse_function_block_or_semicolon(signature_flags, diagnostic_message);
        let result = self.factory.new_method_declaration(modifiers, asterisk_token, name, question_token, type_parameters, Some(parameters), type_node, None /*fullSignature*/, body);
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        self.check_js_syntax(result);
        result
    }
}

pub(crate) fn modifier_list_has_async(modifiers: Option<P<ModifierList>>) -> bool {
    modifiers.is_some_and(|modifiers| modifiers.nodes().iter().any(|m| is_async_modifier(*m)))
}

impl Parser {
    pub(crate) fn parse_property_declaration(
        &mut self,
        pos: i32,
        jsdoc: JsdocScannerInfo,
        modifiers: Option<P<ModifierList>>,
        name: P<Node>,
        question_token: Option<P<Node>>,
    ) -> P<Node> {
        let mut postfix_token = question_token;
        if postfix_token.is_none() && !self.has_preceding_line_break() {
            postfix_token = self.parse_optional_token(Kind::ExclamationToken);
        }
        let type_node = self.parse_type_annotation();
        let initializer = self.do_in_context(
            NodeFlags::YieldContext | NodeFlags::AwaitContext | NodeFlags::DisallowInContext,
            false,
            Parser::parse_initializer,
        );
        self.parse_semicolon_after_property_name(name, type_node, initializer);
        let result = self.factory.new_property_declaration(modifiers, name, postfix_token, type_node, initializer);
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        self.check_js_syntax(result);
        result
    }

    pub(crate) fn parse_semicolon_after_property_name(&mut self, name: P<Node>, type_node: Option<P<Node>>, initializer: Option<P<Node>>) {
        if self.token == Kind::AtToken && !self.has_preceding_line_break() {
            self.parse_error_at_current_token(&diagnostics::Decorators_must_precede_the_name_and_all_keywords_of_property_declarations, &[]);
            return;
        }
        if self.token == Kind::OpenParenToken {
            self.parse_error_at_current_token(&diagnostics::Cannot_start_a_function_call_in_a_type_annotation, &[]);
            self.next_token();
            return;
        }
        if type_node.is_some() && !self.can_parse_semicolon() {
            if initializer.is_some() {
                self.parse_error_at_current_token(&diagnostics::X_0_expected, &[&scanner::token_to_string(Kind::SemicolonToken)]);
            } else {
                self.parse_error_at_current_token(&diagnostics::Expected_for_property_initializer, &[]);
            }
            return;
        }
        if self.try_parse_semicolon() {
            return;
        }
        if initializer.is_some() {
            self.parse_error_at_current_token(&diagnostics::X_0_expected, &[&scanner::token_to_string(Kind::SemicolonToken)]);
            return;
        }
        self.parse_error_for_missing_semicolon_after(name);
    }

    pub(crate) fn parse_error_for_missing_semicolon_after(&mut self, node: P<Node>) {
        // Tagged template literals are sometimes used in places where only simple strings are allowed, i.e.:
        //   module `M1` {
        //   ^^^^^^^^^^^ This block is parsed as a template literal like module`M1`.
        if node.kind() == Kind::TaggedTemplateExpression {
            let loc = self.skip_range_trivia(node.as_tagged_template_expression().template().loc());
            self.parse_error_at_range(loc, &diagnostics::Module_declaration_names_may_only_use_or_quoted_strings, &[]);
            return;
        }
        // Otherwise, if this isn't a well-known keyword-like identifier, give the generic fallback message.
        let mut expression_text = "";
        if node.kind() == Kind::Identifier {
            expression_text = node.text();
        }
        if expression_text.is_empty() {
            self.parse_error_at_current_token(&diagnostics::X_0_expected, &[&scanner::token_to_string(Kind::SemicolonToken)]);
            return;
        }
        let pos = scanner::skip_trivia(self.source_text, node.pos());
        // Some known keywords are likely signs of syntax being used improperly.
        match expression_text {
            "const" | "let" | "var" => {
                self.parse_error_at(pos, node.end(), &diagnostics::Variable_declaration_not_allowed_at_this_location, &[]);
                return;
            }
            "declare" => {
                // If a declared node failed to parse, it would have emitted a diagnostic already.
                return;
            }
            "interface" => {
                self.parse_error_for_invalid_name(&diagnostics::Interface_name_cannot_be_0, &diagnostics::Interface_must_be_given_a_name, Kind::OpenBraceToken);
                return;
            }
            "is" => {
                let end = self.scanner.token_start();
                self.parse_error_at(pos, end, &diagnostics::A_type_predicate_is_only_allowed_in_return_type_position_for_functions_and_methods, &[]);
                return;
            }
            "module" | "namespace" => {
                self.parse_error_for_invalid_name(&diagnostics::Namespace_name_cannot_be_0, &diagnostics::Namespace_must_be_given_a_name, Kind::OpenBraceToken);
                return;
            }
            "type" => {
                self.parse_error_for_invalid_name(&diagnostics::Type_alias_name_cannot_be_0, &diagnostics::Type_alias_must_be_given_a_name, Kind::EqualsToken);
                return;
            }
            _ => {}
        }
        // The user alternatively might have misspelled or forgotten to add a space after a common keyword.
        let mut suggestion = tsrs_core::get_spelling_suggestion_for_strings(expression_text, viable_keyword_suggestions.iter().copied())
            .map(|s| s.to_string())
            .unwrap_or_default();
        if suggestion.is_empty() {
            suggestion = get_space_suggestion(expression_text);
        }
        if !suggestion.is_empty() {
            self.parse_error_at(pos, node.end(), &diagnostics::Unknown_keyword_or_identifier_Did_you_mean_0, &[&suggestion]);
            return;
        }
        // Unknown tokens are handled with their own errors in the scanner
        if self.token == Kind::Unknown {
            return;
        }
        // Otherwise, we know this some kind of unknown word, not just a missing expected semicolon.
        self.parse_error_at(pos, node.end(), &diagnostics::Unexpected_keyword_or_identifier, &[]);
    }
}

pub(crate) fn get_space_suggestion(expression_text: &str) -> String {
    for keyword in viable_keyword_suggestions.iter() {
        if expression_text.len() > keyword.len() + 2 && expression_text.starts_with(keyword) {
            return format!("{} {}", keyword, &expression_text[keyword.len()..]);
        }
    }
    String::new()
}

impl Parser {
    pub(crate) fn parse_error_for_invalid_name(&mut self, name_diagnostic: &'static Message, blank_diagnostic: &'static Message, token_if_blank_name: Kind) {
        if self.token == token_if_blank_name {
            self.parse_error_at_current_token(blank_diagnostic, &[]);
        } else {
            let value = self.scanner.token_value();
            self.parse_error_at_current_token(name_diagnostic, &[&value]);
        }
    }

    pub(crate) fn parse_interface_declaration(&mut self, pos: i32, jsdoc: JsdocScannerInfo, modifiers: Option<P<ModifierList>>) -> P<Node> {
        self.parse_expected(Kind::InterfaceKeyword);
        let name = self.parse_identifier();
        let type_parameters = self.parse_type_parameters();
        let heritage_clauses = self.parse_heritage_clauses(true /*isInterface*/);
        let members = self.parse_object_type_members();
        let result = self.factory.new_interface_declaration(modifiers, name, type_parameters, heritage_clauses, members);
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        self.check_js_syntax(result);
        result
    }

    pub(crate) fn parse_type_alias_declaration(&mut self, pos: i32, jsdoc: JsdocScannerInfo, modifiers: Option<P<ModifierList>>) -> P<Node> {
        self.parse_expected(Kind::TypeKeyword);
        if self.has_preceding_line_break() {
            self.parse_error_at_current_token(&diagnostics::Line_break_not_permitted_here, &[]);
        }
        let name = self.parse_identifier();
        let type_parameters = self.parse_type_parameters();
        self.parse_expected(Kind::EqualsToken);
        let type_node = if self.token == Kind::IntrinsicKeyword && self.look_ahead(Parser::next_is_not_dot) {
            self.parse_keyword_type_node()
        } else {
            self.parse_type()
        };
        self.parse_semicolon();
        let result = self.factory.new_type_alias_declaration(modifiers, name, type_parameters, Some(type_node));
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        self.check_js_syntax(result);
        result
    }

    pub(crate) fn next_is_not_dot(&mut self) -> bool {
        self.next_token() != Kind::DotToken
    }

    // In an ambient declaration, the grammar only allows integer literals as initializers.
    // In a non-ambient declaration, the grammar allows uninitialized members only in a
    // ConstantEnumMemberSection, which starts at the beginning of an enum declaration
    // or any time an integer literal initializer is encountered.
    pub(crate) fn parse_enum_member(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let jsdoc = self.jsdoc_scanner_info();
        let name = self.parse_property_name();
        let initializer = self.do_in_context(NodeFlags::DisallowInContext, false, Parser::parse_initializer);
        let result = self.factory.new_enum_member(name, initializer);
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        result
    }

    pub(crate) fn parse_enum_declaration(&mut self, pos: i32, jsdoc: JsdocScannerInfo, modifiers: Option<P<ModifierList>>) -> P<Node> {
        let save_has_await_identifier = self.statement_has_await_identifier;
        self.parse_expected(Kind::EnumKeyword);
        let name = self.parse_identifier();
        let members: P<NodeList>;
        if self.parse_expected(Kind::OpenBraceToken) {
            let save_context_flags = self.context_flags;
            self.set_context_flags(NodeFlags::YieldContext | NodeFlags::AwaitContext, false);
            members = self.parse_delimited_list(ParsingContext::EnumMembers, Parser::parse_enum_member).unwrap();
            self.context_flags = save_context_flags;
            self.parse_expected(Kind::CloseBraceToken);
        } else {
            members = self.create_missing_list();
        }
        let result = self.factory.new_enum_declaration(modifiers, name, members);
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        self.check_js_syntax(result);
        self.statement_has_await_identifier = save_has_await_identifier;
        result
    }

    pub(crate) fn parse_module_declaration(&mut self, pos: i32, jsdoc: JsdocScannerInfo, modifiers: Option<P<ModifierList>>) -> P<Node> {
        let mut keyword = Kind::ModuleKeyword;
        if self.token == Kind::GlobalKeyword {
            // global augmentation
            return self.parse_ambient_external_module_declaration(pos, jsdoc, modifiers);
        } else if self.parse_optional(Kind::NamespaceKeyword) {
            keyword = Kind::NamespaceKeyword;
        } else {
            self.parse_expected(Kind::ModuleKeyword);
            if self.token == Kind::StringLiteral {
                return self.parse_ambient_external_module_declaration(pos, jsdoc, modifiers);
            }
        }
        self.parse_module_or_namespace_declaration(pos, jsdoc, modifiers, false /*nested*/, keyword)
    }

    pub(crate) fn parse_ambient_external_module_declaration(&mut self, pos: i32, jsdoc: JsdocScannerInfo, modifiers: Option<P<ModifierList>>) -> P<Node> {
        let name: P<Node>;
        let mut keyword = Kind::ModuleKeyword;
        let save_has_await_identifier = self.statement_has_await_identifier;
        if self.token == Kind::GlobalKeyword {
            // parse 'global' as name of global scope augmentation
            name = self.parse_identifier();
            keyword = Kind::GlobalKeyword;
        } else {
            // parse string literal
            name = self.parse_literal_expression();
        }
        let mut attributes = None;
        if keyword == Kind::ModuleKeyword && self.parse_optional(Kind::WithKeyword) {
            attributes = Some(self.parse_type_literal());
        }
        let mut body = None;
        if self.token == Kind::OpenBraceToken {
            body = Some(self.parse_module_block());
        } else {
            self.parse_semicolon();
        }
        let result = self.factory.new_module_declaration(modifiers, keyword, name, attributes, body);
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        self.statement_has_await_identifier = save_has_await_identifier;
        result
    }

    pub(crate) fn parse_module_block(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let statements: P<NodeList>;
        if self.parse_expected(Kind::OpenBraceToken) {
            statements = self.parse_list(ParsingContext::BlockStatements, Parser::parse_statement);
            self.parse_expected(Kind::CloseBraceToken);
        } else {
            statements = self.create_missing_list();
        }
        let result = self.factory.new_module_block(statements);
        self.finish_node(result, pos)
    }

    pub(crate) fn parse_module_or_namespace_declaration(
        &mut self,
        pos: i32,
        jsdoc: JsdocScannerInfo,
        modifiers: Option<P<ModifierList>>,
        nested: bool,
        keyword: Kind,
    ) -> P<Node> {
        let save_has_await_identifier = self.statement_has_await_identifier;
        let name = if nested { self.parse_identifier_name() } else { self.parse_identifier() };
        let body: P<Node>;
        if self.parse_optional(Kind::DotToken) {
            let implicit_export = self.factory.new_modifier(Kind::ExportKeyword);
            let node_pos = self.node_pos();
            implicit_export.set_loc(new_text_range(node_pos, node_pos));
            implicit_export.set_flags(NodeFlags::Reparsed);
            let implicit_modifiers = self.new_modifier_list(implicit_export.loc(), &[implicit_export]);
            let node_pos = self.node_pos();
            body = self.parse_module_or_namespace_declaration(node_pos, JsdocScannerInfo::empty() /*jsdoc*/, Some(implicit_modifiers), true /*nested*/, keyword);
        } else {
            body = self.parse_module_block();
        }
        let result = self.factory.new_module_declaration(modifiers, keyword, name, None, Some(body));
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        self.check_js_syntax(result);
        self.statement_has_await_identifier = save_has_await_identifier;
        result
    }

    pub(crate) fn parse_import_declaration_or_import_equals_declaration(
        &mut self,
        pos: i32,
        jsdoc: JsdocScannerInfo,
        modifiers: Option<P<ModifierList>>,
    ) -> P<Node> {
        self.parse_expected(Kind::ImportKeyword);
        let after_import_pos = self.node_pos();
        // We don't parse the identifier here in await context, instead we will report a grammar error in the checker.
        let save_has_await_identifier = self.statement_has_await_identifier;
        let mut identifier = None;
        if self.is_identifier() {
            identifier = Some(self.parse_identifier());
        }
        let mut phase_modifier = Kind::Unknown;
        if identifier.is_some_and(|identifier| identifier.text() == "type")
            && (self.token != Kind::FromKeyword || self.is_identifier() && self.look_ahead(Parser::next_token_is_from_keyword_or_equals_token))
            && (self.is_identifier() || self.token_after_import_definitely_produces_import_declaration())
        {
            phase_modifier = Kind::TypeKeyword;
            identifier = None;
            if self.is_identifier() {
                identifier = Some(self.parse_identifier());
            }
        } else if identifier.is_some_and(|identifier| identifier.text() == "defer") {
            let should_parse_as_defer_modifier = if self.token == Kind::FromKeyword {
                !self.look_ahead(Parser::next_token_is_token_string_literal)
            } else {
                self.token != Kind::CommaToken && self.token != Kind::EqualsToken
            };
            if should_parse_as_defer_modifier {
                phase_modifier = Kind::DeferKeyword;
                identifier = None;
                if self.is_identifier() {
                    identifier = Some(self.parse_identifier());
                }
            }
        }
        if let Some(identifier) = identifier {
            if !self.token_after_imported_identifier_definitely_produces_import_declaration() && phase_modifier != Kind::DeferKeyword {
                let import_equals = self.parse_import_equals_declaration(pos, jsdoc, modifiers, identifier, phase_modifier == Kind::TypeKeyword);
                let import_equals = self.check_js_syntax(import_equals);
                self.statement_has_await_identifier = save_has_await_identifier; // Import= declaration is always parsed in an Await context, no need to reparse
                return import_equals;
            }
        }
        let import_clause = self.try_parse_import_clause(identifier, after_import_pos, phase_modifier, false /*skipJSDocLeadingAsterisks*/);
        self.statement_has_await_identifier = save_has_await_identifier; // import clause is always parsed in an Await context
        let module_specifier = self.parse_module_specifier();
        let attributes = self.try_parse_import_attributes();
        self.parse_semicolon();
        let result = self.factory.new_import_declaration(modifiers, import_clause, module_specifier, attributes);
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        self.check_js_syntax(result);
        result
    }

    pub(crate) fn next_token_is_from_keyword_or_equals_token(&mut self) -> bool {
        self.next_token();
        self.token == Kind::FromKeyword || self.token == Kind::EqualsToken
    }

    pub(crate) fn token_after_import_definitely_produces_import_declaration(&mut self) -> bool {
        self.token == Kind::AsteriskToken || self.token == Kind::OpenBraceToken
    }

    pub(crate) fn token_after_imported_identifier_definitely_produces_import_declaration(&mut self) -> bool {
        // In `import id ___`, the current token decides whether to produce
        // an ImportDeclaration or ImportEqualsDeclaration.
        self.token == Kind::CommaToken || self.token == Kind::FromKeyword
    }

    pub(crate) fn parse_import_equals_declaration(
        &mut self,
        pos: i32,
        jsdoc: JsdocScannerInfo,
        modifiers: Option<P<ModifierList>>,
        identifier: P<Node>,
        is_type_only: bool,
    ) -> P<Node> {
        self.parse_expected(Kind::EqualsToken);
        let module_reference = self.parse_module_reference();
        self.parse_semicolon();
        let result = self.factory.new_import_equals_declaration(modifiers, is_type_only, identifier, module_reference);
        let result = self.finish_node(result, pos);
        self.with_jsdoc(result, jsdoc);
        result
    }

    pub(crate) fn parse_module_reference(&mut self) -> P<Node> {
        if self.token == Kind::RequireKeyword && self.look_ahead(Parser::next_token_is_open_paren) {
            return self.parse_external_module_reference();
        }
        self.parse_entity_name(false /*allowReservedWords*/, false /*allowPrivateName*/, None /*diagnosticMessage*/)
    }

    pub(crate) fn parse_external_module_reference(&mut self) -> P<Node> {
        let save_has_await_identifier = self.statement_has_await_identifier;
        let pos = self.node_pos();
        self.parse_expected(Kind::RequireKeyword);
        self.parse_expected(Kind::OpenParenToken);
        let expression = self.parse_module_specifier();
        self.parse_expected(Kind::CloseParenToken);
        let result = self.factory.new_external_module_reference(expression);
        let result = self.finish_node(result, pos);
        self.statement_has_await_identifier = save_has_await_identifier;
        result
    }

    pub(crate) fn parse_module_specifier(&mut self) -> P<Node> {
        if self.token == Kind::StringLiteral {
            return self.parse_literal_expression();
        }
        // We allow arbitrary expressions here, even though the grammar only allows string
        // literals.  We check to ensure that it is only a string literal later in the grammar
        // check pass.
        self.parse_expression()
    }

    pub(crate) fn try_parse_import_clause(
        &mut self,
        identifier: Option<P<Node>>,
        pos: i32,
        phase_modifier: Kind,
        skip_jsdoc_leading_asterisks: bool,
    ) -> Option<P<Node>> {
        // ImportDeclaration:
        //  import ImportClause from ModuleSpecifier ;
        //  import ModuleSpecifier;
        if identifier.is_some() || self.token == Kind::AsteriskToken || self.token == Kind::OpenBraceToken {
            let import_clause = self.parse_import_clause(identifier, pos, phase_modifier, skip_jsdoc_leading_asterisks);
            self.parse_expected(Kind::FromKeyword);
            return Some(import_clause);
        }
        None
    }

    pub(crate) fn parse_import_clause(
        &mut self,
        identifier: Option<P<Node>>,
        pos: i32,
        phase_modifier: Kind,
        skip_jsdoc_leading_asterisks: bool,
    ) -> P<Node> {
        // ImportClause:
        //  ImportedDefaultBinding
        //  NameSpaceImport
        //  NamedImports
        //  ImportedDefaultBinding, NameSpaceImport
        //  ImportedDefaultBinding, NamedImports
        // If there was no default import or if there is comma token after default import
        // parse namespace or named imports
        let mut named_bindings = None;
        let save_has_await_identifier = self.statement_has_await_identifier;
        if identifier.is_none() || self.parse_optional(Kind::CommaToken) {
            if skip_jsdoc_leading_asterisks {
                self.scanner.set_skip_jsdoc_leading_asterisks(true);
            }
            if self.token == Kind::AsteriskToken {
                named_bindings = Some(self.parse_namespace_import());
            } else {
                named_bindings = Some(self.parse_named_imports());
            }
            if skip_jsdoc_leading_asterisks {
                self.scanner.set_skip_jsdoc_leading_asterisks(false);
            }
        }
        let result = self.factory.new_import_clause(phase_modifier, identifier, named_bindings);
        let result = self.finish_node(result, pos);
        self.statement_has_await_identifier = save_has_await_identifier;
        result
    }
}
