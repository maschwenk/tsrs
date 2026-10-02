// Go's structcodec.go is a reflection-driven object decoder: it enforces that the value is an object,
// that every `lsp:"required"` field is present, and that a JSON null is rejected for a nilable field
// whose spec is not nullable. Without reflection, the generator emits the per-type field loop and the
// strictness decisions (which fields are required / reject null) itself, and calls the helpers below.

use tsrs_core::collections::OrderedMap;

use crate::json::{is_null, kind, Json, JsonError, Value};
use crate::lsp::{err_missing, err_not_object, err_null};

// unmarshalStruct's object check. A non-strict structure (one Go decodes with json/v2's default struct
// codec) decodes null as its zero value and rejects other non-objects with json/v2's kind error.
pub fn struct_members<'a>(
    v: &'a Value,
    go_type: &str,
    strict: bool,
) -> Result<Option<&'a OrderedMap<String, Value>>, JsonError> {
    match v {
        Value::Object(members) => Ok(Some(members)),
        Value::Null if !strict => Ok(None),
        _ if strict => Err(JsonError::method(go_type, err_not_object(kind(v)))),
        _ => Err(JsonError::mismatch(v, go_type)),
    }
}

// A field decoded with json/v2's codec for its type.
pub fn field<T: Json>(name: &str, v: &Value) -> Result<T, JsonError> {
    T::from_json(v).map_err(|e| e.within(name))
}

// A required nilable field that rejects null.
pub fn field_non_null<T: Json>(name: &str, v: &Value, go_type: &str) -> Result<T, JsonError> {
    if is_null(v) {
        return Err(JsonError::method(go_type, err_null(name)).within(name));
    }
    field(name, v)
}

// An optional (pointer) field: null decodes as nil.
pub fn opt<T: Json>(name: &str, v: &Value) -> Result<Option<T>, JsonError> {
    if is_null(v) {
        return Ok(None);
    }
    field(name, v).map(Some)
}

// An optional (pointer) field that rejects null.
pub fn opt_non_null<T: Json>(name: &str, v: &Value, go_type: &str) -> Result<Option<T>, JsonError> {
    if is_null(v) {
        return Err(JsonError::method(go_type, err_null(name)).within(name));
    }
    field(name, v).map(Some)
}

// structcodec.go:100
pub fn check_required(seen: u64, required_names: &[&str], go_type: &str) -> Result<(), JsonError> {
    let required_mask = if required_names.len() == 64 { u64::MAX } else { (1u64 << required_names.len()) - 1 };
    let missing = required_mask & !seen;
    if missing != 0 {
        let mut missing_props = Vec::new();
        for (id, n) in required_names.iter().enumerate() {
            if missing & (1 << id) != 0 {
                missing_props.push(*n);
            }
        }
        return Err(JsonError::method(go_type, err_missing(&missing_props)));
    }
    Ok(())
}

// A union arm decoded with its own codec (Go `o.X = new(T); json.Unmarshal(data, o.X)`).
pub fn arm<T: Json>(v: &Value) -> Result<Option<T>, JsonError> {
    T::from_json(v).map(Some)
}

pub fn value_str(v: &Value) -> Option<&str> {
    match v {
        Value::String(s) => Some(s),
        _ => None,
    }
}

// structcodec.go:172
pub struct DiscriminatedStructDecoder<'a> {
    type_name: &'static str,
    discriminator: &'static str,
    discriminator_value: Option<&'a Value>,
}

// structcodec.go:184
pub fn scan_discriminated_struct<'a>(
    v: &'a Value,
    go_type: &str,
    type_name: &'static str,
    discriminator: &'static str,
) -> Result<DiscriminatedStructDecoder<'a>, JsonError> {
    let Value::Object(members) = v else {
        return Err(JsonError::method(go_type, err_not_object(kind(v))));
    };
    Ok(DiscriminatedStructDecoder { type_name, discriminator, discriminator_value: members.get(discriminator) })
}

impl DiscriminatedStructDecoder<'_> {
    // Go compares the raw discriminator value against the quoted literal.
    pub fn discriminator_str(&self) -> Option<&str> {
        self.discriminator_value.and_then(value_str)
    }

    // structcodec.go:226
    pub fn invalid_discriminator(&self, go_type: &str) -> JsonError {
        let Some(value) = self.discriminator_value else {
            return JsonError::method(
                go_type,
                format!(
                    "invalid {}: missing discriminator {}",
                    self.type_name,
                    tsrs_core::json::marshal_string(self.discriminator)
                ),
            );
        };
        JsonError::method(
            go_type,
            format!(
                "invalid {} discriminator {}: {}",
                self.type_name,
                tsrs_core::json::marshal_string(self.discriminator),
                tsrs_core::json::marshal(value).unwrap_or_default()
            ),
        )
        .within(self.discriminator)
    }
}

// structcodec.go:235: the arm is decoded with its struct spec, and errors of that decoding are reported
// against the union's type.
pub fn unmarshal_discriminated_arm<T: Json>(v: &Value, union_go_type: &str) -> Result<T, JsonError> {
    T::from_json(v).map_err(|mut e| {
        if e.go_type == T::GO_TYPE {
            e.go_type = union_go_type.to_string();
        }
        e
    })
}

// Go `cmp.Compare`.
pub fn cmp_compare<T: PartialOrd>(a: &T, b: &T) -> i32 {
    if a < b {
        -1
    } else if a > b {
        1
    } else {
        0
    }
}
