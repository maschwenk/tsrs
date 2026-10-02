// PARTIAL port of symbols.go (document/workspace symbols are another wave's): only the helpers that rename and
// call hierarchy use.

use tsrs_ast::{self as ast, Kind, ModifierFlags, Node};
use tsrs_core::P;
use tsrs_lsproto as lsproto;

// symbols.go:619
pub(crate) fn is_inside_node_modules(file_name: &str) -> bool {
    file_name.contains("/node_modules/")
}

// symbols.go:669
pub(crate) fn get_symbol_kind_from_node(node: P<Node>) -> lsproto::SymbolKind {
    match node.kind() {
        Kind::SourceFile => {
            if ast::is_external_module(node.as_source_file_p()) {
                return lsproto::SymbolKind::Module;
            }
            return lsproto::SymbolKind::File;
        }
        Kind::ModuleDeclaration => return lsproto::SymbolKind::Namespace,
        Kind::ClassDeclaration | Kind::ClassExpression => return lsproto::SymbolKind::Class,
        Kind::InterfaceDeclaration => return lsproto::SymbolKind::Interface,
        Kind::TypeAliasDeclaration | Kind::JSDocTypedefTag | Kind::JSDocCallbackTag => return lsproto::SymbolKind::Class,
        Kind::EnumDeclaration => return lsproto::SymbolKind::Enum,
        Kind::VariableDeclaration => return lsproto::SymbolKind::Variable,
        Kind::ArrowFunction | Kind::FunctionDeclaration | Kind::FunctionExpression => return lsproto::SymbolKind::Function,
        Kind::GetAccessor | Kind::SetAccessor => return lsproto::SymbolKind::Property,
        Kind::MethodDeclaration | Kind::MethodSignature => return lsproto::SymbolKind::Method,
        Kind::PropertyDeclaration
        | Kind::PropertySignature
        | Kind::PropertyAssignment
        | Kind::ShorthandPropertyAssignment
        | Kind::SpreadAssignment
        | Kind::IndexSignature => return lsproto::SymbolKind::Property,
        Kind::CallSignature => return lsproto::SymbolKind::Method,
        Kind::ConstructSignature => return lsproto::SymbolKind::Constructor,
        Kind::Constructor | Kind::ClassStaticBlockDeclaration => return lsproto::SymbolKind::Constructor,
        Kind::TypeParameter => return lsproto::SymbolKind::TypeParameter,
        Kind::EnumMember => return lsproto::SymbolKind::EnumMember,
        Kind::Parameter => {
            if ast::has_syntactic_modifier(node, ModifierFlags::ParameterPropertyModifier) {
                return lsproto::SymbolKind::Property;
            }
            return lsproto::SymbolKind::Variable;
        }
        Kind::BinaryExpression | Kind::CallExpression => {
            let kind = ast::get_assignment_declaration_kind(node);
            match kind {
                ast::JSDeclarationKind::ThisProperty | ast::JSDeclarationKind::Property | ast::JSDeclarationKind::ObjectDefinePropertyValue => {
                    return lsproto::SymbolKind::Property;
                }
                _ => {}
            }
        }
        Kind::StringLiteral | Kind::NoSubstitutionTemplateLiteral | Kind::NumericLiteral => {
            // String literals used as property names (e.g., in Object.defineProperty)
            return lsproto::SymbolKind::Property;
        }
        _ => {}
    }
    lsproto::SymbolKind::Variable
}
