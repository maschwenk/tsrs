use crate::*;

// Gate stub (docs/EMIT.md section 7): constructor and the `SubtreeFacts` early return of `visit` are ported; the rest
// of async.go is not. The early return goes through `fallbackVisitor`, whose own early return
// (`capturedSuperProperties == nil && lexicalArguments.binding == nil`) always holds here, because only the unported
// part of the transformer captures anything.

// async.go
pub struct asyncTransformer {
    pub base: Transformer,
}

// async.go:37
pub fn new_async_transformer(opt: &TransformOptions) -> Option<P<Transformer>> {
    let tx = P::new(asyncTransformer { base: Transformer::default(), });
    Some(tx.get().base.new_transformer(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit(n)), Some(opt.context)))
}

impl asyncTransformer {
    fn fallback_visitor(&self, node: P<Node>) -> Option<P<Node>> {
        // capturedSuperProperties == nil && lexicalArguments.binding == nil
        Some(node)
    }

    // async.go:126
    fn visit(&self, node: P<Node>) -> Option<P<Node>> {
        // EFNoLexicalThis only toggles asyncContextHasLexicalThis, which nothing reads before the facts check.
        if !node.subtree_facts().intersects(SubtreeFacts::ContainsAnyAwait | SubtreeFacts::ContainsAwait) {
            return self.fallback_visitor(node);
        }
        unimplemented!("emit: async.go not ported")
    }
}
