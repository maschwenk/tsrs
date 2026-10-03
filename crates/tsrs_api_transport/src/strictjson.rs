// The input checks that the pinned server's JSON decoder (tsc/internal/json -> encoding/json/v2,
// jsontext) applies to every document it unmarshals, which serde_json does not: duplicate object
// member names are rejected at any depth (including inside ignored/unknown fields), strings must be
// valid UTF-8 with correctly paired `\uD800-\uDFFF` escapes (lone surrogates are rejected, never
// replaced), and the grammar is strict. Error texts reproduce jsontext's, including the JSON-pointer
// and offset rules (verified against a Go binary built at the pinned commit:
// tests/fixtures/go_json_oracle.json and go_syntax_oracle.json).
//
// The scan is iterative (no recursion), allocates at most one key set per open object, and rejects
// nesting deeper than jsontext's limit (10000).

use rustc_hash::FxHashSet;

pub const MAX_DEPTH: usize = 10000;

enum Frame {
    Object { keys: FxHashSet<String>, current: Option<String> },
    Array { index: usize },
}

#[derive(Clone, Copy, PartialEq)]
enum Expect {
    /// A value (top level, after `:`, or as an array element).
    Value,
    /// A member name or `}` right after `{`.
    FirstName,
    /// A member name after `,`.
    Name,
    /// `:` after a member name.
    Colon,
    /// `,` or the closer after a value inside a container; at top level, only whitespace.
    AfterValue,
}

struct Scanner<'a> {
    b: &'a [u8],
    i: usize,
    stack: Vec<Frame>,
    /// Offset of the last `,` while a name or value is expected after it.
    comma: Option<usize>,
}

fn escape_token(t: &str) -> String {
    t.replace('~', "~0").replace('/', "~1")
}

/// Go's %q for a rune, as jsontext prints offending characters.
fn quote_char(c: char) -> String {
    match c {
        '\'' => "'\\''".to_string(),
        '\\' => "'\\\\'".to_string(),
        '\n' => "'\\n'".to_string(),
        '\r' => "'\\r'".to_string(),
        '\t' => "'\\t'".to_string(),
        '\u{7}' => "'\\a'".to_string(),
        '\u{8}' => "'\\b'".to_string(),
        '\u{c}' => "'\\f'".to_string(),
        '\u{b}' => "'\\v'".to_string(),
        c if (c as u32) < 0x20 || c as u32 == 0x7f => format!("'\\x{:02x}'", c as u32),
        c => format!("'{c}'"),
    }
}

fn is_value_start(c: u8) -> bool {
    matches!(c, b'"' | b'{' | b'[' | b'-' | b'0'..=b'9' | b't' | b'f' | b'n')
}

impl Scanner<'_> {
    /// Pointer tokens of the frames; `skip_last` drops that many trailing frame tokens.
    fn pointer_upto(&self, frames: usize, include_last_token: bool) -> String {
        let mut out = String::new();
        for (depth, frame) in self.stack.iter().take(frames).enumerate() {
            if depth + 1 == frames && !include_last_token {
                break;
            }
            match frame {
                Frame::Object { current: Some(k), .. } => {
                    out.push('/');
                    out.push_str(&escape_token(k));
                }
                Frame::Object { current: None, .. } => {}
                Frame::Array { index } => {
                    out.push('/');
                    out.push_str(&index.to_string());
                }
            }
        }
        out
    }

    /// Pointer of the value being read (includes the current member name / element index).
    fn value_ptr(&self) -> String {
        self.pointer_upto(self.stack.len(), true)
    }

    /// Pointer of the innermost container itself.
    fn container_ptr(&self) -> String {
        self.pointer_upto(self.stack.len(), false)
    }

    /// Pointer of the innermost container's parent container.
    fn parent_ptr(&self) -> String {
        self.pointer_upto(self.stack.len().saturating_sub(1), false)
    }

    fn err(&self, what: &str, pointer: &str, offset: usize) -> String {
        let mut s = format!("jsontext: {what}");
        if !pointer.is_empty() {
            s.push_str(&format!(" within {}", serde_json::to_string(pointer).unwrap()));
        }
        if offset > 0 {
            s.push_str(&format!(" after offset {offset}"));
        }
        s
    }

    fn char_at(&self, at: usize) -> char {
        std::str::from_utf8(&self.b[at..(at + 4).min(self.b.len())])
            .ok()
            .or_else(|| self.b[at..].utf8_chunks().next().map(|c| c.valid()))
            .and_then(|s| s.chars().next())
            .unwrap_or(self.b[at] as char)
    }

    fn invalid_char(&self, at: usize, context: &str, pointer: &str) -> String {
        self.err(&format!("invalid character {} {context}", quote_char(self.char_at(at))), pointer, at)
    }

    fn eof(&self, pointer: &str, offset: usize) -> String {
        self.err("unexpected EOF", pointer, offset)
    }

    fn skip_ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn hex4(&self, at: usize) -> Option<u32> {
        let s = self.b.get(at..at + 4)?;
        if !s.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        u32::from_str_radix(std::str::from_utf8(s).ok()?, 16).ok()
    }

    /// Reads a string starting at the opening quote; returns its decoded value. `pointer` locates it.
    fn string(&mut self, pointer: &str) -> Result<String, String> {
        self.i += 1;
        let mut out = String::new();
        loop {
            let Some(&c) = self.b.get(self.i) else {
                return Err(self.eof(pointer, self.b.len()));
            };
            match c {
                b'"' => {
                    self.i += 1;
                    return Ok(out);
                }
                b'\\' => {
                    let start = self.i;
                    let window = |s: &Self, n: usize| String::from_utf8_lossy(&s.b[start..(start + n).min(s.b.len())]).into_owned();
                    let Some(&e) = self.b.get(self.i + 1) else {
                        return Err(self.eof(pointer, start));
                    };
                    let simple = match e {
                        b'"' => Some('"'),
                        b'\\' => Some('\\'),
                        b'/' => Some('/'),
                        b'b' => Some('\u{8}'),
                        b'f' => Some('\u{c}'),
                        b'n' => Some('\n'),
                        b'r' => Some('\r'),
                        b't' => Some('\t'),
                        _ => None,
                    };
                    if let Some(ch) = simple {
                        out.push(ch);
                        self.i += 2;
                        continue;
                    }
                    if e != b'u' {
                        return Err(self.err(&format!("invalid escape sequence `{}` in string", window(self, 2)), pointer, start));
                    }
                    if self.b.len() < start + 6 && self.b[start + 2..].iter().all(u8::is_ascii_hexdigit) {
                        return Err(self.eof(pointer, start));
                    }
                    let Some(u) = self.hex4(start + 2) else {
                        return Err(self.err(&format!("invalid escape sequence `{}` in string", window(self, 6)), pointer, start));
                    };
                    let surrogate = |s: &Self| {
                        s.err(&format!("invalid surrogate pair `{}` in string", window(s, 12)), pointer, start)
                    };
                    if (0xDC00..0xE000).contains(&u) {
                        return Err(surrogate(self));
                    }
                    if (0xD800..0xDC00).contains(&u) {
                        let lo = (self.b.get(start + 6) == Some(&b'\\') && self.b.get(start + 7) == Some(&b'u'))
                            .then(|| self.hex4(start + 8))
                            .flatten()
                            .filter(|lo| (0xDC00..0xE000).contains(lo));
                        let Some(lo) = lo else { return Err(surrogate(self)) };
                        out.push(char::from_u32(0x10000 + ((u - 0xD800) << 10) + (lo - 0xDC00)).unwrap());
                        self.i += 12;
                        continue;
                    }
                    out.push(char::from_u32(u).unwrap());
                    self.i += 6;
                }
                c if c < 0x20 => {
                    return Err(self.invalid_char(self.i, "in string (expecting non-control character)", pointer));
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
                        None => return Err(self.err("invalid UTF-8", pointer, self.i)),
                    }
                }
            }
        }
    }

    fn literal(&mut self, lit: &'static [u8], name: &str) -> Result<(), String> {
        let ptr = self.value_ptr();
        for (k, &want) in lit.iter().enumerate() {
            match self.b.get(self.i + k) {
                None => return Err(self.eof(&ptr, self.b.len())),
                Some(&got) if got != want => {
                    return Err(self.invalid_char(self.i + k, &format!("in literal {name} (expecting {})", quote_char(want as char)), &ptr));
                }
                _ => {}
            }
        }
        self.i += lit.len();
        Ok(())
    }

    /// -?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?
    fn number(&mut self) -> Result<(), String> {
        let ptr = self.value_ptr();
        let start = self.i;
        let b = self.b;
        let mut j = self.i;
        let digit_expected = |s: &Self, j: usize| -> String {
            match b.get(j) {
                None => s.eof(&ptr, start),
                Some(_) => s.invalid_char(j, "in number (expecting digit)", &ptr),
            }
        };
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
            _ => return Err(digit_expected(self, j)),
        }
        if b.get(j) == Some(&b'.') {
            j += 1;
            if !b.get(j).is_some_and(u8::is_ascii_digit) {
                return Err(digit_expected(self, j));
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
                return Err(digit_expected(self, j));
            }
            while b.get(j).is_some_and(u8::is_ascii_digit) {
                j += 1;
            }
        }
        self.i = j;
        Ok(())
    }

    /// Reads one value at `self.i` (already past whitespace). Containers are pushed and left open.
    fn value(&mut self) -> Result<Expect, String> {
        let c = self.b[self.i];
        match c {
            b'{' | b'[' => {
                if self.stack.len() >= MAX_DEPTH {
                    return Err(self.err("exceeded max depth", &self.value_ptr(), self.i));
                }
                self.i += 1;
                if c == b'{' {
                    self.stack.push(Frame::Object { keys: FxHashSet::default(), current: None });
                    Ok(Expect::FirstName)
                } else {
                    self.stack.push(Frame::Array { index: 0 });
                    self.skip_ws();
                    if self.b.get(self.i) == Some(&b']') {
                        self.i += 1;
                        self.stack.pop();
                        return Ok(Expect::AfterValue);
                    }
                    Ok(Expect::Value)
                }
            }
            b'"' => {
                let ptr = self.value_ptr();
                self.string(&ptr)?;
                Ok(Expect::AfterValue)
            }
            b't' => self.literal(b"true", "true").map(|_| Expect::AfterValue),
            b'f' => self.literal(b"false", "false").map(|_| Expect::AfterValue),
            b'n' => self.literal(b"null", "null").map(|_| Expect::AfterValue),
            b'-' | b'0'..=b'9' => self.number().map(|_| Expect::AfterValue),
            _ => Err(self.invalid_char(self.i, "at start of value", &self.value_ptr())),
        }
    }

    fn run(&mut self) -> Result<(), String> {
        if self.b.is_empty() {
            return Err("jsontext: unexpected EOF".to_string());
        }
        let mut expect = Expect::Value;
        loop {
            self.skip_ws();
            let at_end = self.i >= self.b.len();
            match expect {
                Expect::Value => {
                    if at_end {
                        // jsontext locates a missing member value at the member, a missing element at the array.
                        let ptr = match self.stack.last() {
                            Some(Frame::Object { .. }) => self.value_ptr(),
                            _ => self.container_ptr(),
                        };
                        return Err(self.eof(&ptr, self.i));
                    }
                    let c = self.b[self.i];
                    if let (Some(comma), b']' | b'}') = (self.comma, c) {
                        // Trailing comma: jsontext blames the comma, located in the container.
                        return Err(self.invalid_char(comma, "at start of value", &self.container_ptr()));
                    }
                    self.comma = None;
                    expect = self.value()?;
                }
                Expect::FirstName | Expect::Name => {
                    if at_end {
                        return Err(self.eof(&self.container_ptr(), self.i));
                    }
                    let c = self.b[self.i];
                    if let (Some(comma), b'}' | b']') = (self.comma, c) {
                        return Err(self.invalid_char(comma, "at start of value", &self.container_ptr()));
                    }
                    if c == b'}' {
                        self.i += 1;
                        self.stack.pop();
                        expect = Expect::AfterValue;
                        continue;
                    }
                    self.comma = None;
                    if c != b'"' {
                        let ptr = self.container_ptr();
                        if is_value_start(c) {
                            return Err(self.err("object member name must be a string", &ptr, self.i));
                        }
                        return Err(self.invalid_char(self.i, "at start of value", &ptr));
                    }
                    let ptr = self.container_ptr();
                    let name = self.string(&ptr)?;
                    let Some(Frame::Object { keys, current }) = self.stack.last_mut() else { unreachable!() };
                    if !keys.insert(name.clone()) {
                        return Err(self.err(&format!("duplicate object member name {}", serde_json::to_string(&name).unwrap()), &ptr, 0));
                    }
                    *current = Some(name);
                    expect = Expect::Colon;
                }
                Expect::Colon => {
                    let ptr = self.value_ptr();
                    if at_end {
                        return Err(self.eof(&ptr, self.i));
                    }
                    if self.b[self.i] != b':' {
                        return Err(self.invalid_char(self.i, "after object name (expecting ':')", &ptr));
                    }
                    self.i += 1;
                    expect = Expect::Value;
                }
                Expect::AfterValue => {
                    let Some(frame) = self.stack.last_mut() else {
                        if at_end {
                            return Ok(());
                        }
                        return Err(self.invalid_char(self.i, "after top-level value", ""));
                    };
                    if at_end {
                        return Err(self.eof(&self.container_ptr(), self.i));
                    }
                    let c = self.b[self.i];
                    match (frame, c) {
                        (Frame::Object { .. }, b',') => {
                            self.comma = Some(self.i);
                            self.i += 1;
                            expect = Expect::Name;
                        }
                        (Frame::Array { index }, b',') => {
                            *index += 1;
                            self.comma = Some(self.i);
                            self.i += 1;
                            expect = Expect::Value;
                        }
                        (Frame::Object { .. }, b'}') | (Frame::Array { .. }, b']') => {
                            self.i += 1;
                            self.stack.pop();
                        }
                        (Frame::Object { .. }, b']') => {
                            return Err(self.invalid_char(self.i, "after object value (expecting ',' or '}')", &self.parent_ptr()));
                        }
                        (Frame::Object { .. }, _) => {
                            return Err(self.invalid_char(self.i, "after object value (expecting ',' or '}')", &self.container_ptr()));
                        }
                        (Frame::Array { .. }, _) => {
                            return Err(self.invalid_char(self.i, "after array element (expecting ',' or ']')", &self.container_ptr()));
                        }
                    }
                }
            }
        }
    }
}

/// Validates one JSON document with jsontext's strictness (see module docs).
pub fn validate(input: &[u8]) -> Result<(), String> {
    Scanner { b: input, i: 0, stack: Vec::new(), comma: None }.run()
}
