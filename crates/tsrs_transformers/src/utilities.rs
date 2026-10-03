// Port of transformers/utilities.go.

use crate::*;

// utilities.go:241
pub fn is_simple_copiable_expression(expression: P<Node>) -> bool {
    ast::is_string_literal_like(expression) || ast::is_numeric_literal(expression) || ast::is_keyword_kind(expression.kind()) || ast::is_identifier(expression)
}

// utilities.go:248
pub fn is_original_node_single_line(emit_context: P<EmitContext>, node: Option<P<Node>>) -> bool {
    let Some(node) = node else {
        return false;
    };
    let Some(original) = emit_context.most_original(Some(node)) else {
        return false;
    };
    let Some(source) = ast::get_source_file_of_node(Some(original)) else {
        return false;
    };
    let start_line = scanner::get_ecma_line_of_position(&*source, original.pos());
    let end_line = scanner::get_ecma_line_of_position(&*source, original.end());
    start_line == end_line
}

// utilities.go:270
pub fn is_simple_inlineable_expression(expression: P<Node>) -> bool {
    !ast::is_identifier(expression) && is_simple_copiable_expression(expression)
}
