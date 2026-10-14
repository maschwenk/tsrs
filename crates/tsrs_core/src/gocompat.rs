// Compatibility for Go-style strings used in errors and baselines. The profiles retain the
// existing callers' escaping differences; they do not change those approximations to strconv.Quote.

use std::fmt::Write;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuoteStyle {
    /// IPC headers and content-mapper errors: escape Unicode whitespace and selected format characters.
    Header,
    /// API transport errors: hex escapes for ASCII controls other than tab and line endings.
    Api,
    /// LSP errors: replace invalid UTF-8, retaining other Unicode characters.
    Lsp,
    /// Mapper harness errors: escape Unicode controls and whitespace.
    MapperTest,
    /// Fourslash baselines: escape Unicode controls, retaining Unicode whitespace.
    Fourslash,
}
fn escaped(c: char, style: QuoteStyle) -> bool {
    match style {
        QuoteStyle::Header => {
            c != ' '
                && (c.is_control()
                    || c.is_whitespace()
                    || matches!(c,'\u{ad}'|'\u{200b}'..='\u{200f}'|'\u{202a}'..='\u{202e}'|'\u{2060}'..='\u{2064}'|'\u{feff}'))
        }
        QuoteStyle::MapperTest => c.is_control() || (c.is_whitespace() && c != ' '),
        QuoteStyle::Fourslash => c.is_control(),
        _ => false,
    }
}
fn push_char(out: &mut String, c: char, style: QuoteStyle) {
    match c {
        '"' => out.push_str("\\\""),
        '\\' => out.push_str("\\\\"),
        '\n' => out.push_str("\\n"),
        '\r' => out.push_str("\\r"),
        '\t' => out.push_str("\\t"),
        '\x07' if style != QuoteStyle::Api => out.push_str("\\a"),
        '\x08' if style != QuoteStyle::Api => out.push_str("\\b"),
        '\x0c' if style != QuoteStyle::Api => out.push_str("\\f"),
        '\x0b' if style != QuoteStyle::Api => out.push_str("\\v"),
        c if (c as u32) < 0x20 || c == '\x7f' => {
            let _ = write!(out, "\\x{:02x}", c as u32);
        }
        c if escaped(c, style) => {
            if style == QuoteStyle::MapperTest || (c as u32) < 0x10000 {
                let _ = write!(out, "\\u{:04x}", c as u32);
            } else {
                let _ = write!(out, "\\U{:08x}", c as u32);
            }
        }
        c => out.push(c),
    }
}
pub fn quote(b: &[u8], style: QuoteStyle) -> String {
    let mut out = String::with_capacity(b.len() + 2);
    out.push('"');
    if style == QuoteStyle::Lsp {
        for c in crate::utf8::from_utf8_lossy(b).chars() {
            push_char(&mut out, c, style);
        }
    } else {
        for chunk in b.utf8_chunks() {
            for c in chunk.valid().chars() {
                push_char(&mut out, c, style);
            }
            for b in chunk.invalid() {
                let _ = write!(out, "\\x{b:02x}");
            }
        }
    }
    out.push('"');
    out
}
pub fn parse_int64(b: &[u8], style: QuoteStyle) -> Result<i64, String> {
    let digits = b
        .strip_prefix(b"+")
        .or_else(|| b.strip_prefix(b"-"))
        .unwrap_or(b);
    let error = if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        "invalid syntax"
    } else {
        if let Ok(n) = std::str::from_utf8(b)
            .expect("validated sign and digits are ASCII")
            .parse::<i64>()
        {
            return Ok(n);
        }
        "value out of range"
    };
    Err(format!(
        "strconv.ParseInt: parsing {}: {error}",
        quote(b, style)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_styles_preserve_control_and_unicode_escapes() {
        let input = "\x07\x08\x0c\x0b\n\r\t\"\u{7f}\u{85}\u{a0}\u{200b}\u{feff}\\";
        for (style, expected) in [
            (
                QuoteStyle::Header,
                "\"\\a\\b\\f\\v\\n\\r\\t\\\"\\x7f\\u0085\\u00a0\\u200b\\ufeff\\\\\"",
            ),
            (
                QuoteStyle::Api,
                "\"\\x07\\x08\\x0c\\x0b\\n\\r\\t\\\"\\x7f\u{85}\u{a0}\u{200b}\u{feff}\\\\\"",
            ),
            (
                QuoteStyle::Lsp,
                "\"\\a\\b\\f\\v\\n\\r\\t\\\"\\x7f\u{85}\u{a0}\u{200b}\u{feff}\\\\\"",
            ),
            (
                QuoteStyle::MapperTest,
                "\"\\a\\b\\f\\v\\n\\r\\t\\\"\\x7f\\u0085\\u00a0\u{200b}\u{feff}\\\\\"",
            ),
            (
                QuoteStyle::Fourslash,
                "\"\\a\\b\\f\\v\\n\\r\\t\\\"\\x7f\\u0085\u{a0}\u{200b}\u{feff}\\\\\"",
            ),
        ] {
            assert_eq!(quote(input.as_bytes(), style), expected, "{style:?}");
            assert_eq!(quote(" space é😀".as_bytes(), style), "\" space é😀\"");
        }
    }

    #[test]
    fn quote_invalid_utf8_uses_each_callers_style() {
        for (input, escaped, replaced) in [
            (b"\xff".as_slice(), "\"\\xff\"", "\"�\""),
            (b"\xc0\xaf".as_slice(), "\"\\xc0\\xaf\"", "\"��\""),
            (b"a\xf0\x9f".as_slice(), "\"a\\xf0\\x9f\"", "\"a�\""),
        ] {
            assert_eq!(quote(input, QuoteStyle::Lsp), replaced);
            for style in [
                QuoteStyle::Header,
                QuoteStyle::Api,
                QuoteStyle::MapperTest,
                QuoteStyle::Fourslash,
            ] {
                assert_eq!(quote(input, style), escaped, "{style:?}");
            }
        }
    }

    #[test]
    fn parse_int64_preserves_syntax_range_and_error_text() {
        for style in [QuoteStyle::Header, QuoteStyle::Api, QuoteStyle::Lsp] {
            for (input, value) in [
                ("+0001", 1),
                ("-0", 0),
                ("9223372036854775807", i64::MAX),
                ("-9223372036854775808", i64::MIN),
            ] {
                assert_eq!(parse_int64(input.as_bytes(), style), Ok(value));
            }
            for (input, expected) in [
                ("", "strconv.ParseInt: parsing \"\": invalid syntax"),
                ("+", "strconv.ParseInt: parsing \"+\": invalid syntax"),
                ("--1", "strconv.ParseInt: parsing \"--1\": invalid syntax"),
                (" 1", "strconv.ParseInt: parsing \" 1\": invalid syntax"),
                (
                    "999999999999999999999999x",
                    "strconv.ParseInt: parsing \"999999999999999999999999x\": invalid syntax",
                ),
                (
                    "9223372036854775808",
                    "strconv.ParseInt: parsing \"9223372036854775808\": value out of range",
                ),
                (
                    "-9223372036854775809",
                    "strconv.ParseInt: parsing \"-9223372036854775809\": value out of range",
                ),
            ] {
                assert_eq!(
                    parse_int64(input.as_bytes(), style),
                    Err(expected.to_string())
                );
            }
        }
        assert_eq!(
            parse_int64(b"\xff", QuoteStyle::Header),
            Err("strconv.ParseInt: parsing \"\\xff\": invalid syntax".to_string()),
        );
    }
}
