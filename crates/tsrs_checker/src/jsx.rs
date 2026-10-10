use crate::*;
use tsrs_ast::*;
use tsrs_core::*;
use tsrs_ast as ast;
use tsrs_diagnostics as diagnostics;
use tsrs_diagnostics::Message;
use std::cell::RefCell;
use std::fmt::Display;
use std::rc::Rc;

// Non-function declarations in jsx.go are hand-ported in jsx_types.rs.

/// Go's `getInvalidTextDiagnostic func() (*diagnostics.Message, []any)`; shared (`Rc`) because the
/// `createDiagnostic` closure of a JSX text elaboration element captures it.
pub(crate) type InvalidTextDiagnosticFn = Rc<dyn Fn(&mut Checker) -> (&'static Message, Vec<String>)>;

impl Checker {
    // jsx.go:72
    pub(crate) fn check_jsx_element(&mut self, node: P<Node>, _check_mode: CheckMode) -> P<Type> {
        self.check_node_deferred(node);
        self.get_jsx_element_type_at(node)
    }

    // jsx.go:77
    pub(crate) fn check_jsx_element_deferred(&mut self, node: P<Node>) {
        let jsx_element = node.as_jsx_element();
        self.check_jsx_opening_like_element_or_opening_fragment(jsx_element.opening_element);
        // Perform resolution on the closing tag so that rename/go to definition/etc work
        if is_jsx_intrinsic_tag_name(jsx_element.closing_element.tag_name()) {
            self.get_intrinsic_tag_symbol(jsx_element.closing_element);
        } else {
            self.check_expression(jsx_element.closing_element.tag_name());
        }
        self.check_jsx_children(node, CheckMode::Normal);
    }

    // jsx.go:89
    pub(crate) fn check_jsx_expression(&mut self, node: P<Node>, check_mode: CheckMode) -> P<Type> {
        self.check_grammar_jsx_expression(node);
        let Some(expression) = node.expression() else {
            return self.error_type;
        };
        let t = self.check_expression_ex(expression, check_mode);
        if node.as_jsx_expression().dot_dot_dot_token.is_some() && t != self.any_type && !self.is_array_type(t) {
            self.error(Some(node), &diagnostics::JSX_spread_child_must_be_an_array_type, &[]);
        }
        t
    }

    // jsx.go:101
    pub(crate) fn check_jsx_self_closing_element(&mut self, node: P<Node>, _check_mode: CheckMode) -> P<Type> {
        self.check_node_deferred(node);
        self.get_jsx_element_type_at(node)
    }

    // jsx.go:106
    pub(crate) fn check_jsx_self_closing_element_deferred(&mut self, node: P<Node>) {
        self.check_jsx_opening_like_element_or_opening_fragment(node);
    }

    // jsx.go:110
    pub(crate) fn check_jsx_fragment(&mut self, node: P<Node>) -> P<Type> {
        self.check_jsx_opening_like_element_or_opening_fragment(node.as_jsx_fragment().opening_fragment);
        // by default, jsx:'react' will use jsxFactory = React.createElement and jsxFragmentFactory = React.Fragment
        // if jsxFactory compiler option is provided, ensure jsxFragmentFactory compiler option or @jsxFrag pragma is provided too
        let node_source_file = ast::get_source_file_of_node(node);
        let compiler_options = self.compiler_options;
        if compiler_options.get_jsx_transform_enabled()
            && (!compiler_options.jsx_factory.is_empty() || ast::get_pragma_from_source_file(node_source_file, "jsx").is_some())
            && compiler_options.jsx_fragment_factory.is_empty()
            && ast::get_pragma_from_source_file(node_source_file, "jsxfrag").is_none()
        {
            let message = if !compiler_options.jsx_factory.is_empty() {
                &diagnostics::The_jsxFragmentFactory_compiler_option_must_be_provided_to_use_JSX_fragments_with_the_jsxFactory_compiler_option
            } else {
                &diagnostics::An_jsxFrag_pragma_is_required_when_using_an_jsx_pragma_with_JSX_fragments
            };
            self.error(Some(node), message, &[]);
        }
        self.check_jsx_children(node, CheckMode::Normal);
        let t = self.get_jsx_element_type_at(node);
        if self.is_error_type(t) { self.any_type } else { t }
    }

    // jsx.go:126
    pub(crate) fn check_jsx_attributes(&mut self, node: P<Node>, check_mode: CheckMode) -> P<Type> {
        self.check_node_deferred(node);
        self.create_jsx_attributes_type_from_attributes_property(node.parent().unwrap(), check_mode)
    }

    // jsx.go:131
    pub(crate) fn check_jsx_opening_like_element_or_opening_fragment(&mut self, node: P<Node>) {
        let is_node_opening_like_element = ast::is_jsx_opening_like_element(node);
        if is_node_opening_like_element {
            self.check_grammar_jsx_element(node);
        }
        self.check_jsx_preconditions(node);
        self.mark_jsx_alias_referenced(node);
        let sig = self.get_resolved_signature(node, None, CheckMode::Normal);
        self.check_deprecated_signature(sig, node);
        if is_node_opening_like_element {
            let element_type_constraint = self.get_jsx_element_type_type_at(node);
            if let Some(element_type_constraint) = element_type_constraint {
                let tag_name = node.tag_name();
                let tag_type = if is_jsx_intrinsic_tag_name(tag_name) {
                    self.get_string_literal_type(tag_name.text())
                } else {
                    self.check_expression(tag_name)
                };
                let mut diags: Vec<P<Diagnostic>> = Vec::new();
                let assignable_relation = self.assignable_relation;
                if !self.check_type_related_to_ex(
                    tag_type,
                    element_type_constraint,
                    assignable_relation,
                    Some(tag_name),
                    Some(&diagnostics::Its_type_0_is_not_a_valid_JSX_element_type),
                    Some(&mut diags),
                ) {
                    let text = tsrs_scanner::get_text_of_node(tag_name);
                    self.add_diagnostic(ast::new_diagnostic_chain(diags[0], &diagnostics::X_0_cannot_be_used_as_a_JSX_component, &[&text]));
                }
            } else {
                let ref_kind = self.get_jsx_reference_kind(node);
                let return_type = self.get_return_type_of_signature(sig);
                self.check_jsx_return_assignable_to_appropriate_bound(ref_kind, return_type, node);
            }
        }
    }

    // jsx.go:160
    pub(crate) fn check_jsx_preconditions(&mut self, error_node: P<Node>) {
        // Preconditions for using JSX
        if self.compiler_options.jsx == JsxEmit::None {
            self.error(Some(error_node), &diagnostics::Cannot_use_JSX_unless_the_jsx_flag_is_provided, &[]);
        }
        // getJsxElementTypeAt never returns nil (getJsxType falls back to errorType), so Go's nil comparison is
        // always false; the call is kept for its side effects.
        if self.no_implicit_any && {
            self.get_jsx_element_type_at(error_node);
            false
        } {
            self.error(Some(error_node), &diagnostics::JSX_element_implicitly_has_type_any_because_the_global_type_JSX_Element_does_not_exist, &[]);
        }
    }

    // jsx.go:170
    pub(crate) fn check_jsx_return_assignable_to_appropriate_bound(&mut self, ref_kind: JsxReferenceKind, elem_instance_type: P<Type>, opening_like_element: P<Node>) {
        let mut diags: Vec<P<Diagnostic>> = Vec::new();
        let assignable_relation = self.assignable_relation;
        match ref_kind {
            JsxReferenceKind::Function => {
                let sfc_return_constraint = self.get_jsx_stateless_element_type_at(opening_like_element);
                if let Some(sfc_return_constraint) = sfc_return_constraint {
                    self.check_type_related_to_ex(
                        elem_instance_type,
                        sfc_return_constraint,
                        assignable_relation,
                        Some(opening_like_element.tag_name()),
                        Some(&diagnostics::Its_return_type_0_is_not_a_valid_JSX_element),
                        Some(&mut diags),
                    );
                }
            }
            JsxReferenceKind::Component => {
                let class_constraint = self.get_jsx_element_class_type_at(opening_like_element);
                if let Some(class_constraint) = class_constraint {
                    // Issue an error if this return type isn't assignable to JSX.ElementClass, failing that
                    self.check_type_related_to_ex(
                        elem_instance_type,
                        class_constraint,
                        assignable_relation,
                        Some(opening_like_element.tag_name()),
                        Some(&diagnostics::Its_instance_type_0_is_not_a_valid_JSX_element),
                        Some(&mut diags),
                    );
                }
            }
            _ => {
                let sfc_return_constraint = self.get_jsx_stateless_element_type_at(opening_like_element);
                let class_constraint = self.get_jsx_element_class_type_at(opening_like_element);
                let (Some(sfc_return_constraint), Some(class_constraint)) = (sfc_return_constraint, class_constraint) else {
                    return;
                };
                let combined = self.get_union_type(&[sfc_return_constraint, class_constraint]);
                self.check_type_related_to_ex(
                    elem_instance_type,
                    combined,
                    assignable_relation,
                    Some(opening_like_element.tag_name()),
                    Some(&diagnostics::Its_element_type_0_is_not_a_valid_JSX_element),
                    Some(&mut diags),
                );
            }
        }
        if !diags.is_empty() {
            let text = tsrs_scanner::get_text_of_node(opening_like_element.tag_name());
            self.add_diagnostic(ast::new_diagnostic_chain(diags[0], &diagnostics::X_0_cannot_be_used_as_a_JSX_component, &[&text]));
        }
    }

    // jsx.go:198
    pub(crate) fn infer_jsx_type_arguments(&mut self, node: P<Node>, signature: P<Signature>, check_mode: CheckMode, context: P<InferenceContext>) -> Vec<P<Type>> {
        let param_type = self.get_effective_first_argument_for_jsx_signature(signature, node).unwrap();
        let check_attr_type = self.check_expression_with_contextual_type(node.attributes().unwrap(), param_type, Some(context), check_mode);
        self.infer_types(&context.inferences.get(), check_attr_type, param_type, InferencePriority::None, false);
        self.get_inferred_types(context)
    }

    // jsx.go:205
    pub(crate) fn get_contextual_type_for_jsx_expression(&mut self, node: P<Node>, context_flags: ContextFlags) -> Option<P<Type>> {
        let parent = node.parent().unwrap();
        if ast::is_jsx_attribute_like(parent) {
            return self.get_contextual_type(node, context_flags);
        } else if ast::is_jsx_element(parent) {
            return self.get_contextual_type_for_child_jsx_expression(parent, node, context_flags);
        }
        None
    }

    // jsx.go:215
    pub(crate) fn get_contextual_type_for_jsx_attribute(&mut self, attribute: P<Node>, context_flags: ContextFlags) -> Option<P<Type>> {
        // When we trying to resolve JsxOpeningLikeElement as a stateless function element, we will already give its attributes a contextual type
        // which is a type of the parameter of the signature we are trying out.
        // If there is no contextual type (e.g. we are trying to resolve stateful component), get attributes type from resolving element's tagName
        if ast::is_jsx_attribute(attribute) {
            let attributes_type = self.get_apparent_type_of_contextual_type(attribute.parent().unwrap(), context_flags);
            let Some(attributes_type) = attributes_type else {
                return None;
            };
            if is_type_any(Some(attributes_type)) {
                return None;
            }
            return self.get_type_of_property_of_contextual_type(attributes_type, attribute.name().unwrap().text());
        }
        self.get_contextual_type(attribute.parent().unwrap(), context_flags)
    }

    // jsx.go:229
    pub(crate) fn get_contextual_jsx_element_attributes_type(&mut self, node: P<Node>, context_flags: ContextFlags) -> Option<P<Type>> {
        if ast::is_jsx_opening_element(node) && context_flags != ContextFlags::IgnoreNodeInferences {
            let index = self.find_contextual_node(node.parent().unwrap(), context_flags == ContextFlags::None);
            if index >= 0 {
                // Contextually applied type is moved from attributes up to the outer jsx attributes so when walking up from the children they get hit
                // _However_ to hit them from the _attributes_ we must look for them here; otherwise we'll used the declared type
                // (as below) instead!
                return self.contextual_infos[index as usize].t;
            }
        }
        self.get_contextual_type_for_argument_at_index(node, 0)
    }

    // jsx.go:242
    pub(crate) fn get_contextual_type_for_child_jsx_expression(&mut self, node: P<Node>, child: P<Node>, context_flags: ContextFlags) -> Option<P<Type>> {
        let attributes_type = self.get_apparent_type_of_contextual_type(node.as_jsx_element().opening_element.attributes().unwrap(), context_flags);
        // JSX expression is in children of JSX Element, we will look for an "children" attribute (we get the name from JSX.ElementAttributesProperty)
        let jsx_namespace = self.get_jsx_namespace_at(Some(node));
        let jsx_children_property_name = self.get_jsx_element_children_property_name(jsx_namespace);
        let attributes_type = match attributes_type {
            Some(attributes_type)
                if !is_type_any(Some(attributes_type))
                    && jsx_children_property_name != InternalSymbolNameMissing
                    && !jsx_children_property_name.is_empty() =>
            {
                attributes_type
            }
            _ => return None,
        };
        let real_children = ast::get_semantic_jsx_children(node.children().nodes());
        let child_index = real_children.iter().position(|&c| c == child).map_or(-1, |i| i as i32);
        let child_field_type = self.get_type_of_property_of_contextual_type(attributes_type, &jsx_children_property_name)?;
        if real_children.len() == 1 {
            return Some(child_field_type);
        }
        self.map_type_ex(
            child_field_type,
            |c, t| {
                if c.is_array_like_type(t) {
                    let index_type = c.get_number_literal_type(Number(child_index as f64));
                    return Some(c.get_indexed_access_type(t, index_type));
                }
                Some(t)
            },
            true, /*noReductions*/
        )
    }

    // jsx.go:266
    pub(crate) fn discriminate_contextual_type_by_jsx_attributes(&mut self, node: P<Node>, contextual_type: P<Type>) -> P<Type> {
        let key = DiscriminatedContextualTypeKey { node_id: ast::get_node_id(node), type_id: contextual_type.id };
        if let Some(discriminated) = self.discriminated_contextual_types.get(&key) {
            return *discriminated;
        }
        let jsx_namespace = self.get_jsx_namespace_at(Some(node));
        let jsx_children_property_name = self.get_jsx_element_children_property_name(jsx_namespace);
        let mut discriminant_properties: Vec<P<Node>> = Vec::new();
        for &p in node.properties() {
            let keep = 'keep: {
                let symbol = p.symbol();
                let Some(symbol) = symbol else {
                    break 'keep false;
                };
                if !ast::is_jsx_attribute(p) {
                    break 'keep false;
                }
                let initializer = p.initializer();
                (initializer.is_none() || self.is_possibly_discriminant_value(initializer.unwrap()))
                    && self.is_discriminant_property(Some(contextual_type), symbol.name())
            };
            if keep {
                discriminant_properties.push(p);
            }
        }
        let mut discriminant_members: Vec<P<Symbol>> = Vec::new();
        for s in self.get_properties_of_type(contextual_type).iter().copied() {
            let keep = 'keep: {
                if !s.flags().intersects(SymbolFlags::Optional) || node.symbol().is_none() {
                    break 'keep false;
                }
                let element = node.parent().unwrap().parent().unwrap();
                if s.name() == jsx_children_property_name
                    && ast::is_jsx_element(element)
                    && !ast::get_semantic_jsx_children(element.children().nodes()).is_empty()
                {
                    break 'keep false;
                }
                node.symbol().unwrap().members().and_then(|members| members.lookup(s.name())).is_none()
                    && self.is_discriminant_property(Some(contextual_type), s.name())
            };
            if keep {
                discriminant_members.push(s);
            }
        }
        let mut discriminator = ObjectLiteralDiscriminator { props: discriminant_properties, members: discriminant_members };
        let discriminated = self.discriminate_type_by_discriminable_items(contextual_type, &mut discriminator);
        self.discriminated_contextual_types.insert(key, discriminated);
        discriminated
    }

    // jsx.go:296
    pub(crate) fn elaborate_jsx_components(&mut self, node: P<Node>, source: P<Type>, target: P<Type>, relation: P<Relation>, mut diagnostic_output: Option<&mut Vec<P<Diagnostic>>>) -> bool {
        let mut reported_error = false;
        for &prop in node.properties() {
            if !ast::is_jsx_spread_attribute(prop) && !is_hyphenated_jsx_name(prop.name().unwrap().text()) {
                let name_type = self.get_string_literal_type(prop.name().unwrap().text());
                if !name_type.flags().intersects(TypeFlags::Never) {
                    reported_error = self.elaborate_element(
                        source,
                        target,
                        relation,
                        prop.name().unwrap(),
                        prop.initializer(),
                        name_type,
                        None,
                        None,
                        diagnostic_output.as_deref_mut(),
                    ) || reported_error;
                }
            }
        }
        let parent = node.parent().unwrap();
        if ast::is_jsx_opening_element(parent) && ast::is_jsx_element(parent.parent().unwrap()) {
            let containing_element = parent.parent().unwrap(); // Containing JSXElement
            let jsx_namespace = self.get_jsx_namespace_at(Some(node));
            let mut children_prop_name = self.get_jsx_element_children_property_name(jsx_namespace);
            if children_prop_name == InternalSymbolNameMissing {
                children_prop_name = "children".to_string();
            }
            let children_name_type = self.get_string_literal_type(&children_prop_name);
            let children_target_type = self.get_indexed_access_type(target, children_name_type);
            let valid_children = ast::get_semantic_jsx_children(containing_element.children().nodes());
            if valid_children.is_empty() {
                return reported_error;
            }
            let more_than_one_real_children = valid_children.len() > 1;
            let array_like_target_parts;
            let non_array_like_target_parts;
            let iterable_type = self.get_global_iterable_type();
            if iterable_type != self.empty_generic_type {
                let any_type = self.any_type;
                let any_iterable = self.create_iterable_type(any_type);
                array_like_target_parts = self.filter_type(children_target_type, |c, t| c.is_type_assignable_to(t, any_iterable));
                non_array_like_target_parts = self.filter_type(children_target_type, |c, t| !c.is_type_assignable_to(t, any_iterable));
            } else {
                array_like_target_parts = self.filter_type(children_target_type, |c, t| c.is_array_or_tuple_like_type(t));
                non_array_like_target_parts = self.filter_type(children_target_type, |c, t| !c.is_array_or_tuple_like_type(t));
            }
            let invalid_text_diagnostic: RefCell<Option<(&'static Message, Vec<String>)>> = RefCell::new(None);
            let children_prop_name_for_diagnostic = children_prop_name.clone();
            let get_invalid_textual_child_diagnostic: InvalidTextDiagnosticFn = Rc::new(move |c: &mut Checker| {
                if invalid_text_diagnostic.borrow().is_none() {
                    let tag_name_text = tsrs_scanner::get_text_of_node(node.parent().unwrap().tag_name());
                    let children_target_type_text = c.type_to_string_exported(children_target_type);
                    *invalid_text_diagnostic.borrow_mut() = Some((
                        &diagnostics::X_0_components_don_t_accept_text_as_child_elements_Text_in_JSX_has_the_type_string_but_the_expected_type_of_1_is_2,
                        vec![tag_name_text, children_prop_name_for_diagnostic.clone(), children_target_type_text],
                    ));
                }
                invalid_text_diagnostic.borrow().clone().unwrap()
            });
            if more_than_one_real_children {
                if array_like_target_parts != self.never_type {
                    let child_types = self.check_jsx_children(containing_element, CheckMode::Normal);
                    let real_source = self.create_tuple_type(&child_types);
                    let children = self.generate_jsx_children(containing_element, Rc::clone(&get_invalid_textual_child_diagnostic));
                    reported_error = self.elaborate_iterable_or_array_like_target_elementwise(children, real_source, array_like_target_parts, relation, diagnostic_output.as_deref_mut())
                        || reported_error;
                } else if {
                    let source_children_type = self.get_indexed_access_type(source, children_name_type);
                    !self.is_type_related_to(source_children_type, children_target_type, relation)
                } {
                    // arity mismatch
                    let children_target_type_text = self.type_to_string_exported(children_target_type);
                    let diag = self.error(
                        Some(containing_element.as_jsx_element().opening_element.tag_name()),
                        &diagnostics::This_JSX_tag_s_0_prop_expects_a_single_child_of_type_1_but_multiple_children_were_provided,
                        &[&children_prop_name, &children_target_type_text],
                    );
                    self.report_diagnostic(Some(diag), diagnostic_output.as_deref_mut());
                    reported_error = true;
                }
            } else if non_array_like_target_parts != self.never_type {
                let child = valid_children[0];
                let e = self.get_elaboration_element_for_jsx_child(child, children_name_type, Rc::clone(&get_invalid_textual_child_diagnostic));
                if let Some(error_node) = e.error_node {
                    let mut create_diagnostic = e.create_diagnostic.clone().map(|f| move |c: &mut Checker, prop: P<Node>| f(c, prop));
                    reported_error = self.elaborate_element(
                        source,
                        target,
                        relation,
                        error_node,
                        e.inner_expression,
                        e.name_type.unwrap(),
                        None,
                        create_diagnostic.as_mut().map(|f| f as &mut dyn FnMut(&mut Checker, P<Node>) -> P<Diagnostic>),
                        diagnostic_output.as_deref_mut(),
                    ) || reported_error;
                }
            } else if {
                let source_children_type = self.get_indexed_access_type(source, children_name_type);
                !self.is_type_related_to(source_children_type, children_target_type, relation)
            } {
                // arity mismatch
                let children_target_type_text = self.type_to_string_exported(children_target_type);
                let diag = self.error(
                    Some(containing_element.as_jsx_element().opening_element.tag_name()),
                    &diagnostics::This_JSX_tag_s_0_prop_expects_type_1_which_requires_multiple_children_but_only_a_single_child_was_provided,
                    &[&children_prop_name, &children_target_type_text],
                );
                self.report_diagnostic(Some(diag), diagnostic_output.as_deref_mut());
                reported_error = true;
            }
        }
        reported_error
    }

    // jsx.go:376
    // Go returns a lazy iter.Seq; the port returns the equivalent pull function (`None` = exhausted) so that
    // element creation interleaves with the consumer's work exactly as in Go.
    pub(crate) fn generate_jsx_children(&mut self, node: P<Node>, get_invalid_text_diagnostic: InvalidTextDiagnosticFn) -> impl FnMut(&mut Checker) -> Option<JsxElaborationElement> + 'static + use<> {
        let children = node.children().nodes();
        let mut i = 0usize;
        let mut member_offset = 0usize;
        move |c: &mut Checker| {
            while i < children.len() {
                let child = children[i];
                let name_type = c.get_number_literal_type(Number((i - member_offset) as f64));
                i += 1;
                let e = c.get_elaboration_element_for_jsx_child(child, name_type, Rc::clone(&get_invalid_text_diagnostic));
                if e.error_node.is_some() {
                    return Some(e);
                } else {
                    member_offset += 1;
                }
            }
            None
        }
    }

    // jsx.go:393
    pub(crate) fn get_elaboration_element_for_jsx_child(&mut self, child: P<Node>, name_type: P<Type>, get_invalid_text_diagnostic: InvalidTextDiagnosticFn) -> JsxElaborationElement {
        match child.kind() {
            Kind::JsxExpression => {
                // child is of the type of the expression
                JsxElaborationElement { error_node: Some(child), inner_expression: child.expression(), name_type: Some(name_type), create_diagnostic: None }
            }
            Kind::JsxText => {
                if child.as_jsx_text().contains_only_trivia_white_spaces {
                    // Whitespace only jsx text isn't real jsx text
                    return JsxElaborationElement::default();
                }
                // child is a string
                JsxElaborationElement {
                    error_node: Some(child),
                    inner_expression: None,
                    name_type: Some(name_type),
                    create_diagnostic: Some(Rc::new(move |c: &mut Checker, prop: P<Node>| {
                        let (error_message, error_args) = get_invalid_text_diagnostic(c);
                        let args: Vec<&dyn Display> = error_args.iter().map(|a| a as &dyn Display).collect();
                        new_diagnostic_for_node(Some(prop), Some(error_message), &args)
                    })),
                }
            }
            Kind::JsxElement | Kind::JsxSelfClosingElement | Kind::JsxFragment => {
                // child is of type JSX.Element
                JsxElaborationElement { error_node: Some(child), inner_expression: Some(child), name_type: Some(name_type), create_diagnostic: None }
            }
            _ => panic!("Unhandled case in getElaborationElementForJsxChild"),
        }
    }

    // jsx.go:420
    pub(crate) fn elaborate_iterable_or_array_like_target_elementwise(
        &mut self,
        mut iterator: impl FnMut(&mut Checker) -> Option<JsxElaborationElement>,
        source: P<Type>,
        target: P<Type>,
        relation: P<Relation>,
        mut diagnostic_output: Option<&mut Vec<P<Diagnostic>>>,
    ) -> bool {
        let tuple_or_array_like_target_parts = self.filter_type(target, |c, t| c.is_array_or_tuple_like_type(t));
        let non_tuple_or_array_like_target_parts = self.filter_type(target, |c, t| !c.is_array_or_tuple_like_type(t));
        // If `nonTupleOrArrayLikeTargetParts` is not `never`, then that should mean `Iterable` is defined.
        let mut iteration_type: Option<P<Type>> = None;
        if non_tuple_or_array_like_target_parts != self.never_type {
            iteration_type = self.get_iteration_type_of_iterable(IterationUse::ForOf, IterationTypeKind::Yield, non_tuple_or_array_like_target_parts, None /*errorNode*/);
        }
        let mut reported_error = false;
        while let Some(e) = iterator(self) {
            let prop = e.error_node.unwrap();
            let next = e.inner_expression;
            let name_type = e.name_type.unwrap();
            let mut target_prop_type = iteration_type;
            let mut target_indexed_prop_type: Option<P<Type>> = None;
            if tuple_or_array_like_target_parts != self.never_type {
                target_indexed_prop_type = self.get_best_match_indexed_access_type_or_undefined(source, tuple_or_array_like_target_parts, name_type);
            }
            if let Some(target_indexed_prop_type) = target_indexed_prop_type {
                if !target_indexed_prop_type.flags().intersects(TypeFlags::IndexedAccess) {
                    if let Some(iteration_type) = iteration_type {
                        target_prop_type = Some(self.get_union_type(&[iteration_type, target_indexed_prop_type]));
                    } else {
                        target_prop_type = Some(target_indexed_prop_type);
                    }
                }
            }
            let Some(mut target_prop_type) = target_prop_type else {
                continue;
            };
            let source_prop_type = self.get_indexed_access_type_or_undefined(source, name_type, AccessFlags::None, None, AliasArg::None);
            let Some(mut source_prop_type) = source_prop_type else {
                continue;
            };
            let prop_name = self.get_property_name_from_index(name_type, None /*accessNode*/);
            if !self.check_type_related_to(source_prop_type, target_prop_type, relation, None /*errorNode*/) {
                let elaborated = next.is_some()
                    && self.elaborate_error(next, source_prop_type, target_prop_type, relation, None /*headMessage*/, diagnostic_output.as_deref_mut());
                reported_error = true;
                if !elaborated {
                    // Issue error on the prop itself, since the prop couldn't elaborate the error. Use the expression type, if available.
                    let mut specific_source = source_prop_type;
                    if let Some(next) = next {
                        specific_source = self.check_expression_for_mutable_location_with_contextual_type(next, source_prop_type);
                    }
                    if let Some(create_diagnostic) = &e.create_diagnostic {
                        // Use the custom diagnostic factory if provided (e.g., for JSX text children with dynamic error messages)
                        let diag = create_diagnostic(self, prop);
                        self.report_diagnostic(Some(diag), diagnostic_output.as_deref_mut());
                    } else if self.exact_optional_property_types && self.is_exact_optional_property_mismatch(Some(specific_source), Some(target_prop_type)) {
                        let specific_source_text = self.type_to_string_exported(specific_source);
                        let target_prop_type_text = self.type_to_string_exported(target_prop_type);
                        let diag = create_diagnostic_for_node(
                            Some(prop),
                            &diagnostics::Type_0_is_not_assignable_to_type_1_with_exactOptionalPropertyTypes_Colon_true_Consider_adding_undefined_to_the_type_of_the_target,
                            &[&specific_source_text, &target_prop_type_text],
                        );
                        self.report_diagnostic(Some(diag), diagnostic_output.as_deref_mut());
                    } else {
                        let target_is_optional = prop_name != InternalSymbolNameMissing
                            && self
                                .get_property_of_type(tuple_or_array_like_target_parts, &prop_name)
                                .unwrap_or(self.unknown_symbol)
                                .flags()
                                .intersects(SymbolFlags::Optional);
                        let source_is_optional = prop_name != InternalSymbolNameMissing
                            && self.get_property_of_type(source, &prop_name).unwrap_or(self.unknown_symbol).flags().intersects(SymbolFlags::Optional);
                        target_prop_type = self.remove_missing_type(target_prop_type, target_is_optional);
                        source_prop_type = self.remove_missing_type(source_prop_type, target_is_optional && source_is_optional);
                        let result = self.check_type_related_to_ex(specific_source, target_prop_type, relation, Some(prop), None, diagnostic_output.as_deref_mut());
                        if result && specific_source != source_prop_type {
                            // If for whatever reason the expression type doesn't yield an error, make sure we still issue an error on the sourcePropType
                            self.check_type_related_to_ex(source_prop_type, target_prop_type, relation, Some(prop), None, diagnostic_output.as_deref_mut());
                        }
                    }
                }
            }
        }
        reported_error
    }

    // jsx.go:485
    pub(crate) fn get_suggested_symbol_for_nonexistent_jsx_attribute(&mut self, name: &str, containing_type: P<Type>) -> Option<P<Symbol>> {
        let properties = self.get_properties_of_type(containing_type);
        let mut jsx_specific: Option<P<Symbol>> = None;
        match name {
            "for" => {
                jsx_specific = properties.iter().copied().find(|&x| ast::symbol_name(x) == "htmlFor");
            }
            "class" => {
                jsx_specific = properties.iter().copied().find(|&x| ast::symbol_name(x) == "className");
            }
            _ => {}
        }
        if jsx_specific.is_some() {
            return jsx_specific;
        }
        self.get_spelling_suggestion_for_name(name, &properties, SymbolFlags::Value)
    }

    // jsx.go:500
    pub(crate) fn get_jsx_fragment_type(&mut self, node: P<Node>) -> P<Type> {
        // An opening fragment is required in order for `getJsxNamespace` to give the fragment factory
        let links = self.source_file_links.get_key(ast::get_source_file_of_node(node).unwrap());
        if let Some(jsx_fragment_type) = self.source_file_links.at(links).jsx_fragment_type.get() {
            return jsx_fragment_type;
        }
        let jsx_fragment_factory_name = self.get_jsx_namespace(Some(node));
        // #38720/60122, allow null as jsxFragmentFactory
        let should_resolve_factory_reference = (self.compiler_options.jsx == JsxEmit::React || !self.compiler_options.jsx_fragment_factory.is_empty())
            && jsx_fragment_factory_name != "null";
        if !should_resolve_factory_reference {
            self.source_file_links.at(links).jsx_fragment_type.set(Some(self.any_type));
            return self.any_type;
        }
        let mut jsx_factory_symbol = self.get_jsx_namespace_container_for_implicit_import(node);
        if jsx_factory_symbol.is_none() {
            let should_module_ref_err = self.compiler_options.jsx != JsxEmit::Preserve && self.compiler_options.jsx != JsxEmit::ReactNative;
            let mut flags = SymbolFlags::Value;
            if !should_module_ref_err {
                flags &= !SymbolFlags::Enum;
            }
            jsx_factory_symbol = self.resolve_name(
                Some(node),
                &jsx_fragment_factory_name,
                flags,
                Some(&diagnostics::Using_JSX_fragments_requires_fragment_factory_0_to_be_in_scope_but_it_could_not_be_found),
                true,  /*isUse*/
                false, /*excludeGlobals*/
            );
        }
        let Some(jsx_factory_symbol) = jsx_factory_symbol else {
            self.source_file_links.at(links).jsx_fragment_type.set(Some(self.error_type));
            return self.error_type;
        };
        if jsx_factory_symbol.name() == ReactNames.fragment {
            let t = self.get_type_of_symbol(jsx_factory_symbol);
            self.source_file_links.at(links).jsx_fragment_type.set(Some(t));
            return t;
        }
        let mut resolved_alias = jsx_factory_symbol;
        if jsx_factory_symbol.flags().intersects(SymbolFlags::Alias) {
            resolved_alias = self.resolve_alias(jsx_factory_symbol);
        }

        let react_exports = self.get_exports_of_symbol(resolved_alias);
        let type_symbol = self.get_symbol(react_exports, ReactNames.fragment, SymbolFlags::BlockScopedVariable);
        if let Some(type_symbol) = type_symbol {
            let t = self.get_type_of_symbol(type_symbol);
            self.source_file_links.at(links).jsx_fragment_type.set(Some(t));
        } else {
            self.source_file_links.at(links).jsx_fragment_type.set(Some(self.error_type));
        }
        self.source_file_links.at(links).jsx_fragment_type.get().unwrap()
    }

    // jsx.go:545
    pub(crate) fn resolve_jsx_opening_like_element(&mut self, node: P<Node>, candidates_out_array: Option<&mut Vec<P<Signature>>>, check_mode: CheckMode) -> P<Signature> {
        let is_jsx_open_fragment = ast::is_jsx_opening_fragment(node);
        let expr_types;
        if !is_jsx_open_fragment {
            if is_jsx_intrinsic_tag_name(node.tag_name()) {
                let result = self.get_intrinsic_attributes_type_from_jsx_opening_like_element(node).unwrap();
                let fake_signature = self.create_signature_for_jsx_intrinsic(node, result);
                let attributes = node.attributes().unwrap();
                let contextual_type = self.get_effective_first_argument_for_jsx_signature(fake_signature, node).unwrap();
                let attributes_type = self.check_expression_with_contextual_type(attributes, contextual_type, None /*inferenceContext*/, CheckMode::Normal);
                self.check_type_assignable_to_and_optionally_elaborate(attributes_type, result, Some(node.tag_name()), Some(attributes), None, None);
                let type_arguments = node.type_arguments();
                if !type_arguments.is_empty() {
                    self.check_source_elements(type_arguments);
                    let source_file = ast::get_source_file_of_node(node).unwrap();
                    let type_argument_list = node.type_argument_list().unwrap();
                    let loc = TextRange::new(tsrs_scanner::skip_trivia(source_file.text(), type_argument_list.pos()), type_argument_list.end());
                    self.add_diagnostic(ast::new_diagnostic(
                        Some(source_file),
                        loc,
                        &diagnostics::Expected_0_type_arguments_but_got_1,
                        &[&0, &type_arguments.len()],
                    ));
                }
                return fake_signature;
            }
            expr_types = self.check_expression(node.tag_name());
        } else {
            expr_types = self.get_jsx_fragment_type(node);
        }
        let apparent_type = self.get_apparent_type(expr_types);
        if self.is_error_type(apparent_type) {
            return self.resolve_error_call(node);
        }
        let signatures = self.get_uninstantiated_jsx_signatures_of_type(expr_types, node);
        if self.is_untyped_function_call(expr_types, apparent_type, signatures.len() as i32, 0 /*constructSignatures*/) {
            return self.resolve_untyped_call(node);
        }
        if signatures.is_empty() {
            // We found no signatures at all, which is an error
            if is_jsx_open_fragment {
                let text = tsrs_scanner::get_text_of_node(node);
                self.error(Some(node), &diagnostics::JSX_element_type_0_does_not_have_any_construct_or_call_signatures, &[&text]);
            } else {
                let text = tsrs_scanner::get_text_of_node(node.tag_name());
                self.error(Some(node.tag_name()), &diagnostics::JSX_element_type_0_does_not_have_any_construct_or_call_signatures, &[&text]);
            }
            return self.resolve_error_call(node);
        }
        self.resolve_call(node, &signatures, candidates_out_array, check_mode, SignatureFlags::None, None)
    }

    // jsx.go:591
    // Check if the given signature can possibly be a signature called by the JSX opening-like element.
    // @param node a JSX opening-like element we are trying to figure its call signature
    // @param signature a candidate signature we are trying whether it is a call signature
    // @param relation a relationship to check parameter and argument type
    pub(crate) fn check_applicable_signature_for_jsx_call_like_element(&mut self, node: P<Node>, signature: P<Signature>, relation: P<Relation>, check_mode: CheckMode, report_errors: bool, mut diagnostic_output: Option<&mut Vec<P<Diagnostic>>>) -> bool {
        // Stateless function components can have maximum of three arguments: "props", "context", and "updater".
        // However "context" and "updater" are implicit and can't be specify by users. Only the first parameter, props,
        // can be specified by users through attributes property.
        let param_type = self.get_effective_first_argument_for_jsx_signature(signature, node).unwrap();
        let attributes_type = if ast::is_jsx_opening_fragment(node) {
            self.create_jsx_attributes_type_from_attributes_property(node, CheckMode::Normal)
        } else {
            self.check_expression_with_contextual_type(node.attributes().unwrap(), param_type, None /*inferenceContext*/, check_mode)
        };
        let check_attributes_type = if check_mode.intersects(CheckMode::SkipContextSensitive) {
            self.get_regular_type_of_object_literal(attributes_type)
        } else {
            attributes_type
        };
        if !check_tag_name_does_not_expect_too_many_arguments(self, node, report_errors, diagnostic_output.as_deref_mut()) {
            return false;
        }
        let mut error_node: Option<P<Node>> = None;
        if report_errors {
            if ast::is_jsx_opening_fragment(node) {
                error_node = Some(node);
            } else {
                error_node = Some(node.tag_name());
            }
        }
        let mut attributes: Option<P<Node>> = None;
        if !ast::is_jsx_opening_fragment(node) {
            attributes = node.attributes();
        }
        self.check_type_related_to_and_optionally_elaborate(check_attributes_type, param_type, relation, error_node, attributes, None, diagnostic_output)
    }

    // jsx.go:710
    // Get attributes type of the JSX opening-like element. The result is from resolving "attributes" property of the opening-like element.
    //
    // @param openingLikeElement a JSX opening-like element
    // @param filter a function to remove attributes that will not participate in checking whether attributes are assignable
    // @return an anonymous type (similar to the one returned by checkObjectLiteral) in which its properties are attributes property.
    // @remarks Because this function calls getSpreadType, it needs to use the same checks as checkObjectLiteral,
    // which also calls getSpreadType.
    pub(crate) fn create_jsx_attributes_type_from_attributes_property(&mut self, opening_like_element: P<Node>, check_mode: CheckMode) -> P<Type> {
        // Local to this call: owned here, not allocated in the arena (as `checkObjectLiteral`'s table).
        let all_attributes_table: Option<SymbolTable> = self.strict_null_checks.then(SymbolTable::default);
        let mut attributes_table = SymbolTable::new();
        let mut attributes_symbol: Option<P<Symbol>> = None;
        let mut attribute_parent = opening_like_element;
        let mut spread = self.empty_jsx_object_type;
        let mut has_spread_any_type = false;
        let mut type_to_intersect: Option<P<Type>> = None;
        let mut explicitly_specify_children_attribute = false;
        let mut object_flags = ObjectFlags::JsxAttributes;
        fn create_jsx_attributes_type(c: &mut Checker, object_flags: &mut ObjectFlags, attributes_symbol: Option<P<Symbol>>, attributes_table: P<SymbolTable>) -> P<Type> {
            *object_flags |= ObjectFlags::FreshLiteral;
            let result = c.new_anonymous_type(attributes_symbol, Some(attributes_table), &[], &[], &[]);
            result.object_flags.set(result.object_flags.get() | *object_flags | ObjectFlags::ObjectLiteral | ObjectFlags::ContainsObjectOrArrayLiteral);
            result
        }
        let jsx_namespace = self.get_jsx_namespace_at(Some(opening_like_element));
        let jsx_children_property_name = self.get_jsx_element_children_property_name(jsx_namespace);
        let is_jsx_open_fragment = ast::is_jsx_opening_fragment(opening_like_element);
        if !is_jsx_open_fragment {
            let attributes = opening_like_element.attributes().unwrap();
            attributes_symbol = attributes.symbol();
            attribute_parent = attributes;
            let contextual_type = self.get_contextual_type(attributes, ContextFlags::None);
            // Create anonymous type from given attributes symbol table.
            // @param symbol a symbol of JsxAttributes containing attributes corresponding to attributesTable
            // @param attributesTable a symbol table of attributes property
            for &attribute_decl in attributes.properties() {
                let member = attribute_decl.symbol();
                if ast::is_jsx_attribute(attribute_decl) {
                    let member = member.unwrap();
                    let expr_type = self.check_jsx_attribute(attribute_decl, check_mode);
                    object_flags |= expr_type.object_flags() & ObjectFlags::PropagatingFlags;
                    let attribute_symbol = self.new_symbol(SymbolFlags::Property | member.flags(), member.name());
                    attribute_symbol.set_declarations_static(member.declarations());
                    attribute_symbol.set_parent(member.parent());
                    if member.value_declaration().is_some() {
                        attribute_symbol.set_value_declaration(member.value_declaration());
                    }
                    let links = self.value_symbol_links.get_key(attribute_symbol);
                    self.value_symbol_links.at(links).resolved_type.set(Some(expr_type));
                    self.value_symbol_links.at(links).set_target(Some(member));
                    attributes_table.set(attribute_symbol.name(), attribute_symbol);
                    if let Some(all_attributes_table) = &all_attributes_table {
                        all_attributes_table.set(attribute_symbol.name(), attribute_symbol);
                    }
                    if attribute_decl.name().unwrap().text() == jsx_children_property_name {
                        explicitly_specify_children_attribute = true;
                    }
                    if contextual_type.is_some()
                        && check_mode.intersects(CheckMode::Inferential)
                        && !check_mode.intersects(CheckMode::SkipContextSensitive)
                        && self.is_context_sensitive(attribute_decl)
                    {
                        let inference_context = self.get_inference_context(attributes);
                        assert!(inference_context.is_some());
                        // In CheckMode.Inferential we should always have an inference context
                        let inference_node = attribute_decl.initializer().unwrap().expression().unwrap();
                        self.add_intra_expression_inference_site(inference_context.unwrap(), inference_node, expr_type);
                    }
                } else {
                    assert!(attribute_decl.kind() == Kind::JsxSpreadAttribute);
                    if attributes_table.len() != 0 {
                        let t = create_jsx_attributes_type(self, &mut object_flags, attributes_symbol, attributes_table);
                        spread = self.get_spread_type(spread, t, attributes_symbol, object_flags, false /*readonly*/);
                        attributes_table = SymbolTable::new();
                    }
                    let checked = self.check_expression_ex(attribute_decl.expression().unwrap(), check_mode & CheckMode::Inferential);
                    let expr_type = self.get_reduced_type(checked);
                    if is_type_any(Some(expr_type)) {
                        has_spread_any_type = true;
                    }
                    if self.is_valid_spread_type(expr_type) {
                        spread = self.get_spread_type(spread, expr_type, attributes_symbol, object_flags, false /*readonly*/);
                        if all_attributes_table.is_some() {
                            self.check_spread_prop_overrides(expr_type, all_attributes_table.as_ref(), attribute_decl);
                        }
                    } else {
                        self.error(attribute_decl.expression(), &diagnostics::Spread_types_may_only_be_created_from_object_types, &[]);
                        if let Some(t) = type_to_intersect {
                            type_to_intersect = Some(self.get_intersection_type(&[t, expr_type]));
                        } else {
                            type_to_intersect = Some(expr_type);
                        }
                    }
                }
            }
            if !has_spread_any_type {
                if attributes_table.len() != 0 {
                    let t = create_jsx_attributes_type(self, &mut object_flags, attributes_symbol, attributes_table);
                    spread = self.get_spread_type(spread, t, attributes_symbol, object_flags, false /*readonly*/);
                }
            }
        }
        fn parent_has_semantic_jsx_children(opening_like_element: P<Node>) -> bool {
            // Handle children attribute
            let Some(parent) = opening_like_element.parent() else {
                return false;
            };
            let mut children: &[P<Node>] = &[];

            if ast::is_jsx_element(parent) {
                // We have to check that openingElement of the parent is the one we are visiting as this may not be true for selfClosingElement
                if parent.as_jsx_element().opening_element == opening_like_element {
                    children = parent.children().nodes();
                }
            } else if ast::is_jsx_fragment(parent) {
                if parent.as_jsx_fragment().opening_fragment == opening_like_element {
                    children = parent.children().nodes();
                }
            }
            !ast::get_semantic_jsx_children(children).is_empty()
        }
        if parent_has_semantic_jsx_children(opening_like_element) {
            let child_types: Vec<P<Type>> = self.check_jsx_children(opening_like_element.parent().unwrap(), check_mode);
            if !has_spread_any_type && jsx_children_property_name != InternalSymbolNameMissing && !jsx_children_property_name.is_empty() {
                // Error if there is a attribute named "children" explicitly specified and children element.
                // This is because children element will overwrite the value from attributes.
                // Note: we will not warn "children" attribute overwritten if "children" attribute is specified in object spread.
                if explicitly_specify_children_attribute {
                    self.error(Some(attribute_parent), &diagnostics::X_0_are_specified_twice_The_attribute_named_0_will_be_overwritten, &[&jsx_children_property_name]);
                }
                let mut children_contextual_type: Option<P<Type>> = None;
                if ast::is_jsx_opening_element(opening_like_element) {
                    if let Some(contextual_type) = self.get_apparent_type_of_contextual_type(opening_like_element.attributes().unwrap(), ContextFlags::None) {
                        children_contextual_type = self.get_type_of_property_of_contextual_type(contextual_type, &jsx_children_property_name);
                    }
                }
                // If there are children in the body of JSX element, create dummy attribute "children" with the union of children types so that it will pass the attribute checking process
                let children_prop_symbol = self.new_symbol(SymbolFlags::Property, &jsx_children_property_name);
                let links = self.value_symbol_links.get_key(children_prop_symbol);
                if child_types.len() == 1 {
                    self.value_symbol_links.at(links).resolved_type.set(Some(child_types[0]));
                } else if children_contextual_type.is_some() && some_type(self, children_contextual_type.unwrap(), |c, t| c.is_tuple_like_type(t)) {
                    let t = self.create_tuple_type(&child_types);
                    self.value_symbol_links.at(links).resolved_type.set(Some(t));
                } else {
                    let union = self.get_union_type(&child_types);
                    let t = self.create_array_type(union);
                    self.value_symbol_links.at(links).resolved_type.set(Some(t));
                }
                // Fake up a property declaration for the children
                let name = self.factory.new_identifier(&jsx_children_property_name);
                let value_declaration = self.factory.new_property_signature_declaration(None, name, None /*postfixToken*/, None /*type*/, None /*initializer*/);
                children_prop_symbol.set_value_declaration(Some(value_declaration));
                value_declaration.set_parent(Some(attribute_parent));
                value_declaration.as_property_signature_declaration().declaration_base.set_symbol(Some(children_prop_symbol));
                let child_prop_map = SymbolTable::new();
                child_prop_map.set(children_prop_symbol.name(), children_prop_symbol);
                let children_type = self.new_anonymous_type(attributes_symbol, Some(child_prop_map), &[], &[], &[]);
                let propagating_flags = self.get_propagating_flags_of_types(&child_types, TypeFlags::None);
                spread = self.get_spread_type(spread, children_type, attributes_symbol, object_flags | propagating_flags, false /*readonly*/);
            }
        }
        if has_spread_any_type {
            return self.any_type;
        }
        if let Some(type_to_intersect) = type_to_intersect {
            if spread != self.empty_jsx_object_type {
                return self.get_intersection_type(&[type_to_intersect, spread]);
            }
            return type_to_intersect;
        }
        if spread == self.empty_jsx_object_type {
            return create_jsx_attributes_type(self, &mut object_flags, attributes_symbol, attributes_table);
        }
        spread
    }

    // jsx.go:869
    pub(crate) fn check_jsx_attribute(&mut self, node: P<Node>, check_mode: CheckMode) -> P<Type> {
        if let Some(initializer) = node.initializer() {
            return self.check_expression_for_mutable_location(initializer, check_mode);
        }
        // <Elem attr /> is sugar for <Elem attr={true} />
        self.true_type
    }

    // jsx.go:877
    pub(crate) fn check_jsx_children(&mut self, node: P<Node>, check_mode: CheckMode) -> Vec<P<Type>> {
        let mut child_types: Vec<P<Type>> = Vec::new();
        for &child in node.children().nodes() {
            // In React, JSX text that contains only whitespaces will be ignored so we don't want to type-check that
            // because then type of children property will have constituent of string type.
            if ast::is_jsx_text(child) {
                if !child.as_jsx_text().contains_only_trivia_white_spaces {
                    child_types.push(self.string_type);
                }
            } else if ast::is_jsx_expression(child) && child.expression().is_none() {
                // empty jsx expressions don't *really* count as present children
                continue;
            } else {
                child_types.push(self.check_expression_for_mutable_location(child, check_mode));
            }
        }
        child_types
    }

    // jsx.go:896
    pub(crate) fn get_uninstantiated_jsx_signatures_of_type(&mut self, element_type: P<Type>, caller: P<Node>) -> Vec<P<Signature>> {
        if element_type.flags().intersects(TypeFlags::String) {
            return vec![self.any_signature];
        }
        if element_type.flags().intersects(TypeFlags::StringLiteral) {
            let intrinsic_type = self.get_intrinsic_attributes_type_from_string_literal_type(element_type, caller);
            let Some(intrinsic_type) = intrinsic_type else {
                let value = get_string_literal_value(element_type);
                let type_name = format!("JSX.{}", JsxNames.intrinsic_elements);
                self.error(Some(caller), &diagnostics::Property_0_does_not_exist_on_type_1, &[&value, &type_name]);
                return Vec::new();
            };
            let fake_signature = self.create_signature_for_jsx_intrinsic(caller, intrinsic_type);
            return vec![fake_signature];
        }
        let apparent_elem_type = self.get_apparent_type(element_type);
        // Resolve the signatures, preferring constructor
        let mut signatures = self.get_signatures_of_type(apparent_elem_type, SignatureKind::Construct).to_vec();
        if signatures.is_empty() {
            // No construct signatures, try call signatures
            signatures = self.get_signatures_of_type(apparent_elem_type, SignatureKind::Call).to_vec();
        }
        if signatures.is_empty() && apparent_elem_type.flags().intersects(TypeFlags::Union) {
            // If each member has some combination of new/call signatures; make a union signature list for those
            let signature_lists: Vec<Vec<P<Signature>>> =
                apparent_elem_type.types().iter().map(|&t| self.get_uninstantiated_jsx_signatures_of_type(t, caller)).collect();
            signatures = self.get_union_signatures(&signature_lists);
        }
        signatures
    }

    // jsx.go:925
    pub(crate) fn get_effective_first_argument_for_jsx_signature(&mut self, signature: P<Signature>, node: P<Node>) -> Option<P<Type>> {
        if ast::is_jsx_opening_fragment(node) || self.get_jsx_reference_kind(node) != JsxReferenceKind::Component {
            return self.get_jsx_props_type_from_call_signature(signature, node);
        }
        self.get_jsx_props_type_from_class_type(signature, node)
    }

    // jsx.go:932
    pub(crate) fn get_jsx_props_type_from_call_signature(&mut self, sig: P<Signature>, context: P<Node>) -> Option<P<Type>> {
        let unknown_type = self.unknown_type;
        let mut props_type = self.get_type_of_first_parameter_of_signature_with_fallback(sig, unknown_type);
        let jsx_namespace = self.get_jsx_namespace_at(Some(context));
        props_type = self.get_jsx_managed_attributes_from_located_attributes(context, jsx_namespace, props_type);
        let intrinsic_attribs = self.get_jsx_type(JsxNames.intrinsic_attributes, context);
        if !self.is_error_type(intrinsic_attribs) {
            props_type = self.intersect_types(Some(intrinsic_attribs), Some(props_type)).unwrap();
        }
        Some(props_type)
    }

    // jsx.go:942
    pub(crate) fn get_jsx_props_type_from_class_type(&mut self, sig: P<Signature>, context: P<Node>) -> Option<P<Type>> {
        let ns = self.get_jsx_namespace_at(Some(context));
        let forced_lookup_location = self.get_jsx_element_properties_name(ns);
        let attributes_type: Option<P<Type>>;
        if forced_lookup_location == InternalSymbolNameMissing {
            let unknown_type = self.unknown_type;
            attributes_type = Some(self.get_type_of_first_parameter_of_signature_with_fallback(sig, unknown_type));
        } else if forced_lookup_location.is_empty() {
            attributes_type = Some(self.get_return_type_of_signature(sig));
        } else {
            attributes_type = self.get_jsx_props_type_for_signature_from_member(sig, &forced_lookup_location);
            if attributes_type.is_none() && !context.attributes().unwrap().properties().is_empty() {
                // There is no property named 'props' on this instance type
                self.error(Some(context), &diagnostics::JSX_element_class_does_not_support_attributes_because_it_does_not_have_a_0_property, &[&forced_lookup_location]);
            }
        }
        let Some(located_attributes_type) = attributes_type else {
            return Some(self.unknown_type);
        };
        let attributes_type = self.get_jsx_managed_attributes_from_located_attributes(context, ns, located_attributes_type);
        if is_type_any(Some(attributes_type)) {
            // Props is of type 'any' or unknown
            return Some(attributes_type);
        }
        // Normal case -- add in IntrinsicClassAttributes<T> and IntrinsicAttributes
        let mut apparent_attributes_type = attributes_type;
        let intrinsic_class_attribs = self.get_jsx_type(JsxNames.intrinsic_class_attributes, context);
        if !self.is_error_type(intrinsic_class_attribs) {
            let type_params = self.get_local_type_parameters_of_class_or_interface_or_type_alias(intrinsic_class_attribs.symbol().unwrap());
            let host_class_type = self.get_return_type_of_signature(sig);
            let library_managed_attribute_type;
            if !type_params.is_empty() {
                // apply JSX.IntrinsicClassAttributes<hostClassType, ...>
                let min_type_argument_count = self.get_min_type_argument_count(&type_params);
                let inferred_args = self.fill_missing_type_arguments(&[host_class_type], &type_params, min_type_argument_count, ast::is_in_js_file(context));
                let mapper = new_type_mapper(&type_params, &inferred_args);
                library_managed_attribute_type = self.instantiate_type(intrinsic_class_attribs, Some(mapper));
            } else {
                library_managed_attribute_type = intrinsic_class_attribs;
            }
            apparent_attributes_type = self.intersect_types(Some(library_managed_attribute_type), Some(apparent_attributes_type)).unwrap();
        }
        let intrinsic_attribs = self.get_jsx_type(JsxNames.intrinsic_attributes, context);
        if !self.is_error_type(intrinsic_attribs) {
            apparent_attributes_type = self.intersect_types(Some(intrinsic_attribs), Some(apparent_attributes_type)).unwrap();
        }
        Some(apparent_attributes_type)
    }

    // jsx.go:989
    pub(crate) fn get_jsx_props_type_for_signature_from_member(&mut self, sig: P<Signature>, forced_lookup_location: &str) -> Option<P<Type>> {
        if let Some(composite) = sig.composite() {
            // JSX Elements using the legacy `props`-field based lookup (eg, react class components) need to treat the `props` member as an input
            // instead of an output position when resolving the signature. We need to go back to the input signatures of the composite signature,
            // get the type of `props` on each return type individually, and then _intersect them_, rather than union them (as would normally occur
            // for a union signature). It's an unfortunate quirk of looking in the output of the signature for the type we want to use for the input.
            // The default behavior of `getTypeOfFirstParameterOfSignatureWithFallback` when no `props` member name is defined is much more sane.
            let mut results: Vec<P<Type>> = Vec::new();
            for signature in composite.signatures.get() {
                let instance = self.get_return_type_of_signature(signature);
                if is_type_any(Some(instance)) {
                    return Some(instance);
                }
                let prop_type = self.get_type_of_property_of_type(instance, forced_lookup_location)?;
                results.push(prop_type);
            }
            return Some(self.get_intersection_type(&results));
            // Same result for both union and intersection signatures
        }
        let instance_type = self.get_return_type_of_signature(sig);
        if is_type_any(Some(instance_type)) {
            return Some(instance_type);
        }
        self.get_type_of_property_of_type(instance_type, forced_lookup_location)
    }

    // jsx.go:1018
    // ns is nil-able (getJsxNamespaceAt can return nil and getJsxLibraryManagedAttributes checks it).
    pub(crate) fn get_jsx_managed_attributes_from_located_attributes(&mut self, context: P<Node>, ns: Option<P<Symbol>>, attributes_type: P<Type>) -> P<Type> {
        let managed_sym = self.get_jsx_library_managed_attributes(ns);
        if let Some(managed_sym) = managed_sym {
            let ctor_type = self.get_static_type_of_referenced_jsx_constructor(context);
            let result = self.instantiate_alias_or_interface_with_defaults(managed_sym, &[ctor_type, attributes_type], ast::is_in_js_file(context));
            if let Some(result) = result {
                return result;
            }
        }
        attributes_type
    }

    // jsx.go:1030
    pub(crate) fn instantiate_alias_or_interface_with_defaults(&mut self, managed_sym: P<Symbol>, type_arguments: &[P<Type>], in_java_script: bool) -> Option<P<Type>> {
        let declared_managed_type = self.get_declared_type_of_symbol(managed_sym);
        // fetches interface type, or initializes symbol links type parameters
        if managed_sym.flags().intersects(SymbolFlags::TypeAlias) {
            let params = self.type_alias_links.get(managed_sym).type_parameters.get();
            if params.len() >= type_arguments.len() {
                let args = self.fill_missing_type_arguments(type_arguments, &params, type_arguments.len() as i32, in_java_script);
                if args.is_empty() {
                    return Some(declared_managed_type);
                }
                return Some(self.get_type_alias_instantiation(managed_sym, &args, None));
            }
        }
        if declared_managed_type.object_flags().intersects(ObjectFlags::ClassOrInterface)
            && declared_managed_type.as_interface_type().type_parameters().len() >= type_arguments.len()
        {
            let args = self.fill_missing_type_arguments(
                type_arguments,
                &declared_managed_type.as_interface_type().type_parameters(),
                type_arguments.len() as i32,
                in_java_script,
            );
            return Some(self.create_type_reference(declared_managed_type, &args));
        }
        None
    }

    // jsx.go:1050
    pub(crate) fn get_jsx_library_managed_attributes(&mut self, jsx_namespace: Option<P<Symbol>>) -> Option<P<Symbol>> {
        if let Some(jsx_namespace) = jsx_namespace {
            return self.get_symbol(jsx_namespace.exports(), JsxNames.library_managed_attributes, SymbolFlags::Type);
        }
        None
    }

    // jsx.go:1057
    pub(crate) fn get_jsx_element_type_symbol(&mut self, jsx_namespace: Option<P<Symbol>>) -> Option<P<Symbol>> {
        // JSX.ElementType [symbol]
        if let Some(jsx_namespace) = jsx_namespace {
            return self.get_symbol(jsx_namespace.exports(), JsxNames.element_type, SymbolFlags::Type);
        }
        None
    }

    // jsx.go:1073
    // e.g. "props" for React.d.ts,
    // or InternalSymbolNameMissing if ElementAttributesProperty doesn't exist (which means all
    //
    //	non-intrinsic elements' attributes type is 'any'),
    //
    // or "" if it has 0 properties (which means every
    //
    //	non-intrinsic elements' attributes type is the element instance type)
    // jsx_namespace is nil-able (Go passes getJsxNamespaceAt's result straight through).
    pub(crate) fn get_jsx_element_properties_name(&mut self, jsx_namespace: Option<P<Symbol>>) -> String {
        self.get_name_from_jsx_element_attributes_container(JsxNames.element_attributes_property_name_container, jsx_namespace)
    }

    // jsx.go:1077
    // jsx_namespace is nil-able (Go passes getJsxNamespaceAt's result straight through).
    pub(crate) fn get_jsx_element_children_property_name(&mut self, jsx_namespace: Option<P<Symbol>>) -> String {
        if self.compiler_options.jsx == JsxEmit::ReactJSX || self.compiler_options.jsx == JsxEmit::ReactJSXDev {
            // In these JsxEmit modes the children property is fixed to 'children'
            return "children".to_string();
        }
        self.get_name_from_jsx_element_attributes_container(JsxNames.element_children_attribute_name_container, jsx_namespace)
    }

    // jsx.go:1091
    // Look into JSX namespace and then look for container with matching name as nameOfAttribPropContainer.
    // Get a single property from that container if existed. Report an error if there are more than one property.
    //
    // @param nameOfAttribPropContainer a string of value JsxNames.ElementAttributesPropertyNameContainer or JsxNames.ElementChildrenAttributeNameContainer
    //
    //	if other string is given or the container doesn't exist, return undefined.
    pub(crate) fn get_name_from_jsx_element_attributes_container(&mut self, name_of_attrib_prop_container: &str, jsx_namespace: Option<P<Symbol>>) -> String {
        // JSX.ElementAttributesProperty | JSX.ElementChildrenAttribute [symbol]
        if let Some(jsx_namespace) = jsx_namespace {
            let jsx_element_attrib_prop_interface_sym = self.get_symbol(jsx_namespace.exports(), name_of_attrib_prop_container, SymbolFlags::Type);
            if let Some(jsx_element_attrib_prop_interface_sym) = jsx_element_attrib_prop_interface_sym {
                let jsx_element_attrib_prop_interface_type = self.get_declared_type_of_symbol(jsx_element_attrib_prop_interface_sym);
                let properties_of_jsx_element_attrib_prop_interface = self.get_properties_of_type(jsx_element_attrib_prop_interface_type);
                // Element Attributes has zero properties, so the element attributes type will be the class instance type
                if properties_of_jsx_element_attrib_prop_interface.is_empty() {
                    return String::new();
                }
                if properties_of_jsx_element_attrib_prop_interface.len() == 1 {
                    return properties_of_jsx_element_attrib_prop_interface[0].name().to_string();
                }
                let first_declaration = jsx_element_attrib_prop_interface_sym.declarations().first().copied();
                if properties_of_jsx_element_attrib_prop_interface.len() > 1 && first_declaration.is_some() {
                    // More than one property on ElementAttributesProperty is an error
                    self.error(first_declaration, &diagnostics::The_global_type_JSX_0_may_not_have_more_than_one_property, &[&name_of_attrib_prop_container]);
                }
            }
        }
        InternalSymbolNameMissing.to_string()
    }

    // jsx.go:1114
    pub(crate) fn get_static_type_of_referenced_jsx_constructor(&mut self, context: P<Node>) -> P<Type> {
        if ast::is_jsx_opening_fragment(context) {
            return self.get_jsx_fragment_type(context);
        }
        if is_jsx_intrinsic_tag_name(context.tag_name()) {
            let result = self.get_intrinsic_attributes_type_from_jsx_opening_like_element(context).unwrap();
            let fake_signature = self.create_signature_for_jsx_intrinsic(context, result);
            return self.get_or_create_type_from_signature(fake_signature);
        }
        let tag_type = self.check_expression_cached(context.tag_name());
        if tag_type.flags().intersects(TypeFlags::StringLiteral) {
            let result = self.get_intrinsic_attributes_type_from_string_literal_type(tag_type, context);
            let Some(result) = result else {
                return self.error_type;
            };
            let fake_signature = self.create_signature_for_jsx_intrinsic(context, result);
            return self.get_or_create_type_from_signature(fake_signature);
        }
        tag_type
    }

    // jsx.go:1135
    pub(crate) fn get_intrinsic_attributes_type_from_string_literal_type(&mut self, t: P<Type>, location: P<Node>) -> Option<P<Type>> {
        // If the elemType is a stringLiteral type, we can then provide a check to make sure that the string literal type is one of the Jsx intrinsic element type
        // For example:
        //      var CustomTag: "h1" = "h1";
        //      <CustomTag> Hello World </CustomTag>
        let intrinsic_elements_type = self.get_jsx_type(JsxNames.intrinsic_elements, location);
        if !self.is_error_type(intrinsic_elements_type) {
            let string_literal_type_name = get_string_literal_value(t);
            let intrinsic_prop = self.get_property_of_type(intrinsic_elements_type, &string_literal_type_name);
            if let Some(intrinsic_prop) = intrinsic_prop {
                return Some(self.get_type_of_symbol(intrinsic_prop));
            }
            let string_type = self.string_type;
            let index_signature_type = self.get_index_type_of_type(intrinsic_elements_type, string_type);
            if index_signature_type.is_some() {
                return index_signature_type;
            }
            return None;
        }
        // If we need to report an error, we already done so here. So just return any to prevent any more error downstream
        Some(self.any_type)
    }

    // jsx.go:1157
    pub(crate) fn get_jsx_reference_kind(&mut self, node: P<Node>) -> JsxReferenceKind {
        if is_jsx_intrinsic_tag_name(node.tag_name()) {
            return JsxReferenceKind::Mixed;
        }
        let checked = self.check_expression(node.tag_name());
        let tag_type = self.get_apparent_type(checked);
        if !self.get_signatures_of_type(tag_type, SignatureKind::Construct).is_empty() {
            return JsxReferenceKind::Component;
        }
        if !self.get_signatures_of_type(tag_type, SignatureKind::Call).is_empty() {
            return JsxReferenceKind::Function;
        }
        JsxReferenceKind::Mixed
    }

    // jsx.go:1171
    pub(crate) fn create_signature_for_jsx_intrinsic(&mut self, node: P<Node>, result: P<Type>) -> P<Signature> {
        let mut element_type = self.error_type;
        if let Some(namespace) = self.get_jsx_namespace_at(Some(node)) {
            let exports = self.get_exports_of_symbol(namespace);
            if let Some(type_symbol) = self.get_symbol(exports, JsxNames.element, SymbolFlags::Type) {
                element_type = self.get_declared_type_of_symbol(type_symbol);
            }
        }
        // returnNode := typeSymbol && c.nodeBuilder.symbolToEntityName(typeSymbol, ast.SymbolFlagsType, node)
        // declaration := factory.createFunctionTypeNode(nil, []ParameterDeclaration{factory.createParameterDeclaration(nil, nil /*dotDotDotToken*/, "props", nil /*questionToken*/, c.nodeBuilder.typeToTypeNode(result, node))}, ifElse(returnNode != nil, factory.createTypeReferenceNode(returnNode, nil /*typeArguments*/), factory.createKeywordTypeNode(ast.KindAnyKeyword)))
        let parameter_symbol = self.new_symbol(SymbolFlags::FunctionScopedVariable, "props");
        self.value_symbol_links.get(parameter_symbol).resolved_type.set(Some(result));
        self.new_signature(SignatureFlags::None, None, &[], None, &[parameter_symbol], Some(element_type), None, 1)
    }

    // jsx.go:1188
    // Get attributes type of the given intrinsic opening-like Jsx element by resolving the tag name.
    // The function is intended to be called from a function which has checked that the opening element is an intrinsic element.
    // @param node an intrinsic JSX opening-like element
    pub(crate) fn get_intrinsic_attributes_type_from_jsx_opening_like_element(&mut self, node: P<Node>) -> Option<P<Type>> {
        assert!(is_jsx_intrinsic_tag_name(node.tag_name()));
        let links = self.jsx_element_links.get_key(node);
        if self.jsx_element_links.at(links).resolved_jsx_element_attributes_type.get().is_some() {
            return self.jsx_element_links.at(links).resolved_jsx_element_attributes_type.get();
        }
        let symbol = self.get_intrinsic_tag_symbol(node);
        if self.jsx_element_links.at(links).jsx_flags.get().intersects(JsxFlags::IntrinsicNamedElement) {
            let t = self.get_type_of_symbol(symbol.unwrap());
            self.jsx_element_links.at(links).resolved_jsx_element_attributes_type.set(Some(t));
            return self.jsx_element_links.at(links).resolved_jsx_element_attributes_type.get();
        }
        if self.jsx_element_links.at(links).jsx_flags.get().intersects(JsxFlags::IntrinsicIndexedElement) {
            let intrinsic_elements_type = self.get_jsx_type(JsxNames.intrinsic_elements, node);
            let index_info = self.get_applicable_index_info_for_name(intrinsic_elements_type, node.tag_name().text());
            if let Some(index_info) = index_info {
                self.jsx_element_links.at(links).resolved_jsx_element_attributes_type.set(self.index_info(index_info).value_type.get());
                return self.jsx_element_links.at(links).resolved_jsx_element_attributes_type.get();
            }
        }
        self.jsx_element_links.at(links).resolved_jsx_element_attributes_type.set(Some(self.error_type));
        self.jsx_element_links.at(links).resolved_jsx_element_attributes_type.get()
    }

    // jsx.go:1214
    // Looks up an intrinsic tag name and returns a symbol that either points to an intrinsic
    // property (in which case nodeLinks.jsxFlags will be IntrinsicNamedElement) or an intrinsic
    // string index signature (in which case nodeLinks.jsxFlags will be IntrinsicIndexedElement).
    // May also return unknownSymbol if both of these lookups fail.
    pub(crate) fn get_intrinsic_tag_symbol(&mut self, node: P<Node>) -> Option<P<Symbol>> {
        let links = self.symbol_node_links.get_key(node);
        if self.symbol_node_links.at(links).resolved_symbol.get().is_some() {
            return self.symbol_node_links.at(links).resolved_symbol.get();
        }
        let intrinsic_elements_type = self.get_jsx_type(JsxNames.intrinsic_elements, node);
        if !self.is_error_type(intrinsic_elements_type) {
            // Property case
            let tag_name = node.tag_name();
            if !ast::is_identifier(tag_name) && !ast::is_jsx_namespaced_name(tag_name) {
                panic!("Invalid tag name");
            }
            let prop_name = tag_name.text();
            let intrinsic_prop = self.get_property_of_type(intrinsic_elements_type, prop_name);
            if intrinsic_prop.is_some() {
                let jsx_links = self.jsx_element_links.get_key(node);
                self.jsx_element_links.at(jsx_links).jsx_flags.set(self.jsx_element_links.at(jsx_links).jsx_flags.get() | JsxFlags::IntrinsicNamedElement);
                self.symbol_node_links.at(links).resolved_symbol.set(intrinsic_prop);
                return self.symbol_node_links.at(links).resolved_symbol.get();
            }
            // Intrinsic string indexer case
            let prop_name_type = self.get_string_literal_type(prop_name);
            let index_symbol = self.get_applicable_index_symbol(intrinsic_elements_type, prop_name_type);
            if index_symbol.is_some() {
                let jsx_links = self.jsx_element_links.get_key(node);
                self.jsx_element_links.at(jsx_links).jsx_flags.set(self.jsx_element_links.at(jsx_links).jsx_flags.get() | JsxFlags::IntrinsicIndexedElement);
                self.symbol_node_links.at(links).resolved_symbol.set(index_symbol);
                return self.symbol_node_links.at(links).resolved_symbol.get();
            }
            if self.get_type_of_property_or_index_signature_of_type(intrinsic_elements_type, prop_name).is_some() {
                let jsx_links = self.jsx_element_links.get_key(node);
                self.jsx_element_links.at(jsx_links).jsx_flags.set(self.jsx_element_links.at(jsx_links).jsx_flags.get() | JsxFlags::IntrinsicIndexedElement);
                self.symbol_node_links.at(links).resolved_symbol.set(intrinsic_elements_type.symbol());
                return self.symbol_node_links.at(links).resolved_symbol.get();
            }
            // Wasn't found
            let type_name = format!("JSX.{}", JsxNames.intrinsic_elements);
            self.error(Some(node), &diagnostics::Property_0_does_not_exist_on_type_1, &[&tag_name.text(), &type_name]);
            self.symbol_node_links.at(links).resolved_symbol.set(Some(self.unknown_symbol));
            return self.symbol_node_links.at(links).resolved_symbol.get();
        }
        if self.no_implicit_any {
            self.error(Some(node), &diagnostics::JSX_element_implicitly_has_type_any_because_no_interface_JSX_0_exists, &[&JsxNames.intrinsic_elements]);
        }
        self.symbol_node_links.at(links).resolved_symbol.set(Some(self.unknown_symbol));
        self.symbol_node_links.at(links).resolved_symbol.get()
    }

    // jsx.go:1257
    pub(crate) fn get_jsx_stateless_element_type_at(&mut self, location: P<Node>) -> Option<P<Type>> {
        let jsx_element_type = self.get_jsx_element_type_at(location);
        // getJsxElementTypeAt never returns nil (getJsxType falls back to errorType), so Go's nil check is always false.
        let null_type = self.null_type;
        Some(self.get_union_type(&[jsx_element_type, null_type]))
    }

    // jsx.go:1265
    pub(crate) fn get_jsx_element_class_type_at(&mut self, location: P<Node>) -> Option<P<Type>> {
        let t = self.get_jsx_type(JsxNames.element_class, location);
        if self.is_error_type(t) {
            return None;
        }
        Some(t)
    }

    // jsx.go:1273
    pub(crate) fn get_jsx_element_type_at(&mut self, location: P<Node>) -> P<Type> {
        self.get_jsx_type(JsxNames.element, location)
    }

    // jsx.go:1277
    pub(crate) fn get_jsx_element_type_type_at(&mut self, location: P<Node>) -> Option<P<Type>> {
        let ns = self.get_jsx_namespace_at(Some(location));
        if ns.is_none() {
            return None;
        }
        let sym = self.get_jsx_element_type_symbol(ns)?;
        let t = self.instantiate_alias_or_interface_with_defaults(sym, &[], ast::is_in_js_file(location));
        match t {
            Some(t) if !self.is_error_type(t) => Some(t),
            _ => None,
        }
    }

    // jsx.go:1293
    pub(crate) fn get_jsx_type(&mut self, name: &str, location: P<Node>) -> P<Type> {
        if let Some(namespace) = self.get_jsx_namespace_at(Some(location)) {
            if let Some(exports) = self.get_exports_of_symbol(namespace) {
                if let Some(type_symbol) = self.get_symbol(Some(exports), name, SymbolFlags::Type) {
                    return self.get_declared_type_of_symbol(type_symbol);
                }
            }
        }
        self.error_type
    }

    // jsx.go:1304
    pub(crate) fn get_jsx_namespace_at(&mut self, location: Option<P<Node>>) -> Option<P<Symbol>> {
        let mut links = None;
        if let Some(location) = location {
            links = Some(self.jsx_element_links.get_key(location));
        }
        if let Some(links) = links {
            if let Some(jsx_namespace) = self.jsx_element_links.at(links).jsx_namespace.get() {
                if jsx_namespace != self.unknown_symbol {
                    return Some(jsx_namespace);
                }
            }
        }
        if links.is_none() || self.jsx_element_links.at(links.unwrap()).jsx_namespace.get() != Some(self.unknown_symbol) {
            // Go dereferences the source file of location here, so location is non-nil on this path.
            let mut resolved_namespace = self.get_jsx_namespace_container_for_implicit_import(location.unwrap());
            if resolved_namespace.is_none() || resolved_namespace == Some(self.unknown_symbol) {
                let namespace_name = self.get_jsx_namespace(location);
                resolved_namespace = self.resolve_name(location, &namespace_name, SymbolFlags::Namespace, None /*nameNotFoundMessage*/, false /*isUse*/, false /*excludeGlobals*/);
            }
            if let Some(resolved_namespace) = resolved_namespace {
                let resolved = self.resolve_symbol(resolved_namespace);
                let exports = self.get_exports_of_symbol(resolved);
                let candidate = self.get_symbol(exports, JsxNames.jsx, SymbolFlags::Namespace).map(|s| self.resolve_symbol(s));
                if let Some(candidate) = candidate {
                    if candidate != self.unknown_symbol {
                        if let Some(links) = links {
                            self.jsx_element_links.at(links).jsx_namespace.set(Some(candidate));
                        }
                        return Some(candidate);
                    }
                }
            }
            if let Some(links) = links {
                self.jsx_element_links.at(links).jsx_namespace.set(Some(self.unknown_symbol));
            }
        }
        // JSX global fallback
        let s = self.get_global_symbol(JsxNames.jsx, SymbolFlags::Namespace, None /*diagnostic*/).map(|s| self.resolve_symbol(s));
        if s == Some(self.unknown_symbol) {
            return None;
        }
        s
    }

    // jsx.go:1339
    pub(crate) fn get_jsx_namespace(&mut self, location: Option<P<Node>>) -> String {
        if let Some(location) = location {
            let file = ast::get_source_file_of_node(location);
            if let Some(file) = file {
                let links = self.source_file_links.get_key(file);
                if ast::is_jsx_opening_fragment(location) {
                    if !self.source_file_links.at(links).local_jsx_fragment_namespace.get().is_empty() {
                        return self.source_file_links.at(links).local_jsx_fragment_namespace.get().to_string();
                    }
                    let jsx_fragment_pragma = ast::get_pragma_from_source_file(Some(file), "jsxfrag");
                    if let Some(jsx_fragment_pragma) = jsx_fragment_pragma {
                        let factory = self.parse_isolated_entity_name(pragma_factory_argument(jsx_fragment_pragma));
                        self.source_file_links.at(links).local_jsx_fragment_factory.set(factory);
                        if let Some(factory) = factory {
                            self.source_file_links.at(links).local_jsx_fragment_namespace.set(ast::get_first_identifier(factory).text());
                            return self.source_file_links.at(links).local_jsx_fragment_namespace.get().to_string();
                        }
                    }
                    let entity = self.get_jsx_fragment_factory_entity(Some(location));
                    if let Some(entity) = entity {
                        self.source_file_links.at(links).local_jsx_fragment_factory.set(Some(entity));
                        self.source_file_links.at(links).local_jsx_fragment_namespace.set(ast::get_first_identifier(entity).text());
                        return self.source_file_links.at(links).local_jsx_fragment_namespace.get().to_string();
                    }
                } else {
                    let local_jsx_namespace = self.get_local_jsx_namespace(file);
                    if !local_jsx_namespace.is_empty() {
                        self.source_file_links.at(links).local_jsx_namespace.set(&local_jsx_namespace);
                        return self.source_file_links.at(links).local_jsx_namespace.get().to_string();
                    }
                }
            }
        }
        if self._jsx_namespace.is_empty() {
            self._jsx_namespace = "React".to_string();
            if !self.compiler_options.jsx_factory.is_empty() {
                let jsx_factory = self.compiler_options.jsx_factory.clone();
                self._jsx_factory_entity = self.parse_isolated_entity_name(&jsx_factory);
                if let Some(jsx_factory_entity) = self._jsx_factory_entity {
                    self._jsx_namespace = ast::get_first_identifier(jsx_factory_entity).text().to_string();
                }
            } else if !self.compiler_options.react_namespace.is_empty() {
                self._jsx_namespace = self.compiler_options.react_namespace.clone();
            }
        }
        if self._jsx_factory_entity.is_none() {
            let left = self.factory.new_identifier(&self._jsx_namespace);
            let right = self.factory.new_identifier("createElement");
            self._jsx_factory_entity = Some(self.factory.new_qualified_name(left, right));
        }
        self._jsx_namespace.clone()
    }

    // jsx.go:1388
    pub(crate) fn get_local_jsx_namespace(&mut self, file: P<SourceFile>) -> String {
        let links = self.source_file_links.get_key(file);
        if !self.source_file_links.at(links).local_jsx_namespace.get().is_empty() {
            return self.source_file_links.at(links).local_jsx_namespace.get().to_string();
        }
        let jsx_pragma = ast::get_pragma_from_source_file(Some(file), "jsx");
        if let Some(jsx_pragma) = jsx_pragma {
            let factory = self.parse_isolated_entity_name(pragma_factory_argument(jsx_pragma));
            self.source_file_links.at(links).local_jsx_factory.set(factory);
            if let Some(factory) = factory {
                self.source_file_links.at(links).local_jsx_namespace.set(ast::get_first_identifier(factory).text());
                return self.source_file_links.at(links).local_jsx_namespace.get().to_string();
            }
        }
        String::new()
    }

    // jsx.go:1404
    pub(crate) fn get_jsx_factory_entity(&mut self, location: Option<P<Node>>) -> Option<P<Node>> {
        if let Some(location) = location {
            self.get_jsx_namespace(Some(location));
            if let Some(local_jsx_factory) = self.source_file_links.get(ast::get_source_file_of_node(location).unwrap()).local_jsx_factory.get() {
                return Some(local_jsx_factory);
            }
        }
        self._jsx_factory_entity
    }

    // jsx.go:1414
    pub(crate) fn get_jsx_fragment_factory_entity(&mut self, location: Option<P<Node>>) -> Option<P<Node>> {
        if let Some(location) = location {
            let file = ast::get_source_file_of_node(location);
            if let Some(file) = file {
                let links = self.source_file_links.get_key(file);
                if let Some(local_jsx_fragment_factory) = self.source_file_links.at(links).local_jsx_fragment_factory.get() {
                    return Some(local_jsx_fragment_factory);
                }
                let jsx_frag_pragma = ast::get_pragma_from_source_file(Some(file), "jsxfrag");
                if let Some(jsx_frag_pragma) = jsx_frag_pragma {
                    let factory = self.parse_isolated_entity_name(pragma_factory_argument(jsx_frag_pragma));
                    self.source_file_links.at(links).local_jsx_fragment_factory.set(factory);
                    return self.source_file_links.at(links).local_jsx_fragment_factory.get();
                }
            }
        }
        if !self.compiler_options.jsx_fragment_factory.is_empty() {
            let jsx_fragment_factory = self.compiler_options.jsx_fragment_factory.clone();
            return self.parse_isolated_entity_name(&jsx_fragment_factory);
        }
        None
    }

    // jsx.go:1435
    // parser.ParseIsolatedEntityName returns nil for an invalid entity name, so the result is nil-able.
    pub(crate) fn parse_isolated_entity_name(&mut self, name: &str) -> Option<P<Node>> {
        let result = tsrs_parser::parse_isolated_entity_name(name);
        if let Some(result) = result {
            mark_as_synthetic(result);
        }
        result
    }
}

/// Go `pragma.Args["factory"].Value` (the zero value "" when the argument is absent).
fn pragma_factory_argument(pragma: &'static Pragma) -> &'static str {
    pragma.args.get("factory").map_or("", |arg| arg.value.as_str())
}

// jsx.go:1443
pub(crate) fn mark_as_synthetic(node: P<Node>) -> bool {
    node.set_loc(TextRange::new(-1, -1));
    node.for_each_child(&mut |child| mark_as_synthetic(child));
    false
}

impl Checker {
    // jsx.go:1449
    pub(crate) fn get_jsx_namespace_container_for_implicit_import(&mut self, location: P<Node>) -> Option<P<Symbol>> {
        let file = ast::get_source_file_of_node(location).unwrap();
        let links = self.jsx_element_links.get_key(file.as_node());
        if let Some(jsx_implicit_import_container) = self.jsx_element_links.at(links).jsx_implicit_import_container.get() {
            return if jsx_implicit_import_container == self.unknown_symbol { None } else { Some(jsx_implicit_import_container) };
        }
        let mut canonical_error_tag = self.jsx_element_links.at(links).first_jsx_tag_in_file.get();
        if canonical_error_tag.is_none() {
            fn visit(links: &JsxElementLinks, node: P<Node>) -> bool {
                if ast::is_jsx_element(node) || ast::is_jsx_self_closing_element(node) {
                    links.first_jsx_tag_in_file.set(Some(node));
                    return true;
                }
                if ast::is_jsx_fragment(node) {
                    links.first_jsx_tag_in_file.set(Some(node.as_jsx_fragment().opening_fragment)); // to match strada, fragments issue errors on the opening fragment instead of the whole tag
                    return true;
                }
                node.for_each_child(&mut |child| visit(links, child))
            }
            file.as_node().for_each_child(&mut |child| visit(self.jsx_element_links.at(links), child));
            canonical_error_tag = self.jsx_element_links.at(links).first_jsx_tag_in_file.get();
        }
        let (module_reference, specifier) = self.get_jsx_runtime_import_specifier(file);
        if module_reference.is_empty() {
            return None;
        }
        let error_message = &diagnostics::This_JSX_tag_requires_the_module_path_0_to_exist_but_none_could_be_found_Make_sure_you_have_types_for_the_appropriate_package_installed;
        let mod_ = self.resolve_external_module(specifier.or(canonical_error_tag), &module_reference, Some(error_message), canonical_error_tag, false, None /*importAttributesType*/);
        let mut result: Option<P<Symbol>> = None;
        if let Some(mod_) = mod_ {
            if mod_ != self.unknown_symbol {
                let resolved = self.resolve_symbol(mod_);
                result = Some(self.get_merged_symbol(resolved));
            }
        }
        self.jsx_element_links.at(links).jsx_implicit_import_container.set(Some(result.unwrap_or(self.unknown_symbol)));
        result
    }

    // jsx.go:1486
    // the specifier is nil when the file has no explicit jsxImportSource pragma node (Program returns Option).
    pub(crate) fn get_jsx_runtime_import_specifier(&mut self, file: P<SourceFile>) -> (String, Option<P<Node>>) {
        self.program.get_jsx_runtime_import_specifier(file.path())
    }
}

/// Go closure `checkTagNameDoesNotExpectTooManyArguments` of checkApplicableSignatureForJsxCallLikeElement.
fn check_tag_name_does_not_expect_too_many_arguments(c: &mut Checker, node: P<Node>, report_errors: bool, diagnostic_output: Option<&mut Vec<P<Diagnostic>>>) -> bool {
    if c.get_jsx_namespace_container_for_implicit_import(node).is_some() {
        return true; // factory is implicitly jsx/jsxdev - assume it fits the bill, since we don't strongly look for the jsx/jsxs/jsxDEV factory APIs anywhere else (at least not yet)
    }
    // We assume fragments have the correct arity since the node does not have attributes
    let mut tag_type: Option<P<Type>> = None;
    if (ast::is_jsx_opening_element(node) || ast::is_jsx_self_closing_element(node))
        && !(is_jsx_intrinsic_tag_name(node.tag_name()) || ast::is_jsx_namespaced_name(node.tag_name()))
    {
        tag_type = Some(c.check_expression(node.tag_name()));
    }
    let Some(tag_type) = tag_type else {
        return true;
    };
    let tag_call_signatures = c.get_signatures_of_type(tag_type, SignatureKind::Call);
    if tag_call_signatures.is_empty() {
        return true;
    }
    let factory = c.get_jsx_factory_entity(Some(node));
    let Some(factory) = factory else {
        return true;
    };
    let factory_symbol = c.resolve_entity_name(factory, SymbolFlags::Value, true /*ignoreErrors*/, false /*dontResolveAlias*/, Some(node));
    let Some(factory_symbol) = factory_symbol else {
        return true;
    };

    let factory_type = c.get_type_of_symbol(factory_symbol);
    let call_signatures = c.get_signatures_of_type(factory_type, SignatureKind::Call);
    if call_signatures.is_empty() {
        return true;
    }
    let mut has_first_param_signatures = false;
    let mut max_param_count = 0;
    // Check that _some_ first parameter expects a FC-like thing, and that some overload of the SFC expects an acceptable number of arguments
    for sig in call_signatures {
        let firstparam = c.get_type_at_position(sig, 0);
        let signatures_of_param = c.get_signatures_of_type(firstparam, SignatureKind::Call);
        if signatures_of_param.is_empty() {
            continue;
        }
        for param_sig in signatures_of_param {
            has_first_param_signatures = true;
            if c.has_effective_rest_parameter(param_sig) {
                return true; // some signature has a rest param, so function components can have an arbitrary number of arguments
            }
            let param_count = c.get_parameter_count(param_sig);
            if param_count > max_param_count {
                max_param_count = param_count;
            }
        }
    }
    if !has_first_param_signatures {
        // Not a single signature had a first parameter which expected a signature - for back compat, and
        // to guard against generic factories which won't have signatures directly, do not error
        return true;
    }
    let mut absolute_min_arg_count = i32::MAX;
    for tag_sig in tag_call_signatures {
        let tag_required_arg_count = c.get_min_argument_count(tag_sig);
        if tag_required_arg_count < absolute_min_arg_count {
            absolute_min_arg_count = tag_required_arg_count;
        }
    }
    if absolute_min_arg_count <= max_param_count {
        return true; // some signature accepts the number of arguments the function component provides
    }
    if report_errors {
        let tag_name = node.tag_name();
        // We will not report errors in this function for fragments, since we do not check them in this function
        let tag_name_text = crate::entity_name_to_string(tag_name);
        let factory_text = crate::entity_name_to_string(factory);
        let diag = new_diagnostic_for_node(
            Some(tag_name),
            Some(&diagnostics::Tag_0_expects_at_least_1_arguments_but_the_JSX_factory_2_provides_at_most_3),
            &[&tag_name_text, &absolute_min_arg_count, &factory_text, &max_param_count],
        );
        let tag_name_symbol = c.get_symbol_at_location(tag_name, false);
        if let Some(tag_name_symbol) = tag_name_symbol {
            if let Some(value_declaration) = tag_name_symbol.value_declaration() {
                let tag_name_text = crate::entity_name_to_string(tag_name);
                diag.add_related_info(new_diagnostic_for_node(Some(value_declaration), Some(&diagnostics::X_0_is_declared_here), &[&tag_name_text]));
            }
        }
        c.report_diagnostic(Some(diag), diagnostic_output);
    }
    false
}
