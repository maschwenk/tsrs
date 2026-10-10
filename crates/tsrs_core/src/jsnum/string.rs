use std::fmt;

use super::big;
use super::jsnum::*;

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

        // Otherwise, the Go json package handles this correctly.
        json_marshal_float64(self.0)
    }
}

impl fmt::Display for Number {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.string())
    }
}

// Go's json.Marshal of a float64 (jsonwire.AppendFloat with bits == 64).
fn json_marshal_float64(src: f64) -> String {
    let abs = src.abs();
    let mut fmt = b'f';
    if abs != 0.0 && (abs < 1e-6 || abs >= 1e21) {
        fmt = b'e';
    }
    let mut dst = strconv_format_float(src, fmt).into_bytes();
    if fmt == b'e' {
        // Clean up e-09 to e-9.
        let n = dst.len();
        if n >= 4 && dst[n - 4] == b'e' && dst[n - 3] == b'-' && dst[n - 2] == b'0' {
            dst[n - 2] = dst[n - 1];
            dst.truncate(n - 1);
        }
    }
    String::from_utf8(dst).unwrap()
}

// Go's strconv.FormatFloat(f, fmt, -1, 64) for fmt 'f' and 'e'.
fn strconv_format_float(f: f64, fmt: u8) -> String {
    let neg = f.is_sign_negative();
    let (d, dp) = shortest_decimal(f.abs());
    if fmt == b'e' {
        fmt_e(neg, &d, dp)
    } else {
        fmt_f(neg, &d, dp)
    }
}

// The shortest decimal digits that round-trip to f (f >= 0), with the decimal
// point dp digits from the left, like Go's ryuFtoaShortest. Rust's float
// formatting yields the same digits except that it breaks an exact tie between
// two equally close candidates upward, where Go picks the even last digit.
fn shortest_decimal(f: f64) -> (Vec<u8>, i32) {
    if f == 0.0 {
        return (Vec::new(), 0);
    }
    let s = format!("{f:e}");
    let (mantissa, exp) = s.split_once('e').unwrap();
    let mut d: Vec<u8> = mantissa.bytes().filter(|&c| c != b'.').collect();
    let dp = exp.parse::<i32>().unwrap() + 1;

    let last = *d.last().unwrap();
    if (last - b'0') % 2 == 1 {
        let (exact, exact_dp) = exact_decimal(f);
        if exact_dp == dp && exact.len() == d.len() + 1 && exact[exact.len() - 1] == b'5' {
            let mut candidate = d.clone();
            *candidate.last_mut().unwrap() -= 1;
            if exact[..candidate.len()] == candidate[..] {
                let text = format!("0.{}e{}", std::str::from_utf8(&candidate).unwrap(), dp);
                if text.parse::<f64>() == Ok(f) {
                    d = candidate;
                }
            }
        }
    }
    (d, dp)
}

// The exact decimal digits of f (f > 0, finite) without trailing zeros, with
// the decimal point dp digits from the left.
fn exact_decimal(f: f64) -> (Vec<u8>, i32) {
    let bits = f.to_bits();
    let exp = ((bits >> 52) & 0x7FF) as i64;
    let mut mant = bits & ((1u64 << 52) - 1);
    let e2 = if exp == 0 {
        -1074
    } else {
        mant |= 1u64 << 52;
        exp - 1075
    };
    let m = big::Int::new(mant as i64);
    let (n, frac_digits) = if e2 >= 0 {
        (m.lsh(e2 as usize), 0i64)
    } else {
        (m.mul(&big::Int::new(5).exp(&big::Int::new(-e2))), -e2)
    };
    let mut digits = n.string().into_bytes();
    let dp = digits.len() as i64 - frac_digits;
    while digits.last() == Some(&b'0') {
        digits.pop();
    }
    (digits, dp as i32)
}

// Go's strconv fmtE with the shortest precision.
fn fmt_e(neg: bool, d: &[u8], dp: i32) -> String {
    let mut dst = String::new();
    if neg {
        dst.push('-');
    }
    let nd = d.len();
    let prec = nd.saturating_sub(1);

    // first digit
    let mut ch = b'0';
    if nd != 0 {
        ch = d[0];
    }
    dst.push(ch as char);

    // .moredigits
    if prec > 0 {
        dst.push('.');
        let mut i = 1;
        let m = nd.min(prec + 1);
        if i < m {
            dst.push_str(std::str::from_utf8(&d[i..m]).unwrap());
            i = m;
        }
        while i <= prec {
            dst.push('0');
            i += 1;
        }
    }

    // e±
    dst.push('e');
    let mut exp = dp - 1;
    if nd == 0 {
        exp = 0;
    }
    if exp < 0 {
        dst.push('-');
        exp = -exp;
    } else {
        dst.push('+');
    }

    // dd or ddd
    if exp < 10 {
        dst.push('0');
    }
    dst.push_str(&exp.to_string());
    dst
}

// Go's strconv fmtF with the shortest precision.
fn fmt_f(neg: bool, d: &[u8], dp: i32) -> String {
    let mut dst = String::new();
    if neg {
        dst.push('-');
    }
    let nd = d.len() as i32;
    let prec = (nd - dp).max(0);

    // integer, padded with zeros as needed.
    if dp > 0 {
        let mut m = nd.min(dp);
        dst.push_str(std::str::from_utf8(&d[..m as usize]).unwrap());
        while m < dp {
            dst.push('0');
            m += 1;
        }
    } else {
        dst.push('0');
    }

    // fraction
    if prec > 0 {
        dst.push('.');
        for i in 1..=prec {
            let mut ch = b'0';
            let j = dp + i - 1;
            if 0 <= j && j < nd {
                ch = d[j as usize];
            }
            dst.push(ch as char);
        }
    }
    dst
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

    for b in s.bytes() {
        if !is_number_byte(b) {
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

    if !s.as_bytes().first().is_some_and(|b| b.is_ascii_digit() || *b == b'.') {
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
    if cutset == "eE" {
        // An ASCII match is always a UTF-8 boundary and occupies one byte.
        return match s.bytes().position(|b| matches!(b, b'e' | b'E')) {
            Some(i) => (&s[..i], &s[i + 1..], true),
            None => (s, "", false),
        };
    }
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
    s.bytes().all(|b| b.is_ascii_digit())
}

pub(crate) fn is_all_binary_digits(s: &str) -> bool {
    s.bytes().all(|b| matches!(b, b'0' | b'1'))
}

pub(crate) fn is_all_octal_digits(s: &str) -> bool {
    s.bytes().all(|b| matches!(b, b'0'..=b'7'))
}

pub(crate) fn is_all_hex_digits(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_hexdigit())
}

pub(crate) fn is_number_byte(b: u8) -> bool {
    if b.is_ascii_digit() {
        return true;
    }

    if (b'a'..=b'f').contains(&b) {
        return true;
    }

    if (b'A'..=b'F').contains(&b) {
        return true;
    }

    matches!(b, b'.' | b'-' | b'+' | b'x' | b'X' | b'o' | b'O')
}
