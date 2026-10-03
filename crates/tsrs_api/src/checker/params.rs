// Decoding of the pinned request parameter shapes (tsc/internal/api/proto.go `*Params` structs).
//
// Go decodes params with encoding/json v2 into typed structs: a missing field (or `null`) leaves the
// zero value, a field of the wrong JSON type is an `ErrInvalidRequest`. The helpers below follow the
// same rules and keep every number bounded (no silent truncation of ids or positions).

use tsrs_core::json::Value;
use tsrs_lsproto::DocumentUri;

use super::host::{CheckerError, CheckerResult};

const MAX_SAFE_INTEGER: f64 = 9007199254740991.0;

pub(crate) fn object<'a>(params: &'a Value, method: &str) -> CheckerResult<&'a tsrs_core::collections::OrderedMap<String, Value>> {
    match params {
        Value::Object(map) => Ok(map),
        _ => Err(CheckerError::invalid(format!("{method}: params must be an object"))),
    }
}

fn get<'a>(obj: &'a tsrs_core::collections::OrderedMap<String, Value>, name: &str) -> Option<&'a Value> {
    match obj.get(name) {
        None | Some(Value::Null) => None,
        Some(v) => Some(v),
    }
}

fn wrong_type(name: &str, expected: &str) -> CheckerError {
    CheckerError::invalid(format!("field {name:?}: expected {expected}"))
}

fn unsigned(v: &Value, name: &str, max: f64) -> CheckerResult<u64> {
    match v {
        Value::Number(n) if n.fract() == 0.0 && *n >= 0.0 && *n <= max => Ok(*n as u64),
        _ => Err(wrong_type(name, "an unsigned integer in range")),
    }
}

fn signed32(v: &Value, name: &str) -> CheckerResult<i32> {
    match v {
        Value::Number(n) if n.fract() == 0.0 && *n >= i32::MIN as f64 && *n <= i32::MAX as f64 => Ok(*n as i32),
        _ => Err(wrong_type(name, "a 32-bit integer")),
    }
}

/// A typed view over one request's params object.
pub(crate) struct Params<'a> {
    obj: &'a tsrs_core::collections::OrderedMap<String, Value>,
}

impl<'a> Params<'a> {
    pub(crate) fn new(params: &'a Value, method: &str) -> CheckerResult<Params<'a>> {
        Ok(Params { obj: object(params, method)? })
    }

    pub(crate) fn raw(&self, name: &str) -> Option<&'a Value> {
        get(self.obj, name)
    }

    /// Go `uint64` ids (snapshot, symbol, signature handles). Missing → 0.
    pub(crate) fn u64(&self, name: &str) -> CheckerResult<u64> {
        self.raw(name).map_or(Ok(0), |v| unsigned(v, name, MAX_SAFE_INTEGER))
    }

    /// Go `uint32` (type ids, positions, symbol flags). Missing → 0.
    pub(crate) fn u32(&self, name: &str) -> CheckerResult<u32> {
        self.raw(name).map_or(Ok(0), |v| unsigned(v, name, u32::MAX as f64).map(|n| n as u32))
    }

    pub(crate) fn opt_u32(&self, name: &str) -> CheckerResult<Option<u32>> {
        self.raw(name).map(|v| unsigned(v, name, u32::MAX as f64).map(|n| n as u32)).transpose()
    }

    /// Go `int32` (kind, index, flags). Missing → 0.
    pub(crate) fn i32(&self, name: &str) -> CheckerResult<i32> {
        self.raw(name).map_or(Ok(0), |v| signed32(v, name))
    }

    pub(crate) fn bool(&self, name: &str) -> CheckerResult<bool> {
        match self.raw(name) {
            None => Ok(false),
            Some(Value::Bool(b)) => Ok(*b),
            Some(_) => Err(wrong_type(name, "a boolean")),
        }
    }

    pub(crate) fn string(&self, name: &str) -> CheckerResult<&'a str> {
        match self.raw(name) {
            None => Ok(""),
            Some(Value::String(s)) => Ok(s),
            Some(_) => Err(wrong_type(name, "a string")),
        }
    }

    pub(crate) fn array(&self, name: &str) -> CheckerResult<&'a [Value]> {
        match self.raw(name) {
            None => Ok(&[]),
            Some(Value::Array(items)) => Ok(items),
            Some(_) => Err(wrong_type(name, "an array")),
        }
    }

    pub(crate) fn u32_array(&self, name: &str) -> CheckerResult<Vec<u32>> {
        self.array(name)?.iter().map(|v| unsigned(v, name, u32::MAX as f64).map(|n| n as u32)).collect()
    }

    pub(crate) fn string_array(&self, name: &str) -> CheckerResult<Vec<&'a str>> {
        self.array(name)?
            .iter()
            .map(|v| match v {
                Value::String(s) => Ok(s.as_str()),
                _ => Err(wrong_type(name, "an array of strings")),
            })
            .collect()
    }

    /// Go `project.ID` (a string; missing → "").
    pub(crate) fn project(&self) -> CheckerResult<&'a str> {
        self.string("project")
    }

    pub(crate) fn document(&self, name: &str) -> CheckerResult<DocumentIdentifier> {
        match self.raw(name) {
            None => Ok(DocumentIdentifier::default()),
            Some(v) => DocumentIdentifier::decode(v, name),
        }
    }

    pub(crate) fn opt_document(&self, name: &str) -> CheckerResult<Option<DocumentIdentifier>> {
        self.raw(name).map(|v| DocumentIdentifier::decode(v, name)).transpose()
    }

    pub(crate) fn documents(&self, name: &str) -> CheckerResult<Vec<DocumentIdentifier>> {
        self.array(name)?.iter().map(|v| DocumentIdentifier::decode(v, name)).collect()
    }

    pub(crate) fn symbol_ref(&self, name: &str) -> CheckerResult<SymbolReference> {
        match self.raw(name) {
            None => Ok(SymbolReference::default()),
            Some(v) => SymbolReference::decode(v, name),
        }
    }

    pub(crate) fn symbol_refs(&self, name: &str) -> CheckerResult<Vec<SymbolReference>> {
        self.array(name)?.iter().map(|v| SymbolReference::decode(v, name)).collect()
    }
}

/// Go `DocumentIdentifier`: `string | { uri: string }`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DocumentIdentifier {
    pub(crate) file_name: String,
    pub(crate) uri: String,
}

impl DocumentIdentifier {
    fn decode(v: &Value, name: &str) -> CheckerResult<DocumentIdentifier> {
        match v {
            Value::String(s) => Ok(DocumentIdentifier { file_name: s.clone(), uri: String::new() }),
            Value::Object(map) => match map.get("uri") {
                None | Some(Value::Null) => Ok(DocumentIdentifier::default()),
                Some(Value::String(uri)) => Ok(DocumentIdentifier { file_name: String::new(), uri: uri.clone() }),
                Some(_) => Err(wrong_type(name, "a document identifier ({ uri: string })")),
            },
            _ => Err(CheckerError::invalid(format!("DocumentIdentifier: expected string or object for field {name:?}"))),
        }
    }

    /// Go `DocumentIdentifier.ToFileName`.
    pub(crate) fn to_file_name(&self) -> String {
        if !self.uri.is_empty() {
            return DocumentUri(self.uri.clone()).file_name();
        }
        self.file_name.clone()
    }

    /// Go `DocumentIdentifier.String` (used in error messages).
    pub(crate) fn display(&self) -> &str {
        if !self.uri.is_empty() {
            &self.uri
        } else {
            &self.file_name
        }
    }
}

/// Go `SymbolOwnerKind`.
pub(crate) const SYMBOL_OWNER_KIND_FILE: u32 = 0;
pub(crate) const SYMBOL_OWNER_KIND_SNAPSHOT: u32 = 1;

/// Go `SymbolReference` (`SymbolOwner` embedded + `id`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SymbolReference {
    pub(crate) kind: u32,
    pub(crate) file: Option<Value>,
    pub(crate) snapshot: u64,
    pub(crate) project: String,
    pub(crate) id: u64,
}

impl SymbolReference {
    fn decode(v: &Value, name: &str) -> CheckerResult<SymbolReference> {
        let Value::Object(_) = v else {
            return Err(wrong_type(name, "a symbol reference object"));
        };
        let p = Params::new(v, name)?;
        let file = match p.raw("file") {
            None => None,
            Some(f @ Value::Object(_)) => Some(f.clone()),
            Some(_) => return Err(wrong_type("file", "a source file descriptor object")),
        };
        Ok(SymbolReference { kind: p.u32("kind")?, file, snapshot: p.u64("snapshot")?, project: p.string("project")?.to_string(), id: p.u64("id")? })
    }

    /// The `path` of a file reference's descriptor.
    pub(crate) fn file_path(&self) -> Option<&str> {
        match self.file.as_ref()? {
            Value::Object(map) => match map.get("path") {
                Some(Value::String(s)) => Some(s),
                _ => None,
            },
            _ => None,
        }
    }
}
