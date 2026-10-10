use tsrs_ast::Kind;
use tsrs_core::stringutil;

use crate::utilities::normalize_jsdoc_type_source_text;
use crate::Scanner;

#[test]
fn test_dispatch_at_end_of_text() {
    let cases = [
        ("", Kind::EndOfFile),
        ("!", Kind::ExclamationToken),
        ("%", Kind::PercentToken),
        ("&", Kind::AmpersandToken),
        ("(", Kind::OpenParenToken),
        (")", Kind::CloseParenToken),
        ("*", Kind::AsteriskToken),
        ("+", Kind::PlusToken),
        (",", Kind::CommaToken),
        ("-", Kind::MinusToken),
        (".", Kind::DotToken),
        ("/", Kind::SlashToken),
        ("0", Kind::NumericLiteral),
        ("9", Kind::NumericLiteral),
        (":", Kind::ColonToken),
        (";", Kind::SemicolonToken),
        ("<", Kind::LessThanToken),
        ("=", Kind::EqualsToken),
        (">", Kind::GreaterThanToken),
        ("?", Kind::QuestionToken),
        ("[", Kind::OpenBracketToken),
        ("]", Kind::CloseBracketToken),
        ("^", Kind::CaretToken),
        ("{", Kind::OpenBraceToken),
        ("|", Kind::BarToken),
        ("}", Kind::CloseBraceToken),
        ("~", Kind::TildeToken),
        ("@", Kind::AtToken),
        ("#", Kind::PrivateIdentifier),
        ("$", Kind::Identifier),
        ("_", Kind::Identifier),
        ("a", Kind::Identifier),
        ("Z", Kind::Identifier),
        ("é", Kind::Identifier),
        ("\\", Kind::Unknown),
        ("\0", Kind::Unknown),
        ("'", Kind::StringLiteral),
        ("`", Kind::NoSubstitutionTemplateLiteral),
    ];
    for (text, expected) in cases {
        let mut s = Scanner::new();
        s.set_text(text);
        assert_eq!(s.scan(), expected, "{text:?}");
        assert_eq!(s.token_start(), 0, "{text:?}");
        assert_eq!(s.token_end(), text.len() as i32, "{text:?}");
        for _ in 0..2 {
            assert_eq!(s.scan(), Kind::EndOfFile, "{text:?}");
            assert_eq!(s.token_start(), text.len() as i32, "{text:?}");
            assert_eq!(s.token_end(), text.len() as i32, "{text:?}");
        }
    }
}

#[test]
fn test_dispatch_retry_preserves_token_state() {
    use tsrs_ast::TokenFlags;

    let text = "name \t\r\n/** @see x */ (";
    let mut s = Scanner::new();
    s.set_text(text);
    assert_eq!(s.scan(), Kind::Identifier);
    assert_eq!(s.scan(), Kind::OpenParenToken);
    assert_eq!(s.token_full_start(), 4);
    assert_eq!(s.token_start(), text.len() as i32 - 1);
    assert_eq!(s.token_end(), text.len() as i32);
    // Punctuation retains the previous token value, as in Go.
    assert_eq!(s.token_value(), "name");
    assert!(s.token_flags().contains(TokenFlags::PrecedingLineBreak | TokenFlags::PrecedingJSDocComment));
    assert_eq!(s.scan(), Kind::EndOfFile);
    assert_eq!(s.token_flags(), TokenFlags::None);

    s.set_text("\r\n* value");
    s.set_skip_jsdoc_leading_asterisks(true);
    assert_eq!(s.scan(), Kind::Identifier);
    assert_eq!(s.token_value(), "value");
    assert_eq!(s.token_full_start(), 0);
    assert_eq!(s.token_start(), 4);
    assert!(s.token_flags().contains(TokenFlags::PrecedingLineBreak | TokenFlags::PrecedingJSDocLeadingAsterisks));
}

#[test]
fn test_dispatch_returns_trivia_without_retry() {
    use tsrs_ast::TokenFlags;

    let mut s = Scanner::new();
    s.set_text(" \t\r\n// x\n/* y */+");
    s.set_skip_trivia(false);
    let cases = [
        (Kind::WhitespaceTrivia, 0, 2),
        (Kind::NewLineTrivia, 2, 4),
        (Kind::SingleLineCommentTrivia, 4, 8),
        (Kind::NewLineTrivia, 8, 9),
        (Kind::MultiLineCommentTrivia, 9, 16),
        (Kind::PlusToken, 16, 17),
        (Kind::EndOfFile, 17, 17),
    ];
    for (kind, start, end) in cases {
        assert_eq!(s.scan(), kind);
        assert_eq!((s.token_full_start(), s.token_start(), s.token_end()), (start, start, end));
        assert_eq!(s.token_flags().intersects(TokenFlags::PrecedingLineBreak), kind == Kind::NewLineTrivia);
    }
}

#[test]
fn test_scan_string_preserves_lone_surrogates() {
    let mut s = Scanner::new();
    s.set_text(r#""🦀퟿\ud800\ud801🦀""#);
    assert_eq!(s.scan(), Kind::StringLiteral);
    let expected = String::from("🦀")
        + &stringutil::encode_js_string_rune(0xD7FF)
        + &stringutil::encode_js_string_rune(0xD800)
        + &stringutil::encode_js_string_rune(0xD801)
        + "🦀";
    assert_eq!(s.token_value().as_bytes(), expected.as_bytes());
}

#[test]
fn test_normalize_jsdoc_type_source_text() {
    let tests: &[(&str, &str, &[&str])] = &[
        ("single line", " \t* \tFoo", &["Foo"]),
        ("ECMAScript line breaks", "Foo\r\n * Bar\r\t* Baz\u{2028} * Qux\u{2029}* Quux", &["Foo", "Bar", "Baz", "Qux", "Quux"]),
        ("blank and trailing lines", "Foo\r\n *\r\n", &["Foo", "", ""]),
        ("line without marker", "Foo\n  Bar", &["Foo", "Bar"]),
        ("only leading marker", "**Foo", &["*Foo"]),
    ];
    for &(name, text, expected_lines) in tests {
        let expected = expected_lines.join("\n");
        assert_eq!(normalize_jsdoc_type_source_text(text), expected, "{name}");
    }
}

#[test]
fn test_mark_rewind_restores_token_and_comment_directives() {
    let mut s = Scanner::new();
    s.set_text("a // @ts-ignore\nb /* @ts-expect-error */ c");
    assert_eq!(s.scan(), Kind::Identifier);
    let state = s.mark();
    assert_eq!(s.scan(), Kind::Identifier);
    assert_eq!(s.token_value(), "b");
    assert_eq!(s.comment_directives().len(), 1);
    assert_eq!(s.scan(), Kind::Identifier);
    assert_eq!(s.comment_directives().len(), 2);
    s.rewind(state);
    assert_eq!(s.token_value(), "a");
    assert_eq!(s.comment_directives().len(), 0);
    assert_eq!(s.scan(), Kind::Identifier);
    assert_eq!(s.token_value(), "b");
    assert_eq!(s.comment_directives().len(), 1);
}

#[test]
fn test_errors_are_buffered_in_report_order() {
    let mut s = Scanner::new();
    s.set_text("'abc\n1__2");
    s.set_on_error(true);
    assert_eq!(s.scan(), Kind::StringLiteral);
    let errors = s.take_errors();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].message.code(), tsrs_diagnostics::Unterminated_string_literal.code());
    assert_eq!((errors[0].start, errors[0].length), (4, 0));
    assert_eq!(s.scan(), Kind::NumericLiteral);
    assert_eq!(s.token_value(), "12");
    let errors = s.take_errors();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].message.code(), tsrs_diagnostics::Multiple_consecutive_numeric_separators_are_not_permitted.code());
    assert!(!s.has_errors());
}

#[test]
fn test_is_jsdoc_type_expression_or_child() {
    use tsrs_ast::{NodeFactory, NodeFlags};
    let f = NodeFactory::default();
    let with = |n: tsrs_core::P<tsrs_ast::Node>, flags: NodeFlags, parent: Option<tsrs_core::P<tsrs_ast::Node>>| {
        n.set_flags(flags);
        n.set_parent(parent);
        n
    };
    let name = f.new_identifier("T");
    let js_doc_type = with(f.new_type_reference_node(name, None), NodeFlags::JSDoc, None);
    let js_doc_type_child = with(f.new_identifier("a"), NodeFlags::JSDoc, Some(js_doc_type));
    let members = f.new_node_list(vec![]);
    let reparsed_type = with(f.new_type_literal_node(members), NodeFlags::Reparsed, None);
    let reparsed_type_child = with(f.new_identifier("b"), NodeFlags::Reparsed, Some(reparsed_type));
    let name = f.new_identifier("U");
    let ordinary_type = f.new_type_reference_node(name, None);
    let (tag_name, param_name) = (f.new_identifier("param"), f.new_identifier("p"));
    let js_doc_tag = with(
        f.new_jsdoc_parameter_or_property_tag(Kind::JSDocParameterTag, tag_name, param_name, false, None, false, None),
        NodeFlags::JSDoc,
        None,
    );
    let js_doc_tag_child = with(f.new_identifier("c"), NodeFlags::JSDoc, Some(js_doc_tag));
    let all_type = f.new_jsdoc_all_type();
    let type_expression = f.new_jsdoc_type_expression(all_type);

    let tests = [
        ("type expression", type_expression, true),
        ("JSDoc type", js_doc_type, true),
        ("JSDoc type child", js_doc_type_child, true),
        ("reparsed type", reparsed_type, true),
        ("reparsed type child", reparsed_type_child, true),
        ("ordinary type", ordinary_type, false),
        ("other JSDoc child", js_doc_tag_child, false),
    ];
    for (name, node, expected) in tests {
        assert_eq!(crate::utilities::is_jsdoc_type_expression_or_child(node), expected, "{name}");
    }
}

#[test]
fn test_get_text_of_node_from_jsdoc_type_preserves_asterisk_type() {
    use tsrs_ast::{NodeFactory, NodeFlags};
    let source_text = ["", " * *"].join("\n");
    let f = NodeFactory::default();
    let node = f.new_jsdoc_all_type();
    node.set_flags(NodeFlags::JSDoc);
    node.set_loc(tsrs_core::TextRange::new(0, source_text.len() as i32));
    assert_eq!(crate::get_text_of_node_from_source_text(&source_text, node, false /*includeTrivia*/), "*");
}
