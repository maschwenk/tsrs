// Option handling from harnessutil.go that needs tsoptions: `compilerOptions`, the harness-only options,
// getOptionValue, and compilerVaryBy (compiler_runner.go).

use std::sync::LazyLock;

use rustc_hash::FxHashSet;
use tsrs_core::tspath;
use tsrs_tsoptions::{self as tsoptions, CommandLineOption, CommandLineOptionKind, CompilerOptionsValue};

use std::collections::BTreeMap;

use tsrs_ast::SourceFileParseOptions;
use tsrs_core::tspath::ComparePathsOptions;
use tsrs_core::{CompilerOptions, ModuleKind, ModuleResolutionKind, NewLineKind, ScriptKind, ScriptTarget, Tristate, P};
use tsrs_tsoptions::ParsedCommandLine;
use tsrs_vfs::{vfstest, FS};

use crate::harnessutil::{HarnessOptions, OptKind, OptionDecl, OptionTable, TestConfiguration};
use crate::test_case_parser;

fn new_option(name: &'static str, kind: CommandLineOptionKind) -> &'static CommandLineOption {
    let mut option = CommandLineOption::DEFAULT;
    option.name = name;
    option.kind = kind;
    Box::leak(Box::new(option))
}

// harnessutil `compilerOptions`: the declared options plus harness-only compiler options.
static HARNESS_COMPILER_OPTIONS: LazyLock<Vec<&'static CommandLineOption>> = LazyLock::new(|| {
    let extra: [&'static CommandLineOption; 4] = [
        new_option("allowNonTsExtensions", CommandLineOptionKind::Boolean),
        new_option("noErrorTruncation", CommandLineOptionKind::Boolean),
        new_option("suppressOutputPathCheck", CommandLineOptionKind::Boolean),
        new_option("noCheck", CommandLineOptionKind::Boolean),
    ];
    tsoptions::OPTIONS_DECLARATIONS.iter().copied().chain(extra).collect()
});

static HARNESS_COMMAND_LINE_OPTIONS: LazyLock<Vec<&'static CommandLineOption>> = LazyLock::new(|| {
    let opt = new_option;
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

pub fn get_command_line_option(option: &str) -> Option<&'static CommandLineOption> {
    HARNESS_COMPILER_OPTIONS.iter().copied().find(|d| d.name.eq_ignore_ascii_case(option))
}

pub fn get_harness_option(name: &str) -> Option<&'static CommandLineOption> {
    HARNESS_COMMAND_LINE_OPTIONS.iter().copied().find(|d| d.name.eq_ignore_ascii_case(name))
}

// compilerVaryBy (compiler_runner.go getCompilerVaryByMap) and the option kinds used for variations.
pub fn tsoptions_option_table() -> OptionTable {
    let mut vary_by = FxHashSet::default();
    for option in tsoptions::OPTIONS_DECLARATIONS.iter() {
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

pub fn get_option_value(option: &'static CommandLineOption, value: &str, cwd: &str) -> Result<CompilerOptionsValue, String> {
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

pub fn set_options_from_test_config(
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
pub fn parse_test_ts_config(content: &test_case_parser::TestCaseContent) -> Option<P<ParsedCommandLine>> {
    let data = content.ts_config_file_unit_data.as_ref()?;
    let current_directory = &content.current_directory;
    // unit tests always list files explicitly
    let mut all_files = BTreeMap::new();
    for unit in &content.test_unit_data {
        all_files.insert(tspath::get_normalized_absolute_path(&unit.name, current_directory), unit.content.clone());
    }
    all_files.insert(tspath::get_normalized_absolute_path(&data.name, current_directory), data.content.clone());
    // Config parsing keeps `&'static dyn ParseConfigHost`; the host is leaked like everything in the arena.
    let parse_config_host: &'static VfsParseConfigHost =
        P::new(new_vfs_parse_config_host_with_symlinks(&all_files, &content.symlinks, current_directory, true)).get();

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
        parse_config_host,
        &config_dir,
        None,
        None,
        &config_file_name,
        &[],
        None,
    )))
}


// The option setup of harnessutil.CompileFiles: tsconfig options (if any), the test defaults, then the
// test configuration.
pub fn test_compiler_options(
    test_config: Option<&TestConfiguration>,
    tsconfig: Option<P<ParsedCommandLine>>,
    current_directory: &str,
) -> Result<(CompilerOptions, HarnessOptions), String> {
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
    Ok((compiler_options, harness_options))
}

pub fn unsupported_reason(options: &CompilerOptions) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler_runner::{self, repo_root};

    // For every variant: option parsing must succeed, the unsupported-option skip decision must agree with
    // the Go harness (tools/oracle/testrunner `names`), and with a tsconfig.json the compiled/other split
    // must reproduce the Go file order (`diags`). Skipped when the oracle files are absent.
    #[test]
    fn options_and_tsconfig_match_go_harness() {
        let scratch = repo_root().join("target/scratch/testrunner");
        let (Ok(names), Ok(diags)) = (std::fs::read_to_string(scratch.join("names.jsonl")), std::fs::read_to_string(scratch.join("diags.jsonl"))) else {
            eprintln!("oracle files missing; skipping");
            return;
        };
        let mut go_skip = rustc_hash::FxHashMap::default();
        for line in names.lines() {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            for variant in v["variants"].as_array().into_iter().flatten() {
                let stem = compiler_runner::baseline_stem(variant["name"].as_str().unwrap());
                go_skip.insert(format!("{}/{stem}", v["suite"].as_str().unwrap()), variant.get("skip").is_some());
            }
        }
        let mut go_files = rustc_hash::FxHashMap::default();
        for line in diags.lines() {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            let stem = compiler_runner::baseline_stem(v["name"].as_str().unwrap());
            let files: Vec<String> = v["files"].as_array().into_iter().flatten().map(|f| f["name"].as_str().unwrap().to_string()).collect();
            go_files.insert(format!("{}/{stem}", v["suite"].as_str().unwrap()), files);
        }
        let table = tsoptions_option_table();
        let mut problems = Vec::new();
        let mut checked = 0;
        for suite in compiler_runner::SUITES {
            for path in compiler_runner::enumerate_test_files(suite) {
                for item in compiler_runner::expand_test_file(suite, &path, &table).unwrap() {
                    let content = compiler_runner::read_test_file(&path);
                    let payload = test_case_parser::make_units_from_test(&content, &path);
                    if payload.global_options.get("runexternalcode").is_some_and(|v| v == "true") {
                        continue;
                    }
                    let mut config =
                        compiler_runner::find_configuration(&content, &table, &item.config).unwrap().map(|c| c.config).unwrap_or_default();
                    let ts_config = parse_test_ts_config(&payload);
                    let split = compiler_runner::split_units(&payload, &mut config, ts_config.map(|t| t.get().parsed_config.file_names.as_slice()));
                    checked += 1;
                    match test_compiler_options(Some(&config), ts_config, &split.current_directory) {
                        Err(e) => problems.push(format!("{}: {e}", item.id())),
                        Ok((options, _)) => {
                            let skip = unsupported_reason(&options).is_some();
                            if go_skip.get(&item.id()).copied() != Some(skip) {
                                problems.push(format!("{}: skip={skip} ({:?})", item.id(), unsupported_reason(&options)));
                            }
                        }
                    }
                    if ts_config.is_some() {
                        let mine: Vec<String> =
                            split.ts_config_files.iter().chain(&split.to_be_compiled).chain(&split.other_files).map(|f| f.unit_name.clone()).collect();
                        if let Some(go) = go_files.get(&item.id()) {
                            if !go.is_empty() && *go != mine {
                                problems.push(format!("{}: files {mine:?} vs go {go:?}", item.id()));
                            }
                        }
                    }
                }
            }
        }
        assert!(problems.is_empty(), "{} of {checked} differ:\n{}", problems.len(), problems[..problems.len().min(15)].join("\n"));
        eprintln!("{checked} variants agree");
    }
}
