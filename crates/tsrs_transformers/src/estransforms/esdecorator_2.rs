// esdecorator.go lines 1231-end (class elements, expressions, descriptors, metadata).

use crate::*;
use printer::EmitFlags;

use super::*;

// esdecorator.go:1232
#[derive(Default)]
pub(crate) struct partialResult {
    pub modifiers: Option<P<ModifierList>>,
    pub referenced_name: Option<P<Node>>,
    pub name: Option<P<Node>>,
    pub initializers_name: Option<P<Node>>,
    pub extra_initializers_name: Option<P<Node>>,
    pub descriptor_name: Option<P<Node>>,
    pub this_arg: Option<P<Node>>,
}

// esdecorator.go:1242
pub(crate) type createDescriptorFunc = fn(&esDecoratorTransformer, P<Node>, Option<P<ModifierList>>) -> P<Node>;

impl esDecoratorTransformer {
    // esdecorator.go:1244
    pub(crate) fn partial_transform_class_element(&self, member: P<Node>, ci: Option<P<classInfo>>, create_descriptor: Option<createDescriptorFunc>) -> partialResult {
        let f = self.base.factory();
        let ec = self.base.emit_context();

        let Some(ci) = ci else {
            let modifiers = self.modifier_visitor.get().unwrap().clone().visit_modifiers(member.modifiers());
            self.enter_name();
            let name = self.visit_property_name(member.name().unwrap());
            self.exit_name();
            return partialResult { modifiers, name: Some(name), ..Default::default() };
        };

        // Member decorators require privileged access to private names. However, computed property
        // evaluation occurs interspersed with decorator evaluation. This means that if we encounter
        // a computed property name we must inline decorator evaluation.

        // Collect decorators for this member. Decorator expressions evaluate outside the class body,
        // so `this` should NOT be replaced with `_classThis`.
        let saved_class_this = self.class_this.get();
        self.class_this.set(None);
        let member_decorators = self.transform_all_decorators_of_declaration(&member.decorators());
        self.class_this.set(saved_class_this);
        let modifiers = self.modifier_visitor.get().unwrap().clone().visit_modifiers(member.modifiers());

        let mut result = partialResult { modifiers, ..Default::default() };

        if !member_decorators.is_empty() {
            let member_decorators_name = self.create_helper_variable(member, "decorators");
            let member_decorators_array = f.new_array_literal_expression(f.new_node_list(member_decorators), false);
            let member_decorators_assignment = f.new_assignment_expression(member_decorators_name, member_decorators_array);
            let mi = P::new(memberInfo::default());
            mi.member_decorators_name.set(Some(member_decorators_name));
            ci.member_infos.borrow_mut().insert(member, mi);
            self.pending_expressions.borrow_mut().get_or_insert_with(Vec::new).push(member_decorators_assignment);

            // 5. Static non-field (method/getter/setter/auto-accessor) element decorators are applied
            // 6. Non-static non-field (method/getter/setter/auto-accessor) element decorators are applied
            // 7. Static field (excl. auto-accessor) element decorators are applied
            // 8. Non-static field (excl. auto-accessor) element decorators are applied

            // Determine decorator kind
            let kind = if ast::is_get_accessor_declaration(member) {
                "getter"
            } else if ast::is_set_accessor_declaration(member) {
                "setter"
            } else if ast::is_method_declaration(member) {
                "method"
            } else if ast::is_auto_accessor_property_declaration(member) {
                "accessor"
            } else if ast::is_property_declaration(member) {
                "field"
            } else {
                panic!("Unexpected class element kind.");
            };

            // Determine the property name for the context
            let mut property_name_computed = false;
            let mut property_name_expr: Option<P<Node>> = None;
            let member_name = member.name();
            if let Some(name) = member_name.filter(|n| ast::is_identifier(*n) || ast::is_private_identifier(*n)) {
                property_name_computed = false;
                property_name_expr = Some(name);
            } else if let Some(name) = member_name.filter(|n| ast::is_property_name_literal(*n)) {
                property_name_computed = true;
                property_name_expr = Some(f.new_string_literal_from_node(name));
            } else if let Some(name) = member_name.filter(|n| ast::is_computed_property_name(*n)) {
                let cpn_expression = name.as_computed_property_name().expression;
                if ast::is_property_name_literal(cpn_expression) && !ast::is_identifier(cpn_expression) {
                    property_name_computed = true;
                    property_name_expr = Some(f.new_string_literal_from_node(cpn_expression));
                } else {
                    self.enter_name();
                    let (referenced_name, name) = self.visit_referenced_property_name(name);
                    result.referenced_name = referenced_name;
                    result.name = Some(name);
                    self.exit_name();
                    property_name_computed = true;
                    property_name_expr = result.referenced_name;
                }
            }

            let context_obj = f.new_es_decorate_class_element_context_object(
                kind,
                property_name_computed,
                property_name_expr.unwrap(),
                ast::is_static(member),
                member_name.is_some_and(|n| ast::is_private_identifier(n)),
                // 15.7.3 CreateDecoratorAccessObject (kind, name)
                // 2. If _kind_ is ~field~, ~method~, ~accessor~, or ~getter~, then ...
                ast::is_property_declaration(member) || ast::is_get_accessor_declaration(member) || ast::is_method_declaration(member),
                // 3. If _kind_ is ~field~, ~accessor~, or ~setter~, then ...
                ast::is_property_declaration(member) || ast::is_set_accessor_declaration(member),
                ci.metadata_reference.get().unwrap(),
            );

            if ast::is_method_or_accessor(member) {
                // produces (public elements):
                //   __esDecorate(this, null, _member_decorators, { kind: "method", name: "...", static: false, private: false, access: { ... } }, _instanceExtraInitializers);
                // produces (private elements):
                //   __esDecorate(this, _member_descriptor = { value() { ... } }, _member_decorators, { kind: "method", name: "...", static: false, private: true, access: { ... } }, _instanceExtraInitializers);
                let method_extra_initializers_name = if ast::is_static(member) { ci.static_method_extra_initializers_name.get() } else { ci.instance_method_extra_initializers_name.get() };
                assert!(method_extra_initializers_name.is_some(), "methodExtraInitializersName should be defined");

                let descriptor_arg = if let (true, Some(create_descriptor)) = (ast::is_private_identifier_class_element_declaration(member), create_descriptor) {
                    // For private members, extract the method/accessor body into a descriptor object.
                    // Filter modifiers to only keep async.
                    let async_mods = self.async_only_modifier_visitor.get().unwrap().clone().visit_modifiers(result.modifiers);
                    let descriptor = create_descriptor(self, member, async_mods);
                    let descriptor_name = self.create_helper_variable(member, "descriptor");
                    mi.member_descriptor_name.set(Some(descriptor_name));
                    result.descriptor_name = Some(descriptor_name);
                    f.new_assignment_expression(descriptor_name, descriptor)
                } else {
                    f.new_token(Kind::NullKeyword)
                };

                let es_decorate_expr = f.new_es_decorate_helper(f.new_this_expression(), descriptor_arg, member_decorators_name, context_obj, f.new_token(Kind::NullKeyword), method_extra_initializers_name.unwrap());
                let es_decorate_statement = f.new_expression_statement(es_decorate_expr);
                ec.set_source_map_range(es_decorate_statement, move_range_past_decorators(member));
                self.append_decoration_statement(ci, member, es_decorate_statement);
            } else if ast::is_property_declaration(member) {
                let initializers_name = self.create_helper_variable(member, "initializers");
                let extra_initializers_name = self.create_helper_variable(member, "extraInitializers");
                mi.member_initializers_name.set(Some(initializers_name));
                mi.member_extra_initializers_name.set(Some(extra_initializers_name));
                result.initializers_name = Some(initializers_name);
                result.extra_initializers_name = Some(extra_initializers_name);
                if ast::is_static(member) {
                    result.this_arg = ci.class_this.get();
                }

                let ctor_arg = if ast::is_auto_accessor_property_declaration(member) { f.new_this_expression() } else { f.new_token(Kind::NullKeyword) };

                let descriptor_arg = if let (true, Some(create_descriptor)) = (ast::is_private_identifier_class_element_declaration(member) && ast::has_accessor_modifier(member), create_descriptor) {
                    let descriptor = create_descriptor(self, member, None);
                    let descriptor_name = self.create_helper_variable(member, "descriptor");
                    mi.member_descriptor_name.set(Some(descriptor_name));
                    result.descriptor_name = Some(descriptor_name);
                    f.new_assignment_expression(descriptor_name, descriptor)
                } else {
                    f.new_token(Kind::NullKeyword)
                };

                // produces:
                //   __esDecorate(null, null, _member_decorators, { kind: "field", name: "...", static: false, private: ..., access: { ... } }, _instanceExtraInitializers);
                let es_decorate_expr = f.new_es_decorate_helper(ctor_arg, descriptor_arg, member_decorators_name, context_obj, initializers_name, extra_initializers_name);
                let es_decorate_statement = f.new_expression_statement(es_decorate_expr);
                ec.set_source_map_range(es_decorate_statement, move_range_past_decorators(member));
                self.append_decoration_statement(ci, member, es_decorate_statement);
            }
        }

        if result.name.is_none() {
            self.enter_name();
            result.name = Some(self.visit_property_name(member.name().unwrap()));
            self.exit_name();
        }

        if result.modifiers.map_or(true, |m| m.nodes().is_empty()) && (ast::is_method_declaration(member) || ast::is_property_declaration(member)) {
            // Don't emit leading comments on the name for methods and properties without modifiers, otherwise we
            // will end up printing duplicate comments.
            ec.set_emit_flags(result.name.unwrap(), EmitFlags::NoLeadingComments);
        }

        result
    }

    // esdecorator.go:1447
    // appendDecorationStatement appends an __esDecorate statement to the appropriate
    // decoration statement list on classInfo based on the member's kind and static-ness.
    pub(crate) fn append_decoration_statement(&self, ci: P<classInfo>, member: P<Node>, stmt: P<Node>) {
        if ast::is_method_or_accessor(member) || ast::is_auto_accessor_property_declaration(member) {
            if ast::is_static(member) {
                ci.static_non_field_decoration_statements.borrow_mut().push(stmt);
            } else {
                ci.non_static_non_field_decoration_statements.borrow_mut().push(stmt);
            }
        } else if ast::is_property_declaration(member) && !ast::is_auto_accessor_property_declaration(member) {
            if ast::is_static(member) {
                ci.static_field_decoration_statements.borrow_mut().push(stmt);
            } else {
                ci.non_static_field_decoration_statements.borrow_mut().push(stmt);
            }
        } else {
            panic!("Unexpected class element kind.");
        }
    }
}

impl esDecoratorTransformer {
    // esdecorator.go:1465
    pub(crate) fn visit_method_declaration(&self, node: P<Node>) -> Option<P<Node>> {
        self.enter_class_element(node);
        let result = self.partial_transform_class_element(node, self.class_info_stack.get(), Some(esDecoratorTransformer::create_method_descriptor_object));
        if let Some(descriptor_name) = result.descriptor_name {
            self.exit_class_element();
            return Some(self.finish_class_element(self.create_method_descriptor_forwarder(result.modifiers, result.name.unwrap(), descriptor_name), node));
        }
        let parameters = self.base.visitor().visit_nodes(node.parameter_list());
        let body = self.base.visitor().visit_node(node.body());
        self.exit_class_element();
        let asterisk = node.as_method_declaration().asterisk_token();
        Some(self.finish_class_element(self.base.factory().update_method_declaration(node, result.modifiers, asterisk, result.name.unwrap(), None, None, parameters, None, None, body), node))
    }

    // esdecorator.go:1482
    pub(crate) fn visit_get_accessor_declaration(&self, node: P<Node>) -> Option<P<Node>> {
        self.enter_class_element(node);
        let result = self.partial_transform_class_element(node, self.class_info_stack.get(), Some(esDecoratorTransformer::create_get_accessor_descriptor_object));
        if let Some(descriptor_name) = result.descriptor_name {
            self.exit_class_element();
            return Some(self.finish_class_element(self.create_get_accessor_descriptor_forwarder(result.modifiers, result.name.unwrap(), descriptor_name), node));
        }
        let parameters = self.base.visitor().visit_nodes(node.parameter_list());
        let body = self.base.visitor().visit_node(node.body());
        self.exit_class_element();
        Some(self.finish_class_element(self.base.factory().update_get_accessor_declaration(node, result.modifiers, result.name.unwrap(), None, parameters, None, None, body), node))
    }

    // esdecorator.go:1499
    pub(crate) fn visit_set_accessor_declaration(&self, node: P<Node>) -> Option<P<Node>> {
        self.enter_class_element(node);
        let result = self.partial_transform_class_element(node, self.class_info_stack.get(), Some(esDecoratorTransformer::create_set_accessor_descriptor_object));
        if let Some(descriptor_name) = result.descriptor_name {
            self.exit_class_element();
            return Some(self.finish_class_element(self.create_set_accessor_descriptor_forwarder(result.modifiers, result.name.unwrap(), descriptor_name), node));
        }
        let parameters = self.base.visitor().visit_nodes(node.parameter_list());
        let body = self.base.visitor().visit_node(node.body());
        self.exit_class_element();
        Some(self.finish_class_element(self.base.factory().update_set_accessor_declaration(node, result.modifiers, result.name.unwrap(), None, parameters, None, None, body), node))
    }

    // esdecorator.go:1516
    pub(crate) fn visit_class_static_block_declaration(&self, node: P<Node>) -> Option<P<Node>> {
        self.enter_class_element(node);
        let f = self.base.factory();
        let ec = self.base.emit_context();

        let mut result: P<Node>;
        if is_class_named_evaluation_helper_block(ec, node) {
            result = self.base.visitor().visit_each_child(Some(node)).unwrap();
            // Transfer AssignedName metadata to the new node so isClassNamedEvaluationHelperBlock
            // can still find it after visiting (visiting may create a new node when this->_classThis)
            if let Some(assigned_name) = ec.assigned_name(node) {
                if result != node {
                    ec.set_assigned_name(result, assigned_name);
                }
            }
        } else if is_class_this_assignment_block(ec, node) {
            let saved_class_this = self.class_this.get();
            self.class_this.set(None);
            result = self.base.visitor().visit_each_child(Some(node)).unwrap();
            self.class_this.set(saved_class_this);
        } else {
            // Use a nested variable environment so temp vars generated during static block
            // content transformation (e.g., super access temps) stay scoped to the static block.
            ec.start_variable_environment();
            result = self.base.visitor().visit_each_child(Some(node)).unwrap();
            let var_statements = ec.end_variable_environment();
            if !var_statements.is_empty() {
                // Inject var declarations at the start of the static block's body
                let block_body = result.as_class_static_block_declaration().body;
                let mut new_stmts: Vec<P<Node>> = var_statements;
                new_stmts.extend_from_slice(block_body.statements());
                result = f.new_class_static_block_declaration(None, f.new_block(f.new_node_list(new_stmts), block_body.as_block().multi_line));
            }
            if let Some(ci) = self.class_info_stack.get() {
                ci.has_static_initializers.set(true);
                if !ci.pending_static_initializers.borrow().is_empty() {
                    // If we tried to inject the pending initializers into the current block, we might run into
                    // variable name collisions due to sharing this blocks scope. To avoid this, we inject a new
                    // static block that contains the pending initializers that precedes this block.
                    let pending = std::mem::take(&mut *ci.pending_static_initializers.borrow_mut());
                    let mut stmts: Vec<P<Node>> = Vec::new();
                    for init in pending {
                        let init_stmt = f.new_expression_statement(init);
                        ec.set_source_map_range(init_stmt, ec.source_map_range(init));
                        stmts.push(init_stmt);
                    }
                    let body = f.new_block(f.new_node_list(stmts), true);
                    let static_block = f.new_class_static_block_declaration(None, body);
                    // Return both the new static block and the original
                    self.exit_class_element();
                    return single_or_many(Some(vec![static_block, result]), f);
                }
            }
        }

        self.exit_class_element();
        Some(result)
    }
}

impl esDecoratorTransformer {
    // esdecorator.go:1574
    pub(crate) fn visit_property_declaration(&self, node: P<Node>) -> Option<P<Node>> {
        let ec = self.base.emit_context();
        let f = self.base.factory();
        let mut node = node;
        if is_named_evaluation_and(ec, node, Some(&is_anonymous_class_needing_assigned_name)) {
            node = transform_named_evaluation(ec, node, can_ignore_empty_string_literal_in_assigned_name(node.initializer()), "");
        }

        self.enter_class_element(node);

        // TODO(rbuckton): We support decorating `declare x` fields with legacyDecorators, but we currently don't
        //                 support them with esDecorators. We need to consider whether we will support them in the
        //                 future, and how. For now, these should be elided by the `ts` transform.
        assert!(!ast::has_syntactic_modifier(node, ModifierFlags::Ambient), "Not yet implemented.");

        let create_descriptor: Option<createDescriptorFunc> = if ast::has_accessor_modifier(node) { Some(esDecoratorTransformer::create_accessor_property_descriptor_object) } else { None };
        let result = self.partial_transform_class_element(node, self.class_info_stack.get(), create_descriptor);

        ec.start_variable_environment();

        let mut initializer = self.base.visitor().visit_node(node.initializer());
        if let Some(initializers_name) = result.initializers_name {
            let this_arg = result.this_arg.unwrap_or_else(|| f.new_this_expression());
            let value = initializer.unwrap_or_else(|| f.new_void_zero_expression());
            initializer = Some(f.new_run_initializers_helper(this_arg, initializers_name, Some(value)));
        }

        if ast::is_static(node) && initializer.is_some() {
            if let Some(ci) = self.class_info_stack.get() {
                ci.has_static_initializers.set(true);
            }
        }

        let declarations = ec.end_variable_environment();
        if !declarations.is_empty() {
            let mut stmts = declarations;
            stmts.push(f.new_return_statement(initializer));
            initializer = Some(f.new_immediately_invoked_arrow_function(stmts));
        }

        if let Some(ci) = self.class_info_stack.get() {
            if ast::is_static(node) {
                initializer = self.inject_pending_initializers(ci, true, initializer);
                if let Some(extra) = result.extra_initializers_name {
                    let this_arg = ci.class_this.get().unwrap_or_else(|| f.new_this_expression());
                    ci.pending_static_initializers.borrow_mut().push(f.new_run_initializers_helper(this_arg, extra, None));
                }
            } else {
                initializer = self.inject_pending_initializers(ci, false, initializer);
                if let Some(extra) = result.extra_initializers_name {
                    ci.pending_instance_initializers.borrow_mut().push(f.new_run_initializers_helper(f.new_this_expression(), extra, None));
                }
            }
        }

        self.exit_class_element();

        if let (true, Some(descriptor_name)) = (ast::has_accessor_modifier(node), result.descriptor_name) {
            // given:
            //  accessor #x = 1;
            //
            // emits:
            //  static {
            //      _esDecorate(null, _private_x_descriptor = { get() { return this.#x_1; }, set(value) { this.#x_1 = value; } }, ...)
            //  }
            //  ...
            //  #x_1 = 1;
            //  get #x() { return _private_x_descriptor.get.call(this); }
            //  set #x(value) { _private_x_descriptor.set.call(this, value); }

            let comment_range = ec.comment_range(node);
            let source_map_range = ec.source_map_range(node);

            // Since we're creating two declarations where there was previously one, cache
            // the expression for any computed property names.
            let prop_name = node.name().unwrap();
            let mut getter_name = result.name.unwrap();
            let mut setter_name = result.name.unwrap();
            if ast::is_computed_property_name(prop_name) && !is_simple_inlineable_expression(prop_name.expression().unwrap()) {
                if let Some(cache_assignment) = find_computed_property_name_cache_assignment(ec, prop_name) {
                    getter_name = f.update_computed_property_name(prop_name, self.base.visitor().visit_node(prop_name.expression()).unwrap());
                    setter_name = f.update_computed_property_name(prop_name, cache_assignment.as_binary_expression().left);
                } else {
                    let temp = f.new_temp_variable();
                    ec.set_source_map_range(temp, prop_name.expression().unwrap().loc());
                    ec.add_variable_declaration(temp);
                    let expression = self.base.visitor().visit_node(prop_name.expression()).unwrap();
                    let assignment = f.new_assignment_expression(temp, expression);
                    ec.set_source_map_range(assignment, prop_name.expression().unwrap().loc());
                    getter_name = f.update_computed_property_name(prop_name, assignment);
                    setter_name = f.update_computed_property_name(prop_name, temp);
                }
            }

            let modifiers_without_accessor = self.accessor_stripping_modifier_visitor.get().unwrap().clone().visit_modifiers(result.modifiers);

            let backing_field = create_accessor_property_backing_field(f, node, modifiers_without_accessor, initializer);
            ec.set_original(backing_field, node);
            ec.set_emit_flags(backing_field, EmitFlags::NoComments);
            ec.set_source_map_range(backing_field, source_map_range);
            ec.set_source_map_range(backing_field.name().unwrap(), ec.source_map_range(node.name().unwrap()));

            let getter = self.create_get_accessor_descriptor_forwarder(modifiers_without_accessor, getter_name, descriptor_name);
            ec.set_original(getter, node);
            ec.set_comment_range(getter, comment_range);
            ec.set_source_map_range(getter, source_map_range);

            let setter = self.create_set_accessor_descriptor_forwarder(modifiers_without_accessor, setter_name, descriptor_name);
            ec.set_original(setter, node);
            ec.set_emit_flags(setter, EmitFlags::NoComments);
            ec.set_source_map_range(setter, source_map_range);

            return single_or_many(Some(vec![backing_field, getter, setter]), f);
        }

        Some(self.finish_class_element(f.update_property_declaration(node, result.modifiers, result.name.unwrap(), None, None, initializer), node))
    }

    // esdecorator.go:1724
    pub(crate) fn visit_this_expression(&self, node: P<Node>) -> P<Node> {
        self.class_this.get().unwrap_or(node)
    }
}
