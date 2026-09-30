use tsrs_ast::{self as ast, CommentRange, Kind, Node, NodeFactory};
use tsrs_core::{LanguageVariant, ScriptKind, P};
use tsrs_scanner as scanner;

pub(crate) fn get_language_variant(script_kind: ScriptKind) -> LanguageVariant {
    match script_kind {
        ScriptKind::TSX | ScriptKind::JSX | ScriptKind::JS | ScriptKind::JSON => {
            // .tsx and .jsx files are treated as jsx language variant.
            LanguageVariant::JSX
        }
        _ => LanguageVariant::Standard,
    }
}

pub(crate) fn token_is_identifier_or_keyword(token: Kind) -> bool {
    token >= Kind::Identifier
}

pub(crate) fn token_is_identifier_or_keyword_or_greater_than(token: Kind) -> bool {
    token == Kind::GreaterThanToken || token_is_identifier_or_keyword(token)
}

pub fn get_jsdoc_comment_ranges(f: &mut NodeFactory, comment_ranges: &[CommentRange], node: P<Node>, text: &str) -> Vec<CommentRange> {
    let mut comment_ranges = comment_ranges.to_vec();
    match node.kind {
        Kind::Parameter
        | Kind::TypeParameter
        | Kind::FunctionExpression
        | Kind::ArrowFunction
        | Kind::ParenthesizedExpression
        | Kind::VariableDeclaration
        | Kind::ExportSpecifier => {
            for comment_range in scanner::get_trailing_comment_ranges(f, text, node.pos()) {
                comment_ranges.push(comment_range);
            }
            for comment_range in scanner::get_leading_comment_ranges(f, text, node.pos()) {
                comment_ranges.push(comment_range);
            }
        }
        _ => {
            for comment_range in scanner::get_leading_comment_ranges(f, text, node.pos()) {
                comment_ranges.push(comment_range);
            }
        }
    }
    // Keep if the comment starts with '/**' but not if it is '/**/'
    let text = text.as_bytes();
    comment_ranges.retain(|comment| {
        let comment_start = comment.pos();
        let comment_len = comment.end() - comment_start;
        !(comment.end() > node.end()
            || comment_len < 4
            || text[(comment_start + 1) as usize] != b'*'
            || text[(comment_start + 2) as usize] != b'*'
            || text[(comment_start + 3) as usize] == b'/')
    });
    comment_ranges
}

pub(crate) fn is_keyword_or_punctuation(token: Kind) -> bool {
    ast::is_keyword_kind(token) || ast::is_punctuation_kind(token)
}

pub(crate) fn is_jsdoc_like_text(text: &str) -> bool {
    let text = text.as_bytes();
    text.len() >= 4 && text[1] == b'*' && text[2] == b'*' && text[3] != b'/'
}
