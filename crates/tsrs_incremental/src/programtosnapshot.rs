// Port of execute/incremental/programtosnapshot.go.

use tsrs_ast::{self as ast, Diagnostic, DiagnosticExt, Node, SourceFile, Symbol};
use tsrs_compiler::{Checker, Context, Program as CompilerProgram};
use tsrs_core::collections::Set;
use tsrs_core::tspath::{self, Path};
use tsrs_core::P;

use crate::program::Program;
use crate::snapshot::{
    buildInfoDiagnosticWithFileName, get_file_emit_kind, get_pending_emit_kind_with_options, repopulate_diagnostic_chain, DiagnosticsCache,
    DiagnosticsOrBuildInfoDiagnosticsWithFileName, EmitSignature, FileEmitKind, FileInfo, Snapshot,
};

// programtosnapshot.go:16
pub(crate) fn program_to_snapshot(program: &'static CompilerProgram, old_program: Option<P<Program>>, hash_with_text: bool) -> P<Snapshot> {
    if let Some(old_program) = old_program {
        if old_program.program.is_some_and(|p| std::ptr::eq(p, program)) {
            return old_program.snapshot;
        }
    }
    let snapshot = P::new(Snapshot { hash_with_text, ..Default::default() });
    snapshot.options.set(Some(program.options()));
    snapshot.check_pending.set(program.options().no_check.is_true());
    let mut to = toProgramSnapshot { program, old_program, snapshot, global_file_removed: false };

    if to.snapshot.can_use_incremental_state() {
        to.reuse_from_old_program();
        to.compute_program_file_changes();
        to.handle_file_delete();
        to.handle_global_scope_change();
        to.handle_pending_emit();
        to.handle_pending_check();
    }
    snapshot
}

// What computeProgramFileChanges' per-file function stores into the snapshot besides the file info and references.
struct fileChange {
    add_to_change_set: bool,
    emit_diagnostics: Option<DiagnosticsCache>,
    semantic_diagnostics: Option<DiagnosticsCache>,
    emit_signature: Option<EmitSignature>,
}

struct toProgramSnapshot {
    program: &'static CompilerProgram,
    old_program: Option<P<Program>>,
    snapshot: P<Snapshot>,
    global_file_removed: bool,
}

impl toProgramSnapshot {
    // programtosnapshot.go:50
    fn reuse_from_old_program(&mut self) {
        if let Some(old_program) = self.old_program {
            let old = old_program.snapshot;
            if self.snapshot.options().composite.is_true() {
                *self.snapshot.latest_changed_dts_file.borrow_mut() = old.latest_changed_dts_file.borrow().clone();
            }
            // Copy old snapshot's changed files set
            old.changed_files_set.range(|key| {
                self.snapshot.changed_files_set.add(key.clone());
                true
            });
            old.affected_files_pending_emit.range(|key, &emit_kind| {
                self.snapshot.affected_files_pending_emit.store(key.clone(), emit_kind);
                true
            });
            self.snapshot.build_info_emit_pending.set(old.build_info_emit_pending.get());
            self.snapshot.has_errors_from_old_state.set(old.has_errors.get());
            self.snapshot.has_semantic_errors_from_old_state.set(old.has_semantic_errors.get());
            *self.snapshot.package_jsons_from_old_state.borrow_mut() = old.package_jsons.borrow().clone();
            *self.snapshot.missing_package_jsons_from_old_state.borrow_mut() = old.missing_package_jsons.borrow().clone();
        } else {
            self.snapshot.build_info_emit_pending.set(self.snapshot.options().is_incremental());
        }
    }

    // programtosnapshot.go:75
    fn compute_program_file_changes(&mut self) {
        let old = self.old_program.map(|p| p.snapshot);
        let can_copy_semantic_diagnostics =
            old.is_some_and(|old| !tsrs_tsoptions::compiler_options_affect_semantic_diagnostics(Some(&old.options()), Some(&self.program.options())));
        // We can only reuse emit signatures (i.e. .d.ts signatures) if the .d.ts file is unchanged,
        // which will eg be depedent on change in options like declarationDir and outDir options are unchanged.
        // We need to look in oldState.compilerOptions, rather than oldCompilerOptions (i.e.we need to disregard useOldState) because
        // oldCompilerOptions can be undefined if there was change in say module from None to some other option
        // which would make useOldState as false since we can now use reference maps that are needed to track what to emit, what to check etc
        // but that option change does not affect d.ts file name so emitSignatures should still be reused.
        let can_copy_emit_signatures = self.snapshot.options().composite.is_true()
            && old.is_some_and(|old| !tsrs_tsoptions::compiler_options_affect_declaration_path(Some(&old.options()), Some(&self.program.options())));
        let copy_declaration_file_diagnostics =
            can_copy_semantic_diagnostics && self.snapshot.options().skip_lib_check.is_true() == old.unwrap().options().skip_lib_check.is_true();
        let copy_lib_file_diagnostics = copy_declaration_file_diagnostics
            && self.snapshot.options().skip_default_lib_check.is_true() == old.unwrap().options().skip_default_lib_check.is_true();

        let files = self.program.get_source_files();
        let old_options = old.map(|old| old.options());
        let ambient_module_files_by_checker = AmbientModuleFilesByChecker::default();
        // Files of the old state that the new program does not have.
        let mut deleted_files: rustc_hash::FxHashSet<Path> = rustc_hash::FxHashSet::default();
        if let Some(old) = old {
            for path in old.file_infos.keys() {
                if self.program.get_source_file_by_path(&path).is_none() {
                    deleted_files.insert(path);
                }
            }
        }
        let new_options = self.snapshot.options();
        // programtosnapshot.go:91: Go queues the per-file work on a work group and lets each function store into the
        // snapshot's sync maps. Here the per-file work runs on the program's worker pool and only reads shared state;
        // its results are stored in file order afterwards, so the maps are filled in the same order as a sequential run.
        let compute = |file: P<SourceFile>, checker_references: Option<checkerReferences>| -> (FileInfo, Option<std::sync::Arc<Set<Path>>>, fileChange) {
            // Content mappers are not supported by tsrs; Go hashes the original text plus the mapper identity here.
            let version = self.snapshot.compute_hash(file.text());
            let implied_node_format = self.program.get_source_file_meta_data(file.path()).implied_node_format;
            let affects_global_scope = file_affects_global_scope(file);
            let mut signature = String::new();
            let checker_references = checker_references.unwrap_or_else(|| {
                let mut checker = self.program.get_type_checker_for_file_exclusive(&Context::default(), file);
                checker_references_of(file, &mut checker, &ambient_module_files_by_checker)
            });
            let old_references = old.and_then(|old| old.referenced_map.get_references(file.path()));
            let new_references = referenced_files_of(self.program, file, checker_references, old_references.as_ref());
            let mut change = fileChange { add_to_change_set: false, emit_diagnostics: None, semantic_diagnostics: None, emit_signature: None };
            if let Some(old) = old {
                if let Some(old_file_info) = old.file_infos.load(file.path()) {
                    if old_file_info.version != version
                        || old_file_info.affects_global_scope != affects_global_scope
                        || old_file_info.implied_node_format != implied_node_format
                    {
                        change.add_to_change_set = true;
                    } else if !new_references.as_ref().is_some_and(|new| old_references.as_ref().is_some_and(|old| std::sync::Arc::ptr_eq(new, old)))
                        && new_references.as_deref() != old_references.as_deref()
                    {
                        // Referenced files changed
                        change.add_to_change_set = true;
                    } else if let Some(new_references) = &new_references {
                        // Go ranges over the set and asks, for each path, whether the new program lacks it while the old
                        // state had it; that is membership in deleted_files, which is usually empty.
                        if !deleted_files.is_empty() && new_references.keys().iter().any(|ref_path| deleted_files.contains(ref_path)) {
                            // Referenced file was deleted in the new program
                            change.add_to_change_set = true;
                        }
                    }
                    signature = old_file_info.signature;
                } else {
                    change.add_to_change_set = true;
                }
                // Only this file's own entry in the change set matters, and the parallel phase does not write the set.
                if !change.add_to_change_set && !self.snapshot.changed_files_set.has(file.path()) {
                    if let Some(emit_diagnostics) = old.emit_diagnostics_per_file.load(file.path()) {
                        change.emit_diagnostics = Some(repopulate_diagnostics_of_file(&emit_diagnostics, self.program, file));
                    }
                    if can_copy_semantic_diagnostics
                        && (!file.is_declaration_file() || copy_declaration_file_diagnostics)
                        && (!self.program.is_source_file_default_library(file.path()) || copy_lib_file_diagnostics)
                    {
                        // Unchanged file copy diagnostics
                        if let Some(diagnostics) = old.semantic_diagnostics_per_file.load(file.path()) {
                            change.semantic_diagnostics = Some(repopulate_diagnostics_of_file(&diagnostics, self.program, file));
                        }
                    }
                }
                if can_copy_emit_signatures {
                    if let Some(old_emit_signature) = old.emit_signatures.load(file.path()) {
                        change.emit_signature = Some(old_emit_signature.get_new_emit_signature(old_options.as_ref().unwrap(), &new_options));
                    }
                }
            } else {
                signature = version.clone();
            }
            (FileInfo { version, signature, affects_global_scope, implied_node_format }, new_references, change)
        };
        let results: Vec<(FileInfo, Option<std::sync::Arc<Set<Path>>>, fileChange)> = if self.program.single_threaded() {
            files.iter().map(|&file| compute(file, None)).collect()
        } else {
            use rayon::prelude::*;
            // The checker lookups first, one task per checker that holds its checker for all of its files (in file
            // order) instead of 18 workers taking turns on 4 checker locks per file; then the rest of the per-file
            // work on the worker pool. Without a pool of its own the program locks per file.
            let checker_references: Vec<std::sync::OnceLock<checkerReferences>> = files.iter().map(|_| std::sync::OnceLock::new()).collect();
            let batched = self.program.for_each_checker_group(files, |checker, i, file| {
                let _ = checker_references[i].set(checker_references_of(file, checker, &ambient_module_files_by_checker));
            });
            let checker_references: Vec<Option<checkerReferences>> =
                checker_references.into_iter().map(|r| if batched { r.into_inner() } else { None }).collect();
            if !batched {
                // Create the checker pool before the parallel loop: its lazy initialization runs on the worker pool, and a
                // worker that steals another file's function while initializing would re-enter the pool's OnceLock.
                if let Some(&first) = files.first() {
                    drop(self.program.get_type_checker_for_file_exclusive(&Context::default(), first));
                }
            }
            tsrs_compiler::worker_pool().install(|| files.par_iter().zip(checker_references).map(|(&file, r)| compute(file, r)).collect())
        };

        for (&file, (info, new_references, change)) in files.iter().zip(results) {
            if let Some(new_references) = new_references {
                self.snapshot.referenced_map.store_shared_references(file.path().clone(), new_references);
            }
            if old.is_some() {
                if change.add_to_change_set {
                    self.snapshot.add_file_to_change_set(file.path().clone());
                }
                if let Some(emit_diagnostics) = change.emit_diagnostics {
                    self.snapshot.emit_diagnostics_per_file.store(file.path().clone(), emit_diagnostics);
                }
                if let Some(diagnostics) = change.semantic_diagnostics {
                    self.snapshot.semantic_diagnostics_per_file.store(file.path().clone(), diagnostics);
                }
                if let Some(emit_signature) = change.emit_signature {
                    self.snapshot.emit_signatures.store(file.path().clone(), emit_signature);
                }
            } else {
                self.snapshot.add_file_to_affected_files_pending_emit(file.path().clone(), get_file_emit_kind(&new_options));
            }
            self.snapshot.file_infos.store(file.path().clone(), info);
        }
    }

    // programtosnapshot.go:160
    fn handle_file_delete(&mut self) {
        if let Some(old_program) = self.old_program {
            // If the global file is removed, add all files as changed
            old_program.snapshot.file_infos.range(|file_path, old_info| {
                if self.snapshot.file_infos.load(file_path).is_none() {
                    if old_info.affects_global_scope {
                        for &file in self.snapshot.get_all_files_excluding_default_library_file(self.program, None) {
                            self.snapshot.add_file_to_change_set(file.path().clone());
                        }
                        self.global_file_removed = true;
                    } else {
                        self.snapshot.build_info_emit_pending.set(true);
                    }
                    return false;
                }
                true
            });
        }
    }

    // programtosnapshot.go:180
    fn handle_global_scope_change(&mut self) {
        let Some(old_program) = self.old_program else { return };
        if self.global_file_removed {
            return;
        }
        let mut global_scope_lost = false;
        old_program.snapshot.file_infos.range(|file_path, old_info| {
            if !old_info.affects_global_scope {
                return true;
            }
            if let Some(new_info) = self.snapshot.file_infos.load(file_path) {
                if !new_info.affects_global_scope {
                    global_scope_lost = true;
                    return false;
                }
            }
            true
        });
        if global_scope_lost {
            for &file in self.snapshot.get_all_files_excluding_default_library_file(self.program, None) {
                self.snapshot.add_file_to_change_set(file.path().clone());
            }
        }
    }

    // programtosnapshot.go:203
    fn handle_pending_emit(&mut self) {
        if let Some(old_program) = self.old_program {
            if self.global_file_removed {
                return;
            }
            // If options affect emit, then we need to do complete emit per compiler options
            // otherwise only the js or dts that needs to emitted because its different from previously emitted options
            let pending_emit_kind = if tsrs_tsoptions::compiler_options_affect_emit(Some(&old_program.snapshot.options()), Some(&self.snapshot.options())) {
                get_file_emit_kind(&self.snapshot.options())
            } else {
                get_pending_emit_kind_with_options(&self.snapshot.options(), &old_program.snapshot.options())
            };
            if pending_emit_kind != FileEmitKind::None {
                // Add all files to affectedFilesPendingEmit since emit changed
                for &file in self.program.get_source_files() {
                    // Add to affectedFilesPending emit only if not changed since any changed file will do full emit
                    if !self.snapshot.changed_files_set.has(file.path()) {
                        self.snapshot.add_file_to_affected_files_pending_emit(file.path().clone(), pending_emit_kind);
                    }
                }
                self.snapshot.build_info_emit_pending.set(true);
            }
        }
    }

    // programtosnapshot.go:226
    fn handle_pending_check(&mut self) {
        if let Some(old_program) = self.old_program {
            if self.snapshot.semantic_diagnostics_per_file.size() != self.program.get_source_files().len()
                && old_program.snapshot.check_pending.get() != self.snapshot.check_pending.get()
            {
                self.snapshot.build_info_emit_pending.set(true);
            }
        }
    }
}

// programtosnapshot.go:235
pub(crate) fn file_affects_global_scope(file: P<SourceFile>) -> bool {
    tsrs_binder::bind_source_file(file);
    // if file contains anything that augments to global scope we need to build them as if
    // they are global files as well as module
    if file.module_augmentations().iter().any(|&augmentation| ast::is_global_scope_augmentation(augmentation.parent().unwrap())) {
        return true;
    }

    if ast::is_external_or_common_js_module(file) || ast::is_json_source_file(file) {
        return false;
    }

    // For script files that contains only ambient external modules, although they are not actually external module files,
    // they can only be consumed via importing elements from them. Regular script files cannot consume them. Therefore,
    // there are no point to rebuild all script files if these special files have changed. However, if any statement
    // in the file is not ambient external module, we treat it as a regular script file.
    file.statements().nodes().iter().any(|&stmt| !ast::is_module_with_string_literal_name(stmt))
}

// Collects the declaring files; referenced_files_of turns them into paths after releasing the checker.
// programtosnapshot.go:259
fn add_referenced_files_from_symbol(file: P<SourceFile>, referenced_files: &mut Vec<P<SourceFile>>, symbol: Option<P<Symbol>>) {
    let Some(symbol) = symbol else { return };
    for &declaration in symbol.declarations() {
        let Some(file_of_decl) = ast::get_source_file_of_node(declaration) else { continue };
        // A symbol often has several declarations in one file; skipping the repeat saves hashing its path again.
        if file != file_of_decl && referenced_files.last() != Some(&file_of_decl) {
            referenced_files.push(file_of_decl);
        }
    }
}

// Get the module source file and all augmenting files from the import name node from file
// programtosnapshot.go:275
fn add_referenced_files_from_import_literal(file: P<SourceFile>, referenced_files: &mut Vec<P<SourceFile>>, checker: &mut Checker, import_name: P<Node>) {
    let symbol = checker.get_symbol_at_location_exported(import_name);
    add_referenced_files_from_symbol(file, referenced_files, symbol);
}

// Gets the path to reference file from file name, it could be resolvedPath if present otherwise path
// programtosnapshot.go:281 (addReferencedFileFromFileName, returning the path it adds)
fn referenced_file_path_from_file_name(program: &CompilerProgram, file_name: &str, source_file_directory: &str) -> Path {
    let redirect = program.get_parse_file_redirect(file_name);
    if !redirect.is_empty() {
        tspath::to_path(&redirect, program.get_current_directory(), program.use_case_sensitive_file_names())
    } else {
        tspath::to_path(file_name, source_file_directory, program.use_case_sensitive_file_names())
    }
}

// The files declaring each checker's ambient modules (in order, and as a set), keyed by the checker's address; the
// checkers outlive the loop.
type AmbientModuleFiles = std::sync::Arc<(Vec<P<SourceFile>>, rustc_hash::FxHashSet<P<SourceFile>>)>;
type AmbientModuleFilesByChecker = std::sync::Mutex<rustc_hash::FxHashMap<usize, AmbientModuleFiles>>;

// The part of getReferencedFiles that needs the file's checker: the files declaring the symbols of the file's imports
// and module augmentations, and the files declaring the checker's ambient modules (the file itself included;
// referenced_files_of leaves it out).
struct checkerReferences {
    import_files: Vec<P<SourceFile>>,
    augmentation_files: Vec<P<SourceFile>>,
    ambient_module_files: AmbientModuleFiles,
}

fn checker_references_of(file: P<SourceFile>, checker: &mut Checker, ambient_module_files_by_checker: &AmbientModuleFilesByChecker) -> checkerReferences {
    let mut import_files: Vec<P<SourceFile>> = Vec::new();
    let mut augmentation_files: Vec<P<SourceFile>> = Vec::new();
    for &import_name in file.imports() {
        add_referenced_files_from_import_literal(file, &mut import_files, checker, import_name);
    }
    // Add module augmentation as references
    for &module_name in file.module_augmentations() {
        if !ast::is_string_literal(module_name) {
            continue;
        }
        add_referenced_files_from_import_literal(file, &mut augmentation_files, checker, module_name);
    }
    // From ambient modules. A checker's ambient modules and their declarations are fixed once it is initialized,
    // so their declaring files are collected once per checker and only filtered per file.
    let key = &*checker as *const Checker as usize;
    let declaring_files = ambient_module_files_by_checker.lock().unwrap().get(&key).cloned();
    let ambient_module_files = declaring_files.unwrap_or_else(|| {
        let mut declaring_files = Vec::new();
        let mut seen: rustc_hash::FxHashSet<P<SourceFile>> = rustc_hash::FxHashSet::default();
        for ambient_module in checker.get_ambient_modules() {
            for &declaration in ambient_module.declarations() {
                let Some(file_of_decl) = ast::get_source_file_of_node(declaration) else { continue };
                if seen.insert(file_of_decl) {
                    declaring_files.push(file_of_decl);
                }
            }
        }
        let declaring_files = std::sync::Arc::new((declaring_files, seen));
        ambient_module_files_by_checker.lock().unwrap().insert(key, declaring_files.clone());
        declaring_files
    });
    checkerReferences { import_files, augmentation_files, ambient_module_files }
}

// Gets the referenced files for a file from the program with values for the keys as referenced file's path to be true
// programtosnapshot.go:290
// Go holds the checker for the whole function. Only the symbol lookups need it (checker_references_of); the paths
// are added here without it, in Go's order: imports, triple slash references, type reference directives, module
// augmentations, ambient modules.
//
// When the file's old reference set (`old`) has exactly these paths, the old set is returned instead of a new one:
// an unchanged file then allocates nothing (on the 38k-file codebase every file references the ~390 files declaring
// ambient modules, ~15 M paths in all).
fn referenced_files_of(
    program: &'static CompilerProgram,
    file: P<SourceFile>,
    checker_references: checkerReferences,
    old: Option<&std::sync::Arc<Set<Path>>>,
) -> Option<std::sync::Arc<Set<Path>>> {
    let checkerReferences { import_files, augmentation_files, ambient_module_files } = checker_references;
    let (ambient_files, ambient_file_set) = &*ambient_module_files;
    let source_file_directory = tspath::get_directory_path(file.file_name());
    // Triple slash references, then type reference directives.
    let mut file_name_paths: Vec<Path> = Vec::new();
    for referenced_file in file.referenced_files() {
        file_name_paths.push(referenced_file_path_from_file_name(program, &referenced_file.file_name, &source_file_directory));
    }
    if let Some(type_refs_in_file) = program.get_resolved_type_reference_directives().get(file.path()) {
        for type_ref in type_refs_in_file.values() {
            if !type_ref.resolved_file_name.is_empty() {
                file_name_paths.push(referenced_file_path_from_file_name(program, type_ref.resolved_file_name, &source_file_directory));
            }
        }
    }

    if let Some(old) = old {
        if references_equal(program, file, &import_files, &augmentation_files, ambient_files, ambient_file_set, &file_name_paths, old) {
            return Some(old.clone());
        }
    }

    // We need to use a set here since the code can contain the same import twice,
    // but that will only be one dependency.
    // To avoid invernal conversion, the key of the referencedFiles map must be of type Path
    let mut referenced_files: Set<Path> = Set::default();
    // The declaring files are distinct apart from repeats among the imports, so this is close to the final size.
    referenced_files.m.reserve(import_files.len() + file_name_paths.len() + augmentation_files.len() + ambient_files.len());
    for f in import_files {
        referenced_files.add(f.path().clone());
    }
    for path in file_name_paths {
        referenced_files.add(path);
    }
    for f in augmentation_files.into_iter().chain(ambient_files.iter().copied().filter(|&file_of_decl| file_of_decl != file)) {
        referenced_files.add(f.path().clone());
    }
    if referenced_files.len() > 0 {
        Some(std::sync::Arc::new(referenced_files))
    } else {
        None
    }
}

// Whether `old` is exactly the set referenced_files_of would build: as large as the number of distinct paths, and
// containing each of them. Distinct program files have distinct paths, and the ambient module files are distinct.
fn references_equal(
    program: &CompilerProgram,
    file: P<SourceFile>,
    import_files: &[P<SourceFile>],
    augmentation_files: &[P<SourceFile>],
    ambient_files: &[P<SourceFile>],
    ambient_file_set: &rustc_hash::FxHashSet<P<SourceFile>>,
    file_name_paths: &[Path],
    old: &Set<Path>,
) -> bool {
    let file_is_ambient = ambient_file_set.contains(&file);
    let in_ambient = |f: P<SourceFile>| f != file && ambient_file_set.contains(&f);
    // Imported and augmenting files that are not also ambient module files, without repeats.
    let mut other_files: Vec<P<SourceFile>> = Vec::new();
    for &f in import_files.iter().chain(augmentation_files) {
        if !in_ambient(f) && !other_files.contains(&f) {
            other_files.push(f);
        }
    }
    let mut count = ambient_files.len() - usize::from(file_is_ambient) + other_files.len();
    for (i, path) in file_name_paths.iter().enumerate() {
        let is_program_file_path = |f: P<SourceFile>| f.path() == path;
        let names_counted_file = program.get_source_file_by_path(path).is_some_and(|f| is_program_file_path(f) && (in_ambient(f) || other_files.contains(&f)));
        if !names_counted_file && !file_name_paths[..i].contains(path) {
            count += 1;
        }
    }
    if count != old.len() {
        return false;
    }
    other_files.iter().all(|f| old.has(f.path()))
        && file_name_paths.iter().all(|path| old.has(path))
        && ambient_files.iter().all(|&f| f == file || old.has(f.path()))
}

// repopulateDiagnosticsOfFile repopulates diagnostic chains that depend on program state.
// When diagnostics are copied from a previous build, their message chains may reference
// stale program state (e.g., resolved module alternate results, package.json scope).
// This function recomputes those chains using the current program's state.
// programtosnapshot.go:339
fn repopulate_diagnostics_of_file(diags: &DiagnosticsCache, p: &'static CompilerProgram, file: P<SourceFile>) -> DiagnosticsCache {
    if let Some(diagnostics) = diags.diagnostics() {
        let Some(repopulated) = repopulate_diagnostics_list(&diagnostics, p, file) else {
            return diags.clone();
        };
        return DiagnosticsOrBuildInfoDiagnosticsWithFileName::from_diagnostics(repopulated);
    }
    // buildInfoDiagnostics will be repopulated via toDiagnostic's repopulateInfo handling
    diags.clone()
}

// repopulateDiagnosticsList repopulates diagnostic chains in a list of diagnostics.
// Returns nil if no diagnostics needed repopulation (i.e., no changes were made).
// programtosnapshot.go:353
fn repopulate_diagnostics_list(diags: &[P<Diagnostic>], p: &'static CompilerProgram, file: P<SourceFile>) -> Option<Vec<P<Diagnostic>>> {
    let mut changed = false;
    let mut result = Vec::with_capacity(diags.len());
    for &d in diags {
        match repopulate_diagnostic_message_chain(d.message_chain(), p, file) {
            Some(repopulated) => {
                let clone = d.clone_diagnostic();
                clone.set_message_chain(&repopulated);
                result.push(clone);
                changed = true;
            }
            None => result.push(d),
        }
    }
    if !changed {
        return None;
    }
    Some(result)
}

// repopulateDiagnosticMessageChain repopulates chains that have repopulate info.
// Returns nil if no changes were made.
// programtosnapshot.go:375
fn repopulate_diagnostic_message_chain(chain: &[P<Diagnostic>], p: &'static CompilerProgram, file: P<SourceFile>) -> Option<Vec<P<Diagnostic>>> {
    if chain.is_empty() {
        return None;
    }
    let mut changed = false;
    let mut result = Vec::with_capacity(chain.len());
    for &c in chain {
        if let Some(info) = c.repopulate_info() {
            // Convert to buildInfoDiagnosticWithFileName and repopulate
            let mut b = buildInfoDiagnosticWithFileName {
                pos: c.pos(),
                end: c.end(),
                code: c.code(),
                category: c.category(),
                source: c.source().to_string(),
                message_text: c.message_text().to_string(),
                message_key: c.message_key().0.to_string(),
                message_args: c.message_args().to_vec(),
                repopulate_info: Some(info.clone()),
                ..Default::default()
            };
            // Recursively handle nested chains
            for &nested in c.message_chain() {
                b.message_chain.push(ast_diag_to_build_info_diag(nested));
            }
            result.push(repopulate_diagnostic_chain(&b, p, Some(file)));
            changed = true;
        } else {
            // Check nested chains
            match repopulate_diagnostic_message_chain(c.message_chain(), p, file) {
                Some(nested) => {
                    let clone = c.clone_diagnostic();
                    clone.set_message_chain(&nested);
                    result.push(clone);
                    changed = true;
                }
                None => result.push(c),
            }
        }
    }
    if !changed {
        return None;
    }
    Some(result)
}

// programtosnapshot.go:420
fn ast_diag_to_build_info_diag(d: P<Diagnostic>) -> buildInfoDiagnosticWithFileName {
    let mut b = buildInfoDiagnosticWithFileName {
        pos: d.pos(),
        end: d.end(),
        code: d.code(),
        category: d.category(),
        source: d.source().to_string(),
        message_text: d.message_text().to_string(),
        message_key: d.message_key().0.to_string(),
        message_args: d.message_args().to_vec(),
        repopulate_info: d.repopulate_info().cloned(),
        ..Default::default()
    };
    for &nested in d.message_chain() {
        b.message_chain.push(ast_diag_to_build_info_diag(nested));
    }
    b
}
