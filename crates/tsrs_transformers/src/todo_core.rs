// TODO(emit/core): stand-ins for what wave E1 (`emit/core`) ports: transformer.go/chain.go/utilities.go/
// modifiervisitor.go of package `transformers`, the factory.go helpers and helpers.go definitions of package
// `printer` that the tstransforms and module transforms call, the `printer.EmitResolver` methods missing from
// `tsrs_declarations::Resolver`, and `binder.ReferenceResolver` as an interface value. Each item is a faithful port
// of the Go function it names, so when `emit/core` lands, this file is deleted and the call sites resolve to E1's
// versions (the factory and resolver helpers are extension traits, so inherent methods of the same name win).
//
// Also here: a few `ast` utilities that tsrs_ast does not have yet (Go ast/utilities.go).

use crate::*;
use std::sync::OnceLock;

pub use tsrs_declarations::{is_original_node_single_line, is_simple_copiable_expression, is_simple_inlineable_expression, Resolver, Transformer};

// ---------------------------------------------------------------------------------------------------------------
// chain.go

/// Go `transformers.TransformOptions`.
#[derive(Clone)]
pub struct TransformOptions {
    pub context: P<EmitContext>,
    pub compiler_options: P<CompilerOptions>,
    pub resolver: ReferenceResolverRef,
    pub emit_resolver: Option<Resolver>,
    pub get_emit_module_format_of_file: Rc<dyn Fn(P<SourceFile>) -> ModuleKind>,
}

/// Go `transformers.TransformerFactory`.
pub type TransformerFactory = fn(&TransformOptions) -> Option<P<Transformer>>;

pub struct chainedTransformer {
    pub base: Transformer,
    pub components: Vec<P<Transformer>>,
}

impl chainedTransformer {
    // chain.go:15
    fn visit(&self, node: P<Node>) -> P<Node> {
        if node.kind() != Kind::SourceFile {
            panic!("Chained transform passed non-sourcefile initial node");
        }
        let mut result = node.as_source_file_p();
        for t in &self.components {
            result = t.transform_source_file(result);
        }
        result.as_node()
    }
}

// chain.go:38
pub fn chain(transforms: &[TransformerFactory], opt: &TransformOptions) -> Option<P<Transformer>> {
    if transforms.len() < 2 {
        if transforms.is_empty() {
            panic!("Expected some number of transforms to chain, but got none");
        }
        return transforms[0](opt);
    }
    let mut constructed: Vec<P<Transformer>> = Vec::with_capacity(transforms.len());
    for t in transforms {
        // TODO: flatten nested chains?
        if let Some(result) = t(opt) {
            constructed.push(result);
        }
    }
    match constructed.len() {
        0 => return None,
        1 => return Some(constructed[0]),
        _ => {}
    }
    let ch = P::new(chainedTransformer { base: Transformer::default(), components: constructed });
    ch.base.new_transformer(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| Some(ch.visit(n))), Some(opt.context));
    Some(P::from_static(&ch.get().base))
}

// ---------------------------------------------------------------------------------------------------------------
// binder.ReferenceResolver as an interface value (Go `getScriptTransformers` passes the emit resolver, or a plain
// binder reference resolver without hooks when no checker information is needed).

#[derive(Clone, Copy)]
pub enum ReferenceResolverRef {
    Emit(Resolver),
    Binder(P<tsrs_binder::ReferenceResolver<()>>),
}

impl ReferenceResolverRef {
    pub fn new_binder(options: P<CompilerOptions>) -> ReferenceResolverRef {
        ReferenceResolverRef::Binder(tsrs_binder::new_reference_resolver(options, tsrs_binder::ReferenceResolverHooks::default()))
    }

    pub fn get_referenced_export_container(&self, node: P<Node>, prefix_locals: bool) -> Option<P<Node>> {
        match self {
            ReferenceResolverRef::Emit(r) => r.lock(|c| r.r.get_referenced_export_container(c, node, prefix_locals)),
            ReferenceResolverRef::Binder(r) => r.get_referenced_export_container(&mut (), node, prefix_locals),
        }
    }

    pub fn get_referenced_import_declaration(&self, node: P<Node>) -> Option<P<Node>> {
        match self {
            ReferenceResolverRef::Emit(r) => r.lock(|c| r.r.get_referenced_import_declaration(c, node)),
            ReferenceResolverRef::Binder(r) => r.get_referenced_import_declaration(&mut (), node),
        }
    }

    pub fn get_referenced_value_declaration(&self, node: P<Node>) -> Option<P<Node>> {
        match self {
            ReferenceResolverRef::Emit(r) => r.lock(|c| r.r.get_referenced_value_declaration(c, node)),
            ReferenceResolverRef::Binder(r) => r.get_referenced_value_declaration(&mut (), node),
        }
    }

    pub fn get_referenced_value_declarations(&self, node: P<Node>) -> Vec<P<Node>> {
        match self {
            ReferenceResolverRef::Emit(r) => r.lock(|c| r.r.get_referenced_value_declarations(c, node)),
            ReferenceResolverRef::Binder(r) => r.get_referenced_value_declarations(&mut (), node),
        }
    }
}

/// The `printer.EmitResolver` methods the script transformers call that `tsrs_declarations::Resolver` lacks
/// (emitresolver.go; each locks the checker like Go's exported methods).
pub trait ResolverExt {
    fn mark_linked_references_recursively(&self, file: P<SourceFile>);
    fn is_referenced_alias_declaration(&self, node: P<Node>) -> bool;
    fn is_value_alias_declaration(&self, node: P<Node>) -> bool;
    fn is_top_level_value_import_equals_with_entity_name(&self, node: P<Node>) -> bool;
    fn get_constant_value(&self, node: P<Node>) -> Option<tsrs_checker::LiteralValue>;
    fn get_external_module_file_from_declaration(&self, declaration: P<Node>) -> Option<P<SourceFile>>;
}

impl ResolverExt for Resolver {
    fn mark_linked_references_recursively(&self, file: P<SourceFile>) {
        self.lock(|c| self.r.mark_linked_references_recursively(c, Some(file)))
    }
    fn is_referenced_alias_declaration(&self, node: P<Node>) -> bool {
        self.lock(|c| self.r.is_referenced_alias_declaration(c, node))
    }
    fn is_value_alias_declaration(&self, node: P<Node>) -> bool {
        self.lock(|c| self.r.is_value_alias_declaration(c, node))
    }
    fn is_top_level_value_import_equals_with_entity_name(&self, node: P<Node>) -> bool {
        self.lock(|c| self.r.is_top_level_value_import_equals_with_entity_name(c, node))
    }
    fn get_constant_value(&self, node: P<Node>) -> Option<tsrs_checker::LiteralValue> {
        self.lock(|c| self.r.get_constant_value(c, node))
    }
    fn get_external_module_file_from_declaration(&self, declaration: P<Node>) -> Option<P<SourceFile>> {
        self.lock(|c| self.r.get_external_module_file_from_declaration(c, declaration))
    }
}

// ---------------------------------------------------------------------------------------------------------------
// utilities.go

// utilities.go:12
pub fn is_generated_identifier(emit_context: P<EmitContext>, name: P<Node>) -> bool {
    emit_context.has_auto_generate_info(Some(name))
}

// utilities.go:16
pub fn is_helper_name(emit_context: P<EmitContext>, name: P<Node>) -> bool {
    emit_context.emit_flags(name).intersects(EmitFlags::HelperName)
}

// utilities.go:20
pub fn is_local_name(emit_context: P<EmitContext>, name: P<Node>) -> bool {
    emit_context.emit_flags(name).intersects(EmitFlags::LocalName)
}

// utilities.go:24
pub fn is_export_name(emit_context: P<EmitContext>, name: P<Node>) -> bool {
    emit_context.emit_flags(name).intersects(EmitFlags::ExportName)
}

// utilities.go:28
pub fn is_identifier_reference(name: P<Node>, parent: P<Node>) -> bool {
    match parent.kind() {
        Kind::BinaryExpression
        | Kind::PrefixUnaryExpression
        | Kind::PostfixUnaryExpression
        | Kind::YieldExpression
        | Kind::AsExpression
        | Kind::SatisfiesExpression
        | Kind::ElementAccessExpression
        | Kind::NonNullExpression
        | Kind::SpreadElement
        | Kind::SpreadAssignment
        | Kind::ParenthesizedExpression
        | Kind::ArrayLiteralExpression
        | Kind::DeleteExpression
        | Kind::TypeOfExpression
        | Kind::VoidExpression
        | Kind::AwaitExpression
        | Kind::TypeAssertionExpression
        | Kind::ExpressionWithTypeArguments
        | Kind::JsxSelfClosingElement
        | Kind::JsxSpreadAttribute
        | Kind::JsxExpression
        | Kind::PartiallyEmittedExpression => {
            // all immediate children that can be `Identifier` would be instances of `IdentifierReference`
            true
        }
        Kind::ComputedPropertyName
        | Kind::Decorator
        | Kind::IfStatement
        | Kind::DoStatement
        | Kind::WhileStatement
        | Kind::WithStatement
        | Kind::ReturnStatement
        | Kind::SwitchStatement
        | Kind::CaseClause
        | Kind::ThrowStatement
        | Kind::ExpressionStatement
        | Kind::ExportAssignment
        | Kind::PropertyAccessExpression
        | Kind::TemplateSpan => {
            // only an `Expression()` child that can be `Identifier` would be an instance of `IdentifierReference`
            parent.expression() == Some(name)
        }
        Kind::VariableDeclaration
        | Kind::Parameter
        | Kind::BindingElement
        | Kind::PropertyDeclaration
        | Kind::PropertySignature
        | Kind::PropertyAssignment
        | Kind::EnumMember
        | Kind::JsxAttribute => {
            // only an `Initializer()` child that can be `Identifier` would be an instance of `IdentifierReference`
            parent.initializer() == Some(name)
        }
        Kind::ShorthandPropertyAssignment => parent.as_shorthand_property_assignment().object_assignment_initializer() == Some(name),
        Kind::ForStatement => {
            parent.initializer() == Some(name) || parent.as_for_statement().condition == Some(name) || parent.as_for_statement().incrementor == Some(name)
        }
        Kind::ForInStatement | Kind::ForOfStatement => parent.initializer() == Some(name) || parent.expression() == Some(name),
        Kind::ImportEqualsDeclaration => parent.as_import_equals_declaration().module_reference == name,
        Kind::ArrowFunction => parent.body() == Some(name),
        Kind::ConditionalExpression => {
            let c = parent.as_conditional_expression();
            c.condition == name || c.when_true == name || c.when_false == name
        }
        Kind::CallExpression | Kind::NewExpression => parent.expression() == Some(name) || parent.arguments().contains(&name),
        Kind::TaggedTemplateExpression => parent.as_tagged_template_expression().tag == name,
        Kind::ImportAttribute => parent.as_import_attribute().value == name,
        Kind::JsxOpeningElement | Kind::JsxClosingElement => parent.tag_name() == name,
        _ => false,
    }
}

// utilities.go:112
fn convert_binding_element_to_array_assignment_element(emit_context: P<EmitContext>, element: P<Node>) -> P<Node> {
    let f = &emit_context.get().factory;
    let Some(name) = element.name() else {
        let elision = f.new_omitted_expression();
        emit_context.set_original(elision, element);
        emit_context.assign_comment_and_source_map_ranges(elision, element);
        return elision;
    };
    let expression = convert_binding_name_to_assignment_element_target(emit_context, name);
    let e = element.as_binding_element();
    if e.dot_dot_dot_token().is_some() {
        let spread = f.new_spread_element(expression);
        emit_context.set_original(spread, element);
        emit_context.assign_comment_and_source_map_ranges(spread, element);
        return spread;
    }
    if let Some(initializer) = e.initializer() {
        let assignment = f.new_assignment_expression(expression, initializer);
        emit_context.set_original(assignment, element);
        emit_context.assign_comment_and_source_map_ranges(assignment, element);
        return assignment;
    }
    expression
}

// utilities.go:135
fn convert_binding_element_to_object_assignment_element(emit_context: P<EmitContext>, element: P<Node>) -> P<Node> {
    let f = &emit_context.get().factory;
    let e = element.as_binding_element();
    if e.dot_dot_dot_token().is_some() {
        let spread = f.new_spread_assignment(element.name().unwrap());
        emit_context.set_original(spread, element);
        emit_context.assign_comment_and_source_map_ranges(spread, element);
        return spread;
    }
    if let Some(property_name) = e.property_name() {
        let mut expression = convert_binding_name_to_assignment_element_target(emit_context, element.name().unwrap());
        if let Some(initializer) = e.initializer() {
            expression = f.new_assignment_expression(expression, initializer);
        }
        let assignment = f.new_property_assignment(None /*modifiers*/, property_name, None /*postfixToken*/, None /*typeNode*/, expression);
        emit_context.set_original(assignment, element);
        emit_context.assign_comment_and_source_map_ranges(assignment, element);
        return assignment;
    }
    let equals_token = if e.initializer().is_some() { Some(f.new_token(Kind::EqualsToken)) } else { None };
    let assignment = f.new_shorthand_property_assignment(
        None, /*modifiers*/
        element.name().unwrap(),
        None, /*postfixToken*/
        None, /*typeNode*/
        equals_token,
        e.initializer(),
    );
    emit_context.set_original(assignment, element);
    emit_context.assign_comment_and_source_map_ranges(assignment, element);
    assignment
}

// utilities.go:169
pub fn convert_binding_pattern_to_assignment_pattern(emit_context: P<EmitContext>, element: P<Node>) -> P<Node> {
    match element.kind() {
        Kind::ArrayBindingPattern => convert_binding_element_to_array_assignment_pattern(emit_context, element),
        Kind::ObjectBindingPattern => convert_binding_element_to_object_assignment_pattern(emit_context, element),
        _ => panic!("Unknown binding pattern"),
    }
}

// utilities.go:180
fn convert_binding_element_to_object_assignment_pattern(emit_context: P<EmitContext>, element: P<Node>) -> P<Node> {
    let f = &emit_context.get().factory;
    let mut properties: Vec<P<Node>> = Vec::new();
    let elements = element.as_binding_pattern().elements;
    for &e in elements.nodes() {
        properties.push(convert_binding_element_to_object_assignment_element(emit_context, e));
    }
    let property_list = f.new_node_list(properties);
    property_list.loc.set(elements.loc.get());
    let object = f.new_object_literal_expression(property_list, false /*multiLine*/);
    emit_context.set_original(object, element);
    emit_context.assign_comment_and_source_map_ranges(object, element);
    object
}

// utilities.go:193
fn convert_binding_element_to_array_assignment_pattern(emit_context: P<EmitContext>, element: P<Node>) -> P<Node> {
    let f = &emit_context.get().factory;
    let mut out: Vec<P<Node>> = Vec::new();
    let elements = element.as_binding_pattern().elements;
    for &e in elements.nodes() {
        out.push(convert_binding_element_to_array_assignment_element(emit_context, e));
    }
    let element_list = f.new_node_list(out);
    element_list.loc.set(elements.loc.get());
    let object = f.new_array_literal_expression(element_list, false /*multiLine*/);
    emit_context.set_original(object, element);
    emit_context.assign_comment_and_source_map_ranges(object, element);
    object
}

// utilities.go:206
fn convert_binding_name_to_assignment_element_target(emit_context: P<EmitContext>, element: P<Node>) -> P<Node> {
    if ast::is_binding_pattern(element) {
        return convert_binding_pattern_to_assignment_pattern(emit_context, element);
    }
    element
}

// utilities.go:213
pub fn convert_variable_declaration_to_assignment_expression(emit_context: P<EmitContext>, element: P<Node>) -> Option<P<Node>> {
    let initializer = element.initializer()?;
    let expression = convert_binding_name_to_assignment_element_target(emit_context, element.name().unwrap());
    let assignment = emit_context.factory.new_assignment_expression(expression, initializer);
    emit_context.set_original(assignment, element);
    emit_context.assign_comment_and_source_map_ranges(assignment, element);
    Some(assignment)
}

// utilities.go:224. Go distinguishes a nil slice (-> nil) from an empty one (-> an empty SyntaxList).
pub fn single_or_many(nodes: Option<Vec<P<Node>>>, factory: &printer::NodeFactory) -> Option<P<Node>> {
    let nodes = nodes?;
    if nodes.len() == 1 {
        return Some(nodes[0]);
    }
    Some(factory.new_syntax_list(alloc_vec(nodes)))
}

// utilities.go:275
pub fn find_super_statement_index_path(statements: &[P<Node>], start: usize) -> Vec<usize> {
    let mut indices = find_super_statement_index_path_worker(statements, start, Vec::new()).unwrap_or_default();
    indices.reverse();
    indices
}

// utilities.go:281
fn find_super_statement_index_path_worker(statements: &[P<Node>], start: usize, indices: Vec<usize>) -> Option<Vec<usize>> {
    for i in start..statements.len() {
        let statement = statements[i];
        if get_super_call_from_statement(statement).is_some() {
            let mut indices = indices;
            indices.push(i);
            return Some(indices);
        } else if ast::is_try_statement(statement) {
            if let Some(mut result) = find_super_statement_index_path_worker(statement.as_try_statement().try_block.statements(), 0, indices.clone()) {
                result.push(i);
                return Some(result);
            }
        }
    }
    None
}

// utilities.go:296
pub fn get_super_call_from_statement(statement: P<Node>) -> Option<P<Node>> {
    if !ast::is_expression_statement(statement) {
        return None;
    }
    let expression = ast::skip_parentheses(statement.expression().unwrap());
    if is_super_call(expression) {
        return Some(expression);
    }
    None
}

// ---------------------------------------------------------------------------------------------------------------
// modifiervisitor.go

// modifiervisitor.go:21 (the visitor keeps no state, so a plain NodeVisitor stands in for `modifierVisitor`)
pub fn extract_modifiers(emit_context: P<EmitContext>, modifiers: Option<P<ModifierList>>, allowed: ModifierFlags) -> Option<P<ModifierList>> {
    let modifiers = modifiers?;
    let mut visitor = emit_context.new_node_visitor(Rc::new(move |_: &mut NodeVisitor, node: P<Node>| {
        let flags = ast::modifier_to_flag(node.kind());
        if flags != ModifierFlags::None && !flags.intersects(allowed) {
            return None;
        }
        Some(node)
    }));
    visitor.visit_modifiers(Some(modifiers))
}

// ---------------------------------------------------------------------------------------------------------------
// printer/factory.go

#[derive(Clone, Copy, Default)]
pub struct NameOptions {
    pub allow_comments: bool,    // indicates whether comments may be emitted for the name.
    pub allow_source_maps: bool, // indicates whether source maps may be emitted for the name.
}

#[derive(Clone, Copy, Default)]
pub struct AssignedNameOptions {
    pub allow_comments: bool,       // indicates whether comments may be emitted for the name.
    pub allow_source_maps: bool,    // indicates whether source maps may be emitted for the name.
    pub ignore_assigned_name: bool, // indicates whether the assigned name of a declaration shouldn't be considered.
}

fn factory_emit_context(f: &printer::NodeFactory) -> P<EmitContext> {
    f.emit_context()
}

pub trait FactoryExt {
    fn new_this_expression(&self) -> P<Node>;
    fn new_true_expression(&self) -> P<Node>;
    fn new_false_expression(&self) -> P<Node>;
    fn new_comma_expression(&self, left: P<Node>, right: P<Node>) -> P<Node>;
    fn new_logical_or_expression(&self, left: P<Node>, right: P<Node>) -> P<Node>;
    fn new_logical_and_expression(&self, left: P<Node>, right: P<Node>) -> P<Node>;
    fn new_strict_inequality_expression(&self, left: P<Node>, right: P<Node>) -> P<Node>;
    fn inline_expressions(&self, expressions: &[P<Node>]) -> Option<P<Node>>;
    fn create_expression_from_entity_name(&self, node: P<Node>) -> P<Node>;
    fn new_method_call(&self, object: P<Node>, method_name: P<Node>, arguments_list: Vec<P<Node>>) -> P<Node>;
    fn new_array_slice_call(&self, array: P<Node>, start: usize) -> P<Node>;
    fn ensure_use_strict(&self, statements: &[P<Node>]) -> Vec<P<Node>>;
    fn split_standard_prologue(&self, source: &'static [P<Node>]) -> (&'static [P<Node>], &'static [P<Node>]);
    fn split_custom_prologue(&self, source: &'static [P<Node>]) -> (&'static [P<Node>], &'static [P<Node>]);
    fn get_local_name(&self, node: P<Node>) -> P<Node>;
    fn get_local_name_ex(&self, node: P<Node>, opts: AssignedNameOptions) -> P<Node>;
    fn get_export_name(&self, node: P<Node>) -> P<Node>;
    fn get_export_name_ex(&self, node: P<Node>, opts: AssignedNameOptions) -> P<Node>;
    fn get_declaration_name(&self, node: P<Node>) -> P<Node>;
    fn get_declaration_name_ex(&self, node: P<Node>, opts: NameOptions) -> P<Node>;
    fn get_namespace_member_name(&self, ns: P<Node>, name: P<Node>, opts: NameOptions) -> P<Node>;
    fn get_external_module_or_namespace_export_name(&self, ns: Option<P<Node>>, node: P<Node>, allow_comments: bool, allow_source_maps: bool) -> P<Node>;
    fn new_unscoped_helper_name(&self, name: &str) -> P<Node>;
    fn new_rest_helper(&self, value: P<Node>, elements: &[P<Node>], computed_temp_variables: &[P<Node>], location: TextRange) -> P<Node>;
    fn new_import_default_helper(&self, expression: P<Node>) -> P<Node>;
    fn new_import_star_helper(&self, expression: P<Node>) -> P<Node>;
    fn new_export_star_helper(&self, module_expression: P<Node>, exports_expression: P<Node>) -> P<Node>;
    fn new_rewrite_relative_import_extensions_helper(&self, first_argument: P<Node>, preserve_jsx: bool) -> P<Node>;
}

// factory.go:245
fn flatten_comma_element(node: P<Node>, expressions: &mut Vec<P<Node>>) {
    if ast::is_binary_expression(node) && ast::node_is_synthesized(node) && node.as_binary_expression().operator_token.kind() == Kind::CommaToken {
        flatten_comma_element(node.as_binary_expression().left, expressions);
        flatten_comma_element(node.as_binary_expression().right(), expressions);
    } else {
        expressions.push(node);
    }
}

// factory.go:255
fn flatten_comma_elements(expressions: &[P<Node>]) -> Vec<P<Node>> {
    let mut result = Vec::new();
    for &expression in expressions {
        flatten_comma_element(expression, &mut result);
    }
    result
}

// factory.go:496
fn get_name(f: &printer::NodeFactory, node: Option<P<Node>>, mut emit_flags: EmitFlags, opts: AssignedNameOptions) -> P<Node> {
    let mut node_name: Option<P<Node>> = None;
    if let Some(node) = node {
        if opts.ignore_assigned_name {
            node_name = ast::get_non_assigned_name_of_declaration(node);
        } else {
            node_name = ast::get_name_of_declaration(node);
        }
    }

    if let Some(node_name) = node_name {
        let name = node_name.clone_node(f);
        if !opts.allow_comments {
            emit_flags |= EmitFlags::NoComments;
        }
        if !opts.allow_source_maps {
            emit_flags |= EmitFlags::NoSourceMap;
        }
        factory_emit_context(f).add_emit_flags(name, emit_flags);
        return name;
    }

    f.new_generated_name_for_node(node.unwrap())
}

impl FactoryExt for printer::NodeFactory {
    // factory.go:193
    fn new_this_expression(&self) -> P<Node> {
        self.new_keyword_expression(Kind::ThisKeyword)
    }

    // factory.go:197
    fn new_true_expression(&self) -> P<Node> {
        self.new_keyword_expression(Kind::TrueKeyword)
    }

    // factory.go:201
    fn new_false_expression(&self) -> P<Node> {
        self.new_keyword_expression(Kind::FalseKeyword)
    }

    // factory.go:209
    fn new_comma_expression(&self, left: P<Node>, right: P<Node>) -> P<Node> {
        self.new_binary_expression(None /*modifiers*/, left, None /*typeNode*/, self.new_token(Kind::CommaToken), right)
    }

    // factory.go:217
    fn new_logical_or_expression(&self, left: P<Node>, right: P<Node>) -> P<Node> {
        self.new_binary_expression(None /*modifiers*/, left, None /*typeNode*/, self.new_token(Kind::BarBarToken), right)
    }

    // factory.go:221
    fn new_logical_and_expression(&self, left: P<Node>, right: P<Node>) -> P<Node> {
        self.new_binary_expression(None /*modifiers*/, left, None /*typeNode*/, self.new_token(Kind::AmpersandAmpersandToken), right)
    }

    // factory.go:233
    fn new_strict_inequality_expression(&self, left: P<Node>, right: P<Node>) -> P<Node> {
        self.new_binary_expression(None /*modifiers*/, left, None /*typeNode*/, self.new_token(Kind::ExclamationEqualsEqualsToken), right)
    }

    // factory.go:264
    fn inline_expressions(&self, expressions: &[P<Node>]) -> Option<P<Node>> {
        if expressions.is_empty() {
            return None;
        }
        if expressions.len() == 1 {
            return Some(expressions[0]);
        }
        let expressions = flatten_comma_elements(expressions);
        let mut expression = expressions[0];
        for &next in &expressions[1..] {
            expression = self.new_comma_expression(expression, next);
        }
        Some(expression)
    }

    // factory.go:283
    fn create_expression_from_entity_name(&self, node: P<Node>) -> P<Node> {
        if ast::is_qualified_name(node) {
            let q = node.as_qualified_name();
            let left = self.create_expression_from_entity_name(q.left);
            let right = q.right.clone_node(self);
            right.set_loc(q.right.loc());
            // TODO(rbuckton): Does this need to be parented?
            right.set_parent(q.right.parent());
            let prop_access = self.new_property_access_expression(left, None, right, NodeFlags::None);
            prop_access.set_loc(node.loc());
            return prop_access;
        }
        let res = node.clone_node(self);
        res.set_loc(node.loc());
        // TODO(rbuckton): Does this need to be parented?
        res.set_parent(node.parent());
        res
    }

    // factory.go:355
    fn new_method_call(&self, object: P<Node>, method_name: P<Node>, arguments_list: Vec<P<Node>>) -> P<Node> {
        // Preserve the optionality of `object`.
        if ast::is_call_expression(object) && object.flags().intersects(NodeFlags::OptionalChain) {
            return self.new_call_expression(
                self.new_property_access_expression(object, None, method_name, NodeFlags::None),
                None,
                None,
                self.new_node_list(arguments_list),
                NodeFlags::OptionalChain,
            );
        }
        self.new_call_expression(self.new_property_access_expression(object, None, method_name, NodeFlags::None), None, None, self.new_node_list(arguments_list), NodeFlags::None)
    }

    // factory.go:387
    fn new_array_slice_call(&self, array: P<Node>, start: usize) -> P<Node> {
        let mut args = Vec::new();
        if start != 0 {
            args.push(self.new_numeric_literal(alloc_str(&start.to_string()), TokenFlags::None));
        }
        self.new_method_call(array, self.new_identifier("slice"), args)
    }

    // factory.go:448
    fn ensure_use_strict(&self, statements: &[P<Node>]) -> Vec<P<Node>> {
        if let Some(&statement) = statements.first() {
            if ast::is_prologue_directive(statement) && statement.expression().unwrap().text() == "use strict" {
                return statements.to_vec();
            }
        }
        let use_strict_prologue = self.new_expression_statement(self.new_string_literal("use strict", TokenFlags::None));
        let mut result = Vec::with_capacity(statements.len() + 1);
        result.push(use_strict_prologue);
        result.extend_from_slice(statements);
        result
    }

    // factory.go:462
    fn split_standard_prologue(&self, source: &'static [P<Node>]) -> (&'static [P<Node>], &'static [P<Node>]) {
        for (i, &statement) in source.iter().enumerate() {
            if !ast::is_prologue_directive(statement) {
                return (&source[..i], &source[i..]);
            }
        }
        (source, &[])
    }

    // factory.go:472
    fn split_custom_prologue(&self, source: &'static [P<Node>]) -> (&'static [P<Node>], &'static [P<Node>]) {
        let emit_context = factory_emit_context(self);
        for (i, &statement) in source.iter().enumerate() {
            if ast::is_prologue_directive(statement) || !emit_context.emit_flags(statement).intersects(EmitFlags::CustomPrologue) {
                return (&source[..i], &source[i..]);
            }
        }
        (&[], source)
    }

    // factory.go:524
    fn get_local_name(&self, node: P<Node>) -> P<Node> {
        self.get_local_name_ex(node, AssignedNameOptions::default())
    }

    // factory.go:531
    fn get_local_name_ex(&self, node: P<Node>, opts: AssignedNameOptions) -> P<Node> {
        get_name(self, Some(node), EmitFlags::LocalName, opts)
    }

    // factory.go:539
    fn get_export_name(&self, node: P<Node>) -> P<Node> {
        self.get_export_name_ex(node, AssignedNameOptions::default())
    }

    // factory.go:547
    fn get_export_name_ex(&self, node: P<Node>, opts: AssignedNameOptions) -> P<Node> {
        get_name(self, Some(node), EmitFlags::ExportName, opts)
    }

    // factory.go:552
    fn get_declaration_name(&self, node: P<Node>) -> P<Node> {
        self.get_declaration_name_ex(node, NameOptions::default())
    }

    // factory.go:557
    fn get_declaration_name_ex(&self, node: P<Node>, opts: NameOptions) -> P<Node> {
        get_name(self, Some(node), EmitFlags::None, AssignedNameOptions { allow_comments: opts.allow_comments, allow_source_maps: opts.allow_source_maps, ignore_assigned_name: false })
    }

    // factory.go:561
    fn get_namespace_member_name(&self, ns: P<Node>, name: P<Node>, opts: NameOptions) -> P<Node> {
        let emit_context = factory_emit_context(self);
        let mut name = name;
        if !emit_context.has_auto_generate_info(Some(name)) {
            name = name.clone_node(self);
        }
        let qualified_name = self.new_property_access_expression(ns, None /*questionDotToken*/, name, NodeFlags::None);
        emit_context.assign_comment_and_source_map_ranges(qualified_name, name);
        if !opts.allow_comments {
            emit_context.add_emit_flags(qualified_name, EmitFlags::NoComments);
        }
        if !opts.allow_source_maps {
            emit_context.add_emit_flags(qualified_name, EmitFlags::NoSourceMap);
        }
        qualified_name
    }

    // factory.go:580
    fn get_external_module_or_namespace_export_name(&self, ns: Option<P<Node>>, node: P<Node>, allow_comments: bool, allow_source_maps: bool) -> P<Node> {
        if let Some(ns) = ns {
            if ast::has_syntactic_modifier(node, ModifierFlags::Export) {
                let name_opts = NameOptions { allow_comments, allow_source_maps };
                return self.get_namespace_member_name(ns, self.get_declaration_name_ex(node, name_opts), name_opts);
            }
        }
        self.get_export_name_ex(node, AssignedNameOptions { allow_comments, allow_source_maps, ignore_assigned_name: false })
    }

    // factory.go:593
    fn new_unscoped_helper_name(&self, name: &str) -> P<Node> {
        let node = self.new_identifier(alloc_str(name));
        factory_emit_context(self).set_emit_flags(node, EmitFlags::HelperName);
        node
    }

    // factory.go:827
    fn new_rest_helper(&self, value: P<Node>, elements: &[P<Node>], computed_temp_variables: &[P<Node>], location: TextRange) -> P<Node> {
        factory_emit_context(self).request_emit_helper(rest_helper());
        let mut property_names: Vec<P<Node>> = Vec::new();
        let mut computed_temp_variable_offset = 0;
        for (i, &element) in elements.iter().enumerate() {
            if i == elements.len() - 1 {
                break;
            }
            if let Some(property_name) = try_get_property_name_of_binding_or_assignment_element(element) {
                if ast::is_computed_property_name(property_name) {
                    let temp = computed_temp_variables[computed_temp_variable_offset];
                    computed_temp_variable_offset += 1;
                    // typeof _tmp === "symbol" ? _tmp : _tmp + ""
                    property_names.push(self.new_conditional_expression(
                        self.new_type_check(temp, "symbol"),
                        self.new_token(Kind::QuestionToken),
                        temp,
                        self.new_token(Kind::ColonToken),
                        self.new_binary_expression(None, temp, None, self.new_token(Kind::PlusToken), self.new_string_literal("", TokenFlags::None)),
                    ));
                } else {
                    property_names.push(self.new_string_literal_from_node(property_name));
                }
            }
        }
        let prop_names = self.new_array_literal_expression(self.new_node_list(property_names), false);
        prop_names.set_loc(location);
        self.new_call_expression(self.new_unscoped_helper_name("__rest"), None, None, self.new_node_list(vec![value, prop_names]), NodeFlags::None)
    }

    // factory.go:1240
    fn new_import_default_helper(&self, expression: P<Node>) -> P<Node> {
        factory_emit_context(self).request_emit_helper(import_default_helper());
        self.new_call_expression(self.new_unscoped_helper_name("__importDefault"), None, None, self.new_node_list(vec![expression]), NodeFlags::None)
    }

    // factory.go:1252
    fn new_import_star_helper(&self, expression: P<Node>) -> P<Node> {
        factory_emit_context(self).request_emit_helper(import_star_helper());
        self.new_call_expression(self.new_unscoped_helper_name("__importStar"), None, None, self.new_node_list(vec![expression]), NodeFlags::None)
    }

    // factory.go:1264
    fn new_export_star_helper(&self, module_expression: P<Node>, exports_expression: P<Node>) -> P<Node> {
        factory_emit_context(self).request_emit_helper(export_star_helper());
        self.new_call_expression(self.new_unscoped_helper_name("__exportStar"), None, None, self.new_node_list(vec![module_expression, exports_expression]), NodeFlags::None)
    }

    // factory.go:1300
    fn new_rewrite_relative_import_extensions_helper(&self, first_argument: P<Node>, preserve_jsx: bool) -> P<Node> {
        factory_emit_context(self).request_emit_helper(rewrite_relative_import_extensions_helper());
        let arguments = if preserve_jsx { vec![first_argument, self.new_token(Kind::TrueKeyword)] } else { vec![first_argument] };
        self.new_call_expression(self.new_unscoped_helper_name("__rewriteRelativeImportExtension"), None, None, self.new_node_list(arguments), NodeFlags::None)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// printer/helpers.go (the helpers the module transforms and destructuring request; text copied byte for byte)

struct HelperCell(OnceLock<P<EmitHelper>>);

fn helper(cell: &'static HelperCell, make: fn() -> EmitHelper) -> P<EmitHelper> {
    *cell.0.get_or_init(|| P::new(make()))
}

static PRIORITY_CREATE_BINDING: printer::Priority = printer::Priority { value: 1 };
static PRIORITY_SET_MODULE_DEFAULT: printer::Priority = printer::Priority { value: 1 };
static PRIORITY_IMPORT_STAR: printer::Priority = printer::Priority { value: 2 };
static PRIORITY_EXPORT_STAR: printer::Priority = printer::Priority { value: 2 };

// helpers.go:334
pub fn rest_helper() -> P<EmitHelper> {
    static CELL: HelperCell = HelperCell(OnceLock::new());
    helper(&CELL, || EmitHelper {
        name: "typescript:rest",
        import_name: "__rest",
        scoped: false,
        text: "var __rest = (this && this.__rest) || function (s, e) {
    var t = {};
    for (var p in s) if (Object.prototype.hasOwnProperty.call(s, p) && e.indexOf(p) < 0)
        t[p] = s[p];
    if (s != null && typeof Object.getOwnPropertySymbols === \"function\")
        for (var i = 0, p = Object.getOwnPropertySymbols(s); i < p.length; i++) {
            if (e.indexOf(p[i]) < 0 && Object.prototype.propertyIsEnumerable.call(s, p[i]))
                t[p[i]] = s[p[i]];
        }
    return t;
};",
        text_callback: None,
        priority: None,
        dependencies: &[],
    })
}

// helpers.go:471
pub fn create_binding_helper() -> P<EmitHelper> {
    static CELL: HelperCell = HelperCell(OnceLock::new());
    helper(&CELL, || EmitHelper {
        name: "typescript:commonjscreatebinding",
        import_name: "__createBinding",
        scoped: false,
        priority: Some(&PRIORITY_CREATE_BINDING),
        text: "var __createBinding = (this && this.__createBinding) || (Object.create ? (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    var desc = Object.getOwnPropertyDescriptor(m, k);
    if (!desc || (\"get\" in desc ? !m.__esModule : desc.writable || desc.configurable)) {
      desc = { enumerable: true, get: function() { return m[k]; } };
    }
    Object.defineProperty(o, k2, desc);
}) : (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    o[k2] = m[k];
}));",
        text_callback: None,
        dependencies: &[],
    })
}

// helpers.go:489
pub fn set_module_default_helper() -> P<EmitHelper> {
    static CELL: HelperCell = HelperCell(OnceLock::new());
    helper(&CELL, || EmitHelper {
        name: "typescript:commonjscreatevalue",
        import_name: "__setModuleDefault",
        scoped: false,
        priority: Some(&PRIORITY_SET_MODULE_DEFAULT),
        text: "var __setModuleDefault = (this && this.__setModuleDefault) || (Object.create ? (function(o, v) {
    Object.defineProperty(o, \"default\", { enumerable: true, value: v });
}) : function(o, v) {
    o[\"default\"] = v;
});",
        text_callback: None,
        dependencies: &[],
    })
}

// helpers.go:501
pub fn import_star_helper() -> P<EmitHelper> {
    static CELL: HelperCell = HelperCell(OnceLock::new());
    helper(&CELL, || EmitHelper {
        name: "typescript:commonjsimportstar",
        import_name: "__importStar",
        scoped: false,
        dependencies: alloc_vec(vec![create_binding_helper(), set_module_default_helper()]),
        priority: Some(&PRIORITY_IMPORT_STAR),
        text: "var __importStar = (this && this.__importStar) || (function () {
    var ownKeys = function(o) {
        ownKeys = Object.getOwnPropertyNames || function (o) {
            var ar = [];
            for (var k in o) if (Object.prototype.hasOwnProperty.call(o, k)) ar[ar.length] = k;
            return ar;
        };
        return ownKeys(o);
    };
    return function (mod) {
        if (mod && mod.__esModule) return mod;
        var result = {};
        if (mod != null) for (var k = ownKeys(mod), i = 0; i < k.length; i++) if (k[i] !== \"default\") __createBinding(result, mod, k[i]);
        __setModuleDefault(result, mod);
        return result;
    };
})();",
        text_callback: None,
    })
}

// helpers.go:526
pub fn import_default_helper() -> P<EmitHelper> {
    static CELL: HelperCell = HelperCell(OnceLock::new());
    helper(&CELL, || EmitHelper {
        name: "typescript:commonjsimportdefault",
        import_name: "__importDefault",
        scoped: false,
        text: "var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { \"default\": mod };
};",
        text_callback: None,
        priority: None,
        dependencies: &[],
    })
}

// helpers.go:535
pub fn export_star_helper() -> P<EmitHelper> {
    static CELL: HelperCell = HelperCell(OnceLock::new());
    helper(&CELL, || EmitHelper {
        name: "typescript:export-star",
        import_name: "__exportStar",
        scoped: false,
        dependencies: alloc_vec(vec![create_binding_helper()]),
        priority: Some(&PRIORITY_EXPORT_STAR),
        text: "var __exportStar = (this && this.__exportStar) || function(m, exports) {
    for (var p in m) if (p !== \"default\" && !Object.prototype.hasOwnProperty.call(exports, p)) __createBinding(exports, m, p);
};",
        text_callback: None,
    })
}

// helpers.go:546
pub fn rewrite_relative_import_extensions_helper() -> P<EmitHelper> {
    static CELL: HelperCell = HelperCell(OnceLock::new());
    helper(&CELL, || EmitHelper {
        name: "typescript:rewriteRelativeImportExtensions",
        import_name: "__rewriteRelativeImportExtension",
        scoped: false,
        text: "var __rewriteRelativeImportExtension = (this && this.__rewriteRelativeImportExtension) || function (path, preserveJsx) {
    if (typeof path === \"string\" && /^\\.\\.?\\//.test(path)) {
        return path.replace(/\\.(tsx)$|((?:\\.d)?)((?:\\.[^./]+?)?)\\.([cm]?)ts$/i, function (m, tsx, d, ext, cm) {
            return tsx ? preserveJsx ? \".jsx\" : \".js\" : d && (!ext || !cm) ? m : (d + ext + \".\" + cm.toLowerCase() + \"js\");
        });
    }
    return path;
};",
        text_callback: None,
        priority: None,
        dependencies: &[],
    })
}

// ---------------------------------------------------------------------------------------------------------------
// ast/utilities.go functions missing from tsrs_ast

// ast/utilities.go:2138
pub fn is_super_call(node: P<Node>) -> bool {
    ast::is_call_expression(node) && node.expression().unwrap().kind() == Kind::SuperKeyword
}

// ast/utilities.go:1708
pub fn is_export_namespace_as_default_declaration(node: P<Node>) -> bool {
    if ast::is_export_declaration(node) {
        let export_clause = node.as_export_declaration().export_clause.unwrap();
        return ast::is_namespace_export(export_clause) && ast::module_export_name_is_default(export_clause.name().unwrap());
    }
    false
}

// ast/utilities.go:3961
pub fn try_get_property_name_of_binding_or_assignment_element(binding_element: P<Node>) -> Option<P<Node>> {
    match binding_element.kind() {
        Kind::BindingElement => {
            // `a` in `let { a: b } = ...`
            // `[a]` in `let { [a]: b } = ...`
            // `"a"` in `let { "a": b } = ...`
            // `1` in `let { 1: b } = ...`
            if let Some(property_name) = binding_element.property_name() {
                if ast::is_computed_property_name(property_name) && ast::is_string_or_numeric_literal_like(property_name.expression().unwrap()) {
                    return property_name.expression();
                }
                return Some(property_name);
            }
        }
        Kind::PropertyAssignment => {
            // `a` in `({ a: b } = ...)`
            // `[a]` in `({ [a]: b } = ...)`
            // `"a"` in `({ "a": b } = ...)`
            // `1` in `({ 1: b } = ...)`
            if let Some(property_name) = binding_element.name() {
                if ast::is_computed_property_name(property_name) && ast::is_string_or_numeric_literal_like(property_name.expression().unwrap()) {
                    return property_name.expression();
                }
                return Some(property_name);
            }
        }
        Kind::SpreadAssignment => {
            // `a` in `({ ...a } = ...)`
            return binding_element.name();
        }
        _ => {}
    }

    let target = ast::get_target_of_binding_or_assignment_element(binding_element);
    if let Some(target) = target {
        if ast::is_property_name(target) {
            return Some(target);
        }
    }
    None
}

// ast/utilities.go:4038
pub fn is_empty_object_literal(expression: P<Node>) -> bool {
    ast::is_object_literal_expression(expression) && expression.properties().is_empty()
}

// ast/utilities.go:4042
pub fn is_empty_array_literal(expression: P<Node>) -> bool {
    ast::is_array_literal_expression(expression) && expression.elements().is_empty()
}

// ast/utilities.go:4046
pub fn get_rest_indicator_of_binding_or_assignment_element(binding_element: P<Node>) -> Option<P<Node>> {
    match binding_element.kind() {
        Kind::Parameter => binding_element.as_parameter_declaration().dot_dot_dot_token(),
        Kind::BindingElement => binding_element.as_binding_element().dot_dot_dot_token(),
        Kind::SpreadElement | Kind::SpreadAssignment => Some(binding_element),
        _ => None,
    }
}
