use crate::*;

// Gate stub (docs/EMIT.md section 7): constructor and the `SubtreeFacts` early return of `visit` are ported; the rest
// of objectrestspread.go is not.

// objectrestspread.go
pub struct objectRestSpreadTransformer {
    pub base: Transformer,
    pub compiler_options: P<CompilerOptions>,
    pub parameters_with_preceding_object_rest_or_spread: RefCell<Option<FxHashSet<P<Node>>>>,
}

// objectrestspread.go:590
pub fn new_object_rest_spread_transformer(opt: &TransformOptions) -> Option<P<Transformer>> {
    let tx = P::new(objectRestSpreadTransformer { base: Transformer::default(), compiler_options: opt.compiler_options, parameters_with_preceding_object_rest_or_spread: RefCell::new(None) });
    Some(tx.get().base.new_transformer(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit(n)), Some(opt.context)))
}

impl objectRestSpreadTransformer {
    // objectrestspread.go:20
    fn visit(&self, node: P<Node>) -> Option<P<Node>> {
        if !node.subtree_facts().intersects(SubtreeFacts::ContainsESObjectRestOrSpread) && self.parameters_with_preceding_object_rest_or_spread.borrow().is_none() {
            return Some(node);
        }
        unimplemented!("emit: objectrestspread.go not ported")
    }
}
