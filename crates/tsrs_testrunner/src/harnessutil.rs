use std::collections::BTreeMap;
use std::path::Path;

use regex::Regex;
use tsrs_core::tspath;

// Posix-style path to additional test libraries
pub const TEST_LIB_FOLDER: &str = "/.lib";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestFile {
    pub unit_name: String,
    pub content: String,
}

// This maps a compiler setting to its string value, after splitting by commas,
// handling inclusions and exclusions, and deduplicating.
// For example, if a test file contains:
//
//	// @target: esnext, es2015
//
// Then the map will map "target" to "esnext", and another map will map "target" to "es2015".
pub type TestConfiguration = BTreeMap<String, String>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedTestConfiguration {
    pub name: String,
    pub config: TestConfiguration,
}

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

// The subset of a tsoptions.CommandLineOption that the configuration logic needs. `Enum` carries the
// enum map in declaration order as (key, normalized value identity).
#[derive(Clone, Debug)]
pub enum OptKind {
    Boolean,
    Enum(Vec<(String, String)>),
    Other,
}

#[derive(Clone, Debug)]
pub struct OptionDecl {
    pub name: String,
    pub kind: OptKind,
}

// `compilerOptions` (tsoptions.OptionsDeclarations + the harness-only compiler options) together with the
// set of options for which variations are allowed (`compilerVaryBy`, lower-cased names).
pub struct OptionTable {
    pub decls: Vec<OptionDecl>,
    pub vary_by: rustc_hash::FxHashSet<String>,
}

impl OptionTable {
    pub fn get_command_line_option(&self, option: &str) -> Option<&OptionDecl> {
        self.decls.iter().find(|d| d.name.eq_ignore_ascii_case(option))
    }
}

pub fn get_config_name_from_file_name(filename: &str) -> String {
    let basename_lower = tspath::get_base_file_name(filename).to_lowercase();
    if basename_lower == "tsconfig.json" || basename_lower == "jsconfig.json" {
        return basename_lower;
    }
    String::new()
}

pub fn enumerate_files(folder: &Path, test_regex: &Regex, recursive: bool) -> std::io::Result<Vec<String>> {
    let mut paths = Vec::new();
    list_files_worker(test_regex, recursive, folder, &mut paths)?;
    Ok(paths)
}

fn list_files_worker(spec: &Regex, recursive: bool, folder: &Path, paths: &mut Vec<String>) -> std::io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(folder)?.collect::<Result<_, _>>()?;
    // os.ReadDir returns entries sorted by filename.
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        if !entry.file_type()?.is_dir() {
            let p = tspath::normalize_path(&path.to_string_lossy());
            if spec.is_match(&p) {
                paths.push(p);
            }
        } else if recursive {
            list_files_worker(spec, recursive, &path, paths)?;
        }
    }
    Ok(())
}

fn get_file_based_test_configuration_description(config: &TestConfiguration) -> String {
    let mut output = String::new();
    for (i, (key, value)) in config.iter().enumerate() {
        if i > 0 {
            output.push(',');
        }
        output.push_str(key);
        output.push('=');
        output.push_str(&value.to_lowercase());
    }
    output
}

pub fn get_file_based_test_configurations(
    settings: &BTreeMap<String, String>,
    table: &OptionTable,
) -> Result<Vec<NamedTestConfiguration>, String> {
    // Each element has the option name as the first element, and the values as the rest
    let mut option_entries: Vec<Vec<String>> = Vec::new();
    let mut variation_count = 1usize;
    let mut non_varying_options = TestConfiguration::new();
    for (option, value) in settings {
        if table.vary_by.contains(option) {
            let entries = split_option_values(value, option, table)?;
            if entries.len() > 1 {
                variation_count *= entries.len();
                if variation_count > 25 {
                    return Err("Provided test options exceeded the maximum number of variations".to_string());
                }
                let mut e = vec![option.clone()];
                e.extend(entries);
                option_entries.push(e);
            } else if entries.len() == 1 {
                non_varying_options.insert(option.clone(), entries.into_iter().next().unwrap());
            }
        } else {
            // Variation is not supported for the option
            non_varying_options.insert(option.clone(), value.clone());
        }
    }

    let mut configurations = Vec::new();
    if !option_entries.is_empty() {
        // Merge varying and non-varying options
        let varying_configurations = compute_file_based_test_configuration_variations(variation_count, &option_entries);
        for mut varying_config in varying_configurations {
            let description = get_file_based_test_configuration_description(&varying_config);
            for (k, v) in &non_varying_options {
                varying_config.insert(k.clone(), v.clone());
            }
            configurations.push(NamedTestConfiguration { name: description, config: varying_config });
        }
    } else if !non_varying_options.is_empty() {
        // Only non-varying options
        configurations.push(NamedTestConfiguration { name: String::new(), config: non_varying_options });
    }
    Ok(configurations)
}

// Splits a string value into an array of strings, each corresponding to a unique value for the given option.
// Also handles the `*` value, which includes all possible values for the option, and exclusions using `-` or `!`.
//
//	splitOptionValues("esnext, es2015, es6", "target") => ["esnext", "es2015"]
//	splitOptionValues("*", "strict") => ["true", "false"]
//	splitOptionValues("*, -true", "strict") => ["false"]
fn split_option_values(value: &str, option: &str, table: &OptionTable) -> Result<Vec<String>, String> {
    if value.is_empty() {
        return Ok(Vec::new());
    }

    let mut star = false;
    let mut includes = Vec::new();
    let mut excludes = Vec::new();
    for s in value.split(',') {
        let s = s.trim();
        if s.is_empty() {
            continue;
        }
        if s == "*" {
            star = true;
        } else if s.starts_with('-') || s.starts_with('!') {
            excludes.push(&s[1..]);
        } else {
            includes.push(s);
        }
    }

    if includes.is_empty() && !star && excludes.is_empty() {
        return Ok(Vec::new());
    }

    // Dedupe the variations by their normalized values. (Go collects map values, so the resulting order is
    // unspecified; configuration names do not depend on it.)
    let mut variations: Vec<(String, String)> = Vec::new();
    let mut add = |value: String, include: &str| {
        if !variations.iter().any(|(v, _)| *v == value) {
            variations.push((value, include.to_string()));
        }
    };

    // add (and deduplicate) all included entries
    for include in &includes {
        let value = get_value_of_option_string(option, include, table)?;
        add(value, include);
    }

    let all_values = get_all_values_for_option(option, table);
    if star && !all_values.is_empty() {
        // add all entries
        for include in &all_values {
            let value = get_value_of_option_string(option, include, table)?;
            add(value, include);
        }
    }

    // remove all excluded entries
    for exclude in excludes {
        // The excluded value may not be recognized (e.g., a removed option like "es3"); there is nothing to remove then.
        if let Some(value) = try_get_value_of_option_string(option, exclude, table) {
            variations.retain(|(v, _)| *v != value);
        }
    }

    if variations.is_empty() {
        return Err(format!("Variations in test option '@{option}' resulted in an empty set."));
    }
    Ok(variations.into_iter().map(|(_, include)| include).collect())
}

fn get_value_of_option_string(option: &str, value: &str, table: &OptionTable) -> Result<String, String> {
    try_get_value_of_option_string(option, value, table).ok_or_else(|| format!("Unknown value '{value}' for option '{option}'"))
}

// Returns an identity for the normalized option value (Go compares tsoptions.CompilerOptionsValue).
fn try_get_value_of_option_string(option: &str, value: &str, table: &OptionTable) -> Option<String> {
    let option_decl = table.get_command_line_option(option)?;
    match &option_decl.kind {
        OptKind::Enum(map) => {
            let lower = value.to_lowercase();
            map.iter().find(|(k, _)| *k == lower).map(|(_, v)| format!("enum:{v}"))
        }
        OptKind::Boolean => match value.to_lowercase().as_str() {
            "true" => Some("bool:true".to_string()),
            "false" => Some("bool:false".to_string()),
            _ => None,
        },
        OptKind::Other => Some(format!("str:{value}")),
    }
}

fn get_all_values_for_option(option: &str, table: &OptionTable) -> Vec<String> {
    let Some(option_decl) = table.get_command_line_option(option) else {
        return Vec::new();
    };
    match &option_decl.kind {
        OptKind::Enum(map) => map.iter().map(|(k, _)| k.clone()).collect(),
        OptKind::Boolean => vec!["true".to_string(), "false".to_string()],
        OptKind::Other => Vec::new(),
    }
}

fn compute_file_based_test_configuration_variations(variation_count: usize, option_entries: &[Vec<String>]) -> Vec<TestConfiguration> {
    let mut configurations = Vec::with_capacity(variation_count);
    compute_file_based_test_configuration_variations_worker(&mut configurations, option_entries, 0, &mut TestConfiguration::new());
    configurations
}

fn compute_file_based_test_configuration_variations_worker(
    configurations: &mut Vec<TestConfiguration>,
    option_entries: &[Vec<String>],
    index: usize,
    variation_state: &mut TestConfiguration,
) {
    if index >= option_entries.len() {
        configurations.push(variation_state.clone());
        return;
    }

    let option_key = &option_entries[index][0];
    for entry in &option_entries[index][1..] {
        // set or overwrite the variation, then compute the next variation
        variation_state.insert(option_key.clone(), entry.clone());
        compute_file_based_test_configuration_variations_worker(configurations, option_entries, index + 1, variation_state);
    }
}

// Reason a configuration is skipped (`SkipUnsupportedCompilerOptions`) or fails outright
// (`failOnUnsupportedCompilerOptions`, reported with a "fatal: " prefix). Empty if supported.
pub struct UnsupportedOptions<'a> {
    pub module: &'a str,
    pub module_resolution: &'a str,
    pub out_file: bool,
    pub es_module_interop_false: bool,
    pub allow_synthetic_default_imports_false: bool,
    pub base_url: bool,
    pub target: &'a str,
    pub always_strict_false: bool,
}

pub fn skip_unsupported_compiler_options(o: &UnsupportedOptions) -> String {
    if o.module == "amd" {
        return "fatal: unsupported module kind AMD".to_string();
    }
    if o.out_file {
        return "fatal: unsupported outFile".to_string();
    }
    match o.module {
        "umd" | "system" => return format!("unsupported module kind {}", o.module),
        _ => {}
    }
    match o.module_resolution {
        "node10" | "classic" => return format!("unsupported module resolution kind {}", o.module_resolution),
        _ => {}
    }
    if o.es_module_interop_false {
        return "esModuleInterop=false is unsupported".to_string();
    }
    if o.allow_synthetic_default_imports_false {
        return "allowSyntheticDefaultImports=false is unsupported".to_string();
    }
    if o.base_url {
        return "unsupported baseUrl".to_string();
    }
    if o.target == "es5" {
        return "unsupported target ES5".to_string();
    }
    if o.always_strict_false {
        return "alwaysStrict=false is unsupported".to_string();
    }
    String::new()
}
