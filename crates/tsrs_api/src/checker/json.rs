// Small JSON object builder mirroring Go encoding/json tags (`omitempty`, `omitzero`).

use tsrs_checker::LiteralValue;
use tsrs_core::collections::OrderedMap;
use tsrs_core::json::Value;

#[derive(Default)]
pub(crate) struct Obj {
    map: OrderedMap<String, Value>,
}

impl Obj {
    pub(crate) fn new() -> Obj {
        Obj::default()
    }

    pub(crate) fn set(&mut self, key: &str, value: Value) {
        self.map.insert(key.to_string(), value);
    }

    pub(crate) fn num(&mut self, key: &str, n: f64) {
        self.set(key, Value::Number(n));
    }

    /// `omitzero` numbers.
    pub(crate) fn nonzero(&mut self, key: &str, n: f64) {
        if n != 0.0 {
            self.num(key, n);
        }
    }

    /// `omitempty` strings.
    pub(crate) fn str_nonempty(&mut self, key: &str, s: &str) {
        if !s.is_empty() {
            self.set(key, Value::String(s.to_string()));
        }
    }

    /// `omitempty` bools.
    pub(crate) fn bool_true(&mut self, key: &str, b: bool) {
        if b {
            self.set(key, Value::Bool(true));
        }
    }

    /// `omitempty` id slices.
    pub(crate) fn ids(&mut self, key: &str, ids: impl Iterator<Item = f64>) {
        let ids: Vec<Value> = ids.map(Value::Number).collect();
        if !ids.is_empty() {
            self.set(key, Value::Array(ids));
        }
    }

    pub(crate) fn extend(&mut self, other: Obj) {
        self.map.extend(other.map);
    }

    pub(crate) fn build(self) -> Value {
        Value::Object(self.map)
    }

    /// Emits the fields in `order` (unknown fields keep their relative order at the end).
    pub(crate) fn build_ordered(mut self, order: &[&str]) -> Value {
        let mut out = OrderedMap::default();
        for key in order {
            if let Some(v) = self.map.shift_remove(*key) {
                out.insert(key.to_string(), v);
            }
        }
        out.extend(self.map);
        Value::Object(out)
    }
}

pub(crate) fn obj() -> Obj {
    Obj::new()
}

/// Go `literalValueToJSON`.
pub(crate) fn literal_value_to_json(value: Option<LiteralValue>) -> Value {
    match value {
        Some(LiteralValue::String(s)) => Value::String(s.to_string()),
        Some(LiteralValue::Number(n)) => {
            let f = n.0;
            if f.is_infinite() {
                Value::String(if f > 0.0 { "+Infinity" } else { "-Infinity" }.to_string())
            } else if f.is_nan() {
                Value::String("NaN".to_string())
            } else {
                Value::Number(f)
            }
        }
        Some(LiteralValue::Boolean(b)) => Value::Bool(b),
        Some(LiteralValue::BigInt(b)) => Value::String(b.to_string()),
        None => Value::Null,
    }
}
