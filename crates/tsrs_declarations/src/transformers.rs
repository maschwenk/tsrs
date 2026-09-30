// Port of the parts of package `transformers` the declaration transformer uses: transformer.go (the `Transformer`
// base embedded by every transformer) and IsSimpleCopiableExpression / IsOriginalNodeSingleLine /
// IsSimpleInlineableExpression from utilities.go. chain.go, modifiervisitor.go and destructuring.go serve the
// script transformers only.

use crate::*;

/// Go `transformers.Transformer`, embedded by value in each transformer. Transformers are arena handles (`&self`
/// methods), so the fields Go assigns in `NewTransformer` are `OnceCell`s. The visitor carries no state besides its
/// callback, factory and hooks, so `visitor()` hands out a copy (Go shares one `*NodeVisitor`, which is re-entrant).
#[derive(Default)]
pub struct Transformer {
    pub emit_context: OnceCell<P<EmitContext>>,
    pub visitor: OnceCell<ast::NodeVisitor>,
}

impl Transformer {
    // transformer.go:14
    pub fn new_transformer(&self, visit: ast::VisitFn, emit_context: Option<P<EmitContext>>) {
        if self.emit_context.get().is_some() {
            panic!("Transformer already initialized");
        }
        let emit_context = emit_context.unwrap_or_else(printer::new_emit_context);
        let _ = self.emit_context.set(emit_context);
        let _ = self.visitor.set(emit_context.new_node_visitor(visit));
    }

    // transformer.go:28
    pub fn emit_context(&self) -> P<EmitContext> {
        *self.emit_context.get().unwrap()
    }

    // transformer.go:32
    pub fn visitor(&self) -> ast::NodeVisitor {
        self.visitor.get().unwrap().clone()
    }

    // transformer.go:36 (Go keeps `emitContext.Factory` in a field; it is the same handle)
    pub fn factory(&self) -> &'static printer::NodeFactory {
        &self.emit_context().get().factory
    }

    // transformer.go:40
    pub fn transform_source_file(&self, file: P<SourceFile>) -> P<SourceFile> {
        let visited = self.visitor().visit_source_file(file.as_node());
        visited.as_source_file_p()
    }
}

// utilities.go:241
pub fn is_simple_copiable_expression(expression: P<Node>) -> bool {
    ast::is_string_literal_like(expression) || ast::is_numeric_literal(expression) || ast::is_keyword_kind(expression.kind) || ast::is_identifier(expression)
}

// utilities.go:248
pub fn is_original_node_single_line(emit_context: P<EmitContext>, node: Option<P<Node>>) -> bool {
    let Some(node) = node else {
        return false;
    };
    let Some(original) = emit_context.most_original(Some(node)) else {
        return false;
    };
    let Some(source) = ast::get_source_file_of_node(Some(original)) else {
        return false;
    };
    let start_line = scanner::get_ecma_line_of_position(&*source, original.loc.get().pos());
    let end_line = scanner::get_ecma_line_of_position(&*source, original.loc.get().end());
    start_line == end_line
}

// utilities.go:270
pub fn is_simple_inlineable_expression(expression: P<Node>) -> bool {
    !ast::is_identifier(expression) && is_simple_copiable_expression(expression)
}
