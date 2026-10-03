use crate::*;

// Gate stub (docs/EMIT.md section 7): constructor and the `SubtreeFacts` early return of `visit` are ported; the rest
// of typeeraser.go is not.

// typeeraser.go:11
pub struct TypeEraserTransformer {
    pub base: Transformer,
    pub compiler_options: P<CompilerOptions>,
    pub parent_node: Cell<Option<P<Node>>>,
    pub current_node: Cell<Option<P<Node>>>,
}

// typeeraser.go:18
pub fn new_type_eraser_transformer(opt: &TransformOptions) -> Option<P<Transformer>> {
    let compiler_options = opt.compiler_options;
    let emit_context = opt.context;
    let tx = P::new(TypeEraserTransformer { base: Transformer::default(), compiler_options, parent_node: Cell::new(None), current_node: Cell::new(None) });
    Some(tx.get().base.new_transformer(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit(n)), Some(emit_context)))
}

impl TypeEraserTransformer {
    // typeeraser.go:43
    fn visit(&self, node: P<Node>) -> Option<P<Node>> {
        if !node.subtree_facts().intersects(SubtreeFacts::ContainsTypeScript) {
            return Some(node);
        }
        unimplemented!("emit: typeeraser.go not ported")
    }
}
