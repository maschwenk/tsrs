mod common;
use common::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tsrs_api::{Session, SessionOptions};
use tsrs_api_transport::{CallbackConfig, CallbackFs, Caller, TransportError};
use tsrs_core::json::{self, Value};
use tsrs_vfs::{bundled, osvfs, FS};

/// A client serving an in-memory tree under `root` and deferring everything else to the OS (`useOS`).
struct MemoryClient {
    root: String,
    files: HashMap<String, String>,
    calls: Mutex<Vec<String>>,
}

impl Caller for MemoryClient {
    fn notify(&self, _method: &str, _params: Option<&[u8]>) -> Result<(), TransportError> {
        Ok(())
    }
    fn call(&self, method: &str, params: Option<&[u8]>) -> Result<Vec<u8>, TransportError> {
        let params = std::str::from_utf8(params.unwrap_or(b"null")).unwrap().to_string();
        Ok(self.respond(method, &params).into_bytes())
    }
}

impl MemoryClient {
    fn respond(&self, method: &str, params: &str) -> String {
        self.calls.lock().unwrap().push(method.to_string());
        let path = match json::unmarshal(params).unwrap() {
            Value::String(p) => p,
            _ => String::new(),
        };
        if !path.starts_with(&self.root) {
            return r#"{"kind":"useOS"}"#.into();
        }
        let r = match method {
            "readFile" => match self.files.get(&path) {
                Some(c) => format!("{{\"kind\":\"value\",\"value\":{}}}", quote(c)),
                None => r#"{"kind":"missing"}"#.into(),
            },
            "fileExists" => format!("{{\"kind\":\"value\",\"value\":{}}}", self.files.contains_key(&path)),
            "directoryExists" => {
                let prefix = format!("{}/", path.trim_end_matches('/'));
                format!("{{\"kind\":\"value\",\"value\":{}}}", path == self.root || self.files.keys().any(|f| f.starts_with(&prefix)))
            }
            "getAccessibleEntries" => {
                let prefix = format!("{}/", path.trim_end_matches('/'));
                let files: Vec<String> = self.files.keys().filter_map(|f| f.strip_prefix(&prefix)).filter(|r| !r.contains('/')).map(quote).collect();
                format!("{{\"kind\":\"value\",\"value\":{{\"files\":[{}],\"directories\":[]}}}}", files.join(","))
            }
            "realpath" => r#"{"kind":"identity"}"#.into(),
            _ => r#"{"kind":"error"}"#.into(),
        };
        r
    }
}

#[test]
fn program_reads_through_client_callbacks() {
    let dir = TempDir::new("cbfs");
    let root = format!("{}/virtual", dir.dir());
    let a = format!("{root}/a.ts");
    let b = format!("{root}/b.ts");
    let client = Arc::new(MemoryClient {
        root: root.clone(),
        files: HashMap::from([(a.clone(), "import { b } from './b';\nexport const a: string = b;\n".to_string()), (b.clone(), "export const b = 42;\n".to_string())]),
        calls: Mutex::new(Vec::new()),
    });
    let callbacks: Vec<String> = ["readFile", "fileExists", "directoryExists", "getAccessibleEntries", "realpath"].iter().map(|s| s.to_string()).collect();
    let cbfs = Arc::new(CallbackFs::new(Arc::new(bundled::wrap_fs(osvfs::fs())), &CallbackConfig::parse(&callbacks).unwrap(), Some(true)));
    cbfs.set_connection(client.clone());
    let fs: Arc<dyn FS> = cbfs;
    let s = Session::new(SessionOptions { cwd: dir.dir(), default_library_path: bundled::lib_path(), fs, binary_responses: false, run_external_code: false });
    let r = call(&s, "createSnapshot", &format!("{{\"createPrograms\":[{{\"rootFiles\":[{}],\"compilerOptions\":{{\"strict\":true}}}}]}}", quote(&a)));
    let snap = json::marshal(get(&r, "snapshot")).unwrap();
    let project = str_of(get(&r, "operation.createdPrograms.0")).to_string();
    let names = json::marshal(&call(&s, "getSourceFileNames", &format!("{{\"snapshot\":{snap},\"project\":{}}}", quote(&project)))).unwrap();
    assert!(names.contains(&b), "{names}");
    let d = call(&s, "getSemanticDiagnostics", &format!("{{\"snapshot\":{snap},\"project\":{},\"files\":[{}]}}", quote(&project), quote(&a)));
    assert_eq!(get(&d, "0.code"), &Value::Number(2322.0));
    assert!(client.calls.lock().unwrap().iter().any(|c| c == "readFile"));
    assert!(!std::path::Path::new(&a).exists());
}

#[test]
fn unknown_callbacks_are_rejected() {
    assert!(CallbackConfig::parse(&["nope"]).is_err());
}
