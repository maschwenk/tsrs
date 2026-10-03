// Request-lifetime stress through a real async (JSON-RPC) connection and core's Session: checker requests
// on snapshot S_k are in flight while S_k is updated to S_{k+1} and released. Every response must be
// either the correct answer for S_k's content or `snapshot N not found`; handles from a released
// snapshot must fail safely; nothing may crash. Run it with `TSRS_ARENA_POISON=1` so a read of freed
// region memory hits poison (0xA5) instead of stale data.
//
// TSRS_CHECKER_STRESS_ROUNDS (default 12) and TSRS_CHECKER_STRESS_FANOUT (default 8) scale the run.
// TSRS_CHECKER_STRESS_REPORT=<file> writes per-outcome counts.

use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use tsrs_api_transport as transport;
use tsrs_api_transport::jsonrpc::{FrameReader, FrameWriter};
use tsrs_core::json::{self, Value};
use tsrs_vfs::{bundled, vfstest, FS};

use super::session_tests::get;
use crate::session::{Session, SessionOptions};

struct Fwd(Arc<Session>);

impl transport::Handler for Fwd {
    fn handle_request(&self, _cx: &transport::RequestContext, method: &str, params: &[u8]) -> Result<transport::Response, transport::ApiError> {
        match crate::handler::Handler::handle_request(&*self.0, method, params) {
            Ok(crate::handler::Response::Json(t)) => Ok(transport::Response::Json(t.into_bytes())),
            Ok(crate::handler::Response::Binary(b)) => Ok(transport::Response::Binary(b)),
            Err(e) => Err(transport::ApiError::internal(e.to_string())),
        }
    }
    fn handle_notification(&self, _cx: &transport::RequestContext, _method: &str, _params: &[u8]) {}
}

fn env_num(name: &str, default: usize) -> usize {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

const B: &str = "import { value, big } from \"./a\";\nexport const use = value;\nexport const all = big;\n";

/// a.ts for version k: `value` is a number (even k) or a string (odd k), plus enough declarations that
/// a fresh checker does real work when `big` is resolved.
fn a_ts(k: usize) -> String {
    let mut s = if k % 2 == 0 { "export const value: number = 1;\n".to_string() } else { "export const value: string = \"s\";\n".to_string() };
    for i in 0..400 {
        s.push_str(&format!("export interface I{i}<T> {{ p{i}: T; q{i}: I{}<T> | undefined; }}\n", (i + 1) % 400));
    }
    s.push_str("export type Big = ");
    s.push_str(&(0..400).map(|i| format!("I{i}<number>")).collect::<Vec<_>>().join(" | "));
    s.push_str(";\n");
    // Deep tail-recursive conditional types: tens of milliseconds of checker work per fresh checker.
    s.push_str("type Build<N extends number, A extends unknown[] = []> = A[\"length\"] extends N ? A : Build<N, [...A, A[\"length\"]]>;\n");
    s.push_str("export declare const big: [");
    s.push_str(&(0..12).map(|i| format!("Build<{}>", 700 + i)).collect::<Vec<_>>().join(", "));
    s.push_str("];\n");
    s
}

fn q(s: &str) -> String {
    json::marshal_string(s)
}

/// Collects responses by id on a reader thread.
struct Client {
    w: FrameWriter<std::io::PipeWriter>,
    rx: mpsc::Receiver<Value>,
    pending: HashMap<String, Value>,
    next: u64,
    run: thread::JoinHandle<Result<(), transport::TransportError>>,
}

impl Client {
    fn send(&mut self, method: &str, params: &str) -> String {
        self.next += 1;
        let id = self.next.to_string();
        self.w.write_frame(format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","params":{params}}}"#).as_bytes()).unwrap();
        id
    }
    fn wait(&mut self, id: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if let Some(v) = self.pending.remove(id) {
                return v;
            }
            let v = self.rx.recv_timeout(deadline.saturating_duration_since(Instant::now())).unwrap_or_else(|_| panic!("response {id} not received"));
            let got = json::marshal(&get(&v, "id")).unwrap();
            self.pending.insert(got, v);
        }
    }
    fn call(&mut self, method: &str, params: &str) -> Value {
        let id = self.send(method, params);
        let v = self.wait(&id);
        assert_eq!(get(&v, "error"), Value::Null, "{method}: {}", json::marshal(&v).unwrap());
        get(&v, "result")
    }
}

fn start() -> Client {
    let files: [(&str, &str); 0] = [];
    let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(vfstest::from_map(files.iter().map(|(k, v)| (k.to_string(), v.to_string())), false)));
    let session = Session::new(SessionOptions::new("/".to_string(), bundled::lib_path(), fs, false));
    let (server_r, client_w) = std::io::pipe().unwrap();
    let (client_r, server_w) = std::io::pipe().unwrap();
    let conn = transport::AsyncConn::new(Box::new(FrameReader::new(server_r)), Box::new(FrameWriter::new(server_w)), Arc::new(Fwd(session)), transport::ConnOptions::default(), None);
    let run = thread::spawn(move || conn.run());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut r = FrameReader::new(client_r);
        let mut arrival = 0.0;
        while let Ok(frame) = r.read_frame() {
            let mut v = json::unmarshal(std::str::from_utf8(&frame).unwrap()).unwrap();
            arrival += 1.0;
            if let Value::Object(o) = &mut v {
                o.insert("arrival".to_string(), Value::Number(arrival));
            }
            if tx.send(v).is_err() {
                break;
            }
        }
    });
    Client { w: FrameWriter::new(client_w), rx, pending: HashMap::new(), next: 0, run }
}

#[derive(Default, Debug)]
struct Counts {
    correct: usize,
    snapshot_gone: usize,
    stale_handle_rejected: usize,
    /// Correct answers on S_k whose response arrived after the response to S_k's release: the request
    /// was in flight while the snapshot was released.
    correct_across_release: usize,
    other: Vec<String>,
}

const TSCONFIG: &str = r#"{ "compilerOptions": { "noLib": true, "strict": true }, "files": ["a.ts", "b.ts"] }"#;
const NUMBER: f64 = 64.0;
const STRING: f64 = 32.0;

#[test]
fn checker_requests_race_snapshot_update_and_release() {
    let rounds = env_num("TSRS_CHECKER_STRESS_ROUNDS", 12);
    let fanout = env_num("TSRS_CHECKER_STRESS_FANOUT", 8);
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(run_stress(rounds, fanout));
    });
    let counts = rx.recv_timeout(Duration::from_secs(600)).expect("stress run hung");
    if let Ok(path) = std::env::var("TSRS_CHECKER_STRESS_REPORT") {
        std::fs::write(path, format!("{counts:#?}\n")).unwrap();
    }
    assert!(counts.other.is_empty(), "unexpected outcomes: {:#?}", counts.other);
    assert!(counts.correct > 0 && counts.stale_handle_rejected > 0, "{counts:#?}");
    assert!(counts.correct_across_release > 0, "no request overlapped a release; the run proves nothing: {counts:#?}");
}

fn run_stress(rounds: usize, fanout: usize) -> Counts {
    let mut c = start();
    let mut counts = Counts::default();
    let full = format!(r#"{{"kind":"full","files":{{"/p/tsconfig.json":{},"/p/a.ts":{},"/p/b.ts":{}}}}}"#, q(TSCONFIG), q(&a_ts(0)), q(B));
    let r = c.call("createSnapshot", &format!(r#"{{"openProjects":["/p/tsconfig.json"],"fileSystem":{full}}}"#));
    let mut snap = match get(&r, "snapshot") {
        Value::Number(n) => n,
        other => panic!("{other:?}"),
    };
    let project = match get(&match get(&r, "projects") {
        Value::Array(a) => a[0].clone(),
        other => panic!("{other:?}"),
    }, "id")
    {
        Value::String(p) => p,
        other => panic!("{other:?}"),
    };
    let use_pos = B.find("use =").unwrap();
    let all_pos = B.find("all =").unwrap();
    for k in 0..rounds {
        let expected = if k % 2 == 0 { NUMBER } else { STRING };
        let sp = format!(r#""snapshot":{snap},"project":{}"#, q(&project));
        // Handles obtained from S_k before the race.
        let t = c.call("getTypeAtPosition", &format!(r#"{{{sp},"file":"/p/b.ts","position":{use_pos}}}"#));
        assert_eq!(get(&t, "flags"), Value::Number(expected));
        let type_id = json::marshal(&get(&t, "id")).unwrap();
        let sym = c.call("getSymbolAtPosition", &format!(r#"{{{sp},"file":"/p/b.ts","position":{use_pos}}}"#));
        let sym_ref = json::marshal(&get(&sym, "reference")).unwrap();
        // Race: fan-out of checker requests on S_k, the update to S_{k+1} and the release of S_k.
        // The update reads S_k, so it completes before S_k is released.
        let layer = format!(r#"{{"kind":"layer","files":{{"/p/a.ts":{}}}}}"#, q(&a_ts(k + 1)));
        let update = c.send("updateSnapshot", &format!(r#"{{"snapshot":{snap},"changes":{{"ensurePrograms":true,"fileSystem":{layer}}}}}"#));
        let v = c.wait(&update);
        let next = Some(match get(&get(&v, "result"), "snapshot") {
            Value::Number(n) => n,
            _ => panic!("updateSnapshot failed: {}", json::marshal(&v).unwrap()),
        });
        // Race: a fan-out of checker requests on S_k (the first ones do the heavy `big` resolution on
        // S_k's API checker) with the release of S_k sent right behind the first heavy request.
        let mut ids = Vec::new();
        for i in 0..fanout {
            let (method, params, check): (&str, String, u8) = match i % 4 {
                0 => ("getTypeAtPosition", format!(r#"{{{sp},"file":"/p/b.ts","position":{all_pos}}}"#), 1),
                1 => ("getTypeAtPosition", format!(r#"{{{sp},"file":"/p/b.ts","position":{use_pos}}}"#), 0),
                2 => ("typeToString", format!(r#"{{{sp},"type":{type_id}}}"#), 2),
                _ => ("getTypeOfSymbol", format!(r#"{{{sp},"symbol":{sym_ref}}}"#), 0),
            };
            ids.push((c.send(method, &params), check));
            if i == 0 {
                ids.push((c.send("release", &format!(r#"{{"snapshot":{snap}}}"#)), 8));
            }
        }
        let mut arrivals = Vec::new();
        for (id, check) in ids {
            let v = c.wait(&id);
            arrivals.push((check, get(&v, "arrival"), get(&v, "error") == Value::Null));
            let result = get(&v, "result");
            let error = get(&get(&v, "error"), "message");
            match (check, &error) {
                (8, Value::Null) => {}
                (0, Value::Null) if get(&result, "flags") == Value::Number(expected) => counts.correct += 1,
                (1, Value::Null) if get(&result, "flags") != Value::Null => counts.correct += 1,
                (2, Value::Null) if result == Value::String(if k % 2 == 0 { "number" } else { "string" }.into()) => counts.correct += 1,
                (0..=2, Value::String(m)) if m == &format!("api: client error: snapshot {snap} not found") => counts.snapshot_gone += 1,
                _ => counts.other.push(format!("round {k} check {check}: {}", json::marshal(&v).unwrap())),
            }
        }
        let released_at = arrivals.iter().find(|(c, _, _)| *c == 8).map(|(_, a, _)| json::marshal(a).unwrap().parse::<f64>().unwrap()).unwrap();
        counts.correct_across_release += arrivals
            .iter()
            .filter(|(c, a, ok)| *c != 8 && *ok && json::marshal(a).unwrap().parse::<f64>().unwrap() > released_at)
            .count();
        // After the release every S_k handle fails safely, also in the new snapshot's requests.
        for (method, params) in [
            ("typeToString", format!(r#"{{{sp},"type":{type_id}}}"#)),
            ("getTypeOfSymbol", format!(r#"{{{sp},"symbol":{sym_ref}}}"#)),
            ("getTypeAtPosition", format!(r#"{{{sp},"file":"/p/b.ts","position":{use_pos}}}"#)),
        ] {
            let id = c.send(method, &params);
            let v = c.wait(&id);
            match get(&get(&v, "error"), "message") {
                Value::String(m) if m == format!("api: client error: snapshot {snap} not found") => counts.stale_handle_rejected += 1,
                _ => counts.other.push(format!("round {k} stale {method}: {}", json::marshal(&v).unwrap())),
            }
        }
        snap = next.expect("updateSnapshot result");
        // The old symbol reference names a.ts/b.ts files: b.ts is unchanged, so a file-owned reference
        // may still resolve in S_{k+1} (pinned Go behaves the same); it must answer the *new* type.
        let sp2 = format!(r#""snapshot":{snap},"project":{}"#, q(&project));
        let id = c.send("getTypeOfSymbol", &format!(r#"{{{sp2},"symbol":{sym_ref}}}"#));
        let v = c.wait(&id);
        let next_expected = if k % 2 == 0 { STRING } else { NUMBER };
        match (get(&v, "error"), get(&get(&v, "result"), "flags")) {
            (Value::Null, f) if f == Value::Number(next_expected) => counts.correct += 1,
            (Value::Object(_), _) => counts.stale_handle_rejected += 1,
            _ => counts.other.push(format!("round {k} carried symbol: {}", json::marshal(&v).unwrap())),
        }
    }
    drop(c.w);
    assert!(c.run.join().unwrap().is_ok());
    counts
}

/// Sensitivity check for the poison runs: after the last reference to a snapshot is released, the
/// checker memory its type handles pointed into is freed; with `TSRS_ARENA_POISON=1` it reads as
/// poison (0xA5). This shows a use-after-release of a registry pointer would be visible in the stress
/// run above. Only asserted in poison mode (it deliberately reads the dead, still-mapped memory).
#[test]
fn released_snapshot_checker_memory_is_poisoned() {
    use crate::handler::{Handler, Response};
    let poison = std::env::var_os("TSRS_ARENA_POISON").is_some_and(|v| v == "1");
    let files: [(&str, &str); 0] = [];
    let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(vfstest::from_map(files.iter().map(|(k, v)| (k.to_string(), v.to_string())), false)));
    let session = Session::new(SessionOptions::new("/".to_string(), bundled::lib_path(), fs, false));
    let call = |method: &str, params: String| -> Value {
        match session.handle_request(method, params.as_bytes()) {
            Ok(Response::Json(t)) => json::unmarshal(&t).unwrap(),
            other => panic!("{method}: {other:?}"),
        }
    };
    let full = format!(r#"{{"kind":"full","files":{{"/p/tsconfig.json":{},"/p/a.ts":{},"/p/b.ts":{}}}}}"#, q(TSCONFIG), q(&a_ts(0)), q(B));
    let r = call("createSnapshot", format!(r#"{{"openProjects":["/p/tsconfig.json"],"fileSystem":{full}}}"#));
    let Value::Number(snap) = get(&r, "snapshot") else { panic!() };
    let Value::Array(projects) = get(&r, "projects") else { panic!() };
    let Value::String(project) = get(&projects[0], "id") else { panic!() };
    let pos = B.find("all =").unwrap();
    let t = call("getTypeAtPosition", format!(r#"{{"snapshot":{snap},"project":{},"file":"/p/b.ts","position":{pos}}}"#, q(&project)));
    let Value::Number(id) = get(&t, "id") else { panic!() };
    let addr = {
        let sd = session.snapshot_data(snap as u64).unwrap();
        let ty = sd.checker_state.registry.resolve_type(&project, id as u32, None).unwrap();
        ty.get() as *const tsrs_checker::Type as usize
    };
    let before: [u8; 16] = unsafe { std::ptr::read_volatile(addr as *const [u8; 16]) };
    assert_ne!(before, [0xA5; 16], "live type memory is not poison");
    call("release", format!(r#"{{"snapshot":{snap}}}"#));
    session.close();
    if poison {
        // SAFETY (test-only): poison mode keeps freed regions mapped and filled; this read is the probe.
        let after: [u8; 16] = unsafe { std::ptr::read_volatile(addr as *const [u8; 16]) };
        assert_eq!(after, [0xA5; 16], "released checker memory was not freed/poisoned at {addr:#x}");
    }
}
