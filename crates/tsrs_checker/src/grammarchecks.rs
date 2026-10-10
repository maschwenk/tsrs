use crate::*;
use tsrs_ast::*;
use tsrs_ast as ast;
use tsrs_diagnostics as diagnostics;
use tsrs_diagnostics::Message;
use rustc_hash::FxHashMap;
use std::fmt::Display;
use tsrs_binder as binder;
use tsrs_core::{stringutil, tspath, TextRange};
use tsrs_scanner as scanner;

impl Checker {
    // grammarchecks.go:19
    pub(crate) fn grammar_error_on_first_token(&mut self, node: P<Node>, message: &'static Message, args: &[&dyn Display]) -> bool {
        let source_file = ast::get_source_file_of_node(node).unwrap();
        if !self.has_parse_diagnostics(source_file) {
            let span = scanner::get_range_of_token_at_position(source_file, node.pos());
            self.add_diagnostic(ast::new_diagnostic(Some(source_file), span, message, args));
            return true;
        }
        false
    }

    // grammarchecks.go:29
    pub(crate) fn grammar_error_at_pos(&mut self, node_for_source_file: P<Node>, start: i32, length: i32, message: &'static Message, args: &[&dyn Display]) -> bool {
        let source_file = ast::get_source_file_of_node(node_for_source_file).unwrap();
        if !self.has_parse_diagnostics(source_file) {
            self.add_diagnostic(ast::new_diagnostic(Some(source_file), TextRange::new(start, start + length), message, args));
            return true;
        }
        false
    }

    // grammarchecks.go:38
    pub(crate) fn grammar_error_on_node(&mut self, node: P<Node>, message: &'static Message, args: &[&dyn Display]) -> bool {
        let source_file = ast::get_source_file_of_node(node).unwrap();
        if !self.has_parse_diagnostics(source_file) {
            self.error(Some(node), message, args);
            return true;
        }
        false
    }

    // grammarchecks.go:47
    pub(crate) fn grammar_error_on_node_skipped_on_no_emit(&mut self, node: P<Node>, message: &'static Message, args: &[&dyn Display]) -> bool {
        let source_file = ast::get_source_file_of_node(node).unwrap();
        if !self.has_parse_diagnostics(source_file) {
            let d = new_diagnostic_for_node(Some(node), Some(message), args);
            d.set_skipped_on_no_emit();
            self.add_diagnostic(d);
            return true;
        }
        false
    }
}

// grammarchecks.go:58
pub(crate) fn get_identifier_from_entity_name_expression(node: P<Node>) -> Option<P<Node>> {
    match node.kind() {
        Kind::Identifier => Some(node),
        Kind::PropertyAccessExpression => Some(node.as_property_access_expression().name),
        _ => None,
    }
}

// Body of the scanner error callback installed by checkGrammarRegularExpressionLiteral. The Rust scanner buffers
// errors instead of calling back, so they are drained (in report order) right after every scanner call.
fn on_reg_exp_scanner_errors(c: &mut Checker, s: &mut Scanner, source_file: P<SourceFile>, last_error: &mut Option<P<Diagnostic>>) {
    for e in s.take_errors() {
        let (message, start, length) = (e.message, e.start, e.length);
        let args: Vec<&dyn Display> = e.args.iter().map(|a| a as &dyn Display).collect();
        if message.category() == diagnostics::Category::Message && last_error.is_some_and(|le| start == le.pos() && length == le.len()) {
            // For providing spelling suggestions.
            let err = ast::new_diagnostic(None, TextRange::new(start, start + length), message, &args);
            last_error.unwrap().add_related_info(err);
        } else if last_error.is_none_or(|le| start != le.pos()) {
            let d = ast::new_diagnostic(Some(source_file), TextRange::new(start, start + length), message, &args);
            *last_error = Some(c.add_diagnostic(d));
        }
    }
}

impl Checker {
    // grammarchecks.go:69
    pub(crate) fn check_grammar_regular_expression_literal(&mut self, node: P<Node>) -> bool {
        let source_file = ast::get_source_file_of_node(node).unwrap();
        if !self.has_parse_diagnostics(source_file) {
            let mut last_error: Option<P<Diagnostic>> = None;
            if self.reg_exp_scanner.is_none() {
                self.reg_exp_scanner = Some(Box::new(Scanner::new()));
            }
            let mut s = self.reg_exp_scanner.take().unwrap();
            s.set_script_target(self.language_version);
            s.set_language_variant(source_file.language_variant());
            s.set_on_error(true);
            s.set_text(source_file.text());
            s.reset_token_state(node.pos());
            s.scan();
            on_reg_exp_scanner_errors(self, &mut s, source_file, &mut last_error);
            let token_is_regular_expression_literal = s.re_scan_slash_token(true) == Kind::RegularExpressionLiteral;
            on_reg_exp_scanner_errors(self, &mut s, source_file, &mut last_error);
            s.set_text("");
            s.set_on_error(false);
            self.reg_exp_scanner = Some(s);
            assert!(token_is_regular_expression_literal);
            return last_error.is_some();
        }
        false
    }

    // grammarchecks.go:100
    pub(crate) fn check_grammar_private_identifier_expression(&mut self, priv_id: P<Node>) -> bool {
        let priv_id_as_node = priv_id;
        if ast::get_containing_class(priv_id).is_none() {
            return self.grammar_error_on_node(priv_id, &diagnostics::Private_identifiers_are_not_allowed_outside_class_bodies, &[]);
        }

        let parent = priv_id.parent().unwrap();
        if !is_for_in_statement(parent) {
            if !ast::is_expression_node(priv_id_as_node) {
                return self.grammar_error_on_node(priv_id_as_node, &diagnostics::Private_identifiers_are_only_allowed_in_class_bodies_and_may_only_be_used_as_part_of_a_class_member_declaration_property_access_or_on_the_left_hand_side_of_an_in_expression, &[]);
            }

            let is_in_operation = is_binary_expression(parent) && parent.as_binary_expression().operator_token.kind() == Kind::InKeyword;
            if self.get_symbol_for_private_identifier_expression(priv_id_as_node).is_none() && !is_in_operation {
                return self.grammar_error_on_node(priv_id_as_node, &diagnostics::Cannot_find_name_0, &[&priv_id.text()]);
            }
        }

        false
    }

    // grammarchecks.go:120
    pub(crate) fn check_grammar_mapped_type(&mut self, node: P<Node>) -> bool {
        let members = node.as_mapped_type_node().members.unwrap();
        if !members.nodes().is_empty() {
            return self.grammar_error_on_node(members.nodes()[0], &diagnostics::A_mapped_type_may_not_declare_properties_or_methods, &[]);
        }
        false
    }

    // grammarchecks.go:127
    pub(crate) fn check_grammar_decorator(&mut self, decorator: P<Node>) -> bool {
        let source_file = ast::get_source_file_of_node(decorator).unwrap();
        if !self.has_parse_diagnostics(source_file) {
            let mut node = decorator.as_decorator().expression;

            // DecoratorParenthesizedExpression :
            //   `(` Expression `)`

            if is_parenthesized_expression(node) {
                return false;
            }

            let mut can_have_call_expression = true;
            let mut error_node: Option<P<Node>> = None;
            loop {
                // Allow TS syntax such as non-null assertions and instantiation expressions
                if is_expression_with_type_arguments(node) || is_non_null_expression(node) {
                    node = node.expression().unwrap();
                    continue;
                }

                // DecoratorCallExpression :
                //   DecoratorMemberExpression Arguments

                if is_call_expression(node) {
                    let call_expr = node.as_call_expression();
                    if !can_have_call_expression {
                        error_node = Some(node);
                    }
                    if call_expr.question_dot_token().is_some() {
                        // Even if we already have an error node, error at the `?.` token since it appears earlier.
                        error_node = call_expr.question_dot_token();
                    }
                    node = call_expr.expression;
                    can_have_call_expression = false;
                    continue;
                }

                // DecoratorMemberExpression :
                //   IdentifierReference
                //   DecoratorMemberExpression `.` IdentifierName
                //   DecoratorMemberExpression `.` PrivateIdentifier

                if is_property_access_expression(node) {
                    let property_access_expr = node.as_property_access_expression();
                    if property_access_expr.question_dot_token().is_some() {
                        // Even if we already have an error node, error at the `?.` token since it appears earlier.
                        error_node = property_access_expr.question_dot_token();
                    }
                    node = property_access_expr.expression;
                    can_have_call_expression = false;
                    continue;
                }

                if !is_identifier(node) {
                    // Even if we already have an error node, error at this node since it appears earlier.
                    error_node = Some(node);
                }

                break;
            }

            if let Some(error_node) = error_node {
                let err = self.error(Some(decorator.as_decorator().expression), &diagnostics::Expression_must_be_enclosed_in_parentheses_to_be_used_as_a_decorator, &[]);
                err.add_related_info(create_diagnostic_for_node(Some(error_node), &diagnostics::Invalid_syntax_in_decorator, &[]));
                return true;
            }
        }

        false
    }

    // grammarchecks.go:199
    pub(crate) fn check_grammar_export_declaration(&mut self, node: P<Node>) -> bool {
        let export_decl = node.as_export_declaration();
        if export_decl.is_type_only {
            if let Some(export_clause) = export_decl.export_clause {
                if export_clause.kind() == Kind::NamedExports {
                    return self.check_grammar_type_only_named_imports_or_exports(export_clause);
                }
            }
        }
        false
    }

    // grammarchecks.go:206
    pub(crate) fn check_grammar_module_element_context(&mut self, node: P<Node>, error_message: &'static Message) -> bool {
        let parent_kind = node.parent().unwrap().kind();
        let is_in_appropriate_context = parent_kind == Kind::SourceFile || parent_kind == Kind::ModuleBlock || parent_kind == Kind::ModuleDeclaration;
        if !is_in_appropriate_context {
            self.grammar_error_on_first_token(node, error_message, &[]);
        }
        !is_in_appropriate_context
    }

    // grammarchecks.go:214
    pub(crate) fn check_grammar_modifiers(&mut self, node: P<Node>) -> bool {
        if node.modifiers().is_none() {
            return false;
        }
        if self.report_obvious_decorator_errors(node) || self.report_obvious_modifier_errors(node) {
            return true;
        }
        if ast::is_this_parameter(node) {
            return self.grammar_error_on_first_token(node, &diagnostics::Neither_decorators_nor_modifiers_may_be_applied_to_this_parameters, &[]);
        }
        let mut block_scope_kind = NodeFlags::None;
        if is_variable_statement(node) {
            block_scope_kind = node.as_variable_statement().declaration_list.flags() & NodeFlags::BlockScoped;
        }
        let mut last_static: Option<P<Node>> = None;
        let mut last_declare: Option<P<Node>> = None;
        let mut last_async: Option<P<Node>> = None;
        let mut last_override: Option<P<Node>> = None;
        let mut first_decorator: Option<P<Node>> = None;
        let mut flags = ModifierFlags::None;
        let mut saw_export_before_decorators = false;
        // We parse decorators and modifiers in four contiguous chunks:
        // [...leadingDecorators, ...leadingModifiers, ...trailingDecorators, ...trailingModifiers]. It is an error to
        // have both leading and trailing decorators.
        let mut has_leading_decorators = false;
        let parent = node.parent().unwrap();
        let modifiers = node.modifier_nodes();
        for &modifier in modifiers {
            if is_decorator(modifier) {
                if !ast::node_can_be_decorated(self.legacy_decorators, node, Some(parent), parent.parent()) {
                    if node.kind() == Kind::MethodDeclaration && !ast::node_is_present(node.body()) {
                        return self.grammar_error_on_first_token(node, &diagnostics::A_decorator_can_only_decorate_a_method_implementation_not_an_overload, &[]);
                    } else {
                        return self.grammar_error_on_first_token(node, &diagnostics::Decorators_are_not_valid_here, &[]);
                    }
                } else if self.legacy_decorators && (node.kind() == Kind::GetAccessor || node.kind() == Kind::SetAccessor) {
                    let symbol = self.get_symbol_of_declaration(node).unwrap();
                    let declarations: Vec<P<Node>> = symbol.declarations().to_vec();
                    let accessors = ast::get_all_accessor_declarations_for_declaration(node, &declarations);
                    if ast::has_decorators(accessors.first_accessor) && Some(node) == accessors.second_accessor {
                        return self.grammar_error_on_first_token(node, &diagnostics::Decorators_cannot_be_applied_to_multiple_get_Slashset_accessors_of_the_same_name, &[]);
                    }
                }

                // if we've seen any modifiers aside from `export`, `default`, or another decorator, then this is an invalid position
                if flags.intersects(!(ModifierFlags::ExportDefault | ModifierFlags::Decorator)) {
                    return self.grammar_error_on_node(modifier, &diagnostics::Decorators_are_not_valid_here, &[]);
                }

                // if we've already seen leading decorators and leading modifiers, then trailing decorators are an invalid position
                if has_leading_decorators && flags.intersects(ModifierFlags::Modifier) {
                    if first_decorator.is_none() {
                        panic!("Expected firstDecorator to be set");
                    }
                    let source_file = ast::get_source_file_of_node(modifier).unwrap();
                    if !self.has_parse_diagnostics(source_file) {
                        let err = self.error(Some(modifier), &diagnostics::Decorators_may_not_appear_after_export_or_export_default_if_they_also_appear_before_export, &[]);
                        err.add_related_info(create_diagnostic_for_node(first_decorator, &diagnostics::Decorator_used_before_export_here, &[]));
                        return true;
                    }
                    return false;
                }

                flags |= ModifierFlags::Decorator;

                // if we have not yet seen a modifier, then these are leading decorators
                if !flags.intersects(ModifierFlags::Modifier) {
                    has_leading_decorators = true;
                } else if flags.intersects(ModifierFlags::Export) {
                    saw_export_before_decorators = true;
                }

                if first_decorator.is_none() {
                    first_decorator = Some(modifier);
                }
            } else {
                let modifier_not_reparsed = !modifier.flags().intersects(NodeFlags::Reparsed);
                if modifier.kind() != Kind::ReadonlyKeyword {
                    if node.kind() == Kind::PropertySignature || node.kind() == Kind::MethodSignature {
                        return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_appear_on_a_type_member, &[&scanner::token_to_string(modifier.kind())]);
                    }
                    if node.kind() == Kind::IndexSignature && (modifier.kind() != Kind::StaticKeyword || !ast::is_class_like(parent)) {
                        return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_appear_on_an_index_signature, &[&scanner::token_to_string(modifier.kind())]);
                    }
                }
                if modifier.kind() != Kind::InKeyword && modifier.kind() != Kind::OutKeyword && modifier.kind() != Kind::ConstKeyword {
                    if node.kind() == Kind::TypeParameter {
                        return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_appear_on_a_type_parameter, &[&scanner::token_to_string(modifier.kind())]);
                    }
                }
                match modifier.kind() {
                    Kind::ConstKeyword => {
                        if node.kind() != Kind::EnumDeclaration && node.kind() != Kind::TypeParameter {
                            return self.grammar_error_on_node(node, &diagnostics::A_class_member_cannot_have_the_0_keyword, &[&scanner::token_to_string(Kind::ConstKeyword)]);
                        }
                        if node.kind() == Kind::TypeParameter {
                            if !(ast::is_function_like_declaration(parent) || ast::is_class_like(parent) || is_function_type_node(parent) || is_constructor_type_node(parent) || is_call_signature_declaration(parent) || is_construct_signature_declaration(parent) || ast::is_method_signature_declaration(parent)) {
                                return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_can_only_appear_on_a_type_parameter_of_a_function_method_or_class, &[&scanner::token_to_string(modifier.kind())]);
                            }
                        }
                    }
                    Kind::OverrideKeyword => {
                        // If node.kind === SyntaxKind.Parameter, checkParameter reports an error if it's not a parameter property.
                        if flags.intersects(ModifierFlags::Override) {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_already_seen, &[&"override"]);
                        } else if flags.intersects(ModifierFlags::Ambient) {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_be_used_with_1_modifier, &[&"override", &"declare"]);
                        } else if flags.intersects(ModifierFlags::Readonly) && modifier_not_reparsed {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_must_precede_1_modifier, &[&"override", &"readonly"]);
                        } else if flags.intersects(ModifierFlags::Accessor) && modifier_not_reparsed {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_must_precede_1_modifier, &[&"override", &"accessor"]);
                        } else if flags.intersects(ModifierFlags::Async) && modifier_not_reparsed {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_must_precede_1_modifier, &[&"override", &"async"]);
                        }
                        flags |= ModifierFlags::Override;
                        last_override = Some(modifier);
                    }
                    Kind::PublicKeyword | Kind::ProtectedKeyword | Kind::PrivateKeyword => {
                        let text = visibility_to_string(ast::modifier_to_flag(modifier.kind()));

                        if flags.intersects(ModifierFlags::AccessibilityModifier) {
                            return self.grammar_error_on_node(modifier, &diagnostics::Accessibility_modifier_already_seen, &[]);
                        } else if flags.intersects(ModifierFlags::Override) && modifier_not_reparsed {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_must_precede_1_modifier, &[&text, &"override"]);
                        } else if flags.intersects(ModifierFlags::Static) && modifier_not_reparsed {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_must_precede_1_modifier, &[&text, &"static"]);
                        } else if flags.intersects(ModifierFlags::Accessor) && modifier_not_reparsed {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_must_precede_1_modifier, &[&text, &"accessor"]);
                        } else if flags.intersects(ModifierFlags::Readonly) && modifier_not_reparsed {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_must_precede_1_modifier, &[&text, &"readonly"]);
                        } else if flags.intersects(ModifierFlags::Async) && modifier_not_reparsed {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_must_precede_1_modifier, &[&text, &"async"]);
                        } else if parent.kind() == Kind::ModuleBlock || parent.kind() == Kind::SourceFile {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_appear_on_a_module_or_namespace_element, &[&text]);
                        } else if flags.intersects(ModifierFlags::Abstract) {
                            if modifier.kind() == Kind::PrivateKeyword {
                                return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_be_used_with_1_modifier, &[&text, &"abstract"]);
                            } else if modifier_not_reparsed {
                                return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_must_precede_1_modifier, &[&text, &"abstract"]);
                            }
                        } else if ast::is_private_identifier_class_element_declaration(node) {
                            return self.grammar_error_on_node(modifier, &diagnostics::An_accessibility_modifier_cannot_be_used_with_a_private_identifier, &[]);
                        }
                        flags |= ast::modifier_to_flag(modifier.kind());
                    }
                    Kind::StaticKeyword => {
                        if flags.intersects(ModifierFlags::Static) {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_already_seen, &[&"static"]);
                        } else if flags.intersects(ModifierFlags::Readonly) && modifier_not_reparsed {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_must_precede_1_modifier, &[&"static", &"readonly"]);
                        } else if flags.intersects(ModifierFlags::Async) && modifier_not_reparsed {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_must_precede_1_modifier, &[&"static", &"async"]);
                        } else if flags.intersects(ModifierFlags::Accessor) && modifier_not_reparsed {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_must_precede_1_modifier, &[&"static", &"accessor"]);
                        } else if parent.kind() == Kind::ModuleBlock || parent.kind() == Kind::SourceFile {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_appear_on_a_module_or_namespace_element, &[&"static"]);
                        } else if node.kind() == Kind::Parameter {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_appear_on_a_parameter, &[&"static"]);
                        } else if flags.intersects(ModifierFlags::Abstract) {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_be_used_with_1_modifier, &[&"static", &"abstract"]);
                        } else if flags.intersects(ModifierFlags::Override) && modifier_not_reparsed {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_must_precede_1_modifier, &[&"static", &"override"]);
                        }
                        flags |= ModifierFlags::Static;
                        last_static = Some(modifier);
                    }
                    Kind::AccessorKeyword => {
                        if flags.intersects(ModifierFlags::Accessor) {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_already_seen, &[&"accessor"]);
                        } else if flags.intersects(ModifierFlags::Readonly) {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_be_used_with_1_modifier, &[&"accessor", &"readonly"]);
                        } else if flags.intersects(ModifierFlags::Ambient) {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_be_used_with_1_modifier, &[&"accessor", &"declare"]);
                        } else if node.kind() != Kind::PropertyDeclaration {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_accessor_modifier_can_only_appear_on_a_property_declaration, &[]);
                        }

                        flags |= ModifierFlags::Accessor;
                    }
                    Kind::ReadonlyKeyword => {
                        if flags.intersects(ModifierFlags::Readonly) {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_already_seen, &[&"readonly"]);
                        } else if node.kind() != Kind::PropertyDeclaration && node.kind() != Kind::PropertySignature && node.kind() != Kind::IndexSignature && node.kind() != Kind::Parameter {
                            // If node.kind === SyntaxKind.Parameter, checkParameter reports an error if it's not a parameter property.
                            return self.grammar_error_on_node(modifier, &diagnostics::X_readonly_modifier_can_only_appear_on_a_property_declaration_or_index_signature, &[]);
                        } else if flags.intersects(ModifierFlags::Accessor) {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_be_used_with_1_modifier, &[&"readonly", &"accessor"]);
                        }
                        flags |= ModifierFlags::Readonly;
                    }
                    Kind::ExportKeyword => {
                        if self.compiler_options.verbatim_module_syntax == Tristate::True
                            && !node.flags().intersects(NodeFlags::Ambient)
                            && node.kind() != Kind::TypeAliasDeclaration
                            && node.kind() != Kind::InterfaceDeclaration
                            && node.kind() != Kind::ModuleDeclaration
                            && parent.kind() == Kind::SourceFile
                            && self.program.get_emit_module_format_of_file(ast::get_source_file_of_node(node).unwrap()) == ModuleKind::CommonJS
                        {
                            return self.grammar_error_on_node(modifier, &diagnostics::A_top_level_export_modifier_cannot_be_used_on_value_declarations_in_a_CommonJS_module_when_verbatimModuleSyntax_is_enabled, &[]);
                        }
                        if flags.intersects(ModifierFlags::Export) {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_already_seen, &[&"export"]);
                        } else if flags.intersects(ModifierFlags::Ambient) && modifier_not_reparsed {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_must_precede_1_modifier, &[&"export", &"declare"]);
                        } else if flags.intersects(ModifierFlags::Abstract) && modifier_not_reparsed {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_must_precede_1_modifier, &[&"export", &"abstract"]);
                        } else if flags.intersects(ModifierFlags::Async) && modifier_not_reparsed {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_must_precede_1_modifier, &[&"export", &"async"]);
                        } else if ast::is_class_like(parent) && !is_js_type_alias_declaration(node) {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_appear_on_class_elements_of_this_kind, &[&"export"]);
                        } else if node.kind() == Kind::Parameter {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_appear_on_a_parameter, &[&"export"]);
                        } else if block_scope_kind == NodeFlags::Using {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_appear_on_a_using_declaration, &[&"export"]);
                        } else if block_scope_kind == NodeFlags::AwaitUsing {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_appear_on_an_await_using_declaration, &[&"export"]);
                        }
                        flags |= ModifierFlags::Export;
                    }
                    Kind::DefaultKeyword => {
                        let container = if parent.kind() == Kind::SourceFile { parent } else { parent.parent().unwrap() };
                        if container.kind() == Kind::ModuleDeclaration && !ast::is_ambient_module(container) {
                            return self.grammar_error_on_node(modifier, &diagnostics::A_default_export_can_only_be_used_in_an_ECMAScript_style_module, &[]);
                        } else if block_scope_kind == NodeFlags::Using {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_appear_on_a_using_declaration, &[&"default"]);
                        } else if block_scope_kind == NodeFlags::AwaitUsing {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_appear_on_an_await_using_declaration, &[&"default"]);
                        } else if !flags.intersects(ModifierFlags::Export) && modifier_not_reparsed {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_must_precede_1_modifier, &[&"export", &"default"]);
                        } else if saw_export_before_decorators {
                            return self.grammar_error_on_node(first_decorator.unwrap(), &diagnostics::Decorators_are_not_valid_here, &[]);
                        }

                        flags |= ModifierFlags::Default;
                    }
                    Kind::DeclareKeyword => {
                        if flags.intersects(ModifierFlags::Ambient) {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_already_seen, &[&"declare"]);
                        } else if flags.intersects(ModifierFlags::Async) {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_be_used_in_an_ambient_context, &[&"async"]);
                        } else if flags.intersects(ModifierFlags::Override) {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_be_used_in_an_ambient_context, &[&"override"]);
                        } else if ast::is_class_like(parent) && !is_property_declaration(node) {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_appear_on_class_elements_of_this_kind, &[&"declare"]);
                        } else if node.kind() == Kind::Parameter {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_appear_on_a_parameter, &[&"declare"]);
                        } else if block_scope_kind == NodeFlags::Using {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_appear_on_a_using_declaration, &[&"declare"]);
                        } else if block_scope_kind == NodeFlags::AwaitUsing {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_appear_on_an_await_using_declaration, &[&"declare"]);
                        } else if parent.flags().intersects(NodeFlags::Ambient) && parent.kind() == Kind::ModuleBlock {
                            return self.grammar_error_on_node(modifier, &diagnostics::A_declare_modifier_cannot_be_used_in_an_already_ambient_context, &[]);
                        } else if ast::is_private_identifier_class_element_declaration(node) {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_be_used_with_a_private_identifier, &[&"declare"]);
                        } else if flags.intersects(ModifierFlags::Accessor) {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_be_used_with_1_modifier, &[&"declare", &"accessor"]);
                        }
                        flags |= ModifierFlags::Ambient;
                        last_declare = Some(modifier);
                    }
                    Kind::AbstractKeyword => {
                        if flags.intersects(ModifierFlags::Abstract) {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_already_seen, &[&"abstract"]);
                        }
                        if node.kind() != Kind::ClassDeclaration && node.kind() != Kind::ConstructorType {
                            if node.kind() != Kind::MethodDeclaration && node.kind() != Kind::PropertyDeclaration && node.kind() != Kind::GetAccessor && node.kind() != Kind::SetAccessor {
                                return self.grammar_error_on_node(modifier, &diagnostics::X_abstract_modifier_can_only_appear_on_a_class_method_or_property_declaration, &[]);
                            }
                            if !(parent.kind() == Kind::ClassDeclaration && ast::has_syntactic_modifier(parent, ModifierFlags::Abstract)) {
                                let message = if node.kind() == Kind::PropertyDeclaration {
                                    &diagnostics::Abstract_properties_can_only_appear_within_an_abstract_class
                                } else {
                                    &diagnostics::Abstract_methods_can_only_appear_within_an_abstract_class
                                };
                                return self.grammar_error_on_node(modifier, message, &[]);
                            }
                            if flags.intersects(ModifierFlags::Static) {
                                return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_be_used_with_1_modifier, &[&"static", &"abstract"]);
                            }
                            if flags.intersects(ModifierFlags::Private) {
                                return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_be_used_with_1_modifier, &[&"private", &"abstract"]);
                            }
                            if flags.intersects(ModifierFlags::Async) {
                                if let Some(last_async) = last_async {
                                    return self.grammar_error_on_node(last_async, &diagnostics::X_0_modifier_cannot_be_used_with_1_modifier, &[&"async", &"abstract"]);
                                }
                            }
                            if flags.intersects(ModifierFlags::Override) && modifier_not_reparsed {
                                return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_must_precede_1_modifier, &[&"abstract", &"override"]);
                            }
                            if flags.intersects(ModifierFlags::Accessor) && modifier_not_reparsed {
                                return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_must_precede_1_modifier, &[&"abstract", &"accessor"]);
                            }
                        }
                        if let Some(name) = node.name() {
                            if name.kind() == Kind::PrivateIdentifier {
                                return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_be_used_with_a_private_identifier, &[&"abstract"]);
                            }
                        }

                        flags |= ModifierFlags::Abstract;
                    }
                    Kind::AsyncKeyword => {
                        if flags.intersects(ModifierFlags::Async) {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_already_seen, &[&"async"]);
                        } else if flags.intersects(ModifierFlags::Ambient) || parent.flags().intersects(NodeFlags::Ambient) {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_be_used_in_an_ambient_context, &[&"async"]);
                        } else if node.kind() == Kind::Parameter {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_appear_on_a_parameter, &[&"async"]);
                        }
                        if flags.intersects(ModifierFlags::Abstract) {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_cannot_be_used_with_1_modifier, &[&"async", &"abstract"]);
                        }
                        flags |= ModifierFlags::Async;
                        last_async = Some(modifier);
                    }
                    Kind::InKeyword | Kind::OutKeyword => {
                        let in_out_flag = if modifier.kind() == Kind::InKeyword { ModifierFlags::In } else { ModifierFlags::Out };
                        let in_out_text = if modifier.kind() == Kind::InKeyword { "in" } else { "out" };
                        let parent = node.parent();
                        if node.kind() != Kind::TypeParameter
                            || parent.is_some_and(|parent| !(is_interface_declaration(parent) || ast::is_class_like(parent) || ast::is_type_or_js_type_alias_declaration(parent)))
                        {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_can_only_appear_on_a_type_parameter_of_a_class_interface_or_type_alias, &[&in_out_text]);
                        }
                        if flags.intersects(in_out_flag) {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_already_seen, &[&in_out_text]);
                        }
                        if in_out_flag.intersects(ModifierFlags::In) && flags.intersects(ModifierFlags::Out) {
                            return self.grammar_error_on_node(modifier, &diagnostics::X_0_modifier_must_precede_1_modifier, &[&"in", &"out"]);
                        }
                        flags |= in_out_flag;
                    }
                    _ => {}
                }
            }
        }

        if node.kind() == Kind::Constructor {
            if flags.intersects(ModifierFlags::Static) {
                return self.grammar_error_on_node(last_static.unwrap(), &diagnostics::X_0_modifier_cannot_appear_on_a_constructor_declaration, &[&"static"]);
            }
            if flags.intersects(ModifierFlags::Override) {
                return self.grammar_error_on_node(last_override.unwrap(), &diagnostics::X_0_modifier_cannot_appear_on_a_constructor_declaration, &[&"override"]);
            }
            if flags.intersects(ModifierFlags::Async) {
                return self.grammar_error_on_node(last_async.unwrap(), &diagnostics::X_0_modifier_cannot_appear_on_a_constructor_declaration, &[&"async"]);
            }
            return false;
        } else if (node.kind() == Kind::ImportDeclaration || node.kind() == Kind::JSImportDeclaration || node.kind() == Kind::ImportEqualsDeclaration) && flags.intersects(ModifierFlags::Ambient) {
            return self.grammar_error_on_node(last_declare.unwrap(), &diagnostics::A_0_modifier_cannot_be_used_with_an_import_declaration, &[&"declare"]);
        } else if node.kind() == Kind::Parameter && flags.intersects(ModifierFlags::ParameterPropertyModifier) && node.name().is_some_and(ast::is_binding_pattern) {
            return self.grammar_error_on_node(node, &diagnostics::A_parameter_property_may_not_be_declared_using_a_binding_pattern, &[]);
        } else if node.kind() == Kind::Parameter && flags.intersects(ModifierFlags::ParameterPropertyModifier) && node.as_parameter_declaration().dot_dot_dot_token().is_some() {
            return self.grammar_error_on_node(node, &diagnostics::A_parameter_property_cannot_be_declared_using_a_rest_parameter, &[]);
        }
        if flags.intersects(ModifierFlags::Async) {
            return self.check_grammar_async_modifier(node, last_async);
        }
        false
    }

    // grammarchecks.go:571
    pub(crate) fn report_obvious_modifier_errors(&mut self, node: P<Node>) -> bool {
        let Some(modifier) = self.find_first_illegal_modifier(node) else {
            return false;
        };
        self.grammar_error_on_first_token(modifier, &diagnostics::Modifiers_cannot_appear_here, &[])
    }

    // grammarchecks.go:579
    pub(crate) fn find_first_modifier_except(&mut self, node: P<Node>, allowed_modifier: Kind) -> Option<P<Node>> {
        let modifier = node.modifier_nodes().iter().copied().find(|&m| ast::is_modifier(m));
        if let Some(modifier) = modifier {
            if modifier.kind() != allowed_modifier {
                return Some(modifier);
            }
        }
        None
    }

    // grammarchecks.go:587
    pub(crate) fn find_first_illegal_modifier(&mut self, node: P<Node>) -> Option<P<Node>> {
        match node.kind() {
            Kind::GetAccessor
            | Kind::SetAccessor
            | Kind::Constructor
            | Kind::PropertyDeclaration
            | Kind::PropertySignature
            | Kind::MethodDeclaration
            | Kind::MethodSignature
            | Kind::IndexSignature
            | Kind::ModuleDeclaration
            | Kind::ImportDeclaration
            | Kind::JSImportDeclaration
            | Kind::ImportEqualsDeclaration
            | Kind::ExportDeclaration
            | Kind::ExportAssignment
            | Kind::FunctionExpression
            | Kind::ArrowFunction
            | Kind::Parameter
            | Kind::TypeParameter
            | Kind::JSTypeAliasDeclaration => None,
            Kind::ClassStaticBlockDeclaration | Kind::PropertyAssignment | Kind::ShorthandPropertyAssignment | Kind::NamespaceExportDeclaration | Kind::MissingDeclaration => {
                node.modifier_nodes().iter().copied().find(|&m| ast::is_modifier(m))
            }
            _ => {
                let parent = node.parent().unwrap();
                if parent.kind() == Kind::ModuleBlock || parent.kind() == Kind::SourceFile {
                    return None;
                }
                match node.kind() {
                    Kind::FunctionDeclaration => self.find_first_modifier_except(node, Kind::AsyncKeyword),
                    Kind::ClassDeclaration | Kind::ConstructorType => self.find_first_modifier_except(node, Kind::AbstractKeyword),
                    Kind::ClassExpression | Kind::InterfaceDeclaration | Kind::TypeAliasDeclaration => node.modifier_nodes().iter().copied().find(|&m| ast::is_modifier(m)),
                    Kind::VariableStatement => {
                        if node.as_variable_statement().declaration_list.flags().intersects(NodeFlags::Using) {
                            return self.find_first_modifier_except(node, Kind::AwaitKeyword);
                        }
                        node.modifier_nodes().iter().copied().find(|&m| ast::is_modifier(m))
                    }
                    Kind::EnumDeclaration => self.find_first_modifier_except(node, Kind::ConstKeyword),
                    _ => panic!("Unhandled case in findFirstIllegalModifier."),
                }
            }
        }
    }

    // grammarchecks.go:642
    pub(crate) fn report_obvious_decorator_errors(&mut self, node: P<Node>) -> bool {
        let Some(decorator) = self.find_first_illegal_decorator(node) else {
            return false;
        };
        self.grammar_error_on_first_token(decorator, &diagnostics::Decorators_are_not_valid_here, &[])
    }

    // grammarchecks.go:650
    pub(crate) fn find_first_illegal_decorator(&mut self, node: P<Node>) -> Option<P<Node>> {
        if ast::can_have_illegal_decorators(node) {
            node.modifier_nodes().iter().copied().find(|&m| is_decorator(m))
        } else {
            None
        }
    }

    // grammarchecks.go:659
    pub(crate) fn check_grammar_async_modifier(&mut self, node: P<Node>, async_modifier: Option<P<Node>>) -> bool {
        match node.kind() {
            Kind::MethodDeclaration | Kind::FunctionDeclaration | Kind::FunctionExpression | Kind::ArrowFunction => return false,
            _ => {}
        }

        self.grammar_error_on_node(async_modifier.unwrap(), &diagnostics::X_0_modifier_cannot_be_used_here, &[&"async"])
    }

    // grammarchecks.go:671
    pub(crate) fn check_grammar_for_disallowed_trailing_comma(&mut self, list: Option<P<NodeList>>, diag: &'static Message) -> bool {
        if let Some(list) = list {
            if list.has_trailing_comma() {
                return self.grammar_error_at_pos(list.nodes()[0], list.end() - ",".len() as i32, ",".len() as i32, diag, &[]);
            }
        }
        false
    }

    // grammarchecks.go:678
    pub(crate) fn check_grammar_type_parameter_list(&mut self, type_parameters: Option<P<NodeList>>, file: P<SourceFile>) -> bool {
        if let Some(type_parameters) = type_parameters {
            if type_parameters.nodes().is_empty() {
                let start = type_parameters.pos() - "<".len() as i32;
                let end = scanner::skip_trivia(file.text(), type_parameters.end()) + ">".len() as i32;
                return self.grammar_error_at_pos(file.as_node(), start, end - start, &diagnostics::Type_parameter_list_cannot_be_empty, &[]);
            }
        }
        false
    }

    // grammarchecks.go:687
    pub(crate) fn check_grammar_parameter_list(&mut self, parameters: P<NodeList>) -> bool {
        let mut seen_optional_parameter = false;
        let parameter_count = parameters.nodes().len();

        for i in 0..parameter_count {
            let parameter_node = parameters.nodes()[i];
            let parameter = parameter_node.as_parameter_declaration();
            if let Some(dot_dot_dot_token) = parameter.dot_dot_dot_token() {
                if i != parameter_count - 1 {
                    return self.grammar_error_on_node(dot_dot_dot_token, &diagnostics::A_rest_parameter_must_be_last_in_a_parameter_list, &[]);
                }
                if !parameter_node.flags().intersects(NodeFlags::Ambient) {
                    self.check_grammar_for_disallowed_trailing_comma(Some(parameters), &diagnostics::A_rest_parameter_or_binding_pattern_may_not_have_a_trailing_comma);
                }

                if let Some(question_token) = parameter.question_token() {
                    return self.grammar_error_on_node(question_token, &diagnostics::A_rest_parameter_cannot_be_optional, &[]);
                }

                if parameter.initializer().is_some() {
                    return self.grammar_error_on_node(parameter.name, &diagnostics::A_rest_parameter_cannot_have_an_initializer, &[]);
                }
            } else if crate::is_optional_declaration(parameter_node) {
                seen_optional_parameter = true;
                // A reparsed '?' token indicates a bracketed name in @param tag
                if parameter.question_token().is_some_and(|q| !q.flags().intersects(NodeFlags::Reparsed)) && parameter.initializer().is_some() {
                    return self.grammar_error_on_node(parameter.name, &diagnostics::Parameter_cannot_have_question_mark_and_initializer, &[]);
                }
            } else if seen_optional_parameter && parameter.initializer().is_none() {
                return self.grammar_error_on_node(parameter.name, &diagnostics::A_required_parameter_cannot_follow_an_optional_parameter, &[]);
            }
        }

        false
    }

    // grammarchecks.go:722
    pub(crate) fn check_grammar_for_use_strict_simple_parameter_list(&mut self, node: P<Node>) -> bool {
        if self.language_version >= ScriptTarget::ES2016 {
            let body = node.body();
            let mut use_strict_directive: Option<P<Node>> = None;
            if let Some(body) = body {
                if is_block(body) {
                    use_strict_directive = binder::find_use_strict_prologue(ast::get_source_file_of_node(node).unwrap(), body.statements());
                }
            }
            if let Some(use_strict_directive) = use_strict_directive {
                let non_simple_parameters: Vec<P<Node>> = node
                    .parameters()
                    .iter()
                    .copied()
                    .filter(|&n| {
                        let parameter = n.as_parameter_declaration();
                        parameter.initializer().is_some() || ast::is_binding_pattern(parameter.name) || crate::is_rest_parameter(n)
                    })
                    .collect();
                if !non_simple_parameters.is_empty() {
                    for &parameter in &non_simple_parameters {
                        let err = self.error(Some(parameter), &diagnostics::This_parameter_is_not_allowed_with_use_strict_directive, &[]);
                        err.add_related_info(create_diagnostic_for_node(Some(use_strict_directive), &diagnostics::X_use_strict_directive_used_here, &[]));
                    }

                    let err = self.error(Some(use_strict_directive), &diagnostics::X_use_strict_directive_cannot_be_used_with_non_simple_parameter_list, &[]);
                    for (index, &parameter) in non_simple_parameters.iter().enumerate() {
                        let related_message = if index == 0 { &diagnostics::Non_simple_parameter_declared_here } else { &diagnostics::X_and_here };
                        err.add_related_info(create_diagnostic_for_node(Some(parameter), related_message, &[]));
                    }

                    return true;
                }
            }
        }
        false
    }

    // grammarchecks.go:758
    pub(crate) fn check_grammar_function_like_declaration(&mut self, node: P<Node>) -> bool {
        // Prevent cascading error by short-circuit
        let file = ast::get_source_file_of_node(node).unwrap();
        let func_data = node.function_like_data().unwrap();
        self.check_grammar_modifiers(node)
            || self.check_grammar_type_parameter_list(func_data.type_parameters.get(), file)
            || self.check_grammar_parameter_list(func_data.parameters.get().unwrap())
            || self.check_grammar_arrow_function(node, file)
            || (ast::is_function_like_declaration(node) && self.check_grammar_for_use_strict_simple_parameter_list(node))
    }

    // grammarchecks.go:767
    pub(crate) fn check_grammar_class_like_declaration(&mut self, node: P<Node>) -> bool {
        let file = ast::get_source_file_of_node(node).unwrap();
        self.check_grammar_class_declaration_heritage_clauses(node, file) || self.check_grammar_type_parameter_list(node.type_parameter_list(), file)
    }

    // grammarchecks.go:772
    pub(crate) fn check_grammar_arrow_function(&mut self, node: P<Node>, file: P<SourceFile>) -> bool {
        if !is_arrow_function(node) {
            return false;
        }

        let arrow_func = node.as_arrow_function();
        let type_parameters = arrow_func.function_like_base.type_parameters.get();
        if let Some(type_parameters) = type_parameters {
            let type_param_nodes = type_parameters.nodes();
            let has_constraint = !type_param_nodes.is_empty() && type_param_nodes[0].as_type_parameter_declaration().constraint.is_some();
            if !(type_param_nodes.len() > 1 || type_parameters.has_trailing_comma() || has_constraint) {
                if tspath::file_extension_is_one_of(file.file_name(), &[tspath::EXTENSION_MTS, tspath::EXTENSION_CTS]) {
                    // TODO(danielr): should we return early here?
                    self.grammar_error_on_node(type_parameters.nodes()[0], &diagnostics::This_syntax_is_reserved_in_files_with_the_mts_or_cts_extension_Add_a_trailing_comma_or_explicit_constraint, &[]);
                }
            }
        }

        let equals_greater_than_token = arrow_func.equals_greater_than_token.unwrap();
        let arrow_full_text = &file.text()[equals_greater_than_token.pos() as usize..equals_greater_than_token.end() as usize];
        stringutil::find_line_break(arrow_full_text).is_some() && self.grammar_error_on_node(equals_greater_than_token, &diagnostics::Line_terminator_not_permitted_before_arrow, &[])
    }

    // grammarchecks.go:796
    pub(crate) fn check_grammar_index_signature_parameters(&mut self, node: P<Node>) -> bool {
        let index_signature = node.as_index_signature_declaration();
        let parameters = index_signature.function_like_base.parameters.get().unwrap();
        let param_nodes = parameters.nodes();

        if param_nodes.is_empty() {
            return self.grammar_error_on_node(node, &diagnostics::An_index_signature_must_have_exactly_one_parameter, &[]);
        }

        let parameter_node = param_nodes[0];
        let parameter = parameter_node.as_parameter_declaration();
        if param_nodes.len() != 1 {
            return self.grammar_error_on_node(parameter.name, &diagnostics::An_index_signature_must_have_exactly_one_parameter, &[]);
        }

        self.check_grammar_for_disallowed_trailing_comma(Some(parameters), &diagnostics::An_index_signature_cannot_have_a_trailing_comma);
        if let Some(dot_dot_dot_token) = parameter.dot_dot_dot_token() {
            return self.grammar_error_on_node(dot_dot_dot_token, &diagnostics::An_index_signature_cannot_have_a_rest_parameter, &[]);
        }
        if parameter_node.modifiers().is_some() {
            return self.grammar_error_on_node(parameter.name, &diagnostics::An_index_signature_parameter_cannot_have_an_accessibility_modifier, &[]);
        }
        if let Some(question_token) = parameter.question_token() {
            return self.grammar_error_on_node(question_token, &diagnostics::An_index_signature_parameter_cannot_have_a_question_mark, &[]);
        }
        if parameter.initializer().is_some() {
            return self.grammar_error_on_node(parameter.name, &diagnostics::An_index_signature_parameter_cannot_have_an_initializer, &[]);
        }
        let Some(type_node) = parameter.type_() else {
            return self.grammar_error_on_node(parameter.name, &diagnostics::An_index_signature_parameter_must_have_a_type_annotation, &[]);
        };
        let t = self.get_type_from_type_node(type_node);
        if some_type(self, t, |_, t| t.flags().intersects(TypeFlags::StringOrNumberLiteralOrUnique)) || self.is_generic_type(t) {
            return self.grammar_error_on_node(parameter.name, &diagnostics::An_index_signature_parameter_type_cannot_be_a_literal_type_or_generic_type_Consider_using_a_mapped_object_type_instead, &[]);
        }
        if !every_type(self, t, |c, t| c.is_valid_index_key_type(t)) {
            return self.grammar_error_on_node(parameter.name, &diagnostics::An_index_signature_parameter_type_must_be_string_number_symbol_or_a_template_literal_type, &[]);
        }
        if index_signature.function_like_base.type_.get().is_none() {
            return self.grammar_error_on_node(node, &diagnostics::An_index_signature_must_have_a_type_annotation, &[]);
        }
        false
    }

    // grammarchecks.go:840
    pub(crate) fn check_grammar_index_signature(&mut self, node: P<Node>) -> bool {
        // Prevent cascading error by short-circuit
        self.check_grammar_modifiers(node) || self.check_grammar_index_signature_parameters(node)
    }

    // grammarchecks.go:845
    pub(crate) fn check_grammar_for_at_least_one_type_argument(&mut self, node: P<Node>, type_arguments: Option<P<NodeList>>) -> bool {
        if let Some(type_arguments) = type_arguments {
            if type_arguments.nodes().is_empty() {
                let source_file = ast::get_source_file_of_node(node).unwrap();
                let start = type_arguments.pos() - "<".len() as i32;
                let end = scanner::skip_trivia(source_file.text(), type_arguments.end()) + ">".len() as i32;
                return self.grammar_error_at_pos(source_file.as_node(), start, end - start, &diagnostics::Type_argument_list_cannot_be_empty, &[]);
            }
        }
        false
    }

    // grammarchecks.go:855
    pub(crate) fn check_grammar_type_arguments(&mut self, node: P<Node>, type_arguments: Option<P<NodeList>>) -> bool {
        self.check_grammar_for_disallowed_trailing_comma(type_arguments, &diagnostics::Trailing_comma_not_allowed) || self.check_grammar_for_at_least_one_type_argument(node, type_arguments)
    }

    // grammarchecks.go:859
    pub(crate) fn check_grammar_tagged_template_chain(&mut self, node: P<Node>) -> bool {
        let tagged_template = node.as_tagged_template_expression();
        if tagged_template.question_dot_token.is_some() || node.flags().intersects(NodeFlags::OptionalChain) {
            return self.grammar_error_on_node(tagged_template.template, &diagnostics::Tagged_template_expressions_are_not_permitted_in_an_optional_chain, &[]);
        }
        false
    }

    // grammarchecks.go:866
    pub(crate) fn check_grammar_heritage_clause(&mut self, node: P<Node>) -> bool {
        let heritage_clause = node.as_heritage_clause();
        let types = heritage_clause.types.get();
        if self.check_grammar_for_disallowed_trailing_comma(Some(types), &diagnostics::Trailing_comma_not_allowed) {
            return true;
        }
        if types.nodes().is_empty() {
            let list_type = scanner::token_to_string(heritage_clause.token);
            // TODO(danielr): why not error on the token?
            return self.grammar_error_at_pos(node, types.pos(), 0, &diagnostics::X_0_list_cannot_be_empty, &[&list_type]);
        }

        for &node in types.nodes() {
            if self.check_grammar_expression_with_type_arguments(node) {
                return true;
            }
        }
        false
    }

    // grammarchecks.go:885
    pub(crate) fn check_grammar_expression_with_type_arguments(&mut self, node: P<Node>) -> bool {
        if is_expression_with_type_arguments(node) && node.expression().unwrap().kind() == Kind::ImportKeyword && node.type_argument_list().is_some() {
            return self.grammar_error_on_node(node, &diagnostics::This_use_of_import_is_invalid_import_calls_can_be_written_but_they_must_have_parentheses_and_cannot_have_type_arguments, &[]);
        }
        self.check_grammar_type_arguments(node, node.type_argument_list())
    }

    // grammarchecks.go:892
    pub(crate) fn check_grammar_class_declaration_heritage_clauses(&mut self, node: P<Node>, _file: P<SourceFile>) -> bool {
        let mut seen_extends_clause = false;
        let mut seen_implements_clause = false;

        let class_like_data = node.class_like_data().unwrap();

        if !self.check_grammar_modifiers(node) {
            if let Some(heritage_clauses) = class_like_data.heritage_clauses.get() {
                for &heritage_clause_node in heritage_clauses.nodes() {
                    let heritage_clause = heritage_clause_node.as_heritage_clause();
                    if heritage_clause.token == Kind::ExtendsKeyword {
                        if seen_extends_clause {
                            return self.grammar_error_on_first_token(heritage_clause_node, &diagnostics::X_extends_clause_already_seen, &[]);
                        }

                        if seen_implements_clause {
                            return self.grammar_error_on_first_token(heritage_clause_node, &diagnostics::X_extends_clause_must_precede_implements_clause, &[]);
                        }

                        let type_nodes = heritage_clause.types.get().nodes();
                        if type_nodes.len() > 1 {
                            return self.grammar_error_on_first_token(type_nodes[1], &diagnostics::Classes_can_only_extend_a_single_class, &[]);
                        }

                        seen_extends_clause = true;
                    } else {
                        if heritage_clause.token != Kind::ImplementsKeyword {
                            panic!("Unexpected token {:?}", heritage_clause.token);
                        }
                        if seen_implements_clause {
                            return self.grammar_error_on_first_token(heritage_clause_node, &diagnostics::X_implements_clause_already_seen, &[]);
                        }

                        seen_implements_clause = true;
                    }

                    // Grammar checking heritageClause inside class declaration
                    self.check_grammar_heritage_clause(heritage_clause_node);
                }
            }
        }

        false
    }

    // grammarchecks.go:935
    pub(crate) fn check_grammar_interface_declaration(&mut self, node: P<Node>) -> bool {
        if let Some(heritage_clauses) = node.as_interface_declaration().heritage_clauses {
            let mut seen_extends_clause = false;
            for &heritage_clause_node in heritage_clauses.nodes() {
                let heritage_clause = heritage_clause_node.as_heritage_clause();

                match heritage_clause.token {
                    Kind::ExtendsKeyword => {
                        if seen_extends_clause {
                            return self.grammar_error_on_first_token(heritage_clause_node, &diagnostics::X_extends_clause_already_seen, &[]);
                        }
                        seen_extends_clause = true;
                    }
                    Kind::ImplementsKeyword => {
                        return self.grammar_error_on_first_token(heritage_clause_node, &diagnostics::Interface_declaration_cannot_have_implements_clause, &[]);
                    }
                    _ => panic!("Unexpected token {:?}", heritage_clause.token),
                }

                // Grammar checking heritageClause inside class declaration
                self.check_grammar_heritage_clause(heritage_clause_node);
            }
        }

        false
    }

    // grammarchecks.go:961
    pub(crate) fn check_grammar_computed_property_name(&mut self, node: P<Node>) -> bool {
        // If node is not a computedPropertyName, just skip the grammar checking
        if node.kind() != Kind::ComputedPropertyName {
            return false;
        }

        let computed_property_name = node.as_computed_property_name();
        if computed_property_name.expression.kind() == Kind::BinaryExpression && computed_property_name.expression.as_binary_expression().operator_token.kind() == Kind::CommaToken {
            return self.grammar_error_on_node(computed_property_name.expression, &diagnostics::A_comma_expression_is_not_allowed_in_a_computed_property_name, &[]);
        }
        false
    }

    // grammarchecks.go:974
    pub(crate) fn check_grammar_for_generator(&mut self, node: P<Node>) -> bool {
        if let Some(body_data) = node.body_data() {
            if let Some(asterisk_token) = body_data.asterisk_token {
                if node.kind() != Kind::FunctionDeclaration && node.kind() != Kind::FunctionExpression && node.kind() != Kind::MethodDeclaration {
                    panic!("Unexpected node kind {:?}", node.kind());
                }
                if node.flags().intersects(NodeFlags::Ambient) {
                    return self.grammar_error_on_node(asterisk_token, &diagnostics::Generators_are_not_allowed_in_an_ambient_context, &[]);
                }
                if body_data.body.is_none() {
                    return self.grammar_error_on_node(asterisk_token, &diagnostics::An_overload_signature_cannot_be_declared_as_a_generator, &[]);
                }
            }
        }

        false
    }

    // grammarchecks.go:990
    pub(crate) fn check_grammar_for_invalid_question_mark(&mut self, postfix_token: Option<P<Node>>, message: &'static Message) -> bool {
        match postfix_token {
            Some(postfix_token) => postfix_token.kind() == Kind::QuestionToken && self.grammar_error_on_node(postfix_token, message, &[]),
            None => false,
        }
    }

    // grammarchecks.go:994
    pub(crate) fn check_grammar_for_invalid_exclamation_token(&mut self, postfix_token: Option<P<Node>>, message: &'static Message) -> bool {
        match postfix_token {
            Some(postfix_token) => postfix_token.kind() == Kind::ExclamationToken && self.grammar_error_on_node(postfix_token, message, &[]),
            None => false,
        }
    }

    // grammarchecks.go:998
    pub(crate) fn check_grammar_object_literal_expression(&mut self, node: P<Node>, in_destructuring: bool) -> bool {
        let mut seen: FxHashMap<String, DeclarationMeaning> = FxHashMap::default();

        let properties = node.as_object_literal_expression().properties.nodes();
        for &prop in properties {
            if prop.kind() == Kind::SpreadAssignment {
                let spread_assignment = prop.as_spread_assignment();
                if in_destructuring {
                    // a rest property cannot be destructured any further
                    let expression = ast::skip_parentheses(spread_assignment.expression);
                    if is_array_literal_expression(expression) || is_object_literal_expression(expression) {
                        return self.grammar_error_on_node(spread_assignment.expression, &diagnostics::A_rest_element_cannot_contain_a_binding_pattern, &[]);
                    }
                }
                continue;
            }
            let name = prop.name().unwrap();
            if name.kind() == Kind::ComputedPropertyName {
                // If the name is not a ComputedPropertyName, the grammar checking will skip it
                self.check_grammar_computed_property_name(name);
            }

            if prop.kind() == Kind::ShorthandPropertyAssignment && !in_destructuring {
                let shorthand_prop = prop.as_shorthand_property_assignment();
                if let Some(object_assignment_initializer) = shorthand_prop.object_assignment_initializer.get() {
                    // having objectAssignmentInitializer is only valid in an ObjectAssignmentPattern.
                    // Outside of destructuring, it is a syntax error.

                    // Try to grab the last node prior to the initializer,
                    // then error on the first token following (which should be the `=` token).
                    let mut last_node_before_initializer: Option<P<Node>> = None;
                    prop.for_each_child(&mut |child| {
                        if child != object_assignment_initializer {
                            last_node_before_initializer = Some(child);
                            return false;
                        }
                        true
                    });

                    self.grammar_error_on_first_token(last_node_before_initializer.unwrap(), &diagnostics::Did_you_mean_to_use_a_Colon_An_can_only_follow_a_property_name_when_the_containing_object_literal_is_part_of_a_destructuring_pattern, &[]);
                }
            }

            if name.kind() == Kind::PrivateIdentifier {
                self.grammar_error_on_node(name, &diagnostics::Private_identifiers_are_not_allowed_outside_class_bodies, &[]);
            }

            // Modifiers are never allowed on properties except for 'async' on a method declaration
            let modifiers = prop.modifier_nodes();
            if !modifiers.is_empty() {
                if ast::can_have_modifiers(prop) {
                    for &m in modifiers {
                        if ast::is_modifier(m) && (m.kind() != Kind::AsyncKeyword || prop.kind() != Kind::MethodDeclaration) {
                            self.grammar_error_on_node(m, &diagnostics::X_0_modifier_cannot_be_used_here, &[&scanner::get_text_of_node(m)]);
                        }
                    }
                } else if ast::can_have_illegal_modifiers(prop) {
                    for &m in modifiers {
                        if ast::is_modifier(m) {
                            self.grammar_error_on_node(m, &diagnostics::X_0_modifier_cannot_be_used_here, &[&scanner::get_text_of_node(m)]);
                        }
                    }
                }
            }

            // ECMA-262 11.1.5 Object Initializer
            // If previous is not undefined then throw a SyntaxError exception if any of the following conditions are true
            // a.This production is contained in strict code and IsDataDescriptor(previous) is true and
            // IsDataDescriptor(propId.descriptor) is true.
            //    b.IsDataDescriptor(previous) is true and IsAccessorDescriptor(propId.descriptor) is true.
            //    c.IsAccessorDescriptor(previous) is true and IsDataDescriptor(propId.descriptor) is true.
            //    d.IsAccessorDescriptor(previous) is true and IsAccessorDescriptor(propId.descriptor) is true
            // and either both previous and propId.descriptor have[[Get]] fields or both previous and propId.descriptor have[[Set]] fields
            let current_kind: DeclarationMeaning;
            match prop.kind() {
                Kind::ShorthandPropertyAssignment | Kind::PropertyAssignment => {
                    let common_prop = if prop.kind() == Kind::ShorthandPropertyAssignment {
                        prop.class_like_data();
                        &prop.as_shorthand_property_assignment().named_member_base
                    } else {
                        &prop.as_property_assignment().named_member_base
                    };

                    // Grammar checking for computedPropertyName and shorthandPropertyAssignment
                    self.check_grammar_for_invalid_exclamation_token(common_prop.postfix_token, &diagnostics::A_definite_assignment_assertion_is_not_permitted_in_this_context);
                    self.check_grammar_for_invalid_question_mark(common_prop.postfix_token, &diagnostics::An_object_member_cannot_be_declared_optional);

                    if name.kind() == Kind::NumericLiteral {
                        self.check_grammar_numeric_literal(name);
                    }

                    if name.kind() == Kind::BigIntLiteral {
                        self.add_error_or_suggestion(true, create_diagnostic_for_node(Some(name), &diagnostics::A_bigint_literal_cannot_be_used_as_a_property_name, &[]));
                    }

                    current_kind = DeclarationMeaning::PropertyAssignment;
                }
                Kind::MethodDeclaration => current_kind = DeclarationMeaning::Method,
                Kind::GetAccessor => current_kind = DeclarationMeaning::GetAccessor,
                Kind::SetAccessor => current_kind = DeclarationMeaning::SetAccessor,
                _ => panic!("Unexpected node kind {:?}", prop.kind()),
            }

            if !in_destructuring {
                let (effective_name, ok) = self.get_effective_property_name_for_property_name_node(name);
                if !ok {
                    continue;
                }

                let existing_kind = seen.get(&effective_name).copied().unwrap_or_default();
                if existing_kind.is_empty() {
                    seen.insert(effective_name, current_kind);
                } else if current_kind.intersects(DeclarationMeaning::Method) && existing_kind.intersects(DeclarationMeaning::Method) {
                    self.grammar_error_on_node(name, &diagnostics::Duplicate_identifier_0, &[&scanner::get_text_of_node(name)]);
                } else if current_kind.intersects(DeclarationMeaning::PropertyAssignment) && existing_kind.intersects(DeclarationMeaning::PropertyAssignment) {
                    self.grammar_error_on_node(name, &diagnostics::An_object_literal_cannot_have_multiple_properties_with_the_same_name, &[&scanner::get_text_of_node(name)]);
                } else if current_kind.intersects(DeclarationMeaning::GetOrSetAccessor) && existing_kind.intersects(DeclarationMeaning::GetOrSetAccessor) {
                    if existing_kind != DeclarationMeaning::GetOrSetAccessor && current_kind != existing_kind {
                        seen.insert(effective_name, current_kind | existing_kind);
                    } else {
                        return self.grammar_error_on_node(name, &diagnostics::An_object_literal_cannot_have_multiple_get_Slashset_accessors_with_the_same_name, &[]);
                    }
                } else {
                    return self.grammar_error_on_node(name, &diagnostics::An_object_literal_cannot_have_property_and_accessor_with_the_same_name, &[]);
                }
            }
        }

        false
    }

    // grammarchecks.go:1138
    pub(crate) fn check_grammar_jsx_element(&mut self, node: P<Node>) -> bool {
        self.check_grammar_jsx_name(node.tag_name());
        self.check_grammar_type_arguments(node, node.type_argument_list());
        let mut seen: FxHashSet<&'static str> = FxHashSet::default();
        for &attr_node in node.attributes().unwrap().properties() {
            if attr_node.kind() == Kind::JsxSpreadAttribute {
                continue;
            }
            let attr = attr_node.as_jsx_attribute();
            let name = attr.name;
            let initializer = attr.initializer;
            let text_of_name = name.text();
            if !seen.insert(text_of_name) {
                return self.grammar_error_on_node(name, &diagnostics::JSX_elements_cannot_have_multiple_attributes_with_the_same_name, &[]);
            }
            if let Some(initializer) = initializer {
                if initializer.kind() == Kind::JsxExpression && initializer.expression().is_none() {
                    return self.grammar_error_on_node(initializer, &diagnostics::JSX_attributes_must_only_be_assigned_a_non_empty_expression, &[]);
                }
            }
        }
        false
    }

    // grammarchecks.go:1162
    pub(crate) fn check_grammar_jsx_name(&mut self, node: P<Node>) -> bool {
        if is_property_access_expression(node) && is_jsx_namespaced_name(node.expression().unwrap()) {
            return self.grammar_error_on_node(node.expression().unwrap(), &diagnostics::JSX_property_access_expressions_cannot_include_JSX_namespace_names, &[]);
        }

        if is_jsx_namespaced_name(node) && self.compiler_options.get_jsx_transform_enabled() && !scanner::is_intrinsic_jsx_name(node.as_jsx_namespaced_name().namespace.text()) {
            return self.grammar_error_on_node(node, &diagnostics::React_components_cannot_include_JSX_namespace_names, &[]);
        }

        false
    }

    // grammarchecks.go:1174
    pub(crate) fn check_grammar_jsx_expression(&mut self, node: P<Node>) -> bool {
        if let Some(expression) = node.as_jsx_expression().expression {
            if ast::is_comma_sequence(expression) {
                return self.grammar_error_on_node(expression, &diagnostics::JSX_expressions_may_not_use_the_comma_operator_Did_you_mean_to_write_an_array, &[]);
            }
        }

        false
    }

    // grammarchecks.go:1182
    pub(crate) fn check_grammar_for_in_or_for_of_statement(&mut self, for_in_or_of_statement: P<Node>) -> bool {
        let as_node = for_in_or_of_statement;
        let stmt = for_in_or_of_statement.as_for_in_or_of_statement();
        if self.check_grammar_statement_in_ambient_context(as_node) {
            return true;
        }

        if for_in_or_of_statement.kind() == Kind::ForOfStatement {
            if let Some(await_modifier) = stmt.await_modifier {
                if !for_in_or_of_statement.flags().intersects(NodeFlags::AwaitContext) {
                    let source_file = ast::get_source_file_of_node(as_node).unwrap();
                    if ast::is_in_top_level_context(as_node) {
                        if !self.has_parse_diagnostics(source_file) {
                            if !ast::is_effective_external_module(source_file, &self.compiler_options) {
                                self.add_diagnostic(create_diagnostic_for_node(Some(await_modifier), &diagnostics::X_for_await_loops_are_only_allowed_at_the_top_level_of_a_file_when_that_file_is_a_module_but_this_file_has_no_imports_or_exports_Consider_adding_an_empty_export_to_make_this_file_a_module, &[]));
                            }
                            let mut fallthrough_es = false;
                            let mut fallthrough_default = false;
                            match self.module_kind {
                                ModuleKind::Node16 | ModuleKind::Node18 | ModuleKind::Node20 | ModuleKind::NodeNext => {
                                    let source_file_meta_data = self.program.get_source_file_meta_data(source_file.path());
                                    if source_file_meta_data.implied_node_format == ModuleKind::CommonJS {
                                        self.add_diagnostic(create_diagnostic_for_node(Some(await_modifier), &diagnostics::The_current_file_is_a_CommonJS_module_and_cannot_use_await_at_the_top_level, &[]));
                                    } else {
                                        fallthrough_es = true;
                                    }
                                }
                                ModuleKind::ES2022 | ModuleKind::ESNext | ModuleKind::Preserve | ModuleKind::System => fallthrough_es = true,
                                _ => fallthrough_default = true,
                            }
                            if fallthrough_es && !(self.language_version >= ScriptTarget::ES2017) {
                                fallthrough_default = true;
                            }
                            if fallthrough_default {
                                self.add_diagnostic(create_diagnostic_for_node(Some(await_modifier), &diagnostics::Top_level_for_await_loops_are_only_allowed_when_the_module_option_is_set_to_es2022_esnext_system_node16_node18_node20_nodenext_or_preserve_and_the_target_option_is_set_to_es2017_or_higher, &[]));
                            }
                        }
                    } else {
                        // use of 'for-await-of' in non-async function
                        if !self.has_parse_diagnostics(source_file) {
                            let diagnostic = create_diagnostic_for_node(Some(await_modifier), &diagnostics::X_for_await_loops_are_only_allowed_within_async_functions_and_at_the_top_levels_of_modules, &[]);
                            let containing_func = ast::get_containing_function(for_in_or_of_statement);
                            if let Some(containing_func) = containing_func {
                                if containing_func.kind() != Kind::Constructor {
                                    assert!(!ast::get_function_flags(Some(containing_func)).intersects(FunctionFlags::Async), "Enclosing function should never be an async function.");
                                    let related_info = create_diagnostic_for_node(Some(containing_func), &diagnostics::Did_you_mean_to_mark_this_function_as_async, &[]);
                                    diagnostic.add_related_info(related_info);
                                }
                            }
                            self.add_diagnostic(diagnostic);
                            return true;
                        }
                    }
                }
            }
        }

        if is_for_of_statement(as_node) && !for_in_or_of_statement.flags().intersects(NodeFlags::AwaitContext) && is_identifier(stmt.initializer) && stmt.initializer.text() == "async" {
            self.grammar_error_on_node(stmt.initializer, &diagnostics::The_left_hand_side_of_a_for_of_statement_may_not_be_async, &[]);
            return false;
        }

        if stmt.initializer.kind() == Kind::VariableDeclarationList {
            let variable_list = stmt.initializer;
            if !self.check_grammar_variable_declaration_list(variable_list) {
                let declarations = variable_list.as_variable_declaration_list().declarations;

                // declarations.length can be zero if there is an error in variable declaration in for-of or for-in
                // See http://www.ecma-international.org/ecma-262/6.0/#sec-for-in-and-for-of-statements for details
                // For example:
                //      var let = 10;
                //      for (let of [1,2,3]) {} // this is invalid ES6 syntax
                //      for (let in [1,2,3]) {} // this is invalid ES6 syntax
                // We will then want to skip on grammar checking on variableList declaration
                if declarations.nodes().is_empty() {
                    return false;
                }

                if declarations.nodes().len() > 1 {
                    let diagnostic = if for_in_or_of_statement.kind() == Kind::ForInStatement {
                        &diagnostics::Only_a_single_variable_declaration_is_allowed_in_a_for_in_statement
                    } else {
                        &diagnostics::Only_a_single_variable_declaration_is_allowed_in_a_for_of_statement
                    };
                    return self.grammar_error_on_first_token(declarations.nodes()[1], diagnostic, &[]);
                }

                let first_variable_declaration_node = declarations.nodes()[0];
                let first_variable_declaration = first_variable_declaration_node.as_variable_declaration();
                if first_variable_declaration.initializer().is_some() {
                    let diagnostic = if for_in_or_of_statement.kind() == Kind::ForInStatement {
                        &diagnostics::The_variable_declaration_of_a_for_in_statement_cannot_have_an_initializer
                    } else {
                        &diagnostics::The_variable_declaration_of_a_for_of_statement_cannot_have_an_initializer
                    };
                    return self.grammar_error_on_node(first_variable_declaration.name, diagnostic, &[]);
                }
                if first_variable_declaration.type_().is_some() {
                    let diagnostic = if for_in_or_of_statement.kind() == Kind::ForInStatement {
                        &diagnostics::The_left_hand_side_of_a_for_in_statement_cannot_use_a_type_annotation
                    } else {
                        &diagnostics::The_left_hand_side_of_a_for_of_statement_cannot_use_a_type_annotation
                    };
                    return self.grammar_error_on_node(first_variable_declaration_node, diagnostic, &[]);
                }
            }
        }

        false
    }

    // grammarchecks.go:1289
    pub(crate) fn check_grammar_accessor(&mut self, accessor: P<Node>) -> bool {
        let body = accessor.body();
        let parent_kind = accessor.parent().unwrap().kind();
        if !accessor.flags().intersects(NodeFlags::Ambient) && parent_kind != Kind::TypeLiteral && parent_kind != Kind::InterfaceDeclaration {
            if body.is_none() && !ast::has_syntactic_modifier(accessor, ModifierFlags::Abstract) {
                return self.grammar_error_at_pos(accessor, accessor.end() - 1, ";".len() as i32, &diagnostics::X_0_expected, &[&"{"]);
            }
        }
        if let Some(body) = body {
            if ast::has_syntactic_modifier(accessor, ModifierFlags::Abstract) {
                return self.grammar_error_on_node(accessor, &diagnostics::An_abstract_accessor_cannot_have_an_implementation, &[]);
            }
            if parent_kind == Kind::TypeLiteral || parent_kind == Kind::InterfaceDeclaration {
                return self.grammar_error_on_node(body, &diagnostics::An_implementation_cannot_be_declared_in_ambient_contexts, &[]);
            }
        }

        let func_data = accessor.function_like_data();
        let mut type_parameters: Option<P<NodeList>> = None;
        if let Some(func_data) = func_data {
            type_parameters = func_data.type_parameters.get();
        }

        if type_parameters.is_some() {
            return self.grammar_error_on_node(accessor.name().unwrap(), &diagnostics::An_accessor_cannot_have_type_parameters, &[]);
        }
        if !self.does_accessor_have_correct_parameter_count(accessor) {
            return self.grammar_error_on_node(
                accessor.name().unwrap(),
                if accessor.kind() == Kind::GetAccessor { &diagnostics::A_get_accessor_cannot_have_parameters } else { &diagnostics::A_set_accessor_must_have_exactly_one_parameter },
                &[],
            );
        }
        if accessor.kind() == Kind::SetAccessor {
            if func_data.unwrap().type_.get().is_some() {
                return self.grammar_error_on_node(accessor.name().unwrap(), &diagnostics::A_set_accessor_cannot_have_a_return_type_annotation, &[]);
            }

            let Some(parameter_node) = get_set_accessor_value_parameter(accessor) else {
                panic!("Return value does not match parameter count assertion.");
            };
            let parameter = parameter_node.as_parameter_declaration();
            if let Some(dot_dot_dot_token) = parameter.dot_dot_dot_token() {
                return self.grammar_error_on_node(dot_dot_dot_token, &diagnostics::A_set_accessor_cannot_have_rest_parameter, &[]);
            }
            if let Some(question_token) = parameter.question_token() {
                return self.grammar_error_on_node(question_token, &diagnostics::A_set_accessor_cannot_have_an_optional_parameter, &[]);
            }
            if parameter.initializer().is_some() {
                return self.grammar_error_on_node(accessor.name().unwrap(), &diagnostics::A_set_accessor_parameter_cannot_have_an_initializer, &[]);
            }
        }

        false
    }

    // Does the accessor have the right number of parameters?
    //
    //	A `get` accessor has no parameters or a single `this` parameter.
    //	A `set` accessor has one parameter or a `this` parameter and one more parameter.
    // grammarchecks.go:1345
    pub(crate) fn does_accessor_have_correct_parameter_count(&mut self, accessor: P<Node>) -> bool {
        // `getAccessorThisParameter` returns `nil` if the accessor's arity is incorrect,
        // even if there is a `this` parameter declared.
        self.get_accessor_this_parameter(accessor).is_some() || accessor.parameters().len() == if accessor.kind() == Kind::GetAccessor { 0 } else { 1 }
    }

    // grammarchecks.go:1351
    pub(crate) fn check_grammar_type_operator_node(&mut self, node: P<Node>) -> bool {
        let type_operator = node.as_type_operator_node();
        if type_operator.operator == Kind::UniqueKeyword {
            let inner_type = type_operator.type_;
            if inner_type.kind() != Kind::SymbolKeyword {
                return self.grammar_error_on_node(inner_type, &diagnostics::X_0_expected, &[&scanner::token_to_string(Kind::SymbolKeyword)]);
            }
            let parent = ast::walk_up_parenthesized_types(node.parent()).unwrap();
            match parent.kind() {
                Kind::VariableDeclaration => {
                    let decl = parent.as_variable_declaration();
                    if decl.name.kind() != Kind::Identifier {
                        return self.grammar_error_on_node(node, &diagnostics::X_unique_symbol_types_may_not_be_used_on_a_variable_declaration_with_a_binding_name, &[]);
                    }
                    if !crate::is_variable_declaration_in_variable_statement(parent) {
                        return self.grammar_error_on_node(node, &diagnostics::X_unique_symbol_types_are_only_allowed_on_variables_in_a_variable_statement, &[]);
                    }
                    if !parent.parent().unwrap().flags().intersects(NodeFlags::Const) {
                        return self.grammar_error_on_node(parent.as_variable_declaration().name, &diagnostics::A_variable_whose_type_is_a_unique_symbol_type_must_be_const, &[]);
                    }
                }
                Kind::PropertyDeclaration => {
                    if !ast::is_static(parent) || !has_readonly_modifier(parent) {
                        return self.grammar_error_on_node(parent.as_property_declaration().name(), &diagnostics::A_property_of_a_class_whose_type_is_a_unique_symbol_type_must_be_both_static_and_readonly, &[]);
                    }
                }
                Kind::PropertySignature => {
                    if !ast::has_syntactic_modifier(parent, ModifierFlags::Readonly) {
                        return self.grammar_error_on_node(parent.as_property_signature_declaration().name(), &diagnostics::A_property_of_an_interface_or_type_literal_whose_type_is_a_unique_symbol_type_must_be_readonly, &[]);
                    }
                }
                _ => {
                    return self.grammar_error_on_node(node, &diagnostics::X_unique_symbol_types_are_not_allowed_here, &[]);
                }
            }
        } else if type_operator.operator == Kind::ReadonlyKeyword {
            let inner_type = type_operator.type_;
            if inner_type.kind() != Kind::ArrayType && inner_type.kind() != Kind::TupleType {
                return self.grammar_error_on_first_token(node, &diagnostics::X_readonly_type_modifier_is_only_permitted_on_array_and_tuple_literal_types, &[&scanner::token_to_string(Kind::SymbolKeyword)]);
            }
        }

        false
    }

    // grammarchecks.go:1391
    pub(crate) fn check_grammar_for_invalid_dynamic_name(&mut self, node: P<Node>, message: &'static Message) -> bool {
        if !self.is_non_bindable_dynamic_name(node) {
            return false;
        }
        let expression = if is_element_access_expression(node) {
            ast::skip_parentheses(node.as_element_access_expression().argument_expression)
        } else {
            node.expression().unwrap()
        };

        if !ast::is_entity_name_expression(expression) {
            return self.grammar_error_on_node(node, message, &[]);
        }

        false
    }

    // Indicates whether a declaration name is a dynamic name that cannot be late-bound.
    // grammarchecks.go:1410
    pub(crate) fn is_non_bindable_dynamic_name(&mut self, node: P<Node>) -> bool {
        ast::is_dynamic_name(node) && !self.is_late_bindable_name(node)
    }

    // grammarchecks.go:1414
    pub(crate) fn check_grammar_method(&mut self, node: P<Node>) -> bool {
        if self.check_grammar_function_like_declaration(node) {
            return true;
        }

        let parent = node.parent().unwrap();
        if node.kind() == Kind::MethodDeclaration {
            if parent.kind() == Kind::ObjectLiteralExpression {
                // We only disallow modifier on a method declaration if it is a property of object-literal-expression
                if let Some(modifiers) = node.modifiers() {
                    let nodes = modifiers.nodes();
                    if !(nodes.len() == 1 && nodes[0].kind() == Kind::AsyncKeyword) {
                        return self.grammar_error_on_first_token(node, &diagnostics::Modifiers_cannot_appear_here, &[]);
                    }
                }

                let method_decl = node.as_method_declaration();
                if self.check_grammar_for_invalid_question_mark(method_decl.named_member_base.postfix_token, &diagnostics::An_object_member_cannot_be_declared_optional) {
                    return true;
                }
                if self.check_grammar_for_invalid_exclamation_token(method_decl.named_member_base.postfix_token, &diagnostics::A_definite_assignment_assertion_is_not_permitted_in_this_context) {
                    return true;
                }
                if node.body().is_none() {
                    return self.grammar_error_at_pos(node, node.end() - 1, ";".len() as i32, &diagnostics::X_0_expected, &[&"{"]);
                }
            }
            if self.check_grammar_for_generator(node) {
                return true;
            }
        }

        if ast::is_class_like(parent) {
            // Technically, computed properties in ambient contexts is disallowed
            // for property declarations and accessors too, not just methods.
            // However, property declarations disallow computed names in general,
            // and accessors are not allowed in ambient contexts in general,
            // so this error only really matters for methods.
            if node.flags().intersects(NodeFlags::Ambient) {
                return self.check_grammar_for_invalid_dynamic_name(node.name().unwrap(), &diagnostics::A_computed_property_name_in_an_ambient_context_must_refer_to_an_expression_whose_type_is_a_literal_type_or_a_unique_symbol_type);
            } else if node.kind() == Kind::MethodDeclaration && node.body().is_none() {
                return self.check_grammar_for_invalid_dynamic_name(node.name().unwrap(), &diagnostics::A_computed_property_name_in_a_method_overload_must_refer_to_an_expression_whose_type_is_a_literal_type_or_a_unique_symbol_type);
            }
        } else if parent.kind() == Kind::InterfaceDeclaration {
            return self.check_grammar_for_invalid_dynamic_name(node.name().unwrap(), &diagnostics::A_computed_property_name_in_an_interface_must_refer_to_an_expression_whose_type_is_a_literal_type_or_a_unique_symbol_type);
        } else if parent.kind() == Kind::TypeLiteral {
            return self.check_grammar_for_invalid_dynamic_name(node.name().unwrap(), &diagnostics::A_computed_property_name_in_a_type_literal_must_refer_to_an_expression_whose_type_is_a_literal_type_or_a_unique_symbol_type);
        }

        false
    }

    // grammarchecks.go:1462
    pub(crate) fn check_grammar_break_or_continue_statement(&mut self, node: P<Node>) -> bool {
        let target_label = node.label();
        let mut current: Option<P<Node>> = Some(node);
        while let Some(cur) = current {
            if ast::is_function_like_or_class_static_block_declaration(cur) {
                return self.grammar_error_on_node(node, &diagnostics::Jump_target_cannot_cross_function_boundary, &[]);
            }

            match cur.kind() {
                Kind::LabeledStatement => {
                    if let Some(target_label) = target_label {
                        if cur.label().unwrap().text() == target_label.text() {
                            // found matching label - verify that label usage is correct
                            // continue can only target labels that are on iteration statements
                            let is_misplaced_continue_label = node.kind() == Kind::ContinueStatement && !ast::is_iteration_statement(cur.statement(), true /*lookInLabeledStatements*/);

                            if is_misplaced_continue_label {
                                return self.grammar_error_on_node(node, &diagnostics::A_continue_statement_can_only_jump_to_a_label_of_an_enclosing_iteration_statement, &[]);
                            }

                            return false;
                        }
                    }
                }
                Kind::SwitchStatement => {
                    if node.kind() == Kind::BreakStatement && target_label.is_none() {
                        // unlabeled break within switch statement - ok
                        return false;
                    }
                }
                _ => {
                    if ast::is_iteration_statement(cur, false /*lookInLabeledStatements*/) && target_label.is_none() {
                        // unlabeled break or continue within iteration statement - ok
                        return false;
                    }
                }
            }

            current = cur.parent();
        }

        if target_label.is_some() {
            let message = if node.kind() == Kind::BreakStatement {
                &diagnostics::A_break_statement_can_only_jump_to_a_label_of_an_enclosing_statement
            } else {
                &diagnostics::A_continue_statement_can_only_jump_to_a_label_of_an_enclosing_iteration_statement
            };

            self.grammar_error_on_node(node, message, &[])
        } else {
            let message = if node.kind() == Kind::BreakStatement {
                &diagnostics::A_break_statement_can_only_be_used_within_an_enclosing_iteration_or_switch_statement
            } else {
                &diagnostics::A_continue_statement_can_only_be_used_within_an_enclosing_iteration_statement
            };
            self.grammar_error_on_node(node, message, &[])
        }
    }

    // grammarchecks.go:1518
    pub(crate) fn check_grammar_binding_element(&mut self, node: P<Node>) -> bool {
        let binding_element = node.as_binding_element();
        if binding_element.dot_dot_dot_token().is_some() {
            let elements = node.parent().unwrap().element_list();
            if Some(node) != elements.nodes().last().copied() {
                return self.grammar_error_on_node(node, &diagnostics::A_rest_element_must_be_last_in_a_destructuring_pattern, &[]);
            }
            self.check_grammar_for_disallowed_trailing_comma(Some(elements), &diagnostics::A_rest_parameter_or_binding_pattern_may_not_have_a_trailing_comma);

            if binding_element.property_name().is_some() {
                return self.grammar_error_on_node(binding_element.name.unwrap(), &diagnostics::A_rest_element_cannot_have_a_property_name, &[]);
            }
        }

        if binding_element.dot_dot_dot_token().is_some() {
            if let Some(initializer) = binding_element.initializer() {
                // Error on equals token which immediately precedes the initializer
                return self.grammar_error_at_pos(node, initializer.pos() - 1, 1, &diagnostics::A_rest_element_cannot_have_an_initializer, &[]);
            }
        }

        false
    }

    // grammarchecks.go:1539
    pub(crate) fn check_grammar_variable_declaration(&mut self, node: P<Node>) -> bool {
        let decl = node.as_variable_declaration();
        let node_flags = self.get_combined_node_flags_cached(node);
        let block_scope_kind = node_flags & NodeFlags::BlockScoped;
        if ast::is_binding_pattern(decl.name) {
            if block_scope_kind == NodeFlags::AwaitUsing {
                return self.grammar_error_on_node(node, &diagnostics::X_0_declarations_may_not_have_binding_patterns, &[&"await using"]);
            } else if block_scope_kind == NodeFlags::Using {
                return self.grammar_error_on_node(node, &diagnostics::X_0_declarations_may_not_have_binding_patterns, &[&"using"]);
            }
        }

        let parent = node.parent().unwrap();
        let grandparent = parent.parent().unwrap();
        if grandparent.kind() != Kind::ForInStatement && grandparent.kind() != Kind::ForOfStatement {
            if node_flags.intersects(NodeFlags::Ambient) {
                self.check_ambient_initializer(node);
            } else if decl.initializer().is_none() {
                if ast::is_binding_pattern(decl.name) && !ast::is_binding_pattern(parent) {
                    return self.grammar_error_on_node(node, &diagnostics::A_destructuring_declaration_must_have_an_initializer, &[]);
                }
                if block_scope_kind == NodeFlags::AwaitUsing {
                    return self.grammar_error_on_node(node, &diagnostics::X_0_declarations_must_be_initialized, &[&"await using"]);
                } else if block_scope_kind == NodeFlags::Using {
                    return self.grammar_error_on_node(node, &diagnostics::X_0_declarations_must_be_initialized, &[&"using"]);
                } else if block_scope_kind == NodeFlags::Const {
                    return self.grammar_error_on_node(node, &diagnostics::X_0_declarations_must_be_initialized, &[&"const"]);
                }
            }
        }

        if let Some(exclamation_token) = decl.exclamation_token() {
            if grandparent.kind() != Kind::VariableStatement || decl.type_().is_none() || decl.initializer().is_some() || node_flags.intersects(NodeFlags::Ambient) {
                let message = if decl.initializer().is_some() {
                    &diagnostics::Declarations_with_initializers_cannot_also_have_definite_assignment_assertions
                } else if decl.type_().is_none() {
                    &diagnostics::Declarations_with_definite_assignment_assertions_must_also_have_type_annotations
                } else {
                    &diagnostics::A_definite_assignment_assertion_is_not_permitted_in_this_context
                };
                return self.grammar_error_on_node(exclamation_token, message, &[]);
            }
        }

        if self.program.get_emit_module_format_of_file(ast::get_source_file_of_node(node).unwrap()) < ModuleKind::System
            && !grandparent.flags().intersects(NodeFlags::Ambient)
            && ast::has_syntactic_modifier(grandparent, ModifierFlags::Export)
        {
            self.check_grammar_for_es_module_marker_in_binding_name(decl.name);
        }

        // 1. LexicalDeclaration : LetOrConst BindingList ;
        // It is a Syntax Error if the BoundNames of BindingList contains "let".
        // 2. ForDeclaration: ForDeclaration : LetOrConst ForBinding
        // It is a Syntax Error if the BoundNames of ForDeclaration contains "let".

        // It is a SyntaxError if a VariableDeclaration or VariableDeclarationNoIn occurs within strict code
        // and its Identifier is eval or arguments
        !block_scope_kind.is_empty() && self.check_grammar_name_in_let_or_const_declarations(decl.name)
    }

    // grammarchecks.go:1596
    pub(crate) fn check_grammar_for_es_module_marker_in_binding_name(&mut self, name: P<Node>) -> bool {
        if is_identifier(name) {
            if name.text() == "__esModule" {
                return self.grammar_error_on_node_skipped_on_no_emit(name, &diagnostics::Identifier_expected_esModule_is_reserved_as_an_exported_marker_when_transforming_ECMAScript_modules, &[]);
            }
        } else {
            for &element in name.elements() {
                if let Some(element_name) = element.name() {
                    return self.check_grammar_for_es_module_marker_in_binding_name(element_name);
                }
            }
        }
        false
    }

    // grammarchecks.go:1611
    pub(crate) fn check_grammar_name_in_let_or_const_declarations(&mut self, name: P<Node>) -> bool {
        if name.kind() == Kind::Identifier {
            if name.text() == "let" {
                return self.grammar_error_on_node(name, &diagnostics::X_let_is_not_allowed_to_be_used_as_a_name_in_let_or_const_declarations, &[]);
            }
        } else {
            let elements = name.elements();
            for &element in elements {
                let binding_element = element.as_binding_element();
                if let Some(binding_element_name) = binding_element.name {
                    self.check_grammar_name_in_let_or_const_declarations(binding_element_name);
                }
            }
        }
        false
    }

    // grammarchecks.go:1628
    pub(crate) fn check_grammar_variable_declaration_list(&mut self, declaration_list: P<Node>) -> bool {
        let declarations = declaration_list.as_variable_declaration_list().declarations;
        if self.check_grammar_for_disallowed_trailing_comma(Some(declarations), &diagnostics::Trailing_comma_not_allowed) {
            return true;
        }

        if declarations.nodes().is_empty() {
            return self.grammar_error_at_pos(declaration_list, declarations.pos(), declarations.end() - declarations.pos(), &diagnostics::Variable_declaration_list_cannot_be_empty, &[]);
        }

        let block_scope_flags = declaration_list.flags() & NodeFlags::BlockScoped;
        if block_scope_flags == NodeFlags::Using || block_scope_flags == NodeFlags::AwaitUsing {
            let parent = declaration_list.parent().unwrap();
            if is_for_in_statement(parent) {
                return self.grammar_error_on_node(
                    declaration_list,
                    if block_scope_flags == NodeFlags::Using {
                        &diagnostics::The_left_hand_side_of_a_for_in_statement_cannot_be_a_using_declaration
                    } else {
                        &diagnostics::The_left_hand_side_of_a_for_in_statement_cannot_be_an_await_using_declaration
                    },
                    &[],
                );
            }
            if declaration_list.flags().intersects(NodeFlags::Ambient) {
                return self.grammar_error_on_node(
                    declaration_list,
                    if block_scope_flags == NodeFlags::Using {
                        &diagnostics::X_using_declarations_are_not_allowed_in_ambient_contexts
                    } else {
                        &diagnostics::X_await_using_declarations_are_not_allowed_in_ambient_contexts
                    },
                    &[],
                );
            }
            if is_variable_statement(parent) && (is_case_clause(parent.parent().unwrap()) || is_default_clause(parent.parent().unwrap())) {
                return self.grammar_error_on_node(
                    declaration_list,
                    if block_scope_flags == NodeFlags::Using {
                        &diagnostics::X_using_declarations_are_not_allowed_in_case_or_default_clauses_unless_contained_within_a_block
                    } else {
                        &diagnostics::X_await_using_declarations_are_not_allowed_in_case_or_default_clauses_unless_contained_within_a_block
                    },
                    &[],
                );
            }
        }

        if block_scope_flags == NodeFlags::AwaitUsing {
            return self.check_grammar_await_or_await_using(declaration_list);
        }

        false
    }

    // grammarchecks.go:1658
    pub(crate) fn check_grammar_await_or_await_using(&mut self, node: P<Node>) -> bool {
        // Grammar checking
        let mut has_error = false;
        let container = get_containing_function_or_class_static_block(node);
        if container.is_some_and(is_class_static_block_declaration) {
            // NOTE: We report this regardless as to whether there are parse diagnostics.
            let message = if is_await_expression(node) {
                &diagnostics::X_await_expression_cannot_be_used_inside_a_class_static_block
            } else {
                &diagnostics::X_await_using_statements_cannot_be_used_inside_a_class_static_block
            };
            self.error(Some(node), message, &[]);
            has_error = true;
        } else if !node.flags().intersects(NodeFlags::AwaitContext) {
            if ast::is_in_top_level_context(node) {
                let source_file = ast::get_source_file_of_node(node).unwrap();
                if !self.has_parse_diagnostics(source_file) {
                    let mut span = TextRange::new(0, 0);
                    let mut span_calculated = false;
                    if !ast::is_effective_external_module(source_file, &self.compiler_options) {
                        span = scanner::get_range_of_token_at_position(source_file, node.pos());
                        span_calculated = true;
                        let message = if is_await_expression(node) {
                            &diagnostics::X_await_expressions_are_only_allowed_at_the_top_level_of_a_file_when_that_file_is_a_module_but_this_file_has_no_imports_or_exports_Consider_adding_an_empty_export_to_make_this_file_a_module
                        } else {
                            &diagnostics::X_await_using_statements_are_only_allowed_at_the_top_level_of_a_file_when_that_file_is_a_module_but_this_file_has_no_imports_or_exports_Consider_adding_an_empty_export_to_make_this_file_a_module
                        };
                        let diagnostic = ast::new_diagnostic(Some(source_file), span, message, &[]);
                        self.add_diagnostic(diagnostic);
                        has_error = true;
                    }
                    let mut fallthrough_es = false;
                    let mut fallthrough_default = false;
                    match self.module_kind {
                        ModuleKind::Node16 | ModuleKind::Node18 | ModuleKind::Node20 | ModuleKind::NodeNext => {
                            let source_file_meta_data = self.program.get_source_file_meta_data(source_file.path());
                            if source_file_meta_data.implied_node_format == ModuleKind::CommonJS {
                                if !span_calculated {
                                    span = scanner::get_range_of_token_at_position(source_file, node.pos());
                                }
                                self.add_diagnostic(ast::new_diagnostic(Some(source_file), span, &diagnostics::The_current_file_is_a_CommonJS_module_and_cannot_use_await_at_the_top_level, &[]));
                                has_error = true;
                            } else {
                                fallthrough_es = true;
                            }
                        }
                        ModuleKind::ES2022 | ModuleKind::ESNext | ModuleKind::Preserve | ModuleKind::System => fallthrough_es = true,
                        _ => fallthrough_default = true,
                    }
                    if fallthrough_es && !(self.language_version >= ScriptTarget::ES2017) {
                        fallthrough_default = true;
                    }
                    if fallthrough_default {
                        if !span_calculated {
                            span = scanner::get_range_of_token_at_position(source_file, node.pos());
                        }
                        let message = if is_await_expression(node) {
                            &diagnostics::Top_level_await_expressions_are_only_allowed_when_the_module_option_is_set_to_es2022_esnext_system_node16_node18_node20_nodenext_or_preserve_and_the_target_option_is_set_to_es2017_or_higher
                        } else {
                            &diagnostics::Top_level_await_using_statements_are_only_allowed_when_the_module_option_is_set_to_es2022_esnext_system_node16_node18_node20_nodenext_or_preserve_and_the_target_option_is_set_to_es2017_or_higher
                        };
                        self.add_diagnostic(ast::new_diagnostic(Some(source_file), span, message, &[]));
                        has_error = true;
                    }
                }
            } else {
                // use of 'await' in non-async function
                let source_file = ast::get_source_file_of_node(node).unwrap();
                if !self.has_parse_diagnostics(source_file) {
                    let span = scanner::get_range_of_token_at_position(source_file, node.pos());
                    let message = if is_await_expression(node) {
                        &diagnostics::X_await_expressions_are_only_allowed_within_async_functions_and_at_the_top_levels_of_modules
                    } else {
                        &diagnostics::X_await_using_statements_are_only_allowed_within_async_functions_and_at_the_top_levels_of_modules
                    };
                    let diagnostic = ast::new_diagnostic(Some(source_file), span, message, &[]);
                    if let Some(container) = container {
                        if container.kind() != Kind::Constructor && !has_async_modifier(container) {
                            let related_info = new_diagnostic_for_node(Some(container), Some(&diagnostics::Did_you_mean_to_mark_this_function_as_async), &[]);
                            diagnostic.add_related_info(related_info);
                        }
                    }
                    self.add_diagnostic(diagnostic);
                    has_error = true;
                }
            }
        }

        if is_await_expression(node) && self.is_in_parameter_initializer_before_containing_function(node) {
            // NOTE: We report this regardless as to whether there are parse diagnostics.
            self.error(Some(node), &diagnostics::X_await_expressions_cannot_be_used_in_a_parameter_initializer, &[]);
            has_error = true;
        }

        has_error
    }

    // grammarchecks.go:1759
    pub(crate) fn check_grammar_yield_expression(&mut self, node: P<Node>) -> bool {
        let mut has_error = false;
        if !node.flags().intersects(NodeFlags::YieldContext) {
            self.grammar_error_on_first_token(node, &diagnostics::A_yield_expression_is_only_allowed_in_a_generator_body, &[]);
            has_error = true;
        }
        if self.is_in_parameter_initializer_before_containing_function(node) {
            self.error(Some(node), &diagnostics::X_yield_expressions_cannot_be_used_in_a_parameter_initializer, &[]);
            has_error = true;
        }
        has_error
    }

    // grammarchecks.go:1772
    pub(crate) fn check_grammar_for_disallowed_block_scoped_variable_statement(&mut self, node: P<Node>) -> bool {
        if !self.container_allows_block_scoped_variable(node.parent().unwrap()) {
            let block_scope_kind = self.get_combined_node_flags_cached(node.as_variable_statement().declaration_list) & NodeFlags::BlockScoped;
            if !block_scope_kind.is_empty() {
                let keyword = if block_scope_kind == NodeFlags::Let {
                    "let"
                } else if block_scope_kind == NodeFlags::Const {
                    "const"
                } else if block_scope_kind == NodeFlags::Using {
                    "using"
                } else if block_scope_kind == NodeFlags::AwaitUsing {
                    "await using"
                } else {
                    panic!("Unknown BlockScope flag")
                };
                self.error(Some(node), &diagnostics::X_0_declarations_can_only_be_declared_inside_a_block, &[&keyword]);
            }
        }

        false
    }

    // grammarchecks.go:1796
    pub(crate) fn container_allows_block_scoped_variable(&mut self, parent: P<Node>) -> bool {
        match parent.kind() {
            Kind::IfStatement | Kind::DoStatement | Kind::WhileStatement | Kind::WithStatement | Kind::ForStatement | Kind::ForInStatement | Kind::ForOfStatement => return false,
            Kind::LabeledStatement => return self.container_allows_block_scoped_variable(parent.parent().unwrap()),
            _ => {}
        }

        true
    }

    // grammarchecks.go:1813
    pub(crate) fn check_grammar_meta_property(&mut self, node: P<Node>) -> bool {
        let meta_property = node.as_meta_property();
        let node_name = meta_property.name;
        let name_text = node_name.text();

        match meta_property.keyword_token {
            Kind::NewKeyword => {
                if name_text != "target" {
                    return self.grammar_error_on_node(node_name, &diagnostics::X_0_is_not_a_valid_meta_property_for_keyword_1_Did_you_mean_2, &[&name_text, &scanner::token_to_string(meta_property.keyword_token), &"target"]);
                }
            }
            Kind::ImportKeyword => {
                if name_text != "meta" {
                    let parent = node.parent().unwrap();
                    let is_callee = is_call_expression(parent) && parent.expression() == Some(node);
                    if name_text == "defer" {
                        if !is_callee {
                            return self.grammar_error_at_pos(node, node.end(), 0, &diagnostics::X_0_expected, &[&"("]);
                        }
                    } else {
                        if is_callee {
                            return self.grammar_error_on_node(node_name, &diagnostics::X_0_is_not_a_valid_meta_property_for_keyword_import_Did_you_mean_meta_or_defer, &[&name_text]);
                        }
                        return self.grammar_error_on_node(node_name, &diagnostics::X_0_is_not_a_valid_meta_property_for_keyword_1_Did_you_mean_2, &[&name_text, &scanner::token_to_string(meta_property.keyword_token), &"meta"]);
                    }
                }
            }
            _ => {}
        }

        false
    }

    // grammarchecks.go:1841
    pub(crate) fn check_grammar_constructor_type_parameters(&mut self, node: P<Node>) -> bool {
        let range_ = node.as_constructor_declaration().function_like_base.type_parameters.get();
        if let Some(range_) = range_ {
            let pos = if range_.pos() == range_.end() {
                range_.pos()
            } else {
                scanner::skip_trivia(ast::get_source_file_of_node(node).unwrap().text(), range_.pos())
            };
            return self.grammar_error_at_pos(node, pos, range_.end() - pos, &diagnostics::Type_parameters_cannot_appear_on_a_constructor_declaration, &[]);
        }

        false
    }

    // grammarchecks.go:1856
    pub(crate) fn check_grammar_constructor_type_annotation(&mut self, node: P<Node>) -> bool {
        let t = node.as_constructor_declaration().function_like_base.type_.get();
        if let Some(t) = t {
            return self.grammar_error_on_node(t, &diagnostics::Type_annotation_cannot_appear_on_a_constructor_declaration, &[]);
        }
        false
    }

    // grammarchecks.go:1864
    pub(crate) fn check_grammar_property(&mut self, node: P<Node>) -> bool {
        let property_name = node.name().unwrap();
        let parent = node.parent().unwrap();
        if is_computed_property_name(property_name) && is_binary_expression(property_name.expression().unwrap()) && property_name.expression().unwrap().as_binary_expression().operator_token.kind() == Kind::InKeyword {
            return self.grammar_error_on_node(parent.members()[0], &diagnostics::A_mapped_type_may_not_declare_properties_or_methods, &[]);
        }
        if ast::is_class_like(parent) {
            if is_string_literal(property_name) && property_name.text() == "constructor" {
                return self.grammar_error_on_node(property_name, &diagnostics::Classes_may_not_have_a_field_named_constructor, &[]);
            }
            if self.check_grammar_for_invalid_dynamic_name(property_name, &diagnostics::A_computed_property_name_in_a_class_property_declaration_must_have_a_simple_literal_type_or_a_unique_symbol_type) {
                return true;
            }
            if ast::is_auto_accessor_property_declaration(node) && self.check_grammar_for_invalid_question_mark(node.postfix_token(), &diagnostics::An_accessor_property_cannot_be_declared_optional) {
                return true;
            }
        } else if is_interface_declaration(parent) {
            if self.check_grammar_for_invalid_dynamic_name(property_name, &diagnostics::A_computed_property_name_in_an_interface_must_refer_to_an_expression_whose_type_is_a_literal_type_or_a_unique_symbol_type) {
                return true;
            }
            if !ast::is_property_signature_declaration(node) {
                // Interfaces cannot contain property declarations
                panic!("Unexpected node kind {:?}", node.kind());
            }
            if let Some(initializer) = node.initializer() {
                return self.grammar_error_on_node(initializer, &diagnostics::An_interface_property_cannot_have_an_initializer, &[]);
            }
        } else if is_type_literal_node(parent) {
            if self.check_grammar_for_invalid_dynamic_name(node.name().unwrap(), &diagnostics::A_computed_property_name_in_a_type_literal_must_refer_to_an_expression_whose_type_is_a_literal_type_or_a_unique_symbol_type) {
                return true;
            }
            if !ast::is_property_signature_declaration(node) {
                // Type literals cannot contain property declarations
                panic!("Unexpected node kind {:?}", node.kind());
            }
            if let Some(initializer) = node.initializer() {
                return self.grammar_error_on_node(initializer, &diagnostics::A_type_literal_property_cannot_have_an_initializer, &[]);
            }
        }

        if node.flags().intersects(NodeFlags::Ambient) {
            self.check_ambient_initializer(node);
        }

        if is_property_declaration(node) {
            let prop_decl = node.as_property_declaration();
            let postfix_token = prop_decl.named_member_base.postfix_token;
            if let Some(postfix_token) = postfix_token {
                if postfix_token.kind() == Kind::ExclamationToken {
                    if prop_decl.initializer().is_some() {
                        return self.grammar_error_on_node(postfix_token, &diagnostics::Declarations_with_initializers_cannot_also_have_definite_assignment_assertions, &[]);
                    } else if prop_decl.type_().is_none() {
                        return self.grammar_error_on_node(postfix_token, &diagnostics::Declarations_with_definite_assignment_assertions_must_also_have_type_annotations, &[]);
                    } else if !ast::is_class_like(parent) || node.flags().intersects(NodeFlags::Ambient) || ast::is_static(node) || ast::has_abstract_modifier(node) {
                        return self.grammar_error_on_node(postfix_token, &diagnostics::A_definite_assignment_assertion_is_not_permitted_in_this_context, &[]);
                    }
                }
            }
        }

        false
    }

    // grammarchecks.go:1925
    pub(crate) fn check_ambient_initializer(&mut self, node: P<Node>) -> bool {
        let initializer: Option<P<Node>>;
        let type_node: Option<P<Node>>;
        match node.kind() {
            Kind::VariableDeclaration => {
                let var_decl = node.as_variable_declaration();
                initializer = var_decl.initializer();
                type_node = var_decl.type_();
            }
            Kind::PropertyDeclaration => {
                let prop_decl = node.as_property_declaration();
                initializer = prop_decl.initializer();
                type_node = prop_decl.type_();
            }
            Kind::PropertySignature => {
                let prop_sig = node.as_property_signature_declaration();
                initializer = prop_sig.initializer();
                type_node = prop_sig.type_();
            }
            _ => panic!("Unexpected node kind {:?}", node.kind()),
        }

        if let Some(initializer) = initializer {
            let is_invalid_initializer = !(is_initializer_string_or_number_literal_expression(initializer)
                || self.is_initializer_simple_literal_enum_reference(initializer)
                || initializer.kind() == Kind::TrueKeyword
                || initializer.kind() == Kind::FalseKeyword
                || is_initializer_big_int_literal_expression(initializer));
            let is_const_or_readonly = crate::is_declaration_readonly(node) || is_variable_declaration(node) && self.is_var_const_like(node);
            if is_const_or_readonly && type_node.is_none() {
                if is_invalid_initializer {
                    return self.grammar_error_on_node(initializer, &diagnostics::A_const_initializer_in_an_ambient_context_must_be_a_string_or_numeric_literal_or_literal_enum_reference, &[]);
                }
            } else {
                return self.grammar_error_on_node(initializer, &diagnostics::Initializers_are_not_allowed_in_ambient_contexts, &[]);
            }
        }

        false
    }
}

// grammarchecks.go:1960
pub(crate) fn is_initializer_string_or_number_literal_expression(expr: P<Node>) -> bool {
    ast::is_string_or_numeric_literal_like(expr)
        || expr.kind() == Kind::PrefixUnaryExpression && expr.as_prefix_unary_expression().operator == Kind::MinusToken && expr.as_prefix_unary_expression().operand.kind() == Kind::NumericLiteral
}

// grammarchecks.go:1965
pub(crate) fn is_initializer_big_int_literal_expression(expr: P<Node>) -> bool {
    if expr.kind() == Kind::BigIntLiteral {
        return true;
    }

    if expr.kind() == Kind::PrefixUnaryExpression {
        let unary_expr = expr.as_prefix_unary_expression();
        return unary_expr.operator == Kind::MinusToken && unary_expr.operand.kind() == Kind::BigIntLiteral;
    }

    false
}

impl Checker {
    // grammarchecks.go:1978
    pub(crate) fn is_initializer_simple_literal_enum_reference(&mut self, expr: P<Node>) -> bool {
        if is_property_access_expression(expr) {
            return self.check_expression_cached(expr).flags().intersects(TypeFlags::EnumLike);
        }

        if is_element_access_expression(expr) {
            let element_access = expr.as_element_access_expression();

            return is_initializer_string_or_number_literal_expression(element_access.argument_expression)
                && ast::is_entity_name_expression(element_access.expression)
                && self.check_expression_cached(expr).flags().intersects(TypeFlags::EnumLike);
        }

        false
    }

    // grammarchecks.go:1994
    pub(crate) fn check_grammar_top_level_element_for_required_declare_modifier(&mut self, node: P<Node>) -> bool {
        // A declare modifier is required for any top level .d.ts declaration except export=, export default, export as namespace
        // interfaces and imports categories:
        //
        //  DeclarationElement:
        //     ExportAssignment
        //     export_opt   InterfaceDeclaration
        //     export_opt   TypeAliasDeclaration
        //     export_opt   ImportDeclaration
        //     export_opt   ExternalImportDeclaration
        //     export_opt   AmbientDeclaration
        //
        // TODO: The spec needs to be amended to reflect this grammar.
        if node.kind() == Kind::InterfaceDeclaration
            || node.kind() == Kind::TypeAliasDeclaration
            || node.kind() == Kind::ImportDeclaration
            || node.kind() == Kind::JSImportDeclaration
            || node.kind() == Kind::ImportEqualsDeclaration
            || node.kind() == Kind::ExportDeclaration
            || node.kind() == Kind::ExportAssignment
            || node.kind() == Kind::NamespaceExportDeclaration
            || ast::has_syntactic_modifier(node, ModifierFlags::Ambient | ModifierFlags::Export | ModifierFlags::Default)
        {
            return false;
        }

        self.grammar_error_on_first_token(node, &diagnostics::Top_level_declarations_in_d_ts_files_must_start_with_either_a_declare_or_export_modifier, &[])
    }

    // grammarchecks.go:2014
    pub(crate) fn check_grammar_top_level_elements_for_required_declare_modifier(&mut self, file: P<SourceFile>) -> bool {
        for &decl in file.statements.nodes() {
            if ast::is_declaration_node(decl) || decl.kind() == Kind::VariableStatement {
                if self.check_grammar_top_level_element_for_required_declare_modifier(decl) {
                    return true;
                }
            }
        }
        false
    }

    // grammarchecks.go:2025
    pub(crate) fn check_grammar_source_file(&mut self, node: P<SourceFile>) -> bool {
        node.as_node().flags().intersects(NodeFlags::Ambient) && self.check_grammar_top_level_elements_for_required_declare_modifier(node)
    }

    // grammarchecks.go:2029
    pub(crate) fn check_grammar_statement_in_ambient_context(&mut self, node: P<Node>) -> bool {
        if node.flags().intersects(NodeFlags::Ambient) {
            // Find containing block which is either Block, ModuleBlock, SourceFile
            let links = self.node_links.get(node);
            let parent = node.parent().unwrap();
            if !links.has_reported_statement_in_ambient_context.get() && (ast::is_function_like(parent) || ast::is_accessor(parent)) {
                let reported = self.grammar_error_on_first_token(node, &diagnostics::An_implementation_cannot_be_declared_in_ambient_contexts, &[]);
                links.has_reported_statement_in_ambient_context.set(reported);
                return links.has_reported_statement_in_ambient_context.get();
            }

            // We are either parented by another statement, or some sort of block.
            // If we're in a block, we only want to really report an error once
            // to prevent noisiness.  So use a bit on the block to indicate if
            // this has already been reported, and don't report if it has.
            //
            if parent.kind() == Kind::Block || parent.kind() == Kind::ModuleBlock || parent.kind() == Kind::SourceFile {
                let links = self.node_links.get(parent);
                // Check if the containing block ever report this error
                if !links.has_reported_statement_in_ambient_context.get() {
                    let reported = self.grammar_error_on_first_token(node, &diagnostics::Statements_are_not_allowed_in_ambient_contexts, &[]);
                    links.has_reported_statement_in_ambient_context.set(reported);
                    return links.has_reported_statement_in_ambient_context.get();
                }
            } else {
                // We must be parented by a statement.  If so, there's no need
                // to report the error as our parent will have already done it.
                // debug.Assert(ast.IsStatement(node.Parent)) // !!! commented out in strada - fails if uncommented
            }
        }
        false
    }

    // grammarchecks.go:2059
    pub(crate) fn check_grammar_numeric_literal(&mut self, node: P<Node>) {
        let node_text = scanner::get_text_of_node(node);
        let numeric_literal = node.as_numeric_literal();

        // Realism (size) checking
        // We should test against `getTextOfNode(node)` rather than `node.text`, because `node.text` for large numeric literals can contain "."
        // e.g. `node.text` for numeric literal `1100000000000000000000` is `1.1e21`.
        let is_fractional = node_text.contains('.');
        let is_scientific = numeric_literal.literal_like_node_base.token_flags.get().intersects(TokenFlags::Scientific);

        // Scientific notation (e.g. 2e54 and 1e00000000010) can't be converted to bigint
        // Fractional numbers (e.g. 9000000000000000.001) are inherently imprecise anyway
        if is_fractional || is_scientific {
            return;
        }

        // Here `node` is guaranteed to be a numeric literal representing an integer.
        // We need to judge whether the integer `node` represents is <= 2 ** 53 - 1, which can be accomplished by comparing to `value` defined below because:
        // 1) when `node` represents an integer <= 2 ** 53 - 1, `node.text` is its exact string representation and thus `value` precisely represents the integer.
        // 2) otherwise, although `node.text` may be imprecise string representation, its mathematical value and consequently `value` cannot be less than 2 ** 53,
        //    thus the result of the predicate won't be affected.
        let value = jsnum::from_string(numeric_literal.literal_like_node_base.text.as_str());
        if value <= jsnum::MaxSafeInteger {
            return;
        }

        self.add_error_or_suggestion(false, create_diagnostic_for_node(Some(node), &diagnostics::Numeric_literals_with_absolute_values_equal_to_2_53_or_greater_are_too_large_to_be_represented_accurately_as_integers, &[]));
    }

    // grammarchecks.go:2087
    pub(crate) fn check_grammar_big_int_literal(&mut self, node: P<Node>) -> bool {
        let parent = node.parent().unwrap();
        let literal_type = is_literal_type_node(parent) || is_prefix_unary_expression(parent) && is_literal_type_node(parent.parent().unwrap());
        if !literal_type {
            // Don't error on BigInt literals in ambient contexts
            if !node.flags().intersects(NodeFlags::Ambient) && self.language_version < ScriptTarget::ES2020 {
                if self.grammar_error_on_node(node, &diagnostics::BigInt_literals_are_not_available_when_targeting_lower_than_ES2020, &[]) {
                    return true;
                }
            }
        }
        false
    }

    // grammarchecks.go:2100
    pub(crate) fn check_grammar_import_clause(&mut self, node: P<Node>) -> bool {
        let import_clause = node.as_import_clause();
        match import_clause.phase_modifier.get() {
            Kind::TypeKeyword => {
                if !node.flags().intersects(NodeFlags::JSDoc) && import_clause.name.is_some() && import_clause.named_bindings.is_some() {
                    return self.grammar_error_on_node(node, &diagnostics::A_type_only_import_can_specify_a_default_import_or_named_bindings_but_not_both, &[]);
                }
                if let Some(named_bindings) = import_clause.named_bindings {
                    if named_bindings.kind() == Kind::NamedImports {
                        return self.check_grammar_type_only_named_imports_or_exports(named_bindings);
                    }
                }
            }
            Kind::DeferKeyword => {
                if import_clause.name.is_some() {
                    return self.grammar_error_on_node(node, &diagnostics::Default_imports_are_not_allowed_in_a_deferred_import, &[]);
                }
                if let Some(named_bindings) = import_clause.named_bindings {
                    if named_bindings.kind() == Kind::NamedImports {
                        return self.grammar_error_on_node(node, &diagnostics::Named_imports_are_not_allowed_in_a_deferred_import, &[]);
                    }
                }
                if self.module_kind != ModuleKind::ESNext && self.module_kind != ModuleKind::Preserve {
                    return self.grammar_error_on_node(node, &diagnostics::Deferred_imports_are_only_supported_when_the_module_flag_is_set_to_esnext_or_preserve, &[]);
                }
            }
            _ => {}
        }
        false
    }

    // grammarchecks.go:2123
    pub(crate) fn check_grammar_import_attribute_values(&mut self, node: P<Node>) -> bool {
        let mut has_error = false;
        for &attribute in node.as_import_attributes().attributes.nodes() {
            let value = attribute.as_import_attribute().value;
            if is_string_literal(value) {
                continue;
            }
            has_error = true;
            self.error(Some(value), &diagnostics::Import_attribute_values_must_be_string_literal_expressions, &[]);
        }
        has_error
    }

    // grammarchecks.go:2136
    pub(crate) fn check_grammar_type_only_named_imports_or_exports(&mut self, named_bindings: P<Node>) -> bool {
        let node_list = named_bindings.element_list();
        for &specifier in node_list.nodes() {
            let specifier_is_type_only: bool;
            let message: &'static Message;
            if specifier.kind() == Kind::ImportSpecifier {
                specifier_is_type_only = specifier.is_type_only();
                message = &diagnostics::The_type_modifier_cannot_be_used_on_a_named_import_when_import_type_is_used_on_its_import_statement;
            } else {
                specifier_is_type_only = specifier.is_type_only();
                message = &diagnostics::The_type_modifier_cannot_be_used_on_a_named_export_when_export_type_is_used_on_its_export_statement;
            }

            if specifier_is_type_only {
                return self.grammar_error_on_first_token(specifier, message, &[]);
            }
        }

        false
    }

    // grammarchecks.go:2157
    pub(crate) fn check_grammar_import_call_expression(&mut self, node: P<Node>) -> bool {
        if self.compiler_options.verbatim_module_syntax == Tristate::True && self.module_kind == ModuleKind::CommonJS {
            return self.grammar_error_on_node(node, get_verbatim_module_syntax_error_message(node), &[]);
        }

        if node.expression().unwrap().kind() == Kind::MetaProperty {
            if self.module_kind != ModuleKind::ESNext && self.module_kind != ModuleKind::Preserve {
                return self.grammar_error_on_node(node, &diagnostics::Deferred_imports_are_only_supported_when_the_module_flag_is_set_to_esnext_or_preserve, &[]);
            }
        } else if self.module_kind == ModuleKind::ES2015 {
            return self.grammar_error_on_node(node, &diagnostics::Dynamic_imports_are_only_supported_when_the_module_flag_is_set_to_es2020_es2022_esnext_commonjs_amd_system_umd_node16_node18_node20_or_nodenext, &[]);
        }

        let node_as_call = node.as_call_expression();
        if node_as_call.type_arguments().is_some() {
            return self.grammar_error_on_node(node, &diagnostics::This_use_of_import_is_invalid_import_calls_can_be_written_but_they_must_have_parentheses_and_cannot_have_type_arguments, &[]);
        }

        let node_arguments = node_as_call.arguments;
        let argument_nodes = node_arguments.nodes();
        if !(ModuleKind::Node16 <= self.module_kind && self.module_kind <= ModuleKind::NodeNext) && self.module_kind != ModuleKind::ESNext && self.module_kind != ModuleKind::Preserve {
            // We are allowed trailing comma after proposal-import-assertions.
            self.check_grammar_for_disallowed_trailing_comma(Some(node_arguments), &diagnostics::Trailing_comma_not_allowed);

            if argument_nodes.len() > 1 {
                let import_attributes_argument = argument_nodes[1];
                return self.grammar_error_on_node(import_attributes_argument, &diagnostics::Dynamic_imports_only_support_a_second_argument_when_the_module_option_is_set_to_esnext_node16_node18_node20_nodenext_or_preserve, &[]);
            }
        }

        if argument_nodes.is_empty() || argument_nodes.len() > 2 {
            return self.grammar_error_on_node(node, &diagnostics::Dynamic_imports_can_only_accept_a_module_specifier_and_an_optional_set_of_attributes_as_arguments, &[]);
        }

        // see: parseArgumentOrArrayLiteralElement...we use this function which parse arguments of callExpression to parse specifier for dynamic import.
        // parseArgumentOrArrayLiteralElement allows spread element to be in an argument list which is not allowed as specifier in dynamic import.
        let spread_element = argument_nodes.iter().copied().find(|&n| is_spread_element(n));
        if let Some(spread_element) = spread_element {
            return self.grammar_error_on_node(spread_element, &diagnostics::Argument_of_dynamic_import_cannot_be_spread_element, &[]);
        }
        false
    }

    // grammarchecks.go:2200
    pub(crate) fn check_grammar_import_attributes_type(&mut self, attributes: P<Node>) -> bool {
        let members = attributes.as_type_literal_node().members;
        for &member in members.nodes() {
            if member.kind() != Kind::PropertySignature {
                return self.grammar_error_on_node(member, &diagnostics::An_import_attributes_type_may_only_contain_property_signatures, &[]);
            }
            let property_signature = member.as_property_signature_declaration();
            if let Some(modifiers) = member.modifiers() {
                for &modifier in modifiers.nodes() {
                    if modifier.kind() == Kind::ReadonlyKeyword {
                        return self.grammar_error_on_node(modifier, &diagnostics::An_import_attributes_property_cannot_have_a_readonly_modifier, &[]);
                    }
                }
            }
            let Some(type_node) = property_signature.type_() else {
                return self.grammar_error_on_node(member, &diagnostics::An_import_attributes_property_must_have_a_type_annotation, &[]);
            };
            if member.question_token().is_some() {
                return self.grammar_error_on_node(member, &diagnostics::An_import_attributes_property_cannot_be_optional, &[]);
            }
            let name = property_signature.name();
            if !(ast::is_string_literal_like(name) || is_identifier(name)) {
                return self.grammar_error_on_node(name, &diagnostics::An_import_attributes_property_must_have_a_string_literal_or_identifier_name, &[]);
            }
            if name.text() == "resolution-mode" {
                return self.grammar_error_on_node(name, &diagnostics::X_0_is_not_a_valid_key_for_an_import_attributes_type, &[&name.text()]);
            }

            if !ast::is_string_literal_like_type(type_node) {
                return self.grammar_error_on_node(type_node, &diagnostics::An_import_attributes_property_must_have_a_string_literal_type_annotation, &[]);
            }
        }
        false
    }
}
