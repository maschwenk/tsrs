use std::fmt;

use super::big;
use super::jsnum::*;
use crate::stringutil;

impl Number {
    // https://tc39.es/ecma262/2024/multipage/ecmascript-data-types-and-values.html#sec-numeric-types-number-tostring
    pub fn string(self) -> String {
        if self.is_nan() {
            return "NaN".to_string();
        } else if self.is_inf() {
            if self < Number(0.0) {
                return "-Infinity".to_string();
            }
            return "Infinity".to_string();
        }

        // Fast path: for safe integers, directly convert to string.
        if MinSafeInteger <= self && self <= MaxSafeInteger {
            let i = self.0 as i64;
            if i as f64 == self.0 {
                return i.to_string();
            }
        }

        dragonbox_ecma::Buffer::new().format_finite(self.0).to_owned()
    }
}

impl fmt::Display for Number {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.string())
    }
}

// https://tc39.es/ecma262/2024/multipage/abstract-operations.html#sec-stringtonumber
pub fn from_string(s: &str) -> Number {
    // Implementing StringToNumber exactly as written in the spec involves
    // writing a parser, along with the conversion of the parsed AST into the
    // actual value.
    //
    // We've already implemented a number parser in the scanner, but we can't
    // import it here. We also do not have the conversion implemented since we
    // previously just wrote `+literal` and let the runtime handle it.
    //
    // The strategy below is to instead break the number apart and fix it up
    // such that Go's own parsing functionality can handle it. This won't be
    // the fastest method, but it saves us from writing the full parser and
    // conversion logic.

    let s = s.trim_matches(is_str_white_space);

    match s {
        "" => return Number(0.0),
        "Infinity" | "+Infinity" => return inf(1),
        "-Infinity" => return inf(-1),
        _ => {}
    }

    for r in s.chars() {
        if !is_number_rune(r) {
            return nan();
        }
    }

    if let Some(n) = try_parse_int(s) {
        return n;
    }

    // Cut this off first so we can ensure -0 is returned as -0.
    let (mut s, negative) = cut_prefix(s, "-");

    if !negative {
        (s, _) = cut_prefix(s, "+");
    }

    let first = s.chars().next().unwrap_or(char::REPLACEMENT_CHARACTER);
    if !stringutil::is_digit(first) && first != '.' {
        return nan();
    }

    let f = parse_float_string(s);
    if f.is_nan() {
        return nan();
    }

    let sign = if negative { -1.0 } else { 1.0 };
    Number(f.copysign(sign))
}

fn cut_prefix<'a>(s: &'a str, prefix: &str) -> (&'a str, bool) {
    match s.strip_prefix(prefix) {
        Some(rest) => (rest, true),
        None => (s, false),
    }
}

pub(crate) fn is_str_white_space(r: char) -> bool {
    // This is different than stringutil.IsWhiteSpaceLike.

    // https://tc39.es/ecma262/2024/multipage/ecmascript-language-lexical-grammar.html#prod-LineTerminator
    // https://tc39.es/ecma262/2024/multipage/ecmascript-language-lexical-grammar.html#prod-WhiteSpace

    match r {
        // LineTerminator
        '\n' | '\r' | '\u{2028}' | '\u{2029}' => return true,
        // WhiteSpace
        '\t' | '\u{000B}' | '\u{000C}' | '\u{FEFF}' => return true,
        _ => {}
    }

    // WhiteSpace (unicode.Zs)
    matches!(r, '\u{0020}' | '\u{00A0}' | '\u{1680}' | '\u{2000}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}')
}

pub(crate) fn try_parse_int(s: &str) -> Option<Number> {
    let mut s = s;
    let mut i: i64 = 0;
    let mut err = false;
    let mut has_int_result = false;

    if s.len() > 2 {
        let (prefix, rest) = (&s[..2], &s[2..]);
        match prefix {
            "0b" | "0B" => {
                if !is_all_binary_digits(rest) {
                    return Some(nan());
                }
                (i, err) = parse_int(rest, 2);
                has_int_result = true;
            }
            "0o" | "0O" => {
                if !is_all_octal_digits(rest) {
                    return Some(nan());
                }
                (i, err) = parse_int(rest, 8);
                has_int_result = true;
            }
            "0x" | "0X" => {
                if !is_all_hex_digits(rest) {
                    return Some(nan());
                }
                (i, err) = parse_int(rest, 16);
                has_int_result = true;
            }
            _ => {}
        }
    }

    if !has_int_result {
        // StringToNumber does not parse leading zeros as octal.
        s = trim_leading_zeros(s);
        if !is_all_digits(s) {
            return None;
        }
        (i, err) = parse_int(s, 10);
        has_int_result = true;
    }

    if has_int_result && !err {
        return Some(Number(i as f64));
    }

    // Using this to parse large integers.
    let Some(bi) = big::Int::set_string(s, 0) else {
        return Some(nan());
    };

    let f = bi.float64();
    Some(Number(f))
}

// Go's strconv.ParseInt(s, base, 64), reporting only whether it failed.
fn parse_int(s: &str, base: u32) -> (i64, bool) {
    match i64::from_str_radix(s, base) {
        Ok(i) => (i, false),
        Err(_) => (0, true),
    }
}

pub(crate) fn parse_float_string(s: &str) -> f64 {
    let has_dot;
    let has_exp;

    // <a>
    // <a>.<b>
    // <a>.<b>e<c>
    // <a>e<c>
    let mut a;
    let mut b = "";
    let c;

    match s.split_once('.') {
        Some((before, rest)) => {
            a = before;
            has_dot = true;
            // <a>.<b>
            // <a>.<b>e<c>
            (b, c, has_exp) = cut_any(rest, "eE");
        }
        None => {
            has_dot = false;
            // <a>
            // <a>e<c>
            (a, c, has_exp) = cut_any(s, "eE");
        }
    }

    let mut sb = String::with_capacity(a.len() + b.len() + c.len() + 3);

    if a.is_empty() {
        if has_dot && b.is_empty() {
            return f64::NAN;
        }
        if has_exp && c.is_empty() {
            return f64::NAN;
        }
        sb.push('0');
    } else {
        a = trim_leading_zeros(a);
        if !is_all_digits(a) {
            return f64::NAN;
        }
        sb.push_str(a);
    }

    if has_dot {
        sb.push('.');
        if b.is_empty() {
            sb.push('0');
        } else {
            b = trim_trailing_zeros(b);
            if !is_all_digits(b) {
                return f64::NAN;
            }
            sb.push_str(b);
        }
    }

    if has_exp {
        sb.push('e');

        let (mut c, negative) = cut_prefix(c, "-");
        if negative {
            sb.push('-');
        } else {
            (c, _) = cut_prefix(c, "+");
        }
        c = trim_leading_zeros(c);
        if !is_all_digits(c) {
            return f64::NAN;
        }
        sb.push_str(c);
    }

    string_to_float64(&sb)
}

pub(crate) fn cut_any<'a>(s: &'a str, cutset: &str) -> (&'a str, &'a str, bool) {
    if let Some(i) = s.find(|r: char| cutset.contains(r)) {
        let before = &s[..i];
        let after_and_found = &s[i..];
        let size = after_and_found.chars().next().unwrap().len_utf8();
        let after = &after_and_found[size..];
        return (before, after, true);
    }
    (s, "", false)
}

pub(crate) fn trim_leading_zeros(s: &str) -> &str {
    let mut s = s;
    if s.starts_with('0') {
        s = s.trim_start_matches('0');
        if s.is_empty() {
            return "0";
        }
    }
    s
}

pub(crate) fn trim_trailing_zeros(s: &str) -> &str {
    let mut s = s;
    if s.ends_with('0') {
        s = s.trim_end_matches('0');
        if s.is_empty() {
            return "0";
        }
    }
    s
}

pub(crate) fn string_to_float64(s: &str) -> f64 {
    // Out-of-range values parse to ±Inf (Go reports ErrRange alongside ±Inf).
    match s.parse::<f64>() {
        Ok(f) => f,
        Err(_) => f64::NAN,
    }
}

pub(crate) fn is_all_digits(s: &str) -> bool {
    for r in s.chars() {
        if !stringutil::is_digit(r) {
            return false;
        }
    }
    true
}

pub(crate) fn is_all_binary_digits(s: &str) -> bool {
    for r in s.chars() {
        if r != '0' && r != '1' {
            return false;
        }
    }
    true
}

pub(crate) fn is_all_octal_digits(s: &str) -> bool {
    for r in s.chars() {
        if !stringutil::is_octal_digit(r) {
            return false;
        }
    }
    true
}

pub(crate) fn is_all_hex_digits(s: &str) -> bool {
    for r in s.chars() {
        if !stringutil::is_hex_digit(r) {
            return false;
        }
    }
    true
}

pub(crate) fn is_number_rune(r: char) -> bool {
    if stringutil::is_digit(r) {
        return true;
    }

    if ('a'..='f').contains(&r) {
        return true;
    }

    if ('A'..='F').contains(&r) {
        return true;
    }

    matches!(r, '.' | '-' | '+' | 'x' | 'X' | 'o' | 'O')
}
