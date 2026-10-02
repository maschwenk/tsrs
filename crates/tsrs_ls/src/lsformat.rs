// PARTIAL port of ls/format.go (the formatting requests are phase 3): only what the phase-1 files use.
// Renamed: module `format` is Go package `internal/format`.

use tsrs_ast::{self as ast, CommentRange, Kind, Node, SourceFile};
use tsrs_core::P;
use tsrs_scanner as scanner;

use crate::astnav;
use crate::utilities::get_leading_comment_ranges_of_node;

// Unlike the TS implementation, this function *will not* compute default values for
// `precedingToken` and `tokenAtPosition`.
// It is the caller's responsibility to call `astnav.GetTokenAtPosition` to compute a default `tokenAtPosition`,
// or `astnav.FindPrecedingToken` to compute a default `precedingToken`.
// format.go:257
pub(crate) fn get_range_of_enclosing_comment(
    file: P<SourceFile>,
    position: i32,
    preceding_token: Option<P<Node>>,
    mut token_at_position: P<Node>,
) -> Option<CommentRange> {
    let jsdoc = ast::find_ancestor(token_at_position, |n| n.is_jsdoc());
    if let Some(jsdoc) = jsdoc {
        token_at_position = jsdoc.parent().unwrap();
    }
    let token_start = astnav::get_start_of_node(token_at_position, file, false /*includeJSDoc*/);
    if token_start <= position && position < token_at_position.end() {
        return None;
    }

    // Between two consecutive tokens, all comments are either trailing on the former
    // or leading on the latter (and none are in both lists).
    let trailing_ranges_of_previous_token = preceding_token.map(|t| scanner::get_trailing_comment_ranges(file.text(), t.end()));
    let leading_ranges_of_next_token = get_leading_comment_ranges_of_node(token_at_position, file);
    let comment_ranges = trailing_ranges_of_previous_token.into_iter().flatten().chain(leading_ranges_of_next_token.into_iter().flatten());
    for comment_range in comment_ranges {
        // The end marker of a single-line comment does not include the newline character.
        // In the following case where the cursor is at `^`, we are inside a comment:
        //
        //    // asdf   ^\n
        //
        // But for closed multi-line comments, we don't want to be inside the comment in the following case:
        //
        //    /* asdf */^
        //
        // Internally, we represent the end of the comment prior to the newline and at the '/', respectively.
        //
        // However, unterminated multi-line comments lack a `/`, end at the end of the file, and *do* contain their end.
        //
        if comment_range.text_range.contains_exclusive(position)
            || position == comment_range.end() && (comment_range.kind == Kind::SingleLineCommentTrivia || position == file.text().len() as i32)
        {
            return Some(comment_range);
        }
    }
    None
}
