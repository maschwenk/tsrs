use crate::*;

// Gate stub (docs/EMIT.md section 7): constructor and the `SubtreeFacts` early return of `visit` are ported; the rest
// of exponentiation.go is not.

// exponentiation.go
pub struct exponentiationTransformer {
    pub base: Transformer,
}

// exponentiation.go:87
pub fn new_exponentiation_transformer(opt: &TransformOptions) -> Option<P<Transformer>> {
    let tx = P::new(exponentiationTransformer { base: Transformer::default(), });
    Some(tx.get().base.new_transformer(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit(n)), Some(opt.context)))
}

impl exponentiationTransformer {
    // exponentiation.go:12
    fn visit(&self, node: P<Node>) -> Option<P<Node>> {
        if !node.subtree_facts().intersects(SubtreeFacts::ContainsExponentiationOperator) {
            return Some(node);
        }
        unimplemented!("emit: exponentiation.go not ported")
    }
}
