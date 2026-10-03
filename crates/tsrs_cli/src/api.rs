// Port of cmd/tsc/api.go (`tsrs --api`): flag parsing, the session filesystem (bundled libs over the OS
// filesystem, optionally behind the client callback filesystem), the in-process build backend for the
// API's build orchestrator methods, and the connection to the wire runtime.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tsrs_api::build::{BuildBackend, BuildOrchestrator, BuildOutcome, BuildRequest};
use tsrs_api::{ClientConn, Handler as _, Session, SessionOptions};
use tsrs_api_transport as transport;
use tsrs_api_transport::{CallbackConfig, CallbackFs};
use tsrs_core::P;
use tsrs_tsoptions::ParseConfigHost;
use tsrs_vfs::{bundled, osvfs, FS};

use crate::build::{new_orchestrator, Options};
use crate::lsp::flagSet;
use crate::tsc::System;

/// Go `apiFlags`.
pub(crate) struct ApiFlags {
    pub cwd: String,
    pub pipe_path: String,
    pub callbacks: Vec<String>,
    pub case_sensitive: bool,
    pub is_async: bool,
    pub timing: bool,
    pub run_external_code: bool,
}

/// Go `parseAPIFlags`.
pub(crate) fn parse_api_flags(args: &[String]) -> Result<ApiFlags, ()> {
    let mut flags = flagSet::new("api");
    flags.string("cwd", "current working directory");
    flags.string("pipe", "use named pipe or Unix domain socket for communication instead of stdio");
    flags.string("callbacks", "comma-separated list of FS callbacks and defaults to enable");
    flags.bool("useCaseSensitiveFileNames", "treat filesystem paths as case-sensitive");
    flags.bool("async", "use JSON-RPC protocol instead of MessagePack (for async API)");
    flags.bool("timing", "collect per-request server processing time, folded into the client's timing snapshot");
    flags.bool("runExternalCode", "allow projects to execute configured external plugins");
    flags.parse(args)?;
    // Go defaults: cwd = os.Getwd(), useCaseSensitiveFileNames = the OS filesystem's sensitivity.
    let cwd = match flags.string_value("cwd") {
        c if !c.is_empty() => c,
        _ => std::env::current_dir().map(|d| d.to_string_lossy().replace('\\', "/")).map_err(|_| ())?,
    };
    let case_flag_given = args.iter().any(|a| {
        let a = a.trim_start_matches('-');
        a == "useCaseSensitiveFileNames" || a.starts_with("useCaseSensitiveFileNames=")
    });
    let case_sensitive = if case_flag_given { flags.bool_value("useCaseSensitiveFileNames") } else { osvfs::fs().use_case_sensitive_file_names() };
    let callbacks = flags.string_value("callbacks");
    Ok(ApiFlags {
        cwd,
        pipe_path: flags.string_value("pipe"),
        callbacks: if callbacks.is_empty() { Vec::new() } else { callbacks.split(',').map(str::to_string).collect() },
        case_sensitive,
        is_async: flags.bool_value("async"),
        timing: flags.bool_value("timing"),
        run_external_code: flags.bool_value("runExternalCode"),
    })
}

/// Go `StdioServer.Run` setup: the session over bundled libs + OS filesystem, wrapped by the client
/// callback filesystem (the CLI always passes a case-sensitivity setting, so Go always wraps).
pub(crate) fn new_api_session(flags: &ApiFlags) -> Result<(Arc<Session>, Arc<CallbackFs>), String> {
    let base: Arc<dyn FS> = Arc::new(bundled::wrap_fs(osvfs::fs()));
    let config = CallbackConfig::parse(&flags.callbacks)?;
    let callback_fs = Arc::new(CallbackFs::new(base, &config, Some(flags.case_sensitive)));
    let fs: Arc<dyn FS> = callback_fs.clone();
    let mut options = SessionOptions::new(flags.cwd.clone(), bundled::lib_path(), fs, !flags.is_async);
    options.run_external_code = flags.run_external_code;
    let session = Session::new(options);
    session.set_build_backend(Arc::new(CliBuildBackend));
    Ok((session, callback_fs))
}

/// Adapts the core session to the transport's handler (contract: crates/tsrs_api_transport/README.md).
struct SessionHandler(Arc<Session>);

impl transport::Handler for SessionHandler {
    fn handle_request(&self, cx: &transport::RequestContext, method: &str, params: &[u8]) -> Result<transport::Response, transport::ApiError> {
        // Go: errors are CodeInternalError with err.Error().
        match self.0.handle_nested_request(method, params, cx.depth) {
            Ok(tsrs_api::Response::Json(s)) => Ok(transport::Response::Json(s.into_bytes())),
            Ok(tsrs_api::Response::Binary(b)) => Ok(transport::Response::Binary(b)),
            Err(e) => Err(transport::ApiError::internal(e.to_string())),
        }
    }

    fn handle_notification(&self, _cx: &transport::RequestContext, method: &str, params: &[u8]) {
        let _ = self.0.handle_notification(method, params);
    }
}

impl Drop for SessionHandler {
    fn drop(&mut self) {
        self.0.close();
    }
}

struct ClientConnAdapter(Arc<dyn transport::Caller>);

impl ClientConn for ClientConnAdapter {
    fn call(&self, method: &str, params: &str) -> tsrs_api::ApiResult<String> {
        let raw = self.0.call(method, Some(params.as_bytes())).map_err(|e| tsrs_api::ApiError::internal(e.to_string()))?;
        String::from_utf8(raw).map_err(|e| tsrs_api::ApiError::internal(e.to_string()))
    }
}

/// `tsrs --api` (Go `runAPI` + `StdioServer.Run`). stdout carries only protocol bytes.
pub fn run_api(args: &[String]) -> i32 {
    let Ok(flags) = parse_api_flags(args) else { return 2 };
    if let Err(err) = CallbackConfig::parse(&flags.callbacks) {
        eprintln!("{err}");
        return 2;
    }
    let mut options = transport::ServeOptions::new(flags.is_async);
    options.collect_timing = flags.timing;
    let pipe = (!flags.pipe_path.is_empty()).then(|| std::path::PathBuf::from(&flags.pipe_path));
    let result = transport::serve(pipe.as_deref(), options, |caller| {
        let (session, callback_fs) = new_api_session(&flags).expect("callbacks validated above");
        callback_fs.set_connection(caller.clone());
        session.set_connection(Arc::new(ClientConnAdapter(caller)));
        Arc::new(SessionHandler(session))
    });
    match result {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("{err}");
            1
        }
    }
}

/// Go `apiBuildSystem`: output is discarded, the filesystem is the session's host filesystem.
struct ApiBuildSystem {
    fs: Arc<dyn FS>,
    default_library_path: String,
    current_directory: String,
    start: Instant,
}

impl System for ApiBuildSystem {
    fn fs(&self) -> Arc<dyn FS> {
        self.fs.clone()
    }
    fn default_library_path(&self) -> &str {
        &self.default_library_path
    }
    fn get_current_directory(&self) -> &str {
        &self.current_directory
    }
    fn write(&self, _text: &str) {}
    fn flush(&self) {}
    fn write_output_is_tty(&self) -> bool {
        false
    }
    fn get_environment_variable(&self, _name: &str) -> Option<String> {
        None
    }
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn since_start(&self) -> Duration {
        self.start.elapsed()
    }
}

impl ParseConfigHost for ApiBuildSystem {
    fn fs(&self) -> &dyn FS {
        &*self.fs
    }
    fn get_current_directory(&self) -> &str {
        &self.current_directory
    }
}

struct CliBuildBackend;

/// One API build orchestrator. Go keeps one `build.Orchestrator`: `Build` rechecks all projects (resetting
/// statuses, configs, mtimes and caches) and regenerates the graph, while `Clean` reuses the graph of the
/// last build when there is one ("cleans the last built configuration"). This port builds a fresh
/// orchestrator for every build (same observable state after Go's recheck) and keeps it for later cleans.
struct CliOrchestrator {
    sys: &'static ApiBuildSystem,
    command: P<tsrs_tsoptions::ParsedBuildCommandLine>,
    last: Option<&'static crate::build::Orchestrator>,
}

impl BuildBackend for CliBuildBackend {
    fn create(&self, request: BuildRequest) -> Box<dyn BuildOrchestrator> {
        // `&'static` system and command line: the CLI build module requires them (one small leak per orchestrator).
        let sys: &'static ApiBuildSystem = Box::leak(Box::new(ApiBuildSystem {
            fs: request.fs,
            default_library_path: request.default_library_path,
            current_directory: request.current_directory,
            start: Instant::now(),
        }));
        let mut command = tsrs_tsoptions::parse_build_command_line(&request.root_names, sys);
        if let Some(options) = request.compiler_options {
            command.compiler_options = options;
        }
        if let Some(options) = request.build_options {
            command.build_options = options;
        }
        Box::new(CliOrchestrator { sys, command: P::new(command), last: None })
    }
}

fn outcome(result: crate::build::OrchestratorResult) -> BuildOutcome {
    BuildOutcome {
        status: result.status.unwrap_or(crate::tsc::ExitStatus::Success) as i32,
        diagnostics: result.errors.unwrap_or_default(),
        projects: result.statistics.projects,
        projects_built: result.statistics.projects_built,
        timestamp_updates: result.statistics.timestamp_updates,
        files_deleted: result.files_to_delete.unwrap_or_default(),
    }
}

impl BuildOrchestrator for CliOrchestrator {
    fn build(&mut self, project: &str, only_references: bool) -> BuildOutcome {
        let orchestrator = new_orchestrator(Options { sys: self.sys, command: self.command, testing: None });
        self.last = Some(orchestrator);
        outcome(orchestrator.build_for_api(project, only_references))
    }
    fn clean(&mut self, project: &str, only_references: bool) -> BuildOutcome {
        let orchestrator = match self.last {
            Some(o) => o,
            None => {
                let o = new_orchestrator(Options { sys: self.sys, command: self.command, testing: None });
                self.last = Some(o);
                o
            }
        };
        outcome(orchestrator.clean_for_api(project, only_references))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tsrs_api::{Handler, Response};
    use tsrs_core::json::{self, Value};

    fn call(s: &Session, method: &str, params: &str) -> Value {
        match s.handle_request(method, params.as_bytes()) {
            Ok(Response::Json(t)) => json::unmarshal(&t).unwrap(),
            other => panic!("{method}: {other:?}"),
        }
    }

    fn get<'a>(v: &'a Value, key: &str) -> &'a Value {
        match v {
            Value::Object(o) => o.get(key).unwrap_or(&Value::Null),
            _ => &Value::Null,
        }
    }

    #[test]
    fn api_flags() {
        let f = parse_api_flags(&["--async".into(), "-cwd".into(), "/x".into(), "--callbacks=readFile,fileExists".into(), "--useCaseSensitiveFileNames=false".into()]).unwrap();
        assert!(f.is_async && !f.case_sensitive);
        assert_eq!(f.cwd, "/x");
        assert_eq!(f.callbacks, vec!["readFile", "fileExists"]);
        assert!(parse_api_flags(&["--nope".into()]).is_err());
    }

    #[test]
    fn build_orchestrator_builds_references_and_cleans() {
        let dir = std::env::temp_dir().join(format!("tsrs-api-build-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("core")).unwrap();
        std::fs::create_dir_all(dir.join("app")).unwrap();
        let w = |p: &str, t: &str| std::fs::write(dir.join(p), t).unwrap();
        w("core/tsconfig.json", r#"{ "compilerOptions": { "composite": true, "outDir": "out" } }"#);
        w("core/index.ts", "export const one = 1;\n");
        w("app/tsconfig.json", r#"{ "compilerOptions": { "composite": true, "outDir": "out" }, "references": [{ "path": "../core" }] }"#);
        w("app/main.ts", "import { one } from '../core/index';\nexport const two: number = one + 1;\n");
        let cwd = dir.canonicalize().unwrap().to_string_lossy().into_owned();
        let flags = ApiFlags { cwd: cwd.clone(), pipe_path: String::new(), callbacks: Vec::new(), case_sensitive: true, is_async: true, timing: false, run_external_code: false };
        let (s, _) = new_api_session(&flags).unwrap();
        let s = s;
        let r = call(&s, "createBuildOrchestrator", r#"{"rootNames":["app"]}"#);
        let id = json::marshal(get(&r, "buildOrchestratorID")).unwrap();

        let r = call(&s, "buildReferences", &format!("{{\"buildOrchestratorID\":{id},\"project\":\"app\"}}"));
        assert_eq!(get(&r, "status"), &Value::Number(0.0), "{}", json::marshal(&r).unwrap());
        assert_eq!(get(get(&r, "statistics"), "Projects"), &Value::Number(1.0));
        assert!(dir.join("core/out/index.js").exists());
        assert!(!dir.join("app/out/main.js").exists());

        let r = call(&s, "build", &format!("{{\"buildOrchestratorID\":{id}}}"));
        assert_eq!(get(&r, "status"), &Value::Number(0.0), "{}", json::marshal(&r).unwrap());
        assert_eq!(get(get(&r, "statistics"), "Projects"), &Value::Number(2.0));
        assert!(dir.join("app/out/main.js").exists());

        // Second build: everything is up to date.
        let r = call(&s, "build", &format!("{{\"buildOrchestratorID\":{id}}}"));
        assert_eq!(get(get(&r, "statistics"), "ProjectsBuilt"), &Value::Number(0.0), "{}", json::marshal(&r).unwrap());

        let r = call(&s, "cleanBuild", &format!("{{\"buildOrchestratorID\":{id}}}"));
        assert_eq!(get(&r, "status"), &Value::Number(0.0));
        assert!(json::marshal(get(&r, "filesDeleted")).unwrap().contains("main.js"));
        assert!(!dir.join("app/out/main.js").exists());
        assert!(!dir.join("core/out/index.js").exists());

        assert_eq!(call(&s, "disposeBuildOrchestrator", &format!("{{\"buildOrchestratorID\":{id}}}")), Value::Bool(true));
        assert!(s.handle_request("build", format!("{{\"buildOrchestratorID\":{id}}}").as_bytes()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
