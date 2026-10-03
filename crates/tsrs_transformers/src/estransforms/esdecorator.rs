use crate::*;

// Gate stub (docs/EMIT.md section 7): constructor, `visit` up to `shouldVisitNode` and `visitSourceFile` are ported;
// the rest of esdecorator.go is not. `classThis`/`classSuper` are only set by the unported class visits, so
// `shouldVisitNode` reduces to the decorators fact here.

// esdecorator.go:100
pub struct esDecoratorTransformer {
    pub base: Transformer,
    pub compiler_options: P<CompilerOptions>,
    pub class_this: Cell<Option<P<Node>>>,
    pub class_super: Cell<Option<P<Node>>>,
    pub should_transform_private_static_elements_in_file: Cell<bool>,
}

// esdecorator.go:124
pub fn new_es_decorator_transformer(opts: &TransformOptions) -> Option<P<Transformer>> {
    // When experimentalDecorators is set, the legacy decorator transformer handles all
    // decorators. When targeting ESNext with useDefineForClassFields, there's nothing to
    // transform. In either case every node would be returned unchanged, so skip entirely.
    if opts.compiler_options.experimental_decorators.is_true()
        || (opts.compiler_options.get_emit_script_target() >= ScriptTarget::ESNext && opts.compiler_options.get_use_define_for_class_fields())
    {
        return None;
    }
    let tx = P::new(esDecoratorTransformer {
        base: Transformer::default(),
        compiler_options: opts.compiler_options,
        class_this: Cell::new(None),
        class_super: Cell::new(None),
        should_transform_private_static_elements_in_file: Cell::new(false),
    });
    let result = tx.get().base.new_transformer(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit(n)), Some(opts.context));
    // TODO(emit/esdecorator): the secondary visitors (outerThisVisitor, discardedVisitor, ...).
    Some(result)
}

impl esDecoratorTransformer {
    // esdecorator.go:271
    fn visit_source_file(&self, node: P<Node>) -> P<Node> {
        // tx.top = nil
        self.should_transform_private_static_elements_in_file.set(false);
        let visited = self.base.visitor().visit_each_child(Some(node)).unwrap();
        let ec = self.base.emit_context();
        ec.add_emit_helper(visited, &ec.read_emit_helpers());
        if self.should_transform_private_static_elements_in_file.get() {
            ec.add_emit_flags(visited, printer::EmitFlags::TransformPrivateStaticElements);
            self.should_transform_private_static_elements_in_file.set(false);
        }
        visited
    }

    // esdecorator.go:298
    fn should_visit_node(&self, node: P<Node>) -> bool {
        node.subtree_facts().intersects(SubtreeFacts::ContainsDecorators)
            || (self.class_this.get().is_some() && node.subtree_facts().intersects(SubtreeFacts::ContainsLexicalThis))
            || (self.class_this.get().is_some() && self.class_super.get().is_some() && node.subtree_facts().intersects(SubtreeFacts::ContainsLexicalSuper))
    }

    // esdecorator.go:304
    fn visit(&self, node: P<Node>) -> Option<P<Node>> {
        if node.kind() == Kind::SourceFile {
            return Some(self.visit_source_file(node));
        }
        if !self.should_visit_node(node) {
            return Some(node);
        }
        unimplemented!("emit: esdecorator.go not ported")
    }
}
