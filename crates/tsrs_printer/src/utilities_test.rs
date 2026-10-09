// Port of utilities_test.go.

use tsrs_ast::*;
use tsrs_core::*;

use crate::*;

#[test]
fn test_escape_string() {
    let data: &[(&str, QuoteChar, &str)] = &[
        ("", QuoteChar::DoubleQuote, ""),
        ("abc", QuoteChar::DoubleQuote, "abc"),
        ("ab\"c", QuoteChar::DoubleQuote, "ab\\\"c"),
        ("ab\tc", QuoteChar::DoubleQuote, "ab\\tc"),
        ("ab\nc", QuoteChar::DoubleQuote, "ab\\nc"),
        ("ab'c", QuoteChar::DoubleQuote, "ab'c"),
        ("ab'c", QuoteChar::SingleQuote, "ab\\'c"),
        ("ab\"c", QuoteChar::SingleQuote, "ab\"c"),
        ("ab`c", QuoteChar::Backtick, "ab\\`c"),
        ("\u{001f}", QuoteChar::Backtick, "\\u001F"),
    ];
    for (i, &(s, quote_char, expected)) in data.iter().enumerate() {
        assert_eq!(escape_string(s, quote_char), expected, "[{}] escapeString({:?}, {:?})", i, s, quote_char);
    }
}

#[test]
fn test_escape_non_ascii_string() {
    let data: &[(&str, QuoteChar, &str)] = &[
        ("", QuoteChar::DoubleQuote, ""),
        ("abc", QuoteChar::DoubleQuote, "abc"),
        ("ab\"c", QuoteChar::DoubleQuote, "ab\\\"c"),
        ("ab\tc", QuoteChar::DoubleQuote, "ab\\tc"),
        ("ab\nc", QuoteChar::DoubleQuote, "ab\\nc"),
        ("ab'c", QuoteChar::DoubleQuote, "ab'c"),
        ("ab'c", QuoteChar::SingleQuote, "ab\\'c"),
        ("ab\"c", QuoteChar::SingleQuote, "ab\"c"),
        ("ab`c", QuoteChar::Backtick, "ab\\`c"),
        ("ab\u{008f}c", QuoteChar::DoubleQuote, "ab\\u008Fc"),
        ("𝟘𝟙", QuoteChar::DoubleQuote, "\\uD835\\uDFD8\\uD835\\uDFD9"),
    ];
    for (i, &(s, quote_char, expected)) in data.iter().enumerate() {
        assert_eq!(escape_non_ascii_string(s, quote_char), expected, "[{}] escapeNonAsciiString({:?}, {:?})", i, s, quote_char);
    }
}

#[test]
fn test_escape_jsx_attribute_string() {
    let data: &[(&str, QuoteChar, &str)] = &[
        ("", QuoteChar::DoubleQuote, ""),
        ("abc", QuoteChar::DoubleQuote, "abc"),
        ("ab\"c", QuoteChar::DoubleQuote, "ab&quot;c"),
        ("ab\tc", QuoteChar::DoubleQuote, "ab&#x9;c"),
        ("ab\nc", QuoteChar::DoubleQuote, "ab&#xA;c"),
        ("ab'c", QuoteChar::DoubleQuote, "ab'c"),
        ("ab'c", QuoteChar::SingleQuote, "ab&apos;c"),
        ("ab\"c", QuoteChar::SingleQuote, "ab\"c"),
        ("ab\u{008f}c", QuoteChar::DoubleQuote, "ab\u{008F}c"),
        ("𝟘𝟙", QuoteChar::DoubleQuote, "𝟘𝟙"),
    ];
    for (i, &(s, quote_char, expected)) in data.iter().enumerate() {
        assert_eq!(escape_jsx_attribute_string(s, quote_char), expected, "[{}] escapeJsxAttributeString({:?}, {:?})", i, s, quote_char);
    }
}

#[test]
fn test_escape_lone_surrogate() {
    // The scanner stores a lone surrogate as its WTF-8 sentinel; the printer escapes it as a UTF-16 escape.
    let s = stringutil::encode_js_string_rune(0xD800);
    assert_eq!(escape_string(&s, QuoteChar::DoubleQuote), "\\uD800");
}

#[test]
fn test_is_recognized_triple_slash_comment() {
    let data: &[(&str, Option<Kind>, bool)] = &[
        ("", Some(Kind::MultiLineCommentTrivia), false),
        ("", Some(Kind::SingleLineCommentTrivia), false),
        ("/a", None, false),
        ("//", None, false),
        ("//a", None, false),
        ("///", None, false),
        ("///a", None, false),
        ("///<reference path=\"foo\" />", None, true),
        ("///<reference types=\"foo\" />", None, true),
        ("///<reference lib=\"foo\" />", None, true),
        ("///<reference no-default-lib=\"foo\" />", None, true),
        ("///<amd-dependency path=\"foo\" />", None, true),
        ("///<amd-module />", None, true),
        ("/// <reference path=\"foo\" />", None, true),
        ("/// <reference types=\"foo\" />", None, true),
        ("/// <reference lib=\"foo\" />", None, true),
        ("/// <reference no-default-lib=\"foo\" />", None, true),
        ("/// <amd-dependency path=\"foo\" />", None, true),
        ("/// <amd-module />", None, true),
        ("/// <reference path=\"foo\"/>", None, true),
        ("/// <reference types=\"foo\"/>", None, true),
        ("/// <reference lib=\"foo\"/>", None, true),
        ("/// <reference no-default-lib=\"foo\"/>", None, true),
        ("/// <amd-dependency path=\"foo\"/>", None, true),
        ("/// <amd-module/>", None, true),
        ("/// <reference path='foo' />", None, true),
        ("/// <reference types='foo' />", None, true),
        ("/// <reference lib='foo' />", None, true),
        ("/// <reference no-default-lib='foo' />", None, true),
        ("/// <amd-dependency path='foo' />", None, true),
        ("/// <reference path=\"foo\" />  ", None, true),
        ("/// <reference types=\"foo\" />  ", None, true),
        ("/// <reference lib=\"foo\" />  ", None, true),
        ("/// <reference no-default-lib=\"foo\" />  ", None, true),
        ("/// <amd-dependency path=\"foo\" />  ", None, true),
        ("/// <amd-module />  ", None, true),
        ("/// <foo />", None, false),
        ("/// <reference />", None, false),
        ("/// <amd-dependency />", None, false),
    ];
    for (i, &(s, kind, expected)) in data.iter().enumerate() {
        let comment_range = match kind {
            Some(kind) => CommentRange { text_range: TextRange::default(), kind, has_trailing_new_line: false },
            None => CommentRange {
                text_range: TextRange::new(0, u32::try_from(s.len()).unwrap()),
                kind: Kind::SingleLineCommentTrivia,
                has_trailing_new_line: false,
            },
        };
        assert_eq!(is_recognized_triple_slash_comment(s, comment_range), expected, "[{}] isRecognizedTripleSlashComment({:?})", i, s);
    }
}
