use crate::*;

// Non-function declarations in diagnostics.go (hand-ported in types.rs):
//   type GetSymbolAccessibilityDiagnostic (diagnostics.go:10)
//   type SymbolAccessibilityDiagnostic (diagnostics.go:12)

// The `wrap*DiagnosticSelector` functions return a `GetSymbolAccessibilityDiagnostic` (`Rc<dyn Fn>`) that captures
// the selector, so the selector is `Fn + 'static` (Go passes named functions and one closure).

// diagnostics.go:18
pub(crate) fn wrap_simple_diagnostic_selector(node: P<Node>, selector: impl Fn(P<Node>, &SymbolAccessibilityResult) -> Option<&'static Message> + 'static) -> GetSymbolAccessibilityDiagnostic {
    Rc::new(move |symbol_accessibility_result: &SymbolAccessibilityResult| {
        let diagnostic_message = selector(node, symbol_accessibility_result)?;
        Some(P::new(SymbolAccessibilityDiagnostic { error_node: Some(node), diagnostic_message, type_name: ast::get_name_of_declaration(node) }))
    })
}

// diagnostics.go:32
pub(crate) fn wrap_named_diagnostic_selector(node: P<Node>, selector: impl Fn(P<Node>, &SymbolAccessibilityResult) -> &'static Message + 'static) -> GetSymbolAccessibilityDiagnostic {
    Rc::new(move |symbol_accessibility_result: &SymbolAccessibilityResult| {
        let diagnostic_message = selector(node, symbol_accessibility_result);
        let name = ast::get_name_of_declaration(node);
        Some(P::new(SymbolAccessibilityDiagnostic { error_node: name, diagnostic_message, type_name: name }))
    })
}

// diagnostics.go:47
pub(crate) fn wrap_fallback_error_diagnostic_selector(node: P<Node>, selector: impl Fn(P<Node>, &SymbolAccessibilityResult) -> &'static Message + 'static) -> GetSymbolAccessibilityDiagnostic {
    Rc::new(move |symbol_accessibility_result: &SymbolAccessibilityResult| {
        let diagnostic_message = selector(node, symbol_accessibility_result);
        let mut error_node = ast::get_name_of_declaration(node);
        if error_node.is_none() {
            error_node = Some(node);
        }
        Some(P::new(SymbolAccessibilityDiagnostic { error_node, diagnostic_message, type_name: None }))
    })
}

// diagnostics.go:64
pub(crate) fn select_diagnostic_based_on_module_name(symbol_accessibility_result: &SymbolAccessibilityResult, module_not_nameable: &'static Message, private_module: &'static Message, non_module: &'static Message) -> &'static Message {
    if !symbol_accessibility_result.error_module_name.is_empty() {
        if symbol_accessibility_result.accessibility == SymbolAccessibility::CannotBeNamed {
            return module_not_nameable;
        }
        return private_module;
    }
    non_module
}

// diagnostics.go:74
pub(crate) fn select_diagnostic_based_on_module_name_no_name_check(symbol_accessibility_result: &SymbolAccessibilityResult, private_module: &'static Message, non_module: &'static Message) -> &'static Message {
    if !symbol_accessibility_result.error_module_name.is_empty() {
        return private_module;
    }
    non_module
}

// diagnostics.go:81
pub(crate) fn create_get_symbol_accessibility_diagnostic_for_node_name(node: P<Node>) -> GetSymbolAccessibilityDiagnostic {
    if ast::is_set_accessor_declaration(node) || ast::is_get_accessor_declaration(node) {
        wrap_simple_diagnostic_selector(node, |n, r| Some(get_accessor_name_visibility_diagnostic_message(n, r)))
    } else if ast::is_method_declaration(node) || ast::is_method_signature_declaration(node) {
        wrap_simple_diagnostic_selector(node, |n, r| Some(get_method_name_visibility_diagnostic_message(n, r)))
    } else {
        create_get_symbol_accessibility_diagnostic_for_node(node)
    }
}

// diagnostics.go:91
pub(crate) fn get_accessor_name_visibility_diagnostic_message(node: P<Node>, symbol_accessibility_result: &SymbolAccessibilityResult) -> &'static Message {
    if ast::is_static(node) {
        select_diagnostic_based_on_module_name(
            symbol_accessibility_result,
            &diagnostics::Public_static_property_0_of_exported_class_has_or_is_using_name_1_from_external_module_2_but_cannot_be_named,
            &diagnostics::Public_static_property_0_of_exported_class_has_or_is_using_name_1_from_private_module_2,
            &diagnostics::Public_static_property_0_of_exported_class_has_or_is_using_private_name_1,
        )
    } else if node.parent().unwrap().kind() == Kind::ClassDeclaration {
        select_diagnostic_based_on_module_name(
            symbol_accessibility_result,
            &diagnostics::Public_property_0_of_exported_class_has_or_is_using_name_1_from_external_module_2_but_cannot_be_named,
            &diagnostics::Public_property_0_of_exported_class_has_or_is_using_name_1_from_private_module_2,
            &diagnostics::Public_property_0_of_exported_class_has_or_is_using_private_name_1,
        )
    } else {
        select_diagnostic_based_on_module_name_no_name_check(
            symbol_accessibility_result,
            &diagnostics::Property_0_of_exported_interface_has_or_is_using_name_1_from_private_module_2,
            &diagnostics::Property_0_of_exported_interface_has_or_is_using_private_name_1,
        )
    }
}

// diagnostics.go:115
pub(crate) fn get_method_name_visibility_diagnostic_message(node: P<Node>, symbol_accessibility_result: &SymbolAccessibilityResult) -> &'static Message {
    if ast::is_static(node) {
        select_diagnostic_based_on_module_name(
            symbol_accessibility_result,
            &diagnostics::Public_static_method_0_of_exported_class_has_or_is_using_name_1_from_external_module_2_but_cannot_be_named,
            &diagnostics::Public_static_method_0_of_exported_class_has_or_is_using_name_1_from_private_module_2,
            &diagnostics::Public_static_method_0_of_exported_class_has_or_is_using_private_name_1,
        )
    } else if node.parent().unwrap().kind() == Kind::ClassDeclaration {
        select_diagnostic_based_on_module_name(
            symbol_accessibility_result,
            &diagnostics::Public_method_0_of_exported_class_has_or_is_using_name_1_from_external_module_2_but_cannot_be_named,
            &diagnostics::Public_method_0_of_exported_class_has_or_is_using_name_1_from_private_module_2,
            &diagnostics::Public_method_0_of_exported_class_has_or_is_using_private_name_1,
        )
    } else {
        select_diagnostic_based_on_module_name_no_name_check(
            symbol_accessibility_result,
            &diagnostics::Method_0_of_exported_interface_has_or_is_using_name_1_from_private_module_2,
            &diagnostics::Method_0_of_exported_interface_has_or_is_using_private_name_1,
        )
    }
}

// diagnostics.go:139
pub(crate) fn create_get_symbol_accessibility_diagnostic_for_node(node: P<Node>) -> GetSymbolAccessibilityDiagnostic {
    if ast::is_variable_declaration(node) || ast::is_property_declaration(node) || ast::is_property_signature_declaration(node) || ast::is_property_access_expression(node) || ast::is_element_access_expression(node) || ast::is_binary_expression(node) || ast::is_binding_element(node) || ast::is_constructor_declaration(node) {
        wrap_simple_diagnostic_selector(node, get_variable_declaration_type_visibility_diagnostic_message)
    } else if ast::is_set_accessor_declaration(node) || ast::is_get_accessor_declaration(node) {
        wrap_named_diagnostic_selector(node, get_accessor_declaration_type_visibility_diagnostic_message)
    } else if ast::is_construct_signature_declaration(node) || ast::is_call_signature_declaration(node) || ast::is_method_declaration(node) || ast::is_method_signature_declaration(node) || ast::is_function_declaration(node) || ast::is_index_signature_declaration(node) {
        wrap_fallback_error_diagnostic_selector(node, get_return_type_visibility_diagnostic_message)
    } else if ast::is_parameter_declaration(node) {
        if ast::is_parameter_property_declaration(node, node.parent().unwrap()) && ast::has_syntactic_modifier(node.parent().unwrap(), ModifierFlags::Private) {
            return wrap_simple_diagnostic_selector(node, get_variable_declaration_type_visibility_diagnostic_message);
        }
        wrap_simple_diagnostic_selector(node, |n, r| Some(get_parameter_declaration_type_visibility_diagnostic_message(n, r)))
    } else if ast::is_type_parameter_declaration(node) {
        wrap_simple_diagnostic_selector(node, |n, r| Some(get_type_parameter_constraint_visibility_diagnostic_message(n, r)))
    } else if ast::is_expression_with_type_arguments(node) {
        // unique node selection behavior, inline closure
        Rc::new(move |_symbol_accessibility_result: &SymbolAccessibilityResult| {
            let diagnostic_message: &'static Message;
            let parent = node.parent().unwrap();
            let grand_parent = parent.parent().unwrap();
            // Heritage clause is written by user so it can always be named
            if ast::is_class_declaration(grand_parent) {
                // Class or Interface implemented/extended is inaccessible
                if ast::is_heritage_clause(parent) && parent.as_heritage_clause().token == Kind::ImplementsKeyword {
                    diagnostic_message = &diagnostics::Implements_clause_of_exported_class_0_has_or_is_using_private_name_1;
                } else if grand_parent.name().is_some() {
                    diagnostic_message = &diagnostics::X_extends_clause_of_exported_class_0_has_or_is_using_private_name_1;
                } else {
                    diagnostic_message = &diagnostics::X_extends_clause_of_exported_class_has_or_is_using_private_name_0;
                }
            } else {
                // interface is inaccessible
                diagnostic_message = &diagnostics::X_extends_clause_of_exported_interface_0_has_or_is_using_private_name_1;
            }

            Some(P::new(SymbolAccessibilityDiagnostic { diagnostic_message, error_node: Some(node), type_name: ast::get_name_of_declaration(grand_parent) }))
        })
    } else if ast::is_import_equals_declaration(node) {
        wrap_simple_diagnostic_selector(node, |_, _| Some(&diagnostics::Import_declaration_0_is_using_private_name_1))
    } else if ast::is_type_alias_declaration(node) || ast::is_js_type_alias_declaration(node) {
        // unique node selection behavior, inline closure
        Rc::new(move |symbol_accessibility_result: &SymbolAccessibilityResult| {
            let diagnostic_message = select_diagnostic_based_on_module_name_no_name_check(
                symbol_accessibility_result,
                &diagnostics::Exported_type_alias_0_has_or_is_using_private_name_1_from_module_2,
                &diagnostics::Exported_type_alias_0_has_or_is_using_private_name_1,
            );
            let error_node = node.type_node();
            let type_name = node.name();
            Some(P::new(SymbolAccessibilityDiagnostic { error_node, diagnostic_message, type_name }))
        })
    } else if ast::is_call_expression(node) {
        // JS object.defineProperty call
        // unique node selection behavior, inline closure
        Rc::new(move |symbol_accessibility_result: &SymbolAccessibilityResult| {
            let diagnostic_message = select_diagnostic_based_on_module_name(
                symbol_accessibility_result,
                &diagnostics::Exported_variable_0_has_or_is_using_name_1_from_external_module_2_but_cannot_be_named,
                &diagnostics::Exported_variable_0_has_or_is_using_name_1_from_private_module_2,
                &diagnostics::Exported_variable_0_has_or_is_using_private_name_1,
            );
            let error_node = node.arguments()[1];
            let type_name = node.arguments()[1];
            Some(P::new(SymbolAccessibilityDiagnostic { error_node: Some(error_node), diagnostic_message, type_name: Some(type_name) }))
        })
    } else {
        panic!("Attempted to set a declaration diagnostic context for unhandled node kind: {}", node.kind_string());
    }
}

// diagnostics.go:223
pub(crate) fn get_variable_declaration_type_visibility_diagnostic_message(node: P<Node>, symbol_accessibility_result: &SymbolAccessibilityResult) -> Option<&'static Message> {
    if node.kind() == Kind::VariableDeclaration || node.kind() == Kind::BindingElement {
        return Some(select_diagnostic_based_on_module_name(
            symbol_accessibility_result,
            &diagnostics::Exported_variable_0_has_or_is_using_name_1_from_external_module_2_but_cannot_be_named,
            &diagnostics::Exported_variable_0_has_or_is_using_name_1_from_private_module_2,
            &diagnostics::Exported_variable_0_has_or_is_using_private_name_1,
        ));

        // This check is to ensure we don't report error on constructor parameter property as that error would be reported during parameter emit
        // The only exception here is if the constructor was marked as private. we are not emitting the constructor parameters at all.
    } else if node.kind() == Kind::PropertyDeclaration || node.kind() == Kind::PropertyAccessExpression || node.kind() == Kind::ElementAccessExpression || node.kind() == Kind::BinaryExpression || node.kind() == Kind::PropertySignature || (node.kind() == Kind::Parameter && ast::has_syntactic_modifier(node.parent().unwrap(), ModifierFlags::Private)) {
        // TODO(jfreeman): Deal with computed properties in error reporting.
        if ast::is_static(node) {
            return Some(select_diagnostic_based_on_module_name(
                symbol_accessibility_result,
                &diagnostics::Public_static_property_0_of_exported_class_has_or_is_using_name_1_from_external_module_2_but_cannot_be_named,
                &diagnostics::Public_static_property_0_of_exported_class_has_or_is_using_name_1_from_private_module_2,
                &diagnostics::Public_static_property_0_of_exported_class_has_or_is_using_private_name_1,
            ));
        } else if node.parent().unwrap().kind() == Kind::ClassDeclaration || node.kind() == Kind::Parameter {
            return Some(select_diagnostic_based_on_module_name(
                symbol_accessibility_result,
                &diagnostics::Public_property_0_of_exported_class_has_or_is_using_name_1_from_external_module_2_but_cannot_be_named,
                &diagnostics::Public_property_0_of_exported_class_has_or_is_using_name_1_from_private_module_2,
                &diagnostics::Public_property_0_of_exported_class_has_or_is_using_private_name_1,
            ));
        } else {
            // Interfaces cannot have types that cannot be named
            return Some(select_diagnostic_based_on_module_name_no_name_check(
                symbol_accessibility_result,
                &diagnostics::Property_0_of_exported_interface_has_or_is_using_name_1_from_private_module_2,
                &diagnostics::Property_0_of_exported_interface_has_or_is_using_private_name_1,
            ));
        }
    }
    None // TODO: Audit behavior - should this panic? potentially silent error state in strada
}

// diagnostics.go:263
pub(crate) fn get_accessor_declaration_type_visibility_diagnostic_message(node: P<Node>, symbol_accessibility_result: &SymbolAccessibilityResult) -> &'static Message {
    if node.kind() == Kind::SetAccessor {
        // Getters can infer the return type from the returned expression, but setters cannot, so the
        // "_from_external_module_1_but_cannot_be_named" case cannot occur.
        if ast::is_static(node) {
            select_diagnostic_based_on_module_name_no_name_check(
                symbol_accessibility_result,
                &diagnostics::Parameter_type_of_public_static_setter_0_from_exported_class_has_or_is_using_name_1_from_private_module_2,
                &diagnostics::Parameter_type_of_public_static_setter_0_from_exported_class_has_or_is_using_private_name_1,
            )
        } else {
            select_diagnostic_based_on_module_name_no_name_check(
                symbol_accessibility_result,
                &diagnostics::Parameter_type_of_public_setter_0_from_exported_class_has_or_is_using_name_1_from_private_module_2,
                &diagnostics::Parameter_type_of_public_setter_0_from_exported_class_has_or_is_using_private_name_1,
            )
        }
    } else if ast::is_static(node) {
        select_diagnostic_based_on_module_name(
            symbol_accessibility_result,
            &diagnostics::Return_type_of_public_static_getter_0_from_exported_class_has_or_is_using_name_1_from_external_module_2_but_cannot_be_named,
            &diagnostics::Return_type_of_public_static_getter_0_from_exported_class_has_or_is_using_name_1_from_private_module_2,
            &diagnostics::Return_type_of_public_static_getter_0_from_exported_class_has_or_is_using_private_name_1,
        )
    } else {
        select_diagnostic_based_on_module_name(
            symbol_accessibility_result,
            &diagnostics::Return_type_of_public_getter_0_from_exported_class_has_or_is_using_name_1_from_external_module_2_but_cannot_be_named,
            &diagnostics::Return_type_of_public_getter_0_from_exported_class_has_or_is_using_name_1_from_private_module_2,
            &diagnostics::Return_type_of_public_getter_0_from_exported_class_has_or_is_using_private_name_1,
        )
    }
}

// diagnostics.go:299
pub(crate) fn get_return_type_visibility_diagnostic_message(node: P<Node>, symbol_accessibility_result: &SymbolAccessibilityResult) -> &'static Message {
    match node.kind() {
        Kind::ConstructSignature => {
            // Interfaces cannot have return types that cannot be named
            select_diagnostic_based_on_module_name_no_name_check(
                symbol_accessibility_result,
                &diagnostics::Return_type_of_constructor_signature_from_exported_interface_has_or_is_using_name_0_from_private_module_1,
                &diagnostics::Return_type_of_constructor_signature_from_exported_interface_has_or_is_using_private_name_0,
            )
        }
        Kind::CallSignature => {
            // Interfaces cannot have return types that cannot be named
            select_diagnostic_based_on_module_name_no_name_check(
                symbol_accessibility_result,
                &diagnostics::Return_type_of_call_signature_from_exported_interface_has_or_is_using_name_0_from_private_module_1,
                &diagnostics::Return_type_of_call_signature_from_exported_interface_has_or_is_using_private_name_0,
            )
        }
        Kind::IndexSignature => {
            // Interfaces cannot have return types that cannot be named
            select_diagnostic_based_on_module_name_no_name_check(
                symbol_accessibility_result,
                &diagnostics::Return_type_of_index_signature_from_exported_interface_has_or_is_using_name_0_from_private_module_1,
                &diagnostics::Return_type_of_index_signature_from_exported_interface_has_or_is_using_private_name_0,
            )
        }

        Kind::MethodDeclaration | Kind::MethodSignature => {
            if ast::is_static(node) {
                select_diagnostic_based_on_module_name(
                    symbol_accessibility_result,
                    &diagnostics::Return_type_of_public_static_method_from_exported_class_has_or_is_using_name_0_from_external_module_1_but_cannot_be_named,
                    &diagnostics::Return_type_of_public_static_method_from_exported_class_has_or_is_using_name_0_from_private_module_1,
                    &diagnostics::Return_type_of_public_static_method_from_exported_class_has_or_is_using_private_name_0,
                )
            } else if node.parent().unwrap().kind() == Kind::ClassDeclaration {
                select_diagnostic_based_on_module_name(
                    symbol_accessibility_result,
                    &diagnostics::Return_type_of_public_method_from_exported_class_has_or_is_using_name_0_from_external_module_1_but_cannot_be_named,
                    &diagnostics::Return_type_of_public_method_from_exported_class_has_or_is_using_name_0_from_private_module_1,
                    &diagnostics::Return_type_of_public_method_from_exported_class_has_or_is_using_private_name_0,
                )
            } else {
                // Interfaces cannot have return types that cannot be named
                select_diagnostic_based_on_module_name_no_name_check(
                    symbol_accessibility_result,
                    &diagnostics::Return_type_of_method_from_exported_interface_has_or_is_using_name_0_from_private_module_1,
                    &diagnostics::Return_type_of_method_from_exported_interface_has_or_is_using_private_name_0,
                )
            }
        }
        Kind::FunctionDeclaration => select_diagnostic_based_on_module_name(
            symbol_accessibility_result,
            &diagnostics::Return_type_of_exported_function_has_or_is_using_name_0_from_external_module_1_but_cannot_be_named,
            &diagnostics::Return_type_of_exported_function_has_or_is_using_name_0_from_private_module_1,
            &diagnostics::Return_type_of_exported_function_has_or_is_using_private_name_0,
        ),
        _ => panic!("This is unknown kind for signature: {}", node.kind_string()),
    }
}

// diagnostics.go:358
pub(crate) fn get_parameter_declaration_type_visibility_diagnostic_message(node: P<Node>, symbol_accessibility_result: &SymbolAccessibilityResult) -> &'static Message {
    let parent = node.parent().unwrap();
    match parent.kind() {
        Kind::Constructor => select_diagnostic_based_on_module_name(
            symbol_accessibility_result,
            &diagnostics::Parameter_0_of_constructor_from_exported_class_has_or_is_using_name_1_from_external_module_2_but_cannot_be_named,
            &diagnostics::Parameter_0_of_constructor_from_exported_class_has_or_is_using_name_1_from_private_module_2,
            &diagnostics::Parameter_0_of_constructor_from_exported_class_has_or_is_using_private_name_1,
        ),

        Kind::ConstructSignature | Kind::ConstructorType => {
            // Interfaces cannot have parameter types that cannot be named
            select_diagnostic_based_on_module_name_no_name_check(
                symbol_accessibility_result,
                &diagnostics::Parameter_0_of_constructor_signature_from_exported_interface_has_or_is_using_name_1_from_private_module_2,
                &diagnostics::Parameter_0_of_constructor_signature_from_exported_interface_has_or_is_using_private_name_1,
            )
        }

        Kind::CallSignature => {
            // Interfaces cannot have parameter types that cannot be named
            select_diagnostic_based_on_module_name_no_name_check(
                symbol_accessibility_result,
                &diagnostics::Parameter_0_of_call_signature_from_exported_interface_has_or_is_using_name_1_from_private_module_2,
                &diagnostics::Parameter_0_of_call_signature_from_exported_interface_has_or_is_using_private_name_1,
            )
        }

        Kind::IndexSignature => {
            // Interfaces cannot have parameter types that cannot be named
            select_diagnostic_based_on_module_name_no_name_check(
                symbol_accessibility_result,
                &diagnostics::Parameter_0_of_index_signature_from_exported_interface_has_or_is_using_name_1_from_private_module_2,
                &diagnostics::Parameter_0_of_index_signature_from_exported_interface_has_or_is_using_private_name_1,
            )
        }

        Kind::MethodDeclaration | Kind::MethodSignature => {
            if ast::is_static(parent) {
                select_diagnostic_based_on_module_name(
                    symbol_accessibility_result,
                    &diagnostics::Parameter_0_of_public_static_method_from_exported_class_has_or_is_using_name_1_from_external_module_2_but_cannot_be_named,
                    &diagnostics::Parameter_0_of_public_static_method_from_exported_class_has_or_is_using_name_1_from_private_module_2,
                    &diagnostics::Parameter_0_of_public_static_method_from_exported_class_has_or_is_using_private_name_1,
                )
            } else if parent.parent().unwrap().kind() == Kind::ClassDeclaration {
                select_diagnostic_based_on_module_name(
                    symbol_accessibility_result,
                    &diagnostics::Parameter_0_of_public_method_from_exported_class_has_or_is_using_name_1_from_external_module_2_but_cannot_be_named,
                    &diagnostics::Parameter_0_of_public_method_from_exported_class_has_or_is_using_name_1_from_private_module_2,
                    &diagnostics::Parameter_0_of_public_method_from_exported_class_has_or_is_using_private_name_1,
                )
            } else {
                // Interfaces cannot have parameter types that cannot be named
                select_diagnostic_based_on_module_name_no_name_check(
                    symbol_accessibility_result,
                    &diagnostics::Parameter_0_of_method_from_exported_interface_has_or_is_using_name_1_from_private_module_2,
                    &diagnostics::Parameter_0_of_method_from_exported_interface_has_or_is_using_private_name_1,
                )
            }
        }

        Kind::FunctionDeclaration | Kind::FunctionType | Kind::ArrowFunction | Kind::FunctionExpression => select_diagnostic_based_on_module_name(
            symbol_accessibility_result,
            &diagnostics::Parameter_0_of_exported_function_has_or_is_using_name_1_from_external_module_2_but_cannot_be_named,
            &diagnostics::Parameter_0_of_exported_function_has_or_is_using_name_1_from_private_module_2,
            &diagnostics::Parameter_0_of_exported_function_has_or_is_using_private_name_1,
        ),
        Kind::SetAccessor | Kind::GetAccessor => select_diagnostic_based_on_module_name(
            symbol_accessibility_result,
            &diagnostics::Parameter_0_of_accessor_has_or_is_using_name_1_from_external_module_2_but_cannot_be_named,
            &diagnostics::Parameter_0_of_accessor_has_or_is_using_name_1_from_private_module_2,
            &diagnostics::Parameter_0_of_accessor_has_or_is_using_private_name_1,
        ),

        _ => panic!("Unknown parent for parameter: {}", parent.kind_string()),
    }
}

// diagnostics.go:436
pub(crate) fn get_type_parameter_constraint_visibility_diagnostic_message(node: P<Node>, _symbol_accessibility_result: &SymbolAccessibilityResult) -> &'static Message {
    // Type parameter constraints are named by user so we should always be able to name it
    let parent = node.parent().unwrap();
    match parent.kind() {
        Kind::ClassDeclaration => &diagnostics::Type_parameter_0_of_exported_class_has_or_is_using_private_name_1,
        Kind::InterfaceDeclaration => &diagnostics::Type_parameter_0_of_exported_interface_has_or_is_using_private_name_1,
        Kind::MappedType => &diagnostics::Type_parameter_0_of_exported_mapped_object_type_is_using_private_name_1,
        Kind::ConstructorType | Kind::ConstructSignature => &diagnostics::Type_parameter_0_of_constructor_signature_from_exported_interface_has_or_is_using_private_name_1,
        Kind::CallSignature => &diagnostics::Type_parameter_0_of_call_signature_from_exported_interface_has_or_is_using_private_name_1,
        Kind::MethodDeclaration | Kind::MethodSignature => {
            if ast::is_static(parent) {
                &diagnostics::Type_parameter_0_of_public_static_method_from_exported_class_has_or_is_using_private_name_1
            } else if parent.parent().unwrap().kind() == Kind::ClassDeclaration {
                &diagnostics::Type_parameter_0_of_public_method_from_exported_class_has_or_is_using_private_name_1
            } else {
                &diagnostics::Type_parameter_0_of_method_from_exported_interface_has_or_is_using_private_name_1
            }
        }
        Kind::FunctionType | Kind::FunctionDeclaration => &diagnostics::Type_parameter_0_of_exported_function_has_or_is_using_private_name_1,

        Kind::InferType => &diagnostics::Extends_clause_for_inferred_type_0_has_or_is_using_private_name_1,

        Kind::TypeAliasDeclaration | Kind::JSTypeAliasDeclaration => &diagnostics::Type_parameter_0_of_exported_type_alias_has_or_is_using_private_name_1,

        _ => panic!("This is unknown parent for type parameter: {}", parent.kind_string()),
    }
}

// diagnostics.go:471
pub(crate) fn get_related_suggestion_by_declaration_kind(kind: Kind) -> Option<&'static Message> {
    match kind {
        Kind::ArrowFunction => Some(&diagnostics::Add_a_return_type_to_the_function_expression),
        Kind::FunctionExpression => Some(&diagnostics::Add_a_return_type_to_the_function_expression),
        Kind::MethodDeclaration => Some(&diagnostics::Add_a_return_type_to_the_method),
        Kind::GetAccessor => Some(&diagnostics::Add_a_return_type_to_the_get_accessor_declaration),
        Kind::SetAccessor => Some(&diagnostics::Add_a_type_to_parameter_of_the_set_accessor_declaration),
        Kind::FunctionDeclaration => Some(&diagnostics::Add_a_return_type_to_the_function_declaration),
        Kind::ConstructSignature => Some(&diagnostics::Add_a_return_type_to_the_function_declaration),
        Kind::Parameter => Some(&diagnostics::Add_a_type_annotation_to_the_parameter_0),
        Kind::VariableDeclaration => Some(&diagnostics::Add_a_type_annotation_to_the_variable_0),
        Kind::PropertyDeclaration => Some(&diagnostics::Add_a_type_annotation_to_the_property_0),
        Kind::PropertySignature => Some(&diagnostics::Add_a_type_annotation_to_the_property_0),
        Kind::ExportAssignment => Some(&diagnostics::Move_the_expression_in_default_export_to_a_variable_and_add_a_type_annotation_to_it),
        _ => None,
    }
}

// diagnostics.go:502
pub(crate) fn get_error_by_declaration_kind(kind: Kind) -> Option<&'static Message> {
    match kind {
        Kind::FunctionExpression => Some(&diagnostics::Function_must_have_an_explicit_return_type_annotation_with_isolatedDeclarations),
        Kind::FunctionDeclaration => Some(&diagnostics::Function_must_have_an_explicit_return_type_annotation_with_isolatedDeclarations),
        Kind::ArrowFunction => Some(&diagnostics::Function_must_have_an_explicit_return_type_annotation_with_isolatedDeclarations),
        Kind::MethodDeclaration => Some(&diagnostics::Method_must_have_an_explicit_return_type_annotation_with_isolatedDeclarations),
        Kind::ConstructSignature => Some(&diagnostics::Method_must_have_an_explicit_return_type_annotation_with_isolatedDeclarations),
        Kind::GetAccessor => Some(&diagnostics::At_least_one_accessor_must_have_an_explicit_type_annotation_with_isolatedDeclarations),
        Kind::SetAccessor => Some(&diagnostics::At_least_one_accessor_must_have_an_explicit_type_annotation_with_isolatedDeclarations),
        Kind::Parameter => Some(&diagnostics::Parameter_must_have_an_explicit_type_annotation_with_isolatedDeclarations),
        Kind::VariableDeclaration => Some(&diagnostics::Variable_must_have_an_explicit_type_annotation_with_isolatedDeclarations),
        Kind::PropertyDeclaration => Some(&diagnostics::Property_must_have_an_explicit_type_annotation_with_isolatedDeclarations),
        Kind::PropertySignature => Some(&diagnostics::Property_must_have_an_explicit_type_annotation_with_isolatedDeclarations),
        Kind::ComputedPropertyName => Some(&diagnostics::Computed_property_names_on_class_or_object_literals_cannot_be_inferred_with_isolatedDeclarations),
        Kind::SpreadAssignment => Some(&diagnostics::Objects_that_contain_spread_assignments_can_t_be_inferred_with_isolatedDeclarations),
        Kind::ShorthandPropertyAssignment => Some(&diagnostics::Objects_that_contain_shorthand_properties_can_t_be_inferred_with_isolatedDeclarations),
        Kind::ArrayLiteralExpression => Some(&diagnostics::Only_const_arrays_can_be_inferred_with_isolatedDeclarations),
        Kind::ExportAssignment => Some(&diagnostics::Default_exports_can_t_be_inferred_with_isolatedDeclarations),
        Kind::SpreadElement => Some(&diagnostics::Arrays_with_spread_elements_can_t_inferred_with_isolatedDeclarations),
        _ => None,
    }
}

// diagnostics.go:543
pub(crate) fn is_declaration_enough_for_errors(node: P<Node>) -> bool {
    ast::is_export_assignment(node) || ast::is_statement(node) || ast::is_variable_declaration(node) || ast::is_property_declaration(node) || ast::is_parameter_declaration(node)
}

// diagnostics.go:547
pub(crate) fn is_function_like_and_not_constructor(node: P<Node>) -> bool {
    ast::is_function_like_declaration(node) && !ast::is_constructor_declaration(node)
}

// diagnostics.go:551
pub(crate) fn find_nearest_declaration(node: P<Node>) -> Option<P<Node>> {
    let result = ast::find_ancestor(node, is_declaration_enough_for_errors)?;
    if ast::is_export_assignment(result) {
        return Some(result);
    }
    if ast::is_return_statement(result) {
        return ast::find_ancestor(result, is_function_like_and_not_constructor);
    }
    if ast::is_statement(result) {
        return None;
    }
    Some(result)
}

// diagnostics.go:568
pub(crate) fn create_entity_in_type_node_error(node: P<Node>) -> P<Diagnostic> {
    let diag = create_diagnostic_for_node(node, &diagnostics::Type_containing_private_name_0_can_t_be_used_with_isolatedDeclarations, &[&scanner::get_text_of_node(node)]);
    add_parent_declaration_related_info(node, diag);
    diag
}

// diagnostics.go:574
pub(crate) fn add_parent_declaration_related_info(node: P<Node>, diag: P<Diagnostic>) {
    let Some(parent_declaration) = find_nearest_declaration(node) else {
        return;
    };
    let mut target_str = String::new();
    if !ast::is_export_assignment(parent_declaration) && parent_declaration.name().is_some() {
        target_str = scanner::get_text_of_node(parent_declaration.name().unwrap());
    }
    diag.add_related_info(create_diagnostic_for_node(parent_declaration, get_related_suggestion_by_declaration_kind(parent_declaration.kind()).unwrap(), &[&target_str]));
}

// diagnostics.go:586
pub(crate) fn create_accessor_type_error(node: P<Node>) -> P<Diagnostic> {
    let all_declarations = ast::get_all_accessor_declarations_for_declaration(node, node.symbol().unwrap().declarations());
    let get_accessor = all_declarations.get_accessor;
    let set_accessor = all_declarations.set_accessor;
    let mut target_node = node;
    if ast::is_set_accessor_declaration(node) && !node.parameters().is_empty() {
        target_node = node.parameters()[0];
    }
    let diag = create_diagnostic_for_node(target_node, get_error_by_declaration_kind(node.kind()).unwrap(), &[]);
    if let Some(set_accessor) = set_accessor {
        diag.add_related_info(create_diagnostic_for_node(set_accessor, get_related_suggestion_by_declaration_kind(set_accessor.kind()).unwrap(), &[]));
    }
    if let Some(get_accessor) = get_accessor {
        diag.add_related_info(create_diagnostic_for_node(get_accessor, get_related_suggestion_by_declaration_kind(get_accessor.kind()).unwrap(), &[]));
    }
    diag
}

// diagnostics.go:604
pub(crate) fn create_object_literal_error(node: P<Node>) -> P<Diagnostic> {
    let diag = create_diagnostic_for_node(node, get_error_by_declaration_kind(node.kind()).unwrap(), &[]);
    add_parent_declaration_related_info(node, diag);
    diag
}

// diagnostics.go:610
pub(crate) fn create_array_literal_error(node: P<Node>) -> P<Diagnostic> {
    let diag = create_diagnostic_for_node(node, get_error_by_declaration_kind(node.kind()).unwrap(), &[]);
    add_parent_declaration_related_info(node, diag);
    diag
}

// diagnostics.go:616
pub(crate) fn create_return_type_error(node: P<Node>) -> P<Diagnostic> {
    let diag = create_diagnostic_for_node(node, get_error_by_declaration_kind(node.kind()).unwrap(), &[]);
    add_parent_declaration_related_info(node, diag);
    diag.add_related_info(create_diagnostic_for_node(node, get_related_suggestion_by_declaration_kind(node.kind()).unwrap(), &[]));
    diag
}

// diagnostics.go:623
pub(crate) fn create_binding_element_error(node: P<Node>) -> P<Diagnostic> {
    create_diagnostic_for_node(node, &diagnostics::Binding_elements_with_initializers_can_t_be_exported_directly_with_isolatedDeclarations, &[])
}

// diagnostics.go:627
pub(crate) fn create_variable_or_property_error(node: P<Node>) -> P<Diagnostic> {
    let diag = create_diagnostic_for_node(node, get_error_by_declaration_kind(node.kind()).unwrap(), &[]);
    diag.add_related_info(create_diagnostic_for_node(node, get_related_suggestion_by_declaration_kind(node.kind()).unwrap(), &[&scanner::get_text_of_node(node.name().unwrap())]));
    diag
}

// diagnostics.go:633
pub(crate) fn create_expression_error(node: P<Node>) -> P<Diagnostic> {
    create_expression_error_ex(node, None)
}

// diagnostics.go:637
pub(crate) fn create_class_expression_error(node: P<Node>) -> P<Diagnostic> {
    create_expression_error_ex(node, Some(&diagnostics::Inference_from_class_expressions_is_not_supported_with_isolatedDeclarations))
}

// diagnostics.go:641
pub(crate) fn is_parent_for_idd_iagnostic(node: P<Node>) -> FindAncestorResult {
    if ast::is_export_assignment(node) {
        return FindAncestorResult::True;
    }
    if ast::is_statement(node) {
        return FindAncestorResult::Quit;
    }
    ast::to_find_ancestor_result(!ast::is_parenthesized_expression(node) && !ast::is_assertion_expression(node))
}

// diagnostics.go:651
pub(crate) fn create_expression_error_ex(node: P<Node>, mut diagnostic_message: Option<&'static Message>) -> P<Diagnostic> {
    let Some(parent_declaration) = find_nearest_declaration(node) else {
        if diagnostic_message.is_none() {
            diagnostic_message = Some(&diagnostics::Expression_type_can_t_be_inferred_with_isolatedDeclarations);
        }
        return create_diagnostic_for_node(node, diagnostic_message.unwrap(), &[]);
    };

    let mut target_str = String::new();
    if !ast::is_export_assignment(parent_declaration) && parent_declaration.name().is_some() {
        target_str = scanner::get_text_of_node(parent_declaration.name().unwrap());
    }
    let parent = ast::find_ancestor_or_quit(node.parent(), is_parent_for_idd_iagnostic);

    if Some(parent_declaration) == parent {
        if diagnostic_message.is_none() {
            diagnostic_message = get_error_by_declaration_kind(parent_declaration.kind());
        }
        let diag = create_diagnostic_for_node(node, diagnostic_message.unwrap(), &[]);
        diag.add_related_info(create_diagnostic_for_node(parent_declaration, get_related_suggestion_by_declaration_kind(parent_declaration.kind()).unwrap(), &[&target_str]));
        return diag;
    }
    if diagnostic_message.is_none() {
        diagnostic_message = Some(&diagnostics::Expression_type_can_t_be_inferred_with_isolatedDeclarations);
    }
    let diag = create_diagnostic_for_node(node, diagnostic_message.unwrap(), &[]);
    diag.add_related_info(create_diagnostic_for_node(parent_declaration, get_related_suggestion_by_declaration_kind(parent_declaration.kind()).unwrap(), &[&target_str]));
    diag.add_related_info(create_diagnostic_for_node(node, &diagnostics::Add_satisfies_and_a_type_assertion_to_this_expression_satisfies_T_as_T_to_make_the_type_explicit, &[]));
    diag
}

// diagnostics.go:683
pub(crate) fn create_get_isolated_declaration_errors(resolver: Resolver) -> GetIsolatedDeclarationError {
    let create_parameter_error = move |c: &mut Checker, node: P<Node>| -> P<Diagnostic> {
        if ast::is_set_accessor_declaration(node.parent().unwrap()) {
            return create_accessor_type_error(node.parent().unwrap());
        }
        let add_undefined = resolver.requires_adding_implicit_undefined_unsafe(c, node, None, None); // skip checker lock - node builder will already have one
        if !add_undefined && node.initializer().is_some() {
            return create_expression_error(node.initializer().unwrap());
        }
        let mut message = get_error_by_declaration_kind(node.kind());
        if add_undefined {
            message = Some(&diagnostics::Declaration_emit_for_this_parameter_requires_implicitly_adding_undefined_to_its_type_This_is_not_supported_with_isolatedDeclarations);
        }
        let diag = create_diagnostic_for_node(node, message.unwrap(), &[]);
        let target_str = scanner::get_text_of_node(node.name().unwrap());
        diag.add_related_info(create_diagnostic_for_node(node, get_related_suggestion_by_declaration_kind(node.kind()).unwrap(), &[&target_str]));
        diag
    };

    Rc::new(move |c: &mut Checker, node: P<Node>| -> P<Diagnostic> {
        let heritage_clause = ast::find_ancestor(node, ast::is_heritage_clause);
        if heritage_clause.is_some() {
            return create_diagnostic_for_node(node, &diagnostics::Extends_clause_can_t_contain_an_expression_with_isolatedDeclarations, &[]);
        }
        if ast::is_part_of_type_node(node) || ast::is_type_query_node(node) {
            return create_entity_in_type_node_error(node);
        }
        if ast::is_entity_name(node) || ast::is_entity_name_expression(node) {
            return create_entity_in_type_node_error(node);
        }
        match node.kind() {
            Kind::GetAccessor | Kind::SetAccessor => create_accessor_type_error(node),
            Kind::ComputedPropertyName | Kind::ShorthandPropertyAssignment | Kind::SpreadAssignment => create_object_literal_error(node),
            Kind::ArrayLiteralExpression | Kind::SpreadElement => create_array_literal_error(node),
            Kind::MethodDeclaration | Kind::ConstructSignature | Kind::FunctionExpression | Kind::ArrowFunction | Kind::FunctionDeclaration => create_return_type_error(node),
            Kind::BindingElement => create_binding_element_error(node),
            Kind::PropertyDeclaration | Kind::VariableDeclaration => create_variable_or_property_error(node),
            Kind::Parameter => create_parameter_error(c, node),
            Kind::PropertyAssignment => create_expression_error(node.initializer().unwrap()),
            Kind::ClassExpression => create_class_expression_error(node),
            _ => create_expression_error(node),
        }
    })
}
