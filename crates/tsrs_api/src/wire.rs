// Small helpers for building/reading the Go wire JSON shapes with `tsrs_core::json::Value`.

use tsrs_core::collections::OrderedMap;
use tsrs_core::json::Value;

use crate::handler::{ApiError, ApiResult};

/// Ordered JSON object builder (Go struct field order matters for byte-identical output).
#[derive(Default)]
pub struct Obj(OrderedMap<String, Value>);

impl Obj {
    pub fn new() -> Obj {
        Obj(OrderedMap::default())
    }
    pub fn set(mut self, key: &str, value: Value) -> Obj {
        self.0.insert(key.to_string(), value);
        self
    }
    pub fn set_opt(self, key: &str, value: Option<Value>) -> Obj {
        match value {
            Some(v) => self.set(key, v),
            None => self,
        }
    }
    /// Go encoding/json/v2 `omitempty`: omitted when the encoded value is `null`, `""`, `[]` or `{}`
    /// (unlike v1, `false` and `0` are NOT omitted).
    pub fn set_omitempty(self, key: &str, value: Value) -> Obj {
        let empty = match &value {
            Value::Null => true,
            Value::String(s) => s.is_empty(),
            Value::Array(a) => a.is_empty(),
            Value::Object(o) => o.is_empty(),
            _ => false,
        };
        if empty {
            self
        } else {
            self.set(key, value)
        }
    }

    pub fn build(self) -> Value {
        Value::Object(self.0)
    }
}

pub fn s(v: impl Into<String>) -> Value {
    Value::String(v.into())
}
pub fn n(v: impl Into<f64>) -> Value {
    Value::Number(v.into())
}
pub fn b(v: bool) -> Value {
    Value::Bool(v)
}
pub fn strings(v: impl IntoIterator<Item = impl Into<String>>) -> Value {
    Value::Array(v.into_iter().map(|x| Value::String(x.into())).collect())
}

/// Typed accessors over request params with Go-like error text.
pub struct Params<'a>(pub &'a Value);

static NULL: Value = Value::Null;

impl<'a> Params<'a> {
    pub fn object(&self) -> ApiResult<&'a OrderedMap<String, Value>> {
        match self.0 {
            Value::Object(o) => Ok(o),
            _ => Err(ApiError::invalid_request("params must be an object")),
        }
    }
    pub fn get(&self, key: &str) -> &'a Value {
        match self.0 {
            Value::Object(o) => o.get(key).unwrap_or(&NULL),
            _ => &NULL,
        }
    }
    /// A `DocumentIdentifier` field: absent is Go's zero value (empty file name); explicit `null` is an error.
    pub fn document(&self, key: &str) -> ApiResult<DocumentIdentifier> {
        match self.0 {
            Value::Object(o) if !o.contains_key(key) => Ok(DocumentIdentifier::FileName(String::new())),
            _ => DocumentIdentifier::parse(self.get(key), key),
        }
    }

    pub fn has(&self, key: &str) -> bool {
        !matches!(self.get(key), Value::Null)
    }
    pub fn u64(&self, key: &str) -> ApiResult<u64> {
        as_u64(self.get(key), key)
    }
    pub fn opt_u64(&self, key: &str) -> ApiResult<Option<u64>> {
        match self.get(key) {
            Value::Null => Ok(None),
            v => as_u64(v, key).map(Some),
        }
    }
    pub fn str(&self, key: &str) -> ApiResult<&'a str> {
        match self.get(key) {
            Value::String(s) => Ok(s),
            Value::Null => Ok(""),
            _ => Err(ApiError::invalid_request(format!("{key} must be a string"))),
        }
    }
    pub fn bool(&self, key: &str) -> ApiResult<bool> {
        match self.get(key) {
            Value::Bool(b) => Ok(*b),
            Value::Null => Ok(false),
            _ => Err(ApiError::invalid_request(format!("{key} must be a boolean"))),
        }
    }
    pub fn array(&self, key: &str) -> ApiResult<&'a [Value]> {
        match self.get(key) {
            Value::Array(a) => Ok(a),
            Value::Null => Ok(&[]),
            _ => Err(ApiError::invalid_request(format!("{key} must be an array"))),
        }
    }
    pub fn strings(&self, key: &str) -> ApiResult<Vec<String>> {
        self.array(key)?
            .iter()
            .map(|v| match v {
                Value::String(s) => Ok(s.clone()),
                // Go: a null element of a []string is the zero string.
                Value::Null => Ok(String::new()),
                _ => Err(ApiError::invalid_request(format!("{key} must be an array of strings"))),
            })
            .collect()
    }
}

pub fn as_u64(v: &Value, key: &str) -> ApiResult<u64> {
    match v {
        // Range and syntax were checked by `predecode` against the field's Go type.
        Value::Number(x) if *x >= 0.0 && x.fract() == 0.0 && *x < 18446744073709551616.0 => Ok(*x as u64),
        Value::Null => Ok(0),
        _ => Err(ApiError::invalid_request(format!("{key} must be a non-negative integer"))),
    }
}

/// Go `DocumentIdentifier`: `string | { uri: string }` on the wire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocumentIdentifier {
    FileName(String),
    Uri(String),
}

impl DocumentIdentifier {
    pub fn parse(v: &Value, key: &str) -> ApiResult<DocumentIdentifier> {
        match v {
            Value::String(s) => Ok(DocumentIdentifier::FileName(s.clone())),
            // proto.go `DocumentIdentifier.UnmarshalJSONFrom`: an explicit null is an error (an absent field is the
            // zero value; see `Params::document`).
            Value::Null => Err(ApiError::invalid_request("DocumentIdentifier: expected string or object, got null")),
            Value::Object(o) => match o.get("uri") {
                Some(Value::String(u)) => Ok(DocumentIdentifier::Uri(u.clone())),
                // Go reads `{}` (no uri) as an empty file name.
                _ => Ok(DocumentIdentifier::FileName(String::new())),
            },
            _ => Err(ApiError::invalid_request(format!("DocumentIdentifier: expected string or object for {key}"))),
        }
    }
    pub fn parse_list(v: &[Value], key: &str) -> ApiResult<Vec<DocumentIdentifier>> {
        v.iter().map(|d| DocumentIdentifier::parse(d, key)).collect()
    }
    /// Go `ToFileName`.
    pub fn to_file_name(&self) -> String {
        match self {
            DocumentIdentifier::FileName(f) => f.clone(),
            DocumentIdentifier::Uri(u) => tsrs_lsproto::DocumentUri(u.clone()).file_name(),
        }
    }
    /// Go `ToAbsoluteFileName`.
    pub fn to_absolute_file_name(&self, cwd: &str) -> String {
        match self {
            DocumentIdentifier::FileName(f) => tsrs_core::tspath::get_normalized_absolute_path(f, cwd),
            DocumentIdentifier::Uri(u) => tsrs_lsproto::DocumentUri(u.clone()).file_name(),
        }
    }
}

impl std::fmt::Display for DocumentIdentifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DocumentIdentifier::FileName(s) | DocumentIdentifier::Uri(s) => f.write_str(s),
        }
    }
}
