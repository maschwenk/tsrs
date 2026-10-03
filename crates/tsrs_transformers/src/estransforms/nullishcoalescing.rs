use crate::*;

// Gate stub (docs/EMIT.md section 7): constructor and the `SubtreeFacts` early return of `visit` are ported; the rest
// of nullishcoalescing.go is not.

// nullishcoalescing.go
pub struct nullishCoalescingTransformer {
    pub base: Transformer,
}

// nullishcoalescing.go:46
pub fn new_nullish_coalescing_transformer(opt: &TransformOptions) -> Option<P<Transformer>> {
    let tx = P::new(nullishCoalescingTransformer { base: Transformer::default(), });
    Some(tx.get().base.new_transformer(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit(n)), Some(opt.context)))
}

impl nullishCoalescingTransformer {
    // nullishcoalescing.go:12
    fn visit(&self, node: P<Node>) -> Option<P<Node>> {
        if !node.subtree_facts().intersects(SubtreeFacts::ContainsNullishCoalescing) {
            return Some(node);
        }
        unimplemented!("emit: nullishcoalescing.go not ported")
    }
}
