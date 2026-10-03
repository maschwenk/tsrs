// Port of modifiervisitor.go.

use crate::*;

// modifiervisitor.go:8
pub struct modifierVisitor {
    pub base: Transformer,
    pub allowed_modifiers: ModifierFlags,
}

impl modifierVisitor {
    // modifiervisitor.go:13
    fn visit(&self, node: P<Node>) -> Option<P<Node>> {
        let flags = ast::modifier_to_flag(node.kind());
        if flags != ModifierFlags::None && !flags.intersects(self.allowed_modifiers) {
            return None;
        }
        Some(node)
    }
}

// modifiervisitor.go:21
pub fn extract_modifiers(emit_context: P<EmitContext>, modifiers: Option<P<ModifierList>>, allowed: ModifierFlags) -> Option<P<ModifierList>> {
    let modifiers = modifiers?;
    let tx = P::new(modifierVisitor { base: Transformer::default(), allowed_modifiers: allowed });
    tx.get().base.new_transformer(Rc::new(move |_: &mut NodeVisitor, n: P<Node>| tx.visit(n)), Some(emit_context));
    tx.base.visitor().visit_modifiers(Some(modifiers))
}
