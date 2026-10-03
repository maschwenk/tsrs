use crate::*;

// Gate stub (docs/EMIT.md section 7): constructor and the `SubtreeFacts` early return of `visit` are ported; the rest
// of runtimesyntax.go is not.

// runtimesyntax.go:16
pub struct RuntimeSyntaxTransformer {
    pub base: Transformer,
    pub compiler_options: P<CompilerOptions>,
    pub parent_node: Cell<Option<P<Node>>>,
    pub current_node: Cell<Option<P<Node>>>,
    pub current_source_file: Cell<Option<P<Node>>>,
    pub current_scope: Cell<Option<P<Node>>>, // SourceFile | Block | ModuleBlock | CaseBlock
    pub current_scope_first_declarations_of_name: RefCell<Option<FxHashMap<String, P<Node>>>>,
    pub current_enum: Cell<Option<P<Node>>>,
    pub current_namespace: Cell<Option<P<Node>>>,
    pub resolver: ReferenceResolverRef,
    pub emit_resolver: Option<Resolver>,
}

// runtimesyntax.go:31
pub fn new_runtime_syntax_transformer(opt: &TransformOptions) -> Option<P<Transformer>> {
    let compiler_options = opt.compiler_options;
    let emit_context = opt.context;
    let tx = P::new(RuntimeSyntaxTransformer { base: Transformer::default(),
        compiler_options,
        parent_node: Cell::new(None),
        current_node: Cell::new(None),
        current_source_file: Cell::new(None),
        current_scope: Cell::new(None),
        current_scope_first_declarations_of_name: RefCell::new(None),
        current_enum: Cell::new(None),
        current_namespace: Cell::new(None),
        resolver: opt.resolver,
        emit_resolver: opt.emit_resolver,
    });
    Some(tx.get().base.new_transformer(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit(n)), Some(emit_context)))
}

impl RuntimeSyntaxTransformer {
    // runtimesyntax.go:79. Go pushes the node and the scope first; `pushScope` records declarations in the current
    // scope, which is unported state, so the stub checks the early return before anything else.
    fn visit(&self, node: P<Node>) -> Option<P<Node>> {
        if !node.subtree_facts().intersects(SubtreeFacts::ContainsTypeScript)
            && (self.current_namespace.get().is_none() && self.current_enum.get().is_none() || !node.subtree_facts().intersects(SubtreeFacts::ContainsIdentifier))
        {
            return Some(node);
        }
        unimplemented!("emit: runtimesyntax.go not ported")
    }
}
