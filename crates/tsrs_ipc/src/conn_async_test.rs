// Port of ipc/conn_async_test.go, then the connection's contract as the content mapper host relies on it, over a
// real socket pair with an in-process peer standing in for the mapper process: concurrent calls, requests from the
// peer, handler errors and panics, EOF and close while calls wait, and the initialize timeout. Expected texts are
// the Go implementation's (checked against it at the pinned commit).

use std::collections::VecDeque;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tsrs_core::json::{self, Value};

use crate::error::eof;
use crate::jsonrpc::{self, new_id_int, new_id_string, ResponseError, ID};
use crate::{
    new_async_conn, new_async_conn_with_protocol, new_jsonrpc_protocol, pipe, AsyncConn, Closer, Conn, Error, ErrorTag,
    Handler, Message, Protocol, ReadWriteCloser, ERR_CONN_CLOSED, METHOD_GET_SERVER_TIMING, METHOD_RESET_SERVER_TIMING,
};

// How long a test waits for something that should happen promptly before it fails instead of hanging.
const PATIENCE: Duration = Duration::from_secs(10);

// Go's close(ch) on a channel goroutines wait on: once open, every wait returns.
#[derive(Default)]
struct gate {
    open: Mutex<bool>,
    cond: Condvar,
}

impl gate {
    fn open(&self) {
        *self.open.lock().unwrap() = true;
        self.cond.notify_all();
    }

    fn wait(&self) {
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.cond.wait(open).unwrap();
        }
    }
}

fn request(id: ID, method: &str) -> Message {
    Message { id: Some(id), method: method.to_string(), ..Message::default() }
}

fn notification(method: &str) -> Message {
    Message { method: method.to_string(), ..Message::default() }
}

fn value(text: &str) -> Value {
    json::unmarshal(text).unwrap()
}

// conn_async_test.go:19
struct noOpHandler;

impl Handler for noOpHandler {
    fn handle_request(&self, _: &str, _: Option<&Value>) -> Result<Value, Error> {
        Ok(Value::Null)
    }

    fn handle_notification(&self, _: &str, _: Option<&Value>) -> Result<(), Error> {
        Ok(())
    }
}

// conn_async_test.go:29
struct queuedProtocol {
    messages: Mutex<VecDeque<Message>>,
    response_err: Option<Error>,
}

impl queuedProtocol {
    fn new(messages: Vec<Message>, response_err: Option<Error>) -> queuedProtocol {
        queuedProtocol { messages: Mutex::new(messages.into()), response_err }
    }
}

impl Protocol for queuedProtocol {
    fn read_message(&self) -> Result<Message, Error> {
        self.messages.lock().unwrap().pop_front().ok_or_else(eof)
    }

    fn write_request(&self, _: &ID, _: &str, _: Option<&Value>) -> Result<(), Error> {
        Ok(())
    }

    fn write_notification(&self, _: &str, _: Option<&Value>) -> Result<(), Error> {
        Ok(())
    }

    fn write_response(&self, _: &ID, _: &Value) -> Result<(), Error> {
        self.response_err.clone().map_or(Ok(()), Err)
    }

    fn write_error(&self, _: &ID, _: &ResponseError) -> Result<(), Error> {
        self.response_err.clone().map_or(Ok(()), Err)
    }
}

// conn_async_test.go:59
struct blockingHandler {
    started: mpsc::Sender<()>,
    release: gate,
}

impl blockingHandler {
    fn new() -> (Arc<blockingHandler>, mpsc::Receiver<()>) {
        let (started, started_rx) = mpsc::channel();
        (Arc::new(blockingHandler { started, release: gate::default() }), started_rx)
    }
}

impl Handler for blockingHandler {
    fn handle_request(&self, _: &str, _: Option<&Value>) -> Result<Value, Error> {
        self.started.send(()).unwrap();
        self.release.wait();
        Ok(Value::Null)
    }

    fn handle_notification(&self, _: &str, _: Option<&Value>) -> Result<(), Error> {
        self.started.send(()).unwrap();
        self.release.wait();
        Ok(())
    }
}

fn run_in_background(conn: &Arc<AsyncConn>) -> mpsc::Receiver<Result<(), Error>> {
    let (done, run_done) = mpsc::channel();
    let conn = Arc::clone(conn);
    thread::spawn(move || {
        let _ = done.send(conn.run());
    });
    run_done
}

fn run_bounded(conn: &Arc<AsyncConn>) -> Result<(), Error> {
    run_in_background(conn).recv_timeout(PATIENCE).expect("Run did not return")
}

// `conn.call` on another thread, so that a test fails instead of hanging when no response comes.
fn spawn_call(conn: &Arc<AsyncConn>, method: &str, params: Option<Value>) -> mpsc::Receiver<Result<Option<Value>, Error>> {
    let (done, result) = mpsc::channel();
    let conn = Arc::clone(conn);
    let method = method.to_string();
    thread::spawn(move || {
        let _ = done.send(conn.call(&method, params.as_ref()));
    });
    result
}

fn call(conn: &Arc<AsyncConn>, method: &str, params: Option<Value>) -> Result<Option<Value>, Error> {
    spawn_call(conn, method, params).recv_timeout(PATIENCE).expect("the call did not return")
}

// conn_async_test.go:88
#[test]
fn test_async_conn_run_waits_for_handlers() {
    let protocol = queuedProtocol::new(vec![request(new_id_string("1"), "request"), notification("notification")], None);
    let (handler, started) = blockingHandler::new();
    let conn = new_async_conn_with_protocol(None, Box::new(protocol), Arc::<blockingHandler>::clone(&handler));

    let run_done = run_in_background(&conn);

    started.recv_timeout(PATIENCE).unwrap();
    started.recv_timeout(PATIENCE).unwrap();
    assert!(run_done.try_recv().is_err(), "Run returned while handlers were active");

    handler.release.open();
    assert_eq!(run_done.recv_timeout(PATIENCE).unwrap(), Ok(()));
}

// conn_async_test.go:120 TestAsyncConnRunCancelsHandlersOnEOF is not ported: Go cancels the context it passes to
// handlers when Run exits, and the port has no handler context.

// conn_async_test.go:138
#[test]
fn test_async_conn_response_write_failure_with_nil_transport() {
    let response_err = Error::new("response write failed");
    let protocol = queuedProtocol::new(vec![request(new_id_string("1"), "request")], Some(response_err));
    let conn = new_async_conn_with_protocol(None, Box::new(protocol), Arc::new(noOpHandler));

    let err = run_bounded(&conn).unwrap_err();
    assert_eq!(err.to_string(), "ipc: failed to write response: response write failed");
}

// conn_async_test.go:153
struct closeSignal {
    closed: Arc<gate>,
}

impl Closer for closeSignal {
    fn close(&self) -> std::io::Result<()> {
        self.closed.open();
        Ok(())
    }
}

// conn_async_test.go:171
struct failingResponseProtocol {
    closed: Arc<gate>,
    request_read: AtomicBool,
    response_err: Error,
}

impl Protocol for failingResponseProtocol {
    fn read_message(&self) -> Result<Message, Error> {
        if !self.request_read.swap(true, Ordering::SeqCst) {
            return Ok(request(new_id_int(1), "transform"));
        }
        self.closed.wait();
        Err(Error::new("io: read/write on closed pipe"))
    }

    fn write_request(&self, _: &ID, _: &str, _: Option<&Value>) -> Result<(), Error> {
        Ok(())
    }

    fn write_notification(&self, _: &str, _: Option<&Value>) -> Result<(), Error> {
        Ok(())
    }

    fn write_response(&self, _: &ID, _: &Value) -> Result<(), Error> {
        Err(self.response_err.clone())
    }

    fn write_error(&self, _: &ID, _: &ResponseError) -> Result<(), Error> {
        Err(self.response_err.clone())
    }
}

// conn_async_test.go:202
#[test]
fn test_async_conn_call_returns_when_peer_closes() {
    let (client, mut server) = pipe().unwrap();
    let client_closer = Arc::clone(&client.closer);
    let conn = new_async_conn(client, Arc::new(noOpHandler));
    let run_done = run_in_background(&conn);

    let call_done = spawn_call(&conn, "transform", None);

    let mut buffer = [0u8; 1024];
    assert!(server.reader.read(&mut buffer).unwrap() > 0);
    server.closer.close().unwrap();
    assert_eq!(run_done.recv_timeout(PATIENCE).unwrap(), Ok(()));
    let err = call_done.recv_timeout(PATIENCE).unwrap().unwrap_err();
    assert!(err.is(ErrorTag::ConnClosed), "{err}");
    assert_eq!(err.to_string(), ERR_CONN_CLOSED);
    client_closer.close().unwrap();
}

// conn_async_test.go:224
#[test]
fn test_async_conn_call_after_read_loop_failure_returns_immediately() {
    let (client, mut server) = pipe().unwrap();
    let conn = new_async_conn(client, Arc::new(noOpHandler));
    let run_done = run_in_background(&conn);

    std::io::Write::write_all(&mut server.writer, b"oops\n").unwrap();
    let err = run_done.recv_timeout(PATIENCE).unwrap().unwrap_err();
    assert!(err.to_string().contains("invalid header"), "{err}");

    let err = conn.call_with_timeout("transform", None, Duration::from_secs(1)).unwrap_err();
    assert!(err.is(ErrorTag::ConnClosed), "expected ErrConnClosed, got {err}");
    assert!(!err.is(ErrorTag::DeadlineExceeded), "call waited for its context deadline: {err}");
    assert_eq!(err.to_string(), "ipc: connection closed\njsonrpc: invalid header: \"oops\\n\"");
    let err = conn.notify("changed", None).unwrap_err();
    assert!(err.is(ErrorTag::ConnClosed), "expected ErrConnClosed, got {err}");
}

// conn_async_test.go:247
#[test]
fn test_async_conn_terminal_error_includes_response_write_failure() {
    let response_err = Error::new("response write failed");
    let closed = Arc::new(gate::default());
    let rwc = Arc::new(closeSignal { closed: Arc::clone(&closed) });
    let protocol = failingResponseProtocol { closed, request_read: AtomicBool::new(false), response_err };
    let conn = new_async_conn_with_protocol(Some(rwc), Box::new(protocol), Arc::new(noOpHandler));

    let err = run_bounded(&conn).unwrap_err();
    assert_eq!(err.to_string(), "io: read/write on closed pipe\nipc: failed to write response: response write failed");
    let err = call(&conn, "transform", None).unwrap_err();
    assert!(err.to_string().contains("response write failed"), "expected terminal response write error, got {err}");
    assert_eq!(err.to_string().matches("response write failed").count(), 1);
    assert!(err.is(ErrorTag::ConnClosed));
}

// conn_async_test.go:264
#[test]
fn test_async_conn_run_waits_for_request_after_peer_closes() {
    let (client, server) = pipe().unwrap();
    let (handler, started) = blockingHandler::new();
    let conn = new_async_conn(server, Arc::<blockingHandler>::clone(&handler));
    let run_done = run_in_background(&conn);

    let ReadWriteCloser { reader, writer, closer } = client;
    let client_protocol = new_jsonrpc_protocol(reader, writer);
    client_protocol.write_request(&new_id_int(1), "transform", None).unwrap();
    started.recv_timeout(Duration::from_secs(1)).expect("request handler did not start");
    // client.Close(). Every handle on the client end goes, so that the response write fails on macOS too (pipe()).
    closer.close().unwrap();
    drop((client_protocol, closer));

    match run_done.recv_timeout(Duration::from_millis(100)) {
        Err(RecvTimeoutError::Timeout) => {}
        result => panic!("connection stopped while request handler was blocked: {result:?}"),
    }

    handler.release.open();
    let err = run_done.recv_timeout(PATIENCE).expect("connection did not stop after request handler completed").unwrap_err();
    assert!(err.to_string().contains("ipc: failed to write response"), "{err}");
    let err = call(&conn, "transform", None).unwrap_err();
    assert!(err.to_string().contains("ipc: failed to write response"), "{err}");
    let err = conn.notify("changed", None).unwrap_err();
    assert!(err.to_string().contains("ipc: failed to write response"), "{err}");
}

// The in-process peer that stands in for a content mapper process. "echo" returns its params, "fail" fails,
// "panic" panics, "wait" announces itself on `started` and blocks until `release` opens; anything else is
// rejected.
struct testMapper {
    started: mpsc::Sender<()>,
    release: gate,
}

impl Handler for testMapper {
    fn handle_request(&self, method: &str, params: Option<&Value>) -> Result<Value, Error> {
        match method {
            "echo" => Ok(params.cloned().unwrap_or(Value::Null)),
            "fail" => Err(Error::new("transform failed")),
            "panic" => panic!("mapper bug"),
            "wait" => {
                self.started.send(()).unwrap();
                self.release.wait();
                Ok(Value::String("released".to_string()))
            }
            _ => Err(Error::new(format!("unexpected method {method}"))),
        }
    }

    fn handle_notification(&self, _: &str, _: Option<&Value>) -> Result<(), Error> {
        Ok(())
    }
}

// Our side's handler, like hostimpl.go's rejectHandler except that it answers "ping"; it reports every request and
// notification it gets.
struct hostHandler {
    handled: mpsc::Sender<(String, Option<Value>)>,
}

impl Handler for hostHandler {
    fn handle_request(&self, method: &str, params: Option<&Value>) -> Result<Value, Error> {
        self.handled.send((method.to_string(), params.cloned())).unwrap();
        if method == "ping" {
            return Ok(value(r#"{"pong":true}"#));
        }
        Err(Error::new(format!("content mapper sent an unexpected request: {method}")))
    }

    fn handle_notification(&self, method: &str, params: Option<&Value>) -> Result<(), Error> {
        self.handled.send((method.to_string(), params.cloned())).unwrap();
        Ok(())
    }
}

// One end of a connection, running on its own thread.
struct end {
    conn: Arc<AsyncConn>,
    closer: Arc<dyn Closer>,
    run: mpsc::Receiver<Result<(), Error>>,
}

fn start(rwc: ReadWriteCloser, handler: Arc<dyn Handler>) -> end {
    let closer = Arc::clone(&rwc.closer);
    let conn = new_async_conn(rwc, handler);
    let run = run_in_background(&conn);
    end { conn, closer, run }
}

// A connection to a test mapper: (host end, mapper end, the mapper, requests and notifications our handler got).
fn connect() -> (end, end, Arc<testMapper>, mpsc::Receiver<()>, mpsc::Receiver<(String, Option<Value>)>) {
    let (host_rwc, mapper_rwc) = pipe().unwrap();
    let (started, started_rx) = mpsc::channel();
    let mapper_handler = Arc::new(testMapper { started, release: gate::default() });
    let mapper = start(mapper_rwc, Arc::<testMapper>::clone(&mapper_handler));
    let (handled, handled_rx) = mpsc::channel();
    let host = start(host_rwc, Arc::new(hostHandler { handled }));
    (host, mapper, mapper_handler, started_rx, handled_rx)
}

// Closes the host's end of the connection and checks that both ends stop.
fn shut_down(host: &end, mapper: &end) {
    host.closer.close().unwrap();
    assert_eq!(host.run.recv_timeout(PATIENCE).unwrap(), Ok(()));
    assert_eq!(mapper.run.recv_timeout(PATIENCE).unwrap(), Ok(()));
}

// The compiler transforms files in parallel: concurrent calls on one connection each get their own response.
#[test]
fn concurrent_calls_each_get_their_own_response() {
    let (host, mapper, _, _, _) = connect();
    let (done, finished) = mpsc::channel();
    for t in 0..8 {
        let conn = Arc::clone(&host.conn);
        let done = done.clone();
        thread::spawn(move || {
            for i in 0..100 {
                let params = value(&format!(r#"{{"fileName":"/f{t}_{i}.vue","n":{i}}}"#));
                let result = conn.call("echo", Some(&params));
                if result != Ok(Some(params.clone())) {
                    let _ = done.send(Err(format!("{params:?} got {result:?}")));
                    return;
                }
            }
            let _ = done.send(Ok(()));
        });
    }
    for _ in 0..8 {
        finished.recv_timeout(PATIENCE).expect("a caller did not finish").unwrap();
    }
    shut_down(&host, &mapper);
}

// A response that overtakes an earlier request's goes to its own caller.
#[test]
fn responses_are_matched_by_id_not_by_order() {
    let (host, mapper, mapper_handler, started, _) = connect();
    let slow = spawn_call(&host.conn, "wait", None);
    started.recv_timeout(PATIENCE).unwrap();
    let fast = Value::String("fast".to_string());
    assert_eq!(call(&host.conn, "echo", Some(fast.clone())).unwrap(), Some(fast));
    mapper_handler.release.open();
    assert_eq!(slow.recv_timeout(PATIENCE).unwrap().unwrap(), Some(Value::String("released".to_string())));
    shut_down(&host, &mapper);
}

// A request the mapper sends is answered by our handler; a notification reaches it too.
#[test]
fn requests_from_the_peer_are_answered_by_the_handler() {
    let (host, mapper, _, _, handled) = connect();
    let params = value(r#"{"a":[1,"b"]}"#);
    assert_eq!(call(&mapper.conn, "ping", Some(params.clone())).unwrap(), Some(value(r#"{"pong":true}"#)));
    assert_eq!(handled.recv_timeout(PATIENCE).unwrap(), ("ping".to_string(), Some(params)));

    let err = call(&mapper.conn, "transform", None).unwrap_err();
    assert_eq!(err.to_string(), "ipc: remote error [-32603]: content mapper sent an unexpected request: transform");
    assert_eq!(handled.recv_timeout(PATIENCE).unwrap(), ("transform".to_string(), None));

    mapper.conn.notify("log", Some(&Value::String("hello".to_string()))).unwrap();
    assert_eq!(handled.recv_timeout(PATIENCE).unwrap(), ("log".to_string(), Some(Value::String("hello".to_string()))));
    shut_down(&host, &mapper);
}

// A failing handler answers with an internal error that the caller sees as a remote error; the connection goes on.
#[test]
fn a_handler_error_is_a_remote_error_for_the_caller() {
    let (host, mapper, _, _, _) = connect();
    let err = call(&host.conn, "fail", None).unwrap_err();
    assert_eq!(err.to_string(), "ipc: remote error [-32603]: transform failed");
    assert!(!err.is(ErrorTag::ConnClosed));
    assert_eq!(call(&host.conn, "echo", Some(Value::Bool(true))).unwrap(), Some(Value::Bool(true)));
    shut_down(&host, &mapper);
}

// A panicking handler answers with an internal error carrying the panic (Go's recover in handleRequest), and
// neither end of the connection stops.
#[test]
fn a_handler_panic_is_a_remote_error_for_the_caller() {
    let (host, mapper, _, _, _) = connect();
    let err = call(&host.conn, "panic", None).unwrap_err();
    assert_eq!(err.to_string(), "ipc: remote error [-32603]: panic: mapper bug");
    assert_eq!(call(&host.conn, "echo", Some(Value::Bool(true))).unwrap(), Some(Value::Bool(true)));
    assert!(mapper.run.try_recv().is_err(), "the mapper's connection stopped after its handler panicked");
    shut_down(&host, &mapper);
}

// The mapper process exits while calls wait: each fails with ErrConnClosed, Run ends cleanly, and later calls fail
// at once.
#[test]
fn eof_fails_every_pending_call() {
    let (host, mapper, mapper_handler, started, _) = connect();
    let calls: Vec<_> = (0..3).map(|_| spawn_call(&host.conn, "wait", None)).collect();
    for _ in 0..3 {
        started.recv_timeout(PATIENCE).unwrap();
    }

    mapper.closer.close().unwrap();
    for call in calls {
        let err = call.recv_timeout(PATIENCE).unwrap().unwrap_err();
        assert!(err.is(ErrorTag::ConnClosed), "{err}");
        assert_eq!(err.to_string(), ERR_CONN_CLOSED);
    }
    assert_eq!(host.run.recv_timeout(PATIENCE).unwrap(), Ok(()));

    let start = Instant::now();
    assert!(call(&host.conn, "echo", None).unwrap_err().is(ErrorTag::ConnClosed));
    assert!(host.conn.call_with_timeout("echo", None, PATIENCE).unwrap_err().is(ErrorTag::ConnClosed));
    assert!(host.conn.notify("log", None).unwrap_err().is(ErrorTag::ConnClosed));
    assert!(start.elapsed() < PATIENCE);

    mapper_handler.release.open();
    // Its responses cannot be written back any more.
    assert!(mapper.run.recv_timeout(PATIENCE).unwrap().is_err());
    host.closer.close().unwrap();
}

// Closing the stream from another thread (Close on the host's mapper process) unblocks the read loop, which ends
// cleanly and fails the calls still waiting.
#[test]
fn close_unblocks_the_read_loop() {
    let (host, mapper, mapper_handler, started, _) = connect();
    let waiting = spawn_call(&host.conn, "wait", None);
    started.recv_timeout(PATIENCE).unwrap();

    host.closer.close().unwrap();
    assert_eq!(host.run.recv_timeout(PATIENCE).unwrap(), Ok(()));
    assert!(waiting.recv_timeout(PATIENCE).unwrap().unwrap_err().is(ErrorTag::ConnClosed));
    // Close is idempotent.
    host.closer.close().unwrap();

    mapper_handler.release.open();
    assert!(mapper.run.recv_timeout(PATIENCE).is_ok());
}

// The bounded wait for the initialize request: the call gives up with context.DeadlineExceeded, the late response
// is dropped, and the connection keeps working.
#[test]
fn call_with_timeout_gives_up_with_deadline_exceeded() {
    let (host, mapper, mapper_handler, started, _) = connect();
    let timeout = Duration::from_millis(50);
    let start = Instant::now();
    let (done, timed_out) = mpsc::channel();
    {
        let conn = Arc::clone(&host.conn);
        thread::spawn(move || {
            let _ = done.send(conn.call_with_timeout("wait", None, timeout));
        });
    }
    let err = timed_out.recv_timeout(PATIENCE).expect("the call did not time out").unwrap_err();
    assert!(start.elapsed() >= timeout);
    assert!(err.is(ErrorTag::DeadlineExceeded), "{err}");
    assert!(!err.is(ErrorTag::ConnClosed));
    assert_eq!(err.to_string(), "context deadline exceeded");

    started.recv_timeout(PATIENCE).unwrap();
    mapper_handler.release.open();
    let next = Value::String("next".to_string());
    assert_eq!(call(&host.conn, "echo", Some(next.clone())).unwrap(), Some(next.clone()));
    assert_eq!(host.conn.call_with_timeout("echo", Some(&next), PATIENCE).unwrap(), Some(next.clone()));
    // A timeout too large for a deadline is no deadline.
    assert_eq!(host.conn.call_with_timeout("echo", Some(&next), Duration::MAX).unwrap(), Some(next));
    shut_down(&host, &mapper);
}

// The requests `call` writes, and how it reads the responses Go's Call distinguishes: no result (an empty
// json.Value, which callers then fail to decode), a null result, and an error response.
#[test]
fn call_writes_api_ids_and_returns_what_the_response_carries() {
    let (host_rwc, peer) = pipe().unwrap();
    let host = start(host_rwc, Arc::new(noOpHandler));
    let ReadWriteCloser { reader, writer, closer } = peer;
    let mut r = jsonrpc::new_reader(reader);
    let mut w = jsonrpc::new_writer(writer);

    let mut answer = |method: &str, params: Option<Value>, response: &str| {
        let result = spawn_call(&host.conn, method, params);
        let request = Message::unmarshal(&r.read().unwrap()).unwrap();
        w.write(response.replace("$ID", &request.id.as_ref().unwrap().string()).as_bytes()).unwrap();
        (request, result.recv_timeout(PATIENCE).expect("the call did not return"))
    };

    let (request, result) = answer("initialize", Some(value(r#"{"locale":"en"}"#)), r#"{"jsonrpc":"2.0","id":"$ID"}"#);
    assert_eq!(request.marshal().unwrap(), r#"{"jsonrpc":"2.0","id":"api1","method":"initialize","params":{"locale":"en"}}"#);
    assert_eq!(result, Ok(None));
    let (request, result) = answer("transform", None, r#"{"jsonrpc":"2.0","id":"$ID","result":null}"#);
    assert_eq!(request.marshal().unwrap(), r#"{"jsonrpc":"2.0","id":"api2","method":"transform"}"#);
    assert_eq!(result, Ok(Some(Value::Null)));
    let (_, result) = answer("transform", None, r#"{"jsonrpc":"2.0","id":"$ID","error":{"code":7,"message":"bad","data":{"x":1}}}"#);
    assert_eq!(result.unwrap_err().to_string(), "ipc: remote error [7]: bad");

    closer.close().unwrap();
    assert_eq!(host.run.recv_timeout(PATIENCE).unwrap(), Ok(()));
}

// The connection answers the timing meta-requests itself, as Go's does with timing collection disabled; they never
// reach the handler.
#[test]
fn server_timing_requests_are_answered_by_the_connection() {
    let (host, mapper, _, _, handled) = connect();
    let timing = call(&mapper.conn, METHOD_GET_SERVER_TIMING, None).unwrap().unwrap();
    assert_eq!(
        json::marshal(&timing).unwrap(),
        r#"{"enabled":false,"totals":{"requestCount":0,"totalProcessingTimeMs":0},"recentRequests":[]}"#
    );
    assert_eq!(call(&mapper.conn, METHOD_RESET_SERVER_TIMING, None).unwrap(), Some(Value::Null));
    assert_eq!(call(&mapper.conn, "ping", None).unwrap(), Some(value(r#"{"pong":true}"#)));
    assert_eq!(handled.recv_timeout(PATIENCE).unwrap(), ("ping".to_string(), None));
    shut_down(&host, &mapper);
}

// A protocol that panics reading once a request is out, like a bug in a Protocol implementation.
struct panickingProtocol {
    requested: gate,
}

impl Protocol for panickingProtocol {
    fn read_message(&self) -> Result<Message, Error> {
        self.requested.wait();
        panic!("protocol bug");
    }

    fn write_request(&self, _: &ID, _: &str, _: Option<&Value>) -> Result<(), Error> {
        self.requested.open();
        Ok(())
    }

    fn write_notification(&self, _: &str, _: Option<&Value>) -> Result<(), Error> {
        Ok(())
    }

    fn write_response(&self, _: &ID, _: &Value) -> Result<(), Error> {
        Ok(())
    }

    fn write_error(&self, _: &ID, _: &ResponseError) -> Result<(), Error> {
        Ok(())
    }
}

// Go's Run closes the pending calls in a defer, which runs when Run panics too: a panic on the read loop fails the
// waiting calls (and still panics Run) instead of leaving them waiting forever.
#[test]
fn a_panic_in_the_read_loop_fails_the_waiting_calls() {
    let conn = new_async_conn_with_protocol(None, Box::new(panickingProtocol { requested: gate::default() }), Arc::new(noOpHandler));
    let run = {
        let conn = Arc::clone(&conn);
        thread::spawn(move || conn.run())
    };
    let err = call(&conn, "transform", None).unwrap_err();
    assert!(err.is(ErrorTag::ConnClosed), "{err}");
    assert!(run.join().is_err(), "Run did not panic");
}
