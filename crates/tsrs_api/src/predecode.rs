// Go decodes a request's whole params struct (proto.go `unmarshalPayload`) before a handler runs, so a field of
// the wrong JSON kind is an invalid request even when another field would fail a lookup first. This checks the
// top-level fields of each method's pinned params struct (`paramfields.rs`, generated from proto.go) in Go's
// field order, for every method (core and checker lanes), before dispatch. Unknown keys are ignored, as Go does.
// Error class and `failed to unmarshal *api.<T>` prefix match Go; the jsontext wording after it is approximate
// except for DocumentIdentifier, whose custom decoder text is reproduced.

use tsrs_core::json::Value;

use crate::handler::{ApiError, ApiResult};
use crate::paramfields::{params_fields, FieldKind, FieldSpec};

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

fn check_field(spec: &FieldSpec, v: &Value) -> Result<(), String> {
    let pointer = format!("/{}", spec.name);
    let mismatch = || format!("cannot unmarshal JSON {} into Go {} within \"{pointer}\"", json_kind(v), go_type_name(spec.go_type));
    if matches!(v, Value::Null) && spec.kind != FieldKind::Doc && spec.kind != FieldKind::DocList {
        // `null` into a non-pointer field leaves the zero value; into a pointer it is nil.
        return Ok(());
    }
    match spec.kind {
        FieldKind::Any => Ok(()),
        FieldKind::Str => matches!(v, Value::String(_)).then_some(()).ok_or_else(mismatch),
        FieldKind::Bool => matches!(v, Value::Bool(_)).then_some(()).ok_or_else(mismatch),
        FieldKind::Int => match v {
            Value::Number(n) if n.fract() == 0.0 => Ok(()),
            _ => Err(mismatch()),
        },
        FieldKind::UInt => match v {
            Value::Number(n) if n.fract() == 0.0 && *n >= 0.0 => Ok(()),
            _ => Err(mismatch()),
        },
        FieldKind::Array => matches!(v, Value::Array(_)).then_some(()).ok_or_else(mismatch),
        FieldKind::Object => matches!(v, Value::Object(_)).then_some(()).ok_or_else(mismatch),
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
            _ => Err(mismatch()),
        },
    }
}

/// Checks `params` (already known to be an object or `{}`) against the method's pinned params struct.
pub(crate) fn predecode(method: &str, go_type: &str, params: &Value) -> ApiResult<()> {
    let Value::Object(o) = params else { return Ok(()) };
    for spec in params_fields(method) {
        if let Some(v) = o.get(spec.name) {
            check_field(spec, v).map_err(|e| ApiError::invalid_request(format!("failed to unmarshal *api.{go_type}: json: {e}")))?;
        }
    }
    Ok(())
}
