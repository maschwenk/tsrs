use crate::*;

// Gate stub (docs/EMIT.md section 7): constructor and the `SubtreeFacts` early return of `visit` are ported; the rest
// of optionalcatch.go is not.

// optionalcatch.go
pub struct optionalCatchTransformer {
    pub base: Transformer,
}

// optionalcatch.go:34
pub fn new_optional_catch_transformer(opt: &TransformOptions) -> Option<P<Transformer>> {
    let tx = P::new(optionalCatchTransformer { base: Transformer::default(), });
    Some(tx.get().base.new_transformer(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit(n)), Some(opt.context)))
}

impl optionalCatchTransformer {
    // optionalcatch.go:12
    fn visit(&self, node: P<Node>) -> Option<P<Node>> {
        if !node.subtree_facts().intersects(SubtreeFacts::ContainsMissingCatchClauseVariable) {
            return Some(node);
        }
        unimplemented!("emit: optionalcatch.go not ported")
    }
}
