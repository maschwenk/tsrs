use crate::*;

// Gate stub (docs/EMIT.md section 7): constructor and the `SubtreeFacts` early return of `visit` are ported; the rest
// of metadata.go is not.

pub const USE_NEW_TYPE_METADATA_FORMAT: bool = false;

// metadata.go:12
pub struct MetadataTransformer {
    pub base: Transformer,
    pub legacy_decorators: bool,
    pub resolver: Option<Resolver>,

    pub language_version: ScriptTarget,
    pub strict_null_checks: bool,
    pub parent: Cell<Option<P<Node>>>,
    pub current_lexical_scope: Cell<Option<P<Node>>>,
}

// metadata.go:24
pub fn new_metadata_transformer(opt: &TransformOptions) -> Option<P<Transformer>> {
    let tx = P::new(MetadataTransformer { base: Transformer::default(),
        legacy_decorators: opt.compiler_options.experimental_decorators.is_true(),
        resolver: opt.emit_resolver,
        language_version: opt.compiler_options.get_emit_script_target(),
        strict_null_checks: opt.compiler_options.get_strict_option_value(opt.compiler_options.strict_null_checks),
        parent: Cell::new(None),
        current_lexical_scope: Cell::new(None),
    });
    Some(tx.get().base.new_transformer(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit(n)), Some(opt.context)))
}

impl MetadataTransformer {
    // metadata.go:34
    fn visit(&self, node: P<Node>) -> Option<P<Node>> {
        if !node.subtree_facts().intersects(SubtreeFacts::ContainsDecorators) {
            return Some(node);
        }
        unimplemented!("emit: metadata.go not ported")
    }
}
