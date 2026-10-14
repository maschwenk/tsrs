use crate::*;
use tsrs_ast as ast;

impl Checker {
    pub(crate) fn is_declaration_visible(&mut self, node: Option<P<Node>>) -> bool {
        let Some(node) = node else {
            return false;
        };
        if !ast::is_parse_tree_node(node) {
            return false;
        }

        let links = self.get_emit_resolver_links().declaration_links.get(node);
        if links.is_visible.get() == Tristate::Unknown {
            if self.determine_if_declaration_is_visible(node) {
                links.is_visible.set(Tristate::True);
            } else {
                links.is_visible.set(Tristate::False);
            }
        }
        links.is_visible.get() == Tristate::True
    }

    pub(crate) fn determine_if_declaration_is_visible(&mut self, node: P<Node>) -> bool {
        match node.kind() {
            Kind::JSDocCallbackTag
            // | Kind::JSDocEnumTag // !!! TODO: JSDoc @enum support?
            | Kind::JSDocTypedefTag => {
                // Top-level jsdoc type aliases are considered exported
                // First parent is comment node, second is hosting declaration or token; we only care about those tokens or declarations whose parent is a source file
                match node.parent().and_then(|p| p.parent()).and_then(|p| p.parent()) {
                    Some(p) => ast::is_source_file(p),
                    None => false,
                }
            }
            Kind::BindingElement => self.is_declaration_visible(node.parent().unwrap().parent()),
            Kind::VariableDeclaration
            | Kind::ModuleDeclaration
            | Kind::ClassDeclaration
            | Kind::InterfaceDeclaration
            | Kind::TypeAliasDeclaration
            | Kind::JSTypeAliasDeclaration
            | Kind::FunctionDeclaration
            | Kind::EnumDeclaration
            | Kind::ImportEqualsDeclaration => {
                if ast::is_variable_declaration(node) {
                    let name = node.name().unwrap();
                    if ast::is_binding_pattern(name) && name.elements().is_empty() {
                        // If the binding pattern is empty, this variable declaration is not visible
                        return false;
                    }
                    // falls through
                }
                // External module augmentation is always visible
                // A @typedef at top-level in an external module is always visible
                if ast::is_external_module_augmentation(node) || ast::is_implicitly_exported_jsdoc_declaration(node) {
                    return true;
                }
                let parent = ast::get_declaration_container(node).unwrap();
                // If the node is not exported or it is not ambient module element (except import declaration)
                if !(self.get_combined_modifier_flags_cached(node).intersects(ModifierFlags::Export) || node.kind() != Kind::ImportEqualsDeclaration && parent.kind() != Kind::SourceFile && parent.flags().intersects(NodeFlags::Ambient))
                {
                    return ast::is_global_source_file(parent);
                }
                // Exported members/ambient module elements (exception import declaration) are visible if parent is visible
                self.is_declaration_visible(Some(parent))
            }

            Kind::PropertyDeclaration
            | Kind::PropertySignature
            | Kind::GetAccessor
            | Kind::SetAccessor
            | Kind::MethodDeclaration
            | Kind::MethodSignature => {
                if self.get_effective_declaration_flags(node, ModifierFlags::Private | ModifierFlags::Protected) != ModifierFlags::None {
                    // Private/protected properties/methods are not visible
                    return false;
                }
                // Public properties/methods are visible if its parents are visible, so:
                self.is_declaration_visible(node.parent())
            }

            Kind::Constructor
            | Kind::ConstructSignature
            | Kind::CallSignature
            | Kind::IndexSignature
            | Kind::Parameter
            | Kind::ModuleBlock
            | Kind::FunctionType
            | Kind::ConstructorType
            | Kind::TypeLiteral
            | Kind::TypeReference
            | Kind::ArrayType
            | Kind::TupleType
            | Kind::UnionType
            | Kind::IntersectionType
            | Kind::ParenthesizedType
            | Kind::NamedTupleMember => self.is_declaration_visible(node.parent()),

            // Default binding, import specifier and namespace import is visible
            // only on demand so by default it is not visible
            Kind::ImportClause | Kind::NamespaceImport | Kind::ImportSpecifier => false,

            // Type parameters are always visible
            Kind::TypeParameter => true,
            // Source file and namespace export are always visible
            Kind::SourceFile | Kind::NamespaceExportDeclaration => true,

            // Export assignments do not create name bindings outside the module
            Kind::ExportAssignment => false,

            // An `export {X}` (without a module specifier) is itself a visible re-export of
            // the named binding; it contributes to the symbol's external visibility.
            Kind::ExportSpecifier => {
                let export_decl = node.parent().unwrap().parent().unwrap();
                if ast::is_export_declaration(export_decl) && export_decl.as_export_declaration().module_specifier.is_none() {
                    return self.is_declaration_visible(export_decl.parent());
                }
                false
            }

            _ => false,
        }
    }

    pub(crate) fn is_entity_name_visible(
        &mut self,
        entity_name: P<Node>,
        enclosing_declaration: Option<P<Node>>,
        should_compute_alias_to_make_visible: bool,
    ) -> SymbolAccessibilityResult {
        if !ast::is_parse_tree_node(entity_name) {
            return SymbolAccessibilityResult {
                accessibility: SymbolAccessibility::NotAccessible,
                ..Default::default()
            };
        }

        let meaning = get_meaning_of_entity_name_reference(entity_name);
        let first_identifier = ast::get_first_identifier(entity_name);

        let symbol = self.resolve_name(
            enclosing_declaration,
            first_identifier.text(),
            meaning,
            None,
            false,
            false,
        );

        if let Some(symbol) = symbol {
            if symbol.flags().intersects(SymbolFlags::TypeParameter)
                && meaning.intersects(SymbolFlags::Type)
            {
                return SymbolAccessibilityResult {
                    accessibility: SymbolAccessibility::Accessible,
                    ..Default::default()
                };
            }
        }

        if symbol.is_none() && ast::is_this_identifier(first_identifier) {
            let this_container = self
                .get_this_container(first_identifier, false, false)
                .unwrap();
            let sym = self.get_symbol_of_declaration(this_container);
            if self
                .is_symbol_accessible(sym, enclosing_declaration, meaning, false)
                .accessibility
                == SymbolAccessibility::Accessible
            {
                return SymbolAccessibilityResult {
                    accessibility: SymbolAccessibility::Accessible,
                    ..Default::default()
                };
            }
        }

        let Some(symbol) = symbol else {
            return SymbolAccessibilityResult {
                accessibility: SymbolAccessibility::NotResolved,
                error_symbol_name: first_identifier.text().to_string(),
                error_node: Some(first_identifier),
                ..Default::default()
            };
        };

        let visible = self.has_visible_declarations(symbol, should_compute_alias_to_make_visible);
        if let Some(visible) = visible {
            return (*visible).clone();
        }

        SymbolAccessibilityResult {
            accessibility: SymbolAccessibility::NotAccessible,
            error_symbol_name: first_identifier.text().to_string(),
            error_node: Some(first_identifier),
            ..Default::default()
        }
    }

    pub(crate) fn has_visible_declarations(
        &mut self,
        symbol: P<Symbol>,
        should_compute_alias_to_make_visible: bool,
    ) -> Option<P<SymbolAccessibilityResult>> {
        // Go collects into a map[NodeId]*Node and returns its values in (random) map order; this keeps
        // first-insertion order, overwriting the value on a repeated key like the map assignment does.
        let mut aliases_to_make_visible_set: Option<Vec<(NodeId, P<Node>)>> = None;

        let resolver_links = self.get_emit_resolver_links();
        {
            let mut add_visible_alias: Box<dyn FnMut(P<Node>, P<Node>) + '_> =
                if should_compute_alias_to_make_visible {
                    let aliases_to_make_visible_set = &mut aliases_to_make_visible_set;
                    Box::new(move |declaration: P<Node>, aliasing_statement: P<Node>| {
                        resolver_links
                            .declaration_links
                            .get(declaration)
                            .is_visible
                            .set(Tristate::True);
                        let set = aliases_to_make_visible_set.get_or_insert_with(Vec::new);
                        let id = ast::get_node_id(declaration);
                        match set.iter_mut().find(|(k, _)| *k == id) {
                            Some(entry) => entry.1 = aliasing_statement,
                            None => set.push((id, aliasing_statement)),
                        }
                    })
                } else {
                    Box::new(noop_add_visible_alias)
                };

            let declarations = symbol.declarations().to_vec();
            for declaration in declarations {
                if ast::is_identifier(declaration) {
                    continue;
                }
                if !self.is_declaration_visible(Some(declaration)) {
                    // Mark the unexported alias as visible if its parent is visible
                    // because these kind of aliases can be used to name types in declaration file
                    let any_import_syntax = get_any_import_syntax(declaration);
                    if let Some(any_import_syntax) = any_import_syntax {
                        if !ast::has_syntactic_modifier(any_import_syntax, ModifierFlags::Export) && // import clause without export
                            self.is_declaration_visible(any_import_syntax.parent())
                        {
                            add_visible_alias(declaration, any_import_syntax);
                            continue;
                        }
                    }
                    if ast::is_variable_declaration(declaration) {
                        let variable_statement = declaration.parent().unwrap().parent().unwrap();
                        if ast::is_variable_statement(variable_statement)
                            && !ast::has_syntactic_modifier(variable_statement, ModifierFlags::Export) // unexported variable statement
                            && self.is_declaration_visible(variable_statement.parent())
                        {
                            add_visible_alias(declaration, variable_statement);
                            continue;
                        }
                    }
                    if ast::is_late_visibility_painted_statement(declaration) // unexported top-level statement
                        && !ast::has_syntactic_modifier(declaration, ModifierFlags::Export)
                        && self.is_declaration_visible(declaration.parent())
                    {
                        add_visible_alias(declaration, declaration);
                        continue;
                    }
                    if ast::is_binding_element(declaration) {
                        if symbol.flags().intersects(SymbolFlags::Alias)
                            && ast::is_in_js_file(declaration)
                        {
                            // exported import-like top-level JS require statement
                            let p2 = declaration.parent().and_then(|p| p.parent());
                            if let Some(p2) = p2 {
                                if ast::is_variable_declaration(p2) {
                                    let p4 = p2.parent().unwrap().parent();
                                    if let Some(p4) = p4 {
                                        if ast::is_variable_statement(p4)
                                            && !ast::has_syntactic_modifier(
                                                p4,
                                                ModifierFlags::Export,
                                            )
                                        {
                                            // check if the thing containing the variable statement is visible (ie, the file)
                                            let p5 = p4.parent();
                                            if p5.is_some() && self.is_declaration_visible(p5) {
                                                add_visible_alias(declaration, p4);
                                                continue;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if symbol.flags().intersects(SymbolFlags::BlockScopedVariable) {
                            let root_declaration =
                                ast::walk_up_binding_elements_and_patterns(declaration);
                            if ast::is_parameter_declaration(root_declaration) {
                                return None;
                            }
                            let variable_statement =
                                root_declaration.parent().unwrap().parent().unwrap();
                            if !ast::is_variable_statement(variable_statement) {
                                return None;
                            }
                            if ast::has_syntactic_modifier(
                                variable_statement,
                                ModifierFlags::Export,
                            ) {
                                continue; // no alias to add, already exported
                            }
                            if !self.is_declaration_visible(variable_statement.parent()) {
                                return None; // not visible
                            }
                            add_visible_alias(declaration, variable_statement);
                            continue;
                        }
                    }

                    // Declaration is not visible
                    return None;
                }
            }
        }

        Some(P::new_scratch(SymbolAccessibilityResult {
            accessibility: SymbolAccessibility::Accessible,
            aliases_to_make_visible: aliases_to_make_visible_set
                .unwrap_or_default()
                .into_iter()
                .map(|(_, n)| n)
                .collect(),
            ..Default::default()
        }))
    }

    pub(crate) fn requires_adding_implicit_undefined_for_declaration(
        &mut self,
        declaration: P<Node>,
        symbol: Option<P<Symbol>>,
        enclosing_declaration: Option<P<Node>>,
    ) -> bool {
        if !ast::is_parse_tree_node(declaration) {
            return false;
        }
        match declaration.kind() {
            Kind::PropertyDeclaration | Kind::PropertySignature | Kind::JSDocPropertyTag => {
                let symbol = match symbol {
                    Some(s) => s,
                    None => self.get_symbol_of_declaration(declaration).unwrap(),
                };
                let t = self.get_type_of_symbol(symbol);
                let _ = self.mapped_symbol_links.has(symbol);
                symbol.flags().intersects(SymbolFlags::Property)
                    && symbol.flags().intersects(SymbolFlags::Optional)
                    && is_optional_declaration(declaration)
                    && self.reverse_mapped_symbol_links.has(symbol)
                    && self
                        .reverse_mapped_symbol_links
                        .get(symbol)
                        .mapped_type
                        .get()
                        .is_some()
                    && contains_non_missing_undefined_type(self, t)
            }
            Kind::Parameter | Kind::JSDocParameterTag => {
                self.requires_adding_implicit_undefined_worker(declaration, enclosing_declaration)
            }
            _ => panic!("Node cannot possibly require adding undefined"),
        }
    }

    pub(crate) fn requires_adding_implicit_undefined_worker(
        &mut self,
        parameter: P<Node>,
        enclosing_declaration: Option<P<Node>>,
    ) -> bool {
        (self.is_required_initialized_parameter(parameter, enclosing_declaration)
            || self.is_optional_uninitialized_parameter_property(parameter))
            && !self.declared_parameter_type_contains_undefined(parameter)
    }

    pub(crate) fn declared_parameter_type_contains_undefined(
        &mut self,
        parameter: P<Node>,
    ) -> bool {
        // typeNode := getNonlocalEffectiveTypeAnnotationNode(parameter); // !!! JSDoc Support
        let Some(type_node) = parameter.type_node() else {
            return false;
        };
        let t = self.get_type_from_type_node(type_node);
        // allow error type here to avoid confusing errors that the annotation has to contain undefined when it does in cases like this:
        //
        // export function fn(x?: Unresolved | undefined): void {}
        self.is_error_type(t) || self.contains_undefined_type(t)
    }

    pub(crate) fn is_optional_uninitialized_parameter_property(
        &mut self,
        parameter: P<Node>,
    ) -> bool {
        self.strict_null_checks
            && self.is_optional_parameter(parameter)
            && ( /*isJSDocParameterTag(parameter) ||*/parameter.initializer().is_none()) // !!! TODO: JSDoc support
            && ast::has_syntactic_modifier(parameter, ModifierFlags::ParameterPropertyModifier)
    }

    pub(crate) fn is_required_initialized_parameter(
        &mut self,
        parameter: P<Node>,
        enclosing_declaration: Option<P<Node>>,
    ) -> bool {
        if !self.strict_null_checks || self.is_optional_parameter(parameter) || /*isJSDocParameterTag(parameter) ||*/ parameter.initializer().is_none()
        {
            // !!! TODO: JSDoc Support
            return false;
        }
        if ast::has_syntactic_modifier(parameter, ModifierFlags::ParameterPropertyModifier) {
            return enclosing_declaration.is_some()
                && ast::is_function_like_declaration(enclosing_declaration);
        }
        true
    }
}
