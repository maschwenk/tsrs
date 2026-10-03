// Callback re-entry against a session-like handler that holds a non-reentrant lock while it calls the
// client (as a snapshot build does while reading files through the callback filesystem). Every
// scenario is bounded: a hang fails the test instead of blocking it.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use tsrs_api_transport::jsonrpc::{FrameReader, FrameWriter};
use tsrs_api_transport::msgpack::{MessagePackReader, MessagePackWriter, MessageType};
use std::time::Instant;

use tsrs_api_transport::{
    current_request, lock_for_request, ApiError, AsyncConn, Caller, ConnOptions, Handler, LateCaller, RequestContext, Response,
    SyncConn, TransportError,
};

struct SimSession {
    caller: Arc<dyn Caller>,
    registry: Mutex<u32>,
}

impl Handler for SimSession {
    fn handle_request(&self, _cx: &RequestContext, method: &str, params: &[u8]) -> Result<Response, ApiError> {
        let ms = || serde_json::from_slice::<u64>(params).unwrap_or(100);
        match method {
            // Holds the lock for `params` ms (no client call).
            "holdMs" => {
                let _g = lock_for_request(&self.registry, "the snapshot registry")?;
                thread::sleep(Duration::from_millis(ms()));
                Response::json("held")
            }
            // Waits on the client from a thread that serves no request (like a compiler worker reading a
            // file during a program build): an unattributed call.
            "callbackFromWorker" => {
                let caller = self.caller.clone();
                let r = thread::spawn(move || caller.call("cb", Some(b"\"/worker.ts\""))).join().unwrap();
                Ok(Response::Json(r.map_err(|e| ApiError::internal(e.to_string()))?))
            }
            // Holds the lock while a worker thread waits on the client (genuine re-entry setup).
            "outerWorker" => {
                let _g = lock_for_request(&self.registry, "the snapshot registry")?;
                let caller = self.caller.clone();
                let r = thread::spawn(move || caller.call("cb", Some(b"\"/held.ts\""))).join().unwrap();
                Ok(Response::Json(r.map_err(|e| ApiError::internal(e.to_string()))?))
            }
            // Two sequential contended acquisitions of a checker-like gate within one request: the first
            // waits `params` ms and succeeds, the second faces a holder that keeps the gate. Reports how
            // long the second wait lasted before it was rejected.
            "gateTwice" | "gateTwiceLegacy" => {
                let legacy = method == "gateTwiceLegacy";
                let first_hold = ms();
                let wait_on = |hold_ms: u64| -> Result<Duration, String> {
                    let gate = Arc::new(Mutex::new(()));
                    let (held_tx, held_rx) = mpsc::channel();
                    let g2 = gate.clone();
                    let holder = thread::spawn(move || {
                        let _h = g2.lock().unwrap();
                        held_tx.send(()).unwrap();
                        thread::sleep(Duration::from_millis(hold_ms));
                    });
                    held_rx.recv().unwrap();
                    let start = std::time::Instant::now();
                    let mut wait = tsrs_api_transport::ContentionWait::new(None);
                    let result = loop {
                        if gate.try_lock().is_ok() {
                            break Ok(start.elapsed());
                        }
                        let stuck = if legacy { tsrs_api_transport::blocking_may_deadlock() } else { wait.may_deadlock() };
                        if stuck {
                            break Err(format!("{}", start.elapsed().as_millis()));
                        }
                        thread::sleep(Duration::from_millis(2));
                    };
                    holder.join().unwrap();
                    result
                };
                let first = wait_on(first_hold);
                let second = wait_on(1000);
                Response::json(&serde_json::json!({
                    "first": first.as_ref().map(|d| d.as_millis() as u64).map_err(|e| e.clone()),
                    "secondRejectedAfterMs": second.err().map(|e| e.parse::<u64>().unwrap()),
                }))
            }
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
            // Waits on the client without holding the lock (an unrelated slow callback).
            "callbackNoLock" => {
                let r = self.caller.call("cb", Some(b"\"/slow.ts\"")).map_err(|e| ApiError::internal(e.to_string()))?;
                Ok(Response::Json(r))
            }
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
    start_async_with(ConnOptions::default())
}

fn start_async_with(
    options: ConnOptions,
) -> (FrameWriter<std::io::PipeWriter>, FrameReader<std::io::PipeReader>, thread::JoinHandle<Result<(), TransportError>>) {
    let (server_r, client_w) = std::io::pipe().unwrap();
    let (client_r, server_w) = std::io::pipe().unwrap();
    let late = LateCaller::new();
    let session = Arc::new(SimSession { caller: late.clone(), registry: Mutex::new(0) });
    let conn = AsyncConn::new(
        Box::new(FrameReader::new(server_r)),
        Box::new(FrameWriter::new(server_w)),
        session,
        options,
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
fn async_nested_request_gets_a_deliberate_error_after_the_grace_period_not_a_hang() {
    bounded("async re-entry", || {
        let grace = Duration::from_millis(300);
        let (mut w, mut r, run) = start_async_with(ConnOptions { reentrancy_grace: grace, ..ConnOptions::default() });
        w.write_frame(br#"{"jsonrpc":"2.0","id":1,"method":"outer"}"#).unwrap();
        let call = json(&mut r);
        assert_eq!(call["method"], "cb");
        let start = std::time::Instant::now();
        w.write_frame(br#"{"jsonrpc":"2.0","id":2,"method":"needsLock"}"#).unwrap();
        assert_eq!(json(&mut r), serde_json::json!({"jsonrpc": "2.0", "id": 2, "error": {"code": -32603, "message": REENTRANT}}));
        assert!(start.elapsed() >= grace, "async conflicts wait out the grace period: {:?}", start.elapsed());
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

#[test]
fn async_unrelated_slow_callback_does_not_reject_ordinary_contention() {
    // Request 1 waits on a slow client callback without holding anything; request 2 holds the lock for a
    // while; request 3 contends for it. Nobody re-enters: request 3 must wait for 2 and succeed (pinned
    // Go behavior), not fail because *some* request is waiting on the client.
    bounded("async unrelated callback", || {
        let (mut w, mut r, run) = start_async();
        w.write_frame(br#"{"jsonrpc":"2.0","id":1,"method":"callbackNoLock"}"#).unwrap();
        let call = json(&mut r);
        assert_eq!(call["params"], "/slow.ts");
        w.write_frame(br#"{"jsonrpc":"2.0","id":2,"method":"slowHold"}"#).unwrap();
        thread::sleep(Duration::from_millis(20));
        w.write_frame(br#"{"jsonrpc":"2.0","id":3,"method":"needsLock"}"#).unwrap();
        let mut got = vec![json(&mut r), json(&mut r)];
        got.sort_by_key(|v| v["id"].as_i64());
        assert_eq!(got[0]["result"], "held", "{got:?}");
        assert_eq!(got[1]["result"], 0, "{got:?}");
        w.write_frame(format!(r#"{{"jsonrpc":"2.0","id":{},"result":"late"}}"#, call["id"]).as_bytes()).unwrap();
        assert_eq!(json(&mut r)["result"], "late");
        drop(w);
        assert!(run.join().unwrap().is_ok());
    });
}

fn grace_opts() -> ConnOptions {
    ConnOptions { reentrancy_grace: Duration::from_millis(100), ..ConnOptions::default() }
}

fn error_message(v: &serde_json::Value) -> Option<String> {
    v["error"]["message"].as_str().map(str::to_string)
}

#[test]
fn async_unrelated_attributed_callback_longer_than_grace_does_not_reject_a_waiter() {
    // Callback owner (1) is unrelated to the lock holder (2) and to the waiter (3); the callback and the
    // hold both outlast the 100 ms grace. Pinned Go: 3 waits for 2 and succeeds. Same here: the holder
    // is known and is not waiting on the client.
    bounded("attributed unrelated callback", || {
        let (mut w, mut r, run) = start_async_with(grace_opts());
        w.write_frame(br#"{"jsonrpc":"2.0","id":1,"method":"callbackNoLock"}"#).unwrap();
        let call = json(&mut r);
        w.write_frame(br#"{"jsonrpc":"2.0","id":2,"method":"holdMs","params":400}"#).unwrap();
        thread::sleep(Duration::from_millis(20));
        let start = Instant::now();
        w.write_frame(br#"{"jsonrpc":"2.0","id":3,"method":"needsLock"}"#).unwrap();
        let mut got = vec![json(&mut r), json(&mut r)];
        got.sort_by_key(|v| v["id"].as_i64());
        assert_eq!(got[0]["result"], "held", "{got:?}");
        assert_eq!(got[1]["result"], 0, "waiter must not be rejected: {got:?}");
        assert!(start.elapsed() >= Duration::from_millis(300), "waited for the holder");
        w.write_frame(format!(r#"{{"jsonrpc":"2.0","id":{},"result":"late"}}"#, call["id"]).as_bytes()).unwrap();
        assert_eq!(json(&mut r)["result"], "late");
        drop(w);
        assert!(run.join().unwrap().is_ok());
    });
}

#[test]
fn async_unattributed_callback_longer_than_grace_rejects_after_grace_documented_divergence() {
    // Same, but the unrelated callback is made from a worker thread (as program-build file reads are):
    // the transport cannot tell it is not the holder's, so the waiter is rejected after the grace period.
    // Pinned Go would wait. Bounded divergence, reported in INTEGRATION.md.
    bounded("unattributed unrelated callback", || {
        let (mut w, mut r, run) = start_async_with(grace_opts());
        w.write_frame(br#"{"jsonrpc":"2.0","id":1,"method":"callbackFromWorker"}"#).unwrap();
        let call = json(&mut r);
        w.write_frame(br#"{"jsonrpc":"2.0","id":2,"method":"holdMs","params":400}"#).unwrap();
        thread::sleep(Duration::from_millis(20));
        let start = Instant::now();
        w.write_frame(br#"{"jsonrpc":"2.0","id":3,"method":"needsLock"}"#).unwrap();
        let first = json(&mut r);
        let elapsed = start.elapsed();
        assert_eq!(first["id"], 3, "{first}");
        assert_eq!(error_message(&first).as_deref(), Some(REENTRANT));
        assert!(elapsed >= Duration::from_millis(100) && elapsed < Duration::from_millis(380), "{elapsed:?}");
        assert_eq!(json(&mut r)["result"], "held");
        w.write_frame(format!(r#"{{"jsonrpc":"2.0","id":{},"result":"late"}}"#, call["id"]).as_bytes()).unwrap();
        assert_eq!(json(&mut r)["result"], "late");
        drop(w);
        assert!(run.join().unwrap().is_ok());
    });
}

#[test]
fn genuine_reentry_through_a_worker_callback_still_fails_boundedly() {
    // The holder waits on the client via a worker thread and the client's callback issues a request
    // for the held lock: sync fails immediately, async after the grace period; neither hangs.
    bounded("async worker re-entry", || {
        let (mut w, mut r, run) = start_async_with(grace_opts());
        w.write_frame(br#"{"jsonrpc":"2.0","id":1,"method":"outerWorker"}"#).unwrap();
        let call = json(&mut r);
        assert_eq!(call["params"], "/held.ts");
        let start = Instant::now();
        w.write_frame(br#"{"jsonrpc":"2.0","id":2,"method":"needsLock"}"#).unwrap();
        let v = json(&mut r);
        assert_eq!(error_message(&v).as_deref(), Some(REENTRANT));
        assert!(start.elapsed() >= Duration::from_millis(100));
        w.write_frame(format!(r#"{{"jsonrpc":"2.0","id":{},"result":"ok"}}"#, call["id"]).as_bytes()).unwrap();
        assert_eq!(json(&mut r)["result"], "ok");
        drop(w);
        assert!(run.join().unwrap().is_ok());
    });
    bounded("sync worker re-entry", || {
        let (server_r, client_w) = std::io::pipe().unwrap();
        let (client_r, server_w) = std::io::pipe().unwrap();
        let late = LateCaller::new();
        let session = Arc::new(SimSession { caller: late.clone(), registry: Mutex::new(0) });
        let conn = SyncConn::new(Box::new(MessagePackReader::new(server_r)), Box::new(MessagePackWriter::new(server_w)), session, grace_opts());
        late.set(conn.caller());
        let run = thread::spawn(move || conn.run());
        let mut w = MessagePackWriter::new(client_w);
        let mut r = MessagePackReader::new(client_r);
        w.write_tuple(MessageType::Request, b"outerWorker", b"").unwrap();
        assert_eq!(r.read_tuple().unwrap().msg_type, MessageType::Call);
        let start = Instant::now();
        w.write_tuple(MessageType::Request, b"needsLock", b"").unwrap();
        let t = r.read_tuple().unwrap();
        assert_eq!((t.msg_type, String::from_utf8(t.payload).unwrap()), (MessageType::Error, REENTRANT.to_string()));
        assert!(start.elapsed() < Duration::from_millis(100), "sync re-entry is reported immediately");
        w.write_tuple(MessageType::CallResponse, b"cb", b"\"ok\"").unwrap();
        assert_eq!(r.read_tuple().unwrap().msg_type, MessageType::Response);
        drop(w);
        assert!(run.join().unwrap().is_ok());
    });
}

#[test]
fn sequential_acquisitions_do_not_inherit_a_previous_wait() {
    // While an unattributed callback is pending (so every holder is possibly stuck), one request waits
    // 80 ms for a gate and gets it, then immediately waits for another gate held for 1 s. With per-
    // acquisition state the second wait gets its full 100 ms grace; the legacy thread-local predicate
    // inherits the first wait's start and rejects the second one ~20 ms in.
    bounded("sequential acquisitions", || {
        let (mut w, mut r, run) = start_async_with(grace_opts());
        w.write_frame(br#"{"jsonrpc":"2.0","id":1,"method":"callbackFromWorker"}"#).unwrap();
        let call = json(&mut r);
        w.write_frame(br#"{"jsonrpc":"2.0","id":2,"method":"gateTwice","params":80}"#).unwrap();
        let v = json(&mut r);
        assert!(v["result"]["first"]["Ok"].as_u64().is_some(), "{v}");
        let second = v["result"]["secondRejectedAfterMs"].as_u64().unwrap();
        assert!(second >= 100, "ContentionWait: second acquisition rejected after {second} ms (< grace)");
        w.write_frame(br#"{"jsonrpc":"2.0","id":3,"method":"gateTwiceLegacy","params":80}"#).unwrap();
        let v = json(&mut r);
        let legacy = v["result"]["secondRejectedAfterMs"].as_u64().unwrap();
        assert!(legacy < 100, "legacy blocking_may_deadlock inherits the previous wait: {legacy} ms");
        w.write_frame(format!(r#"{{"jsonrpc":"2.0","id":{},"result":"late"}}"#, call["id"]).as_bytes()).unwrap();
        assert_eq!(json(&mut r)["result"], "late");
        drop(w);
        assert!(run.join().unwrap().is_ok());
    });
}
