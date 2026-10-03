use crate::*;

// usestrict.go:9
pub fn new_use_strict_transformer(opts: &TransformOptions) -> Option<P<Transformer>> {
    let tx = P::new(useStrictTransformer { base: Transformer::default(), compiler_options: opts.compiler_options, get_emit_module_format_of_file: opts.get_emit_module_format_of_file.clone() });
    tx.base.new_transformer(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| Some(tx.visit(n))), Some(opts.context));
    Some(P::from_static(&tx.get().base))
}

pub struct useStrictTransformer {
    pub base: Transformer,
    compiler_options: P<CompilerOptions>,
    get_emit_module_format_of_file: Rc<dyn Fn(P<SourceFile>) -> ModuleKind>,
}

impl useStrictTransformer {
    // usestrict.go:23
    fn visit(&self, node: P<Node>) -> P<Node> {
        if node.kind() != Kind::SourceFile {
            return node;
        }
        self.visit_source_file(node.as_source_file_p())
    }

    // usestrict.go:30
    fn visit_source_file(&self, node: P<SourceFile>) -> P<Node> {
        if node.script_kind.get() == ScriptKind::JSON {
            return node.as_node();
        }

        let is_external_module = ast::is_external_module(node);
        let module_kind = self.compiler_options.get_emit_module_kind();
        let format = (self.get_emit_module_format_of_file)(node);

        // ESM is always strict. If the file is ESM, and CJS emit
        // has not been requested, then skip adding "use strict".
        if is_external_module && module_kind >= ModuleKind::ES2015 && (module_kind == ModuleKind::Preserve || format >= ModuleKind::ES2015) {
            return node.as_node();
        }

        let f = self.base.factory();
        let statements = f.ensure_use_strict(node.statements.nodes());
        let statement_list = f.new_node_list(statements);
        statement_list.loc.set(node.statements.loc.get());
        f.update_source_file(node.as_node(), statement_list, node.end_of_file_token)
    }
}
