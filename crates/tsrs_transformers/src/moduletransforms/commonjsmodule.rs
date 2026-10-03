use crate::*;

// Gate stub (docs/EMIT.md section 7): the constructor and the `SubtreeFacts` early return of `visitNoStack` are ported;
// the rest of commonjsmodule.go is not.

// commonjsmodule.go:15
pub struct CommonJSModuleTransformer {
    pub base: Transformer,
    pub compiler_options: P<CompilerOptions>,
    pub resolver: ReferenceResolverRef,
    pub get_emit_module_format_of_file: Rc<dyn Fn(P<SourceFile>) -> ModuleKind>,
    pub module_kind: ModuleKind,
    pub language_version: ScriptTarget,
    pub current_source_file: Cell<Option<P<SourceFile>>>,
    pub parent_node: Cell<Option<P<Node>>>, // used for ancestor tracking via pushNode/popNode to detect expression identifiers
    pub current_node: Cell<Option<P<Node>>>, // used for ancestor tracking via pushNode/popNode to detect expression identifiers
}

// commonjsmodule.go:32
pub fn new_common_js_module_transformer(opts: &TransformOptions) -> Option<P<Transformer>> {
    let compiler_options = opts.compiler_options;
    let emit_context = opts.context;
    let tx = P::new(CommonJSModuleTransformer {
        base: Transformer::default(),
        compiler_options,
        resolver: opts.resolver,
        get_emit_module_format_of_file: opts.get_emit_module_format_of_file.clone(),
        module_kind: compiler_options.get_emit_module_kind(),
        language_version: compiler_options.get_emit_script_target(),
        current_source_file: Cell::new(None),
        parent_node: Cell::new(None),
        current_node: Cell::new(None),
    });
    // TODO(emit/transforms): topLevelVisitor, topLevelNestedVisitor, discardedValueVisitor, assignmentPatternVisitor.
    Some(tx.get().base.new_transformer(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit(n)), Some(emit_context)))
}

impl CommonJSModuleTransformer {
    // commonjsmodule.go:46
    fn push_node(&self, node: P<Node>) -> Option<P<Node>> {
        let grandparent_node = self.parent_node.get();
        self.parent_node.set(self.current_node.get());
        self.current_node.set(Some(node));
        grandparent_node
    }

    // commonjsmodule.go:53
    fn pop_node(&self, grandparent_node: Option<P<Node>>) {
        self.current_node.set(self.parent_node.get());
        self.parent_node.set(grandparent_node);
    }

    // commonjsmodule.go:131
    fn visit(&self, node: P<Node>) -> Option<P<Node>> {
        let grandparent_node = self.push_node(node);
        let result = self.visit_no_stack(node, false /*resultIsDiscarded*/);
        self.pop_node(grandparent_node);
        result
    }

    // commonjsmodule.go:138
    fn visit_no_stack(&self, node: P<Node>, result_is_discarded: bool) -> Option<P<Node>> {
        if !ast::is_source_file(node) && !node.subtree_facts().intersects(SubtreeFacts::ContainsDynamicImport | SubtreeFacts::ContainsIdentifier) {
            return Some(node);
        }
        unimplemented!("emit: commonjsmodule.go not ported")
    }
}
