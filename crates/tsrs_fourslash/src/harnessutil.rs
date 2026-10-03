// The part of Go's testutil/harnessutil/harnessutil.go that NewFourslash uses: applying `// @option: value`
// lines to the compiler options of the inferred project, and skipping unsupported option combinations.
// (tsrs_testrunner has its own copy; it is a binary crate.)

use std::sync::LazyLock;

use tsrs_core::tspath;
use tsrs_core::{CompilerOptions, ModuleKind, ModuleResolutionKind, ScriptTarget};
use tsrs_tsoptions::{self as tsoptions, CommandLineOption, CommandLineOptionKind, CompilerOptionsValue};

use crate::testing::T;

// harnessutil.go:64
#[derive(Clone, Debug, Default)]
pub struct HarnessOptions {
    pub use_case_sensitive_file_names: bool,
    pub baseline_file: String,
    pub include_built_file: String,
    pub file_name: String,
    pub lib_files: Vec<String>,
    pub no_implicit_references: bool,
    pub current_directory: String,
    pub symlink: String,
    pub link: String,
    pub no_types_and_symbols: bool,
    pub full_emit_paths: bool,
    pub report_diagnostics: bool,
    pub capture_suggestions: bool,
    pub typescript_version: String,
}

// harnessutil.go:291
pub fn set_options_from_test_config(
    t: &T,
    test_config: &tsrs_core::collections::OrderedMap<String, String>,
    compiler_options: &mut CompilerOptions,
    harness_options: &mut HarnessOptions,
    current_directory: &str,
    allow_unknown_options: bool,
) {
    for (name, value) in test_config {
        if name == "typescriptversion" {
            continue;
        }

        if let Some(command_line_option) = get_command_line_option(name) {
            let parsed_value = get_option_value(t, command_line_option, value, current_directory);
            let errors = tsoptions::parse_compiler_options(command_line_option.name, &parsed_value, compiler_options);
            if !errors.is_empty() {
                t.fatal(&format!("Error parsing value '{value}' for compiler option '{}'.", command_line_option.name));
            }
            continue;
        }
        if let Some(harness_option) = get_harness_option(name) {
            let parsed_value = get_option_value(t, harness_option, value, current_directory);
            parse_harness_option(t, harness_option.name, &parsed_value, harness_options);
            continue;
        }
        if !allow_unknown_options {
            t.fatal(&format!("Unknown compiler option '{name}'."));
        }
    }
}

fn new_option(name: &'static str, kind: CommandLineOptionKind) -> &'static CommandLineOption {
    let mut option = CommandLineOption::DEFAULT;
    option.name = name;
    option.kind = kind;
    Box::leak(Box::new(option))
}

// harnessutil.go:318
static COMPILER_OPTIONS: LazyLock<Vec<&'static CommandLineOption>> = LazyLock::new(|| {
    let extra: [&'static CommandLineOption; 4] = [
        new_option("allowNonTsExtensions", CommandLineOptionKind::Boolean),
        new_option("noErrorTruncation", CommandLineOptionKind::Boolean),
        new_option("suppressOutputPathCheck", CommandLineOptionKind::Boolean),
        new_option("noCheck", CommandLineOptionKind::Boolean),
    ];
    tsoptions::OPTIONS_DECLARATIONS.iter().copied().chain(extra).collect()
});

// harnessutil.go:340
static HARNESS_COMMAND_LINE_OPTIONS: LazyLock<Vec<&'static CommandLineOption>> = LazyLock::new(|| {
    use CommandLineOptionKind::*;
    vec![
        new_option("useCaseSensitiveFileNames", Boolean),
        new_option("baselineFile", String),
        new_option("includeBuiltFile", String),
        new_option("fileName", String),
        new_option("libFiles", List),
        new_option("noImplicitReferences", Boolean),
        new_option("currentDirectory", String),
        new_option("symlink", String),
        new_option("link", String),
        new_option("noTypesAndSymbols", Boolean),
        // Emitted js baseline will print full paths for every output file
        new_option("fullEmitPaths", Boolean),
        // used to enable error collection in `transpile` baselines
        new_option("reportDiagnostics", Boolean),
        // Adds suggestion diagnostics to error baselines
        new_option("captureSuggestions", Boolean),
    ]
});

// harnessutil.go:398
fn get_harness_option(name: &str) -> Option<&'static CommandLineOption> {
    HARNESS_COMMAND_LINE_OPTIONS.iter().copied().find(|option| option.name.eq_ignore_ascii_case(name))
}

// harnessutil.go:404
fn parse_harness_option(t: &T, key: &str, value: &CompilerOptionsValue, harness_options: &mut HarnessOptions) {
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
        _ => t.fatal(&format!("Unknown harness option '{key}'.")),
    }
}

// harnessutil.go:442
fn get_option_value(t: &T, option: &'static CommandLineOption, value: &str, cwd: &str) -> CompilerOptionsValue {
    match option.kind {
        CommandLineOptionKind::String => {
            if option.is_file_path {
                return CompilerOptionsValue::String(tspath::get_normalized_absolute_path(value, cwd));
            }
            CompilerOptionsValue::String(value.to_string())
        }
        CommandLineOptionKind::Number => match value.parse::<i64>() {
            Ok(n) => CompilerOptionsValue::Int(n),
            Err(_) => t.fatal(&format!("Value for option '{}' must be a number, got: {value}", option.name)),
        },
        CommandLineOptionKind::Boolean => match value.to_lowercase().as_str() {
            "true" => CompilerOptionsValue::Bool(true),
            "false" => CompilerOptionsValue::Bool(false),
            _ => t.fatal(&format!("Value for option '{}' must be a boolean, got: {value}", option.name)),
        },
        CommandLineOptionKind::Enum => match option.enum_map().and_then(|m| m.get(value.to_lowercase().as_str())) {
            Some(v) => v.clone(),
            None => {
                let keys: Vec<String> = option.enum_map().map(|m| m.iter().map(|(k, _)| k.to_string()).collect()).unwrap_or_default();
                t.fatal(&format!("Value for option '{}' must be one of {}, got: {value}", option.name, keys.join(",")))
            }
        },
        CommandLineOptionKind::List | CommandLineOptionKind::ListOrElement => {
            let (list_val, errors) = tsoptions::parse_list_type_option(option, value);
            if option.elements().is_some_and(|e| e.is_file_path) {
                return CompilerOptionsValue::Array(
                    list_val.iter().map(|item| CompilerOptionsValue::String(tspath::get_normalized_absolute_path(item.as_str().unwrap_or(""), cwd))).collect(),
                );
            }
            if !errors.is_empty() {
                t.fatal(&format!("Unknown value '{value}' for compiler option '{}'", option.name));
            }
            CompilerOptionsValue::Array(list_val)
        }
        CommandLineOptionKind::Object => t.fatal(&format!("Object type options like '{}' are not supported", option.name)),
    }
}

// harnessutil.go:1182
fn get_command_line_option(option: &str) -> Option<&'static CommandLineOption> {
    COMPILER_OPTIONS.iter().copied().find(|option_decl| option_decl.name.eq_ignore_ascii_case(option))
}

// harnessutil.go:1236
pub fn skip_unsupported_compiler_options(t: &T, options: &CompilerOptions) {
    fail_on_unsupported_compiler_options(t, options);
    match options.module {
        ModuleKind::UMD | ModuleKind::System => t.skip(&format!("unsupported module kind {:?}", options.module)),
        _ => {}
    }
    match options.module_resolution {
        ModuleResolutionKind::Node10 | ModuleResolutionKind::Classic => {
            t.skip(&format!("unsupported module resolution kind {}", options.module_resolution.value()))
        }
        _ => {}
    }
    if options.es_module_interop.is_false() {
        t.skip("esModuleInterop=false is unsupported");
    }
    if options.allow_synthetic_default_imports.is_false() {
        t.skip("allowSyntheticDefaultImports=false is unsupported");
    }
    if !options.base_url.is_empty() {
        t.skip(&format!("unsupported baseUrl {}", options.base_url));
    }
    if options.target == ScriptTarget::ES5 {
        t.skip(&format!("unsupported target {:?}", options.target));
    }
    if options.always_strict.is_false() {
        t.skip("alwaysStrict=false is unsupported");
    }
}

// harnessutil.go:1265
fn fail_on_unsupported_compiler_options(t: &T, options: &CompilerOptions) {
    if options.module == ModuleKind::AMD {
        t.fatal(&format!("unsupported module kind {:?}", options.module));
    }
    if !options.out_file.is_empty() {
        t.fatal(&format!("unsupported outFile {}", options.out_file));
    }
}
