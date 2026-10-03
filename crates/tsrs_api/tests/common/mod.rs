// Shared helpers for tsrs_api integration tests: a real session over the OS filesystem (with bundled libs)
// and a temp project directory.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;

use tsrs_api::{Handler, Response, Session, SessionOptions};
use tsrs_core::json::{self, Value};
use tsrs_vfs::{bundled, osvfs, FS};

pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(name: &str) -> TempDir {
        let base = std::env::temp_dir().join(format!("tsrs-api-test-{}-{}-{}", name, std::process::id(), rand_suffix()));
        std::fs::create_dir_all(&base).unwrap();
        TempDir(base.canonicalize().unwrap())
    }
    pub fn write(&self, rel: &str, text: &str) -> String {
        let path = self.0.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        self.path(rel)
    }
    pub fn path(&self, rel: &str) -> String {
        self.0.join(rel).to_string_lossy().replace('\\', "/")
    }
    pub fn dir(&self) -> String {
        self.0.to_string_lossy().replace('\\', "/")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn rand_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::SeqCst) * 1000 + (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos() as u64 % 1000)
}

pub fn session(cwd: &str, binary: bool) -> Arc<Session> {
    let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(osvfs::fs()));
    Session::new(SessionOptions {
        cwd: cwd.to_string(),
        default_library_path: bundled::lib_path(),
        fs,
        binary_responses: binary,
        run_external_code: false,
    })
}

/// Sends a JSON request and returns the parsed JSON result, panicking on error.
pub fn call(s: &Session, method: &str, params: &str) -> Value {
    match s.handle_request(method, params.as_bytes()) {
        Ok(Response::Json(text)) => json::unmarshal(&text).unwrap_or_else(|e| panic!("{method}: bad JSON {e}: {text}")),
        Ok(Response::Binary(b)) => panic!("{method}: unexpected binary response ({} bytes)", b.len()),
        Err(e) => panic!("{method} failed: {e}"),
    }
}

pub fn call_err(s: &Session, method: &str, params: &str) -> String {
    match s.handle_request(method, params.as_bytes()) {
        Ok(r) => panic!("{method}: expected error, got {r:?}"),
        Err(e) => e.to_string(),
    }
}

pub fn get<'a>(v: &'a Value, path: &str) -> &'a Value {
    let mut cur = v;
    for key in path.split('.') {
        cur = match cur {
            Value::Object(o) => o.get(key).unwrap_or_else(|| panic!("missing {key} in {}", json::marshal(v).unwrap())),
            Value::Array(a) => &a[key.parse::<usize>().unwrap()],
            _ => panic!("cannot index {key} in {}", json::marshal(v).unwrap()),
        };
    }
    cur
}

pub fn str_of(v: &Value) -> &str {
    match v {
        Value::String(s) => s,
        _ => panic!("not a string: {v:?}"),
    }
}

pub fn quote(s: &str) -> String {
    json::marshal_string(s)
}
