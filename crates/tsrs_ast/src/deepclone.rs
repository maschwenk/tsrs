use std::rc::Rc;

use tsrs_core::{undefined_text_range, P};

use crate::*;

// Ideally, this would get cached on the node factory so there's only ever one set of closures made per factory
fn get_deep_clone_visitor(f: NodeFactory, synthetic_location: bool) -> NodeVisitor {
    let visit: VisitFn = Rc::new(move |visitor: &mut NodeVisitor, node: P<Node>| {
        let visited = visitor.visit_each_child(Some(node)).unwrap();
        if visited != node {
            if synthetic_location {
                visited.set_loc(undefined_text_range());
            }
            return Some(visited);
        }
        let c = node.clone_node(&visitor.factory); // forcibly clone leaf nodes, which will then cascade new nodes/arrays upwards via `update` calls
        // In strada, `factory.cloneNode` was dynamic and did _not_ clone positions for any "special cases", meanwhile
        // Node.Clone in corsa reliably uses `Update` calls for all nodes and so copies locations by default.
        // Deep clones are done to copy a node across files, so here, we explicitly make the location range synthetic on all cloned nodes
        if synthetic_location {
            c.set_loc(undefined_text_range());
        }
        Some(c)
    });
    let visit_nodes: VisitNodesHook = Rc::new(move |nodes: Option<P<NodeList>>, v: &mut NodeVisitor| {
        let nodes = nodes?;
        let visited = v.visit_nodes(Some(nodes)).unwrap();
        let new_list = if visited != nodes { visited } else { nodes.clone_list(&v.factory) };
        if synthetic_location {
            new_list.loc.set(undefined_text_range());
            if nodes.has_trailing_comma() {
                new_list.loc.set(synthetic_trailing_comma_range());
            }
        }
        Some(new_list)
    });
    let visit_modifiers: VisitModifiersHook = Rc::new(move |nodes: Option<P<ModifierList>>, v: &mut NodeVisitor| {
        let nodes = nodes?;
        let visited = v.visit_modifiers(Some(nodes)).unwrap();
        let new_list = if visited != nodes { visited } else { nodes.clone_list(&v.factory) };
        if synthetic_location {
            new_list.list.loc.set(undefined_text_range());
            if nodes.has_trailing_comma() {
                new_list.list.loc.set(synthetic_trailing_comma_range());
            }
        }
        Some(new_list)
    });
    new_node_visitor(
        Some(visit),
        Some(f),
        NodeVisitorHooks { visit_nodes: Some(visit_nodes), visit_modifiers: Some(visit_modifiers), ..Default::default() },
    )
}

// The visitor owns a handle to this factory (`NodeFactory::clone` shares hooks and counters).
impl NodeFactory {
    fn with_deep_clone_visitor<R>(&self, synthetic_location: bool, f: impl FnOnce(&mut NodeVisitor) -> R) -> R {
        let mut visitor = get_deep_clone_visitor(self.clone(), synthetic_location);
        f(&mut visitor)
    }

    pub fn deep_clone_node(&self, node: Option<P<Node>>) -> Option<P<Node>> {
        self.with_deep_clone_visitor(true /*syntheticLocation*/, |v| v.visit_node(node))
    }

    pub fn deep_clone_reparse(&self, node: Option<P<Node>>) -> Option<P<Node>> {
        let node = node?;
        let node = self.with_deep_clone_visitor(false /*syntheticLocation*/, |v| v.visit_node(Some(node))).unwrap();
        set_parent_in_children(node);
        node.set_flags(node.flags() | NodeFlags::Reparsed);
        Some(node)
    }

    pub fn deep_clone_reparse_modifiers(&self, modifiers: Option<P<ModifierList>>) -> Option<P<ModifierList>> {
        self.with_deep_clone_visitor(false /*syntheticLocation*/, |v| v.visit_modifiers(modifiers))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tsrs_core::new_text_range;

    #[test]
    fn deep_clone_reparse_copies_subtree_and_sets_parents() {
        let f = NodeFactory::default();
        let left = f.new_identifier("a");
        left.set_loc(new_text_range(0, 1));
        let right = f.new_identifier("b");
        right.set_loc(new_text_range(2, 3));
        let name = f.new_qualified_name(left, right);
        name.set_loc(new_text_range(0, 3));

        let clone = f.deep_clone_reparse(Some(name)).unwrap();
        assert!(clone != name);
        assert_eq!(clone.loc(), name.loc());
        assert!(clone.flags().intersects(NodeFlags::Reparsed));
        let q = clone.as_qualified_name();
        assert!(q.left != left && q.right != right);
        assert_eq!(q.left.text(), "a");
        assert_eq!(q.left.parent(), Some(clone));

        let synthetic = f.deep_clone_node(Some(name)).unwrap();
        assert_eq!(synthetic.loc(), undefined_text_range());
        assert_eq!(synthetic.as_qualified_name().right.loc(), undefined_text_range());
    }

    #[test]
    fn deep_clone_preserves_a_trailing_comma_on_a_synthetic_list() {
        let f = NodeFactory::default();
        let name = f.new_identifier("a");
        let element = f.new_binding_element(None, None, Some(name), None);
        let elements = f.new_node_list(vec![element]);
        elements.loc.set(new_text_range(1, 3));
        assert!(elements.has_trailing_comma());

        let pattern = f.new_binding_pattern(Kind::ObjectBindingPattern, elements);
        pattern.set_loc(new_text_range(0, 4));
        let synthetic = f.deep_clone_node(Some(pattern)).unwrap();
        let synthetic_elements = synthetic.as_binding_pattern().elements();
        assert!(synthetic_elements.has_trailing_comma());
        assert_eq!(synthetic_elements.loc(), synthetic_trailing_comma_range());
        assert_eq!(synthetic_elements.nodes()[0].loc(), undefined_text_range());
    }
}
