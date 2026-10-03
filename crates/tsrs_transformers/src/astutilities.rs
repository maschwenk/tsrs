// Go `ast/utilities.go` functions that tsrs_ast does not export (tsrs_checker has crate-private copies of the first
// two from checker/utilities.go, so adding them to the `tsrs_ast` glob would make the checker's names ambiguous).

use crate::*;

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
