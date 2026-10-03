// ast/utilities.go functions missing from tsrs_ast (Go package ast).

use crate::*;

// ast/utilities.go:1708
pub fn is_export_namespace_as_default_declaration(node: P<Node>) -> bool {
    if ast::is_export_declaration(node) {
        let export_clause = node.as_export_declaration().export_clause.unwrap();
        return ast::is_namespace_export(export_clause) && ast::module_export_name_is_default(export_clause.name().unwrap());
    }
    false
}

// ast/utilities.go:4038
pub fn is_empty_object_literal(expression: P<Node>) -> bool {
    ast::is_object_literal_expression(expression) && expression.properties().is_empty()
}

// ast/utilities.go:4042
pub fn is_empty_array_literal(expression: P<Node>) -> bool {
    ast::is_array_literal_expression(expression) && expression.elements().is_empty()
}

// ast/utilities.go:4046
pub fn get_rest_indicator_of_binding_or_assignment_element(binding_element: P<Node>) -> Option<P<Node>> {
    match binding_element.kind() {
        Kind::Parameter => binding_element.as_parameter_declaration().dot_dot_dot_token(),
        Kind::BindingElement => binding_element.as_binding_element().dot_dot_dot_token(),
        Kind::SpreadElement | Kind::SpreadAssignment => Some(binding_element),
        _ => None,
    }
}
