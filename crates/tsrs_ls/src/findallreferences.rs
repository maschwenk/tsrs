// PARTIAL port of findallreferences.go (references are phase 3): only what the phase-1 files use.

use tsrs_ast::{self as ast, FileReference, Kind, Node, SourceFile};
use tsrs_core::{TextRange, P};
use tsrs_scanner as scanner;

// findallreferences.go:48
pub(crate) struct RefInfo {
    pub(crate) file: Option<P<SourceFile>>,
    pub(crate) file_name: String,
    pub(crate) reference: Option<P<FileReference>>,
    pub(crate) unverified: bool,
}

// findallreferences.go:286
pub(crate) fn get_context_node(node: Option<P<Node>>) -> Option<P<Node>> {
    let node = node?;
    match node.kind() {
        Kind::VariableDeclaration => {
            let parent = node.parent().unwrap();
            if !ast::is_variable_declaration_list(parent) || parent.as_variable_declaration_list().declarations.nodes.len() != 1 {
                Some(node)
            } else if ast::is_variable_statement(parent.parent().unwrap()) {
                parent.parent()
            } else if ast::is_for_in_or_of_statement(parent.parent().unwrap()) {
                get_context_node(parent.parent())
            } else {
                Some(parent)
            }
        }

        Kind::BindingElement => get_context_node(node.parent().unwrap().parent()),

        Kind::ImportSpecifier => node.parent().unwrap().parent().unwrap().parent(),

        Kind::ExportSpecifier | Kind::NamespaceImport => node.parent().unwrap().parent(),

        Kind::ImportClause | Kind::NamespaceExport => node.parent(),

        Kind::BinaryExpression => {
            if node.parent().unwrap().kind() == Kind::ExpressionStatement {
                node.parent()
            } else {
                Some(node)
            }
        }

        Kind::ForOfStatement | Kind::ForInStatement => {
            // !!! not implemented
            None
        }

        Kind::PropertyAssignment | Kind::ShorthandPropertyAssignment => {
            if ast::is_array_literal_or_object_literal_destructuring_pattern(node.parent()) {
                return get_context_node(ast::find_ancestor(node.parent(), |node| {
                    node.kind() == Kind::BinaryExpression || ast::is_for_in_or_of_statement(node)
                }));
            }
            Some(node)
        }
        Kind::SwitchStatement => {
            // !!! not implemented
            None
        }
        _ => Some(node),
    }
}

// findallreferences.go:335
pub(crate) fn get_range_of_node(node: P<Node>, source_file: Option<P<SourceFile>>, end_node: Option<P<Node>>) -> TextRange {
    let source_file = match source_file {
        Some(f) => f,
        None => ast::get_source_file_of_node(node).unwrap(),
    };
    let mut start = scanner::get_token_pos_of_node(node, source_file, false /*includeJsDoc*/);
    let mut end = end_node.unwrap_or(node).end();
    if ast::is_string_literal_like(node) && (end - start) > 2 {
        if end_node.is_some() {
            panic!("endNode is not nil for stringLiteralLike");
        }
        start += 1;
        end -= 1;
    }
    if let Some(end_node) = end_node {
        if end_node.kind() == Kind::CaseBlock {
            end = end_node.pos();
        }
    }
    TextRange::new(start, end)
}
