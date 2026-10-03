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

/// One API build orchestrator handle. Go keeps one `build.Orchestrator`: `Build` rechecks every project in the
/// build order (statuses, configs, mtimes, caches) and regenerates the graph; `Clean` uses the graph of the last
/// build ("cleans the last built configuration"). Here each build gets a fresh CLI orchestrator (the state Go's
/// recheck leaves) whose allocations live in collectable regions; it is kept for later cleans and freed when the
/// next build replaces it or the handle is disposed, so memory stays bounded by one build.
struct CliOrchestrator {
    sys: &'static ApiBuildSystem,
    command: P<tsrs_tsoptions::ParsedBuildCommandLine>,
    orchestrator: Option<&'static crate::build::Orchestrator>,
}

impl CliOrchestrator {
    fn fresh(&self) -> &'static crate::build::Orchestrator {
        let o = new_orchestrator(Options { sys: self.sys, command: self.command, testing: None });
        o.enable_api_regions();
        o
    }

    fn replace(&mut self, o: &'static crate::build::Orchestrator) {
        if let Some(old) = self.orchestrator.replace(o) {
            // SAFETY: no build is running on `old` (builds are serialized per handle) and its results were
            // converted to owned `BuildOutcome`s.
            unsafe { crate::build::free_api_orchestrator(old) };
        }
    }
}

impl Drop for CliOrchestrator {
    fn drop(&mut self) {
        if let Some(o) = self.orchestrator.take() {
            // SAFETY: as in `replace`; the handle is being disposed.
            unsafe { crate::build::free_api_orchestrator(o) };
        }
    }
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
        // API builds allocate in per-task regions (CliOrchestrator); program construction and checking must stay on
        // the task's thread so their allocations land there, not in the compiler worker pool's thread arenas.
        command.compiler_options.single_threaded = tsrs_core::Tristate::True;
        Box::new(CliOrchestrator { sys, command: P::new(command), orchestrator: None })
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
        let o = self.fresh();
        let result = outcome(o.build_for_api(project, only_references));
        self.replace(o);
        result
    }
    fn clean(&mut self, project: &str, only_references: bool) -> BuildOutcome {
        let o = match self.orchestrator {
            Some(o) => o,
            None => {
                let o = self.fresh();
                self.replace(o);
                o
            }
        };
        outcome(o.clean_for_api(project, only_references))
    }
}

/// Serializes the API build tests: `memory_tests` measure process RSS, which a concurrent build test inflates.
#[cfg(test)]
static BUILD_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
        let _serial = super::BUILD_TESTS.lock().unwrap_or_else(|e| e.into_inner());
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

        // Upstream "returns build response information after clean": clean one project, rebuild it, then
        // cleanReferences of the downstream project deletes the rebuilt outputs too.
        call(&s, "build", &format!("{{\"buildOrchestratorID\":{id}}}"));
        let r = call(&s, "cleanBuild", &format!("{{\"buildOrchestratorID\":{id},\"project\":\"core\"}}"));
        assert!(json::marshal(get(&r, "filesDeleted")).unwrap().contains("index.js"), "{}", json::marshal(&r).unwrap());
        let r = call(&s, "build", &format!("{{\"buildOrchestratorID\":{id},\"project\":\"core\"}}"));
        assert_eq!(get(get(&r, "statistics"), "ProjectsBuilt"), &Value::Number(1.0), "{}", json::marshal(&r).unwrap());
        assert!(dir.join("core/out/index.js").exists());
        let r = call(&s, "cleanReferences", &format!("{{\"buildOrchestratorID\":{id},\"project\":\"app\"}}"));
        assert!(json::marshal(get(&r, "filesDeleted")).unwrap().contains("core/out/index.js"), "{}", json::marshal(&r).unwrap());

        // Upstream "cleans and rebuilds ..." tail, parity f703: after build + clean, the client recreates only one
        // output; Go's next clean reuses the cached existence answers (cachedvfs, cleared only by a build), so it
        // lists every output of the project again (index.js, index.d.ts, tsbuildinfo) and status stays 0.
        call(&s, "build", &format!("{{\"buildOrchestratorID\":{id}}}"));
        let all = |r: &Value| {
            let mut v: Vec<String> = match get(r, "filesDeleted") {
                Value::Array(a) => a.iter().map(|f| json::marshal(f).unwrap().rsplit('/').next().unwrap().trim_end_matches('"').to_string()).collect(),
                _ => Vec::new(),
            };
            v.sort();
            v
        };
        let r = call(&s, "cleanBuild", &format!("{{\"buildOrchestratorID\":{id},\"project\":\"core\"}}"));
        assert_eq!(all(&r), ["index.d.ts", "index.js", "tsconfig.tsbuildinfo"], "{}", json::marshal(&r).unwrap());
        w("core/out/index.js", "export const one = 1;\n");
        let r = call(&s, "cleanBuild", &format!("{{\"buildOrchestratorID\":{id},\"project\":\"core\"}}"));
        assert_eq!(get(&r, "status"), &Value::Number(0.0), "{}", json::marshal(&r).unwrap());
        assert_eq!(all(&r), ["index.d.ts", "index.js", "tsconfig.tsbuildinfo"], "{}", json::marshal(&r).unwrap());
        assert!(!dir.join("core/out/index.js").exists());

        // moduleResolution with no named kind: Go creates the orchestrator (and panics while building); tsrs keeps
        // creation, cleaning and disposal and fails builds with a stable error (runtime f552 review).
        let r = call(&s, "createBuildOrchestrator", r#"{"rootNames":["app"],"compilerOptions":{"moduleResolution":12345,"checkers":9223372036854775807}}"#);
        let other = json::marshal(get(&r, "buildOrchestratorID")).unwrap();
        let e = s.handle_request("build", format!("{{\"buildOrchestratorID\":{other}}}").as_bytes()).unwrap_err();
        assert_eq!(e.to_string(), "api: client error: cannot build with unsupported moduleResolution value 12345 (not a ModuleResolutionKind)");
        let r = call(&s, "cleanBuild", &format!("{{\"buildOrchestratorID\":{other}}}"));
        assert_eq!(get(&r, "status"), &Value::Number(0.0), "{}", json::marshal(&r).unwrap());
        assert_eq!(call(&s, "disposeBuildOrchestrator", &format!("{{\"buildOrchestratorID\":{other}}}")), Value::Bool(true));

        // Go `*int` builders: any int64 is accepted (API builds run one builder regardless; runtime-f2-review).
        for n in ["2147483648", "-2147483649", "9223372036854775807"] {
            let r = call(&s, "createBuildOrchestrator", &format!(r#"{{"rootNames":["app"],"buildOptions":{{"builders":{n}}}}}"#));
            let other = json::marshal(get(&r, "buildOrchestratorID")).unwrap();
            let r = call(&s, "build", &format!("{{\"buildOrchestratorID\":{other}}}"));
            assert_eq!(get(&r, "status"), &Value::Number(0.0), "builders {n}: {}", json::marshal(&r).unwrap());
            call(&s, "disposeBuildOrchestrator", &format!("{{\"buildOrchestratorID\":{other}}}"));
        }

        assert_eq!(call(&s, "disposeBuildOrchestrator", &format!("{{\"buildOrchestratorID\":{id}}}")), Value::Bool(true));
        assert!(s.handle_request("build", format!("{{\"buildOrchestratorID\":{id}}}").as_bytes()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod memory_tests {
    use super::*;
    use tsrs_api::{Handler, Response};
    use tsrs_core::json::{self, Value};

    extern "C" {
        fn malloc_trim(pad: usize) -> i32;
    }

    fn rss_kib() -> u64 {
        // SAFETY: glibc `malloc_trim` has no preconditions.
        unsafe { malloc_trim(0) };
        std::fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|s| s.lines().find(|l| l.starts_with("VmRSS:")).and_then(|l| l.split_whitespace().nth(1)?.parse().ok()))
            .unwrap_or(0)
    }

    /// API builds (fresh orchestrator, task regions, one builder, single-threaded programs) write the same files,
    /// byte for byte, as `tsrs -b` on the same project graph.
    #[test]
    fn api_build_outputs_match_cli_build() {
        let _serial = super::BUILD_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let base = std::env::temp_dir().join(format!("tsrs-api-buildeq-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let mk = |root: &std::path::Path| {
            let w = |p: &str, t: &str| {
                std::fs::create_dir_all(root.join(p).parent().unwrap()).unwrap();
                std::fs::write(root.join(p), t).unwrap();
            };
            w("core/tsconfig.json", r#"{ "compilerOptions": { "composite": true, "outDir": "out", "declarationMap": true, "sourceMap": true } }"#);
            w("core/index.ts", "export const one = 1;\nexport class C { x = 1; private y = 'a'; }\n");
            w("app/tsconfig.json", r#"{ "compilerOptions": { "composite": true, "outDir": "out", "strict": true }, "references": [{ "path": "../core" }] }"#);
            w("app/main.ts", "import { one, C } from '../core/index';\nexport const two: number = one + 1;\nexport const c = new C();\nlet bad: string = 1;\n");
        };
        let cli = base.join("cli");
        let api = base.join("api");
        mk(&cli);
        mk(&api);
        // The tsrs binary next to this test binary (target/<profile>/tsrs; `cargo build -p tsrs_cli` first).
        let exe = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().join("tsrs");
        assert!(exe.exists(), "build the tsrs binary first: {}", exe.display());
        let status = std::process::Command::new(&exe).args(["-b", "app"]).current_dir(&cli).status().unwrap();
        let cwd = api.canonicalize().unwrap().to_string_lossy().into_owned();
        let flags = ApiFlags { cwd, pipe_path: String::new(), callbacks: Vec::new(), case_sensitive: true, is_async: true, timing: false, run_external_code: false };
        let (s, _) = new_api_session(&flags).unwrap();
        let r = match s.handle_request("createBuildOrchestrator", br#"{"rootNames":["app"]}"#).unwrap() {
            Response::Json(t) => t,
            _ => unreachable!(),
        };
        assert!(r.contains("\"buildOrchestratorID\":1"), "{r}");
        let b = match s.handle_request("build", br#"{"buildOrchestratorID":1}"#).unwrap() {
            Response::Json(t) => t,
            _ => unreachable!(),
        };
        assert!(b.contains(&format!("\"status\":{}", status.code().unwrap())), "cli exit {:?} vs api {b}", status.code());
        let mut files = Vec::new();
        for proj in ["core/out", "app/out"] {
            for e in std::fs::read_dir(cli.join(proj)).unwrap() {
                files.push(format!("{proj}/{}", e.unwrap().file_name().to_string_lossy()));
            }
        }
        files.sort();
        assert!(files.len() >= 8, "{files:?}");
        for f in &files {
            let a = std::fs::read(cli.join(f)).unwrap();
            let b = std::fs::read(api.join(f)).unwrap_or_else(|_| panic!("api build missing {f}"));
            assert!(a == b, "{f} differs between tsrs -b and the API build");
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Repeated rebuilds on one API build handle (an edit between builds forces a real program build).
    #[test]
    fn repeated_builds_memory() {
        let _serial = super::BUILD_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("tsrs-api-buildmem-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tsconfig.json"), r#"{ "compilerOptions": { "composite": true, "outDir": "out" } }"#).unwrap();
        let cwd = dir.canonicalize().unwrap().to_string_lossy().into_owned();
        let flags = ApiFlags { cwd, pipe_path: String::new(), callbacks: Vec::new(), case_sensitive: true, is_async: true, timing: false, run_external_code: false };
        let (s, _) = new_api_session(&flags).unwrap();
        let id = match s.handle_request("createBuildOrchestrator", br#"{"rootNames":["."]}"#).unwrap() {
            Response::Json(t) => {
                let v = json::unmarshal(&t).unwrap();
                let id = match &v {
                    Value::Object(o) => o.get("buildOrchestratorID").unwrap().clone(),
                    _ => unreachable!(),
                };
                json::marshal(&id).unwrap()
            }
            _ => unreachable!(),
        };
        let n: usize = std::env::var("N").ok().and_then(|v| v.parse().ok()).unwrap_or(40);
        let mut before = 0;
        for i in 0..n + 5 {
            if i == 5 {
                before = rss_kib();
            }
            std::fs::write(dir.join("a.ts"), format!("export const v{i}: number = {i};\n").repeat(50)).unwrap();
            s.handle_request("build", format!("{{\"buildOrchestratorID\":{id}}}").as_bytes()).unwrap();
        }
        let grown = rss_kib().saturating_sub(before);
        eprintln!("BUILDMEM x{n}: rss grew {grown} KiB");
        // Before per-build regions each rebuild retained ~19 MiB (lib files reparsed into never-freed arenas and
        // leaked texts); now one build's worth stays (kept for cleans) and older builds are freed.
        assert!(grown < (n as u64) * 512, "rss grew {grown} KiB over {n} builds");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
