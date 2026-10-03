use crate::*;

// Gate stub (docs/EMIT.md section 7): constructor and the `SubtreeFacts` early return of `visit` are ported; the rest
// of optionalchain.go is not.

// optionalchain.go
pub struct optionalChainTransformer {
    pub base: Transformer,
}

// optionalchain.go:237
pub fn new_optional_chain_transformer(opt: &TransformOptions) -> Option<P<Transformer>> {
    let tx = P::new(optionalChainTransformer { base: Transformer::default(), });
    Some(tx.get().base.new_transformer(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit(n)), Some(opt.context)))
}

impl optionalChainTransformer {
    // optionalchain.go:14
    fn visit(&self, node: P<Node>) -> Option<P<Node>> {
        if !node.subtree_facts().intersects(SubtreeFacts::ContainsOptionalChaining) {
            return Some(node);
        }
        unimplemented!("emit: optionalchain.go not ported")
    }
}
