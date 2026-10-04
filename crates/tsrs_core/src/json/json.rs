// Go's json package wraps encoding/json/v2 (reflection-based). The port provides a JSON value
// type with Go-compatible encoding (v2 defaults: no HTML escaping, invalid UTF-8 allowed,
// ES6 number formatting) and a strict RFC 8259 parser.

use crate::collections::OrderedMap;
use std::fmt::Write;

#[derive(Clone, Debug, PartialEq, Default)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    Number(f64),
    /// An exact integer to encode as written (never produced by `unmarshal`): Go `int`/`int64` values that f64
    /// cannot hold, e.g. a `*int` option echoed back to an API client.
    Integer(i64),
    String(String),
    Array(Vec<Value>),
    Object(OrderedMap<String, Value>),
}

pub fn marshal(v: &Value) -> Result<String, String> {
    marshal_with_capacity(v, 0)
}

/// `marshal` into a buffer that starts with `capacity` bytes, for callers that know roughly how large the output is.
pub fn marshal_with_capacity(v: &Value, capacity: usize) -> Result<String, String> {
    let mut out = String::with_capacity(capacity);
    write_value(&mut out, v, "", "", 0)?;
    Ok(out)
}

pub fn marshal_indent(v: &Value, prefix: &str, indent: &str) -> Result<String, String> {
    if prefix.is_empty() && indent.is_empty() {
        // WithIndentPrefix and WithIndent imply multiline output, so skip them.
        return marshal(v);
    }
    let mut out = String::new();
    write_value(&mut out, v, prefix, indent, 0)?;
    Ok(out)
}

/// `json.Marshal` of a Go string.
pub fn marshal_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    write_string(&mut out, s);
    out
}

/// `json.Marshal` of a Go float64.
pub fn marshal_f64(f: f64) -> Result<String, String> {
    if !f.is_finite() {
        return Err(format!("json: cannot marshal invalid value: {f}"));
    }
    Ok(es6_number_string(f))
}

// ECMAScript Number::toString layout of the shortest round-trip digits.
fn es6_number_string(f: f64) -> String {
    if f == 0.0 {
        return "0".to_string();
    }
    let sci = format!("{:e}", f.abs());
    let (mantissa, exp) = sci.split_once('e').unwrap();
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i32;
    let n = exp.parse::<i32>().unwrap() + 1;
    let mut out = String::new();
    if f < 0.0 {
        out.push('-');
    }
    if k <= n && n <= 21 {
        out.push_str(&digits);
        out.extend(std::iter::repeat_n('0', (n - k) as usize));
    } else if 0 < n && n <= 21 {
        out.push_str(&digits[..n as usize]);
        out.push('.');
        out.push_str(&digits[n as usize..]);
    } else if -6 < n && n <= 0 {
        out.push_str("0.");
        out.extend(std::iter::repeat_n('0', (-n) as usize));
        out.push_str(&digits);
    } else {
        out.push_str(&digits[..1]);
        if k > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if n - 1 >= 0 { '+' } else { '-' });
        out.push_str(&(n - 1).abs().to_string());
    }
    out
}

fn newline(out: &mut String, prefix: &str, indent: &str, depth: usize) {
    out.push('\n');
    out.push_str(prefix);
    for _ in 0..depth {
        out.push_str(indent);
    }
}

fn write_value(out: &mut String, v: &Value, prefix: &str, indent: &str, depth: usize) -> Result<(), String> {
    let multiline = !prefix.is_empty() || !indent.is_empty();
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        // An integral value below 1e21 prints as its digits (es6_number_string's first case); writing those directly
        // avoids the shortest-digits formatting and its allocations, which dominate large integer arrays.
        Value::Number(n) if n.fract() == 0.0 && n.abs() < 9007199254740992.0 => write_i64(out, *n as i64),
        Value::Number(n) => out.push_str(&marshal_f64(*n)?),
        Value::Integer(n) => write_i64(out, *n),
        Value::String(s) => write_string(out, s),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if multiline {
                    newline(out, prefix, indent, depth + 1);
                }
                write_value(out, item, prefix, indent, depth + 1)?;
            }
            if multiline && !items.is_empty() {
                newline(out, prefix, indent, depth);
            }
            out.push(']');
        }
        Value::Object(members) => {
            out.push('{');
            for (i, (k, item)) in members.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if multiline {
                    newline(out, prefix, indent, depth + 1);
                }
                write_string(out, k);
                out.push(':');
                if multiline {
                    out.push(' ');
                }
                write_value(out, item, prefix, indent, depth + 1)?;
            }
            if multiline && !members.is_empty() {
                newline(out, prefix, indent, depth);
            }
            out.push('}');
        }
    }
    Ok(())
}

fn write_i64(out: &mut String, n: i64) {
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    let mut m = n.unsigned_abs();
    loop {
        i -= 1;
        buf[i] = b'0' + (m % 10) as u8;
        m /= 10;
        if m == 0 {
            break;
        }
    }
    if n < 0 {
        out.push('-');
    }
    // Only ASCII digits were written.
    out.push_str(std::str::from_utf8(&buf[i..]).unwrap());
}

fn write_string(out: &mut String, s: &str) {
    out.push('"');
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Copy the run of bytes that need no escaping in one go.
        let run = bytes[i..].iter().position(|&b| b >= 0x80 || b < 0x20 || b == b'"' || b == b'\\').unwrap_or(bytes.len() - i);
        if run > 0 {
            out.push_str(&s[i..i + run]);
            i += run;
            continue;
        }
        let b = bytes[i];
        if b < 0x80 {
            match b {
                b'"' => out.push_str("\\\""),
                b'\\' => out.push_str("\\\\"),
                b'\x08' => out.push_str("\\b"),
                b'\x0c' => out.push_str("\\f"),
                b'\n' => out.push_str("\\n"),
                b'\r' => out.push_str("\\r"),
                b'\t' => out.push_str("\\t"),
                0..=0x1f => {
                    let _ = write!(out, "\\u{:04x}", b);
                }
                _ => out.push(b as char),
            }
            i += 1;
            continue;
        }
        let (r, size) = crate::stringutil::decode_rune(&bytes[i..]);
        // AllowInvalidUTF8: invalid bytes are replaced with U+FFFD.
        crate::stringutil::push_rune(out, r);
        i += size;
    }
    out.push('"');
}

pub fn unmarshal(input: &str) -> Result<Value, String> {
    let mut p = JsonParser { s: input.as_bytes(), pos: 0 };
    p.skip_ws();
    let v = p.parse_value()?;
    p.skip_ws();
    if p.pos != p.s.len() {
        return Err(format!("jsontext: invalid character after top-level value at offset {}", p.pos));
    }
    Ok(v)
}

struct JsonParser<'a> {
    s: &'a [u8],
    pos: usize,
}

impl JsonParser<'_> {
    fn skip_ws(&mut self) {
        while self.pos < self.s.len() && matches!(self.s[self.pos], b' ' | b'\t' | b'\n' | b'\r') {
            self.pos += 1;
        }
    }

    fn err<T>(&self, msg: &str) -> Result<T, String> {
        Err(format!("jsontext: {msg} at offset {}", self.pos))
    }

    fn parse_value(&mut self) -> Result<Value, String> {
        match self.s.get(self.pos) {
            None => self.err("unexpected EOF"),
            Some(b'n') => self.literal("null", Value::Null),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'"') => Ok(Value::String(self.parse_string()?)),
            Some(b'[') => {
                self.pos += 1;
                let mut items = Vec::new();
                self.skip_ws();
                if self.s.get(self.pos) == Some(&b']') {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                loop {
                    self.skip_ws();
                    items.push(self.parse_value()?);
                    self.skip_ws();
                    match self.s.get(self.pos) {
                        Some(b',') => self.pos += 1,
                        Some(b']') => {
                            self.pos += 1;
                            return Ok(Value::Array(items));
                        }
                        _ => return self.err("invalid character in array"),
                    }
                }
            }
            Some(b'{') => {
                self.pos += 1;
                let mut members = OrderedMap::default();
                self.skip_ws();
                if self.s.get(self.pos) == Some(&b'}') {
                    self.pos += 1;
                    return Ok(Value::Object(members));
                }
                loop {
                    self.skip_ws();
                    if self.s.get(self.pos) != Some(&b'"') {
                        return self.err("invalid character in object key");
                    }
                    let key = self.parse_string()?;
                    self.skip_ws();
                    if self.s.get(self.pos) != Some(&b':') {
                        return self.err("missing colon after object key");
                    }
                    self.pos += 1;
                    self.skip_ws();
                    let value = self.parse_value()?;
                    if members.contains_key(&key) {
                        return self.err(&format!("duplicate object member name {}", marshal_string(&key)));
                    }
                    members.insert(key, value);
                    self.skip_ws();
                    match self.s.get(self.pos) {
                        Some(b',') => self.pos += 1,
                        Some(b'}') => {
                            self.pos += 1;
                            return Ok(Value::Object(members));
                        }
                        _ => return self.err("invalid character in object"),
                    }
                }
            }
            Some(b'-' | b'0'..=b'9') => self.parse_number(),
            Some(_) => self.err("invalid character at start of value"),
        }
    }

    fn literal(&mut self, lit: &str, v: Value) -> Result<Value, String> {
        if self.s[self.pos..].starts_with(lit.as_bytes()) {
            self.pos += lit.len();
            return Ok(v);
        }
        self.err("invalid literal")
    }

    fn parse_number(&mut self) -> Result<Value, String> {
        let start = self.pos;
        if self.s[self.pos] == b'-' {
            self.pos += 1;
        }
        let digits = |p: &mut Self| {
            let s = p.pos;
            while p.pos < p.s.len() && p.s[p.pos].is_ascii_digit() {
                p.pos += 1;
            }
            p.pos - s
        };
        match self.s.get(self.pos) {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                digits(self);
            }
            _ => return self.err("invalid number"),
        }
        if self.s.get(self.pos) == Some(&b'.') {
            self.pos += 1;
            if digits(self) == 0 {
                return self.err("invalid number");
            }
        }
        if matches!(self.s.get(self.pos), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.s.get(self.pos), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if digits(self) == 0 {
                return self.err("invalid number");
            }
        }
        let text = std::str::from_utf8(&self.s[start..self.pos]).unwrap();
        match text.parse::<f64>() {
            Ok(f) if f.is_finite() => Ok(Value::Number(f)),
            _ => self.err("number out of range"),
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        if self.pos + 4 > self.s.len() {
            return self.err("invalid escape sequence");
        }
        let text = std::str::from_utf8(&self.s[self.pos..self.pos + 4]).map_err(|e| e.to_string())?;
        let v = u32::from_str_radix(text, 16).or_else(|_| self.err("invalid escape sequence"))?;
        self.pos += 4;
        Ok(v)
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.pos += 1; // opening quote
        let mut out = String::new();
        loop {
            let Some(&b) = self.s.get(self.pos) else {
                return self.err("unexpected EOF in string");
            };
            match b {
                b'"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.pos += 1;
                    let Some(&e) = self.s.get(self.pos) else {
                        return self.err("unexpected EOF in string");
                    };
                    self.pos += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\x08'),
                        b'f' => out.push('\x0c'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut r = self.hex4()?;
                            if (0xD800..0xDC00).contains(&r) && self.s[self.pos..].starts_with(b"\\u") {
                                let save = self.pos;
                                self.pos += 2;
                                let r2 = self.hex4()?;
                                if (0xDC00..0xE000).contains(&r2) {
                                    r = 0x10000 + ((r - 0xD800) << 10) + (r2 - 0xDC00);
                                } else {
                                    self.pos = save;
                                }
                            }
                            out.push(char::from_u32(r).unwrap_or('\u{FFFD}'));
                        }
                        _ => return self.err("invalid escape sequence"),
                    }
                }
                0..=0x1f => return self.err("invalid control character in string"),
                _ => {
                    let (r, size) = crate::stringutil::decode_rune(&self.s[self.pos..]);
                    crate::stringutil::push_rune(&mut out, r);
                    self.pos += size;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let v = unmarshal(r#"{"a": 1, "b": "two", "c": { "d": [4.5, true, null] }, "e": "é😀<>"}"#).unwrap();
        assert_eq!(marshal(&v).unwrap(), r#"{"a":1,"b":"two","c":{"d":[4.5,true,null]},"e":"é😀<>"}"#);
        assert_eq!(
            marshal_indent(&unmarshal(r#"{"a":[1,2],"b":{}}"#).unwrap(), "", "  ").unwrap(),
            "{\n  \"a\": [\n    1,\n    2\n  ],\n  \"b\": {}\n}"
        );
        assert_eq!(marshal_string("a\"\n\u{1}"), "\"a\\\"\\n\\u0001\"");
        assert_eq!(marshal_f64(1e21).unwrap(), "1e+21");
        assert_eq!(marshal_f64(123.456).unwrap(), "123.456");
        assert_eq!(marshal_f64(-0.000001).unwrap(), "-0.000001");
        assert_eq!(marshal_f64(1.5e-7).unwrap(), "1.5e-7");
        assert_eq!(marshal_f64(1e20).unwrap(), "100000000000000000000");
        // The integer fast path agrees with the general ES6 formatting.
        for n in [0.0, -0.0, 1.0, -1.0, 9.0, 10.0, 12345.0, -987654321.0, 9007199254740991.0, -9007199254740991.0, 9007199254740992.0, 1e20] {
            assert_eq!(marshal(&Value::Number(n)).unwrap(), marshal_f64(n).unwrap(), "{n}");
        }
        assert_eq!(marshal(&Value::Integer(i64::MIN)).unwrap(), i64::MIN.to_string());
        assert_eq!(marshal_string("plain/ascii run é \"q\" \\ tab\t end"), "\"plain/ascii run é \\\"q\\\" \\\\ tab\\t end\"");
        assert!(marshal_f64(f64::NAN).is_err());
        assert!(unmarshal("{\"a\":1,\"a\":2}").is_err());
        assert!(unmarshal("[1,]").is_err());
    }
}
