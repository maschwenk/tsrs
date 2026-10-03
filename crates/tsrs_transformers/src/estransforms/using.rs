use crate::*;

// Gate stub (docs/EMIT.md section 7): constructor and the `SubtreeFacts` early return of `visit` are ported; the rest
// of using.go is not.

// using.go
pub struct usingDeclarationTransformer {
    pub base: Transformer,
}

// using.go:21
pub fn new_using_declaration_transformer(opt: &TransformOptions) -> Option<P<Transformer>> {
    let tx = P::new(usingDeclarationTransformer { base: Transformer::default(), });
    Some(tx.get().base.new_transformer(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit(n)), Some(opt.context)))
}

impl usingDeclarationTransformer {
    // using.go:34
    fn visit(&self, node: P<Node>) -> Option<P<Node>> {
        if !node.subtree_facts().intersects(SubtreeFacts::ContainsUsing) {
            return Some(node);
        }
        unimplemented!("emit: using.go not ported")
    }
}
