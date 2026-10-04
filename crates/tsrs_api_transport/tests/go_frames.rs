// Byte-exact comparison against frames produced by the pinned Go implementation
// (tsc/internal/api/protocol_msgpack.go and tsc/internal/ipc/protocol_jsonrpc.go at the commit in
// the workspace Cargo.toml). Regenerate with tests/fixtures/go_frames_gen.go.txt (see README.md).

use serde_json::Value;
use tsrs_api_transport::jsonrpc::{FrameReader, FrameWriter};
use tsrs_api_transport::msgpack::{MessagePackReader, MessagePackWriter};
use tsrs_api_transport::protocol::{ProtocolReader, ProtocolWriter};
use tsrs_api_transport::{Id, Message, Response, ResponseError, TransportError};

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/go_frames.json")).unwrap()
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}

const ASTRAL: &str = "/proj/src/\u{1F600}-\u{e9}-\u{1D7D8}.ts";

fn json_str(s: &str) -> Vec<u8> {
    serde_json::to_vec(s).unwrap()
}

fn produce(name: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let big = vec![b'x'; 300];
    let huge = vec![b'y'; 70000];
    let internal = |m: &str| ResponseError::internal(m);
    if let Some(case) = name.strip_prefix("msgpack/") {
        let mut w = MessagePackWriter::new(&mut out);
        match case {
            "call_readFile" => w.write_request(&Id::string("readFile"), "readFile", Some(&json_str(ASTRAL))),
            "call_nil" => w.write_request(&Id::string("x"), "x", None),
            "call_writeFile" => {
                #[derive(serde::Serialize)]
                struct P<'a> {
                    path: &'a str,
                    data: &'a str,
                }
                let payload = P { path: ASTRAL, data: "a<b>&\"\\\n\u{2028}" };
                w.write_request(&Id::string("writeFile"), "writeFile", Some(&serde_json::to_vec(&payload).unwrap()))
            }
            "response_json" => w.write_response(&Id::string("ping"), &Response::Json(b"\"pong\"".to_vec())),
            "response_nil" => w.write_response(&Id::string("release"), &Response::null()),
            "response_binary_astral" => w.write_response(&Id::string("echo"), &Response::Binary(ASTRAL.as_bytes().to_vec())),
            "response_binary_empty" => w.write_response(&Id::string("getSourceFile"), &Response::Binary(Vec::new())),
            "response_bin16" => w.write_response(&Id::string("echo"), &Response::Binary(big)),
            "response_bin32" => w.write_response(&Id::string("echo"), &Response::Binary(huge)),
            "error" => w.write_error(&Id::string("getSymbol"), &internal("api: client error: symbol \u{1F600} not found")),
            "error_empty_id" => w.write_error(&Id::string(""), &internal("x")),
            other => panic!("unmapped case {other}"),
        }
        .unwrap();
        drop(w);
    } else {
        let case = name.strip_prefix("jsonrpc/").unwrap();
        let mut w = FrameWriter::new(&mut out);
        match case {
            "request_readFile" => w.write_request(&Id::string("api1"), "readFile", Some(&json_str(ASTRAL))),
            "request_nil_params" => w.write_request(&Id::string("api2"), "x", None),
            "notification" => w.write_notification("note", Some(b"{\"a\":1}")),
            "response_int_id" => w.write_response(&Id::Int(7), &Response::Json(b"{\"b\":[1,2]}".to_vec())),
            "response_str_id" => w.write_response(&Id::string("q"), &Response::Json(json_str(ASTRAL))),
            "response_nil" => w.write_response(&Id::Int(3), &Response::null()),
            "response_binary" => w.write_response(&Id::Int(4), &Response::Binary(ASTRAL.as_bytes().to_vec())),
            "error" => w.write_error(&Id::Int(9), &internal("boom \u{1F600}")),
            other => panic!("unmapped case {other}"),
        }
        .unwrap();
        drop(w);
    }
    out
}

#[test]
fn writers_match_go_bytes() {
    let f = fixture();
    let writes = f["writes"].as_array().unwrap();
    assert_eq!(writes.len(), 19);
    for case in writes {
        let name = case["name"].as_str().unwrap();
        let expected = hex(case["hex"].as_str().unwrap());
        let actual = produce(name);
        assert!(actual == expected, "{name}: rust={:?}\n go={:?}", String::from_utf8_lossy(&actual), String::from_utf8_lossy(&expected));
    }
}

fn message_json(m: &Message) -> Value {
    let kind = if m.is_request() {
        "request"
    } else if m.is_notification() {
        "notification"
    } else if m.is_response() {
        "response"
    } else {
        ""
    };
    serde_json::json!({
        "id": m.id.as_ref().map(|id| id.to_json()),
        "method": m.method,
        "params": m.params.as_ref().map(|p| to_hex(p)),
        "result": m.result.as_ref().map(|r| to_hex(r)),
        "error": m.error.as_ref().map(|e| serde_json::json!({"code": e.code, "message": e.message})),
        "kind": kind,
    })
}

fn read_all(reader: &mut dyn ProtocolReader) -> (Vec<Value>, Result<(), TransportError>) {
    let mut msgs = Vec::new();
    loop {
        match reader.read_message() {
            Ok(m) => msgs.push(message_json(&m)),
            Err(TransportError::Eof) => return (msgs, Ok(())),
            Err(e) => return (msgs, Err(e)),
        }
    }
}

/// Go drops ResponseError.Data only on our side; strip it from the Go dump for comparison.
fn normalize_go(mut m: Value) -> Value {
    if let Some(err) = m.get_mut("error").and_then(|e| e.as_object_mut()) {
        err.remove("data");
    }
    m
}

#[test]
fn readers_match_go_interpretation() {
    let f = fixture();
    let reads = f["reads"].as_array().unwrap();
    assert_eq!(reads.len(), 21);
    for case in reads {
        let name = case["name"].as_str().unwrap();
        let input = hex(case["input"].as_str().unwrap());
        let (msgs, result) = if name.starts_with("msgpack/") {
            read_all(&mut MessagePackReader::new(&input[..]))
        } else {
            read_all(&mut FrameReader::new(&input[..]))
        };
        let go_msgs: Vec<Value> = case["messages"].as_array().cloned().unwrap_or_default().into_iter().map(normalize_go).collect();
        assert_eq!(msgs, go_msgs, "{name}: messages");
        let go_eof = case["eof"].as_bool().unwrap();
        let go_err = case["error"].as_str().unwrap();
        match name {
            // Deliberate divergences: Go reports EOF/ErrUnexpectedEOF inside a tuple or JSON body with
            // io error text (and a clean io.EOF for a tuple truncated between fields). We always report
            // a truncated frame as an explicit UnexpectedEof error.
            "msgpack/truncated_payload" | "msgpack/eof_in_header" | "jsonrpc/truncated_body" => {
                assert!(matches!(result, Err(TransportError::UnexpectedEof(_))), "{name}: {result:?}");
            }
            // JSON syntax errors: both reject; the error text comes from different JSON libraries.
            "jsonrpc/invalid_json" | "jsonrpc/float_id" | "jsonrpc/duplicate_key" | "jsonrpc/bad_version" => {
                assert!(!go_err.is_empty());
                assert!(matches!(result, Err(TransportError::Protocol(_))), "{name}: {result:?}");
            }
            _ if go_eof => assert!(result.is_ok(), "{name}: expected clean EOF, got {result:?}"),
            _ => {
                let err = result.expect_err(name);
                assert_eq!(err.to_string(), go_err, "{name}: error text");
            }
        }
    }
}
