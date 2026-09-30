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

const REQUIRE_STR: &str = "require(";
static REFERENCES_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"reference[\t\n\x0C\r ]path").unwrap());

// harnessutil `compilerOptions`: the declared options plus harness-only compiler options.
static HARNESS_COMPILER_OPTIONS: LazyLock<Vec<&'static CommandLineOption>> = LazyLock::new(|| {
    let extra: [&'static CommandLineOption; 4] = [
        Box::leak(Box::new(CommandLineOption { name: "allowNonTsExtensions", kind: CommandLineOptionKind::Boolean, ..CommandLineOption::DEFAULT })),
        Box::leak(Box::new(CommandLineOption { name: "noErrorTruncation", kind: CommandLineOptionKind::Boolean, ..CommandLineOption::DEFAULT })),
        Box::leak(Box::new(CommandLineOption { name: "suppressOutputPathCheck", kind: CommandLineOptionKind::Boolean, ..CommandLineOption::DEFAULT })),
        Box::leak(Box::new(CommandLineOption { name: "noCheck", kind: CommandLineOptionKind::Boolean, ..CommandLineOption::DEFAULT })),
    ];
    tsoptions::OptionsDeclarations.iter().copied().chain(extra).collect()
});

static HARNESS_COMMAND_LINE_OPTIONS: LazyLock<Vec<&'static CommandLineOption>> = LazyLock::new(|| {
    let opt = |name: &'static str, kind: CommandLineOptionKind| -> &'static CommandLineOption {
        Box::leak(Box::new(CommandLineOption { name, kind, ..CommandLineOption::DEFAULT }))
    };
    use CommandLineOptionKind::*;
    vec![
        opt("useCaseSensitiveFileNames", Boolean),
        opt("baselineFile", String),
        opt("includeBuiltFile", String),
        opt("fileName", String),
        opt("libFiles", List),
        opt("noImplicitReferences", Boolean),
        opt("currentDirectory", String),
        opt("symlink", String),
        opt("link", String),
        opt("noTypesAndSymbols", Boolean),
        // Emitted js baseline will print full paths for every output file
        opt("fullEmitPaths", Boolean),
        // used to enable error collection in `transpile` baselines
        opt("reportDiagnostics", Boolean),
        // Adds suggestion diagnostics to error baselines
        opt("captureSuggestions", Boolean),
    ]
});

fn get_command_line_option(option: &str) -> Option<&'static CommandLineOption> {
    HARNESS_COMPILER_OPTIONS.iter().copied().find(|d| d.name.eq_ignore_ascii_case(option))
}

fn get_harness_option(name: &str) -> Option<&'static CommandLineOption> {
    HARNESS_COMMAND_LINE_OPTIONS.iter().copied().find(|d| d.name.eq_ignore_ascii_case(name))
}

// compilerVaryBy (compiler_runner.go getCompilerVaryByMap) and the option kinds used for variations.
pub fn tsoptions_option_table() -> OptionTable {
    let mut vary_by = FxHashSet::default();
    for option in tsoptions::OptionsDeclarations.iter() {
        if !option.is_command_line_only
            && (option.kind == CommandLineOptionKind::Boolean || option.kind == CommandLineOptionKind::Enum)
            && (option.affects_program_structure
                || option.affects_emit
                || option.affects_module_resolution
                || option.affects_bind_diagnostics
                || option.affects_semantic_diagnostics
                || option.affects_source_file
                || option.affects_declaration_path
                || option.affects_build_info)
        {
            vary_by.insert(option.name.to_lowercase());
        }
    }
    // explicit variations that do not match above conditions
    vary_by.insert("noemit".to_string());
    vary_by.insert("isolatedmodules".to_string());

    let decls = HARNESS_COMPILER_OPTIONS
        .iter()
        .map(|o| OptionDecl {
            name: o.name.to_string(),
            kind: match o.kind {
                CommandLineOptionKind::Boolean => OptKind::Boolean,
                CommandLineOptionKind::Enum => {
                    OptKind::Enum(o.enum_map().map(|m| m.iter().map(|(k, v)| (k.to_string(), format!("{v:?}"))).collect()).unwrap_or_default())
                }
                _ => OptKind::Other,
            },
        })
        .collect();
    OptionTable { decls, vary_by }
}

fn get_option_value(option: &'static CommandLineOption, value: &str, cwd: &str) -> Result<CompilerOptionsValue, String> {
    Ok(match option.kind {
        CommandLineOptionKind::String => {
            if option.is_file_path {
                CompilerOptionsValue::String(tspath::get_normalized_absolute_path(value, cwd))
            } else {
                CompilerOptionsValue::String(value.to_string())
            }
        }
        CommandLineOptionKind::Number => match value.parse::<i64>() {
            Ok(n) => CompilerOptionsValue::Int(n),
            Err(_) => return Err(format!("Value for option '{}' must be a number, got: {value}", option.name)),
        },
        CommandLineOptionKind::Boolean => match value.to_lowercase().as_str() {
            "true" => CompilerOptionsValue::Bool(true),
            "false" => CompilerOptionsValue::Bool(false),
            _ => return Err(format!("Value for option '{}' must be a boolean, got: {value}", option.name)),
        },
        CommandLineOptionKind::Enum => match option.enum_map().and_then(|m| m.get(value.to_lowercase().as_str())) {
            Some(v) => v.clone(),
            None => return Err(format!("Value for option '{}' must be one of the enum values, got: {value}", option.name)),
        },
        CommandLineOptionKind::List | CommandLineOptionKind::ListOrElement => {
            let (list_val, errors) = tsoptions::parse_list_type_option(option, value);
            if option.elements().is_some_and(|e| e.is_file_path) {
                return Ok(CompilerOptionsValue::Array(
                    list_val
                        .iter()
                        .map(|item| CompilerOptionsValue::String(tspath::get_normalized_absolute_path(item.as_str().unwrap_or(""), cwd)))
                        .collect(),
                ));
            }
            if !errors.is_empty() {
                return Err(format!("Unknown value '{value}' for compiler option '{}'", option.name));
            }
            CompilerOptionsValue::Array(list_val)
        }
        CommandLineOptionKind::Object => return Err(format!("Object type options like '{}' are not supported", option.name)),
    })
}

fn parse_harness_option(key: &str, value: &CompilerOptionsValue, harness_options: &mut HarnessOptions) -> Result<(), String> {
    let b = || matches!(value, CompilerOptionsValue::Bool(true));
    let s = || value.as_str().unwrap_or("").to_string();
    match key {
        "useCaseSensitiveFileNames" => harness_options.use_case_sensitive_file_names = b(),
        "baselineFile" => harness_options.baseline_file = s(),
        "includeBuiltFile" => harness_options.include_built_file = s(),
        "fileName" => harness_options.file_name = s(),
        "libFiles" => {
            harness_options.lib_files = value.as_array().map(|a| a.iter().map(|v| v.as_str().unwrap_or("").to_string()).collect()).unwrap_or_default()
        }
        "noImplicitReferences" => harness_options.no_implicit_references = b(),
        "currentDirectory" => harness_options.current_directory = s(),
        "symlink" => harness_options.symlink = s(),
        "link" => harness_options.link = s(),
        "noTypesAndSymbols" => harness_options.no_types_and_symbols = b(),
        "fullEmitPaths" => harness_options.full_emit_paths = b(),
        "reportDiagnostics" => harness_options.report_diagnostics = b(),
        "captureSuggestions" => harness_options.capture_suggestions = b(),
        "typescriptVersion" => harness_options.typescript_version = s(),
        _ => return Err(format!("Unknown harness option '{key}'.")),
    }
    Ok(())
}

fn set_options_from_test_config(
    test_config: &TestConfiguration,
    compiler_options: &mut CompilerOptions,
    harness_options: &mut HarnessOptions,
    current_directory: &str,
) -> Result<(), String> {
    for (name, value) in test_config {
        if name == "typescriptversion" {
            continue;
        }
        if let Some(command_line_option) = get_command_line_option(name) {
            let parsed_value = get_option_value(command_line_option, value, current_directory)?;
            let errors = tsoptions::parse_compiler_options(command_line_option.name, &parsed_value, compiler_options);
            if !errors.is_empty() {
                return Err(format!("Error parsing value '{value}' for compiler option '{}'.", command_line_option.name));
            }
            continue;
        }
        if let Some(harness_option) = get_harness_option(name) {
            let parsed_value = get_option_value(harness_option, value, current_directory)?;
            parse_harness_option(harness_option.name, &parsed_value, harness_options)?;
            continue;
        }
        return Err(format!("Unknown compiler option '{name}'."));
    }
    Ok(())
}

// tsoptionstest.VfsParseConfigHost
struct VfsParseConfigHost {
    vfs: Box<dyn FS>,
    current_directory: String,
}

impl tsoptions::ParseConfigHost for VfsParseConfigHost {
    fn fs(&self) -> &dyn FS {
        &*self.vfs
    }
    fn get_current_directory(&self) -> &str {
        &self.current_directory
    }
}

fn new_vfs_parse_config_host_with_symlinks(
    files: &BTreeMap<String, String>,
    symlinks: &BTreeMap<String, String>,
    current_directory: &str,
    use_case_sensitive_file_names: bool,
) -> VfsParseConfigHost {
    let mut entries: Vec<(String, vfstest::MapFile)> = files.iter().map(|(k, v)| (k.clone(), vfstest::MapFile::from(v.as_str()))).collect();
    for (link, target) in symlinks {
        entries.push((
            tspath::get_normalized_absolute_path(link, current_directory),
            vfstest::symlink(&tspath::get_normalized_absolute_path(target, current_directory)),
        ));
    }
    VfsParseConfigHost { vfs: Box::new(vfstest::from_map(entries, use_case_sensitive_file_names)), current_directory: current_directory.to_string() }
}

// The tsconfig half of makeUnitsFromTest.
fn parse_test_ts_config(content: &test_case_parser::TestCaseContent) -> Option<P<ParsedCommandLine>> {
    let data = content.ts_config_file_unit_data.as_ref()?;
    let current_directory = &content.current_directory;
    // unit tests always list files explicitly
    let mut all_files = BTreeMap::new();
    for unit in &content.test_unit_data {
        all_files.insert(tspath::get_normalized_absolute_path(&unit.name, current_directory), unit.content.clone());
    }
    all_files.insert(tspath::get_normalized_absolute_path(&data.name, current_directory), data.content.clone());
    let parse_config_host = new_vfs_parse_config_host_with_symlinks(&all_files, &content.symlinks, current_directory, true);

    // Content mappers are gated behind --runExternalCode (not supported by tsrs).
    let config_file_name = tspath::get_normalized_absolute_path(&data.name, current_directory);
    let path = tspath::to_path(&data.name, current_directory, true);
    let config_json = tsrs_parser::parse_source_file(
        SourceFileParseOptions { file_name: config_file_name.clone(), path, ..Default::default() },
        &data.content,
        ScriptKind::JSON,
    );
    let ts_config_source_file = tsoptions::TsConfigSourceFile::new(config_json);
    let config_dir = tspath::get_directory_path(&config_file_name);
    Some(P::new(tsoptions::parse_json_source_file_config_file_content(
        ts_config_source_file,
        &parse_config_host,
        &config_dir,
        None,
        None,
        &config_file_name,
        &[],
        None,
    )))
}

fn create_harness_test_file(unit: &TestUnit, current_directory: &str) -> TestFile {
    TestFile { unit_name: tspath::get_normalized_absolute_path(&unit.name, current_directory), content: unit.content.clone() }
}

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
) -> Result<CompilationResult, String> {
    let mut compiler_options: CompilerOptions = tsconfig.and_then(|t| t.compiler_options()).map(|o| (*o).clone()).unwrap_or_default();
    // Set default options for tests
    if compiler_options.new_line == NewLineKind::None {
        compiler_options.new_line = NewLineKind::CRLF;
    }
    if compiler_options.skip_default_lib_check == Tristate::Unknown {
        compiler_options.skip_default_lib_check = Tristate::True;
    }
    compiler_options.no_error_truncation = Tristate::True;
    let mut harness_options = HarnessOptions { use_case_sensitive_file_names: true, current_directory: current_directory.to_string(), ..Default::default() };

    // Parse harness and compiler options from the test configuration
    if let Some(test_config) = test_config {
        set_options_from_test_config(test_config, &mut compiler_options, &mut harness_options, current_directory)?;
    }

    compile_files_ex(input_files, other_files, &harness_options, compiler_options, current_directory, symlinks, tsconfig)
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
    errors.extend(program.get_semantic_diagnostics(None));
    errors.extend(program.get_global_diagnostics());
    if harness_options.capture_suggestions {
        todo!("GetSuggestionDiagnostics (captureSuggestions)");
    }
    if program.options().get_emit_declarations() {
        errors.extend(program.get_declaration_diagnostics(None));
    }
    let errors = compiler::sort_and_deduplicate_diagnostics(&errors);
    CompilationResult { diagnostics: errors, options: program.options(), program }
}

fn unsupported_reason(options: &CompilerOptions) -> Option<String> {
    // failOnUnsupportedCompilerOptions
    if options.module == ModuleKind::AMD {
        return Some("fatal: unsupported module kind AMD".to_string());
    }
    if !options.out_file.is_empty() {
        return Some(format!("fatal: unsupported outFile {}", options.out_file));
    }
    // SkipUnsupportedCompilerOptions
    match options.module {
        ModuleKind::UMD | ModuleKind::System => return Some(format!("unsupported module kind {:?}", options.module)),
        _ => {}
    }
    match options.module_resolution {
        ModuleResolutionKind::Node10 | ModuleResolutionKind::Classic => {
            return Some(format!("unsupported module resolution kind {:?}", options.module_resolution))
        }
        _ => {}
    }
    if options.es_module_interop == Tristate::False {
        return Some("esModuleInterop=false is unsupported".to_string());
    }
    if options.allow_synthetic_default_imports == Tristate::False {
        return Some("allowSyntheticDefaultImports=false is unsupported".to_string());
    }
    if !options.base_url.is_empty() {
        return Some(format!("unsupported baseUrl {}", options.base_url));
    }
    if options.target == ScriptTarget::ES5 {
        return Some("unsupported target ES5".to_string());
    }
    if options.always_strict == Tristate::False {
        return Some("alwaysStrict=false is unsupported".to_string());
    }
    None
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
    let current_directory = tspath::get_normalized_absolute_path(harness_config.get("currentdirectory").map_or("", String::as_str), SRC_FOLDER);

    let units = &payload.test_unit_data;
    let mut to_be_compiled = Vec::new();
    let mut other_files = Vec::new();
    let mut ts_config_files = Vec::new();
    if let Some(ts_config) = ts_config {
        ts_config_files.push(create_harness_test_file(payload.ts_config_file_unit_data.as_ref().unwrap(), &current_directory));
        for unit in units {
            if ts_config.parsed_config.file_names.contains(&tspath::get_normalized_absolute_path(&unit.name, &current_directory)) {
                to_be_compiled.push(create_harness_test_file(unit, &current_directory));
            } else {
                other_files.push(create_harness_test_file(unit, &current_directory));
            }
        }
    } else {
        if let Some(base_url) = harness_config.get("baseurl").cloned() {
            if !tspath::is_rooted_disk_path(&base_url) {
                harness_config.insert("baseurl".to_string(), tspath::get_normalized_absolute_path(&base_url, &current_directory));
            }
        }

        let last_unit = units.last().unwrap();
        // We need to assemble the list of input files for the compiler and other related files on the 'filesystem' (ie in a multi-file test)
        // If the last file in a test uses require or a triple slash reference we'll assume all other files will be brought in via references,
        // otherwise, assume all files are just meant to be in the same compilation session without explicit references to one another.
        if harness_config.get("noimplicitreferences").is_some_and(|v| !v.is_empty())
            || last_unit.content.contains(REQUIRE_STR)
            || REFERENCES_REGEX.is_match(&last_unit.content)
        {
            to_be_compiled.push(create_harness_test_file(last_unit, &current_directory));
            for unit in &units[..units.len() - 1] {
                other_files.push(create_harness_test_file(unit, &current_directory));
            }
        } else {
            to_be_compiled = units.iter().map(|unit| create_harness_test_file(unit, &current_directory)).collect();
        }
    }

    let result = match compile_files(&to_be_compiled, &other_files, Some(&harness_config), ts_config, &current_directory, &payload.symlinks) {
        Ok(r) => r,
        Err(e) => return Outcome::Error(e),
    };

    if let Some(reason) = unsupported_reason(result.options) {
        if let Some(fatal) = reason.strip_prefix("fatal: ") {
            return Outcome::Error(fatal.to_string());
        }
        return Outcome::Skip(reason);
    }

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
