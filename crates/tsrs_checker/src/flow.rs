use crate::*;
use tsrs_ast::*;
use tsrs_core::*;
use tsrs_ast as ast;
use tsrs_diagnostics as diagnostics;

// Non-function declarations of flow.go (FlowType, SharedFlow, FlowState, typeofNEFacts,
// nonDottedNameCacheKey) are in flow_types.rs.

impl FlowType {
    // flow.go:24
    pub(crate) fn is_nil(&self) -> bool {
        self.t.is_none()
    }
}

impl FlowState {
    fn ref_node(&self) -> P<Node> {
        self.reference.get().unwrap()
    }

    fn declared(&self) -> P<Type> {
        self.declared_type.get().unwrap()
    }

    fn initial(&self) -> P<Type> {
        self.initial_type.get().unwrap()
    }
}

fn flow_type_of(t: P<Type>) -> FlowType {
    FlowType { t: Some(t), incomplete: false }
}

impl Checker {
    // flow.go:28
    pub(crate) fn new_flow_type(&mut self, t: P<Type>, incomplete: bool) -> FlowType {
        let mut t = t;
        if incomplete && t.flags().intersects(TypeFlags::Never) {
            t = self.silent_never_type;
        }
        FlowType { t: Some(t), incomplete }
    }

    // flow.go:52
    pub(crate) fn get_flow_state(&mut self) -> P<FlowState> {
        let f = match self.free_flow_state {
            Some(f) => f,
            None => P::new(FlowState::default()),
        };
        self.free_flow_state = f.next.get();
        f
    }

    // flow.go:61
    pub(crate) fn put_flow_state(&mut self, f: P<FlowState>) {
        f.reference.set(None);
        f.declared_type.set(None);
        f.initial_type.set(None);
        f.flow_container.set(None);
        f.ref_key.set(CacheHashKey::default());
        f.depth.set(0);
        f.shared_flow_start.set(0);
        f.reduce_labels.borrow_mut().clear();
        f.next.set(self.free_flow_state);
        self.free_flow_state = Some(f);
    }
}

// flow.go:69
pub(crate) fn get_flow_node_of_node(node: P<Node>) -> Option<P<FlowNode>> {
    let flow_node_data = node.flow_node_data();
    if let Some(flow_node_data) = flow_node_data {
        return flow_node_data.flow_node.get();
    }
    None
}

impl Checker {
    // flow.go:77
    pub(crate) fn get_flow_type_of_reference(&mut self, reference: P<Node>, declared_type: P<Type>) -> P<Type> {
        self.get_flow_type_of_reference_ex(reference, declared_type, declared_type, None, None)
    }

    // flow.go:81
    pub(crate) fn get_flow_type_of_reference_ex(&mut self, reference: P<Node>, declared_type: P<Type>, initial_type: P<Type>, flow_container: Option<P<Node>>, flow_node: Option<P<FlowNode>>) -> P<Type> {
        if self.flow_analysis_disabled {
            return self.error_type;
        }
        let flow_node = match flow_node {
            Some(flow_node) => flow_node,
            None => match get_flow_node_of_node(reference) {
                Some(flow_node) => flow_node,
                None => return declared_type,
            },
        };
        let f = self.get_flow_state();
        f.reference.set(Some(reference));
        f.declared_type.set(Some(declared_type));
        f.initial_type.set(Some(initial_type));
        f.flow_container.set(flow_container);
        f.shared_flow_start.set(self.shared_flows.len() as i32);
        self.flow_invocation_count += 1;
        let evolved_type = self.get_type_at_flow_node(f, flow_node).t.unwrap();
        self.shared_flows.truncate(f.shared_flow_start.get() as usize);
        self.put_flow_state(f);
        // When the reference is 'x' in an 'x.length', 'x.push(value)', 'x.unshift(value)' or x[n] = value' operation,
        // we give type 'any[]' to 'x' instead of using the type determined by control flow analysis such that operations
        // on empty arrays are possible without implicit any errors and new element types can be inferred without
        // type mismatch errors.
        let result_type = if evolved_type.object_flags().intersects(ObjectFlags::EvolvingArray) && self.is_evolving_array_operation_target(reference) {
            self.auto_array_type
        } else {
            self.finalize_evolving_array_type(evolved_type)
        };
        if result_type == self.unreachable_never_type
            || reference.parent().is_some()
                && ast::is_non_null_expression(reference.parent().unwrap())
                && !result_type.flags().intersects(TypeFlags::Never)
                && self.get_type_with_facts(result_type, TypeFacts::NEUndefinedOrNull).flags().intersects(TypeFlags::Never)
        {
            return declared_type;
        }
        result_type
    }

    // flow.go:117
    pub(crate) fn get_type_at_flow_node(&mut self, f: P<FlowState>, flow: P<FlowNode>) -> FlowType {
        let mut flow = flow;
        if f.depth.get() == 2000 {
            // We have made 2000 recursive invocations. To avoid overflowing the call stack we report an error
            // and disable further control flow analysis in the containing function or module body.
            self.flow_analysis_disabled = true;
            self.report_flow_control_error(f.ref_node());
            return flow_type_of(self.error_type);
        }
        f.depth.set(f.depth.get() + 1);
        let mut shared_flow: Option<P<FlowNode>> = None;
        loop {
            let flags = flow.flags();
            if flags.intersects(FlowFlags::Shared) {
                // We cache results of flow type resolution for shared nodes that were previously visited in
                // the same getFlowTypeOfReference invocation. A node is considered shared when it is the
                // antecedent of more than one node.
                for i in f.shared_flow_start.get() as usize..self.shared_flows.len() {
                    if self.shared_flows[i].flow == flow {
                        f.depth.set(f.depth.get() - 1);
                        return self.shared_flows[i].flow_type;
                    }
                }
                shared_flow = Some(flow);
            }
            let t: FlowType;
            if flags.intersects(FlowFlags::Assignment) {
                t = self.get_type_at_flow_assignment(f, flow);
                if t.is_nil() {
                    flow = flow.antecedent().unwrap();
                    continue;
                }
            } else if flags.intersects(FlowFlags::Call) {
                t = self.get_type_at_flow_call(f, flow);
                if t.is_nil() {
                    flow = flow.antecedent().unwrap();
                    continue;
                }
            } else if flags.intersects(FlowFlags::Condition) {
                t = self.get_type_at_flow_condition(f, flow);
            } else if flags.intersects(FlowFlags::SwitchClause) {
                t = self.get_type_at_switch_clause(f, flow);
            } else if flags.intersects(FlowFlags::BranchLabel) {
                let antecedents = get_branch_label_antecedents(flow, &f.reduce_labels.borrow()).unwrap();
                if antecedents.next.get().is_none() {
                    flow = antecedents.flow;
                    continue;
                }
                t = self.get_type_at_flow_branch_label(f, flow, antecedents);
            } else if flags.intersects(FlowFlags::LoopLabel) {
                let antecedents = flow.antecedents().unwrap();
                if antecedents.next.get().is_none() {
                    flow = antecedents.flow;
                    continue;
                }
                t = self.get_type_at_flow_loop_label(f, flow);
            } else if flags.intersects(FlowFlags::ArrayMutation) {
                t = self.get_type_at_flow_array_mutation(f, flow);
                if t.is_nil() {
                    flow = flow.antecedent().unwrap();
                    continue;
                }
            } else if flags.intersects(FlowFlags::ReduceLabel) {
                f.reduce_labels.borrow_mut().push(P::from_static(flow.node().unwrap().as_flow_reduce_label_data()));
                t = self.get_type_at_flow_node(f, flow.antecedent().unwrap());
                f.reduce_labels.borrow_mut().pop();
            } else if flags.intersects(FlowFlags::Start) {
                // Check if we should continue with the control flow of the containing function.
                let container = flow.node();
                if let Some(container) = container {
                    let reference = f.ref_node();
                    if Some(container) != f.flow_container.get()
                        && !ast::is_property_access_expression(reference)
                        && !ast::is_element_access_expression(reference)
                        && !(reference.kind == Kind::ThisKeyword && !ast::is_arrow_function(container))
                    {
                        flow = container.flow_node_data().unwrap().flow_node.get().unwrap();
                        continue;
                    }
                }
                // At the top of the flow we have the initial type.
                t = flow_type_of(f.initial());
            } else {
                // Unreachable code errors are reported in the binding phase. Here we
                // simply return the non-auto declared type to reduce follow-on errors.
                t = flow_type_of(self.convert_auto_to_any(f.declared()));
            }
            if let Some(shared_flow) = shared_flow {
                // Record visited node and the associated type in the cache.
                self.shared_flows.push(SharedFlow { flow: shared_flow, flow_type: t });
            }
            f.depth.set(f.depth.get() - 1);
            return t;
        }
    }
}

// flow.go:208
pub(crate) fn get_branch_label_antecedents(flow: P<FlowNode>, reduce_labels: &[P<ast::FlowReduceLabelData>]) -> Option<P<FlowList>> {
    let mut i = reduce_labels.len();
    while i != 0 {
        i -= 1;
        let data = reduce_labels[i];
        if data.target == flow {
            return data.antecedents;
        }
    }
    flow.antecedents()
}

impl Checker {
    // flow.go:220
    pub(crate) fn get_type_at_flow_assignment(&mut self, f: P<FlowState>, flow: P<FlowNode>) -> FlowType {
        let node = flow.node().unwrap();
        // Assignments only narrow the computed type if the declared type is a union type. Thus, we
        // only need to evaluate the assigned type if the declared type is a union type.
        if self.is_matching_reference(f.ref_node(), node) {
            if !self.is_reachable_flow_node(flow) {
                return flow_type_of(self.unreachable_never_type);
            }
            if get_assignment_target_kind(node) == AssignmentKind::Compound {
                let flow_type = self.get_type_at_flow_node(f, flow.antecedent().unwrap());
                let t = self.get_base_type_of_literal_type(flow_type.t.unwrap());
                return self.new_flow_type(t, flow_type.incomplete);
            }
            if f.declared() == self.auto_type || f.declared() == self.auto_array_type {
                if self.is_empty_array_assignment(node) {
                    let never_type = self.never_type;
                    return flow_type_of(self.get_evolving_array_type(never_type));
                }
                let initial_or_assigned = self.get_initial_or_assigned_type(f, flow);
                let assigned_type = self.get_widened_literal_type(initial_or_assigned);
                if self.is_type_assignable_to(assigned_type, f.declared()) {
                    return flow_type_of(assigned_type);
                }
                return flow_type_of(self.any_array_type);
            }
            let mut t = f.declared();
            if is_in_compound_like_assignment(node) {
                t = self.get_base_type_of_literal_type(t);
            }
            if t.flags().intersects(TypeFlags::Union) {
                let initial_or_assigned = self.get_initial_or_assigned_type(f, flow);
                return flow_type_of(self.get_assignment_reduced_type(t, initial_or_assigned));
            }
            return flow_type_of(t);
        }
        // We didn't have a direct match. However, if the reference is a dotted name, this
        // may be an assignment to a left hand part of the reference. For example, for a
        // reference 'x.y.z', we may be at an assignment to 'x.y' or 'x'. In that case,
        // return the declared type.
        if self.contains_matching_reference(f.ref_node(), node) {
            if !self.is_reachable_flow_node(flow) {
                return flow_type_of(self.unreachable_never_type);
            }
            // A matching dotted name might also be an expando property on a function *expression*,
            // in which case we continue control flow analysis back to the function's declaration
            if ast::is_variable_declaration(node) && (ast::is_in_js_file(node) || ast::is_var_const_like(node)) {
                if let Some(init) = node.initializer() {
                    if ast::is_function_expression_or_arrow_function(init) {
                        return self.get_type_at_flow_node(f, flow.antecedent().unwrap());
                    }
                }
            }
            return flow_type_of(f.declared());
        }
        // for (const _ in ref) acts as a nonnull on ref
        if ast::is_variable_declaration(node) && ast::is_for_in_statement(node.parent().unwrap().parent().unwrap()) {
            let for_in_expression = node.parent().unwrap().parent().unwrap().expression().unwrap();
            if self.is_matching_reference(f.ref_node(), for_in_expression) || self.optional_chain_contains_reference(for_in_expression, f.ref_node()) {
                let antecedent_type = self.get_type_at_flow_node(f, flow.antecedent().unwrap()).t.unwrap();
                let finalized = self.finalize_evolving_array_type(antecedent_type);
                return flow_type_of(self.get_non_nullable_type_if_needed(finalized));
            }
        }
        // Assignment doesn't affect reference
        FlowType::default()
    }

    // flow.go:276
    pub(crate) fn get_initial_or_assigned_type(&mut self, f: P<FlowState>, flow: P<FlowNode>) -> P<Type> {
        let node = flow.node().unwrap();
        if ast::is_variable_declaration(node) || ast::is_binding_element(node) {
            let initial_type = self.get_initial_type(node);
            return self.get_narrowable_type_for_reference(initial_type, f.ref_node(), CheckMode::Normal);
        }
        let assigned_type = self.get_assigned_type(node);
        self.get_narrowable_type_for_reference(assigned_type, f.ref_node(), CheckMode::Normal)
    }

    // flow.go:283
    pub(crate) fn is_empty_array_assignment(&mut self, node: P<Node>) -> bool {
        ast::is_variable_declaration(node) && node.initializer().is_some() && is_empty_array_literal(node.initializer().unwrap())
            || !ast::is_binding_element(node) && ast::is_binary_expression(node.parent().unwrap()) && is_empty_array_literal(node.parent().unwrap().as_binary_expression().right())
    }

    // flow.go:288
    pub(crate) fn get_type_at_flow_call(&mut self, f: P<FlowState>, flow: P<FlowNode>) -> FlowType {
        let node = flow.node().unwrap();
        let signature = self.get_effects_signature(node);
        if let Some(signature) = signature {
            let predicate = self.get_type_predicate_of_signature(signature);
            if let Some(predicate) = predicate {
                if predicate.kind.get() == TypePredicateKind::AssertsThis || predicate.kind.get() == TypePredicateKind::AssertsIdentifier {
                    let flow_type = self.get_type_at_flow_node(f, flow.antecedent().unwrap());
                    let t = self.finalize_evolving_array_type(flow_type.t.unwrap());
                    let parameter_index = predicate.parameter_index.get();
                    let narrowed_type = if predicate.t.get().is_some() {
                        self.narrow_type_by_type_predicate(f, t, predicate, node, true /*assumeTrue*/)
                    } else if predicate.kind.get() == TypePredicateKind::AssertsIdentifier && parameter_index >= 0 && (parameter_index as usize) < node.arguments().len() {
                        self.narrow_type_by_assertion(f, t, node.arguments()[parameter_index as usize])
                    } else {
                        t
                    };
                    if narrowed_type == t {
                        return flow_type;
                    }
                    return self.new_flow_type(narrowed_type, flow_type.incomplete);
                }
            }
            if self.get_return_type_of_signature(signature).flags().intersects(TypeFlags::Never) {
                return flow_type_of(self.unreachable_never_type);
            }
        }
        FlowType::default()
    }

    // flow.go:316
    pub(crate) fn narrow_type_by_type_predicate(&mut self, f: P<FlowState>, t: P<Type>, predicate: P<TypePredicate>, call_expression: P<Node>, assume_true: bool) -> P<Type> {
        let mut t = t;
        // Don't narrow from 'any' if the predicate type is exactly 'Object' or 'Function'
        if let Some(predicate_type) = predicate.t.get() {
            if !(is_type_any(Some(t)) && (predicate_type == self.global_object_type || predicate_type == self.global_function_type)) {
                let predicate_argument = self.get_type_predicate_argument(predicate, call_expression);
                if let Some(predicate_argument) = predicate_argument {
                    if self.is_matching_reference(f.ref_node(), predicate_argument) {
                        return self.get_narrowed_type(t, predicate_type, assume_true, false /*checkDerived*/);
                    }
                    if self.strict_null_checks
                        && self.optional_chain_contains_reference(predicate_argument, f.ref_node())
                        && (assume_true && !self.has_type_facts(predicate_type, TypeFacts::EQUndefined) || !assume_true && every_type(predicate_type, |t| self.is_nullable_type(t)))
                    {
                        t = self.get_adjusted_type_with_facts(t, TypeFacts::NEUndefinedOrNull);
                    }
                    let access = self.get_discriminant_property_access(f, predicate_argument, t);
                    if let Some(access) = access {
                        return self.narrow_type_by_discriminant(t, access, move |c, t| c.get_narrowed_type(t, predicate_type, assume_true, false /*checkDerived*/));
                    }
                }
            }
        }
        t
    }

    // flow.go:338
    pub(crate) fn narrow_type_by_assertion(&mut self, f: P<FlowState>, t: P<Type>, expr: P<Node>) -> P<Type> {
        let node = ast::skip_parentheses(expr);
        if node.kind == Kind::FalseKeyword {
            return self.unreachable_never_type;
        }
        if node.kind == Kind::BinaryExpression {
            let binary = node.as_binary_expression();
            if binary.operator_token.kind == Kind::AmpersandAmpersandToken {
                let left_type = self.narrow_type_by_assertion(f, t, binary.left);
                return self.narrow_type_by_assertion(f, left_type, binary.right());
            }
            if binary.operator_token.kind == Kind::BarBarToken {
                let left_type = self.narrow_type_by_assertion(f, t, binary.left);
                let right_type = self.narrow_type_by_assertion(f, t, binary.right());
                return self.get_union_type(&[left_type, right_type]);
            }
        }
        self.narrow_type(f, t, node, true /*assumeTrue*/)
    }

    // flow.go:354
    pub(crate) fn get_type_at_flow_condition(&mut self, f: P<FlowState>, flow: P<FlowNode>) -> FlowType {
        let flow_type = self.get_type_at_flow_node(f, flow.antecedent().unwrap());
        if flow_type.t.unwrap().flags().intersects(TypeFlags::Never) {
            return flow_type;
        }
        // If we have an antecedent type (meaning we're reachable in some way), we first
        // attempt to narrow the antecedent type. If that produces the never type, and if
        // the antecedent type is incomplete (i.e. a transient type in a loop), then we
        // take the type guard as an indication that control *could* reach here once we
        // have the complete type. We proceed by switching to the silent never type which
        // doesn't report errors when operators are applied to it. Note that this is the
        // *only* place a silent never type is ever generated.
        let assume_true = flow.flags().intersects(FlowFlags::TrueCondition);
        let non_evolving_type = self.finalize_evolving_array_type(flow_type.t.unwrap());
        let narrowed_type = self.narrow_type(f, non_evolving_type, flow.node().unwrap(), assume_true);
        if narrowed_type == non_evolving_type {
            return flow_type;
        }
        self.new_flow_type(narrowed_type, flow_type.incomplete)
    }

    // Narrow the given type based on the given expression having the assumed boolean value. The returned type
    // will be a subtype or the same type as the argument.
    // flow.go:377
    pub(crate) fn narrow_type(&mut self, f: P<FlowState>, t: P<Type>, expr: P<Node>, assume_true: bool) -> P<Type> {
        // for `a?.b`, we emulate a synthetic `a !== null && a !== undefined` condition for `a`
        let parent = expr.parent().unwrap();
        if ast::is_expression_of_optional_chain_root(expr)
            || ast::is_binary_expression(parent)
                && (parent.as_binary_expression().operator_token.kind == Kind::QuestionQuestionToken || parent.as_binary_expression().operator_token.kind == Kind::QuestionQuestionEqualsToken)
                && parent.as_binary_expression().left == expr
        {
            return self.narrow_type_by_optionality(f, t, expr, assume_true);
        }
        match expr.kind {
            Kind::Identifier | Kind::ThisKeyword | Kind::SuperKeyword | Kind::PropertyAccessExpression | Kind::ElementAccessExpression => {
                if expr.kind == Kind::Identifier {
                    // When narrowing a reference to a const variable, non-assigned parameter, or readonly property, we inline
                    // up to five levels of aliased conditional expressions that are themselves declared as const variables.
                    if !self.is_matching_reference(f.ref_node(), expr) && self.inline_level < 5 {
                        let symbol = self.get_resolved_symbol(expr);
                        if self.is_constant_variable(symbol) {
                            let declaration = symbol.value_declaration();
                            if let Some(declaration) = declaration {
                                if ast::is_variable_declaration(declaration) && declaration.type_node().is_none() && declaration.initializer().is_some() && self.is_constant_reference(f.ref_node()) {
                                    self.inline_level += 1;
                                    let result = self.narrow_type(f, t, declaration.initializer().unwrap(), assume_true);
                                    self.inline_level -= 1;
                                    return result;
                                }
                            }
                        }
                    }
                }
                return self.narrow_type_by_truthiness(f, t, expr, assume_true);
            }
            Kind::CallExpression => {
                return self.narrow_type_by_call_expression(f, t, expr, assume_true);
            }
            Kind::ParenthesizedExpression | Kind::NonNullExpression | Kind::SatisfiesExpression => {
                return self.narrow_type(f, t, expr.expression().unwrap(), assume_true);
            }
            Kind::BinaryExpression => {
                return self.narrow_type_by_binary_expression(f, t, expr, assume_true);
            }
            Kind::PrefixUnaryExpression => {
                if expr.as_prefix_unary_expression().operator == Kind::ExclamationToken {
                    return self.narrow_type(f, t, expr.as_prefix_unary_expression().operand, !assume_true);
                }
            }
            _ => {}
        }
        t
    }

    // flow.go:415
    pub(crate) fn narrow_type_by_optionality(&mut self, f: P<FlowState>, t: P<Type>, expr: P<Node>, assume_present: bool) -> P<Type> {
        if self.is_matching_reference(f.ref_node(), expr) {
            return self.get_adjusted_type_with_facts(t, if assume_present { TypeFacts::NEUndefinedOrNull } else { TypeFacts::EQUndefinedOrNull });
        }
        let access = self.get_discriminant_property_access(f, expr, t);
        if let Some(access) = access {
            return self.narrow_type_by_discriminant(t, access, move |c, t| c.get_type_with_facts(t, if assume_present { TypeFacts::NEUndefinedOrNull } else { TypeFacts::EQUndefinedOrNull }));
        }
        t
    }

    // flow.go:428
    pub(crate) fn narrow_type_by_truthiness(&mut self, f: P<FlowState>, t: P<Type>, expr: P<Node>, assume_true: bool) -> P<Type> {
        let mut t = t;
        if self.is_matching_reference(f.ref_node(), expr) {
            return self.get_adjusted_type_with_facts(t, if assume_true { TypeFacts::Truthy } else { TypeFacts::Falsy });
        }
        if self.strict_null_checks && assume_true && self.optional_chain_contains_reference(expr, f.ref_node()) {
            t = self.get_adjusted_type_with_facts(t, TypeFacts::NEUndefinedOrNull);
        }
        let access = self.get_discriminant_property_access(f, expr, t);
        if let Some(access) = access {
            return self.narrow_type_by_discriminant(t, access, move |c, t| c.get_type_with_facts(t, if assume_true { TypeFacts::Truthy } else { TypeFacts::Falsy }));
        }
        t
    }

    // flow.go:444
    pub(crate) fn narrow_type_by_call_expression(&mut self, f: P<FlowState>, t: P<Type>, call_expression: P<Node>, assume_true: bool) -> P<Type> {
        if self.has_matching_argument(call_expression, f.ref_node()) {
            let mut predicate: Option<P<TypePredicate>> = None;
            if assume_true || !is_call_chain(call_expression) {
                let signature = self.get_effects_signature(call_expression);
                if let Some(signature) = signature {
                    predicate = self.get_type_predicate_of_signature(signature);
                }
            }
            if let Some(predicate) = predicate {
                if predicate.kind.get() == TypePredicateKind::This || predicate.kind.get() == TypePredicateKind::Identifier {
                    return self.narrow_type_by_type_predicate(f, t, predicate, call_expression, assume_true);
                }
            }
        }
        let reference = f.ref_node();
        if self.contains_missing_type(t) && ast::is_access_expression(reference) && ast::is_property_access_expression(call_expression.expression().unwrap()) {
            let call_access = call_expression.expression().unwrap();
            let candidate = self.get_reference_candidate(call_access.expression().unwrap());
            if self.is_matching_reference(reference.expression().unwrap(), candidate)
                && ast::is_identifier(call_access.name().unwrap())
                && call_access.name().unwrap().text() == "hasOwnProperty"
                && call_expression.arguments().len() == 1
            {
                let argument = call_expression.arguments()[0];
                let (accessed_name, ok) = self.get_accessed_property_name(reference);
                if ok && ast::is_string_literal_like(argument) && accessed_name == argument.text() {
                    return self.get_type_with_facts(t, if assume_true { TypeFacts::NEUndefined } else { TypeFacts::EQUndefined });
                }
            }
        }
        t
    }

    // flow.go:469
    pub(crate) fn narrow_type_by_binary_expression(&mut self, f: P<FlowState>, t: P<Type>, expr: P<Node>, assume_true: bool) -> P<Type> {
        let mut t = t;
        let binary = expr.as_binary_expression();
        match binary.operator_token.kind {
            Kind::EqualsToken | Kind::BarBarEqualsToken | Kind::AmpersandAmpersandEqualsToken | Kind::QuestionQuestionEqualsToken => {
                let narrowed = self.narrow_type(f, t, binary.right(), assume_true);
                return self.narrow_type_by_truthiness(f, narrowed, binary.left, assume_true);
            }
            Kind::EqualsEqualsToken | Kind::ExclamationEqualsToken | Kind::EqualsEqualsEqualsToken | Kind::ExclamationEqualsEqualsToken => {
                let operator = binary.operator_token.kind;
                let left = self.get_reference_candidate(binary.left);
                let right = self.get_reference_candidate(binary.right());
                if left.kind == Kind::TypeOfExpression && ast::is_string_literal_like(right) {
                    return self.narrow_type_by_typeof(f, t, left, operator, right, assume_true);
                }
                if right.kind == Kind::TypeOfExpression && ast::is_string_literal_like(left) {
                    return self.narrow_type_by_typeof(f, t, right, operator, left, assume_true);
                }
                if self.is_matching_reference(f.ref_node(), left) {
                    return self.narrow_type_by_equality(t, operator, right, assume_true);
                }
                if self.is_matching_reference(f.ref_node(), right) {
                    return self.narrow_type_by_equality(t, operator, left, assume_true);
                }
                if self.strict_null_checks {
                    if self.optional_chain_contains_reference(left, f.ref_node()) {
                        t = self.narrow_type_by_optional_chain_containment(f, t, operator, right, assume_true);
                    } else if self.optional_chain_contains_reference(right, f.ref_node()) {
                        t = self.narrow_type_by_optional_chain_containment(f, t, operator, left, assume_true);
                    }
                }
                let left_access = self.get_discriminant_property_access(f, left, t);
                if let Some(left_access) = left_access {
                    return self.narrow_type_by_discriminant_property(t, left_access, operator, right, assume_true);
                }
                let right_access = self.get_discriminant_property_access(f, right, t);
                if let Some(right_access) = right_access {
                    return self.narrow_type_by_discriminant_property(t, right_access, operator, left, assume_true);
                }
                if self.is_matching_constructor_reference(f, left) {
                    return self.narrow_type_by_constructor(t, operator, right, assume_true);
                }
                if self.is_matching_constructor_reference(f, right) {
                    return self.narrow_type_by_constructor(t, operator, left, assume_true);
                }
                if ast::is_boolean_literal(right) && !ast::is_access_expression(left) {
                    return self.narrow_type_by_boolean_comparison(f, t, left, right, operator, assume_true);
                }
                if ast::is_boolean_literal(left) && !ast::is_access_expression(right) {
                    return self.narrow_type_by_boolean_comparison(f, t, right, left, operator, assume_true);
                }
            }
            Kind::InstanceOfKeyword => {
                return self.narrow_type_by_instanceof(f, t, expr, assume_true);
            }
            Kind::InKeyword => {
                if ast::is_private_identifier(binary.left) {
                    return self.narrow_type_by_private_identifier_in_in_expression(f, t, expr, assume_true);
                }
                let target = self.get_reference_candidate(binary.right());
                let reference = f.ref_node();
                if self.contains_missing_type(t) && ast::is_access_expression(reference) && self.is_matching_reference(reference.expression().unwrap(), target) {
                    let left_type = self.get_type_of_expression(binary.left);
                    if is_type_usable_as_property_name(left_type) {
                        let (accessed_name, ok) = self.get_accessed_property_name(reference);
                        if ok && accessed_name == get_property_name_from_type(left_type) {
                            return self.get_type_with_facts(t, if assume_true { TypeFacts::NEUndefined } else { TypeFacts::EQUndefined });
                        }
                    }
                }
                if self.is_matching_reference(f.ref_node(), target) {
                    let left_type = self.get_type_of_expression(binary.left);
                    if is_type_usable_as_property_name(left_type) {
                        return self.narrow_type_by_in_keyword(f, t, left_type, assume_true);
                    }
                }
            }
            Kind::CommaToken => {
                return self.narrow_type(f, t, binary.right(), assume_true);
            }
            Kind::AmpersandAmpersandToken => {
                // Ordinarily we won't see && and || expressions in control flow analysis because the Binder breaks those
                // expressions down to individual conditional control flows. However, we may encounter them when analyzing
                // aliased conditional expressions.
                if assume_true {
                    let left_type = self.narrow_type(f, t, binary.left, true /*assumeTrue*/);
                    return self.narrow_type(f, left_type, binary.right(), true /*assumeTrue*/);
                }
                let left_type = self.narrow_type(f, t, binary.left, false /*assumeTrue*/);
                let right_type = self.narrow_type(f, t, binary.right(), false /*assumeTrue*/);
                return self.get_union_type(&[left_type, right_type]);
            }
            Kind::BarBarToken => {
                if assume_true {
                    let left_type = self.narrow_type(f, t, binary.left, true /*assumeTrue*/);
                    let right_type = self.narrow_type(f, t, binary.right(), true /*assumeTrue*/);
                    return self.get_union_type(&[left_type, right_type]);
                }
                let left_type = self.narrow_type(f, t, binary.left, false /*assumeTrue*/);
                return self.narrow_type(f, left_type, binary.right(), false /*assumeTrue*/);
            }
            _ => {}
        }
        t
    }

    // flow.go:556
    pub(crate) fn narrow_type_by_equality(&mut self, t: P<Type>, operator: Kind, value: P<Node>, assume_true: bool) -> P<Type> {
        let mut assume_true = assume_true;
        if t.flags().intersects(TypeFlags::Any) {
            return t;
        }
        if operator == Kind::ExclamationEqualsToken || operator == Kind::ExclamationEqualsEqualsToken {
            assume_true = !assume_true;
        }
        let value_type = self.get_type_of_expression(value);
        let double_equals = operator == Kind::EqualsEqualsToken || operator == Kind::ExclamationEqualsToken;
        if value_type.flags().intersects(TypeFlags::Nullable) {
            if !self.strict_null_checks {
                return t;
            }
            let facts = if double_equals {
                if assume_true { TypeFacts::EQUndefinedOrNull } else { TypeFacts::NEUndefinedOrNull }
            } else if value_type.flags().intersects(TypeFlags::Null) {
                if assume_true { TypeFacts::EQNull } else { TypeFacts::NENull }
            } else if assume_true {
                TypeFacts::EQUndefined
            } else {
                TypeFacts::NEUndefined
            };
            return self.get_adjusted_type_with_facts(t, facts);
        }
        if assume_true {
            if !double_equals && (t.flags().intersects(TypeFlags::Unknown) || some_type(t, |t| self.is_empty_anonymous_object_type(t))) {
                if value_type.flags().intersects(TypeFlags::Primitive | TypeFlags::NonPrimitive) || self.is_empty_anonymous_object_type(value_type) {
                    return value_type;
                }
                if value_type.flags().intersects(TypeFlags::Object) {
                    return self.non_primitive_type;
                }
            }
            if !double_equals && value_type.flags().intersects(TypeFlags::Primitive) && self.is_uniform_union_type(t) {
                let regular_type = self.get_regular_type_of_literal_type(value_type);
                if self.union_contains_type(t, regular_type, false /*matchSymbol*/) {
                    return regular_type;
                }
            }
            let filtered_type = self.filter_type(t, move |c, t| c.are_types_comparable(t, value_type) || double_equals && is_coercible_under_double_equals(t, value_type));
            return self.replace_primitives_with_literals(filtered_type, value_type);
        }
        if is_unit_type(value_type) {
            if self.is_uniform_union_type(t) {
                let regular_type = self.get_regular_type_of_literal_type(value_type);
                let filtered_type = self.remove_type(t, regular_type);
                if filtered_type != t {
                    return filtered_type;
                }
            }
            return self.filter_type(t, move |c, t| !(c.is_unit_like_type(t) && c.are_types_comparable(t, value_type)));
        }
        t
    }

    // flow.go:614
    pub(crate) fn narrow_type_by_typeof(&mut self, f: P<FlowState>, t: P<Type>, type_of_expr: P<Node>, operator: Kind, literal: P<Node>, assume_true: bool) -> P<Type> {
        let mut t = t;
        let mut assume_true = assume_true;
        // We have '==', '!=', '===', or !==' operator with 'typeof xxx' and string literal operands
        if operator == Kind::ExclamationEqualsToken || operator == Kind::ExclamationEqualsEqualsToken {
            assume_true = !assume_true;
        }
        let target = self.get_reference_candidate(type_of_expr.as_type_of_expression().expression);
        if !self.is_matching_reference(f.ref_node(), target) {
            if self.strict_null_checks && self.optional_chain_contains_reference(target, f.ref_node()) && assume_true == (literal.text() != "undefined") {
                t = self.get_adjusted_type_with_facts(t, TypeFacts::NEUndefinedOrNull);
            }
            let property_access = self.get_discriminant_property_access(f, target, t);
            if let Some(property_access) = property_access {
                return self.narrow_type_by_discriminant(t, property_access, move |c, t| c.narrow_type_by_literal_expression(t, literal, assume_true));
            }
            return t;
        }
        self.narrow_type_by_literal_expression(t, literal, assume_true)
    }

    // flow.go:646
    pub(crate) fn narrow_type_by_literal_expression(&mut self, t: P<Type>, literal: P<Node>, assume_true: bool) -> P<Type> {
        if assume_true {
            return self.narrow_type_by_type_name(t, literal.text());
        }
        let facts = match typeofNEFacts.get(literal.text()) {
            Some(&facts) => facts,
            None => TypeFacts::TypeofNEHostObject,
        };
        self.get_adjusted_type_with_facts(t, facts)
    }

    // flow.go:657
    pub(crate) fn narrow_type_by_type_name(&mut self, t: P<Type>, type_name: &str) -> P<Type> {
        match type_name {
            "string" => return self.narrow_type_by_type_facts(t, self.string_type, TypeFacts::TypeofEQString),
            "number" => return self.narrow_type_by_type_facts(t, self.number_type, TypeFacts::TypeofEQNumber),
            "bigint" => return self.narrow_type_by_type_facts(t, self.bigint_type, TypeFacts::TypeofEQBigInt),
            "boolean" => return self.narrow_type_by_type_facts(t, self.boolean_type, TypeFacts::TypeofEQBoolean),
            "symbol" => return self.narrow_type_by_type_facts(t, self.es_symbol_type, TypeFacts::TypeofEQSymbol),
            "object" => {
                if t.flags().intersects(TypeFlags::Any) {
                    return t;
                }
                let object_type = self.narrow_type_by_type_facts(t, self.non_primitive_type, TypeFacts::TypeofEQObject);
                let null_type = self.narrow_type_by_type_facts(t, self.null_type, TypeFacts::EQNull);
                return self.get_union_type(&[object_type, null_type]);
            }
            "function" => {
                if t.flags().intersects(TypeFlags::Any) {
                    return t;
                }
                return self.narrow_type_by_type_facts(t, self.global_function_type, TypeFacts::TypeofEQFunction);
            }
            "undefined" => return self.narrow_type_by_type_facts(t, self.undefined_type, TypeFacts::EQUndefined),
            _ => {}
        }
        self.narrow_type_by_type_facts(t, self.non_primitive_type, TypeFacts::TypeofEQHostObject)
    }

    // flow.go:685
    pub(crate) fn narrow_type_by_type_facts(&mut self, t: P<Type>, implied_type: P<Type>, facts: TypeFacts) -> P<Type> {
        self.map_type(t, move |c, t| {
            if c.is_type_related_to(t, implied_type, c.strict_subtype_relation) {
                if c.has_type_facts(t, facts) {
                    return Some(t);
                }
                return Some(c.never_type);
            } else if c.is_type_subtype_of(implied_type, t) {
                return Some(implied_type);
            } else if c.has_type_facts(t, facts) {
                return Some(c.get_intersection_type(&[t, implied_type]));
            }
            Some(c.never_type)
        })
        .unwrap()
    }

    // flow.go:702
    pub(crate) fn narrow_type_by_discriminant_property(&mut self, t: P<Type>, access: P<Node>, operator: Kind, value: P<Node>, assume_true: bool) -> P<Type> {
        if (operator == Kind::EqualsEqualsEqualsToken || operator == Kind::ExclamationEqualsEqualsToken) && t.flags().intersects(TypeFlags::Union) {
            let key_property_name = self.get_key_property_name(t);
            if !key_property_name.is_empty() {
                let (accessed_name, ok) = self.get_accessed_property_name(access);
                if ok && key_property_name == accessed_name {
                    let value_type = self.get_type_of_expression(value);
                    let candidate = self.get_constituent_type_for_key_type(t, value_type);
                    if let Some(candidate) = candidate {
                        if assume_true && operator == Kind::EqualsEqualsEqualsToken || !assume_true && operator == Kind::ExclamationEqualsEqualsToken {
                            return candidate;
                        }
                        if let Some(prop_type) = self.get_type_of_property_of_type(candidate, &key_property_name) {
                            if is_unit_type(prop_type) {
                                return self.remove_type(t, candidate);
                            }
                        }
                        return t;
                    }
                }
            }
        }
        self.narrow_type_by_discriminant(t, access, move |c, t| c.narrow_type_by_equality(t, operator, value, assume_true))
    }

    // flow.go:725
    pub(crate) fn narrow_type_by_discriminant(&mut self, t: P<Type>, access: P<Node>, narrow_type: impl FnMut(&mut Checker, P<Type>) -> P<Type>) -> P<Type> {
        let mut narrow_type = narrow_type;
        let (prop_name, ok) = self.get_accessed_property_name(access);
        if !ok {
            return t;
        }
        let optional_chain = ast::is_optional_chain(access);
        let remove_nullable = self.strict_null_checks && (optional_chain || is_non_null_access(access)) && self.maybe_type_of_kind(t, TypeFlags::Nullable);
        let mut non_null_type = t;
        if remove_nullable {
            non_null_type = self.get_type_with_facts(t, TypeFacts::NEUndefinedOrNull);
        }
        let prop_type = self.get_type_of_property_of_type(non_null_type, &prop_name);
        let mut prop_type = match prop_type {
            Some(prop_type) => prop_type,
            None => return t,
        };
        if remove_nullable && optional_chain {
            prop_type = self.get_optional_type(prop_type, false);
        }
        let narrowed_prop_type = narrow_type(self, prop_type);
        self.filter_type(t, move |c, t| {
            let discriminant_type = c.get_type_of_property_or_index_signature_of_type(t, &prop_name).unwrap_or(c.unknown_type);
            !discriminant_type.flags().intersects(TypeFlags::Never) && !narrowed_prop_type.flags().intersects(TypeFlags::Never) && c.are_types_comparable(narrowed_prop_type, discriminant_type)
        })
    }

    // flow.go:750
    pub(crate) fn is_matching_constructor_reference(&mut self, f: P<FlowState>, expr: P<Node>) -> bool {
        let mut name: Option<P<Node>> = None;
        if ast::is_property_access_expression(expr) {
            name = expr.name();
        } else if ast::is_element_access_expression(expr) && ast::is_string_literal_like(expr.as_element_access_expression().argument_expression) {
            name = Some(expr.as_element_access_expression().argument_expression);
        }
        name.is_some() && name.unwrap().text() == "constructor" && self.is_matching_reference(f.ref_node(), expr.expression().unwrap())
    }

    // flow.go:760
    pub(crate) fn narrow_type_by_constructor(&mut self, t: P<Type>, operator: Kind, identifier: P<Node>, assume_true: bool) -> P<Type> {
        // Do not narrow when checking inequality.
        if assume_true && operator != Kind::EqualsEqualsToken && operator != Kind::EqualsEqualsEqualsToken
            || !assume_true && operator != Kind::ExclamationEqualsToken && operator != Kind::ExclamationEqualsEqualsToken
        {
            return t;
        }
        // Get the type of the constructor identifier expression, if it is not a function then do not narrow.
        let identifier_type = self.get_type_of_expression(identifier);
        if !self.is_function_type(identifier_type) && !self.is_constructor_type(identifier_type) {
            return t;
        }
        // Get the prototype property of the type identifier so we can find out its type.
        let prototype_property = self.get_property_of_type(identifier_type, "prototype");
        let prototype_property = match prototype_property {
            Some(prototype_property) => prototype_property,
            None => return t,
        };
        // Get the type of the prototype, if it is undefined, or the global `Object` or `Function` types then do not narrow.
        let prototype_type = self.get_type_of_symbol(prototype_property);
        let mut candidate: Option<P<Type>> = None;
        if !is_type_any(Some(prototype_type)) {
            candidate = Some(prototype_type);
        }
        let candidate = match candidate {
            Some(candidate) if candidate != self.global_object_type && candidate != self.global_function_type => candidate,
            _ => return t,
        };
        // If the type that is being narrowed is `any` then just return the `candidate` type since every type is a subtype of `any`.
        if is_type_any(Some(t)) {
            return candidate;
        }
        // Filter out types that are not considered to be "constructed by" the `candidate` type.
        self.filter_type(t, move |c, t| c.is_constructed_by(t, candidate))
    }

    // flow.go:794
    pub(crate) fn is_constructed_by(&mut self, source: P<Type>, target: P<Type>) -> bool {
        // If either the source or target type are a class type then we need to check that they are the same exact type.
        // This is because you may have a class `A` that defines some set of properties, and another class `B`
        // that defines the same set of properties as class `A`, in that case they are structurally the same
        // type, but when you do something like `instanceOfA.constructor === B` it will return false.
        if source.flags().intersects(TypeFlags::Object) && source.object_flags().intersects(ObjectFlags::Class)
            || target.flags().intersects(TypeFlags::Object) && target.object_flags().intersects(ObjectFlags::Class)
        {
            return source.symbol() == target.symbol();
        }
        // For all other types just check that the `source` type is a subtype of the `target` type.
        self.is_type_subtype_of(source, target)
    }

    // flow.go:806
    pub(crate) fn narrow_type_by_boolean_comparison(&mut self, f: P<FlowState>, t: P<Type>, expr: P<Node>, bool_value: P<Node>, operator: Kind, assume_true: bool) -> P<Type> {
        let assume_true = (assume_true != (bool_value.kind == Kind::TrueKeyword)) != (operator != Kind::ExclamationEqualsEqualsToken && operator != Kind::ExclamationEqualsToken);
        self.narrow_type(f, t, expr, assume_true)
    }

    // flow.go:811
    pub(crate) fn narrow_type_by_instanceof(&mut self, f: P<FlowState>, t: P<Type>, expr: P<Node>, assume_true: bool) -> P<Type> {
        let binary = expr.as_binary_expression();
        let left = self.get_reference_candidate(binary.left);
        if !self.is_matching_reference(f.ref_node(), left) {
            if assume_true && self.strict_null_checks && self.optional_chain_contains_reference(left, f.ref_node()) {
                return self.get_adjusted_type_with_facts(t, TypeFacts::NEUndefinedOrNull);
            }
            return t;
        }
        let right = binary.right();
        let right_type = self.get_type_of_expression(right);
        if !self.is_type_derived_from(right_type, self.global_object_type) {
            return t;
        }
        // if the right-hand side has an object type with a custom `[Symbol.hasInstance]` method, and that method
        // has a type predicate, use the type predicate to perform narrowing. This allows normal `object` types to
        // participate in `instanceof`, as per Step 2 of https://tc39.es/ecma262/#sec-instanceofoperator.
        let mut predicate: Option<P<TypePredicate>> = None;
        if let Some(signature) = self.get_effects_signature(expr) {
            predicate = self.get_type_predicate_of_signature(signature);
        }
        if let Some(predicate) = predicate {
            if predicate.kind.get() == TypePredicateKind::Identifier && predicate.parameter_index.get() == 0 {
                return self.get_narrowed_type(t, predicate.t.get().unwrap(), assume_true, true /*checkDerived*/);
            }
        }
        if !self.is_type_derived_from(right_type, self.global_function_type) {
            return t;
        }
        let instance_type = self.map_type(right_type, |c, t| Some(c.get_instance_type(t))).unwrap();
        // Don't narrow from `any` if the target type is exactly `Object` or `Function`, and narrow
        // in the false branch only if the target is a non-empty object type.
        if is_type_any(Some(t)) && (instance_type == self.global_object_type || instance_type == self.global_function_type)
            || !assume_true && !(instance_type.flags().intersects(TypeFlags::Object) && !self.is_empty_anonymous_object_type(instance_type))
        {
            return t;
        }
        self.get_narrowed_type(t, instance_type, assume_true, true /*checkDerived*/)
    }

    // flow.go:846
    pub(crate) fn get_narrowed_type(&mut self, t: P<Type>, candidate: P<Type>, assume_true: bool, check_derived: bool) -> P<Type> {
        if !t.flags().intersects(TypeFlags::Union) {
            return self.get_narrowed_type_worker(t, candidate, assume_true, check_derived);
        }
        let key = NarrowedTypeKey { t, candidate, assume_true, check_derived };
        if let Some(&narrowed_type) = self.narrowed_types.get(&key) {
            return narrowed_type;
        }
        let narrowed_type = self.get_narrowed_type_worker(t, candidate, assume_true, check_derived);
        self.narrowed_types.insert(key, narrowed_type);
        narrowed_type
    }

    // flow.go:859
    pub(crate) fn get_narrowed_type_worker(&mut self, t: P<Type>, candidate: P<Type>, assume_true: bool, check_derived: bool) -> P<Type> {
        let mut t = t;
        if !assume_true {
            if t == candidate {
                return self.never_type;
            }
            if check_derived {
                return self.filter_type(t, move |c, t| !c.is_type_derived_from(t, candidate));
            }
            if t.flags().intersects(TypeFlags::Unknown) {
                t = self.unknown_union_type;
            }
            let true_type = self.get_narrowed_type(t, candidate, true /*assumeTrue*/, false /*checkDerived*/);
            let filtered = self.filter_type(t, move |c, t| !c.is_type_subset_of(t, true_type));
            return self.recombine_unknown_type(filtered);
        }
        if t.flags().intersects(TypeFlags::AnyOrUnknown) {
            return candidate;
        }
        if t == candidate {
            return candidate;
        }
        // We first attempt to filter the current type, narrowing constituents as appropriate and removing
        // constituents that are unrelated to the candidate.
        let mut key_property_name = String::new();
        if t.flags().intersects(TypeFlags::Union) {
            key_property_name = self.get_key_property_name(t);
        }
        let narrowed_type = self
            .map_type(candidate, |c, n| {
                // If a discriminant property is available, use that to reduce the type.
                let mut matching = t;
                if !key_property_name.is_empty() {
                    if let Some(discriminant) = c.get_type_of_property_of_type(n, &key_property_name) {
                        if let Some(constituent) = c.get_constituent_type_for_key_type(t, discriminant) {
                            matching = constituent;
                        }
                    }
                }
                // For each constituent t in the current type, if t and c are directly related, pick the most
                // specific of the two. When t and c are related in both directions, we prefer c for type predicates
                // because that is the asserted type, but t for `instanceof` because generics aren't reflected in
                // prototype object types.
                let map_type = move |c: &mut Checker, t: P<Type>| -> Option<P<Type>> {
                    if check_derived {
                        if c.is_type_derived_from(t, n) {
                            return Some(t);
                        } else if c.is_type_derived_from(n, t) {
                            return Some(n);
                        }
                        Some(c.never_type)
                    } else {
                        if c.is_type_strict_subtype_of(t, n) {
                            return Some(t);
                        } else if c.is_type_strict_subtype_of(n, t) {
                            return Some(n);
                        } else if c.is_type_subtype_of(t, n) {
                            return Some(t);
                        } else if c.is_type_subtype_of(n, t) {
                            return Some(n);
                        }
                        Some(c.never_type)
                    }
                };
                let directly_related = c.map_type(matching, map_type).unwrap();
                if !directly_related.flags().intersects(TypeFlags::Never) {
                    return Some(directly_related);
                }
                // If no constituents are directly related, create intersections for any generic constituents that
                // are related by constraint.
                let is_related = move |c: &mut Checker, s: P<Type>, t: P<Type>| -> bool {
                    if check_derived {
                        c.is_type_derived_from(s, t)
                    } else {
                        c.is_type_subtype_of(s, t)
                    }
                };
                c.map_type(t, |c, t| {
                    if c.maybe_type_of_kind(t, TypeFlags::Instantiable) {
                        let constraint = c.get_base_constraint_of_type(t);
                        if constraint.is_none() || is_related(c, n, constraint.unwrap()) {
                            return Some(c.get_intersection_type(&[t, n]));
                        }
                    }
                    Some(c.never_type)
                })
            })
            .unwrap();
        // If filtering produced a non-empty type, return that. Otherwise, pick the most specific of the two
        // based on assignability, or as a last resort produce an intersection.
        if !narrowed_type.flags().intersects(TypeFlags::Never) {
            return narrowed_type;
        } else if self.is_type_subtype_of(candidate, t) {
            return candidate;
        } else if self.is_type_assignable_to(t, candidate) {
            return t;
        } else if self.is_type_assignable_to(candidate, t) {
            return candidate;
        }
        self.get_intersection_type(&[t, candidate])
    }

    // flow.go:966
    pub(crate) fn get_instance_type(&mut self, constructor_type: P<Type>) -> P<Type> {
        let prototype_property_type = self.get_type_of_property_of_type(constructor_type, "prototype");
        if let Some(prototype_property_type) = prototype_property_type {
            if !is_type_any(Some(prototype_property_type)) {
                return prototype_property_type;
            }
        }
        let construct_signatures = self.get_signatures_of_type(constructor_type, SignatureKind::Construct);
        if !construct_signatures.is_empty() {
            let mut types = Vec::with_capacity(construct_signatures.len());
            for signature in construct_signatures {
                let erased = self.get_erased_signature(signature);
                types.push(self.get_return_type_of_signature(erased));
            }
            return self.get_union_type(&types);
        }
        // We use the empty object type to indicate we don't know the type of objects created by
        // this constructor function.
        self.empty_object_type
    }

    // flow.go:982
    pub(crate) fn narrow_type_by_private_identifier_in_in_expression(&mut self, f: P<FlowState>, t: P<Type>, expr: P<Node>, assume_true: bool) -> P<Type> {
        let binary = expr.as_binary_expression();
        let target = self.get_reference_candidate(binary.right());
        if !self.is_matching_reference(f.ref_node(), target) {
            return t;
        }
        let symbol = self.get_symbol_for_private_identifier_expression(binary.left);
        let symbol = match symbol {
            Some(symbol) => symbol,
            None => return t,
        };
        let class_symbol = symbol.parent().unwrap();
        let target_type = if ast::has_static_modifier(symbol.value_declaration().unwrap()) {
            self.get_type_of_symbol(class_symbol)
        } else {
            self.get_declared_type_of_symbol(class_symbol)
        };
        self.get_narrowed_type(t, target_type, assume_true, true /*checkDerived*/)
    }

    // flow.go:1001
    pub(crate) fn narrow_type_by_in_keyword(&mut self, f: P<FlowState>, t: P<Type>, name_type: P<Type>, assume_true: bool) -> P<Type> {
        let name = get_property_name_from_type(name_type);
        let is_known_property = some_type(t, |t| self.is_type_presence_possible(t, &name, true /*assumeTrue*/));
        if is_known_property {
            // If the check is for a known property (i.e. a property declared in some constituent of
            // the target type), we filter the target type by presence of absence of the property.
            return self.filter_type(t, |c, t| c.is_type_presence_possible(t, &name, assume_true));
        }
        if assume_true {
            // If the check is for an unknown property, we intersect the target type with `Record<X, unknown>`,
            // where X is the name of the property.
            let record_symbol = self.get_global_record_symbol();
            if let Some(record_symbol) = record_symbol {
                let unknown_type = self.unknown_type;
                let record_type = self.get_type_alias_instantiation(record_symbol, &[name_type, unknown_type], None);
                return self.get_intersection_type(&[t, record_type]);
            }
        }
        t
    }

    // flow.go:1024
    pub(crate) fn is_type_presence_possible(&mut self, t: P<Type>, prop_name: &str, assume_true: bool) -> bool {
        let prop = self.get_property_of_type(t, prop_name);
        if let Some(prop) = prop {
            return prop.flags().intersects(SymbolFlags::Optional) || prop.check_flags.get().intersects(CheckFlags::Partial) || assume_true;
        }
        self.get_applicable_index_info_for_name(t, prop_name).is_some() || !assume_true
    }

    // flow.go:1032
    pub(crate) fn narrow_type_by_optional_chain_containment(&mut self, f: P<FlowState>, t: P<Type>, operator: Kind, value: P<Node>, assume_true: bool) -> P<Type> {
        // We are in a branch of obj?.foo === value (or any one of the other equality operators). We narrow obj as follows:
        // When operator is === and type of value excludes undefined, null and undefined is removed from type of obj in true branch.
        // When operator is !== and type of value excludes undefined, null and undefined is removed from type of obj in false branch.
        // When operator is == and type of value excludes null and undefined, null and undefined is removed from type of obj in true branch.
        // When operator is != and type of value excludes null and undefined, null and undefined is removed from type of obj in false branch.
        // When operator is === and type of value is undefined, null and undefined is removed from type of obj in false branch.
        // When operator is !== and type of value is undefined, null and undefined is removed from type of obj in true branch.
        // When operator is == and type of value is null or undefined, null and undefined is removed from type of obj in false branch.
        // When operator is != and type of value is null or undefined, null and undefined is removed from type of obj in true branch.
        let equals_operator = operator == Kind::EqualsEqualsToken || operator == Kind::EqualsEqualsEqualsToken;
        let nullable_flags = if operator == Kind::EqualsEqualsToken || operator == Kind::ExclamationEqualsToken {
            TypeFlags::Nullable
        } else {
            TypeFlags::Undefined
        };
        let value_type = self.get_type_of_expression(value);
        // Note that we include any and unknown in the exclusion test because their domain includes null and undefined.
        let remove_nullable = equals_operator != assume_true && every_type(value_type, |t| t.flags().intersects(nullable_flags))
            || equals_operator == assume_true && every_type(value_type, |t| !t.flags().intersects(TypeFlags::AnyOrUnknown | nullable_flags));
        if remove_nullable {
            return self.get_adjusted_type_with_facts(t, TypeFacts::NEUndefinedOrNull);
        }
        t
    }

    // flow.go:1059
    pub(crate) fn get_type_at_switch_clause(&mut self, f: P<FlowState>, flow: P<FlowNode>) -> FlowType {
        let data = flow.node().unwrap();
        let expr = ast::skip_parentheses(data.as_flow_switch_clause_data().switch_statement.expression().unwrap());
        let flow_type = self.get_type_at_flow_node(f, flow.antecedent().unwrap());
        let mut t = flow_type.t.unwrap();
        if self.is_matching_reference(f.ref_node(), expr) {
            t = self.narrow_type_by_switch_on_discriminant(t, data);
        } else if expr.kind == Kind::TypeOfExpression && self.is_matching_reference(f.ref_node(), expr.expression().unwrap()) {
            t = self.narrow_type_by_switch_on_type_of(t, data);
        } else if expr.kind == Kind::TrueKeyword {
            t = self.narrow_type_by_switch_on_true(f, t, data);
        } else {
            if self.strict_null_checks {
                if self.optional_chain_contains_reference(expr, f.ref_node()) {
                    t = self.narrow_type_by_switch_optional_chain_containment(t, data, |_, t| !t.flags().intersects(TypeFlags::Undefined | TypeFlags::Never));
                } else if ast::is_type_of_expression(expr) && self.optional_chain_contains_reference(expr.expression().unwrap(), f.ref_node()) {
                    t = self.narrow_type_by_switch_optional_chain_containment(t, data, |_, t| {
                        !(t.flags().intersects(TypeFlags::Never) || t.flags().intersects(TypeFlags::StringLiteral) && get_string_literal_value(t) == "undefined")
                    });
                }
            }
            let access = self.get_discriminant_property_access(f, expr, t);
            if let Some(access) = access {
                t = self.narrow_type_by_switch_on_discriminant_property(t, access, data);
            }
        }
        self.new_flow_type(t, flow_type.incomplete)
    }

    // flow.go:1091
    pub(crate) fn narrow_type_by_switch_on_discriminant(&mut self, t: P<Type>, data: P<Node>) -> P<Type> {
        let data = data.as_flow_switch_clause_data();
        // We only narrow if all case expressions specify
        // values with unit types, except for the case where
        // `type` is unknown. In this instance we map object
        // types to the nonPrimitive type and narrow with that.
        let switch_types = self.get_switch_clause_types(data.switch_statement);
        if switch_types.is_empty() {
            return t;
        }
        let clause_types = &switch_types[data.clause_start as usize..data.clause_end as usize];
        let has_default_clause = data.clause_start == data.clause_end || clause_types.contains(&self.never_type);
        if t.flags().intersects(TypeFlags::Unknown) && !has_default_clause {
            let mut ground_clause_types: Option<Vec<P<Type>>> = None;
            for (i, &s) in clause_types.iter().enumerate() {
                if s.flags().intersects(TypeFlags::Primitive | TypeFlags::NonPrimitive) {
                    if let Some(ground_clause_types) = &mut ground_clause_types {
                        ground_clause_types.push(s);
                    }
                } else if s.flags().intersects(TypeFlags::Object) {
                    if ground_clause_types.is_none() {
                        ground_clause_types = Some(clause_types[..i].to_vec());
                    }
                    ground_clause_types.as_mut().unwrap().push(self.non_primitive_type);
                } else {
                    return t;
                }
            }
            return match &ground_clause_types {
                None => self.get_union_type(clause_types),
                Some(ground_clause_types) => self.get_union_type(ground_clause_types),
            };
        }
        let discriminant_type = self.get_union_type(clause_types);
        let mut case_type: Option<P<Type>> = None;
        if discriminant_type.flags().intersects(TypeFlags::Never) {
            case_type = Some(self.never_type);
        } else {
            if discriminant_type.flags().intersects(TypeFlags::Primitive) && self.is_uniform_union_type(t) {
                let regular_type = self.get_regular_type_of_literal_type(discriminant_type);
                if self.union_contains_type(t, regular_type, false /*matchSymbol*/) {
                    case_type = Some(regular_type);
                }
            }
            if case_type.is_none() {
                let filtered = self.filter_type(t, move |c, t| c.are_types_comparable(discriminant_type, t));
                case_type = Some(self.replace_primitives_with_literals(filtered, discriminant_type));
            }
        }
        let case_type = case_type.unwrap();
        if !has_default_clause {
            return case_type;
        }
        let default_type = self.filter_type(t, |c, t| {
            if !c.is_unit_like_type(t) {
                return true;
            }
            let mut u = c.undefined_type;
            if !t.flags().intersects(TypeFlags::Undefined) {
                let unit_type = c.extract_unit_type(t);
                u = c.get_regular_type_of_literal_type(unit_type);
            }
            !switch_types.iter().any(|&st| is_unit_type(st) && c.are_types_comparable(st, u))
        });
        if case_type.flags().intersects(TypeFlags::Never) {
            return default_type;
        }
        self.get_union_type(&[case_type, default_type])
    }

    // flow.go:1157
    pub(crate) fn narrow_type_by_switch_on_type_of(&mut self, t: P<Type>, data: P<Node>) -> P<Type> {
        let data = data.as_flow_switch_clause_data();
        let witnesses = self.get_switch_clause_type_of_witnesses(data.switch_statement);
        let witnesses = match witnesses {
            Some(witnesses) => witnesses,
            None => return t,
        };
        let clauses = data.switch_statement.as_switch_statement().case_block.as_case_block().clauses.nodes;
        // Equal start and end denotes implicit fallthrough; undefined marks explicit default clause.
        let default_index = clauses.iter().position(|clause| clause.kind == Kind::DefaultClause).map_or(-1, |i| i as i32);
        let clause_start = data.clause_start;
        let clause_end = data.clause_end;
        let has_default_clause = clause_start == clause_end || (default_index >= clause_start && default_index < clause_end);
        if has_default_clause {
            // In the default clause we filter constituents down to those that are not-equal to all handled cases.
            let not_equal_facts = self.get_not_equal_facts_from_typeof_switch(clause_start, clause_end, witnesses);
            return self.filter_type(t, move |c, t| c.get_type_facts(t, not_equal_facts) == not_equal_facts);
        }
        // In the non-default cause we create a union of the type narrowed by each of the listed cases.
        let clause_witnesses = &witnesses[clause_start as usize..clause_end as usize];
        let mut types = Vec::with_capacity(clause_witnesses.len());
        for &text in clause_witnesses {
            if !text.is_empty() {
                types.push(self.narrow_type_by_type_name(t, text));
            } else {
                types.push(self.never_type);
            }
        }
        self.get_union_type(&types)
    }

    // flow.go:1187
    pub(crate) fn narrow_type_by_switch_on_true(&mut self, f: P<FlowState>, t: P<Type>, data: P<Node>) -> P<Type> {
        let mut t = t;
        let data = data.as_flow_switch_clause_data();
        let clauses = data.switch_statement.as_switch_statement().case_block.as_case_block().clauses.nodes;
        let default_index = clauses.iter().position(|clause| clause.kind == Kind::DefaultClause).map_or(-1, |i| i as i32);
        let clause_start = data.clause_start;
        let clause_end = data.clause_end;
        let has_default_clause = clause_start == clause_end || (default_index >= clause_start && default_index < clause_end);
        // First, narrow away all of the cases that preceded this set of cases.
        for i in 0..clause_start as usize {
            let clause = clauses[i];
            if clause.kind == Kind::CaseClause {
                t = self.narrow_type(f, t, clause.expression().unwrap(), false /*assumeTrue*/);
            }
        }
        // If our current set has a default, then none the other cases were hit either.
        // There's no point in narrowing by the other cases in the set, since we can
        // get here through other paths.
        if has_default_clause {
            for i in clause_end as usize..clauses.len() {
                let clause = clauses[i];
                if clause.kind == Kind::CaseClause {
                    t = self.narrow_type(f, t, clause.expression().unwrap(), false /*assumeTrue*/);
                }
            }
            return t;
        }
        // Now, narrow based on the cases in this set.
        let mut types = Vec::with_capacity((clause_end - clause_start) as usize);
        for &clause in &clauses[clause_start as usize..clause_end as usize] {
            if clause.kind == Kind::CaseClause {
                types.push(self.narrow_type(f, t, clause.expression().unwrap(), true /*assumeTrue*/));
            } else {
                types.push(self.never_type);
            }
        }
        self.get_union_type(&types)
    }

    // flow.go:1223
    pub(crate) fn narrow_type_by_switch_optional_chain_containment(&mut self, t: P<Type>, data: P<Node>, clause_check: impl FnMut(&mut Checker, P<Type>) -> bool) -> P<Type> {
        let mut clause_check = clause_check;
        let data = data.as_flow_switch_clause_data();
        let every_clause_checks = data.clause_start != data.clause_end && {
            let switch_types = self.get_switch_clause_types(data.switch_statement);
            let mut every = true;
            for &s in &switch_types[data.clause_start as usize..data.clause_end as usize] {
                if !clause_check(self, s) {
                    every = false;
                    break;
                }
            }
            every
        };
        if every_clause_checks {
            return self.get_type_with_facts(t, TypeFacts::NEUndefinedOrNull);
        }
        t
    }

    // flow.go:1231
    pub(crate) fn narrow_type_by_switch_on_discriminant_property(&mut self, t: P<Type>, access: P<Node>, data: P<Node>) -> P<Type> {
        let clause_data = data.as_flow_switch_clause_data();
        if clause_data.clause_start < clause_data.clause_end && t.flags().intersects(TypeFlags::Union) {
            let (accessed_name, _) = self.get_accessed_property_name(access);
            if !accessed_name.is_empty() && self.get_key_property_name(t) == accessed_name {
                let clause_types = self.get_switch_clause_types(clause_data.switch_statement)[clause_data.clause_start as usize..clause_data.clause_end as usize].to_vec();
                let mut types = Vec::with_capacity(clause_types.len());
                for s in clause_types {
                    let result = self.get_constituent_type_for_key_type(t, s);
                    match result {
                        Some(result) => types.push(result),
                        None => types.push(self.unknown_type),
                    }
                }
                let candidate = self.get_union_type(&types);
                if candidate != self.unknown_type {
                    return candidate;
                }
            }
        }
        self.narrow_type_by_discriminant(t, access, move |c, t| c.narrow_type_by_switch_on_discriminant(t, data))
    }

    // flow.go:1253
    pub(crate) fn get_type_at_flow_branch_label(&mut self, f: P<FlowState>, flow: P<FlowNode>, antecedents: P<FlowList>) -> FlowType {
        let antecedent_start = self.antecedent_types.len();
        let mut subtype_reduction = false;
        let mut seen_incomplete = false;
        let mut bypass_flow: Option<P<FlowNode>> = None;
        let mut next = Some(antecedents);
        while let Some(list) = next {
            next = list.next.get();
            let antecedent = list.flow;
            if bypass_flow.is_none() && antecedent.flags().intersects(FlowFlags::SwitchClause) && antecedent.node().unwrap().as_flow_switch_clause_data().is_empty() {
                // The antecedent is the bypass branch of a potentially exhaustive switch statement.
                bypass_flow = Some(antecedent);
                continue;
            }
            let flow_type = self.get_type_at_flow_node(f, antecedent);
            let flow_type_t = flow_type.t.unwrap();
            // If the type at a particular antecedent path is the declared type and the
            // reference is known to always be assigned (i.e. when declared and initial types
            // are the same), there is no reason to process more antecedents since the only
            // possible outcome is subtypes that will be removed in the final union type anyway.
            if flow_type_t == f.declared() && f.declared() == f.initial() {
                self.antecedent_types.truncate(antecedent_start);
                return flow_type_of(flow_type_t);
            }
            if !self.antecedent_types[antecedent_start..].contains(&flow_type_t) {
                self.antecedent_types.push(flow_type_t);
            }
            // If an antecedent type is not a subset of the declared type, we need to perform
            // subtype reduction. This happens when a "foreign" type is injected into the control
            // flow using the instanceof operator or a user defined type predicate.
            if !self.is_type_subset_of(flow_type_t, f.initial()) {
                subtype_reduction = true;
            }
            if flow_type.incomplete {
                seen_incomplete = true;
            }
        }
        if let Some(bypass_flow) = bypass_flow {
            let flow_type = self.get_type_at_flow_node(f, bypass_flow);
            let flow_type_t = flow_type.t.unwrap();
            // If the bypass flow contributes a type we haven't seen yet and the switch statement
            // isn't exhaustive, process the bypass flow type. Since exhaustiveness checks increase
            // the risk of circularities, we only want to perform them when they make a difference.
            if !flow_type_t.flags().intersects(TypeFlags::Never)
                && !self.antecedent_types[antecedent_start..].contains(&flow_type_t)
                && !self.is_exhaustive_switch_statement(bypass_flow.node().unwrap().as_flow_switch_clause_data().switch_statement)
            {
                if flow_type_t == f.declared() && f.declared() == f.initial() {
                    self.antecedent_types.truncate(antecedent_start);
                    return flow_type_of(flow_type_t);
                }
                self.antecedent_types.push(flow_type_t);
                if !self.is_type_subset_of(flow_type_t, f.initial()) {
                    subtype_reduction = true;
                }
                if flow_type.incomplete {
                    seen_incomplete = true;
                }
            }
        }
        let types = self.antecedent_types[antecedent_start..].to_vec();
        let union_type = self.get_union_or_evolving_array_type(f, &types, if subtype_reduction { UnionReduction::Subtype } else { UnionReduction::Literal });
        let result = self.new_flow_type(union_type, seen_incomplete);
        self.antecedent_types.truncate(antecedent_start);
        result
    }

    // At flow control branch or loop junctions, if the type along every antecedent code path
    // is an evolving array type, we construct a combined evolving array type. Otherwise we
    // finalize all evolving array types.
    // flow.go:1314
    pub(crate) fn get_union_or_evolving_array_type(&mut self, f: P<FlowState>, types: &[P<Type>], subtype_reduction: UnionReduction) -> P<Type> {
        if is_evolving_array_type_list(types) {
            let mut element_types = Vec::with_capacity(types.len());
            for &t in types {
                element_types.push(self.get_element_type_of_evolving_array_type(t));
            }
            let union_type = self.get_union_type(&element_types);
            return self.get_evolving_array_type(union_type);
        }
        let mut finalized_types = Vec::with_capacity(types.len());
        for &t in types {
            finalized_types.push(self.finalize_evolving_array_type(t));
        }
        let union_type = self.get_union_type_ex(&finalized_types, subtype_reduction, None, None);
        let result = self.recombine_unknown_type(union_type);
        let declared_type = f.declared();
        if result != declared_type && (result.flags() & declared_type.flags()).intersects(TypeFlags::Union) && result.as_union_type().types() == declared_type.as_union_type().types() {
            return declared_type;
        }
        result
    }

    // flow.go:1325
    pub(crate) fn get_type_at_flow_loop_label(&mut self, f: P<FlowState>, flow: P<FlowNode>) -> FlowType {
        if f.ref_key.get().is_zero() {
            let ref_key = self.get_flow_reference_key(f);
            f.ref_key.set(ref_key);
        }
        if f.ref_key.get() == nonDottedNameCacheKey {
            // No cache key is generated when binding patterns are in unnarrowable situations
            return flow_type_of(f.declared());
        }
        let key = FlowLoopKey { flow_node: flow, ref_key: f.ref_key.get() };
        // If we have previously computed the control flow type for the reference at
        // this flow loop junction, return the cached type.
        if let Some(&cached) = self.flow_loop_cache.get(&key) {
            return flow_type_of(cached);
        }
        // If this flow loop junction and reference are already being processed, return
        // the union of the types computed for each branch so far, marked as incomplete.
        // It is possible to see an empty array in cases where loops are nested and the
        // back edge of the outer loop reaches an inner loop that is already being analyzed.
        // In such cases we restart the analysis of the inner loop, which will then see
        // a non-empty in-process array for the outer loop and eventually terminate because
        // the first antecedent of a loop junction is always the non-looping control flow
        // path that leads to the top.
        let in_process_types = self.flow_loop_stack.iter().find(|loop_info| loop_info.key == key && !loop_info.types.is_empty()).map(|loop_info| loop_info.types.clone());
        if let Some(in_process_types) = in_process_types {
            let union_type = self.get_union_or_evolving_array_type(f, &in_process_types, UnionReduction::Literal);
            return self.new_flow_type(union_type, true /*incomplete*/);
        }
        // Add the flow loop junction and reference to the in-process stack and analyze
        // each antecedent code path.
        let mut antecedent_types: Vec<P<Type>> = Vec::with_capacity(4);
        let mut subtype_reduction = false;
        let mut first_antecedent_type = FlowType::default();
        let mut next = flow.antecedents();
        while let Some(list) = next {
            next = list.next.get();
            let flow_type: FlowType;
            if first_antecedent_type.is_nil() {
                // The first antecedent of a loop junction is always the non-looping control
                // flow path that leads to the top.
                first_antecedent_type = self.get_type_at_flow_node(f, list.flow);
                flow_type = first_antecedent_type;
            } else {
                // All but the first antecedent are the looping control flow paths that lead
                // back to the loop junction. We track these on the flow loop stack.
                self.flow_loop_stack.push(FlowLoopInfo { key, types: antecedent_types.clone() });
                let save_flow_type_cache = self.flow_type_cache.take();
                flow_type = self.get_type_at_flow_node(f, list.flow);
                self.flow_type_cache = save_flow_type_cache;
                self.flow_loop_stack.pop();
                // If we see a value appear in the cache it is a sign that control flow analysis
                // was restarted and completed by checkExpressionCached. We can simply pick up
                // the resulting type and bail out.
                if let Some(&cached) = self.flow_loop_cache.get(&key) {
                    return flow_type_of(cached);
                }
            }
            let flow_type_t = flow_type.t.unwrap();
            if !antecedent_types.contains(&flow_type_t) {
                antecedent_types.push(flow_type_t);
            }
            // If an antecedent type is not a subset of the declared type, we need to perform
            // subtype reduction. This happens when a "foreign" type is injected into the control
            // flow using the instanceof operator or a user defined type predicate.
            if !self.is_type_subset_of(flow_type_t, f.initial()) {
                subtype_reduction = true;
            }
            // If the type at a particular antecedent path is the declared type there is no
            // reason to process more antecedents since the only possible outcome is subtypes
            // that will be removed in the final union type anyway.
            if flow_type_t == f.declared() {
                break;
            }
        }
        // The result is incomplete if the first antecedent (the non-looping control flow path)
        // is incomplete.
        let result = self.get_union_or_evolving_array_type(f, &antecedent_types, if subtype_reduction { UnionReduction::Subtype } else { UnionReduction::Literal });
        if first_antecedent_type.incomplete {
            return self.new_flow_type(result, true /*incomplete*/);
        }
        self.flow_loop_cache.insert(key, result);
        flow_type_of(result)
    }
