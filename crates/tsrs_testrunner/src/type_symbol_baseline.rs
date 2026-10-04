// internal/testutil/tsbaseline/type_symbol_baseline.go: the `.types` and `.symbols` baselines.

use std::fmt::Write as _;
use std::sync::LazyLock;

use regex::Regex;
use tsrs_ast::{self as ast, Kind, Node, NodeFlags, SemanticMeaning, SourceFile, SymbolFlags};
use tsrs_checker::{self as checker, EmitContext, Flags, InternalFlags, PrintHandlers, PrinterOptions, SymbolFormatFlags, TypeFormatFlags};
use tsrs_compiler::Program;
use tsrs_core::{tspath, P};
use tsrs_scanner as scanner;

use crate::baseline::NO_CONTENT;
use crate::harnessutil::TestFile;
use crate::tsbaseline::{is_default_library_file, remove_test_path_prefixes};

static CODE_LINES_REGEXP: LazyLock<Regex> = LazyLock::new(|| Regex::new("[\r\u{2028}\u{2029}]|\r?\n").unwrap());
// Go's `\s` is ASCII-only.
static BRACKET_LINE_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[\t\n\x0C\r ]*[{|}][\t\n\x0C\r ]*$").unwrap());
static LINE_DELIMITER: LazyLock<Regex> = LazyLock::new(|| Regex::new("\r?\n").unwrap());

pub struct TypeAndSymbolBaselines {
    pub types: String,
    pub symbols: String,
}

// DoTypeAndSymbolBaseline: both baselines from one walker, the type walk first (it runs first in Go's
// test process, and it can create types that the symbol walk then sees). Each walk runs under its own
// panic guard (Go's per-subtest RecoverAndFail); a panic is returned as `Err(message)`.
pub fn do_type_and_symbol_baseline(
    header: &str,
    program: &'static Program,
    all_files: &[TestFile],
    has_error_baseline: bool,
) -> (Result<String, String>, Result<String, String>) {
    let mut full_walker = TypeWriterWalker::new(program, has_error_baseline);
    let types = catch(|| generate_baseline(all_files, &mut full_walker, header, false /*isSymbolBaseline*/));
    let symbols = catch(|| generate_baseline(all_files, &mut full_walker, header, true /*isSymbolBaseline*/));
    (types, symbols)
}

fn catch(f: impl FnOnce() -> String) -> Result<String, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(|e| {
        e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| "<panic>".to_string())
    })
}

fn generate_baseline(all_files: &[TestFile], full_walker: &mut TypeWriterWalker, header: &str, is_symbol_baseline: bool) -> String {
    let mut result = String::new();
    // !!! Perf baseline (not written by Go either)
    let perf_lines: Vec<String> = Vec::new();
    let baselines = iterate_baseline(all_files, full_walker, is_symbol_baseline);
    for value in &baselines {
        result.push_str(value);
    }
    if !result.is_empty() {
        return format!("//// [{header}] ////\r\n\r\n{}{result}", perf_lines.join("\n"));
    }
    NO_CONTENT.to_string()
}

fn iterate_baseline(all_files: &[TestFile], full_walker: &mut TypeWriterWalker, is_symbol_baseline: bool) -> Vec<String> {
    let mut baselines = Vec::new();

    for file in all_files {
        let results = if is_symbol_baseline { full_walker.get_symbols(&file.unit_name) } else { full_walker.get_types(&file.unit_name) };
        match file_baseline(&file.unit_name, &file.content, &results, is_symbol_baseline) {
            Some(text) => baselines.push(text),
            None => return baselines,
        }
    }

    baselines
}

// The body of iterateBaseline's per-file loop; `None` is Go's early `return baselines` (a symbol result without
// a symbol, which the walker never produces).
fn file_baseline(unit_name: &str, content: &str, results: &[TypeWriterResult], is_symbol_baseline: bool) -> Option<String> {
    let mut type_lines = String::new();
    type_lines.push_str("=== ");
    type_lines.push_str(unit_name);
    type_lines.push_str(" ===\r\n");
    let code_lines: Vec<&str> = CODE_LINES_REGEXP.split(content).collect();
    let mut last_index_written: i64 = -1;
    let is_blank_or_bracket = |i: usize| i < code_lines.len() && (BRACKET_LINE_REGEX.is_match(code_lines[i]) || code_lines[i].trim().is_empty());
    for result in results {
        if is_symbol_baseline && result.symbol.is_empty() {
            return None;
        }
        if last_index_written == -1 {
            type_lines.push_str(&code_lines[..result.line + 1].join("\r\n"));
            type_lines.push_str("\r\n");
        } else if last_index_written != result.line as i64 {
            if !is_blank_or_bracket((last_index_written + 1) as usize) {
                type_lines.push_str("\r\n");
            }
            type_lines.push_str(&code_lines[(last_index_written + 1) as usize..result.line + 1].join("\r\n"));
            type_lines.push_str("\r\n");
        }
        last_index_written = result.line as i64;
        let type_or_symbol_string = if is_symbol_baseline { &result.symbol } else { &result.typ };
        let line_text = LINE_DELIMITER.replace_all(&result.source_text, "");
        type_lines.push('>');
        let _ = write!(type_lines, "{line_text} : {type_or_symbol_string}");
        type_lines.push_str("\r\n");
        if !result.underline.is_empty() {
            type_lines.push('>');
            for _ in 0..line_text.len() {
                type_lines.push(' ');
            }
            type_lines.push_str(" : ");
            type_lines.push_str(&result.underline);
            type_lines.push_str("\r\n");
        }
    }

    if ((last_index_written + 1) as usize) < code_lines.len() {
        if !is_blank_or_bracket((last_index_written + 1) as usize) {
            type_lines.push_str("\r\n");
        }
        type_lines.push_str(&code_lines[(last_index_written + 1) as usize..].join("\r\n"));
    }
    type_lines.push_str("\r\n");

    Some(remove_test_path_prefixes(&type_lines, false /*retainTrailingDirectorySeparator*/))
}

// One file's section of the `.types` (or `.symbols`) baseline for a whole project (tsrs-test types-dump): the
// walk of `source_file` under `unit_name`, without the `//// [header] ////` prefix.
pub(crate) fn project_file_baseline(walker: &mut TypeWriterWalker, source_file: P<SourceFile>, unit_name: &str, is_symbol_baseline: bool) -> String {
    walker.current_source_file = Some(source_file);
    let results = walker.visit_node(source_file.as_node(), is_symbol_baseline);
    file_baseline(unit_name, source_file.text(), &results, is_symbol_baseline).unwrap_or_default()
}

pub(crate) struct TypeWriterWalker {
    program: &'static Program,
    had_error_baseline: bool,
    current_source_file: Option<P<SourceFile>>,
    // Go's printer.GetEmitContext pool: one context, reset before each use.
    emit_context: P<EmitContext>,
}

struct TypeWriterResult {
    line: usize,
    source_text: String,
    symbol: String,
    typ: String,
    underline: String, // !!!
}

impl TypeWriterWalker {
    pub(crate) fn new(program: &'static Program, had_error_baseline: bool) -> TypeWriterWalker {
        TypeWriterWalker { program, had_error_baseline, current_source_file: None, emit_context: checker::new_emit_context() }
    }

    fn get_types(&mut self, filename: &str) -> Vec<TypeWriterResult> {
        let source_file = self.program.get_source_file(filename).unwrap();
        self.current_source_file = Some(source_file);
        self.visit_node(source_file.as_node(), false /*isSymbolWalk*/)
    }

    fn get_symbols(&mut self, filename: &str) -> Vec<TypeWriterResult> {
        let source_file = self.program.get_source_file(filename).unwrap();
        self.current_source_file = Some(source_file);
        self.visit_node(source_file.as_node(), true /*isSymbolWalk*/)
    }

    fn visit_node(&mut self, node: P<Node>, is_symbol_walk: bool) -> Vec<TypeWriterResult> {
        let nodes = for_each_ast_node(node);
        let mut results = Vec::new();
        for n in nodes {
            if ast::is_expression_node(n)
                || n.kind() == Kind::Identifier
                || ast::is_declaration_name(n)
                || ast::is_qualified_name(n) && ast::is_name_of_heritage_clause_type_reference(n) && (is_symbol_walk || ast::is_qualified_name(n.parent().unwrap()))
            {
                if let Some(result) = self.write_type_or_symbol(n, is_symbol_walk) {
                    results.push(result);
                }
            }
        }
        results
    }

    fn write_type_or_symbol(&mut self, node: P<Node>, is_symbol_walk: bool) -> Option<TypeWriterResult> {
        let sf = self.current_source_file.unwrap();
        let actual_pos = scanner::skip_trivia(sf.text(), node.pos());
        let line = scanner::get_ecma_line_of_position(&*sf, actual_pos) as usize;
        let source_text = scanner::get_source_text_of_node_from_source_file(sf, node, false /*includeTrivia*/);
        // If we don't use the right checker for the file, its contents won't be up to date
        // since the types/symbols baselines appear to depend on files having been checked.
        let mut file_checker = self.program.get_type_checker_for_file(&tsrs_compiler::Context::default(), sf);
        let c = &mut *file_checker;
        let parent = node.parent().unwrap();

        if !is_symbol_walk {
            // Don't try to get the type of something that's already a type.
            // Exception for `T` in `type T = something` because that may evaluate to some interesting type.
            if ast::is_part_of_type_node(node)
                || (node.kind() == Kind::AsExpression || node.kind() == Kind::SatisfiesExpression) && node.type_node().unwrap().flags().intersects(NodeFlags::Reparsed)
                || ast::is_identifier(node)
                    && !ast::get_meaning_from_declaration(parent).intersects(SemanticMeaning::Value)
                    && !(ast::is_type_or_js_type_alias_declaration(parent) && Some(node) == parent.name())
            {
                return None;
            }

            if ast::is_omitted_expression(node) {
                return None;
            }

            let mut t = None;
            // Workaround to ensure we output 'C' instead of 'typeof C' for base class expressions
            if ast::is_expression_with_type_arguments_in_class_extends_clause(parent) {
                t = Some(c.get_type_at_location(parent));
            }
            if t.is_none() || checker::is_type_any(t) {
                t = Some(c.get_type_at_location(node));
            }
            let t = t.unwrap();
            let type_string;
            if !self.had_error_baseline
                && checker::is_type_any(Some(t))
                && !ast::is_binding_element(parent)
                && !ast::is_property_access_or_qualified_name(parent)
                && !ast::is_label_name(node)
                && !ast::is_global_scope_augmentation(parent)
                && !ast::is_meta_property(parent)
                && !is_import_statement_name(node)
                && !is_export_statement_name(node)
                && !is_intrinsic_jsx_tag(node, sf)
            {
                type_string = t.as_intrinsic_type().intrinsic_name().to_string();
            } else {
                let ctx = self.emit_context;
                ctx.reset();
                let builder = checker::new_node_builder(c, ctx);
                let type_format_flags =
                    TypeFormatFlags::NoTruncation | TypeFormatFlags::AllowUniqueESSymbolType | TypeFormatFlags::GenerateNamesForShadowedTypeParams;
                let mut type_node = builder.type_to_type_node(
                    c,
                    t,
                    Some(parent),
                    Flags::from_bits_retain((type_format_flags & TypeFormatFlags::NodeBuilderFlagsMask).bits()) | Flags::IgnoreErrors,
                    InternalFlags::AllowUnresolvedNames,
                    None,
                );
                if ast::is_identifier(node)
                    && ast::is_type_alias_declaration(parent)
                    && parent.name() == Some(node)
                    && type_node.is_some_and(|n| ast::is_identifier(n) && n.text() == node.text())
                {
                    // for a complex type alias `type T = ...`, showing "T : T" isn't very helpful for type tests. When the type produced is the same as
                    // the name of the type alias, recreate the type string without reusing the alias name
                    type_node = builder.type_to_type_node(
                        c,
                        t,
                        Some(parent),
                        Flags::from_bits_retain(((type_format_flags | TypeFormatFlags::InTypeAlias) & TypeFormatFlags::NodeBuilderFlagsMask).bits())
                            | Flags::IgnoreErrors,
                        InternalFlags::AllowUnresolvedNames,
                        None,
                    );
                }

                // !!! TODO: port underline printer, memoize
                let mut writer = checker::new_text_writer("", 0);
                let mut printer = checker::new_printer(PrinterOptions { remove_comments: true, ..Default::default() }, PrintHandlers::default(), Some(ctx));
                printer.write(type_node.unwrap(), Some(sf), &mut *writer, None);
                type_string = writer.string();
            }
            return Some(TypeWriterResult {
                line,
                source_text,
                symbol: String::new(),
                typ: type_string,
                underline: String::new(), // !!! TODO: underline
            });
        }

        let symbol = c.get_symbol_at_location_exported(node)?;

        let mut symbol_string = String::with_capacity(256);
        symbol_string.push_str("Symbol(");
        symbol_string.push_str(&tsrs_ast::escape_all_internal_symbol_names(&c.symbol_to_string_ex_exported(
            symbol,
            Some(parent),
            SymbolFlags::None,
            SymbolFormatFlags::AllowAnyNodeKind,
        )));
        let declarations = symbol.declarations();
        for (count, &declaration) in declarations.iter().enumerate() {
            if count >= 5 {
                let _ = write!(symbol_string, " ... and {} more", declarations.len() - count);
                break;
            }
            symbol_string.push_str(", ");
            // (Go's declarationTextCache is never written.)
            let decl_source_file = ast::get_source_file_of_node(declaration).unwrap();
            let (decl_line, decl_char) = scanner::get_ecma_line_and_utf16_character_of_position(&*decl_source_file, declaration.pos());
            let file_name = tspath::get_base_file_name(decl_source_file.file_name());
            symbol_string.push_str("Decl(");
            symbol_string.push_str(&file_name);
            symbol_string.push_str(", ");
            if is_default_library_file(&file_name) {
                symbol_string.push_str("--, --)");
            } else {
                let _ = write!(symbol_string, "{}, {})", decl_line, decl_char as i64);
            }
        }
        symbol_string.push(')');
        Some(TypeWriterResult { line, source_text, symbol: symbol_string, typ: String::new(), underline: String::new() })
    }
}

fn for_each_ast_node(node: P<Node>) -> Vec<P<Node>> {
    let mut result = Vec::new();
    let mut work = vec![node];

    let mut res_children: Vec<P<Node>> = Vec::new();

    while let Some(elem) = work.pop() {
        let reparsed = elem.flags().intersects(NodeFlags::Reparsed);
        let parent_kind = elem.parent().map(|p| p.kind());
        let parent_is_as_or_satisfies = matches!(parent_kind, Some(Kind::SatisfiesExpression) | Some(Kind::AsExpression));
        if !reparsed
            || elem.kind() == Kind::AsExpression
            || elem.kind() == Kind::SatisfiesExpression
            || (parent_is_as_or_satisfies && Some(elem) == elem.parent().unwrap().expression())
        {
            if !reparsed || parent_is_as_or_satisfies {
                result.push(elem);
            }
            elem.for_each_child(&mut |child| {
                res_children.push(child);
                false
            });
            res_children.reverse();
            work.append(&mut res_children);
        }
    }
    result
}

fn is_import_statement_name(node: P<Node>) -> bool {
    let parent = node.parent().unwrap();
    if ast::is_import_specifier(parent) && (Some(node) == parent.name() || Some(node) == parent.property_name()) {
        return true;
    }
    if ast::is_import_clause(parent) && Some(node) == parent.name() {
        return true;
    }
    if ast::is_import_equals_declaration(parent) && Some(node) == parent.name() {
        return true;
    }
    false
}

fn is_export_statement_name(node: P<Node>) -> bool {
    let parent = node.parent().unwrap();
    if ast::is_export_assignment(parent) && Some(node) == parent.expression() {
        return true;
    }
    if ast::is_export_specifier(parent) && (Some(node) == parent.name() || Some(node) == parent.property_name()) {
        return true;
    }
    false
}

fn is_intrinsic_jsx_tag(node: P<Node>, source_file: P<SourceFile>) -> bool {
    let parent = node.parent().unwrap();
    if !(ast::is_jsx_opening_element(parent) || ast::is_jsx_closing_element(parent) || ast::is_jsx_self_closing_element(parent)) {
        return false;
    }
    if parent.tag_name() != node {
        return false;
    }
    let text = scanner::get_source_text_of_node_from_source_file(source_file, node, false /*includeTrivia*/);
    scanner::is_intrinsic_jsx_name(&text)
}
