// session.go handleCreateBuildOrchestrator / handleDisposeBuildOrchestrator / handleBuild /
// handleBuildReferences / handleCleanBuild / handleCleanReferences.
//
// The build orchestrator lives in tsrs_execute (crates/tsrs_execute/src/build). The CLI injects it with
// `Session::set_build_backend` (tsrs_cli's `CliBuildBackend`) so the API reuses it in-process instead of spawning
// `tsrs -b` per call; sessions without a backend report the build methods as unsupported.

use rustc_hash::FxHashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tsrs_ast::Diagnostic;
use tsrs_core::json::Value;
use tsrs_core::{BuildOptions, CompilerOptions, P};
use tsrs_tsoptions::gojson;
use tsrs_vfs::FS;

use crate::diagnostics::diagnostic_responses;
use crate::handler::{ApiError, ApiResult};
use crate::session::Session;
use crate::wire::{strings, Obj, Params};

/// Go `build.OrchestratorResult` fields the API reports.
#[derive(Default)]
pub struct BuildOutcome {
    pub status: i32,
    pub diagnostics: Vec<P<Diagnostic>>,
    pub projects: usize,
    pub projects_built: usize,
    pub timestamp_updates: usize,
    pub files_deleted: Vec<String>,
}

/// Inputs of Go `handleCreateBuildOrchestrator` (`apiBuildSystem` + `ParseBuildCommandLine`).
pub struct BuildRequest {
    pub fs: Arc<dyn FS>,
    pub default_library_path: String,
    pub current_directory: String,
    pub root_names: Vec<String>,
    pub build_options: Option<BuildOptions>,
    pub compiler_options: Option<CompilerOptions>,
}

pub trait BuildOrchestrator: Send {
    /// Go `Orchestrator.Build` (`only_references = false`) / `BuildReferences`.
    fn build(&mut self, project: &str, only_references: bool) -> BuildOutcome;
    /// Go `Orchestrator.Clean` / `CleanReferences`.
    fn clean(&mut self, project: &str, only_references: bool) -> BuildOutcome;
}

pub trait BuildBackend: Send + Sync {
    fn create(&self, request: BuildRequest) -> Box<dyn BuildOrchestrator>;
}

#[derive(Default)]
pub(crate) struct BuildState {
    next_id: AtomicU64,
    orchestrators: Mutex<FxHashMap<u64, Orchestrator>>,
}

struct Orchestrator {
    backend: Box<dyn BuildOrchestrator>,
}

fn outcome_response(o: BuildOutcome, clean: bool) -> Value {
    let stats = Obj::new()
        .set("Projects", Value::Number(o.projects as f64))
        .set("ProjectsBuilt", Value::Number(o.projects_built as f64))
        .set("TimestampUpdates", Value::Number(o.timestamp_updates as f64))
        .build();
    let mut r = Obj::new()
        .set("status", Value::Number(o.status as f64))
        .set_opt("diagnostics", (!o.diagnostics.is_empty()).then(|| diagnostic_responses(&o.diagnostics)))
        .set("statistics", stats);
    if clean {
        r = r.set_opt("filesDeleted", (!o.files_deleted.is_empty()).then(|| strings(o.files_deleted)));
    }
    r.build()
}

impl Session {
    pub(crate) fn handle_create_build_orchestrator(&self, p: Params) -> ApiResult<Value> {
        let backend = self.build_backend().ok_or_else(|| ApiError::unsupported("createBuildOrchestrator (no build backend in this session)"))?;
        let root_names = p.strings("rootNames")?;
        let cwd = match p.str("cwd")? {
            "" => self.current_directory().to_string(),
            c => c.to_string(),
        };
        let mut build_options = if p.has("buildOptions") { Some(gojson::build_options_from_go_json(p.get("buildOptions")).map_err(ApiError::invalid_request)?) } else { None };
        if let Some(options) = &mut build_options {
            crate::predecode::exact_build_options_ints(p.get("buildOptions"), options);
        }
        let mut compiler_options =
            if p.has("compilerOptions") { Some(gojson::compiler_options_from_go_json(p.get("compilerOptions")).map_err(ApiError::invalid_request)?) } else { None };
        if let Some(options) = &mut compiler_options {
            crate::predecode::exact_compiler_options_ints(p.get("compilerOptions"), options);
        }
        let orchestrator = backend.create(BuildRequest {
            fs: self.snapshot_host_fs(),
            default_library_path: self.default_library_path().to_string(),
            current_directory: cwd,
            root_names,
            build_options,
            compiler_options,
        });
        let id = self.build_state.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        // A build that failed by unwinding (the moduleResolution boundary above) leaves the map intact; its lock
        // is only poisoned.
        self.build_state.orchestrators.lock().unwrap_or_else(|e| e.into_inner()).insert(id, Orchestrator { backend: orchestrator });
        Ok(Obj::new().set("buildOrchestratorID", Value::Number(id as f64)).build())
    }

    pub(crate) fn handle_dispose_build_orchestrator(&self, p: Params) -> ApiResult<Value> {
        let id = p.u64("buildOrchestratorID")?;
        let removed = self.build_state.orchestrators.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        match removed {
            Some(_) => Ok(Value::Bool(true)),
            None => Err(ApiError::internal("build orchestrator not found while disposing")),
        }
    }

    /// Go holds `buildMu` for the whole build; builds on one session are serialized.
    pub(crate) fn handle_build(&self, p: Params, only_references: bool, clean: bool) -> ApiResult<Value> {
        let id = p.u64("buildOrchestratorID")?;
        let project = p.str("project")?;
        // Go holds `buildMu` for the whole build. A request re-entering from a client callback while a build
        // holds it gets a bounded error instead of a deadlock.
        let mut orchestrators = tsrs_api_transport::reentrancy::lock_for_request(&self.build_state.orchestrators, "the build orchestrator")
            .map_err(|e| ApiError::internal(e.message))?;
        let Some(o) = orchestrators.get_mut(&id) else {
            let what = match (clean, only_references) {
                (false, false) => format!("build orchestrator not found while building {project}"),
                (false, true) => format!("build orchestrator not found for building references for {project}"),
                (true, false) => format!("build orchestrator not found while cleaning {project}"),
                (true, true) => format!("build orchestrator not found while cleaning references for {project}"),
            };
            return Err(ApiError::internal(what));
        };
        // A moduleResolution number with no named kind fails (stable client error, see `Session::handle_request`)
        // only where a module is actually resolved, where pinned Go panics; import-free builds succeed as in Go.
        let outcome = if clean { o.backend.clean(project, only_references) } else { o.backend.build(project, only_references) };
        Ok(outcome_response(outcome, clean))
    }
}
