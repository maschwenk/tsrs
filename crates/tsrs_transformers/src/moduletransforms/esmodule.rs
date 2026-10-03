use crate::*;

// Gate stub (docs/EMIT.md section 7): the constructor is ported; `visit` has no `SubtreeFacts` early return in Go.

// esmodule.go:14
pub struct ESModuleTransformer {
    pub base: Transformer,
    pub compiler_options: P<CompilerOptions>,
    pub resolver: ReferenceResolverRef,
    pub get_emit_module_format_of_file: Rc<dyn Fn(P<SourceFile>) -> ModuleKind>,
    pub current_source_file: Cell<Option<P<SourceFile>>>,
}

// esmodule.go:28
pub fn new_es_module_transformer(opts: &TransformOptions) -> Option<P<Transformer>> {
    let compiler_options = opts.compiler_options;
    let tx = P::new(ESModuleTransformer {
        base: Transformer::default(),
        compiler_options,
        resolver: opts.resolver,
        get_emit_module_format_of_file: opts.get_emit_module_format_of_file.clone(),
        current_source_file: Cell::new(None),
    });
    Some(tx.get().base.new_transformer(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit(n)), Some(opts.context)))
}

impl ESModuleTransformer {
    // esmodule.go:34
    fn visit(&self, node: P<Node>) -> Option<P<Node>> {
        unimplemented!("emit: esmodule.go not ported")
    }
}
