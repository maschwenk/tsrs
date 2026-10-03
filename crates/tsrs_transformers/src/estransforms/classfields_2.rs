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
        let _ = node;
        let env = self.get_private_identifier_environment();
        let identifier: P<Node>;
        if let Some(class_name) = env.data.class_name.get() {
            let prefix = format!("_{}_", class_name.text());
            identifier = self.factory().new_unique_name_ex(
                &format!("{}{}", prefix, name_text),
                printer::AutoGenerateOptions { flags: printer::GeneratedIdentifierFlags::Optimistic | printer::GeneratedIdentifierFlags::ReservedInNestedScopes, suffix: alloc_str(suffix), ..Default::default() },
            );
        } else {
            identifier = self.factory().new_unique_name_ex(
                &format!("_{}", name_text),
                printer::AutoGenerateOptions { flags: printer::GeneratedIdentifierFlags::Optimistic | printer::GeneratedIdentifierFlags::ReservedInNestedScopes, suffix: alloc_str(suffix), ..Default::default() },
            );
        }
        if self.requires_block_scoped_var() {
            self.emit_context().add_lexical_declaration(identifier);
        } else {
            self.emit_context().add_variable_declaration(identifier);
        }
        identifier
    }

    // classfields.go:3145
    pub(crate) fn create_hoisted_variable_for_class_from_node(&self, name: P<Node>, suffix: &str) -> P<Node> {
        let env = self.get_private_identifier_environment();
        let prefix: String;
        if let Some(class_name) = env.data.class_name.get() {
            prefix = format!("_{}_", class_name.text());
        } else {
            prefix = "_".to_string();
        }
        let identifier = self.factory().new_generated_name_for_node_ex(
            name,
            printer::AutoGenerateOptions { flags: printer::GeneratedIdentifierFlags::Optimistic | printer::GeneratedIdentifierFlags::ReservedInNestedScopes, prefix: alloc_str(&prefix), suffix: alloc_str(suffix) },
        );
        if self.requires_block_scoped_var() {
            self.emit_context().add_lexical_declaration(identifier);
        } else {
            self.emit_context().add_variable_declaration(identifier);
        }
        identifier
    }

    // classfields.go:3166
    pub(crate) fn create_hoisted_variable_for_private_name(&self, name: P<Node>, suffix: &str) -> P<Node> {
        // If the name is a generated identifier (e.g., auto-accessor backing field),
        // use node-based name generation so the emitter can resolve the name properly.
        if self.emit_context().has_auto_generate_info(Some(name)) {
            return self.create_hoisted_variable_for_class_from_node(name, suffix);
        }
        let mut text = name.text();
        if !text.is_empty() && text.as_bytes()[0] == b'#' {
            text = &text[1..]; // strip leading '#'
        }
        self.create_hoisted_variable_for_class(text, name, suffix)
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
        let prop = node.as_property_access_expression();
        let parameter = self.factory().new_generated_name_for_node(node);
        let info = self.access_private_identifier(prop.name());
        let Some(info) = info else {
            return self.visitor().visit_each_child(Some(node));
        };
        let mut receiver = prop.expression;
        // We cannot copy `this` or `super` into the function because they will be bound
        // differently inside the function.
        let is_this_or_super_property = prop.expression.kind() == Kind::ThisKeyword || prop.expression.kind() == Kind::SuperKeyword;
        if is_this_or_super_property || !is_simple_copiable_expression(prop.expression) {
            receiver = self.factory().new_temp_variable_ex(printer::AutoGenerateOptions { flags: printer::GeneratedIdentifierFlags::ReservedInNestedScopes, ..Default::default() });
            self.emit_context().add_variable_declaration(receiver);
            let assignment = self.factory().new_assignment_expression(receiver, self.visitor().visit_node(Some(prop.expression)).unwrap());
            self.pending_expressions.borrow_mut().push(assignment);
        }
        let assign_expr = self.create_private_identifier_assignment(info, receiver, parameter, Kind::EqualsToken);
        Some(self.factory().new_assignment_target_wrapper(parameter, assign_expr))
    }

    // classfields.go:3220
    pub(crate) fn visit_assignment_element(&self, mut node: P<Node>) -> Option<P<Node>> {
        // 13.15.5.5 RS: IteratorDestructuringAssignmentEvaluation
        //   AssignmentElement : DestructuringAssignmentTarget Initializer?
        //     ...
        //     4. If |Initializer| is present and _value_ is *undefined*, then
        //        a. If IsAnonymousFunctionDefinition(|Initializer|) and IsIdentifierRef of |DestructuringAssignmentTarget| are both *true*, then
        //           i. Let _v_ be ? NamedEvaluation of |Initializer| with argument _lref_.[[ReferencedName]].
        //     ...

        if is_named_evaluation_and(self.emit_context(), node, Some(&|n| self.is_anonymous_class_needing_assigned_name(n))) {
            node = transform_named_evaluation(self.emit_context(), node, false /*ignoreEmptyStringLiteral*/, "" /*assignedName*/);
        }
        if ast::is_assignment_expression(node, true /*excludeCompoundAssignment*/) {
            let b = node.as_binary_expression();
            let left = self.visit_destructuring_assignment_target(b.left).unwrap();
            let right = self.visitor().visit_node(Some(b.right())).unwrap();
            return Some(self.factory().update_binary_expression(node, None, left, None, b.operator_token, right));
        }
        self.visit_destructuring_assignment_target(node)
    }

    // classfields.go:3247
    pub(crate) fn visit_assignment_rest_element(&self, node: P<Node>) -> Option<P<Node>> {
        let spread = node.as_spread_element();
        if ast::is_left_hand_side_expression(spread.expression) {
            let expr = self.visit_destructuring_assignment_target(spread.expression).unwrap();
            return Some(self.factory().update_spread_element(node, expr));
        }
        self.visitor().visit_each_child(Some(node))
    }

    // classfields.go:3256
    pub(crate) fn visit_array_assignment_element(&self, node: P<Node>) -> Option<P<Node>> {
        if ast::is_array_binding_or_assignment_element(node) {
            if ast::is_spread_element(node) {
                return self.visit_assignment_rest_element(node);
            }
            if node.kind() != Kind::OmittedExpression {
                return self.visit_assignment_element(node);
            }
        }
        self.visitor().visit_each_child(Some(node))
    }

    // classfields.go:3268
    pub(crate) fn visit_assignment_property(&self, node: P<Node>) -> Option<P<Node>> {
        // AssignmentProperty : PropertyName `:` AssignmentElement
        // AssignmentElement : DestructuringAssignmentTarget Initializer?

        // 13.15.5.6 RS: KeyedDestructuringAssignmentEvaluation
        //   AssignmentElement : DestructuringAssignmentTarget Initializer?
        //     ...
        //     3. If |Initializer| is present and _v_ is *undefined*, then
        //        a. If IsAnonymousfunctionDefinition(|Initializer|) and IsIdentifierRef of |DestructuringAssignmentTarget| are both *true*, then
        //           i. Let _rhsValue_ be ? NamedEvaluation of |Initializer| with argument _lref_.[[ReferencedName]].
        //     ...

        let prop = node.as_property_assignment();
        let name = self.visitor().visit_node(node.name()).unwrap();
        let init = prop.initializer.get();
        if ast::is_assignment_expression(init, true /*excludeCompoundAssignment*/) {
            let assign_elem = self.visit_assignment_element(init).unwrap();
            return Some(self.factory().update_property_assignment(node, None, name, None, None, assign_elem));
        }
        if ast::is_left_hand_side_expression(init) {
            let target = self.visit_destructuring_assignment_target(init).unwrap();
            return Some(self.factory().update_property_assignment(node, None, name, None, None, target));
        }
        self.visitor().visit_each_child(Some(node))
    }

    // classfields.go:3294
    pub(crate) fn visit_shorthand_assignment_property(&self, mut node: P<Node>) -> Option<P<Node>> {
        // AssignmentProperty : IdentifierReference Initializer?

        // 13.15.5.3 RS: PropertyDestructuringAssignmentEvaluation
        //   AssignmentProperty : IdentifierReference Initializer?
        //     ...
        //     4. If |Initializer?| is present and _v_ is *undefined*, then
        //        a. If IsAnonymousFunctionDefinition(|Initializer|) is *true*, then
        //           i. Set _v_ to ? NamedEvaluation of |Initializer| with argument _P_.
        //     ...

        if is_named_evaluation_and(self.emit_context(), node, Some(&|n| self.is_anonymous_class_needing_assigned_name(n))) {
            node = transform_named_evaluation(self.emit_context(), node, false /*ignoreEmptyStringLiteral*/, "" /*assignedName*/);
        }
        self.visitor().visit_each_child(Some(node))
    }

    // classfields.go:3311
    pub(crate) fn visit_assignment_rest_property(&self, node: P<Node>) -> Option<P<Node>> {
        let spread = node.as_spread_assignment();
        if ast::is_left_hand_side_expression(spread.expression) {
            let expr = self.visit_destructuring_assignment_target(spread.expression).unwrap();
            return Some(self.factory().update_spread_assignment(node, expr));
        }
        self.visitor().visit_each_child(Some(node))
    }

    // classfields.go:3320
    pub(crate) fn visit_object_assignment_element(&self, node: P<Node>) -> Option<P<Node>> {
        assert!(ast::is_object_binding_or_assignment_element(node));
        if ast::is_spread_assignment(node) {
            return self.visit_assignment_rest_property(node);
        }
        if ast::is_shorthand_property_assignment(node) {
            return self.visit_shorthand_assignment_property(node);
        }
        if ast::is_property_assignment(node) {
            return self.visit_assignment_property(node);
        }
        self.visitor().visit_each_child(Some(node))
    }

    // classfields.go:3334
    pub(crate) fn visit_assignment_pattern(&self, node: P<Node>) -> Option<P<Node>> {
        if ast::is_array_literal_expression(node) {
            // Transforms private names in destructuring assignment array bindings.
            // Transforms SuperProperty assignments in destructuring assignment array bindings in static initializers.
            //
            // Source:
            // ([ this.#myProp ] = [ "hello" ]);
            //
            // Transformation:
            // [ { set value(x) { this.#myProp = x; } }.value ] = [ "hello" ];
            let arr = node.as_array_literal_expression();
            return Some(self.factory().update_array_literal_expression(node, self.array_assignment_element_visitor().visit_nodes(Some(arr.elements)).unwrap(), arr.multi_line));
        }
        // Transforms private names in destructuring assignment object bindings.
        // Transforms SuperProperty assignments in destructuring assignment object bindings in static initializers.
        //
        // Source:
        // ({ stringProperty: this.#myProp } = { stringProperty: "hello" });
        //
        // Transformation:
        // ({ stringProperty: { set value(x) { this.#myProp = x; } }.value }) = { stringProperty: "hello" };
        let obj = node.as_object_literal_expression();
        Some(self.factory().update_object_literal_expression(node, self.object_assignment_element_visitor().visit_nodes(Some(obj.properties)).unwrap(), obj.multi_line))
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
