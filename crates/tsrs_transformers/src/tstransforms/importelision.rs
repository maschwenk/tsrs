use crate::*;

// Gate stub (docs/EMIT.md section 7): the constructor is ported; `visit` has no `SubtreeFacts` early return in Go, so
// the stub stops at its first statement.

// importelision.go:10
pub struct ImportElisionTransformer {
    pub base: Transformer,
    pub compiler_options: P<CompilerOptions>,
    pub current_source_file: Cell<Option<P<SourceFile>>>,
    pub emit_resolver: Option<Resolver>,
}

// importelision.go:17
pub fn new_import_elision_transformer(opt: &TransformOptions) -> Option<P<Transformer>> {
    let compiler_options = opt.compiler_options;
    let emit_context = opt.context;
    if compiler_options.verbatim_module_syntax.is_true() {
        panic!("ImportElisionTransformer should not be used with VerbatimModuleSyntax");
    }
    let tx = P::new(ImportElisionTransformer { base: Transformer::default(), compiler_options, current_source_file: Cell::new(None), emit_resolver: opt.emit_resolver });
    Some(tx.get().base.new_transformer(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit(n)), Some(emit_context)))
}

impl ImportElisionTransformer {
    // importelision.go:27
    fn visit(&self, node: P<Node>) -> Option<P<Node>> {
        unimplemented!("emit: importelision.go not ported")
    }
}
