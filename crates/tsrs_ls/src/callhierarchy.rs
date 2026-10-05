use std::cell::OnceCell;

use rustc_hash::{FxHashMap, FxHashSet};
use tsrs_ast::{self as ast, Kind, ModifierFlags, Node, NodeFlags, NodeId, SourceFile, Symbol, SymbolFlags};
use tsrs_checker::Checker;
use tsrs_compiler::Program;
use tsrs_core::collections::{OrderedMap, Set};
use tsrs_core::context::Context;
use tsrs_core::{TextPos, TextRange, P};
use tsrs_lsproto as lsproto;
use tsrs_printer::{self as printer, EmitTextWriter as _};
use tsrs_scanner as scanner;

use crate::astnav;
use crate::crossproject::{combine_incoming_calls, CrossProjectOrchestrator};
use crate::definition::lsp_range_contains;
use crate::findallreferences::{EntryKind, ReferenceEntry, SymbolAndEntriesData, SymbolEntryTransformOptions};
use crate::languageservice::LanguageService;
use crate::lsconv;
use crate::spanmap::Feature;
use crate::symbols::get_symbol_kind_from_node;

// Go's `resolveCallHierarchyDeclaration` returns `any`: nil, a `*ast.Node` or a `[]*ast.Node`.
// callhierarchy.go:23 (CallHierarchyDeclaration)
pub(crate) enum CallHierarchyDeclarations {
    Node(P<Node>),
    Nodes(Vec<P<Node>>),
}

// Indictates whether a node is named function or class expression.
// callhierarchy.go:26
fn is_named_expression(node: Option<P<Node>>) -> bool {
    let Some(node) = node else {
        return false;
    };
    if !ast::is_function_expression(node) && !ast::is_class_expression(node) {
        return false;
    }
    let name = node.name();
    name.is_some_and(ast::is_identifier)
}

// callhierarchy.go:37
fn is_variable_like(node: Option<P<Node>>) -> bool {
    let Some(node) = node else {
        return false;
    };
    ast::is_property_declaration(node) || ast::is_variable_declaration(node)
}

// Indicates whether a node is a function, arrow, or class expression assigned to a constant variable or class property.
// callhierarchy.go:45
fn is_assigned_expression(node: Option<P<Node>>) -> bool {
    let Some(node) = node else {
        return false;
    };
    if !(ast::is_function_expression(node) || ast::is_arrow_function(node) || ast::is_class_expression(node)) {
        return false;
    }
    if node.name().is_some() {
        return false;
    }
    let parent = node.parent();
    if !is_variable_like(parent) {
        return false;
    }
    let parent = parent.unwrap();

    if parent.initializer() != Some(node) {
        return false;
    }

    let name = parent.name().unwrap();
    if !ast::is_identifier(name) {
        return false;
    }

    ast::get_combined_node_flags(parent).intersects(NodeFlags::Const) || ast::is_property_declaration(parent)
}

// Indicates whether a node could possibly be a call hierarchy declaration.
//
// See `resolveCallHierarchyDeclaration` for the specific rules.
// callhierarchy.go:75
fn is_possible_call_hierarchy_declaration(node: Option<P<Node>>) -> bool {
    let Some(node) = node else {
        return false;
    };
    ast::is_source_file(node)
        || ast::is_module_declaration(node)
        || ast::is_function_declaration(node)
        || ast::is_function_expression(node)
        || ast::is_class_declaration(node)
        || ast::is_class_expression(node)
        || ast::is_class_static_block_declaration(node)
        || ast::is_method_declaration(node)
        || ast::is_method_signature_declaration(node)
        || ast::is_get_accessor_declaration(node)
        || ast::is_set_accessor_declaration(node)
}

// Indicates whether a node is a valid a call hierarchy declaration.
//
// See `resolveCallHierarchyDeclaration` for the specific rules.
// callhierarchy.go:95
fn is_valid_call_hierarchy_declaration(node: Option<P<Node>>) -> bool {
    let Some(node) = node else {
        return false;
    };

    if ast::is_source_file(node) {
        return true;
    }

    if ast::is_module_declaration(node) {
        return ast::is_identifier(node.name().unwrap());
    }

    ast::is_function_declaration(node)
        || ast::is_class_declaration(node)
        || ast::is_class_static_block_declaration(node)
        || ast::is_method_declaration(node)
        || ast::is_method_signature_declaration(node)
        || ast::is_get_accessor_declaration(node)
        || ast::is_set_accessor_declaration(node)
        || is_named_expression(Some(node))
        || is_assigned_expression(Some(node))
}

// Gets the node that can be used as a reference to a call hierarchy declaration.
// callhierarchy.go:120
fn get_call_hierarchy_declaration_reference_node(node: Option<P<Node>>) -> Option<P<Node>> {
    let node = node?;

    if ast::is_source_file(node) {
        return Some(node);
    }

    if let Some(name) = node.name() {
        return Some(name);
    }

    if is_assigned_expression(Some(node)) {
        return node.parent().unwrap().name();
    }

    if let Some(modifiers) = node.modifiers() {
        for &m in modifiers.nodes() {
            if m.kind() == Kind::DefaultKeyword {
                return Some(m);
            }
        }
    }

    None
}

// Gets the symbol for a call hierarchy declaration.
// callhierarchy.go:149
fn get_symbol_of_call_hierarchy_declaration(c: &mut Checker, node: P<Node>) -> Option<P<Symbol>> {
    if ast::is_class_static_block_declaration(node) {
        return None;
    }
    let location = get_call_hierarchy_declaration_reference_node(Some(node))?;
    c.get_symbol_at_location_exported(location)
}

// Gets the text and range for the name of a call hierarchy declaration.
// callhierarchy.go:161
fn get_call_hierarchy_item_name(program: &'static Program, node: P<Node>) -> (String, i32, i32) {
    if ast::is_source_file(node) {
        let source_file = node.as_source_file();
        return (source_file.file_name().to_string(), 0, 0);
    }

    if (ast::is_function_declaration(node) || ast::is_class_declaration(node)) && node.name().is_none() {
        if let Some(modifiers) = node.modifiers() {
            for &m in modifiers.nodes() {
                if m.kind() == Kind::DefaultKeyword {
                    let source_file = ast::get_source_file_of_node(node).unwrap();
                    let start = scanner::skip_trivia(source_file.text(), m.pos());
                    return ("default".to_string(), start, m.end());
                }
            }
        }
    }

    if ast::is_class_static_block_declaration(node) {
        let source_file = ast::get_source_file_of_node(node).unwrap();
        let pos = scanner::skip_trivia(source_file.text(), move_range_past_modifiers(node).pos());
        let end = pos + 6; // "static".length
        let mut c = program.get_type_checker_for_file(&Context::background(), source_file);
        let c: &mut Checker = &mut c;
        let symbol = c.get_symbol_at_location_exported(node.parent().unwrap());
        let mut prefix = String::new();
        if let Some(symbol) = symbol {
            prefix = c.symbol_to_string_exported(symbol) + " ";
        }
        return (prefix + "static {}", pos, end);
    }

    let decl_name = if is_assigned_expression(Some(node)) { node.parent().unwrap().name() } else { ast::get_name_of_declaration(Some(node)) };

    if decl_name.is_none_or(|decl_name| !ast::node_is_present(Some(decl_name))) {
        let source_file = ast::get_source_file_of_node(node).unwrap();
        if ast::is_function_declaration(node) || ast::is_function_expression(node) {
            let kw_pos = scanner::skip_trivia(source_file.text(), move_range_past_modifiers(node).pos());
            return ("(anonymous)".to_string(), kw_pos, kw_pos + 8); // "function".length
        } else if ast::is_class_declaration(node) || ast::is_class_expression(node) {
            let kw_pos = scanner::skip_trivia(source_file.text(), move_range_past_modifiers(node).pos());
            return ("(anonymous)".to_string(), kw_pos, kw_pos + 5); // "class".length
        }
        assert!(decl_name.is_some(), "Expected call hierarchy item to have a name");
    }
    let decl_name = decl_name.unwrap();

    let text = get_text_of_call_hierarchy_name(program, node, decl_name, node);

    let source_file = ast::get_source_file_of_node(node).unwrap();
    let name_pos = scanner::skip_trivia(source_file.text(), decl_name.pos());

    (text, name_pos, decl_name.end())
}

// callhierarchy.go:221
fn get_text_of_call_hierarchy_name(program: &'static Program, source_node: P<Node>, name: P<Node>, print_node: P<Node>) -> String {
    if ast::is_identifier(name) || ast::is_string_or_numeric_literal_like(name) {
        return name.text().to_string();
    }
    if ast::is_computed_property_name(name) {
        let expr = name.expression().unwrap();
        if ast::is_string_or_numeric_literal_like(expr) {
            return expr.text().to_string();
        }
    }

    {
        let mut c = program.get_type_checker_for_file(&Context::background(), ast::get_source_file_of_node(source_node).unwrap());
        let c: &mut Checker = &mut c;
        let symbol = c.get_symbol_at_location_exported(name);
        if let Some(symbol) = symbol {
            let text = c.symbol_to_string_exported(symbol);
            if !text.is_empty() {
                return text;
            }
        }
    }

    let source_file = ast::get_source_file_of_node(source_node).unwrap();
    let (mut writer, put_writer) = printer::get_single_line_string_writer();
    let mut p = printer::new_printer(printer::PrinterOptions { remove_comments: true, ..Default::default() }, printer::PrintHandlers::default(), None);
    p.write(print_node, Some(source_file), &mut *writer, None);
    let result = writer.string();
    put_writer();
    result
}

// callhierarchy.go:250
fn get_call_hierarchy_item_container_name(program: &'static Program, node: P<Node>) -> String {
    if is_assigned_expression(Some(node)) {
        let parent = node.parent().unwrap();
        let parent_parent = parent.parent().unwrap();
        if ast::is_property_declaration(parent) && ast::is_class_like(parent_parent) {
            if ast::is_class_expression(parent_parent) {
                if let Some(assigned_name) = ast::get_assigned_name(parent_parent) {
                    return get_text_of_call_hierarchy_name(program, node, assigned_name, assigned_name);
                }
            } else if let Some(name) = parent_parent.name() {
                return get_text_of_call_hierarchy_name(program, node, name, name);
            }
        }
        if let Some(ppp) = parent_parent.parent().and_then(|ppp| ppp.parent().filter(|&p4| ast::is_module_block(p4)).map(|_| ppp)) {
            let mod_parent = ppp.parent().unwrap().parent();
            if let Some(mod_parent) = mod_parent.filter(|&m| ast::is_module_declaration(m)) {
                if let Some(name) = mod_parent.name().filter(|&n| ast::is_identifier(n)) {
                    return name.text().to_string();
                }
            }
        }
        return String::new();
    }

    match node.kind() {
        Kind::GetAccessor | Kind::SetAccessor | Kind::MethodDeclaration => {
            let parent = node.parent().unwrap();
            if parent.kind() == Kind::ObjectLiteralExpression {
                if let Some(assigned_name) = ast::get_assigned_name(parent) {
                    return get_text_of_call_hierarchy_name(program, node, assigned_name, assigned_name);
                }
            }
            if let Some(name) = ast::get_name_of_declaration(Some(parent)) {
                return get_text_of_call_hierarchy_name(program, node, name, name);
            }
        }
        Kind::FunctionDeclaration | Kind::ClassDeclaration | Kind::ModuleDeclaration => {
            let parent = node.parent().unwrap();
            if ast::is_module_block(parent) {
                let parent_parent = parent.parent().unwrap();
                if ast::is_module_declaration(parent_parent) {
                    if let Some(name) = parent_parent.name().filter(|&n| ast::is_identifier(n)) {
                        return name.text().to_string();
                    }
                }
            }
        }
        _ => {}
    }

    String::new()
}

// callhierarchy.go:298
fn move_range_past_modifiers(node: P<Node>) -> TextRange {
    if let Some(modifiers) = node.modifiers() {
        if let Some(&last_mod) = modifiers.nodes().last() {
            return TextRange::new(last_mod.end(), node.end());
        }
    }
    TextRange::new(node.pos(), node.end())
}

// Finds the implementation of a function-like declaration, if one exists.
// callhierarchy.go:307
fn find_implementation(c: &mut Checker, node: Option<P<Node>>) -> Option<P<Node>> {
    let node = node?;

    if !ast::is_function_like_declaration(Some(node)) {
        return Some(node);
    }

    if node.body().is_some() {
        return Some(node);
    }

    if ast::is_constructor_declaration(node) {
        return ast::get_first_constructor_with_body(node.parent().unwrap());
    }

    if ast::is_function_declaration(node) || ast::is_method_declaration(node) {
        let symbol = get_symbol_of_call_hierarchy_declaration(c, node);
        if let Some(value_declaration) = symbol.and_then(|s| s.value_declaration()) {
            if ast::is_function_like_declaration(Some(value_declaration)) && value_declaration.body().is_some() {
                return Some(value_declaration);
            }
        }
        return None;
    }

    Some(node)
}

// Go returns nil when there is no symbol (or no declarations) and a nil slice when none is valid; both are `None`.
// callhierarchy.go:337
fn find_all_initial_declarations(c: &mut Checker, node: P<Node>) -> Option<Vec<P<Node>>> {
    if ast::is_class_static_block_declaration(node) {
        return None;
    }

    let symbol = get_symbol_of_call_hierarchy_declaration(c, node)?;
    let symbol_declarations = symbol.declarations();
    if symbol_declarations.is_empty() {
        return None;
    }

    #[derive(Clone)]
    struct declKey {
        file: String,
        pos: i32,
    }

    let mut indices: Vec<usize> = (0..symbol_declarations.len()).collect();
    let keys: Vec<declKey> =
        symbol_declarations.iter().map(|&decl| declKey { file: ast::get_source_file_of_node(decl).unwrap().file_name().to_string(), pos: decl.pos() }).collect();

    indices.sort_by(|&a, &b| {
        if keys[a].file != keys[b].file {
            return keys[a].file.cmp(&keys[b].file);
        }
        keys[a].pos.cmp(&keys[b].pos)
    });

    let mut declarations: Vec<P<Node>> = Vec::new();
    let mut last_decl: Option<P<Node>> = None;

    for i in indices {
        let decl = symbol_declarations[i];
        if is_valid_call_hierarchy_declaration(Some(decl)) {
            if last_decl.is_none_or(|last_decl| last_decl.parent() != decl.parent() || last_decl.end() != decl.pos()) {
                declarations.push(decl);
            }
            last_decl = Some(decl);
        }
    }

    if declarations.is_empty() {
        return None;
    }
    Some(declarations)
}

// Find the implementation or the first declaration for a call hierarchy declaration.
// callhierarchy.go:388
fn find_implementation_or_all_initial_declarations(c: &mut Checker, node: P<Node>) -> CallHierarchyDeclarations {
    if ast::is_class_static_block_declaration(node) {
        return CallHierarchyDeclarations::Node(node);
    }

    if ast::is_function_like_declaration(Some(node)) {
        if let Some(implementation) = find_implementation(c, Some(node)) {
            return CallHierarchyDeclarations::Node(implementation);
        }
        if let Some(decls) = find_all_initial_declarations(c, node) {
            return CallHierarchyDeclarations::Nodes(decls);
        }
        return CallHierarchyDeclarations::Node(node);
    }

    if let Some(decls) = find_all_initial_declarations(c, node) {
        return CallHierarchyDeclarations::Nodes(decls);
    }
    CallHierarchyDeclarations::Node(node)
}

// Resolves the call hierarchy declaration for a node.
// callhierarchy.go:410
pub(crate) fn resolve_call_hierarchy_declaration(program: &'static Program, location: P<Node>) -> Option<CallHierarchyDeclarations> {
    // A call hierarchy item must refer to either a SourceFile, Module Declaration, Class Static Block, or something intrinsically callable that has a name:
    // - Class Declarations
    // - Class Expressions (with a name)
    // - Function Declarations
    // - Function Expressions (with a name or assigned to a const variable)
    // - Arrow Functions (assigned to a const variable)
    // - Constructors
    // - Class `static {}` initializer blocks
    // - Methods
    // - Accessors
    //
    // If a call is contained in a non-named callable Node (function expression, arrow function, etc.), then
    // its containing `CallHierarchyItem` is a containing function or SourceFile that matches the above list.

    let mut c = program.get_type_checker(&Context::background());
    let c: &mut Checker = &mut c;

    let mut following_symbol = false;
    let mut location = location;

    loop {
        if is_valid_call_hierarchy_declaration(Some(location)) {
            return Some(find_implementation_or_all_initial_declarations(c, location));
        }

        if is_possible_call_hierarchy_declaration(Some(location)) {
            let ancestor = ast::find_ancestor(Some(location), |n| is_valid_call_hierarchy_declaration(Some(n)));
            if let Some(ancestor) = ancestor {
                return Some(find_implementation_or_all_initial_declarations(c, ancestor));
            }
        }

        if ast::is_declaration_name(location) {
            let parent = location.parent();
            if is_valid_call_hierarchy_declaration(parent) {
                return Some(find_implementation_or_all_initial_declarations(c, parent.unwrap()));
            }
            if is_possible_call_hierarchy_declaration(parent) {
                let ancestor = ast::find_ancestor(parent, |n| is_valid_call_hierarchy_declaration(Some(n)));
                if let Some(ancestor) = ancestor {
                    return Some(find_implementation_or_all_initial_declarations(c, ancestor));
                }
            }
            if is_variable_like(parent) {
                let initializer = parent.unwrap().initializer();
                if let Some(initializer) = initializer.filter(|&i| is_assigned_expression(Some(i))) {
                    return Some(CallHierarchyDeclarations::Node(initializer));
                }
            }
            return None;
        }

        if ast::is_constructor_declaration(location) {
            if is_valid_call_hierarchy_declaration(location.parent()) {
                return Some(CallHierarchyDeclarations::Node(location.parent().unwrap()));
            }
            return None;
        }

        if location.kind() == Kind::StaticKeyword && ast::is_class_static_block_declaration(location.parent().unwrap()) {
            location = location.parent().unwrap();
            continue;
        }

        // #39453
        if ast::is_variable_declaration(location) {
            if let Some(initializer) = location.initializer().filter(|&i| is_assigned_expression(Some(i))) {
                return Some(CallHierarchyDeclarations::Node(initializer));
            }
        }

        if !following_symbol {
            let symbol = c.get_symbol_at_location_exported(location);
            if let Some(mut symbol) = symbol {
                if symbol.flags().intersects(SymbolFlags::Alias) {
                    symbol = c.get_aliased_symbol(symbol);
                }
                if let Some(value_declaration) = symbol.value_declaration() {
                    following_symbol = true;
                    location = value_declaration;
                    continue;
                }
            }
        }

        return None;
    }
}

impl LanguageService {
    // Creates a `CallHierarchyItem` for a call hierarchy declaration.
    // callhierarchy.go:501
    fn create_call_hierarchy_item(&self, program: &'static Program, node: P<Node>) -> Option<lsproto::CallHierarchyItem> {
        let source_file = ast::get_source_file_of_node(node).unwrap();
        let (name_text, name_pos, name_end) = get_call_hierarchy_item_name(program, node);
        let container_name = get_call_hierarchy_item_container_name(program, node);

        let kind = get_symbol_kind_from_node(node);

        let full_start = scanner::skip_trivia_ex(source_file.text(), node.pos(), Some(&scanner::SkipTriviaOptions { stop_at_comments: true, ..Default::default() }));
        let (mut span, span_fidelity) = self.converters.to_lsp_range_for_feature(&source_file, TextRange::new(full_start, node.end()), Feature::CallHierarchy);
        let (selection_span, selection_fidelity) = self.converters.to_lsp_range_for_feature(&source_file, TextRange::new(name_pos, name_end), Feature::CallHierarchy);
        if !selection_fidelity.is_single_segment() {
            return None;
        }
        if span_fidelity.is_none() || !source_file.content_mapper().is_empty() && !lsp_range_contains(span, selection_span) {
            span = selection_span;
        }

        let mut item = lsproto::CallHierarchyItem {
            name: name_text,
            kind,
            uri: lsconv::file_name_to_document_uri(source_file.original_file_name()),
            range: span,
            selection_range: selection_span,
            ..Default::default()
        };

        if !container_name.is_empty() {
            item.detail = Some(container_name);
        }

        Some(item)
    }
}

// callhierarchy.go:533
struct callSite {
    declaration: P<Node>,
    text_range: TextRange,
    source_file: P<Node>,
}

// callhierarchy.go:539
fn convert_entry_to_call_site(entry: &ReferenceEntry) -> Option<callSite> {
    if entry.kind != EntryKind::Node {
        return None;
    }

    let node = entry.node.unwrap();
    if !ast::is_call_or_new_expression_target(node, true /*includeElementAccess*/, true /*skipPastOuterExpressions*/)
        && !ast::is_tagged_template_tag(node, true, true)
        && !ast::is_decorator_target(node, true, true)
        && !ast::is_jsx_opening_like_element_tag_name(node, true, true)
        && !ast::is_right_side_of_property_access(node)
        && !ast::is_argument_expression_of_element_access(node)
    {
        return None;
    }

    let source_file = ast::get_source_file_of_node(node).unwrap();
    let ancestor = ast::find_ancestor(Some(node), |n| is_valid_call_hierarchy_declaration(Some(n))).unwrap_or_else(|| source_file.as_node());

    let start = scanner::skip_trivia(source_file.text(), node.pos());
    Some(callSite { declaration: ancestor, text_range: TextRange::new(start, node.end()), source_file: source_file.as_node() })
}

// callhierarchy.go:568
fn get_call_site_group_key(site: &callSite) -> NodeId {
    ast::get_node_id(site.declaration)
}

impl LanguageService {
    // callhierarchy.go:572
    fn convert_call_site_group_to_incoming_call(&self, program: &'static Program, entries: &[callSite]) -> Option<lsproto::CallHierarchyIncomingCall> {
        let mut from_ranges: Vec<lsproto::Range> = Vec::with_capacity(entries.len());
        for entry in entries {
            let source_file = entry.source_file.as_source_file_p();
            let (lsp_range, fidelity) = self.converters.to_lsp_range_for_feature(&source_file, entry.text_range, Feature::CallHierarchy);
            if !fidelity.is_none() {
                from_ranges.push(lsp_range);
            }
        }
        let from = self.create_call_hierarchy_item(program, entries[0].declaration);
        let from = from?;
        if from_ranges.is_empty() {
            return None;
        }

        from_ranges.sort_by(|a, b| lsproto::compare_ranges(*a, *b).cmp(&0));

        Some(lsproto::CallHierarchyIncomingCall { from, from_ranges })
    }
}

// Go caches the source file, URI and position with `sync.Once`s.
// callhierarchy.go:593
pub(crate) struct incomingEntry<'a> {
    ls: &'a LanguageService,
    node: P<Node>,

    source_file: OnceCell<P<SourceFile>>,

    document_uri: OnceCell<lsproto::DocumentUri>,

    position: OnceCell<lsproto::Position>,
}

impl incomingEntry<'_> {
    // callhierarchy.go:609
    fn get_source_file(&self) -> P<SourceFile> {
        *self.source_file.get_or_init(|| ast::get_source_file_of_node(self.node).unwrap())
    }
}

// callhierarchy.go:616
impl lsproto::HasTextDocumentURI for incomingEntry<'_> {
    fn text_document_uri(&self) -> &lsproto::DocumentUri {
        self.document_uri.get_or_init(|| lsconv::file_name_to_document_uri(self.get_source_file().original_file_name()))
    }
}

// callhierarchy.go:623
impl lsproto::HasTextDocumentPosition for incomingEntry<'_> {
    fn text_document_position(&self) -> lsproto::Position {
        *self.position.get_or_init(|| {
            let start = scanner::get_token_pos_of_node(self.node, self.get_source_file(), false /*includeJsDoc*/);
            let (position, _) = self.ls.create_lsp_position(start, self.get_source_file());
            position
        })
    }
}

impl LanguageService {
    // Gets the call sites that call into the provided call hierarchy declaration.
    // callhierarchy.go:632
    fn get_incoming_calls(
        &self,
        ctx: &Context,
        _program: &'static Program,
        declaration: P<Node>,
        orchestrator: Option<&dyn CrossProjectOrchestrator>,
    ) -> Result<lsproto::CallHierarchyIncomingCallsResponse, lsproto::Error> {
        // Source files and modules have no incoming calls.
        if ast::is_source_file(declaration) || ast::is_module_declaration(declaration) || ast::is_class_static_block_declaration(declaration) {
            return Ok(lsproto::CallHierarchyIncomingCallsOrNull::default());
        }

        let Some(location) = get_call_hierarchy_declaration_reference_node(Some(declaration)) else {
            return Ok(lsproto::CallHierarchyIncomingCallsOrNull::default());
        };
        let location_file = ast::get_source_file_of_node(location).unwrap();
        let location_start = scanner::get_token_pos_of_node(location, location_file, false /*includeJsDoc*/);
        let (_, fidelity) = self.converters.to_lsp_position_for_feature(&location_file, location_start as TextPos, Feature::CallHierarchy);
        if fidelity.is_none() {
            return Ok(lsproto::CallHierarchyIncomingCallsOrNull::default());
        }

        let incoming_entry = incomingEntry { ls: self, node: location, source_file: OnceCell::new(), document_uri: OnceCell::new(), position: OnceCell::new() };

        let mut result = self.handle_cross_project(
            ctx,
            &incoming_entry,
            orchestrator,
            LanguageService::symbol_and_entries_to_incoming_calls,
            combine_incoming_calls,
            false,
            false,
            SymbolEntryTransformOptions::default(),
            None, /*defaultProjectData*/
        );
        if let Ok(result) = &mut result {
            if let Some(calls) = &mut result.call_hierarchy_incoming_calls {
                calls.sort_by(|a, b| {
                    let uri_comp = a.from.uri.0.cmp(&b.from.uri.0);
                    if uri_comp != std::cmp::Ordering::Equal {
                        return uri_comp;
                    }
                    if a.from_ranges.is_empty() || b.from_ranges.is_empty() {
                        return std::cmp::Ordering::Equal;
                    }
                    lsproto::compare_ranges(a.from_ranges[0], b.from_ranges[0]).cmp(&0)
                });
            }
        }
        result
    }

    // Go groups the call sites in a map keyed by node id (random iteration order); the port keeps first-seen order.
    // callhierarchy.go:678
    fn symbol_and_entries_to_incoming_calls(
        &self,
        _ctx: &Context,
        _params: &incomingEntry,
        data: &SymbolAndEntriesData,
        _options: SymbolEntryTransformOptions,
    ) -> Result<lsproto::CallHierarchyIncomingCallsResponse, lsproto::Error> {
        let program = self.get_program();
        let mut ref_entries = Vec::new();
        for symbol_and_entry in &data.symbols_and_entries {
            ref_entries.extend(symbol_and_entry.references.iter().cloned());
        }

        let mut call_sites: Vec<callSite> = Vec::new();
        for entry in &ref_entries {
            if let Some(site) = convert_entry_to_call_site(entry) {
                call_sites.push(site);
            }
        }

        if call_sites.is_empty() {
            return Ok(lsproto::CallHierarchyIncomingCallsOrNull::default());
        }

        let mut grouped: OrderedMap<NodeId, Vec<callSite>> = OrderedMap::default();
        for site in call_sites {
            let key = get_call_site_group_key(&site);
            grouped.entry(key).or_default().push(site);
        }

        let mut result: Vec<lsproto::CallHierarchyIncomingCall> = Vec::new();
        for (_, sites) in grouped.iter() {
            if let Some(incoming_call) = self.convert_call_site_group_to_incoming_call(program, sites) {
                result.push(incoming_call);
            }
        }
        Ok(lsproto::CallHierarchyIncomingCallsOrNull { call_hierarchy_incoming_calls: Some(result) })
    }
}

// callhierarchy.go:711
struct callSiteCollector {
    program: &'static Program,
    call_sites: Vec<callSite>,
}

impl callSiteCollector {
    // callhierarchy.go:716
    fn record_call_site(&mut self, node: P<Node>) {
        let target: Option<P<Node>> = if ast::is_tagged_template_expression(node) {
            Some(node.as_tagged_template_expression().tag)
        } else if ast::is_jsx_opening_element(node) || ast::is_jsx_self_closing_element(node) {
            Some(node.tag_name())
        } else if ast::is_property_access_expression(node) || ast::is_element_access_expression(node) || ast::is_class_static_block_declaration(node) {
            Some(node)
        } else if ast::is_call_expression(node) || ast::is_new_expression(node) || ast::is_decorator(node) {
            node.expression()
        } else {
            None
        };

        let Some(target) = target else {
            return;
        };

        let Some(declaration) = resolve_call_hierarchy_declaration(self.program, target) else {
            return;
        };

        let source_file = ast::get_source_file_of_node(target).unwrap();
        let start = scanner::skip_trivia(source_file.text(), target.pos());
        let text_range = TextRange::new(start, target.end());

        match declaration {
            CallHierarchyDeclarations::Node(decl) => {
                self.call_sites.push(callSite { declaration: decl, text_range, source_file: source_file.as_node() });
            }
            CallHierarchyDeclarations::Nodes(decls) => {
                for d in decls {
                    self.call_sites.push(callSite { declaration: d, text_range, source_file: source_file.as_node() });
                }
            }
        }
    }

    // callhierarchy.go:769
    fn collect(&mut self, node: Option<P<Node>>) {
        let Some(node) = node else {
            return;
        };

        // do not descend into ambient nodes.
        if node.flags().intersects(NodeFlags::Ambient) {
            return;
        }

        // do not descend into other call site declarations, other than class member names
        if is_valid_call_hierarchy_declaration(Some(node)) {
            if ast::is_class_like(node) {
                for &member in node.members() {
                    if let Some(name) = member.name() {
                        if ast::is_computed_property_name(name) {
                            self.collect(name.expression());
                        }
                    }
                }
            }
            return;
        }

        match node.kind() {
            Kind::Identifier
            | Kind::ImportEqualsDeclaration
            | Kind::ImportDeclaration
            | Kind::ExportDeclaration
            | Kind::InterfaceDeclaration
            | Kind::TypeAliasDeclaration => {
                // do not descend into nodes that cannot contain callable nodes
                return;
            }
            Kind::ClassStaticBlockDeclaration => {
                self.record_call_site(node);
                return;
            }
            Kind::TypeAssertionExpression | Kind::AsExpression => {
                // do not descend into the type side of an assertion
                self.collect(node.expression());
                return;
            }
            Kind::VariableDeclaration | Kind::Parameter => {
                // do not descend into the type of a variable or parameter declaration
                self.collect(node.name());
                self.collect(node.initializer());
                return;
            }
            Kind::CallExpression | Kind::NewExpression => {
                // do not descend into the type arguments of a call/new expression
                self.record_call_site(node);
                self.collect(node.expression());
                for &arg in node.arguments() {
                    self.collect(Some(arg));
                }
                return;
            }
            Kind::TaggedTemplateExpression => {
                // do not descend into the type arguments of a tagged template expression
                self.record_call_site(node);
                let tagged_template = node.as_tagged_template_expression();
                self.collect(Some(tagged_template.tag));
                self.collect(Some(tagged_template.template));
                return;
            }
            Kind::JsxOpeningElement | Kind::JsxSelfClosingElement => {
                // do not descend into the type arguments of a JsxOpeningLikeElement
                self.record_call_site(node);
                self.collect(Some(node.tag_name()));
                self.collect(node.attributes());
                return;
            }
            Kind::Decorator => {
                self.record_call_site(node);
                self.collect(node.expression());
                return;
            }
            Kind::PropertyAccessExpression | Kind::ElementAccessExpression => {
                self.record_call_site(node);
                node.for_each_child(&mut |child| {
                    self.collect(Some(child));
                    false
                });
                return;
            }
            Kind::SatisfiesExpression => {
                // do not descend into the type side of an assertion
                self.collect(node.expression());
                return;
            }
            _ => {}
        }

        if ast::is_part_of_type_node(node) {
            // do not descend into types
            return;
        }

        node.for_each_child(&mut |child| {
            self.collect(Some(child));
            false
        });
    }
}

// callhierarchy.go:869
fn collect_call_sites(program: &'static Program, c: &mut Checker, node: P<Node>) -> Vec<callSite> {
    let mut collector = callSiteCollector { program, call_sites: Vec::new() };

    match node.kind() {
        Kind::SourceFile => {
            for &stmt in node.statements() {
                collector.collect(Some(stmt));
            }
        }

        Kind::ModuleDeclaration => {
            let body = node.body();
            if let Some(body) = body.filter(|&body| !ast::has_syntactic_modifier(node, ModifierFlags::Ambient) && ast::is_module_block(body)) {
                for &stmt in body.statements() {
                    collector.collect(Some(stmt));
                }
            }
        }

        Kind::FunctionDeclaration | Kind::FunctionExpression | Kind::ArrowFunction | Kind::MethodDeclaration | Kind::GetAccessor | Kind::SetAccessor => {
            let implementation = find_implementation(c, Some(node));
            if let Some(implementation) = implementation {
                for &param in implementation.parameters() {
                    collector.collect(Some(param));
                }
                collector.collect(implementation.body());
            }
        }

        Kind::ClassDeclaration | Kind::ClassExpression => {
            if let Some(modifiers) = node.modifiers() {
                for &m in modifiers.nodes() {
                    collector.collect(Some(m));
                }
            }

            let heritage = ast::get_class_extends_heritage_element(node);
            if let Some(heritage) = heritage {
                collector.collect(heritage.expression());
            }

            for &member in node.members() {
                if ast::can_have_modifiers(member) {
                    if let Some(modifiers) = member.modifiers() {
                        for &m in modifiers.nodes() {
                            collector.collect(Some(m));
                        }
                    }
                }

                if ast::is_property_declaration(member) {
                    collector.collect(member.initializer());
                } else if ast::is_constructor_declaration(member) {
                    if let Some(body) = member.body() {
                        for &param in member.parameters() {
                            collector.collect(Some(param));
                        }
                        collector.collect(Some(body));
                    }
                } else if ast::is_class_static_block_declaration(member) {
                    collector.collect(Some(member));
                }
            }
        }

        Kind::ClassStaticBlockDeclaration => {
            let static_block = node.as_class_static_block_declaration();
            collector.collect(Some(static_block.body));
        }

        _ => panic!("Unhandled case in collectCallSites: {:?}", node.kind()),
    }

    collector.call_sites
}

impl LanguageService {
    // callhierarchy.go:942
    fn convert_call_site_group_to_outgoing_call(&self, program: &'static Program, entries: &[callSite]) -> Option<lsproto::CallHierarchyOutgoingCall> {
        let mut from_ranges: Vec<lsproto::Range> = Vec::with_capacity(entries.len());
        for entry in entries {
            let source_file = entry.source_file.as_source_file_p();
            let (lsp_range, fidelity) = self.converters.to_lsp_range_for_feature(&source_file, entry.text_range, Feature::CallHierarchy);
            if !fidelity.is_none() {
                from_ranges.push(lsp_range);
            }
        }
        let to = self.create_call_hierarchy_item(program, entries[0].declaration);
        let to = to?;
        if from_ranges.is_empty() {
            return None;
        }

        from_ranges.sort_by(|a, b| lsproto::compare_ranges(*a, *b).cmp(&0));

        Some(lsproto::CallHierarchyOutgoingCall { to, from_ranges })
    }

    // Gets the call sites that call out of the provided call hierarchy declaration.
    // Go groups the call sites in a map keyed by node id (random iteration order); the port keeps first-seen order.
    // callhierarchy.go:964
    fn get_outgoing_calls(&self, program: &'static Program, declaration: P<Node>) -> Vec<lsproto::CallHierarchyOutgoingCall> {
        if declaration.flags().intersects(NodeFlags::Ambient) || ast::is_method_signature_declaration(declaration) {
            return Vec::new();
        }

        let mut c = program.get_type_checker(&Context::background());
        let c: &mut Checker = &mut c;

        let call_sites = collect_call_sites(program, c, declaration);

        if call_sites.is_empty() {
            return Vec::new();
        }

        let mut grouped: OrderedMap<NodeId, Vec<callSite>> = OrderedMap::default();
        for site in call_sites {
            let key = get_call_site_group_key(&site);
            grouped.entry(key).or_default().push(site);
        }

        let mut result: Vec<lsproto::CallHierarchyOutgoingCall> = Vec::new();
        for (_, sites) in grouped.iter() {
            if let Some(outgoing_call) = self.convert_call_site_group_to_outgoing_call(program, sites) {
                result.push(outgoing_call);
            }
        }

        result.sort_by(|a, b| {
            let uri_comp = a.to.uri.0.cmp(&b.to.uri.0);
            if uri_comp != std::cmp::Ordering::Equal {
                return uri_comp;
            }
            if a.from_ranges.is_empty() || b.from_ranges.is_empty() {
                return std::cmp::Ordering::Equal;
            }
            lsproto::compare_ranges(a.from_ranges[0], b.from_ranges[0]).cmp(&0)
        });

        result
    }

    // callhierarchy.go:1004
    pub fn provide_prepare_call_hierarchy(
        &self,
        _ctx: &Context,
        document_uri: &lsproto::DocumentUri,
        position: lsproto::Position,
    ) -> Result<lsproto::CallHierarchyPrepareResponse, lsproto::Error> {
        let (program, file) = self.get_program_and_file(document_uri);
        let declarations = self.call_hierarchy_declarations(file, position, program, false);
        let mut items: Vec<lsproto::CallHierarchyItem> = Vec::new();
        let mut seen: Set<lsproto::Location> = Set::new();
        for declaration in declarations {
            if let Some(item) = self.create_call_hierarchy_item(program, declaration) {
                let location = lsproto::Location { uri: item.uri.clone(), range: item.selection_range };
                if seen.add_if_absent(location) {
                    items.push(item);
                }
            }
        }

        if items.is_empty() {
            return Ok(lsproto::CallHierarchyItemsOrNull::default());
        }
        Ok(lsproto::CallHierarchyItemsOrNull { call_hierarchy_items: Some(items) })
    }

    // callhierarchy.go:1028
    pub fn provide_call_hierarchy_incoming_calls(
        &self,
        ctx: &Context,
        item: &lsproto::CallHierarchyItem,
        orchestrator: Option<&dyn CrossProjectOrchestrator>,
    ) -> Result<lsproto::CallHierarchyIncomingCallsResponse, lsproto::Error> {
        let program = self.get_program();
        let file_name = item.uri.file_name();
        let Some(file) = program.get_source_file(&file_name) else {
            return Ok(lsproto::CallHierarchyIncomingCallsOrNull::default());
        };

        let declarations = self.call_hierarchy_declarations(file, item.selection_range.start, program, true);
        let mut calls: Vec<lsproto::CallHierarchyIncomingCall> = Vec::new();
        let mut seen: FxHashMap<lsproto::Location, usize> = FxHashMap::default();
        for declaration in declarations {
            let response = self.get_incoming_calls(ctx, program, declaration, orchestrator)?;
            if let Some(response_calls) = response.call_hierarchy_incoming_calls {
                for call in response_calls {
                    let location = lsproto::Location { uri: call.from.uri.clone(), range: call.from.selection_range };
                    if let Some(&existing) = seen.get(&location) {
                        for from_range in call.from_ranges {
                            if !calls[existing].from_ranges.contains(&from_range) {
                                calls[existing].from_ranges.push(from_range);
                            }
                        }
                    } else {
                        seen.insert(location, calls.len());
                        calls.push(call);
                    }
                }
            }
        }
        if calls.is_empty() {
            return Ok(lsproto::CallHierarchyIncomingCallsOrNull::default());
        }
        Ok(lsproto::CallHierarchyIncomingCallsOrNull { call_hierarchy_incoming_calls: Some(calls) })
    }

    // callhierarchy.go:1070
    pub fn provide_call_hierarchy_outgoing_calls(&self, _ctx: &Context, item: &lsproto::CallHierarchyItem) -> Result<lsproto::CallHierarchyOutgoingCallsResponse, lsproto::Error> {
        let program = self.get_program();
        let file_name = item.uri.file_name();
        let Some(file) = program.get_source_file(&file_name) else {
            return Ok(lsproto::CallHierarchyOutgoingCallsOrNull::default());
        };

        let declarations = self.call_hierarchy_declarations(file, item.selection_range.start, program, true);
        let mut calls: Vec<lsproto::CallHierarchyOutgoingCall> = Vec::new();
        let mut seen: FxHashMap<lsproto::Location, usize> = FxHashMap::default();
        for declaration in declarations {
            for call in self.get_outgoing_calls(program, declaration) {
                let location = lsproto::Location { uri: call.to.uri.clone(), range: call.to.selection_range };
                if let Some(&existing) = seen.get(&location) {
                    for from_range in call.from_ranges {
                        if !calls[existing].from_ranges.contains(&from_range) {
                            calls[existing].from_ranges.push(from_range);
                        }
                    }
                } else {
                    seen.insert(location, calls.len());
                    calls.push(call);
                }
            }
        }
        if calls.is_empty() {
            return Ok(lsproto::CallHierarchyOutgoingCallsOrNull::default());
        }
        Ok(lsproto::CallHierarchyOutgoingCallsOrNull { call_hierarchy_outgoing_calls: Some(calls) })
    }

    // callhierarchy.go:1105
    fn call_hierarchy_declarations(&self, file: P<SourceFile>, position: lsproto::Position, program: &'static Program, allow_source_file: bool) -> Vec<P<Node>> {
        let positions = self.converters.from_lsp_position_for_source_file(file, position, Feature::CallHierarchy);
        let mut declarations: Vec<P<Node>> = Vec::new();
        let mut seen: FxHashSet<P<Node>> = FxHashSet::default();
        for mapped in positions {
            if !mapped.fidelity.is_single_segment() {
                continue;
            }
            let file = mapped.script;
            let pos = mapped.position;
            let mut node = file.as_node();
            if pos != 0 {
                node = astnav::get_touching_property_name(file, pos);
            }
            if !allow_source_file && node.kind() == Kind::SourceFile {
                continue;
            }
            match resolve_call_hierarchy_declaration(program, node) {
                Some(CallHierarchyDeclarations::Node(declaration)) => {
                    if seen.insert(declaration) {
                        declarations.push(declaration);
                    }
                }
                Some(CallHierarchyDeclarations::Nodes(decls)) => {
                    for declaration in decls {
                        if seen.insert(declaration) {
                            declarations.push(declaration);
                        }
                    }
                }
                None => {}
            }
        }
        declarations
    }
}
