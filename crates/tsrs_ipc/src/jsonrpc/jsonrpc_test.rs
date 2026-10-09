// Message encoding and decoding as the JSON-RPC protocol reads and writes it. Go has no tests of its own for these;
// every expected string here is the Go implementation's output for the same input (JSONRPCProtocol and
// json.Unmarshal into ipc.Message at the pinned commit).

use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use tsrs_core::json::{self, Value};

use crate::jsonrpc::{self, new_id_int, new_id_string, Message, ResponseError, CODE_INTERNAL_ERROR, ID};
use crate::{new_jsonrpc_protocol, Protocol};

#[derive(Clone, Default)]
struct sharedBuffer(Arc<Mutex<Vec<u8>>>);

impl sharedBuffer {
    fn take(&self) -> String {
        String::from_utf8(std::mem::take(&mut *self.0.lock().unwrap())).unwrap()
    }
}

impl Write for sharedBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn value(text: &str) -> Value {
    json::unmarshal(text).unwrap()
}

fn frame(body: &str) -> String {
    format!("Content-Length: {}\r\n\r\n{body}", body.len())
}

// What JSONRPCProtocol writes: member order, omitzero, Go's id encoding (an empty string id is the integer 0),
// compact params, and json/v2's string escaping (no HTML escaping, U+2028 and DEL as they are).
#[test]
fn protocol_writes_go_wire_bytes() {
    let out = sharedBuffer::default();
    let p = new_jsonrpc_protocol(io::empty(), out.clone());

    p.write_request(&new_id_string("api1"), "initialize", Some(&value(r#"{ "locale" : "en", "positionEncodings": ["utf-8","utf-16"] }"#))).unwrap();
    assert_eq!(out.take(), frame(r#"{"jsonrpc":"2.0","id":"api1","method":"initialize","params":{"locale":"en","positionEncodings":["utf-8","utf-16"]}}"#));
    p.write_request(&new_id_string("api2"), "transform", None).unwrap();
    assert_eq!(out.take(), frame(r#"{"jsonrpc":"2.0","id":"api2","method":"transform"}"#));
    p.write_request(&new_id_int(-7), "m", Some(&Value::Null)).unwrap();
    assert_eq!(out.take(), frame(r#"{"jsonrpc":"2.0","id":-7,"method":"m","params":null}"#));
    p.write_request(&new_id_string(""), "m", None).unwrap();
    assert_eq!(out.take(), frame(r#"{"jsonrpc":"2.0","id":0,"method":"m"}"#));
    p.write_request(&new_id_int(1), "", None).unwrap();
    assert_eq!(out.take(), frame(r#"{"jsonrpc":"2.0","id":1,"method":""}"#));
    p.write_request(
        &new_id_string("a\"b"),
        "m\u{2028}<>&",
        Some(&value(r#"{"s":"\u0000\u001f\u007f\b\f\n\r\t\\\/é😀 ","n":[0,1.5,123456789012]}"#)),
    )
    .unwrap();
    assert_eq!(
        out.take(),
        frame("{\"jsonrpc\":\"2.0\",\"id\":\"a\\\"b\",\"method\":\"m\u{2028}<>&\",\"params\":{\"s\":\"\\u0000\\u001f\x7f\\b\\f\\n\\r\\t\\\\/é😀\u{2028}\",\"n\":[0,1.5,123456789012]}}")
    );

    p.write_notification("changed", Some(&value(r#"{"a":1}"#))).unwrap();
    assert_eq!(out.take(), frame(r#"{"jsonrpc":"2.0","method":"changed","params":{"a":1}}"#));
    p.write_notification("changed", None).unwrap();
    assert_eq!(out.take(), frame(r#"{"jsonrpc":"2.0","method":"changed"}"#));

    p.write_response(&new_id_string("api1"), &value(r#"{"positionEncoding":"utf-8","diagnosticSource":"vue"}"#)).unwrap();
    assert_eq!(out.take(), frame(r#"{"jsonrpc":"2.0","id":"api1","result":{"positionEncoding":"utf-8","diagnosticSource":"vue"}}"#));
    // Go's nil result.
    p.write_response(&new_id_int(3), &Value::Null).unwrap();
    assert_eq!(out.take(), frame(r#"{"jsonrpc":"2.0","id":3,"result":null}"#));

    let err = ResponseError { code: CODE_INTERNAL_ERROR, message: "boom \"x\"".to_string(), data: None };
    p.write_error(&new_id_int(3), &err).unwrap();
    assert_eq!(out.take(), frame(r#"{"jsonrpc":"2.0","id":3,"error":{"code":-32603,"message":"boom \"x\""}}"#));
    let err = ResponseError { code: 1, message: "m".to_string(), data: Some(value(r#"{"k":[1,"v"]}"#)) };
    p.write_error(&new_id_string("x"), &err).unwrap();
    assert_eq!(out.take(), frame(r#"{"jsonrpc":"2.0","id":"x","error":{"code":1,"message":"m","data":{"k":[1,"v"]}}}"#));
}

// The owned message types marshal like the borrowed-parts functions the protocol uses (the content mapper host logs
// every message it sends with them).
#[test]
fn request_and_response_messages_marshal_like_the_protocol() {
    let request = jsonrpc::RequestMessage { id: Some(new_id_string("api1")), method: "initialize".to_string(), params: Some(value("[1]")) };
    assert_eq!(request.marshal().unwrap(), r#"{"jsonrpc":"2.0","id":"api1","method":"initialize","params":[1]}"#);
    let response = jsonrpc::ResponseMessage { id: Some(new_id_int(2)), result: None, error: None };
    assert_eq!(response.marshal().unwrap(), r#"{"jsonrpc":"2.0","id":2}"#);
    assert_eq!(
        jsonrpc::marshal_response_message(Some(&new_id_int(2)), Some(&Value::Bool(true)), None).unwrap(),
        r#"{"jsonrpc":"2.0","id":2,"result":true}"#
    );
    // A value json cannot encode fails the marshal instead of writing a broken frame.
    assert!(jsonrpc::marshal_request_message(None, "m", Some(&Value::Number(f64::NAN))).is_err());
}

#[test]
fn response_error_string() {
    let err = ResponseError { code: -32603, message: "boom".to_string(), data: None };
    assert_eq!(err.to_string(), "[-32603]: boom");
    let err = ResponseError { code: -32603, message: "boom".to_string(), data: Some(Value::Number(f64::NAN)) };
    assert_eq!(err.to_string(), "[-32603]: boom\n[]");
}

// Decoding: kinds, null and empty ids, unknown and differently cased members ignored, null for absent, and
// re-encoding a received message (the "receive" log line).
#[test]
fn message_unmarshal_and_remarshal() {
    struct Test {
        input: &'static str,
        id: Option<ID>,
        method: &'static str,
        kind: (bool, bool, bool),
        remarshal: &'static str,
    }
    let tests = [
        Test {
            input: r#"{"jsonrpc":"2.0","id":1,"method":"m"}"#,
            id: Some(new_id_int(1)),
            method: "m",
            kind: (true, false, false),
            remarshal: r#"{"jsonrpc":"2.0","id":1,"method":"m"}"#,
        },
        Test {
            input: r#"{"jsonrpc":"2.0","id":"api1","result":{"a" : 1}}"#,
            id: Some(new_id_string("api1")),
            method: "",
            kind: (false, false, true),
            remarshal: r#"{"jsonrpc":"2.0","id":"api1","result":{"a":1}}"#,
        },
        Test {
            input: r#"{"id":1,"method":"m","params":null}"#,
            id: Some(new_id_int(1)),
            method: "m",
            kind: (true, false, false),
            remarshal: r#"{"jsonrpc":"2.0","id":1,"method":"m","params":null}"#,
        },
        Test {
            input: r#"{"method":"n"}"#,
            id: None,
            method: "n",
            kind: (false, true, false),
            remarshal: r#"{"jsonrpc":"2.0","method":"n"}"#,
        },
        Test {
            input: r#"{"id":null,"method":"m"}"#,
            id: None,
            method: "m",
            kind: (false, true, false),
            remarshal: r#"{"jsonrpc":"2.0","method":"m"}"#,
        },
        Test {
            input: r#"{"id":"","method":"m"}"#,
            id: Some(new_id_int(0)),
            method: "m",
            kind: (true, false, false),
            remarshal: r#"{"jsonrpc":"2.0","id":0,"method":"m"}"#,
        },
        Test {
            input: r#"{"id":"x","method":null}"#,
            id: Some(new_id_string("x")),
            method: "",
            kind: (false, false, true),
            remarshal: r#"{"jsonrpc":"2.0","id":"x"}"#,
        },
        Test {
            input: r#"{"id":"x","error":{"code":-1,"message":"m","data":null}}"#,
            id: Some(new_id_string("x")),
            method: "",
            kind: (false, false, true),
            remarshal: r#"{"jsonrpc":"2.0","id":"x","error":{"code":-1,"message":"m"}}"#,
        },
        Test {
            input: r#"{"id":"x","error":{"code":-1,"message":"m","data":[1,"2"]}}"#,
            id: Some(new_id_string("x")),
            method: "",
            kind: (false, false, true),
            remarshal: r#"{"jsonrpc":"2.0","id":"x","error":{"code":-1,"message":"m","data":[1,"2"]}}"#,
        },
        Test {
            input: r#"{"id":"x","result":null,"error":null}"#,
            id: Some(new_id_string("x")),
            method: "",
            kind: (false, false, true),
            remarshal: r#"{"jsonrpc":"2.0","id":"x","result":null}"#,
        },
        Test {
            input: r#"{"id":"x","error":{"code":null,"message":null}}"#,
            id: Some(new_id_string("x")),
            method: "",
            kind: (false, false, true),
            remarshal: r#"{"jsonrpc":"2.0","id":"x","error":{"code":0,"message":""}}"#,
        },
        Test {
            input: r#"{"ID":1,"Method":"m","extra":{"deep":[1,2]}}"#,
            id: None,
            method: "",
            kind: (false, false, false),
            remarshal: r#"{"jsonrpc":"2.0"}"#,
        },
        Test { input: "null", id: None, method: "", kind: (false, false, false), remarshal: r#"{"jsonrpc":"2.0"}"# },
    ];
    for tt in tests {
        let m = Message::unmarshal(tt.input.as_bytes()).unwrap_or_else(|err| panic!("{}: {err}", tt.input));
        assert_eq!(m.id, tt.id, "{}", tt.input);
        assert_eq!(m.method, tt.method, "{}", tt.input);
        assert_eq!((m.is_request(), m.is_notification(), m.is_response()), tt.kind, "{}", tt.input);
        assert_eq!(m.marshal().unwrap(), tt.remarshal, "{}", tt.input);
    }

    // Absent params and result stay distinguishable from null ones.
    let m = Message::unmarshal(br#"{"id":"x"}"#).unwrap();
    assert_eq!((m.params, m.result), (None, None));
    let m = Message::unmarshal(br#"{"id":"x","result":null}"#).unwrap();
    assert_eq!(m.result, Some(Value::Null));
}

// json/v2's error texts for a message whose members have the wrong JSON kind or do not fit their Go type.
#[test]
fn message_unmarshal_errors() {
    let tests = [
        (r#"{"jsonrpc":"1.0","id":1,"method":"m"}"#, r#"json: cannot unmarshal JSON string into Go jsonrpc.JSONRPCVersion within "/jsonrpc": invalid JSON-RPC version"#),
        (r#"{"jsonrpc":null,"id":1,"method":"m"}"#, r#"json: cannot unmarshal JSON null into Go jsonrpc.JSONRPCVersion within "/jsonrpc": invalid JSON-RPC version"#),
        (r#"{"jsonrpc":2,"id":1,"method":"m"}"#, r#"json: cannot unmarshal JSON number into Go jsonrpc.JSONRPCVersion within "/jsonrpc": invalid JSON-RPC version"#),
        (r#"{"id":true}"#, r#"json: cannot unmarshal JSON boolean into Go int32 within "/id""#),
        (r#"{"id":1.5}"#, r#"json: cannot unmarshal JSON number 1.5 into Go int32 within "/id": invalid syntax"#),
        (r#"{"id":-2147483649}"#, r#"json: cannot unmarshal JSON number -2147483649 into Go int32 within "/id": value out of range"#),
        (r#"{"id":2147483648}"#, r#"json: cannot unmarshal JSON number 2147483648 into Go int32 within "/id": value out of range"#),
        (r#"{"id":{}}"#, r#"json: cannot unmarshal JSON object into Go int32 within "/id""#),
        (r#"{"id":"x","method":5}"#, r#"json: cannot unmarshal JSON number into Go string within "/method""#),
        (r#"{"id":"x","error":5}"#, r#"json: cannot unmarshal JSON number into Go jsonrpc.ResponseError within "/error""#),
        (r#"{"id":"x","error":[]}"#, r#"json: cannot unmarshal JSON array into Go jsonrpc.ResponseError within "/error""#),
        (r#"{"id":"x","error":{"code":"a"}}"#, r#"json: cannot unmarshal JSON string into Go int32 within "/error/code""#),
        (r#"{"id":"x","error":{"code":1.5}}"#, r#"json: cannot unmarshal JSON number 1.5 into Go int32 within "/error/code": invalid syntax"#),
        (r#"{"id":"x","error":{"code":3000000000}}"#, r#"json: cannot unmarshal JSON number 3000000000 into Go int32 within "/error/code": value out of range"#),
        (r#"{"id":"x","error":{"message":7}}"#, r#"json: cannot unmarshal JSON number into Go string within "/error/message""#),
        ("[1]", "json: cannot unmarshal JSON array into Go jsonrpc.Message"),
        (r#""s""#, "json: cannot unmarshal JSON string into Go jsonrpc.Message"),
        ("7", "json: cannot unmarshal JSON number into Go jsonrpc.Message"),
        ("true", "json: cannot unmarshal JSON boolean into Go jsonrpc.Message"),
    ];
    for (input, err) in tests {
        assert_eq!(Message::unmarshal(input.as_bytes()).unwrap_err().to_string(), err, "{input}");
    }
    // Syntax errors are reported (with tsrs_core::json's text, not jsontext's).
    for input in [r#"{"a":1,"a":2}"#, "{", r#"{"id":1} x"#, "\u{feff}{}"] {
        assert!(Message::unmarshal(input.as_bytes()).is_err(), "{input}");
    }
    assert!(Message::unmarshal(b"{\"id\":\"\xff\"}").is_err());
}
