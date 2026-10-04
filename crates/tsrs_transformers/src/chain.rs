// Port of chain.go.

use crate::*;

// chain.go:10
pub struct chainedTransformer {
    pub base: Transformer,
    pub components: Vec<P<Transformer>>,
}

impl chainedTransformer {
    // chain.go:15
    fn visit(&self, node: P<Node>) -> P<Node> {
        if node.kind() != Kind::SourceFile {
            panic!("Chained transform passed non-sourcefile initial node");
        }
        let mut result = node.as_source_file_p();
        for t in &self.components {
            result = t.transform_source_file(result);
        }
        result.as_node()
    }
}

// chain.go:26
#[derive(Clone)]
pub struct TransformOptions {
    pub context: P<EmitContext>,
    pub compiler_options: P<CompilerOptions>,
    pub resolver: ReferenceResolverRef,
    pub emit_resolver: Option<Resolver>,
    pub get_emit_module_format_of_file: Rc<dyn Fn(P<SourceFile>) -> ModuleKind>,
}

// chain.go:34
pub type TransformerFactory = fn(opt: &TransformOptions) -> Option<P<Transformer>>;

// Chains transforms in left-to-right order, running them one at a time in order (as opposed to interleaved at each node)
// - the resulting combined transform only operates on SourceFile nodes
// chain.go:38. Go returns a closure; callers invoke the result immediately (`chain(&[a, b])(opt)`).
pub fn chain(transforms: &[TransformerFactory]) -> Box<dyn Fn(&TransformOptions) -> Option<P<Transformer>>> {
    if transforms.len() < 2 {
        if transforms.is_empty() {
            panic!("Expected some number of transforms to chain, but got none");
        }
        return Box::new(transforms[0]);
    }
    let transforms = transforms.to_vec();
    Box::new(move |opt: &TransformOptions| {
        let mut constructed: Vec<P<Transformer>> = Vec::with_capacity(transforms.len());
        for t in &transforms {
            // TODO: flatten nested chains?
            if let Some(result) = t(opt) {
                constructed.push(result);
            }
        }
        match constructed.len() {
            0 => return None,
            1 => return Some(constructed[0]),
            _ => {}
        }
        let ch = P::new(chainedTransformer { base: Transformer::default(), components: constructed });
        Some(ch.get().base.new_transformer(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| Some(ch.visit(n))), Some(opt.context)))
    })
}
