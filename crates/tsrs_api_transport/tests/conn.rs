// Connection behavior over real OS pipes: partial frames, bounded frames, callbacks while a request
// is in flight, nested requests, remote errors, EOF and shutdown.

use std::io::{Read, Write};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use tsrs_api_transport::jsonrpc::{decode_message, FrameReader, FrameWriter};
use tsrs_api_transport::msgpack::{MessagePackReader, MessagePackWriter, MessageType};
use tsrs_api_transport::protocol::ProtocolReader;
use tsrs_api_transport::{
    ApiError, AsyncConn, Caller, ConnOptions, Handler, LateCaller, RequestContext, Response, SyncConn, TransportError,
};

/// Delivers at most one byte per read call.
struct Trickle<R>(R);

impl<R: Read> Read for Trickle<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = buf.len().min(1);
        self.0.read(&mut buf[..n])
    }
}

#[test]
fn msgpack_frames_split_across_reads() {
    let mut bytes = Vec::new();
    for (t, m, p) in [(1u8, "a", "x".repeat(300)), (2, "readFile", "{\"kind\":\"missing\"}".to_string()), (1, "\u{1F600}", String::new())] {
        let mut w = MessagePackWriter::new(&mut bytes);
        w.write_tuple(MessageType::from_u8(t).unwrap(), m.as_bytes(), p.as_bytes()).unwrap();
    }
    let mut r = MessagePackReader::new(Trickle(&bytes[..]));
    let a = r.read_message().unwrap();
    assert_eq!((a.method.as_str(), a.params.as_ref().unwrap().len()), ("a", 300));
    let b = r.read_message().unwrap();
    assert!(b.is_response());
    assert_eq!(b.result.as_deref(), Some(&b"{\"kind\":\"missing\"}"[..]));
    let c = r.read_message().unwrap();
    assert_eq!(c.method, "\u{1F600}");
    assert!(r.read_message().unwrap_err().is_eof());
}

#[test]
fn jsonrpc_frames_split_across_reads() {
    let mut bytes = Vec::new();
    {
        let mut w = FrameWriter::new(&mut bytes);
        w.write_frame(br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#).unwrap();
        w.write_frame("{\"jsonrpc\":\"2.0\",\"id\":\"api1\",\"result\":\"\u{1F600}\"}".as_bytes()).unwrap();
    }
    let mut r = FrameReader::new(Trickle(&bytes[..]));
    assert_eq!(r.read_message().unwrap().method, "ping");
    assert_eq!(r.read_message().unwrap().result.unwrap(), "\"\u{1F600}\"".as_bytes());
    assert!(r.read_message().unwrap_err().is_eof());
}

#[test]
fn oversized_frames_are_rejected_without_allocating() {
    // bin32 declaring 4 GiB - 1 with a 1 MiB limit.
    let frame = [0x93, 0x01, 0xc4, 0x01, b'a', 0xc6, 0xff, 0xff, 0xff, 0xff];
    let err = MessagePackReader::with_limit(&frame[..], 1 << 20).read_message().unwrap_err();
    assert!(matches!(err, TransportError::FrameTooLarge { declared: 0xffff_ffff, limit: 1048576 }), "{err:?}");
    let header = b"Content-Length: 1000000000000\r\n\r\n";
    let err = FrameReader::with_limit(&header[..], 1 << 20).read_message().unwrap_err();
    assert!(matches!(err, TransportError::FrameTooLarge { .. }), "{err:?}");
    // A declared length within the limit but never delivered is an explicit truncation error.
    let err = FrameReader::new(&b"Content-Length: 100\r\n\r\n{}"[..]).read_message().unwrap_err();
    assert!(matches!(err, TransportError::UnexpectedEof(_)), "{err:?}");
    let long_header = vec![b'X'; 100_000];
    let err = FrameReader::new(&long_header[..]).read_message().unwrap_err();
    assert!(err.to_string().contains("header line exceeds"), "{err}");
}

#[test]
fn decode_message_kinds() {
    assert!(decode_message(br#"{"jsonrpc":"2.0","id":"x","method":"m"}"#).unwrap().is_request());
    assert!(decode_message(br#"{"jsonrpc":"2.0","method":"m"}"#).unwrap().is_notification());
    assert!(decode_message(br#"{"jsonrpc":"2.0","id":3,"result":null}"#).unwrap().is_response());
    assert!(decode_message(br#"{"id":2147483648,"method":"m"}"#).is_err());
}

/// Test handler: `call` makes a server-to-client call with the request params as the method name.
struct CallingHandler {
    caller: Arc<dyn Caller>,
    log: Mutex<Vec<String>>,
}

impl Handler for CallingHandler {
    fn handle_request(&self, cx: &RequestContext, method: &str, params: &[u8]) -> Result<Response, ApiError> {
        self.log.lock().unwrap().push(format!("{method}@{}", cx.depth));
        match method {
            "call" => {
                let name: String = serde_json::from_slice(params).unwrap();
                match self.caller.call(&name, Some(b"\"/p\"")) {
                    Ok(result) => Ok(Response::Json(result)),
                    Err(e) => Err(ApiError::internal(e.to_string())),
                }
            }
            "depth" => Response::json(&cx.depth),
            "bin" => Ok(Response::Binary(vec![0, 159, 146, 150])),
            _ => Err(ApiError::internal(format!("unknown {method}"))),
        }
    }
}

struct MsgpackClient {
    w: MessagePackWriter<std::io::PipeWriter>,
    r: MessagePackReader<std::io::PipeReader>,
}

impl MsgpackClient {
    fn send(&mut self, t: MessageType, method: &str, payload: &[u8]) {
        self.w.write_tuple(t, method.as_bytes(), payload).unwrap();
    }

    fn recv(&mut self) -> (MessageType, String, Vec<u8>) {
        let t = self.r.read_tuple().unwrap();
        (t.msg_type, String::from_utf8(t.method).unwrap(), t.payload)
    }
}

struct MsgpackClient2 {
    r: MessagePackReader<std::io::PipeReader>,
}

impl MsgpackClient2 {
    fn recv(&mut self) -> (MessageType, String, Vec<u8>) {
        let t = self.r.read_tuple().unwrap();
        (t.msg_type, String::from_utf8(t.method).unwrap(), t.payload)
    }
}

fn start_sync() -> (MsgpackClient, thread::JoinHandle<Result<(), TransportError>>, Arc<CallingHandler>) {
    let (server_r, client_w) = std::io::pipe().unwrap();
    let (client_r, server_w) = std::io::pipe().unwrap();
    let late = LateCaller::new();
    let handler = Arc::new(CallingHandler { caller: late.clone(), log: Mutex::new(Vec::new()) });
    let conn = SyncConn::new(
        Box::new(MessagePackReader::new(server_r)),
        Box::new(MessagePackWriter::new(server_w)),
        handler.clone(),
        ConnOptions::default(),
    );
    late.set(conn.caller());
    let run = thread::spawn(move || conn.run());
    (MsgpackClient { w: MessagePackWriter::new(client_w), r: MessagePackReader::new(client_r) }, run, handler)
}

#[test]
fn sync_callback_nested_request_and_errors() {
    let (mut c, run, handler) = start_sync();
    // Plain request with a binary result.
    c.send(MessageType::Request, "bin", b"");
    assert_eq!(c.recv(), (MessageType::Response, "bin".into(), vec![0, 159, 146, 150]));

    // A request whose handler calls back; the client answers after issuing a nested request.
    c.send(MessageType::Request, "call", b"\"readFile\"");
    assert_eq!(c.recv(), (MessageType::Call, "readFile".into(), b"\"/p\"".to_vec()));
    c.send(MessageType::Request, "depth", b"");
    assert_eq!(c.recv(), (MessageType::Response, "depth".into(), b"1".to_vec()));
    c.send(MessageType::CallResponse, "readFile", br#"{"kind":"value","value":"\ud83d\ude00"}"#);
    assert_eq!(c.recv(), (MessageType::Response, "call".into(), br#"{"kind":"value","value":"\ud83d\ude00"}"#.to_vec()));

    // A failed callback surfaces in the handler as a remote error.
    c.send(MessageType::Request, "call", b"\"stat\"");
    assert_eq!(c.recv().0, MessageType::Call);
    c.send(MessageType::CallError, "stat", "boom \u{1F600}".as_bytes());
    assert_eq!(c.recv(), (MessageType::Error, "call".into(), "ipc: remote error [-32603]: boom \u{1F600}".as_bytes().to_vec()));

    // Handler error.
    c.send(MessageType::Request, "nope", b"");
    assert_eq!(c.recv(), (MessageType::Error, "nope".into(), b"unknown nope".to_vec()));

    // EOF while the server waits for a callback: the call fails, the request gets an error, Run is Ok.
    c.send(MessageType::Request, "call", b"\"realpath\"");
    assert_eq!(c.recv().0, MessageType::Call);
    let MsgpackClient { w, r } = c;
    drop(w);
    let mut c = MsgpackClient2 { r };
    let (t, m, p) = c.recv();
    assert_eq!((t, m.as_str()), (MessageType::Error, "call"));
    assert!(String::from_utf8(p).unwrap().starts_with("ipc: connection closed"));
    assert!(run.join().unwrap().is_ok());
    assert_eq!(*handler.log.lock().unwrap(), ["bin@0", "call@0", "depth@1", "call@0", "nope@0", "call@0"]);
}

#[test]
fn sync_rejects_protocol_violations() {
    let (mut c, run, _) = start_sync();
    // A client must not send Response tuples.
    c.send(MessageType::Response, "x", b"");
    let err = run.join().unwrap().unwrap_err();
    assert_eq!(err.to_string(), "unexpected message type: 4");
}

struct JsonClient {
    w: FrameWriter<std::io::PipeWriter>,
    r: FrameReader<std::io::PipeReader>,
}

impl JsonClient {
    fn send(&mut self, json: &str) {
        self.w.write_frame(json.as_bytes()).unwrap();
    }

    fn recv(&mut self) -> serde_json::Value {
        serde_json::from_slice(&self.r.read_frame().unwrap()).unwrap()
    }
}

struct JsonClient2 {
    r: FrameReader<std::io::PipeReader>,
}

impl JsonClient2 {
    fn recv(&mut self) -> serde_json::Value {
        serde_json::from_slice(&self.r.read_frame().unwrap()).unwrap()
    }
}

fn start_async() -> (JsonClient, thread::JoinHandle<Result<(), TransportError>>, Arc<CallingHandler>) {
    let (server_r, client_w) = std::io::pipe().unwrap();
    let (client_r, server_w) = std::io::pipe().unwrap();
    let late = LateCaller::new();
    let handler = Arc::new(CallingHandler { caller: late.clone(), log: Mutex::new(Vec::new()) });
    let conn = AsyncConn::new(
        Box::new(FrameReader::new(server_r)),
        Box::new(FrameWriter::new(server_w)),
        handler.clone(),
        ConnOptions::default(),
        None,
    );
    late.set(conn.caller());
    let run = thread::spawn(move || conn.run());
    (JsonClient { w: FrameWriter::new(client_w), r: FrameReader::new(client_r) }, run, handler)
}

#[test]
fn async_requests_progress_while_one_waits_on_a_callback() {
    let (mut c, run, _) = start_async();
    c.send(r#"{"jsonrpc":"2.0","id":1,"method":"call","params":"readFile"}"#);
    let call = c.recv();
    assert_eq!(call["method"], "readFile");
    assert_eq!(call["id"], "api1");
    assert_eq!(call["params"], "/p");
    // While request 1 is blocked on the client, request 2 completes.
    c.send(r#"{"jsonrpc":"2.0","id":"two","method":"bin"}"#);
    assert_eq!(c.recv(), serde_json::json!({"jsonrpc": "2.0", "id": "two", "result": "AJ+Slg=="}));
    c.send(r#"{"jsonrpc":"2.0","id":"api1","result":{"kind":"value","value":"x"}}"#);
    assert_eq!(c.recv(), serde_json::json!({"jsonrpc": "2.0", "id": 1, "result": {"kind": "value", "value": "x"}}));
    // Remote error.
    c.send(r#"{"jsonrpc":"2.0","id":3,"method":"call","params":"stat"}"#);
    let call = c.recv();
    c.send(&format!(r#"{{"jsonrpc":"2.0","id":{},"error":{{"code":-32601,"message":"no handler"}}}}"#, call["id"]));
    assert_eq!(c.recv()["error"], serde_json::json!({"code": -32603, "message": "ipc: remote error [-32601]: no handler"}));
    // A response for an unknown id is ignored; notifications are accepted.
    c.send(r#"{"jsonrpc":"2.0","id":"api999","result":1}"#);
    c.send(r#"{"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":3}}"#);
    // EOF with a call pending: the waiting handler is released and Run joins it.
    c.send(r#"{"jsonrpc":"2.0","id":4,"method":"call","params":"realpath"}"#);
    assert_eq!(c.recv()["method"], "realpath");
    let JsonClient { w, r } = c;
    drop(w);
    let mut c = JsonClient2 { r };
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || tx.send(run.join().unwrap()).unwrap());
    let result = rx.recv_timeout(Duration::from_secs(10)).expect("Run must return after EOF");
    assert!(result.is_ok(), "{result:?}");
    let last = c.recv();
    assert_eq!(last["id"], 4);
    assert!(last["error"]["message"].as_str().unwrap().starts_with("ipc: connection closed"));
}

#[test]
fn async_protocol_error_is_fatal_and_reported() {
    let (mut c, run, _) = start_async();
    c.send(r#"{"jsonrpc":"1.0","id":1,"method":"bin"}"#);
    let err = run.join().unwrap().unwrap_err();
    assert_eq!(err.to_string(), "invalid JSON-RPC version");
}

#[test]
fn caller_after_connection_dropped_is_closed() {
    let late = LateCaller::new();
    assert!(matches!(late.call("readFile", None), Err(TransportError::Closed(_))));
    let (r, _w) = std::io::pipe().unwrap();
    let (_r2, w) = std::io::pipe().unwrap();
    let handler = Arc::new(CallingHandler { caller: late.clone(), log: Mutex::new(Vec::new()) });
    let conn = SyncConn::new(Box::new(MessagePackReader::new(r)), Box::new(MessagePackWriter::new(w)), handler, ConnOptions::default());
    let caller = conn.caller();
    drop(conn);
    assert!(matches!(caller.call("readFile", None), Err(TransportError::Closed(None))));
}

#[test]
fn stdout_frames_are_flushed_per_message() {
    // Every tuple/frame is flushed as a unit, so a client blocked in a synchronous read never waits
    // on a partially buffered response.
    let (mut r, w) = std::io::pipe().unwrap();
    let mut writer = MessagePackWriter::new(w);
    writer.write_tuple(MessageType::Response, b"ping", b"\"pong\"").unwrap();
    let mut buf = [0u8; 16];
    let n = r.read(&mut buf).unwrap();
    assert_eq!(&buf[..n], b"\x93\x04\xc4\x04ping\xc4\x06\"pong\"");
    let _ = std::io::stdout().flush();
}
