// Go decodes a request's whole params struct (proto.go `unmarshalPayload`) before a handler runs, so a field of
// the wrong JSON kind is an invalid request even when another field would fail a lookup first. This checks the
// top-level fields of each method's pinned params struct (`paramfields.rs`, generated from proto.go) in Go's
// struct field order, plus array element kinds and integer ranges, for every method (core and checker lanes),
// before dispatch. Unknown keys are ignored, as Go does. Integer fields are checked against the raw number
// lexeme: encoding/json/v2 accepts only plain integer syntax (`1e3`, `1.0` are `invalid syntax`).
// Error class and `failed to unmarshal *api.<T>` prefix match Go; the jsontext wording after it is approximate
// (Go reports the first invalid field in document order, this in struct order) except for DocumentIdentifier,
// whose custom decoder text is reproduced.

use std::collections::HashMap;

use tsrs_core::json::Value;

use crate::handler::{ApiError, ApiResult};
use crate::paramfields::{params_fields, Elem, FieldKind, FieldSpec};

fn json_kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn go_type_name(t: &str) -> String {
    let base = t.trim_start_matches('*');
    if base.contains('.') || base.starts_with("[]") || !base.starts_with(|c: char| c.is_ascii_uppercase()) {
        base.to_string()
    } else {
        format!("api.{base}")
    }
}

/// proto.go `DocumentIdentifier.UnmarshalJSONFrom` (error text as probed against the pinned server by the checker
/// lane, `checker/params.rs`).
fn check_document(v: &Value, pointer: &str) -> Result<(), String> {
    let got = match v {
        Value::String(_) => return Ok(()),
        Value::Object(o) => {
            return match o.get("uri") {
                Some(Value::Object(_) | Value::Array(_)) => {
                    Err(format!("cannot unmarshal into Go api.DocumentIdentifier within \"{pointer}\": DocumentIdentifier: unsupported uri value"))
                }
                _ => Ok(()),
            }
        }
        Value::Null => "null",
        Value::Number(_) => "number",
        Value::Bool(true) => "true",
        Value::Bool(false) => "false",
        Value::Array(_) => "[",
    };
    Err(format!("cannot unmarshal into Go api.DocumentIdentifier within \"{pointer}\": DocumentIdentifier: expected string or object, got {got}"))
}

#[derive(Clone, Copy)]
enum IntRange {
    I32,
    I64,
    U32,
    U64,
}

/// A JSON number decoded into a Go integer: plain integer syntax (no fraction or exponent) within range.
fn int_ok(v: &Value, lexeme: Option<&str>, range: IntRange) -> bool {
    let Value::Number(n) = v else { return false };
    if lexeme.is_some_and(|l| l.contains(['.', 'e', 'E'])) || n.fract() != 0.0 {
        return false;
    }
    let n = *n;
    match range {
        IntRange::I32 => (i32::MIN as f64..=i32::MAX as f64).contains(&n),
        // i64::MAX is not representable in f64; 2^63 is the first value out of range.
        IntRange::I64 => n >= -9223372036854775808.0 && n < 9223372036854775808.0,
        IntRange::U32 => (0.0..=u32::MAX as f64).contains(&n),
        IntRange::U64 => n >= 0.0 && n < 18446744073709551616.0,
    }
}

fn check_scalar(kind: Elem, v: &Value, lexeme: Option<&str>) -> bool {
    match kind {
        Elem::Any => true,
        Elem::Str => matches!(v, Value::String(_)),
        Elem::Bool => matches!(v, Value::Bool(_)),
        Elem::Object => matches!(v, Value::Object(_)),
        Elem::I32 => int_ok(v, lexeme, IntRange::I32),
        Elem::I64 => int_ok(v, lexeme, IntRange::I64),
        Elem::U32 => int_ok(v, lexeme, IntRange::U32),
        Elem::U64 => int_ok(v, lexeme, IntRange::U64),
    }
}

fn field_elem(kind: FieldKind) -> Option<Elem> {
    Some(match kind {
        FieldKind::Str => Elem::Str,
        FieldKind::I32 => Elem::I32,
        FieldKind::I64 => Elem::I64,
        FieldKind::U32 => Elem::U32,
        FieldKind::U64 => Elem::U64,
        FieldKind::Bool => Elem::Bool,
        FieldKind::Object => Elem::Object,
        FieldKind::Any => Elem::Any,
        FieldKind::Doc | FieldKind::DocList | FieldKind::Array => return None,
    })
}

fn check_field(spec: &FieldSpec, v: &Value, lexemes: &HashMap<String, String>) -> Result<(), String> {
    let pointer = format!("/{}", spec.name);
    let mismatch = |v: &Value, pointer: &str, go_type: &str| format!("cannot unmarshal JSON {} into Go {} within \"{pointer}\"", json_kind(v), go_type_name(go_type));
    if matches!(v, Value::Null) && spec.kind != FieldKind::Doc && spec.kind != FieldKind::DocList {
        // `null` into a non-pointer field leaves the zero value; into a pointer it is nil.
        return Ok(());
    }
    if let Some(elem) = field_elem(spec.kind) {
        return check_scalar(elem, v, lexemes.get(&pointer).map(String::as_str)).then_some(()).ok_or_else(|| mismatch(v, &pointer, spec.go_type));
    }
    match spec.kind {
        FieldKind::Doc => {
            if matches!(v, Value::Null) && spec.nullable {
                return Ok(());
            }
            check_document(v, &pointer)
        }
        FieldKind::DocList => match v {
            Value::Null => Ok(()),
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    check_document(item, &format!("{pointer}/{i}"))?;
                }
                Ok(())
            }
            _ => Err(mismatch(v, &pointer, spec.go_type)),
        },
        FieldKind::Array => match v {
            Value::Array(items) => {
                let elem = spec.elem.unwrap_or(Elem::Any);
                let elem_type = spec.go_type.trim_start_matches("[]");
                for (i, item) in items.iter().enumerate() {
                    let p = format!("{pointer}/{i}");
                    // A null element is the element type's zero value (or nil), as for fields.
                    if !matches!(item, Value::Null) && !check_scalar(elem, item, lexemes.get(&p).map(String::as_str)) {
                        return Err(mismatch(item, &p, elem_type));
                    }
                }
                Ok(())
            }
            _ => Err(mismatch(v, &pointer, spec.go_type)),
        },
        _ => Ok(()),
    }
}

/// Number lexemes of the top-level members and of their array elements, keyed by JSON pointer (`/key`,
/// `/key/<i>`). `raw` was validated as strict JSON.
fn number_lexemes(raw: &[u8]) -> HashMap<String, String> {
    struct S<'a> {
        b: &'a [u8],
        i: usize,
        out: HashMap<String, String>,
    }
    impl S<'_> {
        fn ws(&mut self) {
            while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
                self.i += 1;
            }
        }
        fn string(&mut self) -> String {
            let start = self.i + 1;
            self.i += 1;
            while self.i < self.b.len() && self.b[self.i] != b'"' {
                if self.b[self.i] == b'\\' {
                    self.i += 1;
                }
                self.i += 1;
            }
            let s = String::from_utf8_lossy(&self.b[start..self.i.min(self.b.len())]).into_owned();
            self.i += 1;
            s
        }
        // Skips one value; records number lexemes at `pointer`, and for `depth < 2` recurses into arrays.
        fn value(&mut self, pointer: Option<String>, depth: u32) {
            self.ws();
            if self.i >= self.b.len() {
                return;
            }
            match self.b[self.i] {
                b'"' => {
                    self.string();
                }
                b'{' => {
                    self.i += 1;
                    loop {
                        self.ws();
                        if self.i >= self.b.len() || self.b[self.i] == b'}' {
                            self.i += 1;
                            return;
                        }
                        if self.b[self.i] == b',' {
                            self.i += 1;
                            continue;
                        }
                        let key = self.string();
                        self.ws();
                        self.i += 1; // ':'
                        let p = if depth == 0 { Some(format!("/{}", key.replace('~', "~0").replace('/', "~1"))) } else { None };
                        self.value(p, depth + 1);
                    }
                }
                b'[' => {
                    self.i += 1;
                    let mut index = 0;
                    loop {
                        self.ws();
                        if self.i >= self.b.len() || self.b[self.i] == b']' {
                            self.i += 1;
                            return;
                        }
                        if self.b[self.i] == b',' {
                            self.i += 1;
                            continue;
                        }
                        let p = if depth == 1 { pointer.as_ref().map(|p| format!("{p}/{index}")) } else { None };
                        self.value(p, depth + 1);
                        index += 1;
                    }
                }
                _ => {
                    let start = self.i;
                    while self.i < self.b.len() && !matches!(self.b[self.i], b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r') {
                        self.i += 1;
                    }
                    if let Some(p) = pointer {
                        let lexeme = String::from_utf8_lossy(&self.b[start..self.i]).into_owned();
                        if lexeme.starts_with(|c: char| c == '-' || c.is_ascii_digit()) {
                            self.out.insert(p, lexeme);
                        }
                    }
                }
            }
        }
    }
    let mut s = S { b: raw, i: 0, out: HashMap::new() };
    s.value(None, 0);
    s.out
}

/// Checks `params` (an object, or `{}` for `null`) against the method's pinned params struct. `raw` is the
/// request payload the value was parsed from.
pub(crate) fn predecode(method: &str, go_type: &str, params: &Value, raw: &[u8]) -> ApiResult<()> {
    let Value::Object(o) = params else { return Ok(()) };
    let lexemes = number_lexemes(raw);
    for spec in params_fields(method) {
        if let Some(v) = o.get(spec.name) {
            check_field(spec, v, &lexemes).map_err(|e| ApiError::invalid_request(format!("failed to unmarshal *api.{go_type}: json: {e}")))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn number_lexemes() {
        let m = super::number_lexemes(br#"{"a":1e3,"b":[1,2.5,{"c":3}],"d":{"e":4},"f":"9"}"#);
        assert_eq!(m.get("/a").map(String::as_str), Some("1e3"));
        assert_eq!(m.get("/b/1").map(String::as_str), Some("2.5"));
        assert!(!m.contains_key("/d/e") && !m.contains_key("/f") && !m.contains_key("/b/2"));
    }
}
