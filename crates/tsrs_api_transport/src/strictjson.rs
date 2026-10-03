// The input checks that the pinned server's JSON decoder (tsc/internal/json -> encoding/json/v2,
// jsontext) applies to every document it unmarshals, which serde_json does not: duplicate object
// member names are rejected at any depth (including inside ignored/unknown fields), and strings must
// be valid UTF-8 with correctly paired `\uD800-\uDFFF` escapes. Lone surrogates are rejected, never
// replaced. Error texts reproduce jsontext's (verified against a Go binary built at the pinned commit,
// tests/fixtures/go_json_oracle.json).
//
// The scan is iterative (no recursion), allocates at most one key set per open object, and rejects
// nesting deeper than jsontext's limit (10000).

use rustc_hash::FxHashSet;

pub const MAX_DEPTH: usize = 10000;

enum Frame {
    Object { keys: FxHashSet<String>, current: Option<String> },
    Array { index: usize },
}

struct Scanner<'a> {
    b: &'a [u8],
    i: usize,
    stack: Vec<Frame>,
}

fn escape_token(t: &str) -> String {
    t.replace('~', "~0").replace('/', "~1")
}

impl Scanner<'_> {
    /// JSON pointer of the container (`for_key`) or of the value currently being read.
    fn pointer(&self, for_key: bool) -> String {
        let mut out = String::new();
        let n = self.stack.len();
        for (depth, frame) in self.stack.iter().enumerate() {
            let last = depth + 1 == n;
            match frame {
                Frame::Object { current: Some(k), .. } if !(last && for_key) => {
                    out.push('/');
                    out.push_str(&escape_token(k));
                }
                Frame::Array { index } if !(last && for_key) => {
                    out.push('/');
                    out.push_str(&index.to_string());
                }
                _ => {}
            }
        }
        out
    }

    fn within(&self, for_key: bool) -> String {
        let p = self.pointer(for_key);
        if p.is_empty() {
            String::new()
        } else {
            format!(" within {}", serde_json::to_string(&p).unwrap())
        }
    }

    fn syntax(&self, what: &str) -> String {
        format!("jsontext: {what}{} after offset {}", self.within(false), self.i)
    }

    fn skip_ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn hex4(&self, at: usize) -> Option<u32> {
        let s = self.b.get(at..at + 4)?;
        let s = std::str::from_utf8(s).ok()?;
        u32::from_str_radix(s, 16).ok().filter(|_| s.bytes().all(|c| c.is_ascii_hexdigit()))
    }

    /// Reads a string starting at the opening quote; returns its decoded value.
    fn string(&mut self, is_key: bool) -> Result<String, String> {
        debug_assert_eq!(self.b[self.i], b'"');
        self.i += 1;
        let mut out = String::new();
        loop {
            let Some(&c) = self.b.get(self.i) else {
                return Err(format!("jsontext: unexpected EOF{} after offset {}", self.within(is_key), self.i));
            };
            match c {
                b'"' => {
                    self.i += 1;
                    return Ok(out);
                }
                b'\\' => {
                    let start = self.i;
                    let surrogate_error = |s: &Self| {
                        let end = (start + 12).min(s.b.len());
                        format!(
                            "jsontext: invalid surrogate pair `{}` in string{} after offset {start}",
                            String::from_utf8_lossy(&s.b[start..end]),
                            s.within(is_key)
                        )
                    };
                    let Some(&e) = self.b.get(self.i + 1) else {
                        return Err(format!("jsontext: unexpected EOF{} after offset {}", self.within(is_key), self.b.len()));
                    };
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let Some(u) = self.hex4(self.i + 2) else {
                                return Err(format!("jsontext: invalid escape sequence{} after offset {start}", self.within(is_key)));
                            };
                            if (0xDC00..0xE000).contains(&u) {
                                return Err(surrogate_error(self));
                            }
                            if (0xD800..0xDC00).contains(&u) {
                                let lo = (self.b.get(self.i + 6) == Some(&b'\\') && self.b.get(self.i + 7) == Some(&b'u'))
                                    .then(|| self.hex4(self.i + 8))
                                    .flatten()
                                    .filter(|lo| (0xDC00..0xE000).contains(lo));
                                let Some(lo) = lo else { return Err(surrogate_error(self)) };
                                out.push(char::from_u32(0x10000 + ((u - 0xD800) << 10) + (lo - 0xDC00)).unwrap());
                                self.i += 12;
                                continue;
                            }
                            out.push(char::from_u32(u).unwrap());
                            self.i += 6;
                            continue;
                        }
                        _ => return Err(format!("jsontext: invalid escape sequence{} after offset {start}", self.within(is_key))),
                    }
                    self.i += 2;
                }
                c if c < 0x20 => {
                    return Err(format!("jsontext: invalid character {:?} in string{} after offset {}", c as char, self.within(is_key), self.i));
                }
                c if c < 0x80 => {
                    out.push(c as char);
                    self.i += 1;
                }
                _ => {
                    let len = match c {
                        0xC2..=0xDF => 2,
                        0xE0..=0xEF => 3,
                        0xF0..=0xF4 => 4,
                        _ => 0,
                    };
                    match self.b.get(self.i..self.i + len).filter(|_| len > 0).and_then(|s| std::str::from_utf8(s).ok()) {
                        Some(s) => {
                            out.push_str(s);
                            self.i += len;
                        }
                        None => return Err(format!("jsontext: invalid UTF-8{} after offset {}", self.within(is_key), self.i)),
                    }
                }
            }
        }
    }

    fn literal_or_number(&mut self) -> Result<(), String> {
        let start = self.i;
        for lit in [&b"true"[..], b"false", b"null"] {
            if self.b[self.i..].starts_with(lit) {
                self.i += lit.len();
                return Ok(());
            }
        }
        // Number: -?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?
        let b = self.b;
        let mut j = self.i;
        if b.get(j) == Some(&b'-') {
            j += 1;
        }
        match b.get(j) {
            Some(b'0') => j += 1,
            Some(b'1'..=b'9') => {
                while b.get(j).is_some_and(u8::is_ascii_digit) {
                    j += 1;
                }
            }
            _ => return Err(self.syntax("invalid character")),
        }
        if b.get(j) == Some(&b'.') {
            j += 1;
            if !b.get(j).is_some_and(u8::is_ascii_digit) {
                self.i = j;
                return Err(self.syntax("invalid number"));
            }
            while b.get(j).is_some_and(u8::is_ascii_digit) {
                j += 1;
            }
        }
        if matches!(b.get(j), Some(b'e' | b'E')) {
            j += 1;
            if matches!(b.get(j), Some(b'+' | b'-')) {
                j += 1;
            }
            if !b.get(j).is_some_and(u8::is_ascii_digit) {
                self.i = j;
                return Err(self.syntax("invalid number"));
            }
            while b.get(j).is_some_and(u8::is_ascii_digit) {
                j += 1;
            }
        }
        debug_assert!(j > start);
        self.i = j;
        Ok(())
    }

    /// Called after a complete value: consume `,` / closers until the next value or key is due.
    /// Returns true when the whole document is complete.
    fn after_value(&mut self) -> Result<bool, String> {
        loop {
            self.skip_ws();
            let Some(frame) = self.stack.last_mut() else { return Ok(true) };
            let c = self.b.get(self.i).copied();
            match (frame, c) {
                (Frame::Object { .. }, Some(b',')) => {
                    self.i += 1;
                    self.skip_ws();
                    self.key()?;
                    return Ok(false);
                }
                (Frame::Array { index }, Some(b',')) => {
                    *index += 1;
                    self.i += 1;
                    return Ok(false);
                }
                (Frame::Object { .. }, Some(b'}')) | (Frame::Array { .. }, Some(b']')) => {
                    self.i += 1;
                    self.stack.pop();
                }
                (_, None) => return Err(format!("jsontext: unexpected EOF{} after offset {}", self.within(true), self.i)),
                _ => return Err(self.syntax("invalid character")),
            }
        }
    }

    /// Reads `"name":` inside the current object, rejecting duplicate names.
    fn key(&mut self) -> Result<(), String> {
        if self.b.get(self.i) != Some(&b'"') {
            return Err(self.syntax("invalid character, expected object name"));
        }
        let name = self.string(true)?;
        let within = self.within(true);
        let Some(Frame::Object { keys, current }) = self.stack.last_mut() else { unreachable!() };
        if !keys.insert(name.clone()) {
            return Err(format!("jsontext: duplicate object member name {}{within}", serde_json::to_string(&name).unwrap()));
        }
        *current = Some(name);
        self.skip_ws();
        if self.b.get(self.i) != Some(&b':') {
            return Err(self.syntax("invalid character, expected ':'"));
        }
        self.i += 1;
        Ok(())
    }

    fn run(&mut self) -> Result<(), String> {
        loop {
            self.skip_ws();
            let Some(&c) = self.b.get(self.i) else {
                return Err(format!("jsontext: unexpected EOF{} after offset {}", self.within(false), self.i));
            };
            match c {
                b'{' | b'[' => {
                    if self.stack.len() >= MAX_DEPTH {
                        return Err(self.syntax("exceeded max depth"));
                    }
                    self.i += 1;
                    self.skip_ws();
                    if c == b'{' {
                        self.stack.push(Frame::Object { keys: FxHashSet::default(), current: None });
                        if self.b.get(self.i) == Some(&b'}') {
                            self.i += 1;
                            self.stack.pop();
                        } else {
                            self.key()?;
                            continue;
                        }
                    } else {
                        self.stack.push(Frame::Array { index: 0 });
                        if self.b.get(self.i) == Some(&b']') {
                            self.i += 1;
                            self.stack.pop();
                        } else {
                            continue;
                        }
                    }
                }
                b'"' => {
                    self.string(false)?;
                }
                _ => self.literal_or_number()?,
            }
            if self.after_value()? {
                self.skip_ws();
                if self.i != self.b.len() {
                    return Err(self.syntax("invalid character after top-level value"));
                }
                return Ok(());
            }
        }
    }
}

/// Validates one JSON document with jsontext's strictness (see module docs).
pub fn validate(input: &[u8]) -> Result<(), String> {
    Scanner { b: input, i: 0, stack: Vec::new() }.run()
}
