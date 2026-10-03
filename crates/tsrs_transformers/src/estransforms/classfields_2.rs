// Port of estransforms/classfields.go, part 2 (Go lines 1889-3618: class declarations/expressions, members,
// constructors, private-name environments, destructuring targets and the free helpers). Part 1 is classfields_1.rs.
// Functions whose body is `unimplemented!("classfields part 2")` are signatures only, still to be ported.

use super::*;
use crate::*;
use printer::PrivateIdentifierKind;

impl classFieldsTransformer {
    // classfields.go:1889
    pub(crate) fn visit_class_declaration(&self, node: P<Node>) -> Option<P<Node>> {
        let _ = node;
        unimplemented!("classfields part 2")
    }

    // classfields.go:2003
    pub(crate) fn visit_class_expression(&self, node: P<Node>) -> Option<P<Node>> {
        let _ = node;
        unimplemented!("classfields part 2")
    }

    // classfields.go:2181
    pub(crate) fn visit_class_static_block_declaration(&self, node: P<Node>) -> Option<P<Node>> {
        let _ = node;
        unimplemented!("classfields part 2")
    }

    // classfields.go:2194
    pub(crate) fn visit_this_expression(&self, node: P<Node>) -> Option<P<Node>> {
        let _ = node;
        unimplemented!("classfields part 2")
    }

    // classfields.go:2365
    pub(crate) fn transform_constructor(&self, constructor: P<Node>, container: P<Node>) -> Option<P<Node>> {
        let _ = (constructor, container);
        unimplemented!("classfields part 2")
    }

    // classfields.go:2650
    pub(crate) fn transform_property_or_class_static_block(&self, property: P<Node>, receiver: P<Node>) -> Option<P<Node>> {
        let _ = (property, receiver);
        unimplemented!("classfields part 2")
    }

    // classfields.go:2853
    pub(crate) fn visit_invalid_super_property(&self, node: P<Node>) -> Option<P<Node>> {
        let _ = node;
        unimplemented!("classfields part 2")
    }

    // classfields.go:2876
    pub(crate) fn get_property_name_expression_if_needed(&self, name: P<Node>, should_hoist: bool) -> Option<P<Node>> {
        let _ = (name, should_hoist);
        unimplemented!("classfields part 2")
    }

    // classfields.go:2910
    pub(crate) fn start_class_lexical_environment(&self) {
        self.lexical_environment.set(Some(P::new(classLexicalEnv { previous: self.lexical_environment.get(), data: Cell::new(None), private_env: Cell::new(None) })));
    }

    // classfields.go:2914
    pub(crate) fn end_class_lexical_environment(&self) {
        self.lexical_environment.set(self.lexical_environment.get().unwrap().previous);
    }

    // classfields.go:2918
    pub(crate) fn get_class_lexical_environment(&self) -> P<classLexicalEnvironment> {
        assert!(self.lexical_environment.get().is_some());
        let lex = self.lexical_environment.get().unwrap();
        if lex.data.get().is_none() {
            lex.data.set(Some(P::new(classLexicalEnvironment::default())));
        }
        lex.data.get().unwrap()
    }

    // classfields.go:2926
    pub(crate) fn get_private_identifier_environment(&self) -> P<privateEnvironment> {
        assert!(self.lexical_environment.get().is_some());
        let lex = self.lexical_environment.get().unwrap();
        if lex.private_env.get().is_none() {
            lex.private_env.set(Some(P::new(privateEnvironment::default())));
        }
        lex.private_env.get().unwrap()
    }

    // classfields.go:2936
    pub(crate) fn add_pending_expressions(&self, exprs: &[P<Node>]) {
        self.pending_expressions.borrow_mut().extend_from_slice(exprs);
    }

    // classfields.go:3102
    pub(crate) fn set_private_identifier(&self, env: P<privateEnvironment>, name: P<Node>, info: P<privateIdentifierInfo>) {
        if self.emit_context().has_auto_generate_info(Some(name)) {
            env.generated_identifiers.borrow_mut().insert(self.emit_context().get_node_for_generated_name(name), info);
        } else {
            env.members.borrow_mut().insert(name.text().to_string(), info);
        }
    }

    // classfields.go:3113
    pub(crate) fn get_private_identifier(&self, env: P<privateEnvironment>, name: P<Node>) -> Option<P<privateIdentifierInfo>> {
        if self.emit_context().has_auto_generate_info(Some(name)) {
            return env.generated_identifiers.borrow().get(&self.emit_context().get_node_for_generated_name(name)).copied();
        }
        env.members.borrow().get(name.text()).copied()
    }

    // classfields.go:3181
    pub(crate) fn access_private_identifier(&self, name: P<Node>) -> Option<P<privateIdentifierInfo>> {
        let mut env = self.lexical_environment.get();
        while let Some(e) = env {
            if let Some(private_env) = e.private_env.get() {
                if let Some(info) = self.get_private_identifier(private_env, name) {
                    if info.kind == PrivateIdentifierKind::Untransformed {
                        return None;
                    }
                    return Some(info);
                }
            }
            env = e.previous;
        }
        None
    }

    // classfields.go:3195
    pub(crate) fn wrap_private_identifier_for_destructuring_target(&self, node: P<Node>) -> Option<P<Node>> {
        let _ = node;
        unimplemented!("classfields part 2")
    }

    // classfields.go:3256
    pub(crate) fn visit_array_assignment_element(&self, node: P<Node>) -> Option<P<Node>> {
        let _ = node;
        unimplemented!("classfields part 2")
    }

    // classfields.go:3320
    pub(crate) fn visit_object_assignment_element(&self, node: P<Node>) -> Option<P<Node>> {
        let _ = node;
        unimplemented!("classfields part 2")
    }

    // classfields.go:3334
    pub(crate) fn visit_assignment_pattern(&self, node: P<Node>) -> Option<P<Node>> {
        let _ = node;
        unimplemented!("classfields part 2")
    }
}

// classfields.go:3395
pub(crate) fn is_static_property_declaration_or_class_static_block(node: P<Node>) -> bool {
    ast::is_class_static_block_declaration(node) || (ast::is_property_declaration(node) && ast::has_static_modifier(node))
}

impl classFieldsTransformer {
    // classfields.go:3493
    pub(crate) fn create_accessor_property_get_redirector(&self, node: P<Node>, modifiers: Option<P<ModifierList>>, name: P<Node>, receiver: P<Node>) -> P<Node> {
        let _ = (node, modifiers, name, receiver);
        unimplemented!("classfields part 2")
    }

    // classfields.go:3514
    pub(crate) fn create_accessor_property_set_redirector(&self, node: P<Node>, modifiers: Option<P<ModifierList>>, name: P<Node>, receiver: P<Node>) -> P<Node> {
        let _ = (node, modifiers, name, receiver);
        unimplemented!("classfields part 2")
    }
}

// classfields.go:3547
// Go returns an `iter.Seq`; every caller ranges over all of it, so the elements are collected.
pub(crate) fn flatten_comma_list(node: P<Node>) -> Vec<P<Node>> {
    let mut result = Vec::new();
    flatten_comma_list_worker(node, &mut |e| {
        result.push(e);
        true
    });
    result
}

// classfields.go:3553
fn flatten_comma_list_worker(node: P<Node>, yield_: &mut dyn FnMut(P<Node>) -> bool) -> bool {
    if ast::is_parenthesized_expression(node) && ast::node_is_synthesized(node) {
        flatten_comma_list_worker(node.expression().unwrap(), yield_)
    } else if ast::is_comma_expression(node) {
        flatten_comma_list_worker(node.as_binary_expression().left, yield_) && flatten_comma_list_worker(node.as_binary_expression().right(), yield_)
    } else {
        yield_(node)
    }
}

// classfields.go:3564
// Returns the BinaryExpression node (Go returns `*ast.BinaryExpression`).
pub(crate) fn find_computed_property_name_cache_assignment(emit_context: P<EmitContext>, name: P<Node>) -> Option<P<Node>> {
    let _ = emit_context;
    let mut node = name.expression().unwrap();
    loop {
        node = ast::skip_outer_expressions(node, OuterExpressionKinds::empty());
        if ast::is_binary_expression(node) && node.as_binary_expression().operator_token.kind() == Kind::CommaToken {
            node = node.as_binary_expression().right();
            continue;
        }
        if ast::is_assignment_expression(node, true /*excludeCompoundAssignment*/) && ast::is_identifier(node.as_binary_expression().left) {
            return Some(node);
        }
        break;
    }
    None
}
