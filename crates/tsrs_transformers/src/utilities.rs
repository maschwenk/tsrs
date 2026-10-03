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

// utilities.go:12
pub fn is_generated_identifier(emit_context: P<EmitContext>, name: P<Node>) -> bool {
    emit_context.has_auto_generate_info(Some(name))
}

// utilities.go:308
// MoveRangePastModifiers returns a text range that starts past any modifiers on the node.
pub fn move_range_past_modifiers(node: P<Node>) -> tsrs_core::TextRange {
    if ast::is_property_declaration(node) || ast::is_method_declaration(node) {
        return tsrs_core::TextRange::new(node.name().unwrap().pos(), node.end());
    }

    let mut last_modifier: Option<P<Node>> = None;
    if ast::can_have_modifiers(node) {
        last_modifier = node.modifier_nodes().last().copied();
    }

    if let Some(last_modifier) = last_modifier {
        if !ast::position_is_synthesized(last_modifier.end()) {
            return tsrs_core::TextRange::new(last_modifier.end(), node.end());
        }
    }
    move_range_past_decorators(node)
}

// utilities.go:325
// MoveRangePastDecorators returns a text range that starts past any decorators on the node.
pub fn move_range_past_decorators(node: P<Node>) -> tsrs_core::TextRange {
    let mut last_decorator: Option<P<Node>> = None;
    if ast::can_have_modifiers(node) {
        let nodes = node.modifier_nodes();
        last_decorator = nodes.iter().rev().find(|n| ast::is_decorator(**n)).copied();
    }

    if let Some(last_decorator) = last_decorator {
        if !ast::position_is_synthesized(last_decorator.end()) {
            return tsrs_core::TextRange::new(last_decorator.end(), node.end());
        }
    }
    node.loc()
}
