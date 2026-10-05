// The emit parts of the Go harness (docs/EMIT.md section 8): harnessutil/recorderfs.go (OutputRecorderFS),
// harnessutil.go's compileFilesWithHost (pre-/post-emit programs) and newCompilationResult (output ordering), and
// tsbaseline/js_emit_baseline.go (DoJSEmitBaseline with DtsFileErrors and the noCheck repeat).
//
// Only used under `--baselines js`; the default mode keeps the single type-check program in compile.rs.

use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use indexmap::IndexMap;
use rustc_hash::FxHashMap;
use tsrs_ast::{new_compiler_diagnostic, DiagnosticExt, SourceFileParseOptions};
use tsrs_compiler::{self as compiler, CompilerHost};
use tsrs_core::tspath::{self, ComparePathsOptions};
use tsrs_core::{CompilerOptions, ScriptKind, Tristate, P};
use tsrs_tsoptions::{outputpaths, ParsedCommandLine};
use tsrs_vfs::{Entries, FileInfo, FS};

use crate::baseline::NO_CONTENT;
use crate::compile::{convert_diagnostics, CompilationResult};
use crate::harnessutil::{HarnessOptions, TestFile};
use crate::tsbaseline;

// recorderfs.go:10
pub struct OutputRecorderFS {
    fs: Arc<dyn FS>,
    outputs: Mutex<(FxHashMap<String, usize>, Vec<TestFile>)>,
}

// recorderfs.go:17
pub fn new_output_recorder_fs(fs: Arc<dyn FS>) -> Arc<OutputRecorderFS> {
    Arc::new(OutputRecorderFS { fs, outputs: Mutex::new((FxHashMap::default(), Vec::new())) })
}

impl OutputRecorderFS {
    // recorderfs.go:42
    pub fn outputs(&self) -> Vec<TestFile> {
        self.outputs.lock().unwrap().1.clone()
    }
}

impl FS for OutputRecorderFS {
    fn use_case_sensitive_file_names(&self) -> bool {
        self.fs.use_case_sensitive_file_names()
    }
    fn file_exists(&self, path: &str) -> bool {
        self.fs.file_exists(path)
    }
    fn read_file(&self, path: &str) -> Option<String> {
        self.fs.read_file(path)
    }
    // recorderfs.go:21
    fn write_file(&self, path: &str, data: &str) -> Result<(), String> {
        self.fs.write_file(path, data)?;
        let path = self.fs.realpath(path);
        let mut guard = self.outputs.lock().unwrap();
        let (map, outputs) = &mut *guard;
        if let Some(&index) = map.get(&path) {
            outputs[index] = TestFile { unit_name: path, content: data.to_string() };
        } else {
            let index = outputs.len();
            map.insert(path.clone(), index);
            outputs.push(TestFile { unit_name: path, content: data.to_string() });
        }
        Ok(())
    }
    fn append_file(&self, path: &str, data: &str) -> Result<(), String> {
        self.fs.append_file(path, data)
    }
    fn remove(&self, path: &str) -> Result<(), String> {
        self.fs.remove(path)
    }
    fn chtimes(&self, path: &str, a_time: SystemTime, m_time: SystemTime) -> Result<(), String> {
        self.fs.chtimes(path, a_time, m_time)
    }
    fn directory_exists(&self, path: &str) -> bool {
        self.fs.directory_exists(path)
    }
    fn get_accessible_entries(&self, path: &str) -> Entries {
        self.fs.get_accessible_entries(path)
    }
    fn stat(&self, path: &str) -> Option<FileInfo> {
        self.fs.stat(path)
    }
    fn realpath(&self, path: &str) -> String {
        self.fs.realpath(path)
    }
}

/// The outputs of an emitting compilation (Go `CompilationResult.JS/DTS/Maps` and `Repeat`).
#[derive(Default)]
pub struct EmitOutputs {
    pub emit_result: compiler::EmitResult,
    pub js: IndexMap<String, TestFile>,
    pub dts: IndexMap<String, TestFile>,
    pub maps: IndexMap<String, TestFile>,
    // Go `CompilationResult.inputs` (every program source file) and `.outputs` (the per-input JS/DTS/map outputs).
    pub inputs: Vec<TestFile>,
    pub outputs: Vec<TestFile>,
}

// harnessutil.go:626 (from the pre-emit program on). Returns the diagnostics and the emit result of the post-emit
// program.
pub fn compile_files_with_host_emit(
    host: Arc<dyn CompilerHost>,
    config: P<ParsedCommandLine>,
    harness_options: &HarnessOptions,
    create_program: &dyn Fn(Arc<dyn CompilerHost>, P<ParsedCommandLine>) -> &'static dyn compiler::ProgramLike,
) -> (Vec<P<tsrs_ast::Diagnostic>>, &'static compiler::Program, compiler::EmitResult) {
    let ctx = &compiler::Context::default();

    let mut pre_errors = Vec::new();
    let mut pre_compiler_options: CompilerOptions = (*config.compiler_options().unwrap()).clone();
    pre_compiler_options.trace_resolution = Tristate::False;
    let mut pre_config = tsrs_tsoptions::new_parsed_command_line(P::new(pre_compiler_options), config.file_names().to_vec(), Vec::new(), ComparePathsOptions::default());
    pre_config.config_file = config.config_file;
    pre_config.errors.clone_from(&config.errors);
    let pre_program = create_program(Arc::clone(&host), P::new(pre_config));
    pre_errors.extend(pre_program.get_config_file_parsing_diagnostics());
    pre_errors.extend(pre_program.get_program_diagnostics());
    pre_errors.extend(pre_program.get_syntactic_diagnostics(ctx, None));
    pre_errors.extend(pre_program.get_semantic_diagnostics(ctx, None));
    pre_errors.extend(pre_program.get_global_diagnostics(ctx));
    if harness_options.capture_suggestions {
        pre_errors.extend(pre_program.get_suggestion_diagnostics(ctx, None));
    }
    if pre_program.options().get_emit_declarations() {
        pre_errors.extend(pre_program.get_declaration_diagnostics(ctx, None));
    }
    let pre_errors = compiler::sort_and_deduplicate_diagnostics(&pre_errors);

    let post_program = create_program(host, config);
    // Go `postProgram.Emit` returns nil only on cancellation.
    let emit_result = post_program.emit(ctx, compiler::EmitOptions::default()).unwrap_or_default();
    let mut post_errors = Vec::new();
    post_errors.extend(post_program.get_config_file_parsing_diagnostics());
    post_errors.extend(post_program.get_program_diagnostics());
    post_errors.extend(post_program.get_syntactic_diagnostics(ctx, None));
    post_errors.extend(post_program.get_semantic_diagnostics(ctx, None));
    post_errors.extend(post_program.get_global_diagnostics(ctx));
    if post_program.options().get_emit_declarations() {
        post_errors.extend(post_program.get_declaration_diagnostics(ctx, None));
    }
    if harness_options.capture_suggestions {
        post_errors.extend(post_program.get_suggestion_diagnostics(ctx, None));
    }
    let post_errors = compiler::sort_and_deduplicate_diagnostics(&post_errors);

    let mut errors = post_errors.clone();
    if post_errors.len() != pre_errors.len() {
        let (longer_errors, shorter_errors) = if pre_errors.len() > post_errors.len() { (&pre_errors, &post_errors) } else { (&post_errors, &pre_errors) };
        let message = format!(
            "Pre-emit ({}) and post-emit ({}) diagnostic counts do not match! This can indicate that a semantic _error_ was added by the emit resolver - such an error may not be reflected on the command line or in the editor, but may be captured in a baseline here!",
            pre_errors.len(),
            post_errors.len()
        );
        let mut diag = new_compiler_diagnostic(tsrs_diagnostics::new_ad_hoc_message(&message), &[]);
        diag = diag.add_related_info(new_compiler_diagnostic(tsrs_diagnostics::new_ad_hoc_message("The excess diagnostics are:"), &[]));
        for &d in longer_errors {
            let matched = shorter_errors.iter().any(|&d2| tsrs_ast::compare_diagnostics(d, d2) == 0);
            if !matched {
                diag = diag.add_related_info(d);
            }
        }
        errors.clone_from(shorter_errors);
        errors.push(diag);
    }

    (errors, post_program.program(), emit_result)
}

// harnessutil.go:746 (the output part of newCompilationResult)
pub fn new_emit_outputs(recorder: &OutputRecorderFS, program: &'static compiler::Program, options: &CompilerOptions, host: &dyn CompilerHost, emit_result: compiler::EmitResult) -> EmitOutputs {
    let mut c = EmitOutputs { emit_result, ..Default::default() };

    // Corsa, unlike Strada, can use multiple threads for emit. As a result, the order of outputs is non-deterministic.
    // To make the order deterministic, we sort the outputs by the order of the inputs.
    let mut js: IndexMap<String, TestFile> = IndexMap::new();
    let mut dts: IndexMap<String, TestFile> = IndexMap::new();
    let mut maps: IndexMap<String, TestFile> = IndexMap::new();
    for document in recorder.outputs() {
        if tspath::has_js_file_extension(&document.unit_name) || tspath::has_json_file_extension(&document.unit_name) {
            js.insert(document.unit_name.clone(), document);
        } else if tspath::is_declaration_file_name(&document.unit_name) {
            dts.insert(document.unit_name.clone(), document);
        } else if tspath::file_extension_is(&document.unit_name, ".map") {
            maps.insert(document.unit_name.clone(), document);
        }
    }

    // using the order from the inputs, populate the outputs
    for &source_file in program.get_source_files() {
        c.inputs.push(TestFile { unit_name: source_file.file_name().to_string(), content: source_file.text().to_string() });
        if !tspath::is_declaration_file_name(source_file.file_name()) {
            let extname = outputpaths::get_output_extension(source_file.file_name(), options.jsx);
            let out_js = js.get(&get_output_path(program, options, host, source_file.file_name(), extname)).cloned();
            let out_dts = dts.get(&get_output_path(program, options, host, source_file.file_name(), &tspath::get_declaration_emit_extension_for_path(source_file.file_name()))).cloned();
            let out_map = maps.get(&get_output_path(program, options, host, source_file.file_name(), &format!("{extname}.map"))).cloned();
            if let Some(f) = out_js {
                js.shift_remove(&f.unit_name);
                c.outputs.push(f.clone());
                c.js.insert(f.unit_name.clone(), f);
            }
            if let Some(f) = out_dts {
                dts.shift_remove(&f.unit_name);
                c.outputs.push(f.clone());
                c.dts.insert(f.unit_name.clone(), f);
            }
            if let Some(f) = out_map {
                maps.shift_remove(&f.unit_name);
                c.outputs.push(f.clone());
                c.maps.insert(f.unit_name.clone(), f);
            }
        }
    }

    // add any unhandled outputs, ordered by unit name
    for (rest, target) in [(js, &mut c.js), (dts, &mut c.dts), (maps, &mut c.maps)] {
        let mut docs: Vec<TestFile> = rest.into_values().collect();
        docs.sort_by(|a, b| a.unit_name.cmp(&b.unit_name));
        for d in docs {
            target.insert(d.unit_name.clone(), d);
        }
    }
    c
}

// harnessutil.go:836
fn get_output_path(program: &'static compiler::Program, options: &CompilerOptions, host: &dyn CompilerHost, path: &str, ext: &str) -> String {
    let mut path = tspath::resolve_path(host.get_current_directory(), &[path]);
    let out_dir = if ext == ".d.ts" || ext == ".d.mts" || ext == ".d.cts" || (ext.ends_with(".ts") && ext.contains(".d.")) {
        if options.declaration_dir.is_empty() {
            &options.out_dir
        } else {
            &options.declaration_dir
        }
    } else {
        &options.out_dir
    };
    if !out_dir.is_empty() {
        let common = program.common_source_directory();
        if !common.is_empty() {
            path = tspath::get_relative_path_from_directory(
                common,
                &path,
                &ComparePathsOptions { use_case_sensitive_file_names: host.fs().use_case_sensitive_file_names(), current_directory: host.get_current_directory().to_string() },
            );
            // Go combines with Options.OutDir here even for declarations (a Go quirk, kept).
            path = tspath::combine_paths(&tspath::resolve_path(host.get_current_directory(), &[&options.out_dir]), &[&path]);
        }
    }
    if ext == tspath::get_declaration_emit_extension_for_path(&path) {
        return outputpaths::change_to_declaration_extension(&path, program);
    }
    tspath::change_extension(&path, ext)
}

// js_emit_baseline.go:174
pub(crate) fn file_output(file: &TestFile, settings: &HarnessOptions) -> String {
    let file_name = if settings.full_emit_paths {
        tsbaseline::remove_test_path_prefixes(&file.unit_name, false /*retainTrailingDirectorySeparator*/)
    } else {
        tspath::get_base_file_name(&file.unit_name)
    };
    format!("//// [{file_name}]\r\n{}", file.content)
}

/// What `DoJSEmitBaseline` needs to recompile (Go: `result.Repeat` and `compileDeclarationFiles` call
/// `CompileFilesEx` again).
pub trait Recompile {
    fn compile(&self, input_files: &[TestFile], other_files: &[TestFile], options: CompilerOptions, tsconfig: Option<P<ParsedCommandLine>>) -> CompilationResult;
}

// js_emit_baseline.go:20. Returns the baseline text (`NO_CONTENT` when empty); a Go `t.Fatal` is an `Err`.
pub fn do_js_emit_baseline(
    header: &str,
    options: &CompilerOptions,
    result: &CompilationResult,
    ts_config_files: &[TestFile],
    to_be_compiled: &[TestFile],
    other_files: &[TestFile],
    harness_settings: &HarnessOptions,
    recompile: &dyn Recompile,
) -> Result<String, String> {
    let outputs = result.emit.as_ref().expect("emit outputs");
    if !options.no_emit.is_true() && !options.emit_declaration_only.is_true() && outputs.js.is_empty() && result.diagnostics.is_empty() {
        return Err("Expected at least one js file to be emitted or at least one error to be created.".to_string());
    }

    // check js output
    let mut ts_code = String::new();
    let ts_sources: Vec<&TestFile> = other_files.iter().chain(to_be_compiled).collect();
    ts_code.push_str("//// [");
    ts_code.push_str(header);
    ts_code.push_str("] ////\r\n\r\n");

    for (i, file) in ts_sources.iter().enumerate() {
        ts_code.push_str("//// [");
        ts_code.push_str(&tspath::get_base_file_name(&file.unit_name));
        ts_code.push_str("]\r\n");
        ts_code.push_str(&file.content);
        if i < ts_sources.len() - 1 {
            ts_code.push_str("\r\n");
        }
    }

    let mut js_code = String::new();
    for file in outputs.js.values() {
        if !js_code.is_empty() && !js_code.ends_with('\n') {
            js_code.push_str("\r\n");
        }
        if result.diagnostics.is_empty() && file.unit_name.ends_with(tspath::EXTENSION_JSON) {
            let file_parse_result = tsrs_parser::parse_source_file(
                SourceFileParseOptions { file_name: file.unit_name.clone(), path: tspath::Path(file.unit_name.as_str().into()), ..Default::default() },
                &file.content,
                ScriptKind::JSON,
            );
            let diags = file_parse_result.diagnostics();
            if !diags.is_empty() {
                js_code.push_str(&tsbaseline::get_error_baseline(std::slice::from_ref(file), &convert_diagnostics(diags), false /*pretty*/));
                continue;
            }
        }
        js_code.push_str(&file_output(file, harness_settings));
    }

    if !outputs.dts.is_empty() {
        js_code.push_str("\r\n\r\n");
        for decl_file in outputs.dts.values() {
            js_code.push_str(&file_output(decl_file, harness_settings));
        }
    }

    let decl_file_context = prepare_declaration_compilation_context(to_be_compiled, other_files, result, options)?;
    if let Some((decl_input_files, decl_other_files)) = decl_file_context {
        // compileDeclarationFiles (js_emit_baseline.go:283): Go passes a tsconfig that carries only the config file
        // (and its content mappers, which tsrs does not support).
        let tsconfig = result.program.command_line().config_file.map(|config_file| {
            let mut c = tsrs_tsoptions::new_parsed_command_line(P::new(CompilerOptions::default()), Vec::new(), Vec::new(), ComparePathsOptions::default());
            c.config_file = Some(config_file);
            P::new(c)
        });
        let decl_result = recompile.compile(&decl_input_files, &decl_other_files, options.clone(), tsconfig);
        if !decl_result.diagnostics.is_empty() {
            js_code.push_str("\r\n\r\n//// [DtsFileErrors]\r\n");
            js_code.push_str("\r\n\r\n");
            let files: Vec<TestFile> = ts_config_files.iter().chain(&decl_input_files).chain(&decl_other_files).cloned().collect();
            js_code.push_str(&tsbaseline::get_error_baseline(&files, &convert_diagnostics(&decl_result.diagnostics), false /*pretty*/));
        }
    }

    if !options.no_check.is_true() && !options.no_emit.is_true() {
        let mut no_check_options = options.clone();
        no_check_options.no_check = Tristate::True;
        let without_checking = recompile.compile(to_be_compiled, other_files, no_check_options, result.tsconfig);
        let without = without_checking.emit.as_ref().expect("emit outputs");
        let mut compare_result_file_sets = |a: &IndexMap<String, TestFile>, b: &IndexMap<String, TestFile>| {
            for (key, doc) in a {
                match b.get(key) {
                    None => {
                        js_code.push_str("\r\n\r\n!!!! File ");
                        js_code.push_str(&tsbaseline::remove_test_path_prefixes(&doc.unit_name, false));
                        js_code.push_str(" missing from original emit, but present in noCheck emit\r\n");
                        js_code.push_str(&file_output(doc, harness_settings));
                    }
                    Some(original) if original.content != doc.content => {
                        js_code.push_str("\r\n\r\n!!!! File ");
                        js_code.push_str(&tsbaseline::remove_test_path_prefixes(&doc.unit_name, false));
                        js_code.push_str(" differs from original emit in noCheck emit\r\n");
                        let file_name = if harness_settings.full_emit_paths {
                            tsbaseline::remove_test_path_prefixes(&doc.unit_name, false)
                        } else {
                            tspath::get_base_file_name(&doc.unit_name)
                        };
                        js_code.push_str("//// [");
                        js_code.push_str(&file_name);
                        js_code.push_str("]\r\n");
                        js_code.push_str(&crate::baseline::diff_text("Expected\tThe full check baseline", "Actual\twith noCheck set", &original.content, &doc.content));
                    }
                    Some(_) => {}
                }
            }
        };
        compare_result_file_sets(&without.dts, &outputs.dts);
        compare_result_file_sets(&without.js, &outputs.js);
    }

    if !js_code.is_empty() {
        Ok(ts_code + "\r\n\r\n" + &js_code)
    } else {
        Ok(NO_CONTENT.to_string())
    }
}

// js_emit_baseline.go:194. Returns the declaration inputs and other files when the emitted `.d.ts` files should be
// recompiled; the Go panics are harness failures (`Err`).
fn prepare_declaration_compilation_context(
    input_files: &[TestFile],
    other_files: &[TestFile],
    result: &CompilationResult,
    options: &CompilerOptions,
) -> Result<Option<(Vec<TestFile>, Vec<TestFile>)>, String> {
    let outputs = result.emit.as_ref().unwrap();
    let program = result.program;
    if options.declaration.is_true() && result.diagnostics.is_empty() {
        if options.emit_declaration_only.is_true() {
            if !outputs.js.is_empty() {
                return Err("panic: Only declaration files should be generated when emitDeclarationOnly:true".to_string());
            }
            if outputs.dts.is_empty() && !options.no_emit.is_true() {
                return Err("panic: Expected at least one declaration file to be emitted when emitDeclarationOnly:true and no errors were generated".to_string());
            }
        } else if outputs.dts.len() != outputs.js.values().filter(|f| !tspath::file_extension_is(&f.unit_name, tspath::EXTENSION_JSON)).count() {
            // (content mappers are not supported, so no source file has one)
            return Err("panic: There were no errors and declFiles generated did not match number of js files generated".to_string());
        }
    }

    let find_unit = |file_name: &str, units: &[TestFile]| units.iter().any(|u| u.unit_name == file_name);

    let find_result_code_file = |file_name: &str| -> Result<Option<TestFile>, String> {
        let Some(source_file) = program.get_source_file(file_name) else {
            return Err(format!("panic: Program has no source file with name '{file_name}'"));
        };
        // Is this file going to be emitted separately
        let source_file_name = if !options.out_dir.is_empty() {
            let mut source_file_path = tspath::get_normalized_absolute_path(source_file.file_name(), result.host.get_current_directory());
            source_file_path = source_file_path.replacen(program.common_source_directory(), "", 1);
            tspath::combine_paths(&options.out_dir, &[&source_file_path])
        } else {
            source_file.file_name().to_string()
        };

        let d_ts_file_name = outputpaths::change_to_declaration_extension(&source_file_name, program);
        Ok(outputs.dts.get(&d_ts_file_name).cloned())
    };

    let mut decl_input_files: Vec<TestFile> = Vec::new();
    let mut decl_other_files: Vec<TestFile> = Vec::new();
    let add_dts_file = |file: &TestFile, into_input: bool, decl_input_files: &mut Vec<TestFile>, decl_other_files: &mut Vec<TestFile>| -> Result<(), String> {
        let dts_files = if into_input { &mut *decl_input_files } else { &mut *decl_other_files };
        if tspath::is_declaration_file_name(&file.unit_name) || tspath::has_json_file_extension(&file.unit_name) {
            dts_files.push(file.clone());
        } else if program.get_source_file(&file.unit_name).is_some()
            && (tspath::has_ts_file_extension(&file.unit_name) || (tspath::has_js_file_extension(&file.unit_name) && options.get_allow_js()))
        {
            if let Some(decl_file) = find_result_code_file(&file.unit_name)? {
                if !find_unit(&decl_file.unit_name, decl_input_files) && !find_unit(&decl_file.unit_name, decl_other_files) {
                    let content = decl_file.content.strip_prefix('\u{FEFF}').unwrap_or(&decl_file.content).to_string();
                    let dts_files = if into_input { &mut *decl_input_files } else { &mut *decl_other_files };
                    dts_files.push(TestFile { unit_name: decl_file.unit_name, content });
                }
            }
        }
        Ok(())
    };

    // if the .d.ts is non-empty, confirm it compiles correctly as well
    if options.declaration.is_true() && result.diagnostics.is_empty() && !outputs.dts.is_empty() {
        for file in input_files {
            add_dts_file(file, true, &mut decl_input_files, &mut decl_other_files)?;
        }
        for file in other_files {
            add_dts_file(file, false, &mut decl_input_files, &mut decl_other_files)?;
        }
        return Ok(Some((decl_input_files, decl_other_files)));
    }
    Ok(None)
}
