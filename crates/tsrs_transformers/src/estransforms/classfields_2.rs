// Port of estransforms/classfields.go, part 2 (Go lines 1889-3618: class declarations/expressions, members,
// constructors, private-name environments, destructuring targets and the free helpers). Part 1 is classfields_1.rs.
// Functions whose body is `unimplemented!("classfields part 2")` are signatures only, still to be ported.

use super::*;
use crate::*;
use printer::PrivateIdentifierKind;

impl classFieldsTransformer {
    // classfields.go:1889
    pub(crate) fn visit_class_declaration(&self, node: P<Node>) -> Option<P<Node>> {
        let _ = node;
        unimplemented!("classfields part 2")
    }

    // classfields.go:2003
    pub(crate) fn visit_class_expression(&self, node: P<Node>) -> Option<P<Node>> {
        let _ = node;
        unimplemented!("classfields part 2")
    }

    // classfields.go:2181
    pub(crate) fn visit_class_static_block_declaration(&self, node: P<Node>) -> Option<P<Node>> {
        let _ = node;
        unimplemented!("classfields part 2")
    }

    // classfields.go:2194
    pub(crate) fn visit_this_expression(&self, node: P<Node>) -> Option<P<Node>> {
        let _ = node;
        unimplemented!("classfields part 2")
    }

    // classfields.go:2365
    pub(crate) fn transform_constructor(&self, constructor: P<Node>, container: P<Node>) -> Option<P<Node>> {
        let _ = (constructor, container);
        unimplemented!("classfields part 2")
    }

    // classfields.go:2650
    pub(crate) fn transform_property_or_class_static_block(&self, property: P<Node>, receiver: P<Node>) -> Option<P<Node>> {
        let _ = (property, receiver);
        unimplemented!("classfields part 2")
    }

    // classfields.go:2853
    pub(crate) fn visit_invalid_super_property(&self, node: P<Node>) -> Option<P<Node>> {
        let _ = node;
        unimplemented!("classfields part 2")
    }

    // classfields.go:2876
    pub(crate) fn get_property_name_expression_if_needed(&self, name: P<Node>, should_hoist: bool) -> Option<P<Node>> {
        let _ = (name, should_hoist);
        unimplemented!("classfields part 2")
    }

    // classfields.go:2910
    pub(crate) fn start_class_lexical_environment(&self) {
        self.lexical_environment.set(Some(P::new(classLexicalEnv { previous: self.lexical_environment.get(), data: Cell::new(None), private_env: Cell::new(None) })));
    }

    // classfields.go:2914
    pub(crate) fn end_class_lexical_environment(&self) {
        self.lexical_environment.set(self.lexical_environment.get().unwrap().previous);
    }

    // classfields.go:2918
    pub(crate) fn get_class_lexical_environment(&self) -> P<classLexicalEnvironment> {
        assert!(self.lexical_environment.get().is_some());
        let lex = self.lexical_environment.get().unwrap();
        if lex.data.get().is_none() {
            lex.data.set(Some(P::new(classLexicalEnvironment::default())));
        }
        lex.data.get().unwrap()
    }

    // classfields.go:2926
    pub(crate) fn get_private_identifier_environment(&self) -> P<privateEnvironment> {
        assert!(self.lexical_environment.get().is_some());
        let lex = self.lexical_environment.get().unwrap();
        if lex.private_env.get().is_none() {
            lex.private_env.set(Some(P::new(privateEnvironment::default())));
        }
        lex.private_env.get().unwrap()
    }

    // classfields.go:2936
    pub(crate) fn add_pending_expressions(&self, exprs: &[P<Node>]) {
        self.pending_expressions.borrow_mut().extend_from_slice(exprs);
    }

    // classfields.go:3102
    pub(crate) fn set_private_identifier(&self, env: P<privateEnvironment>, name: P<Node>, info: P<privateIdentifierInfo>) {
        if self.emit_context().has_auto_generate_info(Some(name)) {
            env.generated_identifiers.borrow_mut().insert(self.emit_context().get_node_for_generated_name(name), info);
        } else {
            env.members.borrow_mut().insert(name.text().to_string(), info);
        }
    }

    // classfields.go:3113
    pub(crate) fn get_private_identifier(&self, env: P<privateEnvironment>, name: P<Node>) -> Option<P<privateIdentifierInfo>> {
        if self.emit_context().has_auto_generate_info(Some(name)) {
            return env.generated_identifiers.borrow().get(&self.emit_context().get_node_for_generated_name(name)).copied();
        }
        env.members.borrow().get(name.text()).copied()
    }

    // classfields.go:3122
    pub(crate) fn create_hoisted_variable_for_class(&self, name_text: &str, node: P<Node>, suffix: &str) -> P<Node> {
        let _ = (name_text, node, suffix);
        unimplemented!("classfields part 2")
    }

    // classfields.go:3181
    pub(crate) fn access_private_identifier(&self, name: P<Node>) -> Option<P<privateIdentifierInfo>> {
        let mut env = self.lexical_environment.get();
        while let Some(e) = env {
            if let Some(private_env) = e.private_env.get() {
                if let Some(info) = self.get_private_identifier(private_env, name) {
                    if info.kind == PrivateIdentifierKind::Untransformed {
                        return None;
                    }
                    return Some(info);
                }
            }
            env = e.previous;
        }
        None
    }

    // classfields.go:3195
    pub(crate) fn wrap_private_identifier_for_destructuring_target(&self, node: P<Node>) -> Option<P<Node>> {
        let _ = node;
        unimplemented!("classfields part 2")
    }

    // classfields.go:3256
    pub(crate) fn visit_array_assignment_element(&self, node: P<Node>) -> Option<P<Node>> {
        let _ = node;
        unimplemented!("classfields part 2")
    }

    // classfields.go:3320
    pub(crate) fn visit_object_assignment_element(&self, node: P<Node>) -> Option<P<Node>> {
        let _ = node;
        unimplemented!("classfields part 2")
    }

    // classfields.go:3334
    pub(crate) fn visit_assignment_pattern(&self, node: P<Node>) -> Option<P<Node>> {
        let _ = node;
        unimplemented!("classfields part 2")
    }
}

// classfields.go:3395
pub(crate) fn is_static_property_declaration_or_class_static_block(node: P<Node>) -> bool {
    ast::is_class_static_block_declaration(node) || (ast::is_property_declaration(node) && ast::has_static_modifier(node))
}

impl classFieldsTransformer {
    // classfields.go:3400
    pub(crate) fn get_properties(&self, node: P<Node>, require_initializer: bool, is_static: bool) -> Vec<P<Node>> {
        let mut result = Vec::new();
        for member in node.members() {
            if ast::is_property_declaration(*member) && (!require_initializer || member.initializer().is_some()) && ast::has_static_modifier(*member) == is_static {
                result.push(*member);
            }
        }
        result
    }

    // classfields.go:3412
    pub(crate) fn get_static_properties_and_class_static_block(&self, node: P<Node>) -> Vec<P<Node>> {
        let mut result = Vec::new();
        for member in node.members() {
            if ast::is_class_static_block_declaration(*member) || (ast::is_property_declaration(*member) && ast::has_static_modifier(*member)) {
                result.push(*member);
            }
        }
        result
    }
}

// classfields.go:3423
// classHasClassThisAssignment checks if a class has a static block that is a class-this assignment.
pub(crate) fn class_has_class_this_assignment(emit_context: P<EmitContext>, node: P<Node>) -> bool {
    for member in node.members() {
        if is_class_this_assignment_block(emit_context, *member) {
            return true;
        }
    }
    false
}

// classfields.go:3432
pub(crate) fn is_non_static_method_or_accessor_with_private_name(member: P<Node>) -> bool {
    !ast::is_static(member) && (ast::is_method_or_accessor(member) || ast::is_auto_accessor_property_declaration(member)) && ast::is_private_identifier(member.name().unwrap())
}

// classfields.go:3438
pub(crate) fn create_member_access_for_property_name(factory: &printer::NodeFactory, emit_context: P<EmitContext>, receiver: P<Node>, name: P<Node>, location: P<Node>) -> P<Node> {
    if ast::is_computed_property_name(name) {
        let expression = factory.new_element_access_expression(receiver, None, name.expression().unwrap(), NodeFlags::None);
        expression.set_loc(location.loc());
        return expression;
    }
    let expression: P<Node>;
    if ast::is_identifier(name) || ast::is_private_identifier(name) {
        expression = factory.new_property_access_expression(receiver, None, name, NodeFlags::None);
    } else {
        // string or numeric literal
        expression = factory.new_element_access_expression(receiver, None, name, NodeFlags::None);
    }
    emit_context.set_comment_range(expression, name.loc());
    emit_context.set_source_map_range(expression, name.loc());
    emit_context.add_emit_flags(expression, printer::EmitFlags::NoNestedSourceMaps);
    expression
}

impl classFieldsTransformer {
    // classfields.go:3457
    // Returns (thisArg, target).
    pub(crate) fn create_call_binding(&self, node: P<Node>) -> (P<Node>, P<Node>) {
        if ast::is_super_property(node) {
            return (self.factory().new_this_expression(), node);
        }
        if ast::is_property_access_expression(node) {
            let expr = node.as_property_access_expression();
            if should_be_captured_in_temp_variable(expr.expression) {
                let this_arg = self.factory().new_temp_variable();
                self.emit_context().add_variable_declaration(this_arg);
                let target = self.factory().new_property_access_expression(
                    self.factory().new_parenthesized_expression(
                        // TODO: do we even need these?
                        self.factory().new_assignment_expression(this_arg, expr.expression),
                    ),
                    None,
                    expr.name(),
                    NodeFlags::None,
                );
                return (this_arg, target);
            }
            return (expr.expression, node);
        }
        let this_arg = self.factory().new_void_zero_expression();
        let target = node;
        (this_arg, target)
    }
}

// classfields.go:3483
pub(crate) fn should_be_captured_in_temp_variable(node: P<Node>) -> bool {
    let target = ast::skip_parentheses(node);
    !matches!(target.kind(), Kind::Identifier | Kind::ThisKeyword | Kind::NumericLiteral | Kind::BigIntLiteral | Kind::StringLiteral)
}

impl classFieldsTransformer {
    // classfields.go:3493
    pub(crate) fn create_accessor_property_get_redirector(&self, node: P<Node>, modifiers: Option<P<ModifierList>>, name: P<Node>, receiver: P<Node>) -> P<Node> {
        let _ = (node, modifiers, name, receiver);
        unimplemented!("classfields part 2")
    }

    // classfields.go:3514
    pub(crate) fn create_accessor_property_set_redirector(&self, node: P<Node>, modifiers: Option<P<ModifierList>>, name: P<Node>, receiver: P<Node>) -> P<Node> {
        let _ = (node, modifiers, name, receiver);
        unimplemented!("classfields part 2")
    }
}

// classfields.go:3547
// Go returns an `iter.Seq`; every caller ranges over all of it, so the elements are collected.
pub(crate) fn flatten_comma_list(node: P<Node>) -> Vec<P<Node>> {
    let mut result = Vec::new();
    flatten_comma_list_worker(node, &mut |e| {
        result.push(e);
        true
    });
    result
}

// classfields.go:3553
fn flatten_comma_list_worker(node: P<Node>, yield_: &mut dyn FnMut(P<Node>) -> bool) -> bool {
    if ast::is_parenthesized_expression(node) && ast::node_is_synthesized(node) {
        flatten_comma_list_worker(node.expression().unwrap(), yield_)
    } else if ast::is_comma_expression(node) {
        flatten_comma_list_worker(node.as_binary_expression().left, yield_) && flatten_comma_list_worker(node.as_binary_expression().right(), yield_)
    } else {
        yield_(node)
    }
}

// classfields.go:3564
// Returns the BinaryExpression node (Go returns `*ast.BinaryExpression`).
pub(crate) fn find_computed_property_name_cache_assignment(emit_context: P<EmitContext>, name: P<Node>) -> Option<P<Node>> {
    let _ = emit_context;
    let mut node = name.expression().unwrap();
    loop {
        node = ast::skip_outer_expressions(node, OuterExpressionKinds::empty());
        if ast::is_binary_expression(node) && node.as_binary_expression().operator_token.kind() == Kind::CommaToken {
            node = node.as_binary_expression().right();
            continue;
        }
        if ast::is_assignment_expression(node, true /*excludeCompoundAssignment*/) && ast::is_identifier(node.as_binary_expression().left) {
            return Some(node);
        }
        break;
    }
    None
}

// classfields.go:3580
pub(crate) fn expand_pre_or_postfix_increment_or_decrement_expression(factory: &printer::NodeFactory, emit_context: P<EmitContext>, node: P<Node>, expression: P<Node>, result_variable: Option<P<Node>>) -> P<Node> {
    let operator: Kind;
    let operand: P<Node>;
    if ast::is_prefix_unary_expression(node) {
        operator = node.as_prefix_unary_expression().operator;
        operand = node.as_prefix_unary_expression().operand;
    } else {
        operator = node.as_postfix_unary_expression().operator;
        operand = node.as_postfix_unary_expression().operand;
    }

    let temp = factory.new_temp_variable();
    emit_context.add_variable_declaration(temp);
    let mut expression = factory.new_assignment_expression(temp, expression);
    expression.set_loc(operand.loc());

    let mut operation: P<Node>;
    if ast::is_prefix_unary_expression(node) {
        operation = factory.new_prefix_unary_expression(operator, temp);
    } else {
        operation = factory.new_postfix_unary_expression(temp, operator);
    }
    operation.set_loc(node.loc());

    if let Some(result_variable) = result_variable {
        operation = factory.new_assignment_expression(result_variable, operation);
        operation.set_loc(node.loc());
    }

    expression = factory.new_comma_expression(expression, operation);
    expression.set_loc(node.loc());

    if ast::is_postfix_unary_expression(node) {
        expression = factory.new_comma_expression(expression, temp);
        expression.set_loc(node.loc());
    }

    expression
}
