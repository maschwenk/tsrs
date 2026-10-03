use crate::*;

// Gate stub (docs/EMIT.md section 7): constructor and the `SubtreeFacts` early return of `visit` are ported; the rest
// of legacydecorators.go is not.

// legacydecorators.go:12
pub struct LegacyDecoratorsTransformer {
    pub base: Transformer,
    pub language_version: ScriptTarget,
    pub reference_resolver: ReferenceResolverRef,

    /**
     * A map that keeps track of aliases created for classes with decorators to avoid issues
     * with the double-binding behavior of classes.
     */
    pub class_aliases: RefCell<FxHashMap<P<Node>, P<Node>>>,
    pub enclosing_classes: RefCell<Vec<P<Node>>>,
}

// legacydecorators.go:25
pub fn new_legacy_decorators_transformer(opt: &TransformOptions) -> Option<P<Transformer>> {
    let tx = P::new(LegacyDecoratorsTransformer { base: Transformer::default(), language_version: opt.compiler_options.get_emit_script_target(), reference_resolver: opt.resolver, class_aliases: RefCell::new(FxHashMap::default()), enclosing_classes: RefCell::new(Vec::new()) });
    Some(tx.get().base.new_transformer(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit(n)), Some(opt.context)))
}

impl LegacyDecoratorsTransformer {
    // legacydecorators.go:30
    fn visit(&self, node: P<Node>) -> Option<P<Node>> {
        // we have to visit all identifiers in classes, just in case they require substitution
        if !node.subtree_facts().intersects(SubtreeFacts::ContainsDecorators) && self.enclosing_classes.borrow().is_empty() {
            return Some(node);
        }
        unimplemented!("emit: legacydecorators.go not ported")
    }
}
