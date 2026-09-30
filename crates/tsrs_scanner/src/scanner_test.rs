use tsrs_ast::Kind;
use tsrs_core::stringutil;

use crate::utilities::normalize_jsdoc_type_source_text;
use crate::Scanner;

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
