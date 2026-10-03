use crate::*;

// Gate stub (docs/EMIT.md section 7): constructor and the `SubtreeFacts` early return of `visit` are ported; the rest
// of logicalassignment.go is not.

// logicalassignment.go
pub struct logicalAssignmentTransformer {
    pub base: Transformer,
}

// logicalassignment.go:110
pub fn new_logical_assignment_transformer(opt: &TransformOptions) -> Option<P<Transformer>> {
    let tx = P::new(logicalAssignmentTransformer { base: Transformer::default(), });
    Some(tx.get().base.new_transformer(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit(n)), Some(opt.context)))
}

impl logicalAssignmentTransformer {
    // logicalassignment.go:12
    fn visit(&self, node: P<Node>) -> Option<P<Node>> {
        if !node.subtree_facts().intersects(SubtreeFacts::ContainsLogicalAssignments) {
            return Some(node);
        }
        unimplemented!("emit: logicalassignment.go not ported")
    }
}
