use crate::*;

// util.go:9
pub(crate) fn needs_scope_marker(result: P<Node>) -> bool {
    !ast::is_any_import_or_re_export(result) && !ast::is_export_assignment(result) && !ast::has_syntactic_modifier(result, ModifierFlags::Export) && !ast::is_ambient_module(result)
}

// util.go:13
pub(crate) fn can_have_literal_initializer(host: &'static dyn DeclarationEmitHost, node: P<Node>) -> bool {
    match node.kind {
        Kind::PropertyDeclaration | Kind::PropertySignature => host.get_effective_declaration_flags(node, ModifierFlags::Private).is_empty(),
        Kind::Parameter | Kind::VariableDeclaration => true,
        _ => false,
    }
}

// util.go:25
pub(crate) fn can_produce_diagnostics(node: P<Node>) -> bool {
    ast::is_variable_declaration(node)
        || ast::is_property_declaration(node)
        || ast::is_property_signature_declaration(node)
        || ast::is_binding_element(node)
        || ast::is_set_accessor_declaration(node)
        || ast::is_get_accessor_declaration(node)
        || ast::is_construct_signature_declaration(node)
        || ast::is_call_signature_declaration(node)
        || ast::is_method_declaration(node)
        || ast::is_method_signature_declaration(node)
        || ast::is_function_declaration(node)
        || ast::is_parameter_declaration(node)
        || ast::is_type_parameter_declaration(node)
        || ast::is_expression_with_type_arguments(node)
        || ast::is_import_equals_declaration(node)
        || ast::is_type_alias_declaration(node)
        || ast::is_js_type_alias_declaration(node)
        || ast::is_constructor_declaration(node)
        || ast::is_index_signature_declaration(node)
        || ast::is_property_access_expression(node)
        || ast::is_element_access_expression(node)
        || ast::is_binary_expression(node)
        || ast::is_call_expression(node) // || // !!! TODO: JSDoc support
                                         /* ast.IsJSDocTypeAlias(node); */
}

// util.go:52
pub(crate) fn can_reuse_modifier_nodes(nodes: &[P<Node>]) -> bool {
    for &node in nodes {
        if ast::is_modifier(node) && node.flags().intersects(NodeFlags::Reparsed) {
            return false;
        }
    }
    true
}

// util.go:61
pub(crate) fn is_declaration_and_not_visible(emit_context: P<EmitContext>, resolver: Resolver, node: P<Node>) -> bool {
    let node = emit_context.parse_node(Some(node)).unwrap();
    match node.kind {
        Kind::FunctionDeclaration | Kind::ModuleDeclaration | Kind::InterfaceDeclaration | Kind::ClassDeclaration | Kind::TypeAliasDeclaration | Kind::JSTypeAliasDeclaration | Kind::EnumDeclaration => {
            !resolver.is_declaration_visible(node)
        }
        // The following should be doing their own visibility checks based on filtering their members
        Kind::VariableDeclaration => !get_binding_name_visible(resolver, node),
        Kind::ImportEqualsDeclaration | Kind::ImportDeclaration | Kind::JSImportDeclaration | Kind::ExportDeclaration | Kind::ExportAssignment => false,
        Kind::ClassStaticBlockDeclaration => true,
        _ => false,
    }
}

// util.go:87
pub(crate) fn get_binding_name_visible(resolver: Resolver, elem: P<Node>) -> bool {
    if ast::is_omitted_expression(elem) {
        return false;
    }
    // TODO: parseArrayBindingElement _never_ parses out an OmittedExpression anymore, instead producing a nameless binding element
    // Audit if OmittedExpression should be removed
    let Some(name) = elem.name() else {
        return false;
    };
    if ast::is_binding_pattern(name) {
        // If any child binding pattern element has been marked visible (usually by collect linked aliases), then this is visible
        for &elem in name.elements() {
            if get_binding_name_visible(resolver, elem) {
                return true;
            }
        }
        false
    } else {
        resolver.is_declaration_visible(elem)
    }
}

// util.go:109
pub(crate) fn is_enclosing_declaration(node: P<Node>) -> bool {
    ast::is_source_file(node)
        || ast::is_type_alias_declaration(node)
        || ast::is_js_type_alias_declaration(node)
        || ast::is_module_declaration(node)
        || ast::is_class_declaration(node)
        || ast::is_interface_declaration(node)
        || ast::is_function_like(node)
        || ast::is_index_signature_declaration(node)
        || ast::is_mapped_type_node(node)
        || ast::is_variable_declaration(node)
}

// util.go:122
pub(crate) fn is_always_type(node: P<Node>) -> bool {
    if node.kind == Kind::InterfaceDeclaration {
        return true;
    }
    false
}

// util.go:129
pub(crate) fn mask_modifier_flags(node: P<Node>, modifier_mask: ModifierFlags, modifier_additions: ModifierFlags) -> ModifierFlags {
    let mut flags = (ast::get_combined_modifier_flags(node) & modifier_mask) | modifier_additions;
    if flags.intersects(ModifierFlags::Default) && !flags.intersects(ModifierFlags::Export) {
        // A non-exported default is a nonsequitor - we usually try to remove all export modifiers
        // from statements in ambient declarations; but a default export must retain its export modifier to be syntactically valid
        flags ^= ModifierFlags::Export;
    }
    if flags.intersects(ModifierFlags::Default) && flags.intersects(ModifierFlags::Ambient) {
        flags ^= ModifierFlags::Ambient; // `declare` is never required alongside `default` (and would be an error if printed)
    }
    flags
}

// util.go:142
pub(crate) fn unwrap_parenthesized_expression(o: P<Node>) -> Option<P<Node>> {
    let mut o = o;
    while o.kind == Kind::ParenthesizedExpression {
        o = o.expression().unwrap();
    }
    Some(o)
}

// util.go:149
pub(crate) fn is_private_method_type_parameter(host: &'static dyn DeclarationEmitHost, node: P<Node>) -> bool {
    let parent = node.parent().unwrap();
    parent.kind == Kind::MethodDeclaration && !host.get_effective_declaration_flags(parent, ModifierFlags::Private).is_empty()
}

// util.go:153
// Returns true if expando properties should be emitted for this function.
// Properties are emitted if any overload in the symbol has a body (implementation).
pub(crate) fn should_emit_function_properties(input: P<Node>) -> bool {
    if input.body().is_some() {
        return true;
    }
    let symbol = input.symbol().unwrap();
    !symbol.declarations().iter().all(|&decl| !ast::is_function_declaration(decl) || decl.body().is_none())
}

// util.go:164
pub(crate) fn get_effective_base_type_node(node: P<Node>) -> Option<P<Node>> {
    let base_type = ast::get_class_extends_heritage_element(node);
    // !!! TODO: JSDoc support
    // if (baseType && isInJSFile(node)) {
    //     // Prefer an @augments tag because it may have type parameters.
    //     const tag = getJSDocAugmentsTag(node);
    //     if (tag) {
    //         return tag.class;
    //     }
    // }
    base_type
}

// util.go:177
pub(crate) fn is_scope_marker(node: P<Node>) -> bool {
    ast::is_export_assignment(node) || ast::is_export_declaration(node)
}

// util.go:181
pub(crate) fn has_scope_marker(statements: Option<P<NodeList>>) -> bool {
    let Some(statements) = statements else {
        return false;
    };
    statements.nodes().iter().any(|&n| is_scope_marker(n))
}
