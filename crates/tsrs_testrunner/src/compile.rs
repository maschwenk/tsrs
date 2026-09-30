// The Rust compiler backend: newCompilerTest (compiler_runner.go), makeUnitsFromTest's tsconfig parsing
// (test_case_parser.go), and CompileFiles/SetOptionsFromTestConfig/compileFilesWithHost (harnessutil.go),
// type-check only (no emit, so no pre-/post-emit comparison and no declaration emit diagnostics).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use regex::Regex;
use rustc_hash::{FxHashMap, FxHashSet};
use tsrs_ast::{Diagnostic, SourceFile, SourceFileParseOptions};
use tsrs_compiler::{self as compiler, CompilerHost};
use tsrs_core::tspath::{self, ComparePathsOptions, Path};
use tsrs_core::{CompilerOptions, ModuleKind, ModuleResolutionKind, NewLineKind, ScriptKind, ScriptTarget, Tristate, P};
use tsrs_tsoptions::{self as tsoptions, CommandLineOption, CommandLineOptionKind, CompilerOptionsValue, ParsedCommandLine};
use tsrs_vfs::{bundled, vfstest, FS};

use crate::compiler_runner::{self, Outcome, TestItem, SRC_FOLDER};
use crate::diagnosticwriter::{Diag, FileLike};
use crate::harnessutil::{self, HarnessOptions, OptKind, OptionDecl, OptionTable, TestConfiguration, TestFile, TEST_LIB_FOLDER};
use crate::test_case_parser::{self, TestUnit};
use crate::tsbaseline;
use crate::options::{parse_test_ts_config, test_compiler_options, unsupported_reason};


thread_local! {
    // harnessutil sourceFileCache: parsed files are shared across the tests a worker runs (lib files above all).
    static SOURCE_FILE_CACHE: RefCell<FxHashMap<(String, String, bool, String), P<SourceFile>>> = RefCell::new(FxHashMap::default());
}

// cachedCompilerHost
struct CachedCompilerHost {
    inner: Arc<dyn CompilerHost>,
}

impl CompilerHost for CachedCompilerHost {
    fn fs(&self) -> &dyn FS {
        self.inner.fs()
    }
    fn default_library_path(&self) -> &str {
        self.inner.default_library_path()
    }
    fn get_current_directory(&self) -> &str {
        self.inner.get_current_directory()
    }
    fn trace(&self, msg: &'static tsrs_diagnostics::Message, args: &[&dyn std::fmt::Display]) {
        self.inner.trace(msg, args)
    }
    fn get_source_file(&self, opts: SourceFileParseOptions) -> Option<P<SourceFile>> {
        let text = self.fs().read_file(&opts.file_name)?;
        let script_kind = tsrs_core::get_script_kind_from_file_name(&opts.file_name);
        if script_kind == ScriptKind::Unknown {
            panic!("Unknown script kind for file  {}", opts.file_name);
        }
        let key = (opts.file_name.clone(), opts.path.as_str().to_string(), opts.external_module_indicator_options.jsx, format!("{:?}{}", script_kind, text));
        if let Some(cached) = SOURCE_FILE_CACHE.with(|c| c.borrow().get(&key).copied()) {
            return Some(cached);
        }
        let source_file = tsrs_parser::parse_source_file(opts, &text, script_kind);
        SOURCE_FILE_CACHE.with(|c| c.borrow_mut().insert(key, source_file));
        Some(source_file)
    }
    fn get_resolved_project_reference(&self, file_name: &str, path: Path) -> Option<P<ParsedCommandLine>> {
        self.inner.get_resolved_project_reference(file_name, path)
    }
}

fn test_lib_folder_map() -> Vec<(String, String)> {
    let root = compiler_runner::testdata_path().join("tests/lib");
    let mut out = Vec::new();
    fn walk(dir: &std::path::Path, root: &std::path::Path, out: &mut Vec<(String, String)>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, root, out);
            } else if let Ok(b) = std::fs::read(&p) {
                let rel = p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                out.push((format!("{TEST_LIB_FOLDER}/{rel}"), String::from_utf8_lossy(&b).into_owned()));
            }
        }
    }
    walk(&root, &root, &mut out);
    out
}

enum Compiled {
    Result(CompilationResult),
    Unsupported(String),
}

pub struct CompilationResult {
    pub diagnostics: Vec<P<Diagnostic>>,
    pub options: &'static CompilerOptions,
    pub program: &'static compiler::Program,
}

#[allow(clippy::too_many_arguments)]
fn compile_files(
    input_files: &[TestFile],
    other_files: &[TestFile],
    test_config: Option<&TestConfiguration>,
    tsconfig: Option<P<ParsedCommandLine>>,
    current_directory: &str,
    symlinks: &BTreeMap<String, String>,
) -> Result<Compiled, String> {
    let (compiler_options, harness_options) = test_compiler_options(test_config, tsconfig, current_directory)?;
    if let Some(reason) = unsupported_reason(&compiler_options) {
        return Ok(Compiled::Unsupported(reason));
    }

    compile_files_ex(input_files, other_files, &harness_options, compiler_options, current_directory, symlinks, tsconfig).map(Compiled::Result)
}

fn compile_files_ex(
    input_files: &[TestFile],
    other_files: &[TestFile],
    harness_options: &HarnessOptions,
    mut compiler_options: CompilerOptions,
    current_directory: &str,
    symlinks: &BTreeMap<String, String>,
    tsconfig: Option<P<ParsedCommandLine>>,
) -> Result<CompilationResult, String> {
    let mut program_file_names = Vec::new();
    for file in input_files {
        let file_name = tspath::get_normalized_absolute_path(&file.unit_name, current_directory);
        if !tspath::file_extension_is(&file_name, tspath::EXTENSION_JSON) && !tspath::file_extension_is(&file_name, tspath::EXTENSION_TS_BUILD_INFO) {
            program_file_names.push(file_name);
        }
    }

    // Performance optimization; avoid copying in the /.lib folder if the test doesn't need it.
    let mut include_lib_dir = input_files.iter().any(|file| file.content.contains(&format!("{TEST_LIB_FOLDER}/")));

    // Files from testdata\lib that are requested by "@libFiles"
    for lib_file in &harness_options.lib_files {
        if lib_file == "lib.d.ts" && compiler_options.no_lib != Tristate::True {
            // We used to override lib with a custom lib.d.ts for some reason. Skip this unless it becomes necessary.
            continue;
        }
        program_file_names.push(tspath::combine_paths(TEST_LIB_FOLDER, &[lib_file]));
        include_lib_dir = true;
    }

    let abs = |p: &mut String| {
        if !p.is_empty() {
            *p = tspath::get_normalized_absolute_path(p, current_directory);
        }
    };
    abs(&mut compiler_options.out_dir);
    abs(&mut compiler_options.project);
    abs(&mut compiler_options.root_dir);
    abs(&mut compiler_options.ts_build_info_file);
    abs(&mut compiler_options.base_url);
    abs(&mut compiler_options.declaration_dir);
    if let Some(root_dirs) = &mut compiler_options.root_dirs {
        for d in root_dirs.iter_mut() {
            *d = tspath::get_normalized_absolute_path(d, current_directory);
        }
    }
    if let Some(type_roots) = &mut compiler_options.type_roots {
        for d in type_roots.iter_mut() {
            *d = tspath::get_normalized_absolute_path(d, current_directory);
        }
    }

    // Create fake FS for testing
    let mut testfs: BTreeMap<String, vfstest::MapFile> = BTreeMap::new();
    for file in input_files.iter().chain(other_files) {
        testfs.insert(tspath::get_normalized_absolute_path(&file.unit_name, current_directory), vfstest::MapFile::from(file.content.as_str()));
    }
    for (src, target) in symlinks {
        testfs.insert(
            tspath::get_normalized_absolute_path(src, current_directory),
            vfstest::symlink(&tspath::get_normalized_absolute_path(target, current_directory)),
        );
    }
    if include_lib_dir {
        for (k, v) in test_lib_folder_map() {
            testfs.insert(k, vfstest::MapFile::from(v));
        }
    }

    let fs = vfstest::from_map(testfs, harness_options.use_case_sensitive_file_names);
    let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(fs));

    let config_file = tsconfig.and_then(|t| t.config_file);
    let errors = tsconfig.map(|t| t.errors.clone()).unwrap_or_default();
    let mut config = tsoptions::new_parsed_command_line(P::new(compiler_options), program_file_names, Vec::new(), ComparePathsOptions::default());
    config.config_file = config_file;
    config.errors = errors;
    let config = P::new(config);

    let inner = compiler::new_compiler_host(current_directory, fs, &bundled::lib_path(), None, None);
    let host: Arc<dyn CompilerHost> = Arc::new(CachedCompilerHost { inner });
    Ok(compile_files_with_host(host, config, harness_options))
}

fn compile_files_with_host(host: Arc<dyn CompilerHost>, config: P<ParsedCommandLine>, harness_options: &HarnessOptions) -> CompilationResult {
    let mut opts = compiler::ProgramOptions::new(config, host);
    opts.single_threaded = Tristate::True;
    let program = compiler::new_program(opts);
    let mut errors = Vec::new();
    errors.extend(program.get_config_file_parsing_diagnostics());
    errors.extend(program.get_program_diagnostics());
    errors.extend(program.get_syntactic_diagnostics(None));
    if !crate::syntax_only() {
        errors.extend(program.get_semantic_diagnostics(None));
        errors.extend(program.get_global_diagnostics());
    }
    if harness_options.capture_suggestions && !crate::syntax_only() {
        errors.extend(program.get_suggestion_diagnostics(None));
    }
    if program.options().get_emit_declarations() && !crate::syntax_only() {
        errors.extend(program.get_declaration_diagnostics(None));
    }
    let errors = compiler::sort_and_deduplicate_diagnostics(&errors);
    CompilationResult { diagnostics: errors, options: program.options().get(), program }
}

// newCompilerTest + verifyDiagnostics
pub fn run(item: &TestItem, table: &OptionTable) -> Outcome {
    let content = compiler_runner::read_test_file(&item.path);
    let named_configuration = match compiler_runner::find_configuration(&content, table, &item.config) {
        Ok(c) => c,
        Err(e) => return Outcome::Error(e),
    };
    let payload = test_case_parser::make_units_from_test(&content, &item.path);
    if payload.global_options.get("runexternalcode").is_some_and(|v| v == "true") {
        return Outcome::Skip("content mappers (runExternalCode) are not supported".to_string());
    }
    let ts_config = parse_test_ts_config(&payload);

    let mut harness_config: TestConfiguration = named_configuration.map(|c| c.config).unwrap_or_default();
    let split = compiler_runner::split_units(&payload, &mut harness_config, ts_config.map(|t| t.get().parsed_config.file_names.as_slice()));
    let compiler_runner::SplitUnits { current_directory, ts_config_files, to_be_compiled, other_files } = split;

    let result = match compile_files(&to_be_compiled, &other_files, Some(&harness_config), ts_config, &current_directory, &payload.symlinks) {
        Ok(Compiled::Result(r)) => r,
        // Go checks SkipUnsupportedCompilerOptions after compiling; checking first keeps crashes in
        // unsupported configurations (which have no reference baselines) out of the results.
        Ok(Compiled::Unsupported(reason)) => match reason.strip_prefix("fatal: ") {
            Some(fatal) => return Outcome::Error(fatal.to_string()),
            None => return Outcome::Skip(reason),
        },
        Err(e) => return Outcome::Error(e),
    };

    let files: Vec<TestFile> = ts_config_files.into_iter().chain(to_be_compiled).chain(other_files).collect();
    let diags = convert_diagnostics(&result.diagnostics);
    Outcome::Baseline(tsbaseline::do_error_baseline(&files, &diags, result.options.pretty.is_true()))
}

pub fn convert_diagnostics(diagnostics: &[P<Diagnostic>]) -> Vec<Diag> {
    let mut files: FxHashMap<P<SourceFile>, Rc<FileLike>> = FxHashMap::default();
    diagnostics.iter().map(|d| convert(*d, &mut files)).collect()
}

fn convert(d: P<Diagnostic>, files: &mut FxHashMap<P<SourceFile>, Rc<FileLike>>) -> Diag {
    let file = d.file().map(|f| files.entry(f).or_insert_with(|| FileLike::new(f.file_name().to_string(), f.text().to_string())).clone());
    let identity = if !d.message_text().is_empty() {
        d.message_text().to_string()
    } else if let (Some(m), -1) = (d.message(), d.code()) {
        m.text().to_string()
    } else {
        d.message_key().as_str().to_string()
    };
    Diag {
        file,
        pos: d.pos(),
        end: d.end(),
        code: d.code(),
        category: d.category(),
        source: d.source().to_string(),
        message: d.localize(),
        identity,
        args: d.message_args().to_vec(),
        chain: d.message_chain().iter().map(|c| convert(*c, files)).collect(),
        related: d.related_information().iter().map(|r| convert(*r, files)).collect(),
    }
}
