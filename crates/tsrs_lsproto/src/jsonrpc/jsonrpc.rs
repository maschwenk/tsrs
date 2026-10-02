// Package jsonrpc provides generic JSON-RPC 2.0 types and utilities
// that can be shared between LSP and other JSON-RPC based protocols.

use std::fmt;

use crate::json::{kind, Json, JsonError, ObjectWriter, Value};
use crate::structcodec::{field, opt, struct_members};

// jsonrpc.go:14
// JSONRPCVersion represents the JSON-RPC version field, always "2.0".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct JSONRPCVersion;

const JSON_RPC_VERSION: &str = "2.0";

pub const ERR_INVALID_JSONRPC_VERSION: &str = "invalid JSON-RPC version";

impl Json for JSONRPCVersion {
    const GO_TYPE: &'static str = "jsonrpc.JSONRPCVersion";

    // jsonrpc.go:18
    fn to_json(&self) -> Value {
        Value::String(JSON_RPC_VERSION.to_string())
    }

    // jsonrpc.go:24
    fn from_json(v: &Value) -> Result<Self, JsonError> {
        if !matches!(v, Value::String(s) if s == JSON_RPC_VERSION) {
            return Err(JsonError::method_v1(kind(v), Self::GO_TYPE, ERR_INVALID_JSONRPC_VERSION));
        }
        Ok(JSONRPCVersion)
    }
}

// jsonrpc.go:32
// ID represents a JSON-RPC message ID, which can be either a string or integer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ID {
    str: String,
    int: i32,
}

// jsonrpc.go:38
// NewID creates an ID from an IntegerOrString value.
pub fn new_id(raw_value: &IntegerOrString) -> ID {
    if let Some(s) = &raw_value.string {
        return ID { str: s.clone(), int: 0 };
    }
    ID { str: String::new(), int: raw_value.integer.unwrap() }
}

// jsonrpc.go:46
// NewIDString creates a string ID.
pub fn new_id_string(str: &str) -> ID {
    ID { str: str.to_string(), int: 0 }
}

// jsonrpc.go:51
// NewIDInt creates an integer ID.
pub fn new_id_int(i: i32) -> ID {
    ID { str: String::new(), int: i }
}

impl ID {
    // jsonrpc.go:55
    pub fn string(&self) -> String {
        if !self.str.is_empty() {
            return self.str.clone();
        }
        self.int.to_string()
    }

    // jsonrpc.go:77
    pub fn try_int(&self) -> Option<i32> {
        if !self.str.is_empty() {
            return None;
        }
        Some(self.int)
    }

    // jsonrpc.go:84
    pub fn must_int(&self) -> i32 {
        if !self.str.is_empty() {
            panic!("ID is not an integer");
        }
        self.int
    }
}

impl fmt::Display for ID {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.string())
    }
}

impl Json for ID {
    const GO_TYPE: &'static str = "jsonrpc.ID";

    // jsonrpc.go:62
    fn to_json(&self) -> Value {
        if !self.str.is_empty() {
            return Value::String(self.str.clone());
        }
        self.int.to_json()
    }

    // jsonrpc.go:69
    fn from_json(v: &Value) -> Result<Self, JsonError> {
        if let Value::String(s) = v {
            return Ok(ID { str: s.clone(), int: 0 });
        }
        Ok(ID { str: String::new(), int: i32::from_json(v)? })
    }
}

// jsonrpc.go:92
// IntegerOrString is a helper type for creating IDs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct IntegerOrString {
    pub integer: Option<i32>,
    pub string: Option<String>,
}

// jsonrpc.go:98
// ResponseError represents a JSON-RPC error response.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResponseError {
    pub code: i32,
    pub message: String,
    pub data: Option<Value>,
}

impl ResponseError {
    // jsonrpc.go:104
    pub fn string(&self) -> String {
        let data = tsrs_core::json::marshal(self.data.as_ref().unwrap_or(&Value::Null));
        if data.is_err() {
            return format!("[{}]: {}\n[]", self.code, self.message);
        }
        format!("[{}]: {}", self.code, self.message)
    }
}

// jsonrpc.go:115
impl fmt::Display for ResponseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.string())
    }
}

impl std::error::Error for ResponseError {}

impl Json for ResponseError {
    const GO_TYPE: &'static str = "jsonrpc.ResponseError";

    fn to_json(&self) -> Value {
        let mut w = ObjectWriter::new(3);
        w.field("code", &self.code);
        w.field("message", &self.message);
        w.opt("data", &self.data);
        w.finish()
    }

    fn from_json(v: &Value) -> Result<Self, JsonError> {
        let mut s = Self::default();
        let Some(members) = struct_members(v, Self::GO_TYPE, false)? else {
            return Ok(s);
        };
        for (k, v) in members {
            match k.as_str() {
                "code" => s.code = field(k, v)?,
                "message" => s.message = field(k, v)?,
                "data" => s.data = opt(k, v)?,
                _ => {}
            }
        }
        Ok(s)
    }
}

// jsonrpc.go:120
// Standard JSON-RPC error codes.
pub const CODE_PARSE_ERROR: i32 = -32700;
pub const CODE_INVALID_REQUEST: i32 = -32600;
pub const CODE_METHOD_NOT_FOUND: i32 = -32601;
pub const CODE_INVALID_PARAMS: i32 = -32602;
pub const CODE_INTERNAL_ERROR: i32 = -32603;

// jsonrpc.go:129
// MessageKind indicates what type of message this is.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum MessageKind {
    #[default]
    Notification,
    Request,
    Response,
}

// jsonrpc.go:139
// Message represents a raw JSON-RPC message that can be a request, notification, or response.
// Unlike lsproto.Message, this keeps params/result as raw JSON for generic handling.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Message {
    pub id: Option<ID>,
    pub method: String,
    pub params: Option<Value>,
    pub result: Option<Value>,
    pub error: Option<ResponseError>,
}

impl Message {
    // jsonrpc.go:149
    // Kind returns the kind of message this is.
    pub fn kind(&self) -> MessageKind {
        if self.id.is_some() && self.method.is_empty() {
            return MessageKind::Response;
        }
        if self.id.is_none() {
            return MessageKind::Notification;
        }
        MessageKind::Request
    }

    // jsonrpc.go:160
    // IsRequest returns true if this message is a request (has ID and method).
    pub fn is_request(&self) -> bool {
        self.id.is_some() && !self.method.is_empty()
    }

    // jsonrpc.go:165
    // IsNotification returns true if this message is a notification (has method but no ID).
    pub fn is_notification(&self) -> bool {
        self.id.is_none() && !self.method.is_empty()
    }

    // jsonrpc.go:170
    // IsResponse returns true if this message is a response (has ID but no method).
    pub fn is_response(&self) -> bool {
        self.id.is_some() && self.method.is_empty()
    }
}

// A raw (json.Value) field: present, even when null.
fn raw(v: &Value) -> Option<Value> {
    Some(v.clone())
}

impl Json for Message {
    const GO_TYPE: &'static str = "jsonrpc.Message";

    fn to_json(&self) -> Value {
        let mut w = ObjectWriter::new(6);
        w.field("jsonrpc", &JSONRPCVersion);
        w.opt("id", &self.id);
        if !self.method.is_empty() {
            w.field("method", &self.method);
        }
        w.opt("params", &self.params);
        w.opt("result", &self.result);
        w.opt("error", &self.error);
        w.finish()
    }

    fn from_json(v: &Value) -> Result<Self, JsonError> {
        let mut s = Self::default();
        let Some(members) = struct_members(v, Self::GO_TYPE, false)? else {
            return Ok(s);
        };
        for (k, v) in members {
            match k.as_str() {
                "jsonrpc" => {
                    field::<JSONRPCVersion>(k, v)?;
                }
                "id" => s.id = opt(k, v)?,
                "method" => s.method = field(k, v)?,
                "params" => s.params = raw(v),
                "result" => s.result = raw(v),
                "error" => s.error = opt(k, v)?,
                _ => {}
            }
        }
        Ok(s)
    }
}

// jsonrpc.go:175
// RequestMessage is a convenience type for creating request/notification messages.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RequestMessage {
    pub id: Option<ID>,
    pub method: String,
    pub params: Option<Value>,
}

impl Json for RequestMessage {
    const GO_TYPE: &'static str = "jsonrpc.RequestMessage";

    fn to_json(&self) -> Value {
        let mut w = ObjectWriter::new(4);
        w.field("jsonrpc", &JSONRPCVersion);
        w.opt("id", &self.id);
        w.field("method", &self.method);
        w.opt("params", &self.params);
        w.finish()
    }

    fn from_json(v: &Value) -> Result<Self, JsonError> {
        let mut s = Self::default();
        let Some(members) = struct_members(v, Self::GO_TYPE, false)? else {
            return Ok(s);
        };
        for (k, v) in members {
            match k.as_str() {
                "jsonrpc" => {
                    field::<JSONRPCVersion>(k, v)?;
                }
                "id" => s.id = opt(k, v)?,
                "method" => s.method = field(k, v)?,
                "params" => s.params = opt(k, v)?,
                _ => {}
            }
        }
        Ok(s)
    }
}

// jsonrpc.go:183
// ResponseMessage is a convenience type for creating response messages.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResponseMessage {
    pub id: Option<ID>,
    pub result: Option<Value>,
    pub error: Option<ResponseError>,
}

impl Json for ResponseMessage {
    const GO_TYPE: &'static str = "jsonrpc.ResponseMessage";

    fn to_json(&self) -> Value {
        let mut w = ObjectWriter::new(4);
        w.field("jsonrpc", &JSONRPCVersion);
        w.opt("id", &self.id);
        w.opt("result", &self.result);
        w.opt("error", &self.error);
        w.finish()
    }

    fn from_json(v: &Value) -> Result<Self, JsonError> {
        let mut s = Self::default();
        let Some(members) = struct_members(v, Self::GO_TYPE, false)? else {
            return Ok(s);
        };
        for (k, v) in members {
            match k.as_str() {
                "jsonrpc" => {
                    field::<JSONRPCVersion>(k, v)?;
                }
                "id" => s.id = opt(k, v)?,
                "result" => s.result = opt(k, v)?,
                "error" => s.error = opt(k, v)?,
                _ => {}
            }
        }
        Ok(s)
    }
}
