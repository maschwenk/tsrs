use std::cell::Cell;
use std::ops::Deref;
use std::rc::Rc;

use tsrs_ast as ast;
use tsrs_ast::*;
use tsrs_core::*;

use crate::*;

// Go embeds `ast.NodeFactory`; here it is the `node_factory` field, reachable through Deref so that
// `factory.new_identifier(..)` works like the promoted Go methods. Like `ast::NodeFactory` it is a handle: all methods
// take `&self` and `clone()` shares the factory, so nested calls (`f.new_x(f.new_y())`) port as written. Only the parts
// the printer and the checker's node builder use are ported (generated names, string literals from nodes); the
// transform helpers are emit-only.
#[derive(Default, Clone)]
pub struct NodeFactory {
    pub node_factory: ast::NodeFactory,
    // Shared with the hooks: `new_emit_context` fills it once the context is allocated (Go builds the context first
    // and then its factory, which a plain field behind `P` cannot express).
    emit_context: Rc<Cell<Option<P<EmitContext>>>>,
}

impl Deref for NodeFactory {
    type Target = ast::NodeFactory;
    fn deref(&self) -> &ast::NodeFactory {
        &self.node_factory
    }
}

/// Builds the factory for an emit context that is not allocated yet; `set_emit_context` must be called before the
/// factory creates its first node.
pub(crate) fn new_node_factory_for_context() -> NodeFactory {
    let slot: Rc<Cell<Option<P<EmitContext>>>> = Rc::default();
    let (s1, s2, s3) = (slot.clone(), slot.clone(), slot.clone());
    NodeFactory {
        node_factory: ast::new_node_factory(NodeFactoryHooks {
            on_create: Some(Rc::new(move |node| s1.get().unwrap().on_create(node))),
            on_update: Some(Rc::new(move |updated, original| s2.get().unwrap().on_update(updated, original))),
            on_clone: Some(Rc::new(move |updated, original| s3.get().unwrap().on_clone(updated, original))),
        }),
        emit_context: slot,
    }
}

impl NodeFactory {
    pub fn as_node_factory(&self) -> &ast::NodeFactory {
        &self.node_factory
    }

    pub(crate) fn set_emit_context(&self, context: P<EmitContext>) {
        self.emit_context.set(Some(context));
    }

    fn emit_context(&self) -> P<EmitContext> {
        self.emit_context.get().unwrap()
    }

    fn new_generated_identifier(&self, kind: GeneratedIdentifierFlags, text: &str, node: Option<P<Node>>, options: AutoGenerateOptions) -> P<Node> {
        let id = next_auto_generate_id();

        let mut text = text.to_string();
        if text.is_empty() {
            text = match node {
                None => format!("(auto@{})", id.0),
                Some(node) if is_member_name(node) => node.text().to_string(),
                Some(node) => format!("(generated@{})", get_node_id(self.emit_context().get_node_for_generated_name_worker(node, id)).0),
            };
            text = format_generated_name(false /*privateName*/, options.prefix, &text, options.suffix);
        }

        let name = self.new_identifier(alloc_str(&text));
        let auto_generate = AutoGenerateInfo {
            id,
            flags: kind | (options.flags & !GeneratedIdentifierFlags::KindMask),
            prefix: options.prefix,
            suffix: options.suffix,
            node,
        };
        self.emit_context().set_auto_generate_info(name, auto_generate);
        name
    }

    // Allocates a new temp variable name, but does not record it in the environment. It is recommended to pass this to either
    // `AddVariableDeclaration` or `AddLexicalDeclaration` to ensure it is properly tracked, if you are not otherwise handling
    // it yourself.
    pub fn new_temp_variable(&self) -> P<Node> {
        self.new_temp_variable_ex(AutoGenerateOptions::default())
    }

    // Allocates a new temp variable name, but does not record it in the environment. It is recommended to pass this to either
    // `AddVariableDeclaration` or `AddLexicalDeclaration` to ensure it is properly tracked, if you are not otherwise handling
    // it yourself.
    pub fn new_temp_variable_ex(&self, options: AutoGenerateOptions) -> P<Node> {
        self.new_generated_identifier(GeneratedIdentifierFlags::Auto, "", None /*node*/, options)
    }

    // Allocates a new loop variable name.
    pub fn new_loop_variable(&self) -> P<Node> {
        self.new_loop_variable_ex(AutoGenerateOptions::default())
    }

    // Allocates a new loop variable name.
    pub fn new_loop_variable_ex(&self, options: AutoGenerateOptions) -> P<Node> {
        self.new_generated_identifier(GeneratedIdentifierFlags::Loop, "", None /*node*/, options)
    }

    // Allocates a new unique name based on the provided text.
    pub fn new_unique_name(&self, text: &str) -> P<Node> {
        self.new_unique_name_ex(text, AutoGenerateOptions::default())
    }

    // Allocates a new unique name based on the provided text.
    pub fn new_unique_name_ex(&self, text: &str, options: AutoGenerateOptions) -> P<Node> {
        self.new_generated_identifier(GeneratedIdentifierFlags::Unique, text, None /*node*/, options)
    }

    // Allocates a new unique name based on the provided node.
    pub fn new_generated_name_for_node(&self, node: P<Node>) -> P<Node> {
        self.new_generated_name_for_node_ex(node, AutoGenerateOptions::default())
    }

    // Allocates a new unique name based on the provided node.
    pub fn new_generated_name_for_node_ex(&self, node: P<Node>, options: AutoGenerateOptions) -> P<Node> {
        let mut options = options;
        if !options.prefix.is_empty() || !options.suffix.is_empty() {
            options.flags |= GeneratedIdentifierFlags::Optimistic;
        }

        self.new_generated_identifier(GeneratedIdentifierFlags::Node, "", Some(node), options)
    }

    fn new_generated_private_identifier(&self, kind: GeneratedIdentifierFlags, text: &str, node: Option<P<Node>>, options: AutoGenerateOptions) -> P<Node> {
        let id = next_auto_generate_id();

        let mut text = text.to_string();
        if text.is_empty() {
            text = match node {
                None => format!("(auto@{})", id.0),
                Some(node) if is_member_name(node) => node.text().to_string(),
                Some(node) => format!("(generated@{})", get_node_id(self.emit_context().get_node_for_generated_name_worker(node, id)).0),
            };
            text = format_generated_name(true /*privateName*/, options.prefix, &text, options.suffix);
        } else if !text.starts_with('#') {
            panic!("First character of private identifier must be #: {}", text);
        }

        let name = self.new_private_identifier(alloc_str(&text));
        let auto_generate = AutoGenerateInfo {
            id,
            flags: kind | (options.flags & !GeneratedIdentifierFlags::KindMask),
            prefix: options.prefix,
            suffix: options.suffix,
            node,
        };
        self.emit_context().set_auto_generate_info(name, auto_generate);
        name
    }

    // Allocates a new unique private name based on the provided text.
    pub fn new_unique_private_name(&self, text: &str) -> P<Node> {
        self.new_unique_private_name_ex(text, AutoGenerateOptions::default())
    }

    // Allocates a new unique private name based on the provided text.
    pub fn new_unique_private_name_ex(&self, text: &str, options: AutoGenerateOptions) -> P<Node> {
        self.new_generated_private_identifier(GeneratedIdentifierFlags::Unique, text, None /*node*/, options)
    }

    // Allocates a new unique private name based on the provided node.
    pub fn new_generated_private_name_for_node(&self, node: P<Node>) -> P<Node> {
        self.new_generated_private_name_for_node_ex(node, AutoGenerateOptions::default())
    }

    // Allocates a new unique private name based on the provided node.
    pub fn new_generated_private_name_for_node_ex(&self, node: P<Node>, options: AutoGenerateOptions) -> P<Node> {
        let mut options = options;
        if !options.prefix.is_empty() || !options.suffix.is_empty() {
            options.flags |= GeneratedIdentifierFlags::Optimistic;
        }

        self.new_generated_private_identifier(GeneratedIdentifierFlags::Node, "", Some(node), options)
    }

    // Allocates a new StringLiteral whose source text is derived from the provided node. This is often used to create a
    // string representation of an Identifier or NumericLiteral.
    pub fn new_string_literal_from_node(&self, text_source_node: P<Node>) -> P<Node> {
        let text = match text_source_node.kind {
            Kind::Identifier
            | Kind::PrivateIdentifier
            | Kind::JsxNamespacedName
            | Kind::StringLiteral
            | Kind::NumericLiteral
            | Kind::BigIntLiteral
            | Kind::NoSubstitutionTemplateLiteral
            | Kind::TemplateHead
            | Kind::TemplateMiddle
            | Kind::TemplateTail
            | Kind::RegularExpressionLiteral => text_source_node.text(),
            _ => "",
        };
        let node = self.new_string_literal(text, TokenFlags::None);
        self.emit_context().set_text_source(node, text_source_node);
        node
    }
}
