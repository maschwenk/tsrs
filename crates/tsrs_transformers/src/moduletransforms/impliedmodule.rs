use super::*;
use crate::*;

pub struct ImpliedModuleTransformer {
    pub base: Transformer,
    opts: TransformOptions,
    resolver: ReferenceResolverRef,
    get_emit_module_format_of_file: Rc<dyn Fn(P<SourceFile>) -> ModuleKind>,
    cjs_transformer: Cell<Option<P<Transformer>>>,
    esm_transformer: Cell<Option<P<Transformer>>>,
}

// impliedmodule.go:19
pub fn new_implied_module_transformer(opts: &TransformOptions) -> Option<P<Transformer>> {
    let tx = P::new(ImpliedModuleTransformer {
        base: Transformer::default(),
        opts: opts.clone(),
        resolver: opts.resolver,
        get_emit_module_format_of_file: opts.get_emit_module_format_of_file.clone(),
        cjs_transformer: Cell::new(None),
        esm_transformer: Cell::new(None),
    });
    tx.base.new_transformer(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| Some(tx.visit(n))), Some(opts.context));
    Some(P::from_static(&tx.get().base))
}

impl ImpliedModuleTransformer {
    // impliedmodule.go:24
    fn visit(&self, node: P<Node>) -> P<Node> {
        match node.kind() {
            Kind::SourceFile => self.visit_source_file(node.as_source_file_p()),
            _ => node,
        }
    }

    // impliedmodule.go:32
    fn visit_source_file(&self, node: P<SourceFile>) -> P<Node> {
        if node.is_declaration_file.get() {
            return node.as_node();
        }

        let format = (self.get_emit_module_format_of_file)(node);

        let transformer = if format >= ModuleKind::ES2015 {
            if self.esm_transformer.get().is_none() {
                self.esm_transformer.set(new_es_module_transformer(&self.opts));
            }
            self.esm_transformer.get().unwrap()
        } else {
            if self.cjs_transformer.get().is_none() {
                self.cjs_transformer.set(new_common_js_module_transformer(&self.opts));
            }
            self.cjs_transformer.get().unwrap()
        };

        transformer.transform_source_file(node).as_node()
    }
}
