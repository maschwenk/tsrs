use crate::*;

// Gate stub (docs/EMIT.md section 7): the constructor is ported; `visit` has no `SubtreeFacts` early return in Go.

// constenum.go:15
pub struct ConstEnumInliningTransformer {
    pub base: Transformer,
    pub compiler_options: P<CompilerOptions>,
    pub current_source_file: Cell<Option<P<SourceFile>>>,
    pub emit_resolver: Option<Resolver>,
}

// constenum.go:22
pub fn new_const_enum_inlining_transformer(opt: &TransformOptions) -> Option<P<Transformer>> {
    let compiler_options = opt.compiler_options;
    let emit_context = opt.context;
    if compiler_options.get_isolated_modules() {
        panic!("const enums are not inlined under isolated modules");
    }
    let tx = P::new(ConstEnumInliningTransformer { base: Transformer::default(), compiler_options, current_source_file: Cell::new(None), emit_resolver: opt.emit_resolver });
    Some(tx.get().base.new_transformer(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit(n)), Some(emit_context)))
}

impl ConstEnumInliningTransformer {
    // constenum.go:32
    fn visit(&self, node: P<Node>) -> Option<P<Node>> {
        unimplemented!("emit: constenum.go not ported")
    }
}
