// PLACEHOLDER (see mod.rs): import_adder.go, reduced to what completions and the missing-member fixer call. The
// type-node conversion functions are ported in full; the adder itself is not (a View, which `NewImportAdder`
// needs, is never handed out while the registry is never prepared).

use std::cell::RefCell;
use std::rc::Rc;

use rustc_hash::FxHashMap;
use tsrs_ast::{self as ast, Node, NodeFactory, NodeFactoryHooks, NodeVisitor, NodeVisitorHooks, SourceFile, Symbol};
use tsrs_checker::{self as checker, Checker, Flags, Type};
use tsrs_compiler::Program;
use tsrs_core::context::Context;
use tsrs_core::P;
use tsrs_lsproto as lsproto;

use super::{Fix, View};
use crate::lsconv::Converters;
use crate::lsutil::{self, FormatCodeSettings, UserPreferences};

pub type IdToSymbol = P<RefCell<FxHashMap<P<Node>, P<Symbol>>>>;

// import_adder.go:24
pub trait ImportAdder {
    fn has_fixes(&self) -> bool;
    fn add_import_from_exported_symbol(&mut self, symbol: P<Symbol>, is_valid_type_only_use_site: bool);
    fn add_import_fix(&mut self, fix: &Fix);
    fn edits(&mut self) -> Vec<lsproto::TextEdit>;
}

// import_adder.go:53 (placeholder: no state)
struct importAdder;

// import_adder.go:70 (placeholder: unreachable until the registry is ported, since no prepared View exists)
pub fn new_import_adder(
    _ctx: &Context,
    _program: &'static Program,
    _checker: &mut Checker,
    _file: P<SourceFile>,
    _view: View,
    _format_options: FormatCodeSettings,
    _converters: std::sync::Arc<Converters>,
    _preferences: UserPreferences,
) -> Box<dyn ImportAdder> {
    Box::new(importAdder)
}

impl ImportAdder for importAdder {
    fn has_fixes(&self) -> bool {
        false
    }
    fn add_import_from_exported_symbol(&mut self, _symbol: P<Symbol>, _is_valid_type_only_use_site: bool) {
        unreachable!("autoimport placeholder: no import adder exists while the registry is never prepared")
    }
    fn add_import_fix(&mut self, _fix: &Fix) {
        unreachable!("autoimport placeholder: no import adder exists while the registry is never prepared")
    }
    fn edits(&mut self) -> Vec<lsproto::TextEdit> {
        Vec::new()
    }
}

// import_adder.go:382
pub fn type_to_auto_importable_type_node(
    c: &mut Checker,
    import_adder: Option<&mut dyn ImportAdder>,
    t: P<Type>,
    context_node: P<Node>, // !!! flags
) -> Option<P<Node>> {
    let id_to_symbol: IdToSymbol = P::new(RefCell::new(FxHashMap::default()));
    let type_node = c.type_to_type_node(t, Some(context_node), Flags::None, Some(id_to_symbol))?;
    type_node_to_auto_importable_type_node(type_node, import_adder, id_to_symbol)
}

// TypeNodeToAutoImportableTypeNode converts import type references in a type node to
// simple type references and registers needed imports with the import adder.
// import_adder.go:398
pub fn type_node_to_auto_importable_type_node(mut type_node: P<Node>, import_adder: Option<&mut dyn ImportAdder>, id_to_symbol: IdToSymbol) -> Option<P<Node>> {
    let (reference_type_node, importable_symbols) = try_get_auto_importable_reference_from_type_node(Some(type_node), id_to_symbol);
    if let Some(reference_type_node) = reference_type_node {
        if let Some(import_adder) = import_adder {
            import_symbols(import_adder, &importable_symbols);
        }
        type_node = reference_type_node;
    }

    // !!! handle type node reuse: nodes needs to be fresh here but also preserve symbols
    Some(type_node)
}

// import_adder.go:415
fn import_symbols(import_adder: &mut dyn ImportAdder, symbols: &[P<Symbol>]) {
    for &symbol in symbols {
        import_adder.add_import_from_exported_symbol(symbol, true /*isValidTypeOnlyUseSite*/);
    }
}

// Given a type node containing 'import("./a").SomeType<import("./b").OtherType<...>>',
// returns an equivalent type reference node with any nested ImportTypeNodes also replaced
// with type references, and a list of symbols that must be imported to use the type reference.
// TryGetAutoImportableReferenceFromTypeNode converts import type references in a type node
// to simple type references and returns the transformed type node and the symbols that need
// to be imported.
// import_adder.go:427
pub fn try_get_auto_importable_reference_from_type_node(import_type_node: Option<P<Node>>, id_to_symbol: IdToSymbol) -> (Option<P<Node>>, Vec<P<Symbol>>) {
    let symbols: Rc<RefCell<Vec<P<Symbol>>>> = Rc::new(RefCell::new(Vec::new()));
    let factory = NodeFactory::new(NodeFactoryHooks::default());
    let visit_symbols = symbols.clone();
    let visit_factory = factory.clone();
    let visit = move |visitor: &mut NodeVisitor, node: P<Node>| -> Option<P<Node>> {
        if ast::is_literal_import_type_node(node) && node.as_import_type_node().qualifier.is_some() {
            let import_type_node = node.as_import_type_node();
            let qualifier_node = import_type_node.qualifier.unwrap();
            // Symbol for the left-most thing after the dot
            let first_identifier = ast::get_first_identifier(qualifier_node);
            let symbol = id_to_symbol.borrow().get(&first_identifier).copied();
            let Some(symbol) = symbol else {
                // if symbol is missing then this doesn't come from a synthesized import type node
                // it has to be an import type node authored by the user and thus it has to be valid
                // it can't refer to reserved internal symbol names and such
                return Some(node.visit_each_child(visitor));
            };
            let name = get_name_for_exported_symbol(symbol, false /*preferCapitalized*/);
            let qualifier = if name != first_identifier.text() {
                replace_first_identifier_of_entity_name(&visit_factory, qualifier_node, visit_factory.new_identifier(tsrs_core::alloc_str(&name)))
            } else {
                qualifier_node
            };
            visit_symbols.borrow_mut().push(symbol);
            let type_arguments = visitor.visit_nodes(import_type_node.type_arguments());
            return Some(visit_factory.new_type_reference_node(qualifier, type_arguments));
        }
        visitor.visit_each_child(Some(node))
    };
    let mut visitor = ast::new_node_visitor(Some(Rc::new(visit)), Some(factory), NodeVisitorHooks::default());

    let type_node = visitor.visit_node(import_type_node);
    assert!(type_node.is_none() || ast::is_type_node(type_node.unwrap()), "expected a type node");
    let symbols = symbols.borrow().clone();
    (type_node, symbols)
}

// If a type checker and multiple files are available, consider using `forEachNameOfDefaultExport`
// instead, which searches for names of re-exported defaults/namespaces in target files.
// import_adder.go:465
fn get_name_for_exported_symbol(symbol: P<Symbol>, prefer_capitalized: bool) -> String {
    if symbol.name() == ast::InternalSymbolNameExportEquals || symbol.name() == ast::InternalSymbolNameDefault {
        // Names for default exports:
        // - export default foo => foo
        // - export { foo as default } => foo
        // - export default 0 => filename converted to camelCase
        let name = get_default_like_export_name_from_declaration(symbol);
        if !name.is_empty() {
            return name;
        }
        assert!(symbol.parent().is_some(), "Expected exported symbol to have module symbol as parent");
        return lsutil::module_symbol_to_valid_identifier(symbol.parent().unwrap(), prefer_capitalized);
    }
    symbol.name().to_string()
}

// import_adder.go:481
fn replace_first_identifier_of_entity_name(factory: &NodeFactory, name: P<Node>, new_identifier: P<Node>) -> P<Node> {
    if name.kind() == ast::Kind::Identifier {
        return new_identifier;
    }
    factory.new_qualified_name(replace_first_identifier_of_entity_name(factory, name.as_qualified_name().left, new_identifier), name.as_qualified_name().right)
}

// util.go:118
fn get_default_like_export_name_from_declaration(symbol: P<Symbol>) -> String {
    for &d in symbol.declarations() {
        // "export default" in this case. See `ExportAssignment`for more details.
        if ast::is_export_assignment(d) {
            let inner_expression = ast::skip_outer_expressions(d.expression().unwrap(), ast::OEK::All);
            if ast::is_identifier(inner_expression) {
                return inner_expression.text().to_string();
            }
            continue;
        }
        // "export { ~ as default }"
        if ast::is_export_specifier(d) && d.symbol().unwrap().flags() == ast::SymbolFlags::Alias && d.property_name().is_some() {
            if d.property_name().unwrap().kind() == ast::Kind::Identifier {
                return d.property_name().unwrap().text().to_string();
            }
            continue;
        }
        // GH#52694
        if let Some(name) = ast::get_name_of_declaration(Some(d)) {
            if name.kind() == ast::Kind::Identifier {
                return name.text().to_string();
            }
        }
        if let Some(parent) = symbol.parent() {
            if !checker::is_external_module_symbol(parent) {
                return parent.name().to_string();
            }
        }
    }
    String::new()
}
