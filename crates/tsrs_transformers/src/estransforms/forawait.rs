use crate::*;

// Gate stub (docs/EMIT.md section 7): constructor and the `SubtreeFacts` early return of `visit` are ported; the rest
// of forawait.go is not. The early return goes through `fallbackVisitor`, whose own early return
// (`capturedSuperProperties == nil && lexicalArguments.binding == nil`) always holds here, because only the unported
// part of the transformer captures anything.

// forawait.go
pub struct forawaitTransformer {
    pub base: Transformer,
    pub compiler_options: P<CompilerOptions>,
}

// forawait.go:56
pub fn newforawait_transformer(opt: &TransformOptions) -> Option<P<Transformer>> {
    let tx = P::new(forawaitTransformer { base: Transformer::default(), compiler_options: opt.compiler_options });
    Some(tx.get().base.new_transformer(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit(n)), Some(opt.context)))
}

impl forawaitTransformer {
    fn fallback_visitor(&self, node: P<Node>) -> Option<P<Node>> {
        // capturedSuperProperties == nil && lexicalArguments.binding == nil
        Some(node)
    }

    // forawait.go:126
    fn visit(&self, node: P<Node>) -> Option<P<Node>> {
        if !node.subtree_facts().intersects(SubtreeFacts::ContainsForAwaitOrAsyncGenerator) {
            return self.fallback_visitor(node);
        }
        unimplemented!("emit: forawait.go not ported")
    }
}
