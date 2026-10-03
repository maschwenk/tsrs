use crate::*;

// Gate stub (docs/EMIT.md section 7): constructor and the `SubtreeFacts` early return of `visit` are ported; the rest
// of jsx.go is not.

// jsx.go:19
pub struct JSXTransformer {
    pub base: Transformer,
    pub compiler_options: P<CompilerOptions>,
    pub emit_resolver: Option<Resolver>,
    pub in_jsx_child: Cell<bool>,
}

// jsx.go:32
pub fn new_jsx_transformer(opts: &TransformOptions) -> Option<P<Transformer>> {
    let compiler_options = opts.compiler_options;
    let emit_context = opts.context;
    let tx = P::new(JSXTransformer { base: Transformer::default(), compiler_options, emit_resolver: opts.emit_resolver, in_jsx_child: Cell::new(false) });
    Some(tx.get().base.new_transformer(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit(Some(n))), Some(emit_context)))
}

impl JSXTransformer {
    // jsx.go:106
    fn visit(&self, node: Option<P<Node>>) -> Option<P<Node>> {
        let node = node?;
        if !node.subtree_facts().intersects(SubtreeFacts::ContainsJsx) {
            return Some(node);
        }
        unimplemented!("emit: jsx.go not ported")
    }
}
