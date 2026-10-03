// Callback re-entry against a session-like handler that holds a non-reentrant lock while it calls the
// client (as a snapshot build does while reading files through the callback filesystem). Every
// scenario is bounded: a hang fails the test instead of blocking it.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use tsrs_api_transport::jsonrpc::{FrameReader, FrameWriter};
use tsrs_api_transport::msgpack::{MessagePackReader, MessagePackWriter, MessageType};
use tsrs_api_transport::{
    current_request, lock_for_request, ApiError, AsyncConn, Caller, ConnOptions, Handler, LateCaller, RequestContext, Response,
    SyncConn, TransportError,
};

struct SimSession {
    caller: Arc<dyn Caller>,
    registry: Mutex<u32>,
}

impl Handler for SimSession {
    fn handle_request(&self, _cx: &RequestContext, method: &str, _params: &[u8]) -> Result<Response, ApiError> {
        match method {
            "outer" => {
                let mut g = lock_for_request(&self.registry, "the snapshot registry")?;
                *g += 1;
                let r = self.caller.call("cb", Some(b"\"/f.ts\"")).map_err(|e| ApiError::internal(e.to_string()))?;
                Ok(Response::Json(r))
            }
            "needsLock" => {
                let g = lock_for_request(&self.registry, "the snapshot registry")?;
                Response::json(&*g)
            }
            "slowHold" => {
                let _g = lock_for_request(&self.registry, "the snapshot registry")?;
                thread::sleep(Duration::from_millis(100));
                Response::json("held")
            }
            "free" => Response::json(&current_request().unwrap().depth),
            _ => Err(ApiError::internal("unknown")),
        }
    }
}

fn bounded<T: Send + 'static>(what: &str, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(Duration::from_secs(20)).unwrap_or_else(|_| panic!("{what}: hung"))
}

const REENTRANT: &str = "api: client error: the snapshot registry is in use by a request that is waiting on a client callback; API requests made from inside a callback cannot use it until that callback returns";

#[test]
fn sync_nested_request_gets_a_deliberate_error_not_a_hang() {
    bounded("sync re-entry", || {
        let (server_r, client_w) = std::io::pipe().unwrap();
        let (client_r, server_w) = std::io::pipe().unwrap();
        let late = LateCaller::new();
        let session = Arc::new(SimSession { caller: late.clone(), registry: Mutex::new(0) });
        let conn = SyncConn::new(
            Box::new(MessagePackReader::new(server_r)),
            Box::new(MessagePackWriter::new(server_w)),
            session,
            ConnOptions::default(),
        );
        late.set(conn.caller());
        let run = thread::spawn(move || conn.run());
        let mut w = MessagePackWriter::new(client_w);
        let mut r = MessagePackReader::new(client_r);
        let mut recv = || {
            let t = r.read_tuple().unwrap();
            (t.msg_type, String::from_utf8(t.method).unwrap(), String::from_utf8(t.payload).unwrap())
        };
        w.write_tuple(MessageType::Request, b"outer", b"").unwrap();
        assert_eq!(recv(), (MessageType::Call, "cb".into(), "\"/f.ts\"".into()));
        // Inside the callback: a request that needs the held lock fails fast...
        w.write_tuple(MessageType::Request, b"needsLock", b"").unwrap();
        assert_eq!(recv(), (MessageType::Error, "needsLock".into(), REENTRANT.into()));
        // ...one that does not need it succeeds, at depth 1 (Go behavior).
        w.write_tuple(MessageType::Request, b"free", b"").unwrap();
        assert_eq!(recv(), (MessageType::Response, "free".into(), "1".into()));
        w.write_tuple(MessageType::CallResponse, b"cb", b"{\"kind\":\"missing\"}").unwrap();
        assert_eq!(recv(), (MessageType::Response, "outer".into(), "{\"kind\":\"missing\"}".into()));
        // After the callback returns the lock is free again.
        w.write_tuple(MessageType::Request, b"needsLock", b"").unwrap();
        assert_eq!(recv(), (MessageType::Response, "needsLock".into(), "1".into()));
        drop(w);
        assert!(run.join().unwrap().is_ok());
    });
}

fn start_async() -> (FrameWriter<std::io::PipeWriter>, FrameReader<std::io::PipeReader>, thread::JoinHandle<Result<(), TransportError>>) {
    let (server_r, client_w) = std::io::pipe().unwrap();
    let (client_r, server_w) = std::io::pipe().unwrap();
    let late = LateCaller::new();
    let session = Arc::new(SimSession { caller: late.clone(), registry: Mutex::new(0) });
    let conn = AsyncConn::new(
        Box::new(FrameReader::new(server_r)),
        Box::new(FrameWriter::new(server_w)),
        session,
        ConnOptions::default(),
        None,
    );
    late.set(conn.caller());
    let run = thread::spawn(move || conn.run());
    (FrameWriter::new(client_w), FrameReader::new(client_r), run)
}

fn json(r: &mut FrameReader<std::io::PipeReader>) -> serde_json::Value {
    serde_json::from_slice(&r.read_frame().unwrap()).unwrap()
}

#[test]
fn async_nested_request_gets_a_deliberate_error_not_a_hang() {
    bounded("async re-entry", || {
        let (mut w, mut r, run) = start_async();
        w.write_frame(br#"{"jsonrpc":"2.0","id":1,"method":"outer"}"#).unwrap();
        let call = json(&mut r);
        assert_eq!(call["method"], "cb");
        w.write_frame(br#"{"jsonrpc":"2.0","id":2,"method":"needsLock"}"#).unwrap();
        assert_eq!(json(&mut r), serde_json::json!({"jsonrpc": "2.0", "id": 2, "error": {"code": -32603, "message": REENTRANT}}));
        w.write_frame(br#"{"jsonrpc":"2.0","id":3,"method":"free"}"#).unwrap();
        assert_eq!(json(&mut r)["result"], 0);
        w.write_frame(format!(r#"{{"jsonrpc":"2.0","id":{},"result":"done"}}"#, call["id"]).as_bytes()).unwrap();
        assert_eq!(json(&mut r), serde_json::json!({"jsonrpc": "2.0", "id": 1, "result": "done"}));
        drop(w);
        assert!(run.join().unwrap().is_ok());
    });
}

#[test]
fn async_plain_contention_waits_instead_of_failing() {
    bounded("async contention", || {
        let (mut w, mut r, run) = start_async();
        w.write_frame(br#"{"jsonrpc":"2.0","id":1,"method":"slowHold"}"#).unwrap();
        thread::sleep(Duration::from_millis(20));
        w.write_frame(br#"{"jsonrpc":"2.0","id":2,"method":"needsLock"}"#).unwrap();
        let mut got = vec![json(&mut r), json(&mut r)];
        got.sort_by_key(|v| v["id"].as_i64());
        assert_eq!(got[0]["result"], "held");
        assert_eq!(got[1]["result"], 0, "{got:?}");
        drop(w);
        assert!(run.join().unwrap().is_ok());
    });
}

#[test]
fn closing_while_a_handler_waits_on_a_callback_is_bounded() {
    // Async: the client vanishes (both directions) while `outer` holds the lock and waits for `cb`.
    bounded("async close", || {
        let (mut w, mut r, run) = start_async();
        w.write_frame(br#"{"jsonrpc":"2.0","id":1,"method":"outer"}"#).unwrap();
        assert_eq!(json(&mut r)["method"], "cb");
        drop(w);
        drop(r);
        // Run returns once the waiting handler is released and joined (its error write may fail).
        let _ = run.join().unwrap();
    });
    // Sync: same, EOF while the server is blocked inline in the call.
    bounded("sync close", || {
        let (server_r, client_w) = std::io::pipe().unwrap();
        let (client_r, server_w) = std::io::pipe().unwrap();
        let late = LateCaller::new();
        let session = Arc::new(SimSession { caller: late.clone(), registry: Mutex::new(0) });
        let conn = SyncConn::new(
            Box::new(MessagePackReader::new(server_r)),
            Box::new(MessagePackWriter::new(server_w)),
            session.clone(),
            ConnOptions::default(),
        );
        late.set(conn.caller());
        let run = thread::spawn(move || conn.run());
        let mut w = MessagePackWriter::new(client_w);
        let mut r = MessagePackReader::new(client_r);
        w.write_tuple(MessageType::Request, b"outer", b"").unwrap();
        assert_eq!(r.read_tuple().unwrap().msg_type, MessageType::Call);
        drop(w);
        drop(r);
        let _ = run.join().unwrap();
        // The lock was released on the way out: nothing stays wedged.
        assert!(session.registry.try_lock().is_ok());
    });
}
