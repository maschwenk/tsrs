//! Test-only server for exercising the transport with the pinned Node clients. It accepts the same
//! arguments the pinned clients pass (`--api [--async] --cwd <dir> --useCaseSensitiveFileNames=<b>
//! [--callbacks=<list>] [--timing] [--pipe <path>]`). It implements `echo` and `ping` like the pinned
//! session and a few `test/*` methods that drive the callback filesystem and connection behavior.
//! It is not the API server: the real session is wired by the CLI.

use std::sync::Arc;

use serde::Deserialize;
use tsrs_api_transport::callbackfs::{CallbackConfig, CallbackFs};
use tsrs_api_transport::{serve, ApiError, Caller, Handler, RequestContext, Response, ServeOptions};
use tsrs_vfs::FS;

struct TestHandler {
    is_async: bool,
    fs: Arc<CallbackFs>,
    caller: Arc<dyn Caller>,
    cwd: String,
}

#[derive(Deserialize)]
struct FsParams {
    op: String,
    path: String,
    #[serde(default)]
    data: String,
}

#[derive(Deserialize)]
struct CallClientParams {
    method: String,
    #[serde(default)]
    params: Option<Box<serde_json::value::RawValue>>,
}

fn bad(e: impl std::fmt::Display) -> ApiError {
    ApiError::internal(format!("api: invalid request: {e}"))
}

impl TestHandler {
    fn fs_op(&self, p: &FsParams) -> serde_json::Value {
        use serde_json::json;
        let fs: &dyn FS = &*self.fs;
        match p.op.as_str() {
            "readFile" => json!(fs.read_file(&p.path)),
            "fileExists" => json!(fs.file_exists(&p.path)),
            "directoryExists" => json!(fs.directory_exists(&p.path)),
            "getAccessibleEntries" => {
                let e = fs.get_accessible_entries(&p.path);
                let mut symlinks: Option<Vec<String>> = e.symlinks.map(|s| s.into_iter().collect());
                if let Some(s) = &mut symlinks {
                    s.sort();
                }
                json!({"files": e.files, "directories": e.directories, "symlinks": symlinks})
            }
            "realpath" => json!(fs.realpath(&p.path)),
            "stat" => match fs.stat(&p.path) {
                None => serde_json::Value::Null,
                Some(info) => {
                    let mtime_ms = info
                        .mod_time
                        .map(|t| match t.duration_since(std::time::UNIX_EPOCH) {
                            Ok(d) => d.as_millis() as i64,
                            Err(e) => -(e.duration().as_millis() as i64),
                        });
                    json!({"name": info.name, "size": info.size, "mode": info.mode.bits(), "isDir": info.is_dir(), "mtimeMs": mtime_ms})
                }
            },
            "writeFile" => json!(fs.write_file(&p.path, &p.data).err()),
            "removeFile" => json!(fs.remove(&p.path).err()),
            "useCaseSensitiveFileNames" => json!(fs.use_case_sensitive_file_names()),
            other => json!({"unknownOp": other}),
        }
    }
}

impl Handler for TestHandler {
    fn handle_request(&self, cx: &RequestContext, method: &str, params: &[u8]) -> Result<Response, ApiError> {
        match method {
            // Session.HandleRequest: echo is raw binary under msgpack, the JSON params otherwise.
            "echo" if !self.is_async => Ok(Response::Binary(params.to_vec())),
            "echo" => Ok(if params.is_empty() { Response::null() } else { Response::Json(params.to_vec()) }),
            "ping" => Response::json("pong"),
            "initialize" => Response::json(&serde_json::json!({
                "useCaseSensitiveFileNames": self.fs.use_case_sensitive_file_names(),
                "currentDirectory": self.cwd,
            })),
            // Test double of Session.handleBatchRequests (sequential, unpaginated) so the pinned
            // async client's automatic batching works against this server.
            "batchRequests" => {
                #[derive(Deserialize)]
                struct Item {
                    method: String,
                    #[serde(default)]
                    params: Option<Box<serde_json::value::RawValue>>,
                }
                #[derive(Deserialize)]
                struct Batch {
                    requests: Vec<Item>,
                }
                let batch: Batch = serde_json::from_slice(params).map_err(bad)?;
                let mut out = String::from("{\"responses\":[");
                for (i, item) in batch.requests.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    let p = item.params.as_ref().map(|r| r.get().as_bytes()).unwrap_or(b"");
                    let method_json = serde_json::to_string(&item.method).unwrap();
                    match self.handle_request(cx, &item.method, p) {
                        Ok(Response::Json(json)) => {
                            out.push_str(&format!("{{\"method\":{method_json},\"result\":{}}}", String::from_utf8_lossy(&json)))
                        }
                        Ok(Response::Binary(_)) => return Err(ApiError::internal("binary result in batch")),
                        Err(e) => out.push_str(&format!(
                            "{{\"method\":{method_json},\"result\":null,\"error\":{}}}",
                            serde_json::to_string(&e.message).unwrap()
                        )),
                    }
                }
                out.push_str("]}");
                Ok(Response::Json(out.into_bytes()))
            }
            "test/fs" => {
                let p: FsParams = serde_json::from_slice(params).map_err(bad)?;
                Response::json(&self.fs_op(&p))
            }
            "test/parallelFs" => {
                #[derive(Deserialize)]
                struct Ops {
                    ops: Vec<FsParams>,
                }
                let ps = serde_json::from_slice::<Ops>(params).map_err(bad)?.ops;
                let results: Vec<serde_json::Value> = std::thread::scope(|s| {
                    let handles: Vec<_> = ps.iter().map(|p| s.spawn(move || self.fs_op(p))).collect();
                    handles.into_iter().map(|h| h.join().unwrap_or(serde_json::Value::String("<panicked>".into()))).collect()
                });
                Response::json(&results)
            }
            "test/callClient" => {
                let p: CallClientParams = serde_json::from_slice(params).map_err(bad)?;
                let raw = self
                    .caller
                    .call(&p.method, p.params.as_ref().map(|r| r.get().as_bytes()))
                    .map_err(|e| ApiError::internal(e.to_string()))?;
                let text = String::from_utf8(raw).map_err(bad)?;
                Response::json(&serde_json::json!({"raw": text}))
            }
            "test/depth" => Response::json(&cx.depth),
            "test/fail" => Err(ApiError::internal("api: client error: requested failure \u{1F600}")),
            "test/panic" => panic!("requested panic \u{1F600}"),
            "test/badJson" => Ok(Response::Json(b"{not json".to_vec())),
            "test/exit" => std::process::exit(3),
            "test/sleep" => {
                let ms: u64 = serde_json::from_slice(params).map_err(bad)?;
                let start = std::time::Instant::now();
                while start.elapsed().as_millis() < ms as u128 {
                    if cx.cancel.is_cancelled() {
                        return Err(ApiError::internal("cancelled"));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Response::json(&ms)
            }
            _ => Err(ApiError::internal(format!("api: invalid request: unknown method {method:?}"))),
        }
    }
}

fn main() {
    let mut is_async = false;
    let mut cwd = String::new();
    let mut callbacks: Vec<String> = Vec::new();
    let mut case_sensitive = None;
    let mut timing = false;
    let mut pipe = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--api" => {}
            "--async" => is_async = true,
            "--timing" => timing = true,
            "--cwd" => cwd = args.next().unwrap_or_default(),
            "--pipe" => pipe = args.next(),
            _ if arg.starts_with("--callbacks=") => {
                callbacks = arg["--callbacks=".len()..].split(',').filter(|s| !s.is_empty()).map(String::from).collect();
            }
            _ if arg.starts_with("--useCaseSensitiveFileNames=") => {
                case_sensitive = Some(&arg["--useCaseSensitiveFileNames=".len()..] == "true");
            }
            _ => {
                eprintln!("transport_test_server: unknown argument {arg:?}");
                std::process::exit(2);
            }
        }
    }
    let config = match CallbackConfig::parse(&callbacks) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("transport_test_server: {e}");
            std::process::exit(2);
        }
    };
    let mut options = ServeOptions::new(is_async);
    options.collect_timing = timing;
    let result = serve(pipe.as_deref().map(std::path::Path::new), options, |caller| {
        let fs = Arc::new(CallbackFs::new(Arc::new(tsrs_vfs::osvfs::fs()), &config, case_sensitive));
        fs.set_connection(caller.clone());
        Arc::new(TestHandler { is_async, fs, caller, cwd })
    });
    if let Err(e) = result {
        eprintln!("transport_test_server: {e}");
        std::process::exit(1);
    }
}
