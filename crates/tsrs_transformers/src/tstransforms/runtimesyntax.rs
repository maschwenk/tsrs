use crate::*;

// runtimesyntax.go:989
pub(crate) fn get_innermost_module_declaration_from_dotted_module(module_declaration: P<Node>) -> P<Node> {
    let mut module_declaration = module_declaration;
    while let Some(body) = module_declaration.body() {
        if body.kind() != Kind::ModuleDeclaration {
            break;
        }
        module_declaration = body;
    }
    module_declaration
}
