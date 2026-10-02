// Go built-ins and standard-library pieces the generated tests call (tools/gen-fourslash maps them here), and
// `Any`, the Rust form of Go's `any` / `interface{}` values in the harness API.

use std::fmt;
use std::hash::Hash;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;

use tsrs_core::collections::OrderedMap;
use tsrs_core::json::Value;
use tsrs_lsproto as lsproto;

use crate::fourslash::{EditRange, Marker, RangeMarker};
use crate::testing::T;

// Go `any` (and its aliases MarkerInput, MarkerOrRangeOrName, CompletionsExpectedItem,
// ExpectedCompletionEditRange). One variant per dynamic type the tests store in one; `Nil` is the nil interface.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Any {
    #[default]
    Nil,
    String(String),
    Bool(bool),
    Int(i32),
    Float64(f64),
    Marker(Arc<Marker>),
    RangeMarker(Arc<RangeMarker>),
    EditRange(EditRange),
    CompletionItem(lsproto::CompletionItem),
    // fourslash.Ignored / util.Ignored (`struct{}{}`)
    Ignored,
    StringSlice(Vec<String>),
    MarkerSlice(Vec<Arc<Marker>>),
    // []any and map[string]any, as produced by Go's encoding/json for object markers
    Slice(Vec<Any>),
    Map(OrderedMap<String, Any>),
}

impl Any {
    pub fn is_nil(&self) -> bool {
        matches!(self, Any::Nil)
    }

    // Go's `x.(T)` type assertions (panic when the dynamic type differs).
    pub fn assert_bool(&self) -> bool {
        match self {
            Any::Bool(b) => *b,
            _ => panic!("interface conversion: interface {{}} is {}, not bool", self.type_name()),
        }
    }

    pub fn assert_string(&self) -> String {
        match self {
            Any::String(s) => s.clone(),
            _ => panic!("interface conversion: interface {{}} is {}, not string", self.type_name()),
        }
    }

    // Go's %T of the dynamic type.
    pub fn type_name(&self) -> &'static str {
        match self {
            Any::Nil => "nil",
            Any::String(_) => "string",
            Any::Bool(_) => "bool",
            Any::Int(_) => "int",
            Any::Float64(_) => "float64",
            Any::Marker(_) => "*fourslash.Marker",
            Any::RangeMarker(_) => "*fourslash.RangeMarker",
            Any::EditRange(_) => "*fourslash.EditRange",
            Any::CompletionItem(_) => "*lsproto.CompletionItem",
            Any::Ignored => "struct {}",
            Any::StringSlice(_) => "[]string",
            Any::MarkerSlice(_) => "[]*fourslash.Marker",
            Any::Slice(_) => "[]interface {}",
            Any::Map(_) => "map[string]interface {}",
        }
    }

    // A value decoded by Go's encoding/json into `any`.
    pub fn from_json(v: &Value) -> Any {
        match v {
            Value::Null => Any::Nil,
            Value::Bool(b) => Any::Bool(*b),
            Value::Number(n) => Any::Float64(*n),
            Value::String(s) => Any::String(s.clone()),
            Value::Array(a) => Any::Slice(a.iter().map(Any::from_json).collect()),
            Value::Object(o) => Any::Map(o.iter().map(|(k, v)| (k.clone(), Any::from_json(v))).collect()),
        }
    }

    // The JSON value Go's encoding/json would marshal this as (for map[string]any configuration literals).
    pub fn to_json(&self) -> Value {
        match self {
            Any::Nil => Value::Null,
            Any::String(s) => Value::String(s.clone()),
            Any::Bool(b) => Value::Bool(*b),
            Any::Int(i) => Value::Number(*i as f64),
            Any::Float64(f) => Value::Number(*f),
            Any::StringSlice(v) => Value::Array(v.iter().map(|s| Value::String(s.clone())).collect()),
            Any::Slice(v) => Value::Array(v.iter().map(Any::to_json).collect()),
            Any::Map(m) => Value::Object(m.iter().map(|(k, v)| (k.clone(), v.to_json())).collect()),
            Any::Ignored => Value::Object(OrderedMap::default()),
            _ => panic!("json: unsupported value of type {}", self.type_name()),
        }
    }
}

impl From<Arc<RangeMarker>> for Any {
    fn from(r: Arc<RangeMarker>) -> Any {
        Any::RangeMarker(r)
    }
}

impl From<Arc<Marker>> for Any {
    fn from(m: Arc<Marker>) -> Any {
        Any::Marker(m)
    }
}

impl From<String> for Any {
    fn from(s: String) -> Any {
        Any::String(s)
    }
}

// Runs the rest of a block whose Go code has a `defer`: the deferred call runs after it, panicking or not.
pub fn run(f: impl FnOnce()) -> std::thread::Result<()> {
    panic::catch_unwind(AssertUnwindSafe(f))
}

pub fn resume(r: std::thread::Result<()>) {
    if let Err(p) = r {
        panic::resume_unwind(p);
    }
}

// A Go construct tools/gen-fourslash could not translate.
pub fn untranslated(t: &T, what: &str) -> ! {
    t.fatal(&format!("untranslated: {what}"))
}

pub fn strs(v: &[String]) -> Vec<&str> {
    v.iter().map(|s| s.as_str()).collect()
}

pub fn owned_strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

pub fn map_of<K: Hash + Eq, V, const N: usize>(items: [(K, V); N]) -> OrderedMap<K, V> {
    let mut m = OrderedMap::default();
    for (k, v) in items {
        m.insert(k, v);
    }
    m
}

// core.Filter
pub fn filter<T: Clone>(xs: &[T], mut pred: impl FnMut(&T) -> bool) -> Vec<T> {
    xs.iter().filter(|x| pred(x)).cloned().collect()
}

// core.Map
pub fn map<T, U>(xs: &[T], f: impl FnMut(&T) -> U) -> Vec<U> {
    xs.iter().map(f).collect()
}

// core.OrElse
pub fn or_else<T: Default + PartialEq>(value: T, default_value: T) -> T {
    if value != T::default() {
        value
    } else {
        default_value
    }
}

// append(s, items...)
pub fn append<T: Clone>(mut s: Vec<T>, items: &[T]) -> Vec<T> {
    s.extend_from_slice(items);
    s
}

// s[lo:hi] on a string
pub fn slice_str(s: &str, lo: Option<usize>, hi: Option<usize>) -> String {
    s[lo.unwrap_or(0)..hi.unwrap_or(s.len())].to_string()
}

// strconv.Quote
pub fn quote(s: &str) -> String {
    let mut b = String::with_capacity(s.len() + 2);
    b.push('"');
    for c in s.chars() {
        match c {
            '"' => b.push_str("\\\""),
            '\\' => b.push_str("\\\\"),
            '\x07' => b.push_str("\\a"),
            '\x08' => b.push_str("\\b"),
            '\x0c' => b.push_str("\\f"),
            '\n' => b.push_str("\\n"),
            '\r' => b.push_str("\\r"),
            '\t' => b.push_str("\\t"),
            '\x0b' => b.push_str("\\v"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => b.push_str(&format!("\\x{:02x}", c as u32)),
            c if c.is_control() => {
                if (c as u32) < 0x10000 {
                    b.push_str(&format!("\\u{:04x}", c as u32))
                } else {
                    b.push_str(&format!("\\U{:08x}", c as u32))
                }
            }
            c => b.push(c),
        }
    }
    b.push('"');
    b
}

// map[string]any configuration literal -> the JSON object lsutil.ParseUserPreferences receives.
pub fn json_object(m: &OrderedMap<String, Any>) -> OrderedMap<String, Value> {
    m.iter().map(|(k, v)| (k.clone(), v.to_json())).collect()
}

// `%v` of a non-basic value (approximated with Debug).
pub struct GoFmt<'a, T: fmt::Debug>(pub &'a T);

impl<T: fmt::Debug> fmt::Display for GoFmt<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

pub mod strings {
    pub fn has_prefix(s: &str, prefix: &str) -> bool {
        s.starts_with(prefix)
    }

    pub fn has_suffix(s: &str, suffix: &str) -> bool {
        s.ends_with(suffix)
    }

    pub fn index(s: &str, substr: &str) -> i32 {
        s.find(substr).map(|i| i as i32).unwrap_or(-1)
    }

    pub fn join(elems: &[&str], sep: &str) -> String {
        elems.join(sep)
    }
}

// gotest.tools/v3/assert
pub mod assert {
    use std::fmt::Debug;

    use crate::testing::T;

    fn with_msg(base: String, msg: &str) -> String {
        if msg.is_empty() {
            base
        } else {
            format!("{base}: {msg}")
        }
    }

    pub fn equal<A: PartialEq + Debug>(t: &T, x: A, y: A, msg: &str) {
        if x != y {
            t.fatal(&with_msg(format!("assertion failed: {x:?} (x) != {y:?} (y)"), msg));
        }
    }

    pub fn deep_equal<A: PartialEq + Debug>(t: &T, x: A, y: A, msg: &str) {
        if x != y {
            t.fatal(&with_msg(format!("assertion failed: \n--- x\n+++ y\n{x:#?}\n{y:#?}"), msg));
        }
    }

    pub fn assert(t: &T, cond: bool, msg: &str) {
        if !cond {
            t.fatal(&with_msg("assertion failed".to_string(), msg));
        }
    }

    pub fn check(t: &T, cond: bool, msg: &str) -> bool {
        if !cond {
            t.error(&with_msg("assertion failed".to_string(), msg));
        }
        cond
    }
}

// Go conversions `T(s)` from a string to a named string type that is a Rust enum.
pub mod conv {
    use tsrs_ls::lsutil;

    pub fn quote_preference(s: &str) -> lsutil::QuotePreference {
        match s {
            "" => lsutil::QuotePreference::Unknown,
            "auto" => lsutil::QuotePreference::Auto,
            "double" => lsutil::QuotePreference::Double,
            "single" => lsutil::QuotePreference::Single,
            _ => panic!("lsutil.QuotePreference({s:?}) has no Rust representation"),
        }
    }
}
