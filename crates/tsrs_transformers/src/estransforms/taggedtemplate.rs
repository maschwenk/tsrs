use crate::*;

// Gate stub (docs/EMIT.md section 7): constructor and the `SubtreeFacts` early return of `visit` are ported; the rest
// of taggedtemplate.go is not.

// taggedtemplate.go
pub struct taggedTemplateTransformer {
    pub base: Transformer,
}

// taggedtemplate.go:22
pub fn new_tagged_template_lift_restriction_transformer(opt: &TransformOptions) -> Option<P<Transformer>> {
    let tx = P::new(taggedTemplateTransformer { base: Transformer::default(), });
    Some(tx.get().base.new_transformer(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit(n)), Some(opt.context)))
}

impl taggedTemplateTransformer {
    // taggedtemplate.go:27
    fn visit(&self, node: P<Node>) -> Option<P<Node>> {
        if !node.subtree_facts().intersects(SubtreeFacts::ContainsInvalidTemplateEscape) {
            return Some(node);
        }
        unimplemented!("emit: taggedtemplate.go not ported")
    }
}
