// Port of execute/incremental/snapshottobuildinfo.go.

use rustc_hash::FxHashMap;
use tsrs_ast::{self as ast, Diagnostic, RepopulateDiagnosticInfo, SourceFile};
use tsrs_compiler::Program as CompilerProgram;
use tsrs_core::collections::{OrderedMap, Set};
use tsrs_core::json::Value;
use tsrs_core::tspath::{self, ComparePathsOptions, Path};
use tsrs_core::P;
use tsrs_tsoptions::{self as tsoptions, CommandLineOption, CommandLineOptionKind, CompilerOptionsValue};

use crate::buildinfo::*;
use crate::snapshot::{buildInfoDiagnosticWithFileName, get_file_emit_kind, DiagnosticsOrBuildInfoDiagnosticsWithFileName, Snapshot};

// snapshottobuildinfo.go:18
pub(crate) fn snapshot_to_build_info(snapshot: &Snapshot, program: &'static CompilerProgram, build_info_file_name: &str) -> Result<BuildInfo, String> {
    let content_mapper_identities = content_mapper_identities()?;
    let mut build_info = BuildInfo { version: tsrs_core::version().to_string(), content_mapper_identities, ..Default::default() };
    let mut to = toBuildInfo {
        snapshot,
        program,
        build_info: &mut build_info,
        build_info_directory: tspath::get_directory_path(build_info_file_name),
        compare_paths_options: ComparePathsOptions {
            current_directory: program.get_current_directory().to_string(),
            use_case_sensitive_file_names: program.use_case_sensitive_file_names(),
        },
        file_name_to_file_id: FxHashMap::default(),
        file_names_to_file_id_list_id: FxHashMap::default(),
        roots: FxHashMap::default(),
    };

    if snapshot.options().is_incremental() {
        to.collect_root_files();
        to.set_file_info_and_emit_signatures();
        to.set_root_of_incremental_program();
        to.set_compiler_options();
        to.set_referenced_map();
        to.set_change_file_set();
        to.set_semantic_diagnostics();
        to.set_emit_diagnostics();
        to.set_affected_files_pending_emit();
        let latest_changed_dts_file = snapshot.latest_changed_dts_file.borrow().clone();
        if !latest_changed_dts_file.is_empty() {
            to.build_info.latest_changed_dts_file = to.relative_to_build_info(&latest_changed_dts_file);
        }
    } else {
        to.set_root_of_non_incremental_program();
    }
    to.build_info.errors = snapshot.has_errors.get().is_true();
    to.build_info.semantic_errors = snapshot.has_semantic_errors.get();
    to.build_info.check_pending = snapshot.check_pending.get();
    to.set_package_jsons();
    Ok(build_info)
}

struct toBuildInfo<'a> {
    snapshot: &'a Snapshot,
    program: &'static CompilerProgram,
    build_info: &'a mut BuildInfo,
    build_info_directory: String,
    compare_paths_options: ComparePathsOptions,
    file_name_to_file_id: FxHashMap<String, BuildInfoFileId>,
    file_names_to_file_id_list_id: FxHashMap<String, BuildInfoFileIdListId>,
    roots: FxHashMap<P<SourceFile>, Path>,
}

impl toBuildInfo<'_> {
    // snapshottobuildinfo.go:73
    fn relative_to_build_info(&self, path: &str) -> String {
        tspath::ensure_path_is_non_module_name(&tspath::get_relative_path_from_directory(&self.build_info_directory, path, &self.compare_paths_options))
    }

    // snapshottobuildinfo.go:77
    fn to_file_id(&mut self, path: &Path) -> BuildInfoFileId {
        let mut file_id = self.file_name_to_file_id.get(path.as_str()).copied().unwrap_or(0);
        if file_id == 0 {
            match self.program.get_default_lib_file(path) {
                Some(lib_file) if !lib_file.replaced => self.build_info.file_names.push(lib_file.name.clone()),
                _ => {
                    let name = self.relative_to_build_info(path);
                    self.build_info.file_names.push(name);
                }
            }
            file_id = self.build_info.file_names.len() as BuildInfoFileId;
            self.file_name_to_file_id.insert(path.to_string(), file_id);
        }
        file_id
    }

    // snapshottobuildinfo.go:91
    fn to_file_id_list_id(&mut self, set: &Set<Path>) -> BuildInfoFileIdListId {
        // Go iterates the set's keys in random order; the ids are sorted right after.
        let mut keys: Vec<&Path> = set.keys().iter().collect();
        keys.sort();
        let mut file_ids: Vec<BuildInfoFileId> = keys.into_iter().map(|p| self.to_file_id(p)).collect();
        file_ids.sort();
        let key = file_ids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");

        let mut file_id_list_id = self.file_names_to_file_id_list_id.get(&key).copied().unwrap_or(0);
        if file_id_list_id == 0 {
            self.build_info.file_ids_list.push(file_ids);
            file_id_list_id = self.build_info.file_ids_list.len() as BuildInfoFileIdListId;
            self.file_names_to_file_id_list_id.insert(key, file_id_list_id);
        }
        file_id_list_id
    }

    // snapshottobuildinfo.go:108
    fn to_relative_to_build_info_compiler_option_value(&self, option: &CommandLineOption, v: CompilerOptionsValue) -> CompilerOptionsValue {
        if option.kind == CommandLineOptionKind::List {
            if option.elements().unwrap().is_file_path {
                if let CompilerOptionsValue::StringArray(arr) = &v {
                    return CompilerOptionsValue::StringArray(arr.iter().map(|s| self.relative_to_build_info(s)).collect());
                }
            }
        } else if option.is_file_path {
            if let CompilerOptionsValue::String(s) = &v {
                if !s.is_empty() {
                    return CompilerOptionsValue::String(self.relative_to_build_info(s));
                }
            }
        }
        v
    }

    // snapshottobuildinfo.go:123
    fn to_build_info_diagnostics_from_file_name_diagnostics(&mut self, diagnostics: &[buildInfoDiagnosticWithFileName]) -> Vec<BuildInfoDiagnostic> {
        diagnostics
            .iter()
            .map(|d| {
                let file = if !d.file.is_empty() { self.to_file_id(&d.file) } else { 0 };
                BuildInfoDiagnostic {
                    file,
                    no_file: d.no_file,
                    pos: d.pos,
                    end: d.end,
                    code: d.code,
                    category: d.category,
                    source: d.source.clone(),
                    message_text: d.message_text.clone(),
                    message_key: d.message_key.clone(),
                    message_args: d.message_args.clone(),
                    message_chain: self.to_build_info_diagnostics_from_file_name_diagnostics(&d.message_chain),
                    related_information: self.to_build_info_diagnostics_from_file_name_diagnostics(&d.related_information),
                    reports_unnecessary: d.reports_unnecessary,
                    reports_deprecated: d.reports_deprecated,
                    skipped_on_no_emit: d.skipped_on_no_emit,
                    repopulate_info: to_build_info_repopulate_info(d.repopulate_info.as_ref()),
                }
            })
            .collect()
    }

    // snapshottobuildinfo.go:150
    fn to_build_info_diagnostics_from_diagnostics(&mut self, file_path: &Path, diagnostics: &[P<Diagnostic>]) -> Vec<BuildInfoDiagnostic> {
        diagnostics
            .iter()
            .map(|d| {
                let mut file = 0;
                let mut no_file = false;
                match d.file() {
                    None => no_file = true,
                    Some(f) if f.path().clone() != *file_path => file = self.to_file_id(f.path()),
                    _ => {}
                }
                BuildInfoDiagnostic {
                    file,
                    no_file,
                    pos: d.loc().pos(),
                    end: d.loc().end(),
                    code: d.code(),
                    category: d.category(),
                    source: d.source().to_string(),
                    message_text: d.message_text().to_string(),
                    message_key: d.message_key().0.to_string(),
                    message_args: d.message_args().to_vec(),
                    message_chain: self.to_build_info_diagnostics_from_diagnostics(file_path, d.message_chain()),
                    related_information: self.to_build_info_diagnostics_from_diagnostics(file_path, d.related_information()),
                    reports_unnecessary: d.reports_unnecessary(),
                    reports_deprecated: d.reports_deprecated(),
                    skipped_on_no_emit: d.skipped_on_no_emit(),
                    repopulate_info: to_build_info_repopulate_info(d.repopulate_info()),
                }
            })
            .collect()
    }

    // snapshottobuildinfo.go:195
    fn to_build_info_diagnostics_of_file(&mut self, file_path: &Path, diags: &DiagnosticsOrBuildInfoDiagnosticsWithFileName) -> Option<BuildInfoDiagnosticsOfFile> {
        let diagnostics = diags.diagnostics().unwrap_or_default();
        if !diagnostics.is_empty() {
            return Some(BuildInfoDiagnosticsOfFile {
                file_id: self.to_file_id(file_path),
                diagnostics: self.to_build_info_diagnostics_from_diagnostics(file_path, &diagnostics),
            });
        }
        if !diags.build_info_diagnostics.is_empty() {
            return Some(BuildInfoDiagnosticsOfFile {
                file_id: self.to_file_id(file_path),
                diagnostics: self.to_build_info_diagnostics_from_file_name_diagnostics(&diags.build_info_diagnostics),
            });
        }
        None
    }

    // snapshottobuildinfo.go:211
    fn collect_root_files(&mut self) {
        let program = self.program;
        for file_name in program.command_line().file_names() {
            let redirect = program.get_parse_file_redirect(file_name);
            let file = if !redirect.is_empty() { program.get_source_file(&redirect) } else { program.get_source_file(file_name) };
            if let Some(file) = file {
                self.roots.insert(
                    file,
                    tspath::to_path(file_name, &self.compare_paths_options.current_directory, self.compare_paths_options.use_case_sensitive_file_names),
                );
            }
        }
    }

    // snapshottobuildinfo.go:225
    fn set_file_info_and_emit_signatures(&mut self) {
        let program = self.program;
        let snapshot = self.snapshot;
        let mut file_infos = Vec::with_capacity(program.get_source_files().len());
        for &file in program.get_source_files() {
            let path = file.path().clone();
            let info = snapshot.file_infos.load(&path).unwrap();
            let file_id = self.to_file_id(&path);
            //  tryAddRoot(key, fileId);
            if self.build_info.file_names[file_id as usize - 1] != self.relative_to_build_info(&path) {
                match program.get_default_lib_file(&path) {
                    Some(lib_file) if !lib_file.replaced && self.build_info.file_names[file_id as usize - 1] == lib_file.name => {}
                    _ => panic!(
                        "File name at index {} does not match expected relative path or libName: {} != {}",
                        file_id - 1,
                        self.build_info.file_names[file_id as usize - 1],
                        self.relative_to_build_info(&path)
                    ),
                }
            }
            if snapshot.options().composite.is_true() && !ast::is_json_source_file(file) && program.source_file_may_be_emitted(file, false) {
                match snapshot.emit_signatures.load(&path) {
                    None => self.build_info.emit_signatures.push(BuildInfoEmitSignature { file_id, ..Default::default() }),
                    Some(emit_signature) if emit_signature.signature != info.signature => {
                        let mut incremental_emit_signature = BuildInfoEmitSignature { file_id, ..Default::default() };
                        if !emit_signature.signature.is_empty() {
                            incremental_emit_signature.signature = emit_signature.signature.clone();
                        } else if emit_signature.signature_with_different_options.as_ref().unwrap()[0] == info.signature {
                            incremental_emit_signature.differs_only_in_dts_map = true;
                        } else {
                            incremental_emit_signature.signature = emit_signature.signature_with_different_options.as_ref().unwrap()[0].clone();
                            incremental_emit_signature.differs_in_options = true;
                        }
                        self.build_info.emit_signatures.push(incremental_emit_signature);
                    }
                    Some(_) => {}
                }
            }
            file_infos.push(new_build_info_file_info(&info));
        }
        self.build_info.file_infos = file_infos;
        self.build_info.file_infos_non_nil = true;
    }

    // snapshottobuildinfo.go:256
    fn set_root_of_incremental_program(&mut self) {
        let mut keys: Vec<P<SourceFile>> = self.roots.keys().copied().collect();
        let mut ids: FxHashMap<P<SourceFile>, BuildInfoFileId> = FxHashMap::default();
        for &k in &keys {
            let id = self.to_file_id(k.path());
            ids.insert(k, id);
        }
        keys.sort_by_key(|k| ids[k]);
        for file in keys {
            let root_path = self.roots[&file].clone();
            let root = self.to_file_id(&root_path);
            let resolved = self.to_file_id(file.path());
            if self.build_info.root.is_empty() {
                // First fileId as is
                self.build_info.root.push(BuildInfoRoot { start: resolved, ..Default::default() });
            } else {
                let last = self.build_info.root.last_mut().unwrap();
                if last.end == resolved - 1 {
                    // If its [..., last = [start, end = fileId - 1]], update last to [start, fileId]
                    last.end = resolved;
                } else if last.end == 0 && last.start == resolved - 1 {
                    // If its [..., last = start = fileId - 1 ], update last to [start, fileId]
                    last.end = resolved;
                } else {
                    self.build_info.root.push(BuildInfoRoot { start: resolved, ..Default::default() });
                }
            }
            if root != resolved {
                self.build_info.resolved_root.push(BuildInfoResolvedRoot { resolved, root });
            }
        }
    }

    // snapshottobuildinfo.go:287
    fn set_compiler_options(&mut self) {
        let options = self.snapshot.options();
        let mut entries: Vec<(&'static str, CompilerOptionsValue)> = Vec::new();
        tsoptions::for_each_compiler_option_value(
            &options,
            |option| option.affects_build_info,
            |option, value, _i| {
                if value.is_zero() {
                    return false;
                }
                entries.push((option.name, value.interface()));
                false
            },
        );
        for (name, value) in entries {
            let option = tsoptions::COMMAND_LINE_COMPILER_OPTIONS_MAP.get(name).unwrap();
            // Make it relative to buildInfo directory if file path
            let value = self.to_relative_to_build_info_compiler_option_value(option, value);
            self.build_info.options.get_or_insert_with(OrderedMap::default).insert(name.to_string(), option_value_to_json(&value));
        }
    }

    // snapshottobuildinfo.go:307
    fn set_referenced_map(&mut self) {
        let mut keys = self.snapshot.referenced_map.get_paths_with_references();
        keys.sort();
        let mut referenced_map = Vec::with_capacity(keys.len());
        for file_path in keys {
            let references = self.snapshot.referenced_map.get_references(&file_path).unwrap();
            let file_id = self.to_file_id(&file_path);
            let file_id_list_id = self.to_file_id_list_id(&references);
            referenced_map.push(BuildInfoReferenceMapEntry { file_id, file_id_list_id });
        }
        self.build_info.referenced_map = referenced_map;
    }

    // snapshottobuildinfo.go:319
    fn set_change_file_set(&mut self) {
        let mut files = self.snapshot.changed_files_set.keys();
        files.sort();
        self.build_info.change_file_set = files.iter().map(|f| self.to_file_id(f)).collect();
    }

    // snapshottobuildinfo.go:325
    fn set_semantic_diagnostics(&mut self) {
        for &file in self.program.get_source_files() {
            let path = file.path().clone();
            match self.snapshot.semantic_diagnostics_per_file.load(&path) {
                None => {
                    if !self.snapshot.changed_files_set.has(&path) {
                        let file_id = self.to_file_id(&path);
                        self.build_info.semantic_diagnostics_per_file.push(BuildInfoSemanticDiagnostic { file_id, diagnostics: None });
                    }
                }
                Some(value) => {
                    if let Some(diagnostics) = self.to_build_info_diagnostics_of_file(&path, &value) {
                        self.build_info.semantic_diagnostics_per_file.push(BuildInfoSemanticDiagnostic { file_id: 0, diagnostics: Some(diagnostics) });
                    }
                }
            }
        }
    }

    // snapshottobuildinfo.go:344
    fn set_emit_diagnostics(&mut self) {
        let mut files = self.snapshot.emit_diagnostics_per_file.keys();
        files.sort();
        let mut result = Vec::with_capacity(files.len());
        for file_path in files {
            let value = self.snapshot.emit_diagnostics_per_file.load(&file_path).unwrap();
            // Go stores a nil entry when the cache holds no diagnostics (it never does in practice).
            if let Some(d) = self.to_build_info_diagnostics_of_file(&file_path, &value) {
                result.push(d);
            }
        }
        self.build_info.emit_diagnostics_per_file = result;
    }

    // snapshottobuildinfo.go:353
    fn set_affected_files_pending_emit(&mut self) {
        let mut files = self.snapshot.affected_files_pending_emit.keys();
        files.sort();
        let full_emit_kind = get_file_emit_kind(&self.snapshot.options());
        for file_path in files {
            let Some(file) = self.program.get_source_file_by_path(&file_path) else { continue };
            if !self.program.source_file_may_be_emitted(file, false) {
                continue;
            }
            let pending_emit = self.snapshot.affected_files_pending_emit.load(&file_path).unwrap_or_default();
            let file_id = self.to_file_id(&file_path);
            self.build_info.affected_files_pending_emit.push(BuildInfoFilePendingEmit {
                file_id,
                emit_kind: if pending_emit == full_emit_kind { crate::snapshot::FileEmitKind::None } else { pending_emit },
            });
        }
    }

    // snapshottobuildinfo.go:371
    fn set_root_of_non_incremental_program(&mut self) {
        let program = self.program;
        self.build_info.root = program
            .command_line()
            .file_names()
            .iter()
            .map(|file_name| BuildInfoRoot {
                non_incremental: self.relative_to_build_info(&tspath::to_path(
                    file_name,
                    &self.compare_paths_options.current_directory,
                    self.compare_paths_options.use_case_sensitive_file_names,
                )),
                ..Default::default()
            })
            .collect();
    }

    // snapshottobuildinfo.go:379
    fn set_package_jsons(&mut self) {
        let package_jsons = self.snapshot.package_jsons.borrow().clone().unwrap_or_default();
        if !package_jsons.is_empty() {
            self.build_info.package_jsons = Some(package_jsons.iter().map(|p| self.relative_to_build_info(p)).collect());
        }
        let missing_package_jsons = self.snapshot.missing_package_jsons.borrow().clone().unwrap_or_default();
        if !missing_package_jsons.is_empty() {
            self.build_info.missing_package_jsons = Some(missing_package_jsons.iter().map(|p| self.relative_to_build_info(p)).collect());
        }
    }
}

// snapshottobuildinfo.go:177
fn to_build_info_repopulate_info(info: Option<&RepopulateDiagnosticInfo>) -> Option<BuildInfoRepopulateInfo> {
    let info = info?;
    Some(BuildInfoRepopulateInfo {
        kind: info.kind,
        module_reference: info.module_reference.clone(),
        mode: info.mode,
        package_name: info.package_name.clone(),
    })
}

// Go json.Marshal of the option value: Tristate marshals as a bool (or null), the enums as their numbers.
fn option_value_to_json(v: &CompilerOptionsValue) -> Value {
    use CompilerOptionsValue as V;
    match v {
        V::Null | V::NilArray | V::EmptyStruct => Value::Null,
        V::Bool(b) => Value::Bool(*b),
        V::Float(f) => Value::Number(*f),
        V::Int(i) => Value::Number(*i as f64),
        V::String(s) => Value::String(s.clone()),
        V::Array(items) => Value::Array(items.iter().map(option_value_to_json).collect()),
        V::StringArray(items) => Value::Array(items.iter().map(|s| Value::String(s.clone())).collect()),
        V::Object(m) => Value::Object(m.iter().map(|(k, v)| (k.clone(), option_value_to_json(v))).collect()),
        V::Tristate(t) => match t {
            tsrs_core::Tristate::True => Value::Bool(true),
            tsrs_core::Tristate::False => Value::Bool(false),
            tsrs_core::Tristate::Unknown => Value::Null,
        },
        V::ModuleKind(k) => Value::Number(*k as i32 as f64),
        V::ModuleResolutionKind(k) => Value::Number(*k as i32 as f64),
        V::ModuleDetectionKind(k) => Value::Number(*k as i32 as f64),
        V::ScriptTarget(k) => Value::Number(*k as i32 as f64),
        V::JsxEmit(k) => Value::Number(*k as i32 as f64),
        V::NewLineKind(k) => Value::Number(*k as i32 as f64),
        V::WatchFileKind(k) => Value::Number(*k as i32 as f64),
        V::WatchDirectoryKind(k) => Value::Number(*k as i32 as f64),
        V::PollingKind(k) => Value::Number(*k as i32 as f64),
    }
}
