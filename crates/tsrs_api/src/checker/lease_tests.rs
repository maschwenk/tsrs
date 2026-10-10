// API checker gate (lease.rs) through real sync (MessagePack) and async (JSON-RPC) transport connections,
// using the runtime's holder-aware per-acquisition contention (`Holder`, `ContentionWait`).
//
// The async grace period is short and configurable: TSRS_CHECKER_TEST_GRACE_MS (default 300). Every
// scenario runs under a hard bound so a hang fails instead of blocking the test run.

use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use tsrs_api_transport as transport;
use tsrs_api_transport::jsonrpc::{FrameReader, FrameWriter};
use tsrs_api_transport::msgpack::{MessagePackReader, MessagePackWriter, MessageType};
use tsrs_core::json::{self, Value};

use super::lease;
use super::session_tests::{get, obj, session_with, S};

const Q_FILES: &[(&str, &str)] = &[("/q/tsconfig.json", r#"{ "files": ["q.ts"] }"#), ("/q/q.ts", "export const q: number = 1;\n")];
const REENTRANT_PREFIX: &str = "api: client error: the program's API checker is in use by a request that is waiting on a client callback";

fn grace() -> Duration {
    Duration::from_millis(std::env::var("TSRS_CHECKER_TEST_GRACE_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(300))
}

/// Forwards to core's Session, plus test methods that hold a program's API checker gate:
/// `holdCb {which}` holds it across a client call "cb"; `hold {which, ms}` holds it while sleeping;
/// `cbOnly` makes a client call without holding anything; `waitOne {which}` acquires once;
/// `twoWaits` acquires p then q (sequentially, releasing in between).
struct H {
    s: S,
    caller: Arc<transport::LateCaller>,
    ctl: Arc<Ctl>,
}

/// In-process synchronization for tests that need ordering guarantees instead of sleeps: handlers report
/// milestones on `events`, and `holdGate` keeps the gate until the test sets `released`.
#[derive(Default)]
struct Ctl {
    events: std::sync::Mutex<Option<mpsc::Sender<&'static str>>>,
    released: std::sync::Mutex<bool>,
    cv: std::sync::Condvar,
}

impl Ctl {
    fn event(&self, e: &'static str) {
        if let Some(tx) = self.events.lock().unwrap().as_ref() {
            let _ = tx.send(e);
        }
    }
    fn subscribe(&self) -> mpsc::Receiver<&'static str> {
        let (tx, rx) = mpsc::channel();
        *self.events.lock().unwrap() = Some(tx);
        rx
    }
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.cv.notify_all();
    }
}

impl H {
    fn program(&self, which: &str) -> Arc<tsrs_compiler::Program> {
        let sd = self.s.session.snapshot_data(self.s.snapshot as u64).unwrap();
        let project = if which == "q" { "/q/tsconfig.json".to_string() } else { self.s.project.clone() };
        sd.get_program(&tsrs_project::ID(project)).unwrap()
    }

    fn acquire(&self, which: &str) -> Result<lease::ApiCheckerLease, transport::ApiError> {
        lease::acquire(&self.program(which)).map_err(|e| transport::ApiError::internal(e.message))
    }
}

fn param(params: &[u8], key: &str) -> Value {
    if params.is_empty() {
        return Value::Null;
    }
    get(&json::unmarshal(std::str::from_utf8(params).unwrap()).unwrap(), key)
}

fn which(params: &[u8]) -> String {
    match param(params, "which") {
        Value::String(w) => w,
        _ => "p".to_string(),
    }
}

impl transport::Handler for H {
    fn handle_request(&self, _cx: &transport::RequestContext, method: &str, params: &[u8]) -> Result<transport::Response, transport::ApiError> {
        let call = |name: &str, arg: &str| -> Result<Vec<u8>, transport::ApiError> {
            transport::Caller::call(&*self.caller, name, Some(arg.as_bytes())).map_err(|e| transport::ApiError::internal(e.to_string()))
        };
        match method {
            "holdCb" => {
                let w = which(params);
                let _lease = self.acquire(&w)?;
                let r = call("cb", &json::marshal_string(&w))?;
                Ok(transport::Response::Json(r))
            }
            "hold" => {
                let _lease = self.acquire(&which(params))?;
                let ms = match param(params, "ms") {
                    Value::Number(n) => n as u64,
                    _ => 0,
                };
                thread::sleep(Duration::from_millis(ms));
                Ok(transport::Response::Json(b"\"held\"".to_vec()))
            }
            "cbOnly" => Ok(transport::Response::Json(call("cbOnly", "null")?)),
            "holdGate" => {
                let _lease = self.acquire(&which(params))?;
                self.ctl.event("holding");
                let mut released = self.ctl.released.lock().unwrap();
                while !*released {
                    released = self.ctl.cv.wait(released).unwrap();
                }
                Ok(transport::Response::Json(b"\"held\"".to_vec()))
            }
            "waitOne" => {
                self.ctl.event("waiting");
                let start = Instant::now();
                drop(self.acquire(&which(params))?);
                Ok(transport::Response::Json(start.elapsed().as_millis().to_string().into_bytes()))
            }
            "twoWaits" => {
                let start = Instant::now();
                drop(self.acquire("p")?);
                let first = start.elapsed().as_millis();
                drop(self.acquire("q")?);
                Ok(transport::Response::Json(format!("[{first},{}]", start.elapsed().as_millis()).into_bytes()))
            }
            _ => match crate::handler::Handler::handle_request(&*self.s.session, method, params) {
                Ok(crate::handler::Response::Json(t)) => Ok(transport::Response::Json(t.into_bytes())),
                Ok(crate::handler::Response::Binary(b)) => Ok(transport::Response::Binary(b)),
                Err(e) => Err(transport::ApiError::internal(e.to_string())),
            },
        }
    }

    fn handle_notification(&self, _cx: &transport::RequestContext, _method: &str, _params: &[u8]) {}
}

fn bounded<T: Send + 'static>(what: &str, limit: Duration, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(limit).unwrap_or_else(|_| panic!("{what}: hung (> {limit:?})"))
}

/// (params for a checker request that needs the /p API checker, params for one that needs none)
fn requests(s: &S) -> (String, String) {
    let needs = json::marshal(&s.at("box:")).unwrap();
    let reference = get(&s.call("getSymbolAtPosition", &s.at("legs")).unwrap(), "reference");
    (needs, json::marshal(&obj(&[("symbol", reference)])).unwrap())
}

struct Async {
    ctl: Arc<Ctl>,
    w: FrameWriter<std::io::PipeWriter>,
    r: FrameReader<std::io::PipeReader>,
    run: thread::JoinHandle<Result<(), transport::TransportError>>,
}

impl Async {
    fn start(s: S, grace: Duration) -> Async {
        let ctl = Arc::new(Ctl::default());
        let late = transport::LateCaller::new();
        let (server_r, client_w) = std::io::pipe().unwrap();
        let (client_r, server_w) = std::io::pipe().unwrap();
        let conn = transport::AsyncConn::new(
            Box::new(FrameReader::new(server_r)),
            Box::new(FrameWriter::new(server_w)),
            Arc::new(H { s, caller: late.clone(), ctl: ctl.clone() }),
            transport::ConnOptions { reentrancy_grace: grace, ..Default::default() },
            None,
        );
        late.set(conn.caller());
        let run = thread::spawn(move || conn.run());
        Async { ctl, w: FrameWriter::new(client_w), r: FrameReader::new(client_r), run }
    }

    fn send(&mut self, id: u32, method: &str, params: &str) {
        let params = if params.is_empty() { String::new() } else { format!(r#","params":{params}"#) };
        self.w.write_frame(format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}"{params}}}"#).as_bytes()).unwrap();
    }

    fn read(&mut self) -> Value {
        json::unmarshal(std::str::from_utf8(&self.r.read_frame().unwrap()).unwrap()).unwrap()
    }

    /// Reads a server -> client call and returns its id (JSON text) and params.
    fn read_call(&mut self, method: &str) -> (String, Value) {
        let v = self.read();
        assert_eq!(get(&v, "method"), Value::String(method.into()), "{}", json::marshal(&v).unwrap());
        (json::marshal(&get(&v, "id")).unwrap(), get(&v, "params"))
    }

    fn answer(&mut self, call_id: &str, result: &str) {
        self.w.write_frame(format!(r#"{{"jsonrpc":"2.0","id":{call_id},"result":{result}}}"#).as_bytes()).unwrap();
    }

    fn finish(self) {
        drop(self.w);
        assert!(self.run.join().unwrap().is_ok());
    }
}

fn result(v: &Value) -> Value {
    assert_eq!(get(v, "error"), Value::Null, "unexpected error: {}", json::marshal(v).unwrap());
    get(v, "result")
}

fn error_message(v: &Value) -> String {
    match get(&get(v, "error"), "message") {
        Value::String(m) => m,
        _ => panic!("expected an error: {}", json::marshal(v).unwrap()),
    }
}

fn ms(v: &Value) -> u128 {
    match v {
        Value::Number(n) => *n as u128,
        other => panic!("{other:?}"),
    }
}

#[test]
fn async_genuine_reentry_fails_after_the_grace_period_and_lease_free_requests_proceed() {
    let g = grace();
    bounded("async genuine re-entry", g * 20 + Duration::from_secs(30), move || {
        let s = session_with(&[], &["/p/tsconfig.json"]);
        let (needs, free) = requests(&s);
        let mut a = Async::start(s, g);
        a.send(1, "holdCb", r#"{"which":"p"}"#);
        let (cb, _) = a.read_call("cb");
        let start = Instant::now();
        a.send(2, "getTypeAtPosition", &needs);
        let resp = a.read();
        let waited = start.elapsed();
        assert!(error_message(&resp).starts_with(REENTRANT_PREFIX), "{}", json::marshal(&resp).unwrap());
        assert!(waited >= g, "rejected after {waited:?}, before the {g:?} grace period");
        a.send(3, "getParentOfSymbol", &free);
        assert_eq!(get(&result(&a.read()), "name"), Value::String("Dog".into()));
        a.answer(&cb, "\"done\"");
        assert_eq!(result(&a.read()), Value::String("done".into()));
        a.send(4, "getTypeAtPosition", &needs);
        assert_eq!(get(&result(&a.read()), "flags"), Value::Number(1048576.0));
        a.finish();
    });
}

#[test]
fn async_ordinary_contention_waits_past_the_grace_period() {
    let g = grace();
    bounded("async contention", g * 20 + Duration::from_secs(30), move || {
        let s = session_with(&[], &["/p/tsconfig.json"]);
        let (needs, _) = requests(&s);
        let mut a = Async::start(s, g);
        let hold = (g * 3).as_millis();
        a.send(1, "hold", &format!(r#"{{"which":"p","ms":{hold}}}"#));
        thread::sleep(g / 6);
        a.send(2, "waitOne", r#"{"which":"p"}"#);
        a.send(3, "getTypeAtPosition", &needs);
        let mut ids = Vec::new();
        let mut waited = 0;
        for _ in 0..3 {
            let v = a.read();
            if get(&v, "id") == Value::Number(2.0) {
                waited = ms(&result(&v));
            } else {
                result(&v);
            }
            ids.push(get(&v, "id"));
        }
        assert_eq!(ids[0], Value::Number(1.0), "the holder finishes first: {ids:?}");
        assert!(waited >= (g * 2).as_millis(), "waitOne waited only {waited} ms behind a {hold} ms holder");
        a.finish();
    });
}

#[test]
fn async_unrelated_attributed_callback_does_not_reject_a_legitimate_wait() {
    // Ordering is established with in-process events, not sleeps: the holder has the gate before the
    // waiter is sent, the unrelated callback is pending before the waiter starts, and the gate stays held
    // for three grace periods after the waiter has started contending. (A previous version slept a fixed
    // g/6 and measured the waiter against a timed holder, so a slow dispatch could shorten the measured wait.)
    let g = grace();
    bounded("async unrelated callback", g * 20 + Duration::from_secs(30), move || {
        let s = session_with(&[], &["/p/tsconfig.json"]);
        let mut a = Async::start(s, g);
        let events = a.ctl.subscribe();
        let next = |what: &str| events.recv_timeout(Duration::from_secs(20)).unwrap_or_else(|_| panic!("no {what} event"));
        a.send(1, "holdGate", r#"{"which":"p"}"#);
        assert_eq!(next("holding"), "holding");
        // An unrelated request blocks on the client (attributed to that request) for the whole wait.
        a.send(2, "cbOnly", "");
        let (cb, _) = a.read_call("cbOnly");
        a.send(3, "waitOne", r#"{"which":"p"}"#);
        assert_eq!(next("waiting"), "waiting");
        // Longer than the grace period: a spurious rejection would have answered request 3 by now.
        thread::sleep(g * 3);
        a.ctl.release();
        let first = a.read();
        assert_eq!(get(&first, "id"), Value::Number(1.0), "the holder answers first: {}", json::marshal(&first).unwrap());
        result(&first);
        let waiter = a.read();
        assert_eq!(get(&waiter, "id"), Value::Number(3.0));
        // Not rejected, and it really waited behind the holder for longer than the grace period.
        assert!(ms(&result(&waiter)) >= (g * 3).as_millis(), "{}", json::marshal(&waiter).unwrap());
        a.answer(&cb, "null");
        assert_eq!(get(&a.read(), "id"), Value::Number(2.0));
        a.finish();
    });
}

#[test]
fn async_two_waits_in_one_request_each_get_the_full_grace_period() {
    let g = grace();
    bounded("async two waits", g * 20 + Duration::from_secs(30), move || {
        let s = session_with(Q_FILES, &["/p/tsconfig.json", "/q/tsconfig.json"]);
        let mut a = Async::start(s, g);
        // Two holders, each possibly stuck on its own client call (attributed to it).
        a.send(1, "holdCb", r#"{"which":"p"}"#);
        let (cb_p, _) = a.read_call("cb");
        a.send(2, "holdCb", r#"{"which":"q"}"#);
        let (cb_q, _) = a.read_call("cb");
        // One request waits on p, then on q; each wait sees a stuck holder for ~0.6 × grace: together
        // longer than the grace period, each alone shorter.
        a.send(3, "twoWaits", "");
        thread::sleep(g * 6 / 10);
        a.answer(&cb_p, "\"p\"");
        assert_eq!(result(&a.read()), Value::String("p".into()));
        thread::sleep(g * 6 / 10);
        a.answer(&cb_q, "\"q\"");
        let mut got = Vec::new();
        for _ in 0..2 {
            let v = a.read();
            got.push((get(&v, "id"), result(&v)));
        }
        let (_, waits) = got.iter().find(|(id, _)| *id == Value::Number(3.0)).expect("twoWaits response").clone();
        let Value::Array(w) = waits else { panic!("{waits:?}") };
        assert!(ms(&w[1]) >= (g * 11 / 10).as_millis(), "the two waits together exceed the grace period: {w:?}");
        a.finish();
    });
}

#[test]
fn async_waiter_exits_when_the_connection_closes() {
    // Grace far beyond the bound: only cancellation can end the wait in time.
    bounded("async cancellation", Duration::from_secs(30), || {
        let s = session_with(&[], &["/p/tsconfig.json"]);
        let mut a = Async::start(s, Duration::from_secs(600));
        a.send(1, "holdCb", r#"{"which":"p"}"#);
        let _ = a.read_call("cb");
        a.send(2, "waitOne", r#"{"which":"p"}"#);
        thread::sleep(Duration::from_millis(50));
        let start = Instant::now();
        let Async { w, r, run, .. } = a;
        drop(w);
        drop(r);
        let _ = run.join().unwrap();
        assert!(start.elapsed() < Duration::from_secs(20), "connection close took {:?}", start.elapsed());
    });
}

#[test]
fn sync_genuine_reentry_fails_immediately_and_lease_free_requests_proceed() {
    // Sync connections reject a provable re-entry at once, whatever the (async) grace setting.
    bounded("sync genuine re-entry", Duration::from_secs(60), || {
        let s = session_with(&[], &["/p/tsconfig.json"]);
        let (needs, free) = requests(&s);
        let late = transport::LateCaller::new();
        let (server_r, client_w) = std::io::pipe().unwrap();
        let (client_r, server_w) = std::io::pipe().unwrap();
        let conn = transport::SyncConn::new(
            Box::new(MessagePackReader::new(server_r)),
            Box::new(MessagePackWriter::new(server_w)),
            Arc::new(H { s, caller: late.clone(), ctl: Arc::new(Ctl::default()) }),
            transport::ConnOptions { reentrancy_grace: Duration::from_secs(600), ..Default::default() },
        );
        late.set(conn.caller());
        let run = thread::spawn(move || conn.run());
        let mut w = MessagePackWriter::new(client_w);
        let mut r = MessagePackReader::new(client_r);
        let mut recv = || {
            let t = r.read_tuple().unwrap();
            (t.msg_type, String::from_utf8(t.method).unwrap(), String::from_utf8(t.payload).unwrap())
        };
        w.write_tuple(MessageType::Request, b"holdCb", br#"{"which":"p"}"#).unwrap();
        assert_eq!(recv().0, MessageType::Call);
        let start = Instant::now();
        w.write_tuple(MessageType::Request, b"getTypeAtPosition", needs.as_bytes()).unwrap();
        let (ty, method, payload) = recv();
        assert_eq!((ty, method.as_str()), (MessageType::Error, "getTypeAtPosition"), "{payload}");
        assert!(payload.starts_with(REENTRANT_PREFIX), "{payload}");
        assert!(start.elapsed() < Duration::from_secs(5), "sync rejection took {:?}", start.elapsed());
        w.write_tuple(MessageType::Request, b"getParentOfSymbol", free.as_bytes()).unwrap();
        let (ty, _, payload) = recv();
        assert_eq!(ty, MessageType::Response, "{payload}");
        assert!(payload.contains("\"name\":\"Dog\""), "{payload}");
        w.write_tuple(MessageType::CallResponse, b"cb", b"\"done\"").unwrap();
        assert_eq!(recv(), (MessageType::Response, "holdCb".into(), "\"done\"".into()));
        w.write_tuple(MessageType::Request, b"getTypeAtPosition", needs.as_bytes()).unwrap();
        assert_eq!(recv().0, MessageType::Response);
        drop(w);
        assert!(run.join().unwrap().is_ok());
    });
}

#[test]
fn gate_entries_are_dropped_once_unused() {
    let s = session_with(&[], &["/p/tsconfig.json"]);
    let sd = s.session.snapshot_data(s.snapshot as u64).unwrap();
    let program = sd.get_program(&tsrs_project::ID(s.project.clone())).unwrap();
    drop(lease::acquire(&program).unwrap());
    assert!(!lease::is_tracked(&program));
}

#[test]
fn sync_nested_request_reads_its_own_exact_uint64_literals() {
    // A request issued from inside a client callback is dispatched while the outer request's literals
    // are still current; it must look up its own exact values.
    bounded("sync nested exact ids", Duration::from_secs(60), || {
        let s = session_with(&[], &["/p/tsconfig.json"]);
        let project = s.project.clone();
        let late = transport::LateCaller::new();
        let (server_r, client_w) = std::io::pipe().unwrap();
        let (client_r, server_w) = std::io::pipe().unwrap();
        let conn = transport::SyncConn::new(
            Box::new(MessagePackReader::new(server_r)),
            Box::new(MessagePackWriter::new(server_w)),
            Arc::new(H { s, caller: late.clone(), ctl: Arc::new(Ctl::default()) }),
            transport::ConnOptions::default(),
        );
        late.set(conn.caller());
        let run = thread::spawn(move || conn.run());
        let mut w = MessagePackWriter::new(client_w);
        let mut r = MessagePackReader::new(client_r);
        let mut recv = || {
            let t = r.read_tuple().unwrap();
            (t.msg_type, String::from_utf8(t.method).unwrap(), String::from_utf8(t.payload).unwrap())
        };
        // Outer request (no checker lease involved) blocked in a client call.
        w.write_tuple(MessageType::Request, b"cbOnly", b"").unwrap();
        assert_eq!(recv().0, MessageType::Call);
        let nested = format!(r#"{{"snapshot":9007199254740993,"project":{},"file":"/p/main.ts","position":0}}"#, json::marshal_string(&project));
        w.write_tuple(MessageType::Request, b"getTypeAtPosition", nested.as_bytes()).unwrap();
        assert_eq!(recv(), (MessageType::Error, "getTypeAtPosition".into(), "api: client error: snapshot 9007199254740993 not found".into()));
        w.write_tuple(MessageType::CallResponse, b"cbOnly", b"null").unwrap();
        assert_eq!(recv().0, MessageType::Response);
        drop(w);
        assert!(run.join().unwrap().is_ok());
    });
}
