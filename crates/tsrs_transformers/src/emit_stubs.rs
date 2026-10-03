// TODO(emit/core): everything in this file belongs to the `emit/core` wave (printer factory.go and helpers.go,
// transformers/transformer.go, chain.go, utilities.go, and the `printer.EmitResolver` methods the script
// transformers call). It is the minimum `emit/jsx-decorators` needs to compile and test its transformers before
// that branch lands; on rebase, every item here is replaced by the `emit/core` version and this file is deleted.
// The names and signatures follow the Go originals so the call sites do not change.

use crate::*;
use tsrs_binder as binder;
use tsrs_checker::TypeReferenceSerializationKind;

pub use tsrs_declarations::{is_simple_inlineable_expression, Resolver, Transformer};

//
// transformers/chain.go: TransformOptions
//

/// Go `binder.ReferenceResolver` as chosen in `getScriptTransformers`: the emit resolver, or a plain binder
/// reference resolver (no checker) for isolated-modules emit without import elision.
#[derive(Clone, Copy)]
pub enum ReferenceResolverRef {
    Emit(Resolver),
    Plain(P<binder::ReferenceResolver<()>>),
}

impl ReferenceResolverRef {
    pub fn get_referenced_value_declaration(&self, node: P<Node>) -> Option<P<Node>> {
        match self {
            ReferenceResolverRef::Emit(r) => r.get_referenced_value_declaration(node),
            ReferenceResolverRef::Plain(r) => r.get_referenced_value_declaration(&mut (), node),
        }
    }
}

pub struct TransformOptions {
    pub context: P<EmitContext>,
    pub compiler_options: P<CompilerOptions>,
    pub resolver: ReferenceResolverRef,
    pub emit_resolver: Resolver,
}

//
// printer.EmitResolver methods missing from `tsrs_declarations::Resolver`
//

pub trait ResolverExt {
    fn set_referenced_import_declaration(&self, node: P<Node>, ref_: P<Node>);
    fn get_referenced_export_container(&self, node: P<Node>, prefix_locals: bool) -> Option<P<Node>>;
    fn get_jsx_factory_entity(&self, location: P<Node>) -> Option<P<Node>>;
    fn get_jsx_fragment_factory_entity(&self, location: P<Node>) -> Option<P<Node>>;
    fn get_type_reference_serialization_kind(&self, type_name: Option<P<Node>>, location: Option<P<Node>>) -> TypeReferenceSerializationKind;
}

impl ResolverExt for Resolver {
    fn set_referenced_import_declaration(&self, node: P<Node>, ref_: P<Node>) {
        self.lock(|c| self.r.set_referenced_import_declaration(c, node, ref_))
    }
    fn get_referenced_export_container(&self, node: P<Node>, prefix_locals: bool) -> Option<P<Node>> {
        self.lock(|c| self.r.get_referenced_export_container(c, node, prefix_locals))
    }
    fn get_jsx_factory_entity(&self, location: P<Node>) -> Option<P<Node>> {
        self.lock(|c| self.r.get_jsx_factory_entity(c, location))
    }
    fn get_jsx_fragment_factory_entity(&self, location: P<Node>) -> Option<P<Node>> {
        self.lock(|c| self.r.get_jsx_fragment_factory_entity(c, location))
    }
    fn get_type_reference_serialization_kind(&self, type_name: Option<P<Node>>, location: Option<P<Node>>) -> TypeReferenceSerializationKind {
        self.lock(|c| self.r.get_type_reference_serialization_kind(c, type_name, location))
    }
}

//
// transformers/utilities.go
//

// utilities.go:12
pub fn is_generated_identifier(emit_context: P<EmitContext>, name: P<Node>) -> bool {
    emit_context.has_auto_generate_info(Some(name))
}

// utilities.go:308
// MoveRangePastModifiers returns a text range that starts past any modifiers on the node.
pub fn move_range_past_modifiers(node: P<Node>) -> TextRange {
    if ast::is_property_declaration(node) || ast::is_method_declaration(node) {
        return TextRange::new(node.name().unwrap().pos(), node.end());
    }

    let mut last_modifier: Option<P<Node>> = None;
    if ast::can_have_modifiers(node) {
        last_modifier = node.modifier_nodes().last().copied();
    }

    if let Some(last_modifier) = last_modifier {
        if !ast::position_is_synthesized(last_modifier.end()) {
            return TextRange::new(last_modifier.end(), node.end());
        }
    }
    move_range_past_decorators(node)
}

// utilities.go:325
// MoveRangePastDecorators returns a text range that starts past any decorators on the node.
pub fn move_range_past_decorators(node: P<Node>) -> TextRange {
    let mut last_decorator: Option<P<Node>> = None;
    if ast::can_have_modifiers(node) {
        let nodes = node.modifier_nodes();
        last_decorator = nodes.iter().rev().find(|n| ast::is_decorator(**n)).copied();
    }

    if let Some(last_decorator) = last_decorator {
        if !ast::position_is_synthesized(last_decorator.end()) {
            return TextRange::new(last_decorator.end(), node.end());
        }
    }
    node.loc()
}

//
// printer/helpers.go (the helpers this wave requests)
//

static DECORATE_HELPER_PRIORITY: printer::Priority = printer::Priority { value: 2 };
pub static DECORATE_HELPER: printer::EmitHelper = printer::EmitHelper {
    name: "typescript:decorate",
    import_name: "__decorate",
    scoped: false,
    priority: Some(&DECORATE_HELPER_PRIORITY),
    text: r#"var __decorate = (this && this.__decorate) || function (decorators, target, key, desc) {
    var c = arguments.length, r = c < 3 ? target : desc === null ? desc = Object.getOwnPropertyDescriptor(target, key) : desc, d;
    if (typeof Reflect === "object" && typeof Reflect.decorate === "function") r = Reflect.decorate(decorators, target, key, desc);
    else for (var i = decorators.length - 1; i >= 0; i--) if (d = decorators[i]) r = (c < 3 ? d(r) : c > 3 ? d(target, key, r) : d(target, key)) || r;
    return c > 3 && r && Object.defineProperty(target, key, r), r;
};"#,
    text_callback: None,
    dependencies: &[],
};

static METADATA_HELPER_PRIORITY: printer::Priority = printer::Priority { value: 3 };
pub static METADATA_HELPER: printer::EmitHelper = printer::EmitHelper {
    name: "typescript:metadata",
    import_name: "__metadata",
    scoped: false,
    priority: Some(&METADATA_HELPER_PRIORITY),
    text: r#"var __metadata = (this && this.__metadata) || function (k, v) {
    if (typeof Reflect === "object" && typeof Reflect.metadata === "function") return Reflect.metadata(k, v);
};"#,
    text_callback: None,
    dependencies: &[],
};

static PARAM_HELPER_PRIORITY: printer::Priority = printer::Priority { value: 4 };
pub static PARAM_HELPER: printer::EmitHelper = printer::EmitHelper {
    name: "typescript:param",
    import_name: "__param",
    scoped: false,
    priority: Some(&PARAM_HELPER_PRIORITY),
    text: r#"var __param = (this && this.__param) || function (paramIndex, decorator) {
    return function (target, key) { decorator(target, key, paramIndex); }
};"#,
    text_callback: None,
    dependencies: &[],
};

//
// printer/factory.go
//

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

pub trait FactoryExt {
    fn new_this_expression(&self) -> P<Node>;
    fn new_true_expression(&self) -> P<Node>;
    fn new_false_expression(&self) -> P<Node>;
    fn new_logical_and_expression(&self, left: P<Node>, right: P<Node>) -> P<Node>;
    fn new_strict_inequality_expression(&self, left: P<Node>, right: P<Node>) -> P<Node>;
    fn create_expression_from_entity_name(&self, node: P<Node>) -> P<Node>;
    fn get_name(&self, node: P<Node>, emit_flags: EmitFlags, opts: AssignedNameOptions) -> P<Node>;
    fn get_local_name(&self, node: P<Node>) -> P<Node>;
    fn get_local_name_ex(&self, node: P<Node>, opts: AssignedNameOptions) -> P<Node>;
    fn get_declaration_name(&self, node: P<Node>) -> P<Node>;
    fn get_declaration_name_ex(&self, node: P<Node>, opts: NameOptions) -> P<Node>;
    fn new_unscoped_helper_name(&self, name: &'static str) -> P<Node>;
    fn new_decorate_helper(&self, decorator_expressions: &[P<Node>], target: P<Node>, member_name: Option<P<Node>>, descriptor: Option<P<Node>>) -> P<Node>;
    fn new_metadata_helper(&self, metadata_key: &'static str, metadata_value: P<Node>) -> P<Node>;
    fn new_param_helper(&self, expression: P<Node>, parameter_offset: usize, location: TextRange) -> P<Node>;
    fn new_export_default(&self, expression: P<Node>) -> P<Node>;
    fn new_external_module_export(&self, name: P<Node>) -> P<Node>;
    fn new_assign_helper(&self, attributes_segments: &[P<Node>], script_target: ScriptTarget) -> P<Node>;
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

    // factory.go:221
    fn new_logical_and_expression(&self, left: P<Node>, right: P<Node>) -> P<Node> {
        self.new_binary_expression(None /*modifiers*/, left, None /*typeNode*/, self.new_token(Kind::AmpersandAmpersandToken), right)
    }

    // factory.go:233
    fn new_strict_inequality_expression(&self, left: P<Node>, right: P<Node>) -> P<Node> {
        self.new_binary_expression(None /*modifiers*/, left, None /*typeNode*/, self.new_token(Kind::ExclamationEqualsEqualsToken), right)
    }

    // factory.go:283
    fn create_expression_from_entity_name(&self, node: P<Node>) -> P<Node> {
        if ast::is_qualified_name(node) {
            let qn = node.as_qualified_name();
            let left = self.create_expression_from_entity_name(qn.left);
            let right = qn.right.clone_node(self.as_node_factory());
            right.set_loc(qn.right.loc());
            // TODO(rbuckton): Does this need to be parented?
            right.set_parent(qn.right.parent());
            let prop_access = self.new_property_access_expression(left, None, right, NodeFlags::None);
            prop_access.set_loc(node.loc());
            return prop_access;
        }
        let res = node.clone_node(self.as_node_factory());
        res.set_loc(node.loc());
        // TODO(rbuckton): Does this need to be parented?
        res.set_parent(node.parent());
        res
    }

    // factory.go:496
    fn get_name(&self, node: P<Node>, emit_flags: EmitFlags, opts: AssignedNameOptions) -> P<Node> {
        let mut emit_flags = emit_flags;
        let node_name = if opts.ignore_assigned_name { ast::get_non_assigned_name_of_declaration(node) } else { ast::get_name_of_declaration(node) };

        if let Some(node_name) = node_name {
            let name = node_name.clone_node(self.as_node_factory());
            if !opts.allow_comments {
                emit_flags |= EmitFlags::NoComments;
            }
            if !opts.allow_source_maps {
                emit_flags |= EmitFlags::NoSourceMap;
            }
            self.emit_context().add_emit_flags(name, emit_flags);
            return name;
        }

        self.new_generated_name_for_node(node)
    }

    // factory.go:524
    fn get_local_name(&self, node: P<Node>) -> P<Node> {
        self.get_local_name_ex(node, AssignedNameOptions::default())
    }

    // factory.go:531
    fn get_local_name_ex(&self, node: P<Node>, opts: AssignedNameOptions) -> P<Node> {
        self.get_name(node, EmitFlags::LocalName, opts)
    }

    // factory.go:552
    fn get_declaration_name(&self, node: P<Node>) -> P<Node> {
        self.get_declaration_name_ex(node, NameOptions::default())
    }

    // factory.go:557
    fn get_declaration_name_ex(&self, node: P<Node>, opts: NameOptions) -> P<Node> {
        self.get_name(node, EmitFlags::None, AssignedNameOptions { allow_comments: opts.allow_comments, allow_source_maps: opts.allow_source_maps, ignore_assigned_name: false })
    }

    // factory.go:593
    fn new_unscoped_helper_name(&self, name: &'static str) -> P<Node> {
        let node = self.new_identifier(name);
        self.emit_context().set_emit_flags(node, EmitFlags::HelperName);
        node
    }

    // factory.go:601
    fn new_decorate_helper(&self, decorator_expressions: &[P<Node>], target: P<Node>, member_name: Option<P<Node>>, descriptor: Option<P<Node>>) -> P<Node> {
        self.emit_context().request_emit_helper(P::from_static(&DECORATE_HELPER));

        let mut arguments_array: Vec<P<Node>> = Vec::new();
        arguments_array.push(self.new_array_literal_expression(self.new_node_list(decorator_expressions.to_vec()), true));
        arguments_array.push(target);
        if let Some(member_name) = member_name {
            arguments_array.push(member_name);
            if let Some(descriptor) = descriptor {
                arguments_array.push(descriptor);
            }
        }

        self.new_call_expression(
            self.new_unscoped_helper_name("__decorate"),
            None, /*questionDotToken*/
            None, /*typeArguments*/
            self.new_node_list(arguments_array),
            NodeFlags::None,
        )
    }

    // factory.go:623
    fn new_metadata_helper(&self, metadata_key: &'static str, metadata_value: P<Node>) -> P<Node> {
        self.emit_context().request_emit_helper(P::from_static(&METADATA_HELPER));

        self.new_call_expression(
            self.new_unscoped_helper_name("__metadata"),
            None, /*questionDotToken*/
            None, /*typeArguments*/
            self.new_node_list(vec![self.new_string_literal(metadata_key, TokenFlags::None), metadata_value]),
            NodeFlags::None,
        )
    }

    // factory.go:638
    fn new_param_helper(&self, expression: P<Node>, parameter_offset: usize, location: TextRange) -> P<Node> {
        self.emit_context().request_emit_helper(P::from_static(&PARAM_HELPER));
        let helper = self.new_call_expression(
            self.new_unscoped_helper_name("__param"),
            None, /*questionDotToken*/
            None, /*typeArguments*/
            self.new_node_list(vec![self.new_numeric_literal(alloc_str(&parameter_offset.to_string()), TokenFlags::None), expression]),
            NodeFlags::None,
        );
        helper.set_loc(location);
        helper
    }

    // factory.go:808
    // Creates `export default <expression>;`.
    fn new_export_default(&self, expression: P<Node>) -> P<Node> {
        self.new_export_assignment(None, false, None, expression)
    }

    // factory.go:813
    // Creates `export { <name> };`.
    fn new_external_module_export(&self, name: P<Node>) -> P<Node> {
        let specifier = self.new_export_specifier(false, None, name);
        let named_exports = self.new_named_exports(self.new_node_list(vec![specifier]));
        self.new_export_declaration(None, false, Some(named_exports), None, None)
    }

    // factory.go:821
    // Chains a sequence of expressions using the __assign helper or Object.assign if available in the target
    fn new_assign_helper(&self, attributes_segments: &[P<Node>], script_target: ScriptTarget) -> P<Node> {
        self.new_call_expression(self.new_property_access_expression(self.new_identifier("Object"), None, self.new_identifier("assign"), NodeFlags::None), None, None, self.new_node_list(attributes_segments.to_vec()), NodeFlags::None)
    }
}
