// Port of estransforms/classfields.go, part 1 (Go lines 1-1888: types, transformer, constructor and the functions up
// to `visitInNewClassLexicalEnvironment`). Part 2 is classfields_2.rs.

use super::*;
use crate::*;
use printer::{EmitFlags, PrivateIdentifierKind};

bitflags::bitflags! {
    // classfields.go:18
    // classFacts tracks various facts about a class being transformed.
    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
    pub(crate) struct classFacts: i32 {
        const None = 0;
        const ClassWasDecorated = 1 << 0;
        const NeedsClassConstructorReference = 1 << 1;
        const NeedsClassSuperReference = 1 << 2;
        const NeedsSubstitutionForThisInClassStaticField = 1 << 3;
        const WillHoistInitializersToConstructor = 1 << 4;
    }
}

// classfields.go:31
// privateIdentifierKind represents the kind of private identifier declaration.
// privateIdentifierInfo stores information about a private identifier during transformation.
pub(crate) struct privateIdentifierInfo {
    pub(crate) kind: PrivateIdentifierKind,
    // brandCheckIdentifier can contain:
    //  - For instance field: The WeakMap that will be the storage for the field.
    //  - For instance methods or accessors: The WeakSet that will be used for brand checking.
    //  - For static members: The constructor that will be used for brand checking.
    pub(crate) brand_check_identifier: Option<P<Node>>,
    // isStatic stores if the identifier is static or not.
    pub(crate) is_static: bool,
    // isValid stores if the identifier declaration is valid or not. Reserved names (e.g. #constructor)
    // or duplicate identifiers are considered invalid.
    pub(crate) is_valid: bool,
    // variableName contains the variable that will serve as the storage for a static field.
    pub(crate) variable_name: Option<P<Node>>,
    // methodName is the identifier for a variable that will contain the private method implementation.
    pub(crate) method_name: Option<P<Node>>,
    // getterName is the identifier for a variable that will contain the private get accessor implementation, if any.
    pub(crate) getter_name: Cell<Option<P<Node>>>,
    // setterName is the identifier for a variable that will contain the private set accessor implementation, if any.
    pub(crate) setter_name: Cell<Option<P<Node>>>,
}

// classfields.go:54
// privateEnvironmentData stores class-scoped environment data for private identifiers.
#[derive(Default)]
pub(crate) struct privateEnvironmentData {
    // className is used for prefixing generated variable names.
    pub(crate) class_name: Cell<Option<P<Node>>>,
    // weakSetName is used for brand check on private methods.
    pub(crate) weak_set_name: Cell<Option<P<Node>>>,
}

// classfields.go:65
// privateEnvironment stores a map of private identifier names to their transform info.
// Like Strada, it uses two separate maps: one for non-generated identifiers (keyed by text)
// and one for generated identifiers (keyed by original AST node). This prevents collisions
// when different auto-accessors produce generated backing field names with the same text.
#[derive(Default)]
pub(crate) struct privateEnvironment {
    pub(crate) data: privateEnvironmentData,
    pub(crate) members: RefCell<FxHashMap<String, P<privateIdentifierInfo>>>,
    pub(crate) generated_identifiers: RefCell<FxHashMap<P<Node>, P<privateIdentifierInfo>>>,
}

// classfields.go:72
// classLexicalEnvironment stores information about the lexical environment of a class.
#[derive(Default)]
pub(crate) struct classLexicalEnvironment {
    pub(crate) facts: Cell<classFacts>,
    // classConstructor is used for brand checks on static members, and `this` references in static initializers.
    pub(crate) class_constructor: Cell<Option<P<Node>>>,
    pub(crate) class_this: Cell<Option<P<Node>>>,
    // superClassReference is used for `super` references in static initializers.
    pub(crate) super_class_reference: Cell<Option<P<Node>>>,
}

// classfields.go:82
// classLexicalEnv is a linked list of class lexical environments.
pub(crate) struct classLexicalEnv {
    pub(crate) previous: Option<P<classLexicalEnv>>,
    pub(crate) data: Cell<Option<P<classLexicalEnvironment>>>,
    pub(crate) private_env: Cell<Option<P<privateEnvironment>>>,
}

// classfields.go:88
pub struct classFieldsTransformer {
    pub base: Transformer,
    pub(crate) compiler_options: P<CompilerOptions>,
    pub(crate) resolver: ReferenceResolverRef,

    // Computed configuration flags
    pub(crate) should_transform_initializers_using_set: Cell<bool>,
    pub(crate) should_transform_initializers_using_define: Cell<bool>,
    pub(crate) should_transform_initializers: Cell<bool>,
    pub(crate) should_transform_private_elements_or_class_static_blocks: Cell<bool>,
    pub(crate) should_transform_auto_accessors: Cell<bool>,
    pub(crate) should_transform_this_in_static_initializers: Cell<bool>,
    pub(crate) should_transform_super_in_static_initializers: Cell<bool>,
    pub(crate) should_transform_private_static_elements_in_file: Cell<bool>,
    pub(crate) legacy_decorators: bool,

    // pendingExpressions tracks what computed name expressions originating from elided names
    // must be inlined at the next execution site, in document order.
    pub(crate) pending_expressions: RefCell<Vec<P<Node>>>,
    // pendingStatements tracks what computed name expression statements and static property
    // initializers must be emitted at the next execution site, in document order (for decorated classes).
    pub(crate) pending_statements: RefCell<Vec<P<Node>>>,
    pub(crate) lexical_environment: Cell<Option<P<classLexicalEnv>>>,
    pub(crate) current_class_container: Cell<Option<P<Node>>>,
    pub(crate) current_class_element: Cell<Option<P<Node>>>,
    // classAliases maps class declarations to alias identifiers for substituting class name
    // references in static initializers. Replaces Strada's onSubstituteNode/trySubstituteClassAlias.
    pub(crate) class_aliases: RefCell<FxHashMap<P<Node>, P<Node>>>,
    pub(crate) enclosing_class_declarations: RefCell<FxHashSet<P<Node>>>,
    pub(crate) in_iteration_statement: Cell<bool>,
    // insideComputedPropertyName replaces Strada's onEmitNode for ComputedPropertyName, which
    // switches to the outer lexical environment. Used by visitThisExpression() to apply
    // the outer environment's substitution without requiring currentClassElement to be static.
    pub(crate) inside_computed_property_name: Cell<bool>,
    pub(crate) parent_node: Cell<Option<P<Node>>>,
    pub(crate) current_node: Cell<Option<P<Node>>>,

    // Visitors
    pub(crate) modifier_visitor: OnceCell<NodeVisitor>,
    pub(crate) discarded_value_visitor: OnceCell<NodeVisitor>,
    pub(crate) heritage_clause_visitor: OnceCell<NodeVisitor>,
    pub(crate) assignment_target_visitor: OnceCell<NodeVisitor>,
    pub(crate) class_element_visitor: OnceCell<NodeVisitor>,
    pub(crate) accessor_field_result_visitor: OnceCell<NodeVisitor>,
    pub(crate) array_assignment_element_visitor: OnceCell<NodeVisitor>,
    pub(crate) object_assignment_element_visitor: OnceCell<NodeVisitor>,
    pub(crate) substitution_visitor: OnceCell<NodeVisitor>,
    // Go `isAnonymousClassNeedingAssignedName` (a pre-bound method value) is a closure built at each call site.
}

// classfields.go:140
pub fn new_class_fields_transformer(opts: &TransformOptions) -> Option<P<Transformer>> {
    let language_version = opts.compiler_options.get_emit_script_target();
    let use_define_for_class_fields = opts.compiler_options.get_use_define_for_class_fields();

    // When targeting ESNext+ with useDefineForClassFields (the default), there are no class
    // field transformations to perform and no prior transform sets EFTransformPrivateStaticElements,
    // so every node would be returned unchanged. Skip entirely.
    if language_version >= ScriptTarget::ESNext && use_define_for_class_fields {
        return None;
    }

    let tx = P::new(classFieldsTransformer {
        base: Transformer::default(),
        compiler_options: opts.compiler_options,
        resolver: opts.resolver,
        should_transform_initializers_using_set: Cell::new(false),
        should_transform_initializers_using_define: Cell::new(false),
        should_transform_initializers: Cell::new(false),
        should_transform_private_elements_or_class_static_blocks: Cell::new(false),
        should_transform_auto_accessors: Cell::new(false),
        should_transform_this_in_static_initializers: Cell::new(false),
        should_transform_super_in_static_initializers: Cell::new(false),
        should_transform_private_static_elements_in_file: Cell::new(false),
        legacy_decorators: opts.compiler_options.experimental_decorators.is_true(),
        pending_expressions: RefCell::new(Vec::new()),
        pending_statements: RefCell::new(Vec::new()),
        lexical_environment: Cell::new(None),
        current_class_container: Cell::new(None),
        current_class_element: Cell::new(None),
        class_aliases: RefCell::new(FxHashMap::default()),
        enclosing_class_declarations: RefCell::new(FxHashSet::default()),
        in_iteration_statement: Cell::new(false),
        inside_computed_property_name: Cell::new(false),
        parent_node: Cell::new(None),
        current_node: Cell::new(None),
        modifier_visitor: OnceCell::new(),
        discarded_value_visitor: OnceCell::new(),
        heritage_clause_visitor: OnceCell::new(),
        assignment_target_visitor: OnceCell::new(),
        class_element_visitor: OnceCell::new(),
        accessor_field_result_visitor: OnceCell::new(),
        array_assignment_element_visitor: OnceCell::new(),
        object_assignment_element_visitor: OnceCell::new(),
        substitution_visitor: OnceCell::new(),
    });

    // Always transform field initializers using Set semantics when `useDefineForClassFields: false`.
    tx.should_transform_initializers_using_set.set(!use_define_for_class_fields);

    // Transform field initializers using Define semantics when `useDefineForClassFields: true` and target < ES2022.
    tx.should_transform_initializers_using_define.set(use_define_for_class_fields && language_version < ScriptTarget::ES2022);

    tx.should_transform_initializers.set(tx.should_transform_initializers_using_set.get() || tx.should_transform_initializers_using_define.get());

    // We need to transform private members and class static blocks when target < ES2022.
    tx.should_transform_private_elements_or_class_static_blocks.set(language_version < ScriptTarget::ES2022);

    // We need to transform `accessor` fields when target < ESNext.
    // We may need to transform `accessor` fields when `useDefineForClassFields: false`
    tx.should_transform_auto_accessors.set(language_version < ScriptTarget::ESNext);

    // We need to transform `this` in a static initializer into a reference to the class
    // when target < ES2022 since the assignment will be moved outside of the class body.
    tx.should_transform_this_in_static_initializers.set(language_version < ScriptTarget::ES2022);

    // Since target is always >= ES2015, this is always the same as
    // shouldTransformThisInStaticInitializers.
    tx.should_transform_super_in_static_initializers.set(tx.should_transform_this_in_static_initializers.get());

    let result = tx.get().base.new_transformer(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit(n)), Some(opts.context));
    let ec = tx.emit_context();
    let _ = tx.modifier_visitor.set(ec.new_node_visitor(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit_modifier(n))));
    let _ = tx.discarded_value_visitor.set(ec.new_node_visitor(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit_discarded_value(n))));
    let _ = tx.heritage_clause_visitor.set(ec.new_node_visitor(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit_heritage_clause(n))));
    let _ = tx.assignment_target_visitor.set(ec.new_node_visitor(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit_assignment_target(n))));
    let _ = tx.class_element_visitor.set(ec.new_node_visitor(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit_class_element(n))));
    let _ = tx.accessor_field_result_visitor.set(ec.new_node_visitor(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit_accessor_field_result(n))));
    let _ = tx.array_assignment_element_visitor.set(ec.new_node_visitor(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit_array_assignment_element(n))));
    let _ = tx.object_assignment_element_visitor.set(ec.new_node_visitor(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit_object_assignment_element(n))));
    let _ = tx.substitution_visitor.set(ec.new_node_visitor(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit_for_substitution(n))));

    Some(result)
}

impl classFieldsTransformer {
    pub(crate) fn emit_context(&self) -> P<EmitContext> {
        self.base.emit_context()
    }

    pub(crate) fn factory(&self) -> &'static printer::NodeFactory {
        self.base.factory()
    }

    pub(crate) fn visitor(&self) -> NodeVisitor {
        self.base.visitor()
    }

    pub(crate) fn modifier_visitor(&self) -> NodeVisitor {
        self.modifier_visitor.get().unwrap().clone()
    }

    pub(crate) fn discarded_value_visitor(&self) -> NodeVisitor {
        self.discarded_value_visitor.get().unwrap().clone()
    }

    pub(crate) fn heritage_clause_visitor(&self) -> NodeVisitor {
        self.heritage_clause_visitor.get().unwrap().clone()
    }

    pub(crate) fn assignment_target_visitor(&self) -> NodeVisitor {
        self.assignment_target_visitor.get().unwrap().clone()
    }

    pub(crate) fn class_element_visitor(&self) -> NodeVisitor {
        self.class_element_visitor.get().unwrap().clone()
    }

    pub(crate) fn accessor_field_result_visitor(&self) -> NodeVisitor {
        self.accessor_field_result_visitor.get().unwrap().clone()
    }

    pub(crate) fn array_assignment_element_visitor(&self) -> NodeVisitor {
        self.array_assignment_element_visitor.get().unwrap().clone()
    }

    pub(crate) fn object_assignment_element_visitor(&self) -> NodeVisitor {
        self.object_assignment_element_visitor.get().unwrap().clone()
    }

    pub(crate) fn substitution_visitor(&self) -> NodeVisitor {
        self.substitution_visitor.get().unwrap().clone()
    }

    // Go `tx.isAnonymousClassNeedingAssignedName`.
    pub(crate) fn is_anonymous_class_needing_assigned_name(&self, node: P<Node>) -> bool {
        self.is_anonymous_class_needing_assigned_name_worker(node)
    }

    // classfields.go:199
    // requiresBlockScopedVar returns true when private field temp variables should be
    // declared as block-scoped (let) rather than function-scoped (var). This occurs when
    // a class expression is directly inside a loop body.
    // Replaces Strada's resolver.hasNodeCheckFlag(node, NodeCheckFlags.BlockScopedBindingInLoop).
    pub(crate) fn requires_block_scoped_var(&self) -> bool {
        self.in_iteration_statement.get() && self.current_class_container.get().is_some_and(|c| ast::is_class_expression(c))
    }

    // classfields.go:207
    // classExpressionNeedsBlockScopedTemp returns true when the class expression's temp variable
    // must be block-scoped. This is more specific than requiresBlockScopedVar: the class temp only
    // needs to be block-scoped when the class expression has a non-static property with a computed
    // property name inside a loop (matching the checker's BlockScopedBindingInLoop on the class node).
    pub(crate) fn class_expression_needs_block_scoped_temp(&self) -> bool {
        if !self.requires_block_scoped_var() {
            return false;
        }
        for member in self.current_class_container.get().unwrap().members() {
            if ast::is_property_declaration(*member) && !ast::has_static_modifier(*member) && member.name().is_some_and(|n| ast::is_computed_property_name(n)) {
                return true;
            }
        }
        false
    }

    // classfields.go:220
    pub(crate) fn visit_source_file(&self, node: P<Node>) -> P<Node> {
        if node.as_source_file().is_declaration_file.get() {
            return node;
        }
        self.lexical_environment.set(None);
        self.should_transform_private_static_elements_in_file.set(self.emit_context().emit_flags(node).intersects(EmitFlags::TransformPrivateStaticElements));
        *self.class_aliases.borrow_mut() = FxHashMap::default();
        self.enclosing_class_declarations.borrow_mut().clear();
        let visited = self.visitor().visit_each_child(Some(node)).unwrap();
        self.emit_context().add_emit_helper(visited, &self.emit_context().read_emit_helpers());
        *self.class_aliases.borrow_mut() = FxHashMap::default();
        self.enclosing_class_declarations.borrow_mut().clear();
        visited
    }

    // classfields.go:235
    pub(crate) fn visit_modifier(&self, node: P<Node>) -> Option<P<Node>> {
        if node.kind() == Kind::AccessorKeyword {
            if self.should_transform_auto_accessors_in_current_class() {
                return None;
            }
            return Some(node);
        }
        if ast::is_modifier(node) {
            return Some(node);
        }
        None
    }

    // classfields.go:248
    pub(crate) fn push_node(&self, node: P<Node>) -> Option<P<Node>> {
        let grandparent_node = self.parent_node.get();
        self.parent_node.set(self.current_node.get());
        self.current_node.set(Some(node));
        grandparent_node
    }

    // classfields.go:255
    pub(crate) fn pop_node(&self, grandparent_node: Option<P<Node>>) {
        self.current_node.set(self.parent_node.get());
        self.parent_node.set(grandparent_node);
    }

    // classfields.go:265
    // visitForSubstitution visits nodes solely for class alias substitution in subtrees
    // that don't contain class field or lexical this/super transforms. It substitutes
    // identifiers that reference class declarations with their aliases, while skipping
    // the .Name() of PropertyAccessExpressions since Strada's onSubstituteNode only
    // fires for EmitHint.Expression, which excludes property access names.
    pub(crate) fn visit_for_substitution(&self, node: P<Node>) -> Option<P<Node>> {
        if node.kind() == Kind::Identifier {
            return Some(self.visit_identifier(node));
        }
        if node.kind() == Kind::PropertyAccessExpression && ast::is_identifier(node.as_property_access_expression().name()) {
            return Some(self.visit_property_access_expression_for_substitution(node));
        }
        self.substitution_visitor().visit_each_child(Some(node))
    }

    // classfields.go:276
    // visit is the main visitor.
    pub(crate) fn visit(&self, node: P<Node>) -> Option<P<Node>> {
        let grandparent_node = self.push_node(node);
        let result = self.visit_worker(node);
        self.pop_node(grandparent_node);
        result
    }

    // Body of Go `visit` after the deferred `popNode`.
    fn visit_worker(&self, node: P<Node>) -> Option<P<Node>> {
        if !node.subtree_facts().intersects(SubtreeFacts::ContainsClassFields | SubtreeFacts::ContainsLexicalThisOrSuper) {
            if self.current_class_container.get().is_some() && !self.class_aliases.borrow().is_empty() {
                // Continue visiting for alias substitution even in non-class-field subtrees.
                return self.visit_for_substitution(node);
            }
            return Some(node);
        }

        match node.kind() {
            Kind::SourceFile => Some(self.visit_source_file(node)),
            Kind::ClassDeclaration => self.visit_class_declaration(node),
            Kind::ClassExpression => self.visit_class_expression(node),
            Kind::ClassStaticBlockDeclaration | Kind::PropertyDeclaration => panic!("Use `classElementVisitor` instead."),
            Kind::PropertyAssignment => self.visit_property_assignment(node),
            Kind::VariableStatement => self.visit_variable_statement(node),
            Kind::VariableDeclaration => self.visit_variable_declaration(node),
            Kind::Parameter => self.visit_parameter_declaration(node),
            Kind::BindingElement => self.visit_binding_element(node),
            Kind::ExportAssignment => self.visit_export_assignment(node),
            Kind::PrivateIdentifier => self.visit_private_identifier(node),
            Kind::PropertyAccessExpression => self.visit_property_access_expression(node),
            Kind::ElementAccessExpression => self.visit_element_access_expression(node),
            Kind::PrefixUnaryExpression | Kind::PostfixUnaryExpression => self.visit_pre_or_postfix_unary_expression(node, false /*discarded*/),
            Kind::BinaryExpression => self.visit_binary_expression(node, false /*discarded*/),
            Kind::ParenthesizedExpression => self.visit_parenthesized_expression(node, false /*discarded*/),
            Kind::CallExpression => self.visit_call_expression(node),
            Kind::ExpressionStatement => self.visit_expression_statement(node),
            Kind::TaggedTemplateExpression => self.visit_tagged_template_expression(node),
            Kind::ForStatement => self.visit_for_statement(node),
            Kind::ForInStatement | Kind::ForOfStatement | Kind::DoStatement | Kind::WhileStatement => self.set_in_iteration_statement_and(true, Self::visit_each_child_of_node, node),
            Kind::ThisKeyword => self.visit_this_expression(node),
            Kind::FunctionDeclaration | Kind::FunctionExpression => self.set_in_iteration_statement_and(false, Self::visit_function_expression_or_declaration, node),
            Kind::Constructor | Kind::MethodDeclaration | Kind::GetAccessor | Kind::SetAccessor => self.set_in_iteration_statement_and(false, Self::set_class_element_and_visit_each_child, node),
            _ => self.visitor().visit_each_child(Some(node)),
        }
    }

    // classfields.go:343
    // visitDiscardedValue visits a node in an expression whose result is discarded.
    pub(crate) fn visit_discarded_value(&self, node: P<Node>) -> Option<P<Node>> {
        match node.kind() {
            Kind::PrefixUnaryExpression | Kind::PostfixUnaryExpression => self.visit_pre_or_postfix_unary_expression(node, true /*discarded*/),
            Kind::BinaryExpression => self.visit_binary_expression(node, true /*discarded*/),
            Kind::ParenthesizedExpression => self.visit_parenthesized_expression(node, true /*discarded*/),
            _ => self.visit(node),
        }
    }

    // classfields.go:357
    // visitHeritageClause visits a node in a HeritageClause.
    pub(crate) fn visit_heritage_clause(&self, node: P<Node>) -> Option<P<Node>> {
        match node.kind() {
            Kind::HeritageClause => self.heritage_clause_visitor().visit_each_child(Some(node)),
            Kind::ExpressionWithTypeArguments => self.visit_expression_with_type_arguments_in_heritage_clause(node),
            _ => self.visit(node),
        }
    }

    // classfields.go:369
    // visitAssignmentTarget visits the assignment target of a destructuring assignment.
    pub(crate) fn visit_assignment_target(&self, node: P<Node>) -> Option<P<Node>> {
        match node.kind() {
            Kind::ObjectLiteralExpression | Kind::ArrayLiteralExpression => self.visit_assignment_pattern(node),
            _ => self.visit(node),
        }
    }

    // classfields.go:378
    pub(crate) fn visit_destructuring_assignment_target(&self, node: P<Node>) -> Option<P<Node>> {
        if ast::is_object_literal_expression(node) || ast::is_array_literal_expression(node) {
            return self.visit_assignment_pattern(node);
        }
        if ast::is_property_access_expression(node) && ast::is_private_identifier(node.as_property_access_expression().name()) {
            return self.wrap_private_identifier_for_destructuring_target(node);
        }
        if self.should_transform_super_in_static_initializers.get()
            && self.current_class_element.get().is_some()
            && ast::is_super_property(node)
            && is_static_property_declaration_or_class_static_block(self.current_class_element.get().unwrap())
            && self.lexical_environment.get().is_some_and(|l| l.data.get().is_some())
        {
            let data = self.lexical_environment.get().unwrap().data.get().unwrap();
            if data.facts.get().intersects(classFacts::ClassWasDecorated) {
                return self.visit_invalid_super_property(node);
            }
            if let (Some(class_constructor), Some(super_class_reference)) = (data.class_constructor.get(), data.super_class_reference.get()) {
                let mut name: Option<P<Node>> = None;
                if ast::is_element_access_expression(node) {
                    name = self.visitor().visit_node(Some(node.as_element_access_expression().argument_expression));
                } else if ast::is_property_access_expression(node) && ast::is_identifier(node.as_property_access_expression().name()) {
                    name = Some(self.factory().new_string_literal_from_node(node.as_property_access_expression().name()));
                }
                if let Some(name) = name {
                    let temp = self.factory().new_temp_variable();
                    let set_expr = self.factory().new_reflect_set_call(super_class_reference, name, temp, class_constructor);
                    return Some(self.factory().new_assignment_target_wrapper(temp, set_expr));
                }
            }
        }
        self.visitor().visit_each_child(Some(node))
    }

    // classfields.go:416
    // visitClassElement visits a member of a class.
    pub(crate) fn visit_class_element(&self, node: P<Node>) -> Option<P<Node>> {
        match node.kind() {
            Kind::Constructor => self.set_current_class_element_and(Some(node), Self::visit_constructor_declaration, node),
            Kind::GetAccessor | Kind::SetAccessor | Kind::MethodDeclaration => self.set_current_class_element_and(Some(node), Self::visit_method_or_accessor_declaration, node),
            Kind::PropertyDeclaration => self.set_current_class_element_and(Some(node), Self::visit_property_declaration, node),
            Kind::ClassStaticBlockDeclaration => self.set_current_class_element_and(Some(node), Self::visit_class_static_block_declaration, node),
            Kind::ComputedPropertyName => self.visit_computed_property_name(node),
            Kind::SemicolonClassElement => Some(node),
            _ => {
                if ast::is_modifier_like(node) {
                    return self.visit_modifier(node);
                }
                self.visit(node)
            }
        }
    }

    // classfields.go:439
    // visitPropertyName visits a property name of a class member.
    pub(crate) fn visit_property_name(&self, name: P<Node>) -> Option<P<Node>> {
        if ast::is_computed_property_name(name) {
            return self.visit_computed_property_name(name);
        }
        self.visitor().visit_node(Some(name))
    }

    // classfields.go:447
    // visitAccessorFieldResult visits the results of an auto-accessor field transformation in a second pass.
    pub(crate) fn visit_accessor_field_result(&self, node: P<Node>) -> Option<P<Node>> {
        match node.kind() {
            Kind::PropertyDeclaration => self.transform_field_initializer(node),
            Kind::GetAccessor | Kind::SetAccessor => self.visit_class_element(node),
            _ => tsrs_core::debug::fail_bad_syntax_kind(&node.kind_string(), &[&"Expected node to either be a PropertyDeclaration, GetAccessorDeclaration, or SetAccessorDeclaration"]),
        }
    }

    // classfields.go:462
    // visitIdentifier replaces Strada's onSubstituteNode/trySubstituteClassAlias. Instead of
    // substituting at emit time using NodeCheckFlags.ConstructorReference, we resolve the
    // identifier to its declaration and check if that declaration has a registered alias.
    pub(crate) fn visit_identifier(&self, node: P<Node>) -> P<Node> {
        let declaration = self.resolver.get_referenced_value_declaration(self.emit_context().most_original(Some(node)).unwrap());
        if let Some(declaration) = declaration {
            let alias = self.class_aliases.borrow().get(&declaration).copied();
            if let Some(alias) = alias {
                if self.enclosing_class_declarations.borrow().contains(&declaration) {
                    let clone = alias.clone_node(self.factory());
                    self.emit_context().set_source_map_range(clone, node.loc());
                    self.emit_context().set_comment_range(clone, node.loc());
                    return clone;
                }
            }
        }
        node
    }

    // classfields.go:479
    // visitPrivateIdentifier handles an undeclared private name. Replace it with an empty
    // identifier to indicate a problem with the code.
    // Note: private identifiers in statement position (e.g., `#;`) are intercepted earlier
    // by visitExpressionStatement, which preserves them so the runtime throws a SyntaxError.
    pub(crate) fn visit_private_identifier(&self, node: P<Node>) -> Option<P<Node>> {
        if !self.should_transform_private_elements_or_class_static_blocks.get() {
            return Some(node);
        }
        if self.parent_node.get().is_some_and(|p| ast::is_statement(p)) {
            return Some(node);
        }
        let result = self.factory().new_identifier("");
        self.emit_context().set_original(result, node);
        Some(result)
    }

    // classfields.go:492
    // transformPrivateIdentifierInInExpression visits `#id in expr`.
    pub(crate) fn transform_private_identifier_in_in_expression(&self, node: P<Node>) -> Option<P<Node>> {
        let b = node.as_binary_expression();
        let info = self.access_private_identifier(b.left);
        if let Some(info) = info {
            let receiver = self.visitor().visit_node(Some(b.right())).unwrap();
            let result = self.factory().new_class_private_field_in_helper(info.brand_check_identifier.unwrap(), receiver);
            self.emit_context().set_original(result, node);
            return Some(result);
        }
        // Private name has not been declared. Subsequent transformers will handle this error
        self.visitor().visit_each_child(Some(node))
    }

    // classfields.go:504
    pub(crate) fn visit_property_assignment(&self, mut node: P<Node>) -> Option<P<Node>> {
        // 13.2.5.5 RS: PropertyDefinitionEvaluation
        //   PropertyAssignment : PropertyName `:` AssignmentExpression
        //     ...
        //     5. If IsAnonymousFunctionDefinition(|AssignmentExpression|) is *true* and _isProtoSetter_ is *false*, then
        //        a. Let _popValue_ be ? NamedEvaluation of |AssignmentExpression| with argument _propKey_.
        //     ...

        if is_named_evaluation_and(self.emit_context(), node, Some(&|n| self.is_anonymous_class_needing_assigned_name(n))) {
            node = transform_named_evaluation(self.emit_context(), node, false /*ignoreEmptyStringLiteral*/, "" /*assignedName*/);
        }
        self.visitor().visit_each_child(Some(node))
    }

    // classfields.go:518
    pub(crate) fn visit_variable_statement(&self, node: P<Node>) -> Option<P<Node>> {
        let saved_pending_statements = std::mem::take(&mut *self.pending_statements.borrow_mut());

        let visited_node = self.visitor().visit_each_child(Some(node));

        if !self.pending_statements.borrow().is_empty() {
            let pending = std::mem::replace(&mut *self.pending_statements.borrow_mut(), saved_pending_statements);
            let mut result: Vec<P<Node>> = Vec::with_capacity(1 + pending.len());
            result.push(visited_node.unwrap());
            result.extend(pending);
            return Some(self.factory().new_syntax_list(alloc_vec(result)));
        }

        *self.pending_statements.borrow_mut() = saved_pending_statements;
        visited_node
    }

    // classfields.go:536
    pub(crate) fn visit_variable_declaration(&self, mut node: P<Node>) -> Option<P<Node>> {
        // 14.3.1.2 RS: Evaluation
        //   LexicalBinding : BindingIdentifier Initializer
        //     ...
        //     3. If IsAnonymousFunctionDefinition(|Initializer|) is *true*, then
        //        a. Let _value_ be ? NamedEvaluation of |Initializer| with argument _bindingId_.
        //     ...
        //
        // 14.3.2.1 RS: Evaluation
        //   VariableDeclaration : BindingIdentifier Initializer
        //     ...
        //     3. If IsAnonymousFunctionDefinition(|Initializer|) is *true*, then
        //        a. Let _value_ be ? NamedEvaluation of |Initializer| with argument _bindingId_.
        //     ...

        if is_named_evaluation_and(self.emit_context(), node, Some(&|n| self.is_anonymous_class_needing_assigned_name(n))) {
            node = transform_named_evaluation(self.emit_context(), node, false, "");
        }
        self.visitor().visit_each_child(Some(node))
    }

    // classfields.go:557
    pub(crate) fn visit_parameter_declaration(&self, mut node: P<Node>) -> Option<P<Node>> {
        // 8.6.3 RS: IteratorBindingInitialization
        //   SingleNameBinding : BindingIdentifier Initializer?
        //     ...
        //     5. If |Initializer| is present and _v_ is *undefined*, then
        //        a. If IsAnonymousFunctionDefinition(|Initializer|) is *true*, then
        //           i. Set _v_ to ? NamedEvaluation of |Initializer| with argument _bindingId_.
        //     ...
        //
        // 14.3.3.3 RS: KeyedBindingInitialization
        //   SingleNameBinding : BindingIdentifier Initializer?
        //     ...
        //     4. If |Initializer| is present and _v_ is *undefined*, then
        //        a. If IsAnonymousFunctionDefinition(|Initializer|) is *true*, then
        //           i. Set _v_ to ? NamedEvaluation of |Initializer| with argument _bindingId_.
        //     ...

        if is_named_evaluation_and(self.emit_context(), node, Some(&|n| self.is_anonymous_class_needing_assigned_name(n))) {
            node = transform_named_evaluation(self.emit_context(), node, false, "");
        }
        self.visitor().visit_each_child(Some(node))
    }

    // classfields.go:580
    pub(crate) fn visit_binding_element(&self, mut node: P<Node>) -> Option<P<Node>> {
        // 8.6.3 RS: IteratorBindingInitialization
        //   SingleNameBinding : BindingIdentifier Initializer?
        //     ...
        //     5. If |Initializer| is present and _v_ is *undefined*, then
        //        a. If IsAnonymousFunctionDefinition(|Initializer|) is *true*, then
        //           i. Set _v_ to ? NamedEvaluation of |Initializer| with argument _bindingId_.
        //     ...
        //
        // 14.3.3.3 RS: KeyedBindingInitialization
        //   SingleNameBinding : BindingIdentifier Initializer?
        //     ...
        //     4. If |Initializer| is present and _v_ is *undefined*, then
        //        a. If IsAnonymousFunctionDefinition(|Initializer|) is *true*, then
        //           i. Set _v_ to ? NamedEvaluation of |Initializer| with argument _bindingId_.
        //     ...

        if is_named_evaluation_and(self.emit_context(), node, Some(&|n| self.is_anonymous_class_needing_assigned_name(n))) {
            node = transform_named_evaluation(self.emit_context(), node, false, "");
        }
        self.visitor().visit_each_child(Some(node))
    }

    // classfields.go:603
    pub(crate) fn visit_export_assignment(&self, mut node: P<Node>) -> Option<P<Node>> {
        // 16.2.3.7 RS: Evaluation
        //   ExportDeclaration : `export` `default` AssignmentExpression `;`
        //     1. If IsAnonymousFunctionDefinition(|AssignmentExpression|) is *true*, then
        //        a. Let _value_ be ? NamedEvaluation of |AssignmentExpression| with argument `"default"`.
        //     ...

        // NOTE: Since emit for `export =` translates to `module.exports = ...`, the assigned name of the class
        // is `""`.

        if is_named_evaluation_and(self.emit_context(), node, Some(&|n| self.is_anonymous_class_needing_assigned_name(n))) {
            let mut assigned_name = "";
            if !node.as_export_assignment().is_export_equals {
                assigned_name = "default";
            }
            node = transform_named_evaluation(self.emit_context(), node, true /*ignoreEmptyStringLiteral*/, assigned_name);
        }
        self.visitor().visit_each_child(Some(node))
    }

    // classfields.go:623
    pub(crate) fn inject_pending_expressions(&self, mut expression: P<Node>) -> P<Node> {
        if !self.pending_expressions.borrow().is_empty() {
            if ast::is_parenthesized_expression(expression) {
                self.pending_expressions.borrow_mut().push(expression.expression().unwrap());
                let inlined = self.factory().inline_expressions(&self.pending_expressions.borrow()).unwrap();
                expression = self.factory().update_parenthesized_expression(expression, inlined);
            } else {
                let mut exprs = self.pending_expressions.borrow().clone();
                exprs.push(expression);
                expression = self.factory().inline_expressions(&exprs).unwrap();
            }
            self.pending_expressions.borrow_mut().clear();
        }
        expression
    }

    // classfields.go:640
    pub(crate) fn visit_computed_property_name(&self, node: P<Node>) -> Option<P<Node>> {
        // Computed property names are evaluated in the enclosing scope, not the current class.
        // Replaces Strada's onEmitNode for ComputedPropertyName which switches to
        // lexicalEnvironment?.previous. We do this explicitly during transformation.
        let saved_lexical_environment = self.lexical_environment.get();
        let saved_inside_computed_property_name = self.inside_computed_property_name.get();
        self.inside_computed_property_name.set(true);
        if let Some(previous) = self.lexical_environment.get().and_then(|l| l.previous) {
            self.lexical_environment.set(Some(previous));
        }
        let expression = self.visitor().visit_node(Some(node.as_computed_property_name().expression)).unwrap();
        self.lexical_environment.set(saved_lexical_environment);
        self.inside_computed_property_name.set(saved_inside_computed_property_name);
        Some(self.factory().update_computed_property_name(node, self.inject_pending_expressions(expression)))
    }

    // classfields.go:656
    pub(crate) fn visit_constructor_declaration(&self, node: P<Node>) -> Option<P<Node>> {
        if let Some(container) = self.current_class_container.get() {
            return self.transform_constructor(node, container);
        }
        self.visitor().visit_each_child(Some(node))
    }

    // classfields.go:663
    pub(crate) fn should_transform_class_element_to_weak_map(&self, node: P<Node>) -> bool {
        if self.should_transform_private_elements_or_class_static_blocks.get() {
            return true;
        }
        self.should_always_transform_private_static_elements(node)
    }

    // classfields.go:670
    pub(crate) fn should_always_transform_private_static_elements(&self, node: P<Node>) -> bool {
        ast::has_static_modifier(node) && self.emit_context().emit_flags(node).intersects(EmitFlags::TransformPrivateStaticElements)
    }

    // classfields.go:677
    // nodeHasTransformPrivateStaticElementsFlag checks the emit flag on a class node (not a member).
    // Unlike shouldAlwaysTransformPrivateStaticElements, this does not check HasStaticModifier,
    // since class nodes themselves don't have a static modifier.
    pub(crate) fn node_has_transform_private_static_elements_flag(&self, node: P<Node>) -> bool {
        self.emit_context().emit_flags(node).intersects(EmitFlags::TransformPrivateStaticElements)
    }

    // classfields.go:681
    pub(crate) fn visit_method_or_accessor_declaration(&self, node: P<Node>) -> Option<P<Node>> {
        assert!(!ast::has_decorators(node));

        if !ast::is_private_identifier_class_element_declaration(node) || !self.should_transform_class_element_to_weak_map(node) {
            return self.class_element_visitor().visit_each_child(Some(node));
        }

        // leave invalid code untransformed
        let info = self.access_private_identifier(node.name().unwrap());
        assert!(info.is_some(), "Undeclared private name for property declaration.");
        if !info.unwrap().is_valid {
            return Some(node);
        }

        let function_name = self.get_hoisted_function_name(node);
        if let Some(function_name) = function_name {
            let modifiers = self.extract_non_static_non_accessor_modifiers(node);
            self.emit_context().start_variable_environment();
            let saved = self.in_iteration_statement.get();
            self.in_iteration_statement.set(false);
            let body = self.emit_context().visit_function_body(node.body(), &mut self.visitor());
            let params = self.visitor().visit_nodes(node.parameter_list());
            self.in_iteration_statement.set(saved);

            let func_expr = self.factory().new_function_expression(modifiers, node.body_data().unwrap().asterisk_token, Some(function_name), None, params, None, None, body);
            let assignment = self.factory().new_assignment_expression(function_name, func_expr);
            self.add_pending_expressions(&[assignment]);
        }

        // remove method declaration from class
        None
    }

    // classfields.go:714
    pub(crate) fn extract_non_static_non_accessor_modifiers(&self, node: P<Node>) -> Option<P<ModifierList>> {
        extract_modifiers(self.emit_context(), node.modifiers(), !(ModifierFlags::Static | ModifierFlags::Accessor))
    }

    // classfields.go:718
    pub(crate) fn set_current_class_element_and(&self, class_element: Option<P<Node>>, visitor: fn(&Self, P<Node>) -> Option<P<Node>>, node: P<Node>) -> Option<P<Node>> {
        if class_element != self.current_class_element.get() {
            let saved = self.current_class_element.get();
            self.current_class_element.set(class_element);
            let result = visitor(self, node);
            self.current_class_element.set(saved);
            return result;
        }
        visitor(self, node)
    }

    // classfields.go:730
    // visitEachChildOfNode just calls Visitor.VisitEachChild, but is necessary to avoid repeated closure allocations when passing as a callback.
    pub(crate) fn visit_each_child_of_node(&self, node: P<Node>) -> Option<P<Node>> {
        self.visitor().visit_each_child(Some(node))
    }

    // classfields.go:734
    pub(crate) fn set_in_iteration_statement_and(&self, in_iteration: bool, visitor: fn(&Self, P<Node>) -> Option<P<Node>>, node: P<Node>) -> Option<P<Node>> {
        if self.in_iteration_statement.get() != in_iteration {
            let saved = self.in_iteration_statement.get();
            self.in_iteration_statement.set(in_iteration);
            let result = visitor(self, node);
            self.in_iteration_statement.set(saved);
            return result;
        }
        visitor(self, node)
    }

    // classfields.go:745
    pub(crate) fn clear_class_element_and_visit_each_child(&self, node: P<Node>) -> Option<P<Node>> {
        self.set_current_class_element_and(None, Self::visit_each_child_of_node, node)
    }

    // classfields.go:761
    // visitFunctionExpressionOrDeclaration handles lexical environment scoping for function
    // expressions and declarations, mirroring Strada's onEmitNode behavior.
    //
    // In Strada, onEmitNode checks whether a FunctionExpression has been registered in
    // lexicalEnvironmentMap (via its original node). If found, the lexical environment is
    // restored; otherwise it is cleared (since regular functions create a new `this` scope).
    //
    // Since Corsa performs substitution eagerly (no emit-time hooks), we replicate this by
    // preserving currentClassElement for function expressions whose original node is a class
    // member of the current class. This allows visitThisExpression to correctly substitute
    // `this` -> `_classThis` inside synthesized functions (e.g., ES decorator descriptor
    // methods for static private auto-accessors).
    pub(crate) fn visit_function_expression_or_declaration(&self, node: P<Node>) -> Option<P<Node>> {
        if self.current_class_element.get().is_some() {
            let original = self.emit_context().most_original(Some(node));
            if original != Some(node) && self.current_class_container.get().is_some() {
                for member in self.current_class_container.get().unwrap().members() {
                    if self.emit_context().most_original(Some(*member)) == original && ast::is_static(*member) {
                        // The function expression originates from a static class member (e.g., a
                        // descriptor method synthesized by the ES decorator transformer for a
                        // static private auto-accessor). Preserve the current class element so
                        // that visitThisExpression can substitute `this` with `_classThis`.
                        // Non-static members must NOT preserve the class element because `this`
                        // inside their descriptor functions should remain dynamic.
                        return self.visit_each_child_of_node(node);
                    }
                }
            }
        }
        self.set_current_class_element_and(None, Self::visit_each_child_of_node, node)
    }

    // classfields.go:781
    pub(crate) fn set_class_element_and_visit_each_child(&self, node: P<Node>) -> Option<P<Node>> {
        self.set_current_class_element_and(Some(node), Self::visit_each_child_of_node, node)
    }

    // classfields.go:785
    pub(crate) fn get_hoisted_function_name(&self, node: P<Node>) -> Option<P<Node>> {
        assert!(node.name().is_some_and(|n| ast::is_private_identifier(n)));
        let info = self.access_private_identifier(node.name().unwrap());
        assert!(info.is_some(), "Undeclared private name for property declaration.");
        let info = info.unwrap();
        if info.kind == PrivateIdentifierKind::Method {
            return info.method_name;
        }
        if info.kind == PrivateIdentifierKind::Accessor {
            if ast::is_get_accessor_declaration(node) {
                return info.getter_name.get();
            }
            if ast::is_set_accessor_declaration(node) {
                return info.setter_name.get();
            }
        }
        None
    }

    // classfields.go:803
    pub(crate) fn try_get_class_this(&self) -> Option<P<Node>> {
        if let Some(class_this) = self.try_get_class_this_no_container() {
            return Some(class_this);
        }
        if let Some(container) = self.current_class_container.get() {
            return container.name();
        }
        None
    }

    // classfields.go:813
    pub(crate) fn try_get_class_this_no_container(&self) -> Option<P<Node>> {
        let lex = self.get_class_lexical_environment();
        if let Some(class_this) = lex.class_this.get() {
            return Some(class_this);
        }
        if let Some(class_constructor) = lex.class_constructor.get() {
            return Some(class_constructor);
        }
        None
    }

    // classfields.go:833
    // transformAutoAccessor transforms an auto-accessor property:
    //
    //	accessor x = 1;
    //
    // into:
    //
    //	#x = 1;
    //	get x() { return this.#x; }
    //	set x(value) { this.#x = value; }
    pub(crate) fn transform_auto_accessor(&self, node: P<Node>) -> Option<P<Node>> {
        let comment_range = self.emit_context().comment_range(node);
        let source_map_range = self.emit_context().source_map_range(node);

        // Since we're creating two declarations where there was previously one, cache
        // the expression for any computed property names.
        let name = node.name().unwrap();
        let mut getter_name = name;
        let mut setter_name = name;
        if ast::is_computed_property_name(name) && !is_simple_inlineable_expression(name.expression().unwrap()) {
            let cache_assignment = find_computed_property_name_cache_assignment(self.emit_context(), name);
            if let Some(cache_assignment) = cache_assignment {
                getter_name = self.factory().update_computed_property_name(name, self.visitor().visit_node(name.expression()).unwrap());
                setter_name = self.factory().update_computed_property_name(name, cache_assignment.as_binary_expression().left);
            } else {
                let temp = self.factory().new_temp_variable();
                self.emit_context().set_source_map_range(temp, name.expression().unwrap().loc());
                self.emit_context().add_variable_declaration(temp);
                let expression = self.visitor().visit_node(name.expression()).unwrap();
                let assignment = self.factory().new_assignment_expression(temp, expression);
                self.emit_context().set_source_map_range(assignment, name.expression().unwrap().loc());
                getter_name = self.factory().update_computed_property_name(name, assignment);
                setter_name = self.factory().update_computed_property_name(name, temp);
            }
        }

        let modifiers = self.modifier_visitor().visit_modifiers(node.modifiers());
        let backing_field = create_accessor_property_backing_field(self.factory(), node, modifiers, node.initializer());
        self.emit_context().set_original(backing_field, node);
        self.emit_context().add_emit_flags(backing_field, EmitFlags::NoComments);
        self.emit_context().set_source_map_range(backing_field, source_map_range);

        let receiver;
        if ast::is_static(node) {
            receiver = self.try_get_class_this().unwrap_or_else(|| self.factory().new_this_expression());
        } else {
            receiver = self.factory().new_this_expression();
        }

        let getter = self.create_accessor_property_get_redirector(node, modifiers, getter_name, receiver);
        self.emit_context().set_original(getter, node);
        self.emit_context().set_comment_range(getter, comment_range);
        self.emit_context().set_source_map_range(getter, source_map_range);

        // create a fresh copy of the modifiers so that we don't duplicate comments
        let mut setter_modifiers: Option<P<ModifierList>> = None;
        if let Some(modifiers) = modifiers {
            setter_modifiers = Some(self.factory().new_modifier_list(ast::create_modifiers_from_modifier_flags(modifiers.modifier_flags, |k| self.factory().new_modifier(k))));
        }
        let setter = self.create_accessor_property_set_redirector(node, setter_modifiers, setter_name, receiver);
        self.emit_context().set_original(setter, node);
        self.emit_context().add_emit_flags(setter, EmitFlags::NoComments);
        self.emit_context().set_source_map_range(setter, source_map_range);

        // Visit the results in a second pass
        let (visited, _) = self.accessor_field_result_visitor().visit_slice(alloc_vec(vec![backing_field, getter, setter]));
        Some(self.factory().new_syntax_list(visited))
    }

    // classfields.go:895
    pub(crate) fn transform_private_field_initializer(&self, mut node: P<Node>) -> Option<P<Node>> {
        if self.should_transform_class_element_to_weak_map(node) {
            // If we are transforming private elements into WeakMap/WeakSet, we should elide the node.
            let info = self.access_private_identifier(node.name().unwrap());
            assert!(info.is_some(), "Undeclared private name for property declaration.");
            let info = info.unwrap();

            // Leave invalid code untransformed
            if !info.is_valid {
                return Some(node);
            }

            // If we encounter a valid private static field and we're not transforming
            // class static blocks, convert to a static block initializer.
            if info.is_static && !self.should_transform_private_elements_or_class_static_blocks.get() {
                // TODO: fix
                let statement = self.transform_property_or_class_static_block(node, self.factory().new_this_expression());
                if let Some(statement) = statement {
                    return Some(self.factory().new_class_static_block_declaration(
                        None, /*modifiers*/
                        self.factory().new_block(self.factory().new_node_list(vec![statement]), true /*multiLine*/),
                    ));
                }
            }

            return None;
        }

        if self.should_transform_initializers_using_set.get()
            && !ast::has_static_modifier(node)
            && self.lexical_environment.get().is_some_and(|l| l.data.get().is_some_and(|d| d.facts.get().intersects(classFacts::WillHoistInitializersToConstructor)))
        {
            return Some(self.factory().update_property_declaration(
                node,
                self.visitor().visit_modifiers(node.modifiers()),
                node.name().unwrap(),
                None, /*postfixToken*/
                None, /*typeNode*/
                None, /*initializer*/
            ));
        }

        if is_named_evaluation_and(self.emit_context(), node, Some(&|n| self.is_anonymous_class_needing_assigned_name(n))) {
            node = transform_named_evaluation(self.emit_context(), node, false, "");
        }

        Some(self.factory().update_property_declaration(
            node,
            self.modifier_visitor().visit_modifiers(node.modifiers()),
            self.visit_property_name(node.name().unwrap()).unwrap(),
            None, /*postfixToken*/
            None, /*typeNode*/
            self.visitor().visit_node(node.initializer()),
        ))
    }

    // classfields.go:949
    pub(crate) fn transform_public_field_initializer(&self, node: P<Node>) -> Option<P<Node>> {
        if self.should_transform_initializers.get() && !ast::is_auto_accessor_property_declaration(node) {
            // Elide the property declaration; the initializer will be moved to the constructor.
            // For computed property names, we still need to emit the expression.
            let expr = self.get_property_name_expression_if_needed(node.name().unwrap(), node.initializer().is_some() || self.compiler_options.get_use_define_for_class_fields());
            if let Some(expr) = expr {
                for e in flatten_comma_list(expr) {
                    self.add_pending_expressions(&[e]);
                }
            }

            // When target >= ES2022 (i.e., !shouldTransformPrivateElementsOrClassStaticBlocks) and we
            // still need to transform initializers (useDefineForClassFields: false), static property
            // initializers must be converted into `static { this.x = ...; }` blocks so that `this`
            // refers to the class constructor inside the static block.
            if ast::is_static(node) && !self.should_transform_private_elements_or_class_static_blocks.get() {
                let initializer_statement = self.transform_property_or_class_static_block(node, self.factory().new_this_expression());
                if let Some(initializer_statement) = initializer_statement {
                    let static_block = self.factory().new_class_static_block_declaration(
                        None, /*modifiers*/
                        self.factory().new_block(self.factory().new_node_list(vec![initializer_statement]), false),
                    );

                    self.emit_context().set_original(static_block, node);
                    self.emit_context().set_comment_range(static_block, node.loc());

                    self.emit_context().add_emit_flags(initializer_statement, EmitFlags::NoComments);
                    return Some(static_block);
                }
            }

            return None;
        }

        Some(self.factory().update_property_declaration(
            node,
            self.modifier_visitor().visit_modifiers(node.modifiers()),
            self.visit_property_name(node.name().unwrap()).unwrap(),
            None, /*postfixToken*/
            None, /*typeNode*/
            self.visitor().visit_node(node.initializer()),
        ))
    }

    // classfields.go:993
    pub(crate) fn transform_field_initializer(&self, node: P<Node>) -> Option<P<Node>> {
        assert!(!ast::has_decorators(node), "Decorators should already have been transformed and elided.");
        if ast::is_private_identifier_class_element_declaration(node) {
            return self.transform_private_field_initializer(node);
        }
        self.transform_public_field_initializer(node)
    }

    // classfields.go:1001
    pub(crate) fn should_transform_auto_accessors_in_current_class(&self) -> bool {
        if self.should_transform_auto_accessors.get() {
            return true;
        }
        // When targeting ESNext with useDefineForClassFields: false, auto-accessors are only
        // transformed if the current class will hoist initializers to the constructor.
        self.lexical_environment.get().is_some_and(|l| l.data.get().is_some_and(|d| d.facts.get().intersects(classFacts::WillHoistInitializersToConstructor)))
    }

    // classfields.go:1011
    pub(crate) fn visit_property_declaration(&self, node: P<Node>) -> Option<P<Node>> {
        // If this is an auto-accessor, we defer to `transformAutoAccessor`. That function
        // will in turn call `transformFieldInitializer` as needed.
        if ast::is_auto_accessor_property_declaration(node) && (self.should_transform_auto_accessors_in_current_class() || ast::has_static_modifier(node) && self.should_always_transform_private_static_elements(node)) {
            return self.transform_auto_accessor(node);
        }
        self.transform_field_initializer(node)
    }

    // classfields.go:1022
    pub(crate) fn create_private_identifier_access(&self, info: P<privateIdentifierInfo>, receiver: P<Node>) -> P<Node> {
        let receiver = self.visitor().visit_node(Some(receiver)).unwrap();
        self.create_private_identifier_access_helper(info, receiver)
    }

    // classfields.go:1027
    pub(crate) fn create_private_identifier_access_helper(&self, info: P<privateIdentifierInfo>, receiver: P<Node>) -> P<Node> {
        self.emit_context().set_comment_range(receiver, tsrs_core::TextRange::new(-1, receiver.end()));

        match info.kind {
            PrivateIdentifierKind::Accessor => self.factory().new_class_private_field_get_helper(receiver, info.brand_check_identifier.unwrap(), info.kind, info.getter_name.get()),
            PrivateIdentifierKind::Method => self.factory().new_class_private_field_get_helper(receiver, info.brand_check_identifier.unwrap(), info.kind, info.method_name),
            PrivateIdentifierKind::Field => {
                let mut f: Option<P<Node>> = None;
                if info.is_static {
                    f = info.variable_name;
                }
                self.factory().new_class_private_field_get_helper(receiver, info.brand_check_identifier.unwrap(), info.kind, f)
            }
            PrivateIdentifierKind::Untransformed => tsrs_core::debug::fail("Access helpers should not be created for untransformed private elements"),
        }
    }

    // classfields.go:1064
    pub(crate) fn visit_property_access_expression(&self, node: P<Node>) -> Option<P<Node>> {
        let pa = node.as_property_access_expression();
        if ast::is_private_identifier(pa.name()) {
            let info = self.access_private_identifier(pa.name());
            if let Some(info) = info {
                let result = self.create_private_identifier_access(info, pa.expression);
                self.emit_context().set_original(result, node);
                result.set_loc(node.loc());
                return Some(result);
            }
        }
        if self.should_transform_super_in_static_initializers.get()
            && self.current_class_element.get().is_some()
            && ast::is_super_property(node)
            && ast::is_identifier(pa.name())
            && is_static_property_declaration_or_class_static_block(self.current_class_element.get().unwrap())
            && self.lexical_environment.get().is_some_and(|l| l.data.get().is_some())
        {
            let data = self.lexical_environment.get().unwrap().data.get().unwrap();
            if data.facts.get().intersects(classFacts::ClassWasDecorated) {
                return self.visit_invalid_super_property(node);
            }
            if let (Some(class_constructor), Some(super_class_reference)) = (data.class_constructor.get(), data.super_class_reference.get()) {
                // converts `super.x` into `Reflect.get(_baseTemp, "x", _classTemp)`
                let super_property = self.factory().new_reflect_get_call(super_class_reference, self.factory().new_string_literal_from_node(pa.name()), class_constructor);
                self.emit_context().set_original(super_property, pa.expression);
                super_property.set_loc(pa.expression.loc());
                return Some(super_property);
            }
        }
        // Visit only the expression, not the name (when it's a regular identifier), to prevent
        // substitution of property names. Strada's onSubstituteNode only fires for
        // EmitHint.Expression, which excludes the .name of PropertyAccessExpression.
        // Private identifier names are still visited through VisitEachChild so they can be
        // transformed by visitPrivateIdentifier.
        if ast::is_identifier(pa.name()) {
            return Some(self.visit_property_access_expression_for_substitution(node));
        }
        self.visitor().visit_each_child(Some(node))
    }

    // classfields.go:1108
    // visitPropertyAccessExpressionForSubstitution visits only the expression of a PropertyAccessExpression,
    // leaving the name unchanged. This prevents the name from being treated as a standalone identifier
    // reference and incorrectly substituted with a class alias.
    pub(crate) fn visit_property_access_expression_for_substitution(&self, node: P<Node>) -> P<Node> {
        let pa = node.as_property_access_expression();
        let expression = self.visitor().visit_node(Some(pa.expression)).unwrap();
        if expression != pa.expression {
            return self.factory().update_property_access_expression(node, expression, pa.question_dot_token(), pa.name(), node.flags());
        }
        node
    }

    // classfields.go:1116
    pub(crate) fn visit_element_access_expression(&self, node: P<Node>) -> Option<P<Node>> {
        let ea = node.as_element_access_expression();
        if self.should_transform_super_in_static_initializers.get()
            && self.current_class_element.get().is_some()
            && ast::is_super_property(node)
            && is_static_property_declaration_or_class_static_block(self.current_class_element.get().unwrap())
            && self.lexical_environment.get().is_some_and(|l| l.data.get().is_some())
        {
            let data = self.lexical_environment.get().unwrap().data.get().unwrap();
            if data.facts.get().intersects(classFacts::ClassWasDecorated) {
                return self.visit_invalid_super_property(node);
            }
            if let (Some(class_constructor), Some(super_class_reference)) = (data.class_constructor.get(), data.super_class_reference.get()) {
                // converts `super[x]` into `Reflect.get(_baseTemp, x, _classTemp)`
                let super_property = self.factory().new_reflect_get_call(super_class_reference, self.visitor().visit_node(Some(ea.argument_expression)).unwrap(), class_constructor);
                self.emit_context().set_original(super_property, ea.expression);
                super_property.set_loc(ea.expression.loc());
                return Some(super_property);
            }
        }
        self.visitor().visit_each_child(Some(node))
    }

    // classfields.go:1140
    pub(crate) fn visit_pre_or_postfix_unary_expression(&self, node: P<Node>, discarded: bool) -> Option<P<Node>> {
        let operator: Kind;
        let operand: P<Node>;
        if ast::is_prefix_unary_expression(node) {
            operator = node.as_prefix_unary_expression().operator;
            operand = node.as_prefix_unary_expression().operand;
        } else {
            operator = node.as_postfix_unary_expression().operator;
            operand = node.as_postfix_unary_expression().operand;
        }

        if operator == Kind::PlusPlusToken || operator == Kind::MinusMinusToken {
            let operand_skipped = ast::skip_parentheses(operand);

            // Private identifier property access
            if ast::is_property_access_expression(operand_skipped) && ast::is_private_identifier(operand_skipped.name().unwrap()) {
                let info = self.access_private_identifier(operand_skipped.name().unwrap());
                if let Some(info) = info {
                    let receiver = self.visitor().visit_node(operand_skipped.expression()).unwrap();
                    let (read_expression, initialize_expression) = self.create_copiable_receiver_expr(receiver);

                    let mut expression = self.create_private_identifier_access_helper(info, read_expression);
                    let mut temp: Option<P<Node>> = None;
                    if !ast::is_prefix_unary_expression(node) && !discarded {
                        temp = Some(self.factory().new_temp_variable());
                        self.emit_context().add_variable_declaration(temp.unwrap());
                    }
                    expression = expand_pre_or_postfix_increment_or_decrement_expression(self.factory(), self.emit_context(), node, expression, temp);
                    let mut assign_receiver = read_expression;
                    if let Some(initialize_expression) = initialize_expression {
                        assign_receiver = initialize_expression;
                    }
                    expression = self.create_private_identifier_assignment(info, assign_receiver, expression, Kind::EqualsToken);
                    self.emit_context().set_original(expression, node);
                    expression.set_loc(node.loc());
                    if let Some(temp) = temp {
                        expression = self.factory().new_comma_expression(expression, temp);
                        expression.set_loc(node.loc());
                    }
                    return Some(expression);
                }
            } else if self.should_transform_super_in_static_initializers.get()
                && self.current_class_element.get().is_some()
                && ast::is_super_property(operand_skipped)
                && is_static_property_declaration_or_class_static_block(self.current_class_element.get().unwrap())
                && self.lexical_environment.get().is_some_and(|l| l.data.get().is_some())
            {
                // converts `++super.a` into `(Reflect.set(_baseTemp, "a", (_a = Reflect.get(_baseTemp, "a", _classTemp), _b = ++_a), _classTemp), _b)`
                // converts `++super[f()]` into `(Reflect.set(_baseTemp, _a = f(), (_b = Reflect.get(_baseTemp, _a, _classTemp), _c = ++_b), _classTemp), _c)`
                // converts `--super.a` into `(Reflect.set(_baseTemp, "a", (_a = Reflect.get(_baseTemp, "a", _classTemp), _b = --_a), _classTemp), _b)`
                // converts `--super[f()]` into `(Reflect.set(_baseTemp, _a = f(), (_b = Reflect.get(_baseTemp, _a, _classTemp), _c = --_b), _classTemp), _c)`
                // converts `super.a++` into `(Reflect.set(_baseTemp, "a", (_a = Reflect.get(_baseTemp, "a", _classTemp), _b = _a++), _classTemp), _b)`
                // converts `super[f()]++` into `(Reflect.set(_baseTemp, _a = f(), (_b = Reflect.get(_baseTemp, _a, _classTemp), _c = _b++), _classTemp), _c)`
                // converts `super.a--` into `(Reflect.set(_baseTemp, "a", (_a = Reflect.get(_baseTemp, "a", _classTemp), _b = _a--), _classTemp), _b)`
                // converts `super[f()]--` into `(Reflect.set(_baseTemp, _a = f(), (_b = Reflect.get(_baseTemp, _a, _classTemp), _c = _b--), _classTemp), _c)`
                let data = self.lexical_environment.get().unwrap().data.get().unwrap();
                if data.facts.get().intersects(classFacts::ClassWasDecorated) {
                    let visited_expr = self.visit_invalid_super_property(operand_skipped).unwrap();
                    if ast::is_prefix_unary_expression(node) {
                        return Some(self.factory().update_prefix_unary_expression(node, node.as_prefix_unary_expression().operator, visited_expr));
                    }
                    return Some(self.factory().update_postfix_unary_expression(node, visited_expr, node.as_postfix_unary_expression().operator));
                }
                if let (Some(class_constructor), Some(super_class_reference)) = (data.class_constructor.get(), data.super_class_reference.get()) {
                    let mut setter_name: Option<P<Node>> = None;
                    let mut getter_name: Option<P<Node>> = None;
                    if ast::is_property_access_expression(operand_skipped) {
                        if ast::is_identifier(operand_skipped.name().unwrap()) {
                            getter_name = Some(self.factory().new_string_literal_from_node(operand_skipped.name().unwrap()));
                            setter_name = getter_name;
                        }
                    } else if ast::is_element_access_expression(operand_skipped) {
                        let argument_expression = operand_skipped.as_element_access_expression().argument_expression;
                        if is_simple_inlineable_expression(argument_expression) {
                            getter_name = Some(argument_expression);
                            setter_name = getter_name;
                        } else {
                            getter_name = Some(self.factory().new_temp_variable());
                            self.emit_context().add_variable_declaration(getter_name.unwrap());
                            setter_name = Some(self.factory().new_assignment_expression(getter_name.unwrap(), self.visitor().visit_node(Some(argument_expression)).unwrap()));
                        }
                    }
                    if let (Some(setter_name), Some(getter_name)) = (setter_name, getter_name) {
                        let mut expression = self.factory().new_reflect_get_call(super_class_reference, getter_name, class_constructor);
                        expression.set_loc(operand_skipped.loc());

                        let mut temp: Option<P<Node>> = None;
                        if !discarded {
                            temp = Some(self.factory().new_temp_variable());
                            self.emit_context().add_variable_declaration(temp.unwrap());
                        }
                        expression = expand_pre_or_postfix_increment_or_decrement_expression(self.factory(), self.emit_context(), node, expression, temp);
                        expression = self.factory().new_reflect_set_call(super_class_reference, setter_name, expression, class_constructor);
                        self.emit_context().set_original(expression, node);
                        expression.set_loc(node.loc());
                        if let Some(temp) = temp {
                            expression = self.factory().new_comma_expression(expression, temp);
                            expression.set_loc(node.loc());
                        }
                        return Some(expression);
                    }
                }
            }
        }
        self.visitor().visit_each_child(Some(node))
    }
}

// TEMP(part 1 in progress): not yet ported part-1 functions.
impl classFieldsTransformer {
    pub(crate) fn visit_for_statement(&self, node: P<Node>) -> Option<P<Node>> { let _ = node; todo!() }
    pub(crate) fn visit_expression_statement(&self, node: P<Node>) -> Option<P<Node>> { let _ = node; todo!() }
    pub(crate) fn visit_call_expression(&self, node: P<Node>) -> Option<P<Node>> { let _ = node; todo!() }
    pub(crate) fn visit_tagged_template_expression(&self, node: P<Node>) -> Option<P<Node>> { let _ = node; todo!() }
    pub(crate) fn visit_binary_expression(&self, node: P<Node>, discarded: bool) -> Option<P<Node>> { let _ = (node, discarded); todo!() }
    pub(crate) fn visit_parenthesized_expression(&self, node: P<Node>, discarded: bool) -> Option<P<Node>> { let _ = (node, discarded); todo!() }
    pub(crate) fn is_anonymous_class_needing_assigned_name_worker(&self, node: P<Node>) -> bool { let _ = node; todo!() }
    pub(crate) fn visit_expression_with_type_arguments_in_heritage_clause(&self, node: P<Node>) -> Option<P<Node>> { let _ = node; todo!() }
    pub(crate) fn create_copiable_receiver_expr(&self, receiver: P<Node>) -> (P<Node>, Option<P<Node>>) { let _ = receiver; todo!() }
    pub(crate) fn create_private_identifier_assignment(&self, info: P<privateIdentifierInfo>, receiver: P<Node>, right: P<Node>, operator: Kind) -> P<Node> { let _ = (info, receiver, right, operator); todo!() }
}
