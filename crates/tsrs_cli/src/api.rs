// Port of cmd/tsc/api.go (`tsrs --api`): flag parsing, the session filesystem (bundled libs over the OS
// filesystem, optionally behind the client callback filesystem), the in-process build backend for the
// API's build orchestrator methods, and the connection to the wire runtime.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tsrs_api::build::{BuildBackend, BuildOrchestrator, BuildOutcome, BuildRequest};
use tsrs_api::callbackfs::CallbackFs;
use tsrs_api::{Session, SessionOptions};
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

/// Go `StdioServer.Run` setup: the session and, when callbacks are enabled, the callback filesystem that must
/// be connected to the client once the transport is up.
pub(crate) fn new_api_session(flags: &ApiFlags) -> Result<(Arc<Session>, Option<Arc<CallbackFs>>), String> {
    let base: Arc<dyn FS> = Arc::new(bundled::wrap_fs(osvfs::fs()));
    // Go wraps whenever callbacks are requested or case sensitivity is given; the CLI always passes the latter.
    let callback_fs = Arc::new(CallbackFs::new(base, &flags.callbacks, Some(flags.case_sensitive))?);
    let fs: Arc<dyn FS> = callback_fs.clone();
    let mut options = SessionOptions::new(flags.cwd.clone(), bundled::lib_path(), fs, !flags.is_async);
    options.run_external_code = flags.run_external_code;
    let session = Session::new(options);
    session.set_build_backend(Arc::new(CliBuildBackend));
    Ok((session, Some(callback_fs)))
}

/// `tsrs --api`.
pub fn run_api(args: &[String]) -> i32 {
    let Ok(flags) = parse_api_flags(args) else { return 2 };
    let (session, _callback_fs) = match new_api_session(&flags) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("{err}");
            return 2;
        }
    };
    // The wire runtime (crates/tsrs_api_transport, runtime lane) is not integrated on this branch yet. Fail
    // explicitly instead of falling through to the compiler command line or speaking a different protocol.
    let _ = (flags.pipe_path, flags.timing, session);
    eprintln!("tsrs --api: the wire transport is not integrated in this build yet (see docs/NODE_API.md)");
    1
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

/// One API build orchestrator. Go keeps one `build.Orchestrator` and rechecks all projects on every call
/// (`recheckAllProjects` resets statuses, configs, mtimes and caches); this port builds a fresh orchestrator
/// per call from the same parsed command line, which re-reads the same state from disk.
struct CliOrchestrator {
    sys: &'static ApiBuildSystem,
    command: P<tsrs_tsoptions::ParsedBuildCommandLine>,
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
        Box::new(CliOrchestrator { sys, command: P::new(command) })
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
        outcome(orchestrator.build_for_api(project, only_references))
    }
    fn clean(&mut self, project: &str, only_references: bool) -> BuildOutcome {
        let orchestrator = new_orchestrator(Options { sys: self.sys, command: self.command, testing: None });
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
