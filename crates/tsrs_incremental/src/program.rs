// Port of execute/incremental/program.go.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use rustc_hash::FxHashMap;
use tsrs_ast::{new_compiler_diagnostic, Diagnostic, SourceFile};
use tsrs_compiler::{filter_no_emit_semantic_diagnostics, Context, Program as CompilerProgram};
use tsrs_core::tspath::{self, ComparePathsOptions, Path};
use tsrs_core::{CompilerOptions, Tristate, P};
use tsrs_diagnostics as diagnostics;

use crate::affectedfileshandler::collect_all_affected_files;
use crate::emit::{handle_no_emit_options, EmitOnly, EmitOptions, EmitResult, ProgramLike, WriteFileData};
use crate::emitfileshandler::emit_files;
use crate::host::Host;
use crate::programtosnapshot::program_to_snapshot;
use crate::snapshot::{DiagnosticsOrBuildInfoDiagnosticsWithFileName, Snapshot};
use crate::snapshottobuildinfo::snapshot_to_build_info;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignatureUpdateKind {
    ComputedDts,
    StoredAtEmit,
    UsedVersion,
}

#[derive(Default)]
struct nestedEmitState {
    depth: i32,
    start: Option<Instant>,
    time: Duration,
}

pub struct Program {
    pub(crate) snapshot: P<Snapshot>,
    pub(crate) program: Option<&'static CompilerProgram>,
    pub(crate) host: Option<Box<dyn Host + Send + Sync>>,

    // Testing data
    pub(crate) testing_data: Option<Mutex<TestingData>>,

    nested_emit_now: Option<fn() -> Instant>,
    nested_emit: Mutex<nestedEmitState>,
}

// program.go:46
pub fn new_program(
    program: &'static CompilerProgram,
    old_program: Option<P<Program>>,
    host: Box<dyn Host + Send + Sync>,
    nested_emit_now: Option<fn() -> Instant>,
    testing: bool,
) -> P<Program> {
    let snapshot = program_to_snapshot(program, old_program, testing);
    let testing_data = if testing {
        let testing_data = TestingData { old_snapshot: old_program.map(|p| p.snapshot), updated_signature_kinds: FxHashMap::default() };
        Some(Mutex::new(testing_data))
    } else {
        None
    };
    P::new(Program {
        snapshot,
        program: Some(program),
        host: Some(host),
        testing_data,
        nested_emit_now,
        nested_emit: Mutex::new(nestedEmitState::default()),
    })
}

// Go's TestingData holds pointers to the program's and the old program's semantic diagnostics maps; Rust keeps the
// old snapshot and answers the questions the harness asks (Program::testing_semantic_diagnostics_state).
pub struct TestingData {
    old_snapshot: Option<P<Snapshot>>,
    pub updated_signature_kinds: FxHashMap<Path, SignatureUpdateKind>,
}

// The tsctests "SemanticDiagnostics::" line of a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemanticDiagnosticsState {
    NotCached,
    Refreshed,
    Reused,
}

impl Program {
    pub(crate) fn new_from_snapshot(snapshot: P<Snapshot>) -> P<Program> {
        P::new(Program { snapshot, program: None, host: None, testing_data: None, nested_emit_now: None, nested_emit: Mutex::new(nestedEmitState::default()) })
    }

    pub fn get_testing_data(&self) -> Option<&Mutex<TestingData>> {
        self.testing_data.as_ref()
    }

    // sys.go OnProgram: `*refresh*` when the cached entry is not the one the old program had (by pointer).
    pub fn testing_semantic_diagnostics_state(&self, path: &Path) -> SemanticDiagnosticsState {
        let Some(diagnostics) = self.snapshot.semantic_diagnostics_per_file.load(path) else {
            return SemanticDiagnosticsState::NotCached;
        };
        let testing_data = self.testing_data.as_ref().unwrap().lock().unwrap();
        let old = testing_data.old_snapshot.and_then(|s| s.semantic_diagnostics_per_file.load(path));
        match old {
            Some(old) if std::sync::Arc::ptr_eq(&old, &diagnostics) => SemanticDiagnosticsState::Reused,
            _ => SemanticDiagnosticsState::Refreshed,
        }
    }

    pub fn testing_updated_signature_kind(&self, path: &Path) -> Option<SignatureUpdateKind> {
        self.testing_data.as_ref().unwrap().lock().unwrap().updated_signature_kinds.get(path).copied()
    }

    // program.go:82
    pub(crate) fn begin_nested_emit(&self) -> impl FnOnce() + '_ {
        let now = self.nested_emit_now;
        if let Some(now) = now {
            let mut state = self.nested_emit.lock().unwrap();
            if state.depth == 0 {
                state.start = Some(now());
            }
            state.depth += 1;
        }
        move || {
            if let Some(now) = now {
                let mut state = self.nested_emit.lock().unwrap();
                state.depth -= 1;
                if state.depth == 0 {
                    let elapsed = now() - state.start.unwrap();
                    state.time += elapsed;
                }
            }
        }
    }

    // program.go:105
    pub fn take_nested_emit_time(&self) -> Duration {
        let mut state = self.nested_emit.lock().unwrap();
        std::mem::take(&mut state.time)
    }

    // program.go:113
    fn panic_if_no_program(&self, method: &str) {
        if self.program.is_none() {
            panic!("{method}: should not be called without program");
        }
    }

    // program.go:119
    pub fn get_program(&self) -> &'static CompilerProgram {
        self.panic_if_no_program("GetProgram");
        self.program.unwrap()
    }

    // program.go:124
    pub fn has_changed_dts_file(&self) -> bool {
        self.snapshot.has_changed_dts_file.get()
    }

    pub(crate) fn p(&self) -> &'static CompilerProgram {
        self.program.unwrap()
    }

    // program.go:160
    fn get_semantic_diagnostics_impl(&self, ctx: &Context, file: Option<P<SourceFile>>) -> Vec<P<Diagnostic>> {
        self.panic_if_no_program("GetSemanticDiagnostics");
        if self.snapshot.options().no_check.is_true() {
            return Vec::new();
        }

        // Ensure all the diagnsotics are cached
        self.collect_semantic_diagnostics_of_affected_files(ctx, file);
        if ctx.err().is_some() {
            return Vec::new();
        }

        // Return result from cache
        if let Some(file) = file {
            return self.get_semantic_diagnostics_of_file(file);
        }

        let mut diagnostics = Vec::new();
        for &file in self.p().get_source_files() {
            diagnostics.extend(self.get_semantic_diagnostics_of_file(file));
        }
        diagnostics
    }

    // program.go:186
    fn get_semantic_diagnostics_of_file(&self, file: P<SourceFile>) -> Vec<P<Diagnostic>> {
        let Some(cached_diagnostics) = self.snapshot.semantic_diagnostics_per_file.load(file.path()) else {
            panic!("After handling all the affected files, there shouldnt be more changes");
        };
        let mut result = filter_no_emit_semantic_diagnostics(cached_diagnostics.get_diagnostics(self.p(), Some(file)), &self.snapshot.options());
        result.extend(self.p().get_include_processor_diagnostics(file));
        result
    }

    // Handle affected files and cache the semantic diagnostics for all of them or the file asked for
    // program.go:256
    fn collect_semantic_diagnostics_of_affected_files(&self, ctx: &Context, file: Option<P<SourceFile>>) {
        if self.snapshot.can_use_incremental_state() {
            // Get all affected files
            collect_all_affected_files(ctx, self);
            if ctx.err().is_some() {
                return;
            }

            if self.snapshot.semantic_diagnostics_per_file.size() == self.p().get_source_files().len() {
                // If we have all the files,
                return;
            }
        }

        let affected_files: Vec<P<SourceFile>> = match file {
            Some(file) => {
                if self.snapshot.semantic_diagnostics_per_file.load(file.path()).is_some() {
                    return;
                }
                vec![file]
            }
            None => self
                .p()
                .get_source_files()
                .iter()
                .copied()
                .filter(|file| self.snapshot.semantic_diagnostics_per_file.load(file.path()).is_none())
                .collect(),
        };

        // Get their diagnostics and cache them
        let diagnostics_per_file = self.p().get_semantic_diagnostics_for_incremental(ctx, &affected_files);
        // commit changes if no err
        if ctx.err().is_some() {
            return;
        }

        // Commit changes to snapshot
        for (file, diagnostics) in diagnostics_per_file {
            self.snapshot.semantic_diagnostics_per_file.store(file.path().clone(), DiagnosticsOrBuildInfoDiagnosticsWithFileName::from_diagnostics(diagnostics));
        }
        if self.snapshot.semantic_diagnostics_per_file.size() == self.p().get_source_files().len()
            && self.snapshot.check_pending.get()
            && !self.snapshot.options().no_check.is_true()
        {
            self.snapshot.check_pending.set(false);
        }
        self.snapshot.build_info_emit_pending.set(true);
    }

    // program.go:304
    pub(crate) fn emit_build_info(&self, ctx: &Context, options: &EmitOptions) -> Option<EmitResult> {
        let program = self.p();
        let build_info_file_name = tsrs_tsoptions::outputpaths::get_build_info_file_name(
            &self.snapshot.options(),
            &ComparePathsOptions {
                current_directory: program.get_current_directory().to_string(),
                use_case_sensitive_file_names: program.use_case_sensitive_file_names(),
            },
        );
        if build_info_file_name.is_empty() || program.is_emit_blocked(&build_info_file_name) {
            return None;
        }
        if self.snapshot.has_errors.get() == Tristate::Unknown {
            tsrs_core::phases::time("BuildInfo: has errors", || self.ensure_has_errors_for_state(ctx, program));
            if self.snapshot.has_errors.get() != self.snapshot.has_errors_from_old_state.get()
                || self.snapshot.has_semantic_errors.get() != self.snapshot.has_semantic_errors_from_old_state.get()
            {
                self.snapshot.build_info_emit_pending.set(true);
            }
        }
        if self.snapshot.package_jsons.borrow().is_none() {
            tsrs_core::phases::time("BuildInfo: package.jsons", || self.ensure_package_jsons_for_state());
            if self.snapshot.package_jsons.borrow().as_deref().unwrap_or_default()
                != self.snapshot.package_jsons_from_old_state.borrow().as_deref().unwrap_or_default()
                || self.snapshot.missing_package_jsons.borrow().as_deref().unwrap_or_default()
                    != self.snapshot.missing_package_jsons_from_old_state.borrow().as_deref().unwrap_or_default()
            {
                self.snapshot.build_info_emit_pending.set(true);
            }
        }
        if !self.snapshot.build_info_emit_pending.get() {
            return None;
        }
        if ctx.err().is_some() {
            return None;
        }
        let build_info = match tsrs_core::phases::time("BuildInfo: from snapshot", || snapshot_to_build_info(&self.snapshot, program, &build_info_file_name)) {
            Ok(build_info) => build_info,
            Err(err) => {
                // Go: compiler.ContentMapperProjectDiagnostic(err); content mappers are not supported, so this never fails.
                return Some(EmitResult { emit_skipped: true, diagnostics: vec![new_compiler_diagnostic(&diagnostics::Could_not_write_file_0_Colon_1, &[&build_info_file_name, &err])], ..Default::default() });
            }
        };
        let text = tsrs_core::phases::time("BuildInfo: marshal", || build_info.marshal());
        let write_start = std::time::Instant::now();
        let err = match options.write_file {
            Some(write_file) => {
                let mut data = WriteFileData { build_info: Some(std::sync::Arc::new(build_info)), ..Default::default() };
                write_file(&build_info_file_name, &text, &mut data)
            }
            None => program.host().fs().write_file(&build_info_file_name, &text),
        };
        tsrs_core::phases::record("BuildInfo: write", write_start.elapsed());
        if let Err(err) = err {
            return Some(EmitResult {
                emit_skipped: true,
                diagnostics: vec![new_compiler_diagnostic(&diagnostics::Could_not_write_file_0_Colon_1, &[&build_info_file_name, &err])],
                ..Default::default()
            });
        }
        self.snapshot.build_info_emit_pending.set(false);
        Some(EmitResult { emit_skipped: false, emitted_files: vec![build_info_file_name], ..Default::default() })
    }

    // program.go:368
    fn ensure_has_errors_for_state(&self, ctx: &Context, program: &'static CompilerProgram) {
        let mut has_include_processing_diagnostics: Option<bool> = None;
        let mut has_emit_diagnostics = false;
        let snapshot = &self.snapshot;
        let compute_has_include_processing_diagnostics: Box<dyn Fn() -> bool> = if snapshot.can_use_incremental_state() {
            if program.get_source_files().iter().any(|&file| {
                if snapshot.emit_diagnostics_per_file.load(file.path()).is_some() {
                    // emit diagnostics will be encoded in buildInfo;
                    return true;
                }
                if has_include_processing_diagnostics.is_none() && !program.get_include_processor_diagnostics(file).is_empty() {
                    has_include_processing_diagnostics = Some(true);
                }
                false
            }) {
                has_emit_diagnostics = true;
            }
            let value = has_include_processing_diagnostics.unwrap_or(false);
            Box::new(move || value)
        } else {
            has_emit_diagnostics = snapshot.has_emit_diagnostics.get();
            Box::new(move || program.get_source_files().iter().any(|&file| !program.get_include_processor_diagnostics(file).is_empty()))
        };

        if has_emit_diagnostics {
            // Record this for only non incremental build info
            snapshot.has_errors.set(if snapshot.options().is_incremental() { Tristate::False } else { Tristate::True });
            // Dont need to encode semantic errors state since the emit diagnostics are encoded
            snapshot.has_semantic_errors.set(false);
            return;
        }

        if compute_has_include_processing_diagnostics()
            || !program.get_config_file_parsing_diagnostics().is_empty()
            || !program.get_syntactic_diagnostics(ctx, None).is_empty()
            || !program.get_program_diagnostics().is_empty()
            || !program.get_global_diagnostics(ctx).is_empty()
        {
            snapshot.has_errors.set(Tristate::True);
            // Dont need to encode semantic errors state since the syntax and program diagnostics are encoded as present
            snapshot.has_semantic_errors.set(false);
            return;
        }

        snapshot.has_errors.set(Tristate::False);
        // Check semantic and emit diagnostics first as we dont need to ask program about it
        if program.get_source_files().iter().any(|&file| {
            let Some(semantic_diagnostics) = snapshot.semantic_diagnostics_per_file.load(file.path()) else {
                // Missing semantic diagnostics in cache will be encoded in incremental buildInfo
                return snapshot.options().is_incremental();
            };
            // cached semantic diagnostics will be encoded in buildInfo
            semantic_diagnostics.has_diagnostics() || !semantic_diagnostics.build_info_diagnostics.is_empty()
        }) {
            // Because semantic diagnostics are recorded in buildInfo, we dont need to encode hasErrors in incremental buildInfo
            // But encode as errors in non incremental buildInfo
            snapshot.has_semantic_errors.set(!snapshot.options().is_incremental());
        }
    }

    // program.go:443
    fn ensure_package_jsons_for_state(&self) {
        let program = self.p();
        let mut package_jsons: Vec<String> = Vec::new();
        let mut missing_package_jsons: Vec<String> = Vec::new();
        let config = tspath::get_directory_path(program.command_line().config_name());
        if !config.is_empty() {
            program.package_json_cache_entries(|_key, value| {
                let mut package_json = tspath::combine_paths(&value.package_directory, &["package.json"]);
                if value.exists() || value.directory_exists {
                    package_json = program.host().fs().realpath(&package_json);
                }
                if value.exists() {
                    package_jsons.push(package_json);
                } else if package_json.contains("/node_modules/") {
                    missing_package_jsons.push(package_json);
                }
                true
            });
        }
        *self.snapshot.package_jsons.borrow_mut() = Some(normalize_package_jsons(package_jsons));
        *self.snapshot.missing_package_jsons.borrow_mut() = Some(normalize_package_jsons(missing_package_jsons));
    }

    // program.go:478
    pub fn package_json_lookup_paths(&self) -> Vec<String> {
        let program = self.p();
        let config = tspath::get_directory_path(program.command_line().config_name());
        if config.is_empty() {
            return Vec::new();
        }

        let mut package_jsons = Vec::new();
        program.package_json_cache_entries(|_key, value| {
            let mut package_json = tspath::combine_paths(&value.package_directory, &["package.json"]);
            if value.exists() || value.directory_exists {
                package_json = program.host().fs().realpath(&package_json);
            }
            package_jsons.push(package_json);
            true
        });
        package_jsons.sort();
        package_jsons.dedup();
        package_jsons
    }
}

// program.go:470
fn normalize_package_jsons(mut package_jsons: Vec<String>) -> Vec<String> {
    package_jsons.sort();
    package_jsons.dedup();
    package_jsons
}

// `&'static Program` (not `P<Program>`: the orphan rule forbids a foreign trait on the foreign `P`).
impl ProgramLike for &'static Program {
    // Options implements compiler.AnyProgram interface.
    fn options(&self) -> P<CompilerOptions> {
        self.snapshot.options()
    }

    // GetSourceFile implements compiler.AnyProgram interface.
    fn get_source_file(&self, path: &str) -> Option<P<SourceFile>> {
        self.panic_if_no_program("GetSourceFile");
        self.p().get_source_file(path)
    }

    // GetSourceFiles implements compiler.AnyProgram interface.
    fn get_source_files(&self) -> &'static [P<SourceFile>] {
        self.panic_if_no_program("GetSourceFiles");
        self.p().get_source_files()
    }

    // GetConfigFileParsingDiagnostics implements compiler.AnyProgram interface.
    fn get_config_file_parsing_diagnostics(&self) -> Vec<P<Diagnostic>> {
        self.panic_if_no_program("GetConfigFileParsingDiagnostics");
        self.p().get_config_file_parsing_diagnostics()
    }

    // GetSyntacticDiagnostics implements compiler.AnyProgram interface.
    fn get_syntactic_diagnostics(&self, ctx: &Context, file: Option<P<SourceFile>>) -> Vec<P<Diagnostic>> {
        self.panic_if_no_program("GetSyntacticDiagnostics");
        self.p().get_syntactic_diagnostics(ctx, file)
    }

    // GetBindDiagnostics implements compiler.AnyProgram interface.
    fn get_bind_diagnostics(&self, ctx: &Context, file: Option<P<SourceFile>>) -> Vec<P<Diagnostic>> {
        self.panic_if_no_program("GetBindDiagnostics");
        self.p().get_bind_diagnostics(ctx, file)
    }

    fn get_program_diagnostics(&self) -> Vec<P<Diagnostic>> {
        self.panic_if_no_program("GetProgramDiagnostics");
        self.p().get_program_diagnostics()
    }

    fn get_global_diagnostics(&self, ctx: &Context) -> Vec<P<Diagnostic>> {
        self.panic_if_no_program("GetGlobalDiagnostics");
        self.p().get_global_diagnostics(ctx)
    }

    // GetSemanticDiagnostics implements compiler.AnyProgram interface.
    fn get_semantic_diagnostics(&self, ctx: &Context, file: Option<P<SourceFile>>) -> Vec<P<Diagnostic>> {
        self.get_semantic_diagnostics_impl(ctx, file)
    }

    // GetDeclarationDiagnostics implements compiler.AnyProgram interface.
    fn get_declaration_diagnostics(&self, ctx: &Context, file: Option<P<SourceFile>>) -> Vec<P<Diagnostic>> {
        self.panic_if_no_program("GetDeclarationDiagnostics");
        let result = emit_files(ctx, P::from_static(*self), EmitOptions { target_source_files: file.map(|f| vec![f]), ..Default::default() }, true);
        match result {
            Some(result) => result.diagnostics,
            None => Vec::new(),
        }
    }

    // GetSuggestionDiagnostics implements compiler.AnyProgram interface.
    fn get_suggestion_diagnostics(&self, ctx: &Context, file: Option<P<SourceFile>>) -> Vec<P<Diagnostic>> {
        self.panic_if_no_program("GetSuggestionDiagnostics");
        self.p().get_suggestion_diagnostics(ctx, file) // TODO: incremental suggestion diagnostics (only relevant in editor incremental builder?)
    }

    // program.go:221
    fn emit(&self, ctx: &Context, options: EmitOptions) -> Option<EmitResult> {
        self.panic_if_no_program("Emit");

        let mut result = None;
        if !options.force_emit && options.emit_only != EmitOnly::EmitOnlyBuilderSignature {
            let emit_build_info = || self.emit_build_info(ctx, &options);
            let emit_build_info: Option<&dyn Fn() -> Option<EmitResult>> = if self.options().no_emit.is_true() { Some(&emit_build_info) } else { None };
            result = handle_no_emit_options(ctx, self, options.target_source_files.as_deref(), emit_build_info);
            if ctx.err().is_some() {
                return None;
            }
        }
        if let Some(mut result) = result {
            if options.target_source_files.is_some() || self.options().no_emit.is_true() {
                return Some(result);
            }

            // Emit buildInfo and combine result
            if let Some(build_info_result) = self.emit_build_info(ctx, &options) {
                result.diagnostics.extend(build_info_result.diagnostics);
                result.emitted_files.extend(build_info_result.emitted_files);
            }
            return Some(result);
        }
        emit_files(ctx, P::from_static(*self), options, false)
    }

    // CommonSourceDirectory implements compiler.AnyProgram interface.
    fn common_source_directory(&self) -> String {
        self.panic_if_no_program("CommonSourceDirectory");
        self.p().common_source_directory().to_string()
    }

    // IsSourceFileDefaultLibrary implements compiler.AnyProgram interface.
    fn is_source_file_default_library(&self, path: &Path) -> bool {
        self.panic_if_no_program("IsSourceFileDefaultLibrary");
        self.p().is_source_file_default_library(path)
    }

    // Program implements compiler.AnyProgram interface.
    fn program(&self) -> &'static CompilerProgram {
        self.panic_if_no_program("Program");
        self.p()
    }

    fn is_compiler_program(&self) -> bool {
        false
    }
}
