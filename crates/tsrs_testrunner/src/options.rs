// Option handling from harnessutil.go that needs tsoptions: `compilerOptions`, the harness-only options,
// getOptionValue, and compilerVaryBy (compiler_runner.go).

use std::sync::LazyLock;

use rustc_hash::FxHashSet;
use tsrs_core::tspath;
use tsrs_tsoptions::{self as tsoptions, CommandLineOption, CommandLineOptionKind, CompilerOptionsValue};

use crate::harnessutil::{OptKind, OptionDecl, OptionTable};

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

