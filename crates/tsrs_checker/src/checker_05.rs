use crate::*;
use tsrs_ast::*;
use tsrs_core::*;
use tsrs_ast as ast;
use tsrs_diagnostics as diagnostics;
use tsrs_diagnostics::Message;
use std::cell::Cell;

// Generated as signature stubs by tools/gosig (checker.json); the bodies have since been ported by hand. Do not re-run
// gosig into this directory: it rewrites every file listed in checker.json.

// Non-function declarations in checker.go:8749-10856 (hand-ported by checker-foundation):
//   type constructorAccessibilityError (checker.go:8829)
//   type CallState (checker.go:9014)

impl Checker {
    // checker.go:8749
    pub(crate) fn resolve_new_expression(&mut self, node: P<Node>, candidates_out_array: Option<&mut Vec<P<Signature>>>, check_mode: CheckMode) -> P<Signature> {
        let mut expression_type = self.check_non_null_expression(node.expression().unwrap());
        if expression_type == self.silent_never_type {
            return self.silent_never_signature;
        }
        // If expressionType's apparent type(section 3.8.1) is an object type with one or
        // more construct signatures, the expression is processed in the same manner as a
        // function call, but using the construct signatures as the initial set of candidate
        // signatures for overload resolution. The result type of the function call becomes
        // the result type of the operation.
        expression_type = self.get_apparent_type(expression_type);
        if self.is_error_type(expression_type) {
            // Another error has already been reported
            return self.resolve_error_call(node);
        }
        // TS 1.0 spec: 4.11
        // If expressionType is of type Any, Args can be any argument
        // list and the result of the operation is of type Any.
        if is_type_any(Some(expression_type)) {
            if !node.type_arguments().is_empty() {
                self.error(Some(node), &diagnostics::Untyped_function_calls_may_not_accept_type_arguments, &[]);
            }
            return self.resolve_untyped_call(node);
        }
        // Technically, this signatures list may be incomplete. We are taking the apparent type,
        // but we are not including construct signatures that may have been added to the Object or
        // Function interface, since they have none by default. This is a bit of a leap of faith
        // that the user will not add any.
        let construct_signatures = self.get_signatures_of_type(expression_type, SignatureKind::Construct);
        if !construct_signatures.is_empty() {
            let accessibility_error = self.get_constructor_accessibility_error(node, &construct_signatures, ModifierFlags::NonPublicAccessibilityModifier);
            if let Some(accessibility_error) = accessibility_error {
                if accessibility_error.kind.intersects(ModifierFlags::Private) {
                    let s = self.type_to_string_exported(accessibility_error.declaring_class);
                    self.error(Some(node), &diagnostics::Constructor_of_class_0_is_private_and_only_accessible_within_the_class_declaration, &[&s]);
                }
                if accessibility_error.kind.intersects(ModifierFlags::Protected) {
                    let s = self.type_to_string_exported(accessibility_error.declaring_class);
                    self.error(Some(node), &diagnostics::Constructor_of_class_0_is_protected_and_only_accessible_within_the_class_declaration, &[&s]);
                }
                return self.resolve_error_call(node);
            }
            // If the expression is a class of abstract type, or an abstract construct signature,
            // then it cannot be instantiated.
            // In the case of a merged class-module or class-interface declaration,
            // only the class declaration node will have the Abstract flag set.
            if some_signature(&construct_signatures, |sig| sig.flags().intersects(SignatureFlags::Abstract)) {
                self.error(Some(node), &diagnostics::Cannot_create_an_instance_of_an_abstract_class, &[]);
                return self.resolve_error_call(node);
            }
            if let Some(symbol) = expression_type.symbol() {
                let value_decl = get_class_like_declaration_of_symbol(symbol);
                if let Some(value_decl) = value_decl {
                    if has_modifier(value_decl, ModifierFlags::Abstract) {
                        self.error(Some(node), &diagnostics::Cannot_create_an_instance_of_an_abstract_class, &[]);
                        return self.resolve_error_call(node);
                    }
                }
            }
            return self.resolve_call(node, &construct_signatures, candidates_out_array, check_mode, SignatureFlags::None, None);
        }
        // If expressionType's apparent type is an object type with no construct signatures but
        // one or more call signatures, the expression is processed as a function call. A compile-time
        // error occurs if the result of the function call is not Void. The type of the result of the
        // operation is Any. It is an error to have a Void this type.
        let call_signatures = self.get_signatures_of_type(expression_type, SignatureKind::Call);
        if !call_signatures.is_empty() {
            let signature = self.resolve_call(node, &call_signatures, candidates_out_array, check_mode, SignatureFlags::None, None);
            if !self.no_implicit_any {
                if signature.declaration().is_some() && self.get_return_type_of_signature(signature) != self.void_type {
                    self.error(Some(node), &diagnostics::Only_a_void_function_can_be_called_with_the_new_keyword, &[]);
                }
                if self.get_this_type_of_signature(signature) == Some(self.void_type) {
                    self.error(Some(node), &diagnostics::A_function_that_is_called_with_the_new_keyword_cannot_have_a_this_type_that_is_void, &[]);
                }
            }
            return signature;
        }
        self.invocation_error(node.expression().unwrap(), expression_type, SignatureKind::Construct, None);
        self.resolve_error_call(node)
    }

    // checker.go:8834
    pub(crate) fn get_constructor_accessibility_error(&mut self, node: P<Node>, signatures: &[P<Signature>], modifiers_mask: ModifierFlags) -> Option<P<constructorAccessibilityError>> {
        for &signature in signatures {
            let Some(declaration) = signature.declaration() else {
                continue;
            };
            let modifiers = get_selected_modifier_flags(declaration, modifiers_mask);
            // (1) Public constructors and (2) constructor functions are always accessible.
            if modifiers.is_empty() || !is_constructor_declaration(declaration) {
                continue;
            }
            let parent_symbol = declaration.parent().unwrap().symbol().unwrap();
            let declaring_class_declaration = get_class_like_declaration_of_symbol(parent_symbol);
            // A private or protected constructor can only be instantiated within its own class (or a subclass, for protected)
            if !self.is_node_within_class(node, declaring_class_declaration.unwrap()) {
                let containing_class = get_containing_class(node);
                if let Some(containing_class) = containing_class {
                    if modifiers.intersects(ModifierFlags::Protected) {
                        let containing_type = self.get_type_of_node(containing_class);
                        if self.type_has_protected_accessible_base(parent_symbol, containing_type) {
                            continue;
                        }
                    }
                }
                let declaring_class = self.get_declared_type_of_symbol(parent_symbol);
                return Some(P::new(constructorAccessibilityError { kind: modifiers, declaring_class }));
            }
        }
        None
    }

    // checker.go:8864
    pub(crate) fn type_has_protected_accessible_base(&mut self, target: P<Symbol>, t: P<Type>) -> bool {
        let target_type = self.get_target_type(t).unwrap();
        let base_types = self.get_base_types(target_type);
        if base_types.is_empty() {
            return false;
        }
        let first_base = base_types[0];
        if first_base.flags().intersects(TypeFlags::Intersection) {
            let types = first_base.as_intersection_type().types.get();
            let (mixin_flags, _) = self.find_mixins(types);
            for (i, &intersection_member) in first_base.types().iter().enumerate() {
                // We want to ignore mixin ctors
                if !mixin_flags[i] {
                    if intersection_member.object_flags().intersects(ObjectFlags::Class | ObjectFlags::Interface) {
                        if intersection_member.symbol() == Some(target) {
                            return true;
                        }
                        if self.type_has_protected_accessible_base(target, intersection_member) {
                            return true;
                        }
                    }
                }
            }
            return false;
        }
        if first_base.symbol() == Some(target) {
            return true;
        }
        self.type_has_protected_accessible_base(target, first_base)
    }
}

// checker.go:8894
pub(crate) fn some_signature(signatures: &[P<Signature>], mut f: impl FnMut(P<Signature>) -> bool) -> bool {
    for &sig in signatures {
        let composite = sig.composite();
        if composite.is_some_and(|composite| composite.is_union.get() && composite.signatures.get().iter().any(|&s| f(s))) || composite.is_none() && f(sig) {
            return true;
        }
    }
    false
}

impl Checker {
    // checker.go:8903
    pub(crate) fn resolve_tagged_template_expression(&mut self, node: P<Node>, candidates_out_array: Option<&mut Vec<P<Signature>>>, check_mode: CheckMode) -> P<Signature> {
        let tag = node.as_tagged_template_expression().tag;
        let tag_type = self.check_expression(tag);
        let apparent_type = self.get_apparent_type(tag_type);
        if self.is_error_type(apparent_type) {
            // Another error has already been reported
            return self.resolve_error_call(node);
        }
        let call_signatures = self.get_signatures_of_type(apparent_type, SignatureKind::Call);
        let num_construct_signatures = self.get_signatures_of_type(apparent_type, SignatureKind::Construct).len() as i32;
        if self.is_untyped_function_call(tag_type, apparent_type, call_signatures.len() as i32, num_construct_signatures) {
            return self.resolve_untyped_call(node);
        }
        if call_signatures.is_empty() {
            if is_array_literal_expression(node.parent().unwrap()) {
                self.error(Some(tag), &diagnostics::It_is_likely_that_you_are_missing_a_comma_to_separate_these_two_template_expressions_They_form_a_tagged_template_expression_which_cannot_be_invoked, &[]);
                return self.resolve_error_call(node);
            }
            self.invocation_error(tag, apparent_type, SignatureKind::Call, None);
            return self.resolve_error_call(node);
        }
        self.resolve_call(node, &call_signatures, candidates_out_array, check_mode, SignatureFlags::None, None)
    }

    // checker.go:8927
    pub(crate) fn resolve_decorator(&mut self, node: P<Node>, candidates_out_array: Option<&mut Vec<P<Signature>>>, check_mode: CheckMode) -> P<Signature> {
        if !can_have_decorators(node.parent().unwrap()) {
            return self.resolve_error_call(node);
        }
        let expression = node.expression().unwrap();
        let func_type = self.check_expression(expression);
        let apparent_type = self.get_apparent_type(func_type);
        if self.is_error_type(apparent_type) {
            return self.resolve_error_call(node);
        }
        let call_signatures = self.get_signatures_of_type(apparent_type, SignatureKind::Call);
        let num_construct_signatures = self.get_signatures_of_type(apparent_type, SignatureKind::Construct).len() as i32;
        if self.is_untyped_function_call(func_type, apparent_type, call_signatures.len() as i32, num_construct_signatures) {
            return self.resolve_untyped_call(node);
        }
        if self.is_potentially_uncalled_decorator(node, &call_signatures) && !is_parenthesized_expression(expression) {
            let node_str = tsrs_scanner::get_text_of_node(expression);
            self.error(Some(node), &diagnostics::X_0_accepts_too_few_arguments_to_be_used_as_a_decorator_here_Did_you_mean_to_call_it_first_and_write_0, &[&node_str]);
            return self.resolve_error_call(node);
        }
        let head_message = self.get_diagnostic_head_message_for_decorator_resolution(node);
        if call_signatures.is_empty() {
            let details = self.invocation_error_details(expression, apparent_type, SignatureKind::Call);
            let mut diag = ast::new_diagnostic_chain(details, head_message, &[]);
            diag = self.add_diagnostic(diag);
            self.invocation_error_recovery(apparent_type, SignatureKind::Call, diag);
            return self.resolve_error_call(node);
        }
        let decorator_signature = self.get_decorator_call_signature(node);
        if decorator_signature.is_none() {
            return self.resolve_error_call(node);
        }
        self.resolve_call(node, &call_signatures, candidates_out_array, check_mode, SignatureFlags::None, Some(head_message))
    }

    // Sometimes, we have a decorator that could accept zero arguments,
    // but is receiving too many arguments as part of the decorator invocation.
    // In those cases, a user may have meant to *call* the expression before using it as a decorator.
    // checker.go:8963
    pub(crate) fn is_potentially_uncalled_decorator(&mut self, decorator: P<Node>, signatures: &[P<Signature>]) -> bool {
        if signatures.is_empty() {
            return false;
        }
        for &sig in signatures {
            if !(sig.min_argument_count.get() == 0 && !signature_has_rest_parameter(sig) && (sig.parameters().len() as i32) < self.get_decorator_argument_count(decorator, sig)) {
                return false;
            }
        }
        true
    }

    // Gets the localized diagnostic head message to use for errors when resolving a decorator as a call expression.
    // checker.go:8970
    pub(crate) fn get_diagnostic_head_message_for_decorator_resolution(&mut self, node: P<Node>) -> &'static Message {
        match node.parent().unwrap().kind() {
            Kind::ClassDeclaration | Kind::ClassExpression => &diagnostics::Unable_to_resolve_signature_of_class_decorator_when_called_as_an_expression,
            Kind::Parameter => &diagnostics::Unable_to_resolve_signature_of_parameter_decorator_when_called_as_an_expression,
            Kind::PropertyDeclaration => &diagnostics::Unable_to_resolve_signature_of_property_decorator_when_called_as_an_expression,
            Kind::MethodDeclaration | Kind::GetAccessor | Kind::SetAccessor => &diagnostics::Unable_to_resolve_signature_of_method_decorator_when_called_as_an_expression,
            _ => panic!("Unhandled case in getDiagnosticHeadMessageForDecoratorResolution"),
        }
    }

    // checker.go:8984
    pub(crate) fn resolve_instanceof_expression(&mut self, node: P<Node>, candidates_out_array: Option<&mut Vec<P<Signature>>>, check_mode: CheckMode) -> P<Signature> {
        // if rightType is an object type with a custom `[Symbol.hasInstance]` method, then it is potentially
        // valid on the right-hand side of the `instanceof` operator. This allows normal `object` types to
        // participate in `instanceof`, as per Step 2 of https://tc39.es/ecma262/#sec-instanceofoperator.
        let right = node.as_binary_expression().right();
        let right_type = self.check_expression(right);
        if !is_type_any(Some(right_type)) {
            let has_instance_method_type = self.get_symbol_has_instance_method_of_object_type(right_type);
            if let Some(has_instance_method_type) = has_instance_method_type {
                let apparent_type = self.get_apparent_type(has_instance_method_type);
                if self.is_error_type(apparent_type) {
                    return self.resolve_error_call(node);
                }
                let call_signatures = self.get_signatures_of_type(apparent_type, SignatureKind::Call);
                let construct_signatures = self.get_signatures_of_type(apparent_type, SignatureKind::Construct);
                if self.is_untyped_function_call(has_instance_method_type, apparent_type, call_signatures.len() as i32, construct_signatures.len() as i32) {
                    return self.resolve_untyped_call(node);
                }
                if !call_signatures.is_empty() {
                    return self.resolve_call(node, &call_signatures, candidates_out_array, check_mode, SignatureFlags::None, None);
                }
            } else if !(self.type_has_call_or_construct_signatures(right_type) || {
                let global_function_type = self.global_function_type;
                self.is_type_subtype_of(right_type, global_function_type)
            }) {
                self.error(Some(right), &diagnostics::The_right_hand_side_of_an_instanceof_expression_must_be_either_of_type_any_a_class_function_or_other_type_assignable_to_the_Function_interface_type_or_an_object_type_with_a_Symbol_hasInstance_method, &[]);
                return self.resolve_error_call(node);
            }
        }
        // fall back to a default signature
        self.any_signature
    }

    // checker.go:9028
    #[cfg_attr(not(feature = "work-census"), inline(always), expect(clippy::inline_always, reason = "without the census the wrapper is a forwarding call; inlined, callers call the body as before (notes/perf-checker-algorithms.md)"))]
    pub(crate) fn resolve_call(&mut self, node: P<Node>, signatures: &[P<Signature>], candidates_out_array: Option<&mut Vec<P<Signature>>>, check_mode: CheckMode, call_chain_flags: SignatureFlags, head_message: Option<&'static Message>) -> P<Signature> {
        if self.census_on() {
            let callee = signatures.first().and_then(|s| s.declaration());
            let generic = signatures.iter().filter(|s| !s.type_parameters().is_empty()).count() as u64;
            self.census.as_mut().unwrap().call_stack.push(callee);
            let span = self.census_begin(crate::workcensus::Cat::Call, || crate::workcensus::CKey::OptNode(callee));
            let r = self.resolve_call_worker(node, signatures, candidates_out_array, check_mode, call_chain_flags, head_message);
            let timing = self.census_end(span).unwrap();
            let census = self.census.as_mut().unwrap();
            census.call_stack.pop();
            census.record(crate::workcensus::Cat::Call, crate::workcensus::CKey::OptNode(callee), timing, signatures.len() as u64, generic, 0);
            return r;
        }
        self.resolve_call_worker(node, signatures, candidates_out_array, check_mode, call_chain_flags, head_message)
    }

    fn resolve_call_worker(&mut self, node: P<Node>, signatures: &[P<Signature>], candidates_out_array: Option<&mut Vec<P<Signature>>>, check_mode: CheckMode, call_chain_flags: SignatureFlags, head_message: Option<&'static Message>) -> P<Signature> {
        let mut candidates_out_array = candidates_out_array;
        let mut head_message = head_message;
        let is_tagged_template = node.kind() == Kind::TaggedTemplateExpression;
        let is_decorator = node.kind() == Kind::Decorator;
        let is_jsx_opening_or_self_closing_element = is_jsx_opening_like_element(node);
        let is_instanceof = node.kind() == Kind::BinaryExpression;
        let report_errors = !self.is_inference_partially_blocked && candidates_out_array.is_none();
        let mut s = CallState::default();
        s.node = Some(node);
        if !is_decorator && !is_instanceof && !is_super_call(node) && !is_jsx_opening_fragment(node) {
            s.type_arguments = node.type_arguments().to_vec();
            // We already perform checking on the type arguments on the class declaration itself.
            if is_tagged_template || is_jsx_opening_or_self_closing_element || node.expression().unwrap().kind() != Kind::SuperKeyword {
                let type_arguments = s.type_arguments.clone();
                self.check_source_elements(&type_arguments);
            }
        }
        s.candidates = self.reorder_candidates(signatures, call_chain_flags);
        // Go stores the candidates slice itself in *candidatesOutArray, so later in-place updates of
        // s.candidates (chooseOverload, pickLongestCandidateSignature) are visible to the caller. We
        // re-copy the candidates before every return below to preserve that aliasing.
        if let Some(out) = candidates_out_array.as_deref_mut() {
            out.clone_from(&s.candidates);
        }

        if s.candidates.is_empty() {
            // In Strada we would error here, but no known repro doesn't have at least
            // one other error in this codepath. Just return instead. See #54442
            return self.unknown_signature;
        }

        s.args = self.get_effective_call_arguments(node);
        // The excludeArgument array contains true for each context sensitive argument (an argument
        // is context sensitive it is susceptible to a one-time permanent contextual typing).
        //
        // The idea is that we will perform type argument inference & assignability checking once
        // without using the susceptible parameters that are functions, and once more for those
        // parameters, contextually typing each as we go along.
        //
        // For a tagged template, then the first argument be 'undefined' if necessary because it
        // represents a TemplateStringsArray.
        //
        // For a decorator, no arguments are susceptible to contextual typing due to the fact
        // decorators are applied to a declaration by the emitter, and not to an expression.
        s.is_single_non_generic_candidate = s.candidates.len() == 1 && s.candidates[0].type_parameters().is_empty();
        let mut some_context_sensitive = false;
        if !is_decorator && !s.is_single_non_generic_candidate {
            let args = s.args.clone();
            for arg in args {
                if self.is_context_sensitive(arg) {
                    some_context_sensitive = true;
                    break;
                }
            }
        }
        if some_context_sensitive {
            s.arg_check_mode = CheckMode::SkipContextSensitive;
        } else {
            s.arg_check_mode = CheckMode::Normal;
        }
        // The following variables are captured and modified by calls to chooseOverload.
        // If overload resolution or type argument inference fails, we want to report the
        // best error possible. The best error is one which says that an argument was not
        // assignable to a parameter. This implies that everything else about the overload
        // was fine. So if there is any overload that is only incorrect because of an
        // argument, we will report an error on that one.
        //
        //     function foo(s: string): void;
        //     function foo(n: number): void; // Report argument error on this overload
        //     function foo(): void;
        //     foo(true);
        //
        // If none of the overloads even made it that far, there are two possibilities.
        // There was a problem with type arguments for some overload, in which case
        // report an error on that. Or none of the overloads even had correct arity,
        // in which case give an arity error.
        //
        //     function foo<T extends string>(x: T): void; // Report type argument error
        //     function foo(): void;
        //     foo<number>(0);
        //
        // If we are in signature help, a trailing comma indicates that we intend to provide another argument,
        // so we will only accept overloads with arity at least 1 higher than the current number of provided arguments.
        s.signature_help_trailing_comma = check_mode.intersects(CheckMode::IsForSignatureHelp) && is_call_expression(node) && node.argument_list().unwrap().has_trailing_comma();
        // Section 4.12.1:
        // if the candidate list contains one or more signatures for which the type of each argument
        // expression is a subtype of each corresponding parameter type, the return type of the first
        // of those signatures becomes the return type of the function call.
        // Otherwise, the return type of the first signature in the candidate list becomes the return
        // type of the function call.
        //
        // Whether the call is an error is determined by assignability of the arguments. The subtype pass
        // is just important for choosing the best signature. So in the case where there is only one
        // signature, the subtype pass is useless. So skipping it is an optimization.
        let mut result: Option<P<Signature>> = None;
        let s_node = s.node.unwrap();
        s.recursive_resolution = self.call_resolution_stack.contains(&s_node);
        self.call_resolution_stack.push(s_node);
        if s.candidates.len() > 1 {
            let subtype_relation = self.subtype_relation;
            result = self.choose_overload(&mut s, subtype_relation);
        }
        if result.is_none() {
            let assignable_relation = self.assignable_relation;
            result = self.choose_overload(&mut s, assignable_relation);
        }
        self.call_resolution_stack.pop();
        if let Some(result) = result {
            if let Some(out) = candidates_out_array.as_deref_mut() {
                out.clone_from(&s.candidates);
            }
            return result;
        }
        let args = s.args.clone();
        let result = self.get_candidate_for_overload_failure(s_node, &mut s.candidates, &args, candidates_out_array.is_some(), check_mode);
        if let Some(out) = candidates_out_array.as_deref_mut() {
            out.clone_from(&s.candidates);
        }
        // Preemptively cache the result; getResolvedSignature will do this after we return, but
        // we need to ensure that the result is present for the error checks below so that if
        // this signature is encountered again, we handle the circularity (rather than producing a
        // different result which may produce no errors and assert). Callers of getResolvedSignature
        // don't hit this issue because they only observe this result after it's had a chance to
        // be cached, but the error reporting code below executes before getResolvedSignature sets
        // resolvedSignature.
        self.signature_links.get(node).resolved_signature.set(Some(result));
        // No signatures were applicable. Now report errors based on the last applicable signature with
        // no arguments excluded from assignability checks.
        // If candidate is undefined, it means that no candidates had a suitable arity. In that case,
        // skip the checkApplicableSignature check.
        if report_errors {
            // If the call expression is a synthetic call to a `[Symbol.hasInstance]` method then we will produce a head
            // message when reporting diagnostics that explains how we got to `right[Symbol.hasInstance](left)` from
            // `left instanceof right`, as it pertains to "Argument" related messages reported for the call.
            if head_message.is_none() && is_instanceof {
                head_message = Some(&diagnostics::The_left_hand_side_of_an_instanceof_expression_must_be_assignable_to_the_first_argument_of_the_right_hand_side_s_Symbol_hasInstance_method);
            }
            self.report_call_resolution_errors(node, &mut s, signatures, head_message);
        }
        result
    }

    // checker.go:9145
    pub(crate) fn reorder_candidates(&mut self, signatures: &[P<Signature>], call_chain_flags: SignatureFlags) -> Vec<P<Signature>> {
        let mut last_parent: Option<P<Node>> = None;
        let mut last_symbol: Option<P<Symbol>> = None;
        let mut index: usize = 0;
        let mut cutoff_index: usize = 0;
        let mut splice_index: usize;
        let mut specialized_index: i32 = -1;
        let mut result: Vec<P<Signature>> = Vec::with_capacity(signatures.len());
        for &signature in signatures {
            let mut signature = signature;
            let mut symbol: Option<P<Symbol>> = None;
            let mut parent: Option<P<Node>> = None;
            if let Some(declaration) = signature.declaration() {
                symbol = self.get_symbol_of_declaration(declaration);
                parent = declaration.parent();
            }
            if last_symbol.is_none() || symbol == last_symbol {
                if last_parent.is_some() && parent == last_parent {
                    index += 1;
                } else {
                    last_parent = parent;
                    index = cutoff_index;
                }
            } else {
                // current declaration belongs to a different symbol
                // set cutoffIndex so re-orderings in the future won't change result set from 0 to cutoffIndex
                index = result.len();
                cutoff_index = result.len();
                last_parent = parent;
            }
            last_symbol = symbol;
            // specialized signatures always need to be placed before non-specialized signatures regardless
            // of the cutoff position; see GH#1133
            if signature_has_literal_types(signature) {
                specialized_index += 1;
                splice_index = specialized_index as usize;
                // The cutoff index always needs to be greater than or equal to the specialized signature index
                // in order to prevent non-specialized signatures from being added before a specialized
                // signature.
                cutoff_index += 1;
            } else {
                splice_index = index;
            }
            if !call_chain_flags.is_empty() {
                signature = self.get_optional_call_signature(signature, call_chain_flags);
            }
            result.insert(splice_index, signature);
        }
        result
    }
}

// checker.go:9195
pub(crate) fn signature_has_literal_types(s: P<Signature>) -> bool {
    s.flags().intersects(SignatureFlags::HasLiteralTypes)
}

impl Checker {
    // checker.go:9199
    pub(crate) fn get_optional_call_signature(&mut self, signature: P<Signature>, call_chain_flags: SignatureFlags) -> P<Signature> {
        if signature.flags() & SignatureFlags::CallChainFlags == call_chain_flags {
            return signature;
        }
        let key = CachedSignatureKey { sig: signature, key: if call_chain_flags == SignatureFlags::IsInnerCallChain { SignatureKeyInner } else { SignatureKeyOuter } };
        if let Some(cached) = self.cached_signatures.get(&key) {
            return cached;
        }
        let result = self.clone_signature(signature);
        result.flags.set(result.flags.get() | call_chain_flags);
        self.cached_signatures.insert(key, result);
        result
    }

    // checker.go:9213
    pub(crate) fn choose_overload(&mut self, s: &mut CallState, relation: P<Relation>) -> Option<P<Signature>> {
        s.candidates_for_argument_error = Vec::new();
        s.candidate_for_argument_arity_error = None;
        s.candidate_for_type_argument_error = None;
        let node = s.node.unwrap();
        let args = s.args.clone();
        if s.is_single_non_generic_candidate {
            let candidate = s.candidates[0];
            if !s.type_arguments.is_empty() || !self.has_correct_arity(node, &args, candidate, s.signature_help_trailing_comma) {
                return None;
            }
            if !self.is_signature_applicable(node, &args, candidate, relation, CheckMode::Normal, false /*reportErrors*/, None /*diagnosticOutput*/) {
                s.candidates_for_argument_error = vec![candidate];
                return None;
            }
            return Some(candidate);
        }
        let type_arguments = s.type_arguments.clone();
        for candidate_index in 0..s.candidates.len() {
            let candidate = s.candidates[candidate_index];
            if !self.has_correct_type_argument_arity(candidate, &type_arguments) || !self.has_correct_arity(node, &args, candidate, s.signature_help_trailing_comma) {
                continue;
            }
            let mut inference_context: Option<P<InferenceContext>> = None;
            let chosen = self.choose_overload_candidate(s, relation, node, &args, &type_arguments, candidate_index, &mut inference_context);
            // 94% of these contexts are garbage once the candidate is decided (notes/mem-census.md); the inference
            // context stack entries that referred to it were popped by `inferTypeArguments`.
            if let Some(ctx) = inference_context {
                InferenceContext::recycle(ctx);
            }
            if chosen.is_some() {
                return chosen;
            }
        }
        None
    }

    /// One iteration of `chooseOverload`'s candidate loop (`None` = `continue`).
    fn choose_overload_candidate(
        &mut self,
        s: &mut CallState,
        relation: P<Relation>,
        node: P<Node>,
        args: &[P<Node>],
        type_arguments: &[P<Node>],
        candidate_index: usize,
        inference_context: &mut Option<P<InferenceContext>>,
    ) -> Option<P<Signature>> {
        {
            let candidate = s.candidates[candidate_index];
            let mut check_candidate: P<Signature>;
            if !candidate.type_parameters().is_empty() {
                let type_argument_types: Vec<P<Type>>;
                if !type_arguments.is_empty() {
                    type_argument_types = self.check_type_arguments(candidate, type_arguments, false /*reportErrors*/, None);
                    if type_argument_types.is_empty() {
                        s.candidate_for_type_argument_error = Some(candidate);
                        return None;
                    }
                } else {
                    // When we are recursively resolving a call with a single candidate, we skip constraints checks during
                    // type inference to avoid circularity errors. For example, see #64192.
                    let inference_flags = (if s.recursive_resolution && s.candidates.len() == 1 { InferenceFlags::NoConstraintChecks } else { InferenceFlags::None })
                        | (if is_in_js_file(node) { InferenceFlags::AnyDefault } else { InferenceFlags::None });
                    let ctx = self.new_inference_context(candidate.type_parameters(), Some(candidate), inference_flags /*flags*/, None);
                    *inference_context = Some(ctx);
                    type_argument_types = self.infer_type_arguments(node, candidate, args, s.arg_check_mode | CheckMode::SkipGenericFunctions, ctx);
                    if ctx.flags.get().intersects(InferenceFlags::SkippedGenericFunction) {
                        s.arg_check_mode |= CheckMode::SkipGenericFunctions;
                    }
                }
                let inferred_type_parameters: &[P<Type>] = match *inference_context {
                    Some(ctx) => ctx.inferred_type_parameters(),
                    None => &[],
                };
                check_candidate = self.get_signature_instantiation(candidate, &type_argument_types, is_in_js_file(candidate.declaration()), inferred_type_parameters);
                // If the original signature has a generic rest type, instantiation may produce a
                // signature with different arity and we need to perform another arity check.
                if self.get_non_array_rest_type(candidate).is_some() && !self.has_correct_arity(node, args, check_candidate, s.signature_help_trailing_comma) {
                    s.candidate_for_argument_arity_error = Some(check_candidate);
                    return None;
                }
            } else {
                check_candidate = candidate;
            }
            if !self.is_signature_applicable(node, args, check_candidate, relation, s.arg_check_mode, false /*reportErrors*/, None /*diagnosticOutput*/) {
                // Give preference to error candidates that have no rest parameters (as they are more specific)
                s.candidates_for_argument_error.push(check_candidate);
                return None;
            }
            if !s.arg_check_mode.is_empty() {
                // If one or more context sensitive arguments were excluded, we start including
                // them now (and keeping do so for any subsequent candidates) and perform a second
                // round of type inference and applicability checking for this particular candidate.
                s.arg_check_mode = CheckMode::Normal;
                if let Some(ctx) = *inference_context {
                    let type_argument_types = self.infer_type_arguments(node, candidate, args, s.arg_check_mode, ctx);
                    check_candidate = self.get_signature_instantiation(candidate, &type_argument_types, is_in_js_file(candidate.declaration()), ctx.inferred_type_parameters());
                    // If the original signature has a generic rest type, instantiation may produce a
                    // signature with different arity and we need to perform another arity check.
                    if self.get_non_array_rest_type(candidate).is_some() && !self.has_correct_arity(node, args, check_candidate, s.signature_help_trailing_comma) {
                        s.candidate_for_argument_arity_error = Some(check_candidate);
                        return None;
                    }
                }
                if !self.is_signature_applicable(node, args, check_candidate, relation, s.arg_check_mode, false /*reportErrors*/, None /*diagnosticOutput*/) {
                    // Give preference to error candidates that have no rest parameters (as they are more specific)
                    s.candidates_for_argument_error.push(check_candidate);
                    return None;
                }
            }
            s.candidates[candidate_index] = check_candidate;
            Some(check_candidate)
        }
    }

    // checker.go:9299
    pub(crate) fn has_correct_arity(&mut self, node: P<Node>, args: &[P<Node>], signature: P<Signature>, signature_help_trailing_comma: bool) -> bool {
        if is_jsx_opening_fragment(node) {
            return true;
        }
        let arg_count: i32;
        let mut call_is_incomplete = false;
        // In incomplete call we want to be lenient when we have too few arguments
        let mut effective_parameter_count = self.get_parameter_count(signature);
        let mut effective_minimum_arguments = self.get_min_argument_count(signature);
        if is_tagged_template_expression(node) {
            arg_count = args.len() as i32;
            let template = node.as_tagged_template_expression().template;
            if is_template_expression(template) {
                // If a tagged template expression lacks a tail literal, the call is incomplete.
                // Specifically, a template only can end in a TemplateTail or a Missing literal.
                let last_span = template.as_template_expression().template_spans.nodes().last().copied().unwrap();
                // we should always have at least one span.
                let literal = last_span.as_template_span().literal;
                call_is_incomplete = node_is_missing(literal) || is_unterminated_literal(literal);
            } else {
                // If the template didn't end in a backtick, or its beginning occurred right prior to EOF,
                // then this might actually turn out to be a TemplateHead in the future;
                // so we consider the call to be incomplete.
                call_is_incomplete = is_unterminated_literal(template);
            }
        } else if is_decorator(node) {
            arg_count = self.get_decorator_argument_count(node, signature);
        } else if is_binary_expression(node) {
            arg_count = 1;
        } else if is_jsx_opening_like_element(node) {
            call_is_incomplete = node.attributes().unwrap().end() == node.end();
            if call_is_incomplete {
                return true;
            }
            arg_count = if effective_minimum_arguments == 0 { args.len() as i32 } else { 1 };
            effective_parameter_count = if args.is_empty() { effective_parameter_count } else { 1 }; // class may have argumentless ctor functions - still resolve ctor and compare vs props member type
            effective_minimum_arguments = effective_minimum_arguments.min(1); // sfc may specify context argument - handled by framework and not typechecked
        } else if is_new_expression(node) && node.argument_list().is_none() {
            // This only happens when we have something of the form: 'new C'
            return self.get_min_argument_count(signature) == 0;
        } else {
            if signature_help_trailing_comma {
                arg_count = args.len() as i32 + 1;
            } else {
                arg_count = args.len() as i32;
            }
            // If we are missing the close parenthesis, the call is incomplete.
            call_is_incomplete = node.argument_list().unwrap().end() == node.end();
            // If a spread argument is present, check that it corresponds to a rest parameter or at least that it's in the valid range.
            let spread_arg_index = self.get_spread_argument_index(args);
            if spread_arg_index >= 0 {
                return spread_arg_index >= self.get_min_argument_count(signature) && (self.has_effective_rest_parameter(signature) || spread_arg_index < self.get_parameter_count(signature));
            }
        }
        // Too many arguments implies incorrect arity.
        if !self.has_effective_rest_parameter(signature) && arg_count > effective_parameter_count {
            return false;
        }
        // If the call is incomplete, we should skip the lower bound check.
        // JSX signatures can have extra parameters provided by the library which we don't check
        if call_is_incomplete || arg_count >= effective_minimum_arguments {
            return true;
        }
        for i in arg_count..effective_minimum_arguments {
            let t = self.get_type_at_position(signature, i);
            if self.filter_type(t, |_, t| accepts_void(t)).flags().intersects(TypeFlags::Never) {
                return false;
            }
        }
        true
    }
}

// checker.go:9371
pub(crate) fn accepts_void(t: P<Type>) -> bool {
    t.flags().intersects(TypeFlags::Void)
}

impl Checker {
    // checker.go:9375
    pub(crate) fn get_decorator_argument_count(&mut self, node: P<Node>, signature: P<Signature>) -> i32 {
        if self.compiler_options.experimental_decorators.is_true() {
            return self.get_legacy_decorator_argument_count(node, signature);
        }
        self.get_parameter_count(signature).max(1).min(2)
    }

    /**
     * Returns the argument count for a decorator node that works like a function invocation.
     */
    // checker.go:9385
    pub(crate) fn get_legacy_decorator_argument_count(&mut self, node: P<Node>, signature: P<Signature>) -> i32 {
        let parent = node.parent().unwrap();
        match parent.kind() {
            Kind::ClassDeclaration | Kind::ClassExpression => 1,
            Kind::PropertyDeclaration => {
                if has_accessor_modifier(parent) {
                    return 3;
                }
                2
            }
            Kind::MethodDeclaration | Kind::GetAccessor | Kind::SetAccessor => {
                // For decorators with only two parameters we supply only two arguments
                if self.get_parameter_count(signature) <= 2 {
                    return 2;
                }
                3
            }
            Kind::Parameter => 3,
            _ => panic!("Unhandled case in getLegacyDecoratorArgumentCount"),
        }
    }

    // checker.go:9406
    pub(crate) fn has_correct_type_argument_arity(&mut self, signature: P<Signature>, type_arguments: &[P<Node>]) -> bool {
        // If the user supplied type arguments, but the number of type arguments does not match
        // the declared number of type parameters, the call has an incorrect arity.
        let num_type_parameters = signature.type_parameters().len() as i32;
        let min_type_argument_count = self.get_min_type_argument_count(signature.type_parameters());
        let len = type_arguments.len() as i32;
        len == 0 || len >= min_type_argument_count && len <= num_type_parameters
    }

    // checker.go:9414
    pub(crate) fn check_type_arguments(&mut self, signature: P<Signature>, type_argument_nodes: &[P<Node>], report_errors: bool, head_message: Option<&'static Message>) -> Vec<P<Type>> {
        let is_java_script = is_in_js_file(signature.declaration());
        let type_parameters = signature.type_parameters();
        let mut type_argument_node_types = Vec::with_capacity(type_argument_nodes.len());
        for &n in type_argument_nodes {
            type_argument_node_types.push(self.get_type_from_type_node(n));
        }
        let min_type_argument_count = self.get_min_type_argument_count(type_parameters);
        let type_argument_types = self.fill_missing_type_arguments(&type_argument_node_types, type_parameters, min_type_argument_count, is_java_script);
        let mut mapper: Option<P<TypeMapper>> = None;
        for i in 0..type_argument_nodes.len() {
            assert!(i < type_parameters.len(), "Should not call checkTypeArguments with too many type arguments");
            let constraint = self.get_constraint_of_type_parameter(type_parameters[i]);
            if let Some(constraint) = constraint {
                let type_argument_head_message = head_message.unwrap_or(&diagnostics::Type_0_does_not_satisfy_the_constraint_1);
                if mapper.is_none() {
                    mapper = Some(new_type_mapper(type_parameters, alloc_slice(&type_argument_types)));
                }
                let type_argument = type_argument_types[i];
                let error_node = if report_errors { Some(type_argument_nodes[i]) } else { None };
                let mut diags: Vec<P<Diagnostic>> = Vec::new();
                let instantiated_constraint = self.instantiate_type(constraint, mapper);
                let target = self.get_type_with_this_argument(instantiated_constraint, Some(type_argument), false);
                if !self.check_type_assignable_to_ex(type_argument, target, error_node, Some(type_argument_head_message), &mut diags) {
                    if !diags.is_empty() {
                        let mut diagnostic = diags[0];
                        if head_message.is_some() {
                            diagnostic = ast::new_diagnostic_chain(diagnostic, &diagnostics::Type_0_does_not_satisfy_the_constraint_1, &[]);
                        }
                        self.add_diagnostic(diagnostic);
                    }
                    return Vec::new();
                }
            }
        }
        type_argument_types
    }

    // checker.go:9448
    pub(crate) fn is_signature_applicable(&mut self, node: P<Node>, args: &[P<Node>], signature: P<Signature>, relation: P<Relation>, check_mode: CheckMode, report_errors: bool, diagnostic_output: Option<&mut Vec<P<Diagnostic>>>) -> bool {
        let mut diagnostic_output = diagnostic_output;
        if is_jsx_call_like(node) {
            return self.check_applicable_signature_for_jsx_call_like_element(node, signature, relation, check_mode, report_errors, diagnostic_output);
        }
        let this_type = self.get_this_type_of_signature(signature);
        if let Some(this_type) = this_type {
            if this_type != self.void_type && !(is_new_expression(node) || is_call_expression(node) && is_super_property(node.expression().unwrap())) {
                // If the called expression is not of the form `x.f` or `x["f"]`, then sourceType = voidType
                // If the signature's 'this' type is voidType, then the check is skipped -- anything is compatible.
                // If the expression is a new expression or super call expression, then the check is skipped.
                let this_argument_node = self.get_this_argument_of_call(node);
                let this_argument_type = self.get_this_argument_type(this_argument_node);
                let mut error_node: Option<P<Node>> = None;
                if report_errors {
                    error_node = this_argument_node;
                    if error_node.is_none() {
                        error_node = Some(node);
                    }
                }
                let head_message = &diagnostics::The_this_context_of_type_0_is_not_assignable_to_method_s_this_of_type_1;
                if !self.check_type_related_to_ex(this_argument_type, this_type, relation, error_node, Some(head_message), diagnostic_output.as_deref_mut()) {
                    return false;
                }
            }
        }
        let head_message = &diagnostics::Argument_of_type_0_is_not_assignable_to_parameter_of_type_1;
        let rest_type = self.get_non_array_rest_type(signature);
        let arg_count: i32 = if rest_type.is_some() {
            (self.get_parameter_count(signature) - 1).min(args.len() as i32)
        } else {
            args.len() as i32
        };
        for i in 0..arg_count {
            let arg = args[i as usize];
            if !is_omitted_expression(arg) {
                let param_type = self.get_type_at_position(signature, i);
                let arg_type = self.check_expression_with_contextual_type(arg, param_type, None /*inferenceContext*/, check_mode);
                // If one or more arguments are still excluded (as indicated by CheckMode.SkipContextSensitive),
                // we obtain the regular type of any object literal arguments because we may not have inferred complete
                // parameter types yet and therefore excess property checks may yield false positives (see #17041).
                let check_arg_type = if check_mode.intersects(CheckMode::SkipContextSensitive) {
                    self.get_regular_type_of_object_literal(arg_type)
                } else {
                    arg_type
                };
                let effective_check_argument_node = self.get_effective_check_node(arg);
                if !self.check_type_related_to_and_optionally_elaborate(check_arg_type, param_type, relation, if report_errors { effective_check_argument_node } else { None }, effective_check_argument_node, Some(head_message), diagnostic_output.as_deref_mut()) {
                    self.maybe_add_missing_await_info(Some(arg), check_arg_type, param_type, relation, report_errors, diagnostic_output.as_deref_mut());
                    return false;
                }
            }
        }
        if let Some(rest_type) = rest_type {
            let spread_type = self.get_spread_argument_type(args, arg_count, args.len() as i32, rest_type, None /*context*/, check_mode);
            let rest_arg_count = args.len() as i32 - arg_count;
            let mut error_node: Option<P<Node>> = None;
            if report_errors {
                match rest_arg_count {
                    0 => {
                        error_node = Some(node);
                    }
                    1 => {
                        error_node = self.get_effective_check_node(args[arg_count as usize]);
                    }
                    _ => {
                        let synthetic = self.create_synthetic_expression(node, spread_type, false, None);
                        synthetic.set_loc(TextRange::new(args[arg_count as usize].pos(), args[args.len() - 1].end()));
                        error_node = Some(synthetic);
                    }
                }
            }
            if !self.check_type_related_to_ex(spread_type, rest_type, relation, error_node, Some(head_message), diagnostic_output.as_deref_mut()) {
                self.maybe_add_missing_await_info(error_node, spread_type, rest_type, relation, report_errors, diagnostic_output.as_deref_mut());
                return false;
            }
        }
        true
    }

    // checker.go:9523
    pub(crate) fn maybe_add_missing_await_info(&mut self, error_node: Option<P<Node>>, source: P<Type>, target: P<Type>, relation: P<Relation>, report_errors: bool, diagnostic_output: Option<&mut Vec<P<Diagnostic>>>) {
        if let (Some(error_node), true, Some(diagnostic_output)) = (error_node, report_errors, diagnostic_output) {
            if diagnostic_output.is_empty() {
                return;
            }
            // Bail if target is Promise-like---something else is wrong
            if self.get_awaited_type_of_promise(target).is_some() {
                return;
            }
            let awaited_type_of_source = self.get_awaited_type_of_promise(source);
            if let Some(awaited_type_of_source) = awaited_type_of_source {
                if self.is_type_related_to(awaited_type_of_source, target, relation) {
                    diagnostic_output[0].add_related_info(new_diagnostic_for_node(Some(error_node), Some(&diagnostics::Did_you_forget_to_use_await), &[]));
                }
            }
        }
    }

    // Returns the `this` argument node in calls like `x.f(...)` and `x[f](...)`. `nil` otherwise.
    // checker.go:9537
    pub(crate) fn get_this_argument_of_call(&mut self, node: P<Node>) -> Option<P<Node>> {
        if is_binary_expression(node) {
            return Some(node.as_binary_expression().right());
        }
        let mut expression: Option<P<Node>> = None;
        if is_call_expression(node) {
            expression = node.expression();
        } else if is_tagged_template_expression(node) {
            expression = Some(node.as_tagged_template_expression().tag);
        } else if is_decorator(node) && !self.legacy_decorators {
            expression = node.expression();
        }
        if let Some(expression) = expression {
            let callee = skip_outer_expressions(expression, OuterExpressionKinds::All);
            if is_access_expression(callee) {
                return callee.expression();
            }
        }
        None
    }

    // checker.go:9559
    pub(crate) fn get_this_argument_type(&mut self, node: Option<P<Node>>) -> P<Type> {
        let Some(node) = node else {
            return self.void_type;
        };
        let this_argument_type = self.check_expression(node);
        let parent = node.parent().unwrap();
        if is_optional_chain_root(parent) {
            return self.get_non_nullable_type(this_argument_type);
        } else if is_optional_chain(parent) {
            return self.remove_optional_type_marker(this_argument_type);
        }
        this_argument_type
    }

    // checker.go:9573
    pub(crate) fn get_effective_check_node(&mut self, argument: P<Node>) -> Option<P<Node>> {
        let flags = if is_in_js_file(argument) {
            OuterExpressionKinds::Parentheses | OuterExpressionKinds::Satisfies | OuterExpressionKinds::ExcludeJSDocTypeAssertion
        } else {
            OuterExpressionKinds::Parentheses | OuterExpressionKinds::Satisfies
        };
        Some(skip_outer_expressions(argument, flags))
    }

    // checker.go:9582
    #[cfg_attr(not(feature = "work-census"), inline(always), expect(clippy::inline_always, reason = "without the census the wrapper is a forwarding call; inlined, callers call the body as before (notes/perf-checker-algorithms.md)"))]
    pub(crate) fn infer_type_arguments(&mut self, node: P<Node>, signature: P<Signature>, args: &[P<Node>], check_mode: CheckMode, context: P<InferenceContext>) -> Vec<P<Type>> {
        if self.census_on() {
            let start = self.census.as_ref().unwrap().infer_args.len();
            let span = self.census_begin(crate::workcensus::Cat::Infer, || crate::workcensus::CKey::OptNode(signature.declaration()));
            let r = self.infer_type_arguments_worker(node, signature, args, check_mode, context);
            let timing = self.census_end(span).unwrap();
            let census = self.census.as_mut().unwrap();
            let mut words: Vec<u32> = vec![signature.to_bits() as u32, check_mode.bits() as u32];
            words.extend_from_slice(&census.infer_args[start..]);
            census.infer_args.truncate(start);
            let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
            let key = xxhash_rust::xxh3::xxh3_128(&bytes);
            let e = census.infer_keys.entry(key).or_insert((0, 0, node.to_bits(), 0, signature.declaration()));
            e.0 += 1;
            e.1 += timing.incl_ns;
            if e.2 != node.to_bits() {
                e.3 += 1;
            }
            census.record(crate::workcensus::Cat::Infer, crate::workcensus::CKey::OptNode(signature.declaration()), timing, args.len() as u64, 0, 0);
            return r;
        }
        self.infer_type_arguments_worker(node, signature, args, check_mode, context)
    }

    fn infer_type_arguments_worker(&mut self, node: P<Node>, signature: P<Signature>, args: &[P<Node>], check_mode: CheckMode, context: P<InferenceContext>) -> Vec<P<Type>> {
        if is_jsx_opening_like_element(node) {
            return self.infer_jsx_type_arguments(node, signature, check_mode, context);
        }
        // If a contextual type is available, infer from that type to the return type of the call expression. For
        // example, given a 'function wrap<T, U>(cb: (x: T) => U): (x: T) => U' and a call expression
        // 'let f: (x: string) => number = wrap(s => s.length)', we infer from the declared type of 'f' to the
        // return type of 'wrap'.
        if !is_decorator(node) && !is_binary_expression(node) {
            let mut skip_binding_patterns = true;
            for &p in signature.type_parameters() {
                if self.get_default_from_type_parameter(p).is_none() {
                    skip_binding_patterns = false;
                    break;
                }
            }
            let contextual_type = self.get_contextual_type(node, if skip_binding_patterns { ContextFlags::SkipBindingPatterns } else { ContextFlags::None });
            if let Some(contextual_type) = contextual_type {
                if let Some(census) = self.census_mut() {
                    census.infer_args.push(contextual_type.id.0 | 0x8000_0000);
                }
                let inference_target_type = self.get_return_type_of_signature(signature);
                if self.could_contain_type_variables(inference_target_type) {
                    let outer_context = self.get_inference_context(node);
                    let is_from_binding_pattern = !skip_binding_patterns && self.get_contextual_type(node, ContextFlags::SkipBindingPatterns) != Some(contextual_type);
                    // A return type inference from a binding pattern can be used in instantiating the contextual
                    // type of an argument later in inference, but cannot stand on its own as the final return type.
                    // It is incorporated into `context.returnMapper` which is used in `instantiateContextualType`,
                    // but doesn't need to go into `context.inferences`. This allows a an array binding pattern to
                    // produce a tuple for `T` in
                    //   declare function f<T>(cb: () => T): T;
                    //   const [e1, e2, e3] = f(() => [1, "hi", true]);
                    // but does not produce any inference for `T` in
                    //   declare function f<T>(): T;
                    //   const [e1, e2, e3] = f();
                    if !is_from_binding_pattern {
                        // We clone the inference context to avoid disturbing a resolution in progress for an
                        // outer call expression. Effectively we just want a snapshot of whatever has been
                        // inferred for any outer call expression so far.
                        let cloned = self.clone_inference_context(outer_context, InferenceFlags::NoDefault);
                        let outer_mapper = self.get_mapper_from_context(cloned);
                        let instantiated_type = self.instantiate_type(contextual_type, outer_mapper);
                        // If the contextual type is a generic function type with a single call signature, we
                        // instantiate the type with its own type parameters and type arguments. This ensures that
                        // the type parameters are not erased to type any during type inference such that they can
                        // be inferred as actual types from the contextual type. For example:
                        //   declare function arrayMap<T, U>(f: (x: T) => U): (a: T[]) => U[];
                        //   const boxElements: <A>(a: A[]) => { value: A }[] = arrayMap(value => ({ value }));
                        // Above, the type of the 'value' parameter is inferred to be 'A'.
                        let contextual_signature = self.get_single_call_signature(instantiated_type);
                        let inference_source_type = match contextual_signature {
                            Some(contextual_signature) if !contextual_signature.type_parameters().is_empty() => {
                                let instantiated = self.get_signature_instantiation_without_filling_in_type_arguments(contextual_signature, contextual_signature.type_parameters());
                                self.get_or_create_type_from_signature(instantiated)
                            }
                            _ => instantiated_type,
                        };
                        // Inferences made from return types have lower priority than all other inferences.
                        self.infer_types(context.inferences.get(), inference_source_type, inference_target_type, InferencePriority::ReturnType, false);
                        // The snapshot is garbage now unless its mapper was stored (notes/mem-scoped-arenas.md).
                        if let Some(cloned) = cloned {
                            InferenceContext::recycle(cloned);
                        }
                    }
                    // Create a type mapper for instantiating generic contextual types using the inferences made
                    // from the return type. We need a separate inference pass here because (a) instantiation of
                    // the source type uses the outer context's return mapper (which excludes inferences made from
                    // outer arguments), and (b) we don't want any further inferences going into this context.
                    // We use `createOuterReturnMapper` to ensure that all occurrences of outer type parameters are
                    // replaced with inferences produced from the outer return type or preceding outer arguments.
                    // This protects against circular inferences, i.e. avoiding situations where inferences reference
                    // type parameters for which the inferences are being made.
                    let return_context = self.new_inference_context(signature.type_parameters(), Some(signature), context.flags.get(), None);
                    let mut outer_return_mapper: Option<P<TypeMapper>> = None;
                    if let Some(outer_context) = outer_context {
                        outer_return_mapper = Some(self.create_outer_return_mapper(outer_context));
                    }
                    let return_source_type = self.instantiate_type(contextual_type, outer_return_mapper);
                    self.infer_types(return_context.inferences.get(), return_source_type, inference_target_type, InferencePriority::None, false);
                    if return_context.inferences.get().iter().any(|&info| has_inference_candidates(info)) {
                        let cloned = self.clone_inferred_part_of_context(return_context);
                        let return_mapper = self.get_mapper_from_context(cloned);
                        context.set_return_mapper(return_mapper);
                    } else {
                        context.set_return_mapper(None);
                    }
                    // Its inferred part was copied (`cloneInferredPartOfContext` clones the infos).
                    InferenceContext::recycle(return_context);
                }
            }
        }
        let rest_type = self.get_non_array_rest_type(signature);
        let mut arg_count = args.len() as i32;
        if rest_type.is_some() {
            arg_count = (self.get_parameter_count(signature) - 1).min(arg_count);
        }
        if let Some(rest_type) = rest_type {
            if rest_type.flags().intersects(TypeFlags::TypeParameter) {
                let info = context.inferences.get().iter().copied().find(|info| info.type_parameter.get() == Some(rest_type));
                if let Some(info) = info {
                    if !args[arg_count as usize..].iter().any(|&arg| is_spread_argument(arg)) {
                        info.implied_arity.set(args.len() as i32 - arg_count);
                    }
                }
            }
        }
        let this_type = self.get_this_type_of_signature(signature);
        if let Some(this_type) = this_type {
            if self.could_contain_type_variables(this_type) {
                let this_argument_node = self.get_this_argument_of_call(node);
                let this_argument_type = self.get_this_argument_type(this_argument_node);
                self.infer_types(context.inferences.get(), this_argument_type, this_type, InferencePriority::None, false);
            }
        }
        for i in 0..arg_count {
            let arg = args[i as usize];
            if arg.kind() != Kind::OmittedExpression {
                let param_type = self.get_type_at_position(signature, i);
                if self.could_contain_type_variables(param_type) {
                    let arg_type = self.check_expression_with_contextual_type(arg, param_type, Some(context), check_mode);
                    if let Some(census) = self.census_mut() {
                        census.infer_args.push(arg_type.id.0);
                    }
                    self.infer_types(context.inferences.get(), arg_type, param_type, InferencePriority::None, false);
                }
            }
        }
        if let Some(rest_type) = rest_type {
            if self.could_contain_type_variables(rest_type) {
                let spread_type = self.get_spread_argument_type(args, arg_count, args.len() as i32, rest_type, Some(context), check_mode);
                self.infer_types(context.inferences.get(), spread_type, rest_type, InferencePriority::None, false);
            }
        }
        self.get_inferred_types(context)
    }

    // No signature was applicable. We have already reported the errors for the invalid signature.
    // checker.go:9690
    pub(crate) fn get_candidate_for_overload_failure(&mut self, node: P<Node>, candidates: &mut [P<Signature>], args: &[P<Node>], has_candidates_out_array: bool, check_mode: CheckMode) -> P<Signature> {
        // Else should not have called this.
        self.check_node_deferred(node);
        // Normally we will combine overloads. Skip this if they have type parameters since that's hard to combine.
        // Don't do this if there is a `candidatesOutArray`,
        // because then we want the chosen best candidate to be one of the overloads, not a combination.
        if has_candidates_out_array || candidates.len() == 1 || candidates.iter().any(|s| !s.type_parameters().is_empty()) {
            return self.pick_longest_candidate_signature(node, candidates, args, check_mode);
        }
        self.create_union_of_signatures_for_overload_failure(candidates)
    }

    // checker.go:9702
    pub(crate) fn pick_longest_candidate_signature(&mut self, node: P<Node>, candidates: &mut [P<Signature>], args: &[P<Node>], check_mode: CheckMode) -> P<Signature> {
        // Pick the longest signature. This way we can get a contextual type for cases like:
        //     declare function f(a: { xa: number; xb: number; }, b: number);
        //     f({ |
        // Also, use explicitly-supplied type arguments if they are provided, so we can get a contextual signature in cases like:
        //     declare function f<T>(k: keyof T);
        //     f<Foo>("
        let mut arg_count = args.len() as i32;
        if let Some(apparent_argument_count) = self.apparent_argument_count {
            arg_count = apparent_argument_count;
        }
        let best_index = self.get_longest_candidate_index(candidates, arg_count) as usize;
        let candidate = candidates[best_index];
        let type_parameters = candidate.type_parameters();
        if type_parameters.is_empty() {
            return candidate;
        }
        let mut type_argument_nodes: &[P<Node>] = &[];
        if self.call_like_expression_may_have_type_arguments(node) {
            type_argument_nodes = node.type_arguments();
        }
        let instantiated = if !type_argument_nodes.is_empty() {
            let type_arguments = self.get_type_arguments_from_nodes(type_argument_nodes, type_parameters);
            self.create_signature_instantiation(candidate, &type_arguments)
        } else {
            self.infer_signature_instantiation_for_overload_failure(node, type_parameters, candidate, args, check_mode)
        };
        candidates[best_index] = instantiated;
        instantiated
    }

    // checker.go:9733
    pub(crate) fn get_longest_candidate_index(&mut self, candidates: &[P<Signature>], args_count: i32) -> i32 {
        let mut max_params_index: i32 = -1;
        let mut max_params: i32 = -1;
        for (i, &candidate) in candidates.iter().enumerate() {
            let param_count = self.get_parameter_count(candidate);
            if self.has_effective_rest_parameter(candidate) || param_count >= args_count {
                return i as i32;
            }
            if param_count > max_params {
                max_params = param_count;
                max_params_index = i as i32;
            }
        }
        max_params_index
    }

    // checker.go:9749
    pub(crate) fn get_type_arguments_from_nodes(&mut self, type_argument_nodes: &[P<Node>], type_parameters: &[P<Type>]) -> Vec<P<Type>> {
        let mut type_argument_nodes = type_argument_nodes;
        if type_argument_nodes.len() > type_parameters.len() {
            type_argument_nodes = &type_argument_nodes[..type_parameters.len()];
        }
        let mut type_arguments: Vec<P<Type>> = Vec::with_capacity(type_parameters.len());
        for &n in type_argument_nodes {
            type_arguments.push(self.get_type_from_type_node(n));
        }
        while type_arguments.len() < type_parameters.len() {
            let mut t = self.get_default_from_type_parameter(type_parameters[type_arguments.len()]);
            if t.is_none() {
                t = self.get_constraint_of_type_parameter(type_parameters[type_arguments.len()]);
                if t.is_none() {
                    t = Some(self.unknown_type);
                }
            }
            type_arguments.push(t.unwrap());
        }
        type_arguments
    }

    // checker.go:9767
    pub(crate) fn infer_signature_instantiation_for_overload_failure(&mut self, node: P<Node>, type_parameters: &[P<Type>], candidate: P<Signature>, args: &[P<Node>], check_mode: CheckMode) -> P<Signature> {
        let inference_context = self.new_inference_context(type_parameters, Some(candidate), if is_in_js_file(node) { InferenceFlags::AnyDefault } else { InferenceFlags::None }, None);
        let type_argument_types = self.infer_type_arguments(node, candidate, args, check_mode | CheckMode::SkipContextSensitive | CheckMode::SkipGenericFunctions, inference_context);
        self.create_signature_instantiation(candidate, &type_argument_types)
    }

    // checker.go:9773
    pub(crate) fn create_union_of_signatures_for_overload_failure(&mut self, candidates: &[P<Signature>]) -> P<Signature> {
        let this_parameters: Vec<P<Symbol>> = candidates.iter().filter_map(|c| c.this_parameter()).collect();
        let mut this_parameter: Option<P<Symbol>> = None;
        if !this_parameters.is_empty() {
            let mut this_types = Vec::with_capacity(this_parameters.len());
            for &p in &this_parameters {
                this_types.push(self.get_type_of_parameter(p));
            }
            this_parameter = Some(self.create_combined_symbol_from_types(&this_parameters, &this_types));
        }
        let (min_argument_count, max_non_rest_param) = min_and_max(candidates, get_non_rest_parameter_count);
        let mut parameters: Vec<P<Symbol>> = Vec::with_capacity(max_non_rest_param.max(0) as usize);
        for i in 0..max_non_rest_param {
            let symbols: Vec<P<Symbol>> = candidates
                .iter()
                .filter_map(|&s| {
                    let params = s.parameters();
                    if signature_has_rest_parameter(s) {
                        if (i as usize) < params.len().wrapping_sub(1) && params.len() >= 1 {
                            return Some(params[i as usize]);
                        }
                        return params.last().copied();
                    }
                    if (i as usize) < params.len() {
                        return Some(params[i as usize]);
                    }
                    None
                })
                .collect();
            let mut types = Vec::new();
            for &s in candidates {
                if let Some(t) = self.try_get_type_at_position(s, i) {
                    types.push(t);
                }
            }
            parameters.push(self.create_combined_symbol_from_types(&symbols, &types));
        }
        let rest_parameter_symbols: Vec<P<Symbol>> = candidates
            .iter()
            .filter_map(|&s| {
                if signature_has_rest_parameter(s) {
                    return s.parameters().last().copied();
                }
                None
            })
            .collect();
        let mut flags = SignatureFlags::IsSignatureCandidateForOverloadFailure;
        if !rest_parameter_symbols.is_empty() {
            let mut rest_types = Vec::new();
            for &s in candidates {
                if let Some(t) = self.try_get_rest_type_of_signature(s) {
                    rest_types.push(t);
                }
            }
            let union = self.get_union_type_ex(&rest_types, UnionReduction::Subtype, AliasArg::None, None);
            let t = self.create_array_type(union);
            parameters.push(self.create_combined_symbol_for_overload_failure(&rest_parameter_symbols, t));
            flags |= SignatureFlags::HasRestParameter;
        }
        if candidates.iter().any(|&s| signature_has_literal_types(s)) {
            flags |= SignatureFlags::HasLiteralTypes;
        }
        let mut return_types = Vec::with_capacity(candidates.len());
        for &s in candidates {
            return_types.push(self.get_return_type_of_signature(s));
        }
        let return_type = self.get_intersection_type(&return_types);
        self.new_signature(flags, candidates[0].declaration(), &[], this_parameter, &parameters, Some(return_type), None, min_argument_count)
    }

    // checker.go:9814
    pub(crate) fn create_combined_symbol_from_types(&mut self, sources: &[P<Symbol>], types: &[P<Type>]) -> P<Symbol> {
        let t = self.get_union_type_ex(types, UnionReduction::Subtype, AliasArg::None, None);
        self.create_combined_symbol_for_overload_failure(sources, t)
    }

    // checker.go:9818
    pub(crate) fn create_combined_symbol_for_overload_failure(&mut self, sources: &[P<Symbol>], t: P<Type>) -> P<Symbol> {
        // This function is currently only used for erroneous overloads, so it's good enough to just use the first source.
        self.create_symbol_with_type(sources.first().copied().unwrap(), Some(t))
    }

    // checker.go:9823
    pub(crate) fn get_rest_type_of_signature(&mut self, signature: P<Signature>) -> P<Type> {
        match self.try_get_rest_type_of_signature(signature) {
            Some(t) => t,
            None => self.any_type,
        }
    }

    // checker.go:9827
    pub(crate) fn try_get_rest_type_of_signature(&mut self, signature: P<Signature>) -> Option<P<Type>> {
        if !signature_has_rest_parameter(signature) {
            return None;
        }
        let parameters = signature.parameters();
        let mut rest_type = self.get_type_of_symbol(parameters[parameters.len() - 1]);
        if is_tuple_type(rest_type) {
            rest_type = self.get_rest_type_of_tuple_type(rest_type)?;
        }
        let number_type = self.number_type;
        self.get_index_type_of_type(rest_type, number_type)
    }

    // checker.go:9841
    pub(crate) fn report_call_resolution_errors(&mut self, node: P<Node>, s: &mut CallState, signatures: &[P<Signature>], head_message: Option<&'static Message>) {
        let s_node = s.node.unwrap();
        if !s.candidates_for_argument_error.is_empty() {
            let last = s.candidates_for_argument_error[s.candidates_for_argument_error.len() - 1];
            let mut diags: Vec<P<Diagnostic>> = Vec::new();
            let args = s.args.clone();
            let assignable_relation = self.assignable_relation;
            self.is_signature_applicable(s_node, &args, last, assignable_relation, CheckMode::Normal, true /*reportErrors*/, Some(&mut diags));
            for diagnostic in diags {
                let mut diagnostic = diagnostic;
                if s.candidates_for_argument_error.len() > 1 {
                    diagnostic = ast::new_diagnostic_chain(diagnostic, &diagnostics::The_last_overload_gave_the_following_error, &[]);
                    diagnostic = ast::new_diagnostic_chain(diagnostic, &diagnostics::No_overload_matches_this_call, &[]);
                }
                if let Some(head_message) = head_message {
                    diagnostic = ast::new_diagnostic_chain(diagnostic, head_message, &[]);
                }
                if last.declaration().is_some() && s.candidates_for_argument_error.len() > 1 {
                    diagnostic.add_related_info(new_diagnostic_for_node(last.declaration(), Some(&diagnostics::The_last_overload_is_declared_here), &[]));
                }
                self.add_implementation_success_elaboration(s, last, diagnostic);
                self.add_diagnostic(diagnostic);
            }
        } else if let Some(candidate_for_argument_arity_error) = s.candidate_for_argument_arity_error {
            let args = s.args.clone();
            let d = self.get_argument_arity_error(s_node, &[candidate_for_argument_arity_error], &args, head_message);
            self.add_diagnostic(d);
        } else if let Some(candidate_for_type_argument_error) = s.candidate_for_type_argument_error {
            self.check_type_arguments(candidate_for_type_argument_error, s_node.type_arguments(), true /*reportErrors*/, head_message);
        } else if !is_jsx_opening_fragment(node) {
            let type_arguments = s.type_arguments.clone();
            let mut signatures_with_correct_type_argument_arity = Vec::new();
            for &sig in signatures {
                if self.has_correct_type_argument_arity(sig, &type_arguments) {
                    signatures_with_correct_type_argument_arity.push(sig);
                }
            }
            if signatures_with_correct_type_argument_arity.is_empty() {
                let d = self.get_type_argument_arity_error(s_node, signatures, &type_arguments, head_message);
                self.add_diagnostic(d);
            } else {
                let args = s.args.clone();
                let d = self.get_argument_arity_error(s_node, &signatures_with_correct_type_argument_arity, &args, head_message);
                self.add_diagnostic(d);
            }
        }
    }

    // checker.go:9877
    pub(crate) fn add_implementation_success_elaboration(&mut self, s: &mut CallState, failed: P<Signature>, diagnostic: P<Diagnostic>) {
        if let Some(failed_declaration) = failed.declaration() {
            if let Some(symbol) = failed_declaration.symbol() {
                let declarations = symbol.declarations();
                if declarations.len() > 1 {
                    let implementation = declarations.iter().copied().find(|&d| is_function_like_declaration(d) && node_is_present(d.body()));
                    if let Some(implementation) = implementation {
                        let candidate = self.get_signature_from_declaration(implementation);
                        let mut local_state = s.clone();
                        local_state.candidates = vec![candidate];
                        local_state.is_single_non_generic_candidate = candidate.type_parameters().is_empty();
                        let assignable_relation = self.assignable_relation;
                        if self.choose_overload(&mut local_state, assignable_relation).is_some() {
                            diagnostic.add_related_info(new_diagnostic_for_node(Some(implementation), Some(&diagnostics::The_call_would_have_succeeded_against_this_implementation_but_implementation_signatures_of_overloads_are_not_externally_visible), &[]));
                        }
                    }
                }
            }
        }
    }

    // checker.go:9897
    pub(crate) fn get_argument_arity_error(&mut self, node: P<Node>, signatures: &[P<Signature>], args: &[P<Node>], head_message: Option<&'static Message>) -> P<Diagnostic> {
        let spread_index = self.get_spread_argument_index(args);
        if spread_index > -1 {
            return new_diagnostic_for_node(Some(args[spread_index as usize]), Some(&diagnostics::A_spread_argument_must_either_have_a_tuple_type_or_be_passed_to_a_rest_parameter), &[]);
        }
        let args_len = args.len() as i64;
        let mut min_count = i64::MAX; // smallest parameter count
        let mut max_count = i64::MIN; // largest parameter count
        let mut max_below = i64::MIN; // largest parameter count that is smaller than the number of arguments
        let mut min_above = i64::MAX; // smallest parameter count that is larger than the number of arguments
        let mut closest_signature: Option<P<Signature>> = None;
        for &sig in signatures {
            let min_parameter = self.get_min_argument_count(sig) as i64;
            let max_parameter = self.get_parameter_count(sig) as i64;
            // smallest/largest parameter counts
            if min_parameter < min_count {
                min_count = min_parameter;
                closest_signature = Some(sig);
            }
            max_count = max_count.max(max_parameter);
            // shortest parameter count *longer than the call*/longest parameter count *shorter than the call*
            if min_parameter < args_len && min_parameter > max_below {
                max_below = min_parameter;
            }
            if args_len < max_parameter && max_parameter < min_above {
                min_above = max_parameter;
            }
        }
        let mut has_rest_parameter = false;
        for &sig in signatures {
            if self.has_effective_rest_parameter(sig) {
                has_rest_parameter = true;
                break;
            }
        }
        let parameter_range = if has_rest_parameter {
            min_count.to_string()
        } else if min_count < max_count {
            min_count.to_string() + "-" + &max_count.to_string()
        } else {
            min_count.to_string()
        };
        let is_void_promise_error = !has_rest_parameter && parameter_range == "1" && args.is_empty() && self.is_promise_resolve_arity_error(node);
        let error_node = get_error_node_for_call_node(node);
        if is_void_promise_error && is_in_js_file(node) {
            return new_diagnostic_for_node(error_node, Some(&diagnostics::Expected_1_argument_but_got_0_new_Promise_needs_a_JSDoc_hint_to_produce_a_resolve_that_can_be_called_without_arguments), &[]);
        }
        let message: &'static Message = if is_decorator(node) {
            if has_rest_parameter {
                &diagnostics::The_runtime_will_invoke_the_decorator_with_1_arguments_but_the_decorator_expects_at_least_0
            } else {
                &diagnostics::The_runtime_will_invoke_the_decorator_with_1_arguments_but_the_decorator_expects_0
            }
        } else if has_rest_parameter {
            &diagnostics::Expected_at_least_0_arguments_but_got_1
        } else if is_void_promise_error {
            &diagnostics::Expected_0_arguments_but_got_1_Did_you_forget_to_include_void_in_your_type_argument_to_Promise
        } else {
            &diagnostics::Expected_0_arguments_but_got_1
        };
        if min_count < args_len && args_len < max_count {
            // between min and max, but with no matching overload
            let mut diagnostic = new_diagnostic_for_node(error_node, Some(&diagnostics::No_overload_expects_0_arguments_but_overloads_do_exist_that_expect_either_1_or_2_arguments), &[&args_len, &max_below, &min_above]);
            if let Some(head_message) = head_message {
                diagnostic = ast::new_diagnostic_chain(diagnostic, head_message, &[]);
            }
            diagnostic
        } else if args_len < min_count {
            // too short: put the error span on the call expression, not any of the args
            let mut diagnostic = new_diagnostic_for_node(error_node, Some(message), &[&parameter_range, &args_len]);
            if let Some(head_message) = head_message {
                diagnostic = ast::new_diagnostic_chain(diagnostic, head_message, &[]);
            }
            let mut parameter: Option<P<Node>> = None;
            if let Some(closest_signature) = closest_signature {
                if let Some(declaration) = closest_signature.declaration() {
                    parameter = element_or_nil(declaration.parameters(), args.len() + if closest_signature.this_parameter().is_some() { 1 } else { 0 });
                }
            }
            if let Some(parameter) = parameter {
                let related = if is_binding_pattern(parameter.name().unwrap()) {
                    new_diagnostic_for_node(Some(parameter), Some(&diagnostics::An_argument_matching_this_binding_pattern_was_not_provided), &[])
                } else if is_rest_parameter(parameter) {
                    new_diagnostic_for_node(Some(parameter), Some(&diagnostics::Arguments_for_the_rest_parameter_0_were_not_provided), &[&parameter.name().unwrap().text()])
                } else {
                    new_diagnostic_for_node(Some(parameter), Some(&diagnostics::An_argument_for_0_was_not_provided), &[&parameter.name().unwrap().text()])
                };
                diagnostic.add_related_info(related);
            }
            diagnostic
        } else {
            // Guard against out-of-bounds access when maxCount >= len(args).
            // This can happen when we reach this fallback error path but the argument
            // count actually matches the parameter count (e.g., due to trailing commas
            // causing signature resolution to fail for other reasons).
            if max_count >= args_len {
                let mut diagnostic = new_diagnostic_for_node(error_node, Some(message), &[&parameter_range, &args_len]);
                if let Some(head_message) = head_message {
                    diagnostic = ast::new_diagnostic_chain(diagnostic, head_message, &[]);
                }
                return diagnostic;
            }
            let source_file = get_source_file_of_node(node).unwrap();
            let mut pos = args[max_count as usize].pos();
            let mut end = args[args.len() - 1].end();
            if end == pos {
                end += 1;
            }
            pos = tsrs_scanner::skip_trivia(source_file.text(), pos);
            if end < pos {
                end = pos;
            }
            let mut diagnostic = ast::new_diagnostic(Some(source_file), TextRange::new(pos, end), message, &[&parameter_range, &args_len]);
            if let Some(head_message) = head_message {
                diagnostic = ast::new_diagnostic_chain(diagnostic, head_message, &[]);
            }
            diagnostic
        }
    }

    // checker.go:10015
    pub(crate) fn is_promise_resolve_arity_error(&mut self, node: P<Node>) -> bool {
        if !is_call_expression(node) || !is_identifier(node.expression().unwrap()) {
            return false;
        }
        let expression = node.expression().unwrap();
        let symbol = self.resolve_name(Some(expression), expression.text(), SymbolFlags::Value, None /*nameNotFoundMessage*/, false /*isUse*/, false);
        let Some(symbol) = symbol else {
            return false;
        };
        let decl = symbol.value_declaration();
        let Some(decl) = decl else {
            return false;
        };
        if !is_parameter_declaration(decl) || !is_function_expression_or_arrow_function(decl.parent().unwrap()) || !is_new_expression(decl.parent().unwrap().parent().unwrap()) || !is_identifier(decl.parent().unwrap().parent().unwrap().expression().unwrap()) {
            return false;
        }
        let global_promise_symbol = self.get_global_promise_constructor_symbol_or_nil();
        let Some(global_promise_symbol) = global_promise_symbol else {
            return false;
        };
        let constructor_symbol = self.get_resolved_symbol(decl.parent().unwrap().parent().unwrap().expression().unwrap());
        constructor_symbol == global_promise_symbol
    }
}

// checker.go:10035
pub(crate) fn get_error_node_for_call_node(node: P<Node>) -> Option<P<Node>> {
    let mut node = node;
    if is_call_expression(node) {
        node = node.expression().unwrap();
        if is_property_access_expression(node) {
            node = node.name().unwrap();
        }
    }
    Some(node)
}

impl Checker {
    // checker.go:10045
    pub(crate) fn get_type_argument_arity_error(&mut self, node: P<Node>, signatures: &[P<Signature>], type_arguments: &[P<Node>], head_message: Option<&'static Message>) -> P<Diagnostic> {
        let mut diagnostic: P<Diagnostic>;
        let arg_count = type_arguments.len() as i64;
        let source_file = get_source_file_of_node(node).unwrap();
        let type_argument_list = node.type_argument_list().unwrap();
        let loc = TextRange::new(tsrs_scanner::skip_trivia(source_file.text(), type_argument_list.loc.get().pos()), type_argument_list.loc.get().end());
        if signatures.len() == 1 {
            // No overloads exist
            let sig = signatures[0];
            let min_count = self.get_min_type_argument_count(sig.type_parameters());
            let max_count = sig.type_parameters().len() as i32;
            let mut expected = min_count.to_string();
            if min_count < max_count {
                expected = expected + "-" + &max_count.to_string();
            }
            diagnostic = ast::new_diagnostic(Some(source_file), loc, &diagnostics::Expected_0_type_arguments_but_got_1, &[&expected, &arg_count]);
        } else {
            // Overloads exist
            let mut below_arg_count = i64::MIN;
            let mut above_arg_count = i64::MAX;
            for &sig in signatures {
                let min_count = self.get_min_type_argument_count(sig.type_parameters()) as i64;
                let max_count = sig.type_parameters().len() as i64;
                if min_count > arg_count {
                    above_arg_count = above_arg_count.min(min_count);
                } else if max_count < arg_count {
                    below_arg_count = below_arg_count.max(max_count);
                }
            }
            if below_arg_count != i64::MIN && above_arg_count != i64::MAX {
                diagnostic = ast::new_diagnostic(Some(source_file), loc, &diagnostics::No_overload_expects_0_type_arguments_but_overloads_do_exist_that_expect_either_1_or_2_type_arguments, &[&arg_count, &below_arg_count, &above_arg_count]);
            } else {
                let expected = if below_arg_count == i64::MIN { above_arg_count } else { below_arg_count };
                diagnostic = ast::new_diagnostic(Some(source_file), loc, &diagnostics::Expected_0_type_arguments_but_got_1, &[&expected, &arg_count]);
            }
        }
        if let Some(head_message) = head_message {
            diagnostic = ast::new_diagnostic_chain(diagnostic, head_message, &[]);
        }
        diagnostic
    }

    // checker.go:10086
    pub(crate) fn report_cannot_invoke_possibly_null_or_undefined_error(&mut self, node: P<Node>, facts: TypeFacts) {
        let message: &'static Message = if facts.intersects(TypeFacts::IsUndefined) {
            if facts.intersects(TypeFacts::IsNull) {
                &diagnostics::Cannot_invoke_an_object_which_is_possibly_null_or_undefined
            } else {
                &diagnostics::Cannot_invoke_an_object_which_is_possibly_undefined
            }
        } else {
            &diagnostics::Cannot_invoke_an_object_which_is_possibly_null
        };
        self.error(Some(node), message, &[]);
    }

    // checker.go:10094
    pub(crate) fn resolve_untyped_call(&mut self, node: P<Node>) -> P<Signature> {
        if self.call_like_expression_may_have_type_arguments(node) {
            // Check type arguments even though we will give an error that untyped calls may not accept type arguments.
            // This gets us diagnostics for the type arguments and marks them as referenced.
            self.check_source_elements(node.type_arguments());
        }
        match node.kind() {
            Kind::TaggedTemplateExpression => {
                self.check_expression(node.as_tagged_template_expression().template);
            }
            Kind::JsxOpeningElement | Kind::JsxSelfClosingElement => {
                self.check_expression(node.attributes().unwrap());
            }
            Kind::BinaryExpression => {
                self.check_expression(node.as_binary_expression().left());
            }
            Kind::CallExpression | Kind::NewExpression => {
                for &argument in node.arguments() {
                    self.check_expression(argument);
                }
            }
            _ => {}
        }
        self.any_signature
    }

    // checker.go:10115
    pub(crate) fn resolve_error_call(&mut self, node: P<Node>) -> P<Signature> {
        self.resolve_untyped_call(node);
        self.unknown_signature
    }

    /**
     * TS 1.0 spec: 4.12
     * If FuncExpr is of type Any, or of an object type that has no call or construct signatures
     * but is a subtype of the Function interface, the call is an untyped function call.
     */
    // checker.go:10125
    pub(crate) fn is_untyped_function_call(&mut self, func_type: P<Type>, apparent_func_type: P<Type>, num_call_signatures: i32, num_construct_signatures: i32) -> bool {
        // We exclude union types because we may have a union of function types that happen to have no common signatures.
        is_type_any(Some(func_type))
            || is_type_any(Some(apparent_func_type)) && func_type.flags().intersects(TypeFlags::TypeParameter)
            || num_call_signatures == 0
                && num_construct_signatures == 0
                && !apparent_func_type.flags().intersects(TypeFlags::Union)
                && !self.get_reduced_type(apparent_func_type).flags().intersects(TypeFlags::Never)
                && {
                    let global_function_type = self.global_function_type;
                    self.is_type_assignable_to(func_type, global_function_type)
                }
    }

    // checker.go:10132
    pub(crate) fn invocation_error_details(&mut self, error_target: P<Node>, apparent_type: P<Type>, kind: SignatureKind) -> P<Diagnostic> {
        let mut diagnostic: Option<P<Diagnostic>> = None;
        let is_call = kind == SignatureKind::Call;
        let awaited_type = self.get_awaited_type(apparent_type);
        let maybe_missing_await = match awaited_type {
            Some(awaited_type) => !self.get_signatures_of_type(awaited_type, kind).is_empty(),
            None => false,
        };
        let mut target = error_target;
        if is_property_access_expression(error_target) && is_call_expression(error_target.parent().unwrap()) {
            target = error_target.name().unwrap();
        }
        if apparent_type.flags().intersects(TypeFlags::Union) {
            let types = apparent_type.types();
            let mut has_signatures = false;
            for &constituent in types {
                let signatures = self.get_signatures_of_type(constituent, kind);
                if !signatures.is_empty() {
                    has_signatures = true;
                    if diagnostic.is_some() {
                        // Bail early if we already have an error, no chance of "No constituent of type is callable"
                        break;
                    }
                } else {
                    // Error on the first non callable constituent only
                    if diagnostic.is_none() {
                        let s = self.type_to_string_exported(constituent);
                        let d = new_diagnostic_for_node(Some(target), Some(if is_call { &diagnostics::Type_0_has_no_call_signatures } else { &diagnostics::Type_0_has_no_construct_signatures }), &[&s]);
                        let s = self.type_to_string_exported(apparent_type);
                        diagnostic = Some(new_diagnostic_chain_for_node(Some(d), target, Some(if is_call { &diagnostics::Not_all_constituents_of_type_0_are_callable } else { &diagnostics::Not_all_constituents_of_type_0_are_constructable }), &[&s]));
                    }
                    if has_signatures {
                        // Bail early if we already found a signature, no chance of "No constituent of type is callable"
                        break;
                    }
                }
            }
            if !has_signatures {
                let s = self.type_to_string_exported(apparent_type);
                diagnostic = Some(new_diagnostic_for_node(Some(target), Some(if is_call { &diagnostics::No_constituent_of_type_0_is_callable } else { &diagnostics::No_constituent_of_type_0_is_constructable }), &[&s]));
            }
            if diagnostic.is_none() {
                let s = self.type_to_string_exported(apparent_type);
                diagnostic = Some(new_diagnostic_for_node(
                    Some(target),
                    Some(if is_call {
                        &diagnostics::Each_member_of_the_union_type_0_has_signatures_but_none_of_those_signatures_are_compatible_with_each_other
                    } else {
                        &diagnostics::Each_member_of_the_union_type_0_has_construct_signatures_but_none_of_those_signatures_are_compatible_with_each_other
                    }),
                    &[&s],
                ));
            }
        } else {
            let s = self.type_to_string_exported(apparent_type);
            diagnostic = Some(new_diagnostic_chain_for_node(diagnostic, target, Some(if is_call { &diagnostics::Type_0_has_no_call_signatures } else { &diagnostics::Type_0_has_no_construct_signatures }), &[&s]));
        }
        let mut head_message: &'static Message = if is_call { &diagnostics::This_expression_is_not_callable } else { &diagnostics::This_expression_is_not_constructable };
        // Diagnose get accessors incorrectly called as functions
        let error_target_parent = error_target.parent().unwrap();
        if is_call_expression(error_target_parent) && error_target_parent.arguments().is_empty() {
            let resolved_symbol = self.get_resolved_symbol_or_nil(error_target);
            if let Some(resolved_symbol) = resolved_symbol {
                if resolved_symbol.flags().intersects(SymbolFlags::GetAccessor) {
                    head_message = &diagnostics::This_expression_is_not_callable_because_it_is_a_get_accessor_Did_you_mean_to_use_it_without;
                }
            }
        }
        let diagnostic = new_diagnostic_chain_for_node(diagnostic, target, Some(head_message), &[]);
        if maybe_missing_await {
            diagnostic.add_related_info(new_diagnostic_for_node(Some(error_target), Some(&diagnostics::Did_you_forget_to_use_await), &[]));
        }
        diagnostic
    }

    // checker.go:10188
    pub(crate) fn invocation_error(&mut self, error_target: P<Node>, apparent_type: P<Type>, kind: SignatureKind, related_information: Option<P<Diagnostic>>) {
        let mut diagnostic = self.invocation_error_details(error_target, apparent_type, kind);
        if let Some(related_information) = related_information {
            diagnostic.add_related_info(related_information);
        }
        diagnostic = self.add_diagnostic(diagnostic);
        self.invocation_error_recovery(apparent_type, kind, diagnostic);
    }

    // checker.go:10197
    pub(crate) fn invocation_error_recovery(&mut self, apparent_type: P<Type>, kind: SignatureKind, diagnostic: P<Diagnostic>) {
        let Some(symbol) = apparent_type.symbol() else {
            return;
        };
        let import_node = self.export_type_links.get(symbol).originating_import.get();
        // Create a diagnostic on the originating import if possible onto which we can attach a quickfix
        //  An import call expression cannot be rewritten into another form to correct the error - the only solution is to use `.default` at the use-site
        if let Some(import_node) = import_node {
            if !is_import_call(import_node) {
                let target = self.export_type_links.get(symbol).target.get().unwrap();
                let target_type = self.get_type_of_symbol(target);
                let sigs = self.get_signatures_of_type(target_type, kind);
                if sigs.is_empty() {
                    return;
                }
                diagnostic.add_related_info(new_diagnostic_for_node(Some(import_node), Some(&diagnostics::Type_originates_at_this_import_A_namespace_style_import_cannot_be_called_or_constructed_and_will_cause_a_failure_at_runtime_Consider_using_a_default_import_or_import_require_here_instead), &[]));
            }
        }
    }

    // checker.go:10213
    pub(crate) fn is_generic_function_returning_function(&mut self, signature: P<Signature>) -> bool {
        if signature.type_parameters().is_empty() {
            return false;
        }
        let return_type = self.get_return_type_of_signature(signature);
        self.is_function_type(return_type)
    }

    // checker.go:10217
    pub(crate) fn skipped_generic_function(&mut self, node: P<Node>, check_mode: CheckMode) {
        if check_mode.intersects(CheckMode::Inferential) {
            // We have skipped a generic function during inferential typing. Obtain the inference context and
            // indicate this has occurred such that we know a second pass of inference is be needed.
            let context = self.get_inference_context(node).unwrap();
            context.flags.set(context.flags.get() | InferenceFlags::SkippedGenericFunction);
        }
    }

    // checker.go:10226
    pub(crate) fn check_tagged_template_expression(&mut self, node: P<Node>) -> P<Type> {
        if !self.check_grammar_tagged_template_chain(node) {
            self.check_grammar_type_arguments(node, node.type_argument_list());
        }
        let signature = self.get_resolved_signature(node, None, CheckMode::Normal);
        self.check_deprecated_signature(signature, node);
        self.get_return_type_of_signature(signature)
    }

    // checker.go:10235
    pub(crate) fn check_parenthesized_expression(&mut self, node: P<Node>, check_mode: CheckMode) -> P<Type> {
        self.check_expression_ex(node.expression().unwrap(), check_mode)
    }

    // checker.go:10239
    pub(crate) fn check_class_expression(&mut self, node: P<Node>) -> P<Type> {
        self.check_class_like_declaration(node);
        self.check_node_deferred(node);
        self.check_class_expression_external_helpers(node);
        let symbol = self.get_symbol_of_declaration(node).unwrap();
        self.get_type_of_symbol(symbol)
    }

    // checker.go:10246
    pub(crate) fn get_first_transformable_static_class_element(&mut self, node: P<Node>) -> Option<P<Node>> {
        let will_transform_static_elements_of_decorated_class = !self.legacy_decorators
            && self.language_version < LanguageFeatureMinimumTarget.class_and_class_element_decorators
            && class_or_constructor_parameter_is_decorated(false, node);
        let will_transform_private_elements_or_class_static_blocks = self.language_version < LanguageFeatureMinimumTarget.private_names_and_class_static_blocks || self.language_version < LanguageFeatureMinimumTarget.class_and_class_element_decorators;
        let will_transform_initializers = !self.emit_standard_class_fields;
        if will_transform_static_elements_of_decorated_class || will_transform_private_elements_or_class_static_blocks {
            for &member in node.members() {
                if will_transform_static_elements_of_decorated_class && class_element_or_class_element_parameter_is_decorated(false, member, node) {
                    if let Some(first_decorator) = node.decorators().first().copied() {
                        return Some(first_decorator);
                    }
                    return Some(node);
                } else if will_transform_private_elements_or_class_static_blocks {
                    if is_class_static_block_declaration(member) {
                        return Some(member);
                    } else if ast::is_static(member) && (is_private_identifier_class_element_declaration(member) || will_transform_initializers && is_initialized_property(member)) {
                        return Some(member);
                    }
                }
            }
        }
        None
    }

    // checker.go:10273
    pub(crate) fn check_class_expression_external_helpers(&mut self, node: P<Node>) {
        if node.name().is_some() {
            return;
        }
        let parent = walk_up_outer_expressions(node).unwrap();
        if !is_named_evaluation_source(parent) {
            return;
        }

        let will_transform_es_decorators = !self.legacy_decorators && self.language_version < LanguageFeatureMinimumTarget.class_and_class_element_decorators;
        let location: Option<P<Node>>;
        if will_transform_es_decorators && class_or_constructor_parameter_is_decorated(false, node) {
            let mut loc = node;
            if let Some(first_decorator) = node.decorators().first().copied() {
                loc = first_decorator;
            }
            location = Some(loc);
        } else {
            location = self.get_first_transformable_static_class_element(node);
        }

        if let Some(location) = location {
            self.check_external_emit_helpers(location, ExternalEmitHelpers::SetFunctionName);
            if (is_property_assignment(parent) || is_property_declaration(parent) || is_binding_element(parent)) && is_computed_property_name(parent.name().unwrap()) {
                self.check_external_emit_helpers(location, ExternalEmitHelpers::PropKey);
            }
        }
    }

    // checker.go:10301
    pub(crate) fn check_class_expression_deferred(&mut self, node: P<Node>) {
        self.check_source_elements(node.members());
        self.register_for_unused_identifiers_check(node);
    }

    // checker.go:10306
    pub(crate) fn check_function_expression_or_object_literal_method(&mut self, node: P<Node>, check_mode: CheckMode) -> P<Type> {
        self.check_node_deferred(node);
        if let Some(full_signature) = node.function_like_data().unwrap().full_signature() {
            self.check_source_element(Some(full_signature));
        }
        if is_function_expression(node) {
            self.check_collisions_for_declaration_name(node, node.name());
        }
        if check_mode.intersects(CheckMode::SkipContextSensitive) && self.is_context_sensitive(node) {
            // Skip parameters, return signature with return type that retains noncontextual parts so inferences can still be drawn in an early stage
            if node.type_node().is_none() && !has_context_sensitive_parameters(node) {
                // Return plain anyFunctionType if there is no possibility we'll make inferences from the return type
                let contextual_signature = self.get_contextual_signature(node);
                if let Some(contextual_signature) = contextual_signature {
                    let contextual_return_type = self.get_return_type_of_signature(contextual_signature);
                    if self.could_contain_type_variables(contextual_return_type) {
                        if let Some(&cached) = self.context_free_types.get(&node) {
                            return cached;
                        }
                        let return_type = self.get_return_type_from_body(node, check_mode);
                        let return_only_signature = self.new_signature(SignatureFlags::IsNonInferrable, None, &[] /*typeParameters*/, None /*thisParameter*/, &[], Some(return_type), None /*resolvedTypePredicate*/, 0);
                        let return_only_type = self.new_anonymous_type(node.symbol(), None, &[return_only_signature], &[], &[]);
                        return_only_type.object_flags.set(return_only_type.object_flags.get_lazy() | ObjectFlags::NonInferrableType);
                        self.context_free_types.insert(node, return_only_type);
                        return return_only_type;
                    }
                }
            }
            return self.any_function_type;
        }
        // Grammar checking
        let has_grammar_error = self.check_grammar_function_like_declaration(node);
        if !has_grammar_error && is_function_expression(node) {
            self.check_grammar_for_generator(node);
        }
        if let Some(full_signature) = node.function_like_data().unwrap().full_signature() {
            let t = self.get_type_from_type_node(full_signature);
            if self.get_contextual_call_signature(t, node).is_none() {
                self.error(Some(full_signature), &diagnostics::A_JSDoc_type_tag_on_a_function_must_have_a_signature_with_the_correct_number_of_arguments, &[]);
            }
        }
        self.contextually_check_function_expression_or_object_literal_method(node, check_mode);
        let symbol = self.get_symbol_of_declaration(node).unwrap();
        self.get_type_of_symbol(symbol)
    }

    // checker.go:10347
    pub(crate) fn contextually_check_function_expression_or_object_literal_method(&mut self, node: P<Node>, check_mode: CheckMode) {
        let links = self.node_links.get(node);
        // Check if function expression is contextually typed and assign parameter types if so.
        if !links.flags.get().intersects(NodeCheckFlags::ContextChecked) {
            let contextual_signature = self.get_contextual_signature(node);
            // If a type check is started at a function expression that is an argument of a function call, obtaining the
            // contextual type may recursively get back to here during overload resolution of the call. If so, we will have
            // already assigned contextual types.
            if !links.flags.get().intersects(NodeCheckFlags::ContextChecked) {
                links.flags.set(links.flags.get() | NodeCheckFlags::ContextChecked);
                let symbol = self.get_symbol_of_declaration(node).unwrap();
                let symbol_type = self.get_type_of_symbol(symbol);
                let signature = self.get_signatures_of_type(symbol_type, SignatureKind::Call).first().copied();
                let Some(signature) = signature else {
                    return;
                };
                if self.is_context_sensitive(node) {
                    if let Some(contextual_signature) = contextual_signature {
                        let inference_context = self.get_inference_context(node);
                        let mut instantiated_contextual_signature: Option<P<Signature>> = None;
                        if check_mode.intersects(CheckMode::Inferential) {
                            self.infer_from_annotated_parameters_and_return(signature, contextual_signature, inference_context);
                            let rest_type = self.get_effective_rest_type(contextual_signature);
                            if let Some(rest_type) = rest_type {
                                if rest_type.flags().intersects(TypeFlags::TypeParameter) {
                                    let non_fixing_mapper = inference_context.unwrap().non_fixing_mapper();
                                    instantiated_contextual_signature = Some(self.instantiate_signature(contextual_signature, non_fixing_mapper));
                                }
                            }
                        }
                        if instantiated_contextual_signature.is_none() {
                            if let Some(inference_context) = inference_context {
                                let mapper = inference_context.mapper();
                                instantiated_contextual_signature = Some(self.instantiate_signature(contextual_signature, mapper));
                            } else {
                                instantiated_contextual_signature = Some(contextual_signature);
                            }
                        }
                        self.assign_contextual_parameter_types(signature, instantiated_contextual_signature.unwrap());
                    } else {
                        // Force resolution of all parameter types such that the absence of a contextual type is consistently reflected.
                        self.assign_non_contextual_parameter_types(signature);
                    }
                } else if let Some(contextual_signature) = contextual_signature.filter(|cs| node.type_parameter_list().is_none() && cs.parameters().len() > node.parameters().len()) {
                    let inference_context = self.get_inference_context(node);
                    if check_mode.intersects(CheckMode::Inferential) {
                        self.infer_from_annotated_parameters_and_return(signature, contextual_signature, inference_context);
                    }
                }
                if contextual_signature.is_some() && self.get_return_type_from_annotation(node).is_none() && signature.resolved_return_type.get().is_none() {
                    // resolvedReturnType is cached indefinitely, so the return type here has to be computed without CheckModeSkipContextSensitive;
                    // otherwise anyFunctionType could leak as part of the computed (and cached) return type.
                    let return_type = self.get_return_type_from_body(node, check_mode & !CheckMode::SkipContextSensitive);
                    if signature.resolved_return_type.get().is_none() {
                        signature.resolved_return_type.set(Some(return_type));
                    }
                }
                self.check_signature_declaration(node);
            }
        }
    }

    // checker.go:10403
    pub(crate) fn check_function_expression_or_object_literal_method_deferred(&mut self, node: P<Node>) {
        let function_flags = get_function_flags(Some(node));
        let return_type = self.get_return_type_from_annotation(node);
        self.check_all_code_paths_in_non_void_function_return_or_throw(node, return_type);
        let body = node.body();
        if let Some(body) = body {
            if node.type_node().is_none() {
                // There are some checks that are only performed in getReturnTypeFromBody, that may produce errors
                // we need. An example is the noImplicitAny errors resulting from widening the return expression
                // of a function. Because checking of function expression bodies is deferred, there was never an
                // appropriate time to do this during the main walk of the file (see the comment at the top of
                // checkFunctionExpressionBodies). So it must be done now.
                let signature = self.get_signature_from_declaration(node);
                self.get_return_type_of_signature(signature);
            }
            if is_block(body) {
                self.check_source_element(Some(body));
            } else {
                // From within an async function you can return either a non-promise value or a promise. Any
                // Promise/A+ compatible implementation will always assimilate any foreign promise, so we
                // should not be checking assignability of a promise to the return type. Instead, we need to
                // check assignability of the awaited type of the expression body against the promised type of
                // its return type annotation.
                let expr_type = self.check_expression(body);
                if let Some(return_type) = return_type {
                    let return_or_promised_type = self.unwrap_return_type(return_type, function_flags);
                    if let Some(return_or_promised_type) = return_or_promised_type {
                        self.check_return_expression(node, return_or_promised_type, body, Some(body), expr_type, false);
                    }
                }
            }
        }
    }

    // checker.go:10436
    pub(crate) fn infer_from_annotated_parameters_and_return(&mut self, sig: P<Signature>, context: P<Signature>, inference_context: Option<P<InferenceContext>>) {
        let length = sig.parameters().len() - if signature_has_rest_parameter(sig) { 1 } else { 0 };
        for i in 0..length {
            let declaration = sig.parameters()[i].value_declaration().unwrap();
            let type_node = declaration.type_node();
            if let Some(type_node) = type_node {
                let t = self.get_type_from_type_node(type_node);
                let source = self.add_optionality_ex(t, false /*isProperty*/, is_optional_declaration(declaration));
                let target = self.get_type_at_position(context, i as i32);
                self.infer_types(inference_context.unwrap().inferences.get(), source, target, InferencePriority::None, false);
            }
        }
        if let Some(declaration) = sig.declaration() {
            if let Some(return_type_node) = declaration.type_node() {
                let source = self.get_type_from_type_node(return_type_node);
                let target = self.get_return_type_of_signature(context);
                self.infer_types(inference_context.unwrap().inferences.get(), source, target, InferencePriority::None, false);
            }
        }
    }

    // Return the contextual signature for a given expression node. A contextual type provides a
    // contextual signature if it has a single call signature and if that call signature is non-generic.
    // If the contextual type is a union type, get the signature from each type possible and if they are
    // all identical ignoring their return type, the result is same signature but with return type as
    // union type of return types from these signatures
    // checker.go:10461
    pub(crate) fn get_contextual_signature(&mut self, node: P<Node>) -> Option<P<Signature>> {
        let t = self.get_apparent_type_of_contextual_type(node, ContextFlags::Signature)?;
        if !t.flags().intersects(TypeFlags::Union) {
            return self.get_contextual_call_signature(t, node);
        }
        let mut signature_list: Vec<P<Signature>> = Vec::new();
        let types = t.types();
        for &current in types {
            let signature = self.get_contextual_call_signature(current, node);
            if let Some(signature) = signature {
                if !signature_list.is_empty() && self.compare_signatures_identical(signature_list[0], signature, false /*partialMatch*/, true /*ignoreThisTypes*/, true /*ignoreReturnTypes*/, |c, s, t| c.compare_types_identical(s, t)) == Ternary::False {
                    // Signatures aren't identical, do not use
                    return None;
                }
                // Use this signature for contextual union signature
                signature_list.push(signature);
            }
        }
        match signature_list.len() {
            0 => return None,
            1 => return Some(signature_list[0]),
            _ => {}
        }
        // Result is union of signatures collected (return type is union of return types of this signature set)
        Some(self.create_union_signature(signature_list[0], &signature_list))
    }

    // checker.go:10492
    pub(crate) fn create_union_signature(&mut self, sig: P<Signature>, union_signatures: &[P<Signature>]) -> P<Signature> {
        let result = self.clone_signature(sig);
        result.set_composite(Some(P::new(CompositeSignature { is_union: Cell::new(true), signatures: Cell::new(alloc_slice(union_signatures)) })));
        result.target.set(None);
        result.mapper.set(None);
        result
    }

    // If the given type is an object or union type with a single signature, and if that signature has at
    // least as many parameters as the given function, return the signature. Otherwise return undefined.
    // checker.go:10502
    pub(crate) fn get_contextual_call_signature(&mut self, t: P<Type>, node: P<Node>) -> Option<P<Signature>> {
        let signatures = self.get_signatures_of_type(t, SignatureKind::Call);
        let mut applicable_by_arity = Vec::new();
        for &s in signatures {
            if !self.is_arity_smaller(s, node) {
                applicable_by_arity.push(s);
            }
        }
        if applicable_by_arity.len() == 1 {
            return Some(applicable_by_arity[0]);
        }
        self.get_intersected_signatures(&applicable_by_arity)
    }

    // checker.go:10511
    pub(crate) fn get_intersected_signatures(&mut self, signatures: &[P<Signature>]) -> Option<P<Signature>> {
        if !self.no_implicit_any {
            return None;
        }
        let mut combined: Option<P<Signature>> = None;
        for &sig in signatures {
            match combined {
                None => combined = Some(sig),
                Some(c) if c == sig => combined = Some(sig),
                Some(c) if self.compare_type_parameters_identical(c.type_parameters(), sig.type_parameters()) => {
                    combined = Some(self.combine_union_or_intersection_member_signatures(c, sig, false /*isUnion*/));
                }
                _ => return None,
            }
        }
        combined
    }

    /** If the contextual signature has fewer parameters than the function expression, do not use it */
    // checker.go:10530
    pub(crate) fn is_arity_smaller(&mut self, signature: P<Signature>, target: P<Node>) -> bool {
        let parameters = target.parameters();
        let mut target_parameter_count: i32 = 0;
        while (target_parameter_count as usize) < parameters.len() {
            let param = parameters[target_parameter_count as usize];
            if param.initializer().is_some() || param.question_token().is_some() || has_dot_dot_dot_token(param) {
                break;
            }
            target_parameter_count += 1;
        }
        if !parameters.is_empty() && is_this_parameter(parameters[0]) {
            target_parameter_count -= 1;
        }
        !self.has_effective_rest_parameter(signature) && self.get_parameter_count(signature) < target_parameter_count
    }

    // checker.go:10546
    pub(crate) fn assign_contextual_parameter_types(&mut self, sig: P<Signature>, context: P<Signature>) {
        if !context.type_parameters().is_empty() {
            if !sig.type_parameters().is_empty() {
                // This signature has already has a contextual inference performed and cached on it
                return;
            }
            sig.type_parameters.set(context.type_parameters());
        }
        if let Some(context_this_parameter) = context.this_parameter() {
            let parameter = sig.this_parameter();
            if parameter.is_none() || parameter.unwrap().value_declaration().is_some_and(|vd| vd.type_node().is_none()) {
                if parameter.is_none() {
                    sig.set_this_parameter(Some(self.create_symbol_with_type(context_this_parameter, None /*type*/)));
                }
                let t = self.get_type_of_symbol(context_this_parameter);
                self.assign_parameter_type(sig.this_parameter().unwrap(), Some(t));
            }
        }
        let length = sig.parameters().len() - if signature_has_rest_parameter(sig) { 1 } else { 0 };
        for i in 0..length {
            let parameter = sig.parameters()[i];
            let declaration = parameter.value_declaration().unwrap();
            if declaration.type_node().is_none() {
                let mut t = self.try_get_type_at_position(context, i as i32);
                if let Some(tt) = t {
                    if declaration.initializer().is_some() {
                        let mut initializer_type = self.check_declaration_initializer(declaration, CheckMode::Normal, None);
                        if !self.is_type_assignable_to(initializer_type, tt) {
                            initializer_type = self.widen_type_inferred_from_initializer(declaration, initializer_type);
                            if self.is_type_assignable_to(tt, initializer_type) {
                                t = Some(initializer_type);
                            }
                        }
                    }
                }
                self.assign_parameter_type(parameter, t);
            }
        }
        if signature_has_rest_parameter(sig) {
            // parameter might be a transient symbol generated by use of `arguments` in the function body.
            let parameter = sig.parameters().last().copied().unwrap();
            let value_declaration = parameter.value_declaration();
            if value_declaration.is_some_and(|vd| vd.type_node().is_none()) || value_declaration.is_none() && parameter.check_flags().intersects(CheckFlags::DeferredType) {
                let contextual_parameter_type = self.get_rest_type_at_position(context, length as i32, false);
                self.assign_parameter_type(parameter, Some(contextual_parameter_type));
            }
        }
    }

    // checker.go:10592
    pub(crate) fn assign_non_contextual_parameter_types(&mut self, signature: P<Signature>) {
        if let Some(this_parameter) = signature.this_parameter() {
            self.assign_parameter_type(this_parameter, None);
        }
        for &parameter in signature.parameters() {
            self.assign_parameter_type(parameter, None);
        }
    }

    // checker.go:10601
    pub(crate) fn assign_parameter_type(&mut self, parameter: P<Symbol>, contextual_type: Option<P<Type>>) {
        let links = self.value_symbol_links.get(parameter);
        if links.resolved_type.get().is_some() {
            return;
        }
        let declaration = parameter.value_declaration();
        let t = match contextual_type {
            Some(t) => t,
            None => match declaration {
                Some(declaration) => self.get_widened_type_for_variable_like_declaration(declaration, true /*reportErrors*/),
                None => self.get_type_of_symbol(parameter),
            },
        };
        let is_optional = declaration.is_some_and(|declaration| declaration.initializer().is_none() && is_optional_declaration(declaration));
        links.resolved_type.set(Some(self.add_optionality_ex(t, false, is_optional)));
        if let Some(declaration) = declaration {
            let name = declaration.name().unwrap();
            if !is_identifier(name) {
                // if inference didn't come up with anything but unknown, fall back to the binding pattern if present.
                if links.resolved_type.get() == Some(self.unknown_type) {
                    links.resolved_type.set(Some(self.get_type_from_binding_pattern(name, false, false)));
                }
                self.assign_binding_element_types(name, links.resolved_type.get().unwrap());
            }
        }
    }

    // When contextual typing assigns a type to a parameter that contains a binding pattern, we also need to push
    // the destructured type into the contained binding elements.
    // checker.go:10627
    pub(crate) fn assign_binding_element_types(&mut self, pattern: P<Node>, parent_type: P<Type>) {
        for &element in pattern.elements() {
            let name = element.name();
            if let Some(name) = name {
                let t = self.get_binding_element_type_from_parent_type(element, parent_type, false /*noTupleBoundsCheck*/);
                if is_identifier(name) {
                    let symbol = self.get_symbol_of_declaration(element).unwrap();
                    self.value_symbol_links.get(symbol).resolved_type.set(Some(t));
                } else {
                    self.assign_binding_element_types(name, t);
                }
            }
        }
    }

    // checker.go:10641
    pub(crate) fn check_collisions_for_declaration_name(&mut self, node: P<Node>, name: Option<P<Node>>) {
        let Some(name) = name else {
            return;
        };
        self.check_collision_with_require_exports_in_generated_code(node, Some(name));
        self.check_collision_with_global_object_in_generated_code(node, Some(name));
        self.check_collision_with_global_promise_in_generated_code(node, Some(name));
        self.record_potential_collision_with_weak_map_set_in_generated_code(node, name);
        self.record_potential_collision_with_reflect_in_generated_code(node, Some(name));
        if is_class_like(node) {
            self.check_type_name_is_reserved(name, &diagnostics::Class_name_cannot_be_0);
            if !node.flags().intersects(NodeFlags::Ambient) {
                self.check_class_name_collision_with_object(name);
            }
        } else if is_enum_declaration(node) {
            self.check_type_name_is_reserved(name, &diagnostics::Enum_name_cannot_be_0);
        }
    }

    // checker.go:10660
    pub(crate) fn check_collision_with_require_exports_in_generated_code(&mut self, node: P<Node>, name: Option<P<Node>>) {
        // No need to check for require or exports for ES6 modules and later
        if self.program.get_emit_module_format_of_file(get_source_file_of_node(node).unwrap()) >= ModuleKind::ES2015 {
            return;
        }
        let Some(name_node) = name else {
            return;
        };
        if !self.need_collision_check_for_identifier(node, name, "require") && !self.need_collision_check_for_identifier(node, name, "exports") {
            return;
        }
        // Uninstantiated modules shouldnt do this check
        if is_module_declaration(node) && get_module_instance_state(node) != ModuleInstanceState::Instantiated {
            return;
        }
        // In case of variable declaration, node.parent is variable statement so look at the variable statement's parent
        let parent = get_declaration_container(node).unwrap();
        if is_source_file(parent) && is_external_or_common_js_module(parent.as_source_file_p()) {
            // If the declaration happens to be in external module, report error that require and exports are reserved keywords
            let s = tsrs_scanner::declaration_name_to_string(Some(name_node));
            self.error_skipped_on_no_emit(name_node, &diagnostics::Duplicate_identifier_0_Compiler_reserves_name_1_in_top_level_scope_of_a_module, &[&s, &s]);
        }
    }

    // checker.go:10680
    pub(crate) fn check_collision_with_global_object_in_generated_code(&mut self, node: P<Node>, name: Option<P<Node>>) {
        let Some(name_node) = name else {
            return;
        };
        if is_class_like(node) || !self.need_collision_check_for_identifier(node, name, "Object") {
            return;
        }
        // Uninstantiated modules shouldn't do this check
        if is_module_declaration(node) && get_module_instance_state(node) != ModuleInstanceState::Instantiated {
            return;
        }
        // In case of variable declaration, node.parent is variable statement so look at the variable statement's parent
        let parent = get_declaration_container(node).unwrap();
        if is_source_file(parent) && is_external_or_common_js_module(parent.as_source_file_p()) && self.program.get_emit_module_format_of_file(parent.as_source_file_p()) == ModuleKind::CommonJS {
            // If the declaration happens to be in external module, report error that Object is a reserved identifier.
            let s = tsrs_scanner::declaration_name_to_string(Some(name_node));
            self.error_skipped_on_no_emit(name_node, &diagnostics::Duplicate_identifier_0_Compiler_reserves_name_1_in_top_level_scope_of_a_module, &[&s, &s]);
        }
    }

    // checker.go:10696
    pub(crate) fn need_collision_check_for_identifier(&mut self, node: P<Node>, identifier: Option<P<Node>>, name: &str) -> bool {
        if let Some(identifier) = identifier {
            if identifier.text() != name {
                return false;
            }
        }
        match node.kind() {
            Kind::PropertyDeclaration | Kind::PropertySignature | Kind::MethodDeclaration | Kind::MethodSignature | Kind::GetAccessor | Kind::SetAccessor | Kind::PropertyAssignment => {
                // it is ok to have member named '_super', '_this', `Promise`, etc. - member access is always qualified
                return false;
            }
            _ => {}
        }
        if node.flags().intersects(NodeFlags::Ambient) {
            // ambient context - no codegen impact
            return false;
        }
        if is_import_clause(node) || is_import_equals_declaration(node) || is_import_specifier(node) {
            // type-only imports do not require collision checks against runtime values.
            if is_type_only_import_or_export_declaration(node) {
                return false;
            }
        }
        let root = get_root_declaration(node);
        if is_parameter_declaration(root) && node_is_missing(root.parent().unwrap().body()) {
            // just an overload - no codegen impact
            return false;
        }
        true
    }

    // checker.go:10724
    pub(crate) fn set_node_links_for_private_identifier_scope(&mut self, node: P<Node>) {
        let name = node.name().unwrap();
        if is_private_identifier(name) {
            if self.language_version < LanguageFeatureMinimumTarget.private_names_and_class_static_blocks
                || self.language_version < LanguageFeatureMinimumTarget.class_and_class_element_decorators
                || !self.compiler_options.get_use_define_for_class_fields()
            {
                let mut lexical_scope = get_enclosing_block_scope_container(node);
                while let Some(scope) = lexical_scope {
                    let links = self.node_links.get(scope);
                    links.flags.set(links.flags.get() | NodeCheckFlags::ContainsClassWithPrivateIdentifiers);
                    lexical_scope = get_enclosing_block_scope_container(scope);
                }
            }
        }
    }

    // checker.go:10736
    pub(crate) fn record_potential_collision_with_weak_map_set_in_generated_code(&mut self, node: P<Node>, name: P<Node>) {
        if self.language_version <= ScriptTarget::ES2021 && (self.need_collision_check_for_identifier(node, Some(name), "WeakMap") || self.need_collision_check_for_identifier(node, Some(name), "WeakSet")) {
            self.add_deferred_diagnostic(move |c| {
                c.check_weak_map_set_collision(node);
            });
        }
    }

    // checker.go:10745
    pub(crate) fn check_weak_map_set_collision(&mut self, node: P<Node>) {
        let enclosing_block_scope = get_enclosing_block_scope_container(node).unwrap();
        if self.node_links.get(enclosing_block_scope).flags.get().intersects(NodeCheckFlags::ContainsClassWithPrivateIdentifiers) {
            let name = node.name();
            if let Some(name) = name {
                if is_identifier(name) {
                    self.error_skipped_on_no_emit(node, &diagnostics::Compiler_reserves_name_0_when_emitting_private_identifier_downlevel, &[&name.text()]);
                }
            }
        }
    }

    // checker.go:10755
    pub(crate) fn check_collision_with_global_promise_in_generated_code(&mut self, node: P<Node>, name: Option<P<Node>>) {
        let Some(name_node) = name else {
            return;
        };
        if self.language_version >= ScriptTarget::ES2017 || !self.need_collision_check_for_identifier(node, name, "Promise") {
            return;
        }
        // Uninstantiated modules shouldn't do this check
        if is_module_declaration(node) && get_module_instance_state(node) != ModuleInstanceState::Instantiated {
            return;
        }
        // In case of variable declaration, node.parent is variable statement so look at the variable statement's parent
        let parent = get_declaration_container(node).unwrap();
        if is_source_file(parent) && is_external_or_common_js_module(parent.as_source_file_p()) && parent.flags().intersects(NodeFlags::HasAsyncFunctions) {
            // If the declaration happens to be in external module, report error that Promise is a reserved identifier.
            let s = tsrs_scanner::declaration_name_to_string(Some(name_node));
            self.error_skipped_on_no_emit(name_node, &diagnostics::Duplicate_identifier_0_Compiler_reserves_name_1_in_top_level_scope_of_a_module_containing_async_functions, &[&s, &s]);
        }
    }

    // checker.go:10771
    pub(crate) fn record_potential_collision_with_reflect_in_generated_code(&mut self, node: P<Node>, name: Option<P<Node>>) {
        if name.is_some() && self.language_version <= ScriptTarget::ES2021 && self.need_collision_check_for_identifier(node, name, "Reflect") {
            self.add_deferred_diagnostic(move |c| {
                c.check_reflect_collision(node);
            });
        }
    }

    // checker.go:10779
    pub(crate) fn check_reflect_collision(&mut self, node: P<Node>) {
        let mut has_collision = false;
        if is_class_expression(node) {
            // ClassExpression names don't contribute to their containers, but do matter for any of their block-scoped members.
            for &member in node.members() {
                if self.node_links.get(member).flags.get().intersects(NodeCheckFlags::ContainsSuperPropertyInStaticInitializer) {
                    has_collision = true;
                    break;
                }
            }
        } else if is_function_expression(node) {
            // FunctionExpression names don't contribute to their containers, but do matter for their contents
            if self.node_links.get(node).flags.get().intersects(NodeCheckFlags::ContainsSuperPropertyInStaticInitializer) {
                has_collision = true;
            }
        } else {
            let container = get_enclosing_block_scope_container(node);
            if let Some(container) = container {
                if self.node_links.get(container).flags.get().intersects(NodeCheckFlags::ContainsSuperPropertyInStaticInitializer) {
                    has_collision = true;
                }
            }
        }
        if has_collision {
            let name = node.name();
            if let Some(name) = name {
                if is_identifier(name) {
                    let s = tsrs_scanner::declaration_name_to_string(Some(name));
                    self.error_skipped_on_no_emit(node, &diagnostics::Duplicate_identifier_0_Compiler_reserves_name_1_when_emitting_super_references_in_static_initializers, &[&s, &"Reflect"]);
                }
            }
        }
    }

    // checker.go:10808
    pub(crate) fn check_class_name_collision_with_object(&mut self, name: P<Node>) {
        if name.text() == "Object" && self.program.get_emit_module_format_of_file(get_source_file_of_node(name).unwrap()) < ModuleKind::ES2015 {
            let module_kind = self.module_kind.to_string();
            self.error(Some(name), &diagnostics::Class_name_cannot_be_Object_when_targeting_ES5_and_above_with_module_0, &[&module_kind]);
        }
    }

    // checker.go:10814
    pub(crate) fn check_type_of_expression(&mut self, node: P<Node>) -> P<Type> {
        self.check_expression(node.expression().unwrap());
        self.typeof_type
    }

    // checker.go:10819
    pub(crate) fn check_non_null_assertion(&mut self, node: P<Node>) -> P<Type> {
        if node.flags().intersects(NodeFlags::OptionalChain) {
            // checkNonNullChain checks the same operand expression (node.Expression()),
            // so the child is still visited on this branch.
            return self.check_non_null_chain(node);
        }
        let t = self.check_expression(node.expression().unwrap());
        self.get_non_nullable_type(t)
    }

    // checker.go:10828
    pub(crate) fn check_non_null_chain(&mut self, node: P<Node>) -> P<Type> {
        let expression = node.expression().unwrap();
        let left_type = self.check_expression(expression);
        let non_optional_type = self.get_optional_expression_type(left_type, expression);
        let non_nullable = self.get_non_nullable_type(non_optional_type);
        self.propagate_optional_type_marker(non_nullable, node, non_optional_type != left_type)
    }

    // checker.go:10834
    pub(crate) fn check_expression_with_type_arguments(&mut self, node: P<Node>) -> P<Type> {
        self.check_grammar_expression_with_type_arguments(node);
        self.check_source_elements(node.type_arguments());
        if is_expression_with_type_arguments(node) {
            let parent = walk_up_parenthesized_expressions(node.parent()).unwrap();
            if is_binary_expression(parent) && parent.as_binary_expression().operator_token.kind() == Kind::InstanceOfKeyword && crate::is_node_descendant_of(Some(node), parent.as_binary_expression().right()) {
                self.error(Some(node), &diagnostics::The_right_hand_side_of_an_instanceof_expression_must_not_be_an_instantiation_expression, &[]);
            }
        }
        let expr_type = if is_expression_with_type_arguments(node) {
            self.check_expression(node.expression().unwrap())
        } else {
            let expr_name = node.as_type_query_node().expr_name;
            if is_this_identifier(expr_name) {
                self.check_this_expression(node.as_type_query_node().expr_name)
            } else {
                self.check_expression(node.as_type_query_node().expr_name)
            }
        };
        self.get_instantiation_expression_type(expr_type, node)
    }
}
