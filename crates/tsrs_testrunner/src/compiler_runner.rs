// internal/testrunner/compiler_runner.go: test enumeration, configurations and per-variant execution.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use tsrs_core::tspath;

use crate::harnessutil::{self, NamedTestConfiguration, OptionTable, TestConfiguration, TestFile};
use crate::test_case_parser::{self, TestCaseContent, TestUnit};

// Posix-style path to sources under test
pub const SRC_FOLDER: &str = "/.src";

pub const SUITES: [&str; 2] = ["compiler", "conformance"];

const REQUIRE_STR: &str = "require(";
static REFERENCES_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"reference[\t\n\x0C\r ]path").unwrap());

static COMPILER_BASELINE_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.tsx?$").unwrap());

pub static SKIPPED_TESTS: &[&str] = &[
    // Tests that depended on typescript.d.ts in built.
    "APILibCheck.ts",
    "APISample_Watch.ts",
    "APISample_WatchWithDefaults.ts",
    "APISample_WatchWithOwnWatchHost.ts",
    "APISample_compile.ts",
    "APISample_jsdoc.ts",
    "APISample_linter.ts",
    "APISample_parseConfig.ts",
    "APISample_transform.ts",
    "APISample_watcher.ts",
    // These tests contain options that have been completely removed, so fail to parse.
    "preserveUnusedImports.ts",
    "noCrashWithVerbatimModuleSyntaxAndImportsNotUsedAsValues.ts",
    "verbatimModuleSyntaxCompat.ts",
    "verbatimModuleSyntaxCompat2.ts",
    "verbatimModuleSyntaxCompat3.ts",
    "verbatimModuleSyntaxCompat4.ts",
    "preserveValueImports.ts",
    "preserveValueImports_importsNotUsedAsValues.ts",
    "preserveValueImports_errors.ts",
    "preserveValueImports_mixedImports.ts",
    "preserveValueImports_module.ts",
    "importsNotUsedAsValues_error.ts",
    "alwaysStrictNoImplicitUseStrict.ts",
    "nonPrimitiveIndexingWithForInSupressError.ts",
    "parameterInitializerBeforeDestructuringEmit.ts",
    "mappedTypeUnionConstraintInferences.ts",
    "lateBoundConstraintTypeChecksCorrectly.ts",
    "keyofDoesntContainSymbols.ts",
    "noStrictGenericChecks.ts",
    "noImplicitUseStrict_umd.ts",
    "noImplicitUseStrict_system.ts",
    "noImplicitUseStrict_es6.ts",
    "noImplicitUseStrict_commonjs.ts",
    "noImplicitAnyIndexingSuppressed.ts",
    "excessPropertyErrorsSuppressed.ts",
    "moduleNoneDynamicImport.ts",
    "moduleNoneErrors.ts",
    "noErrorUsingImportExportModuleAugmentationInDeclarationFile1.ts",
    "noErrorUsingImportExportModuleAugmentationInDeclarationFile2.ts",
    "noErrorUsingImportExportModuleAugmentationInDeclarationFile3.ts",
    "requireOfJsonFileWithModuleEmitNone.ts",
    "requireOfJsonFileWithModuleNodeResolutionEmitNone.ts",
];

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

pub fn testdata_path() -> PathBuf {
    match std::env::var_os("TSRS_TESTDATA") {
        Some(p) => PathBuf::from(p),
        None => repo_root().join("ts-ref/tsc/testdata"),
    }
}

pub fn reference_baseline_path(suite: &str, name: &str) -> PathBuf {
    testdata_path().join("baselines/reference").join(suite).join(format!("{name}.errors.txt"))
}

pub fn read_reference_baseline(suite: &str, name: &str) -> Option<String> {
    std::fs::read(reference_baseline_path(suite, name)).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
}

// The reference `.types` / `.symbols` baseline (`ext` = "types" | "symbols").
pub fn read_reference_extra_baseline(suite: &str, name: &str, ext: &str) -> Option<String> {
    let path = testdata_path().join("baselines/reference").join(suite).join(format!("{name}.{ext}"));
    let text = std::fs::read(path).ok().map(|b| String::from_utf8_lossy(&b).into_owned());
    #[cfg(feature = "checker")]
    if ext == "js" && crate::compile::dts_only_mode() {
        let block = extract_dts_block(text.as_deref().unwrap_or(""));
        return (block != crate::baseline::NO_CONTENT).then_some(block);
    }
    text
}

// The `.d.ts` outputs of a `.js` baseline (DoJSEmitBaseline writes them as the last run of `//// [name]` sections
// before `//// [DtsFileErrors]` / `!!!! File`), for the TSRS_TEST_DTS_ONLY metric.
pub fn extract_dts_block(text: &str) -> String {
    let mut text = text;
    for marker in ["\r\n\r\n//// [DtsFileErrors]", "\r\n\r\n!!!! File "] {
        if let Some(i) = text.find(marker) {
            text = &text[..i];
        }
    }
    let mut starts: Vec<usize> = Vec::new();
    let mut pos = 0;
    while let Some(i) = text[pos..].find("//// [") {
        let at = pos + i;
        if at == 0 || text.as_bytes()[at - 1] == b'\n' {
            starts.push(at);
        }
        pos = at + 6;
    }
    let is_dts = |at: usize| {
        let line_end = text[at..].find("\r\n").map_or(text.len(), |e| at + e);
        let name = &text[at + 6..line_end].trim_end_matches(']');
        tsrs_core::tspath::is_declaration_file_name(name)
    };
    let mut first = starts.len();
    while first > 0 && is_dts(starts[first - 1]) {
        first -= 1;
    }
    if first == starts.len() {
        return crate::baseline::NO_CONTENT.to_string();
    }
    // Declaration inputs can precede the outputs; the output block starts after Go's "\r\n\r\n" separator, and the
    // `.d.ts` outputs inside it follow each other directly.
    let begin = starts[first..].iter().rev().find(|&&at| text[..at].ends_with("\r\n\r\n")).copied().unwrap_or(starts[first]);
    text[begin..].to_string()
}

pub fn enumerate_test_files(suite: &str) -> Vec<String> {
    harnessutil::enumerate_files(&testdata_path().join("tests/cases").join(suite), &COMPILER_BASELINE_REGEX, true)
        .unwrap_or_else(|e| panic!("Could not read compiler test files: {e}"))
}

// vfs internal.decodeBytes: UTF-16 with BOM is decoded, a UTF-8 BOM is dropped.
pub fn decode_bytes(b: &[u8]) -> String {
    if b.len() >= 2 && (b[0..2] == [0xFF, 0xFE] || b[0..2] == [0xFE, 0xFF]) {
        let le = b[0] == 0xFF;
        let units: Vec<u16> =
            b[2..].chunks_exact(2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
        return String::from_utf16_lossy(&units);
    }
    let b = if b.len() >= 3 && b[0..3] == [0xEF, 0xBB, 0xBF] { &b[3..] } else { b };
    String::from_utf8_lossy(b).into_owned()
}

pub fn read_test_file(path: &str) -> String {
    match std::fs::read(path) {
        Ok(b) => decode_bytes(&b),
        Err(_) => panic!("Could not read test file: {path}"),
    }
}

// One runnable unit: a test file under one named configuration.
#[derive(Clone, Debug)]
pub struct TestItem {
    pub suite: String,
    pub path: String,
    // NamedTestConfiguration.Name ("" when the test has no varying options)
    pub config: String,
    // configuredName without the .ts/.tsx extension, e.g. `foo(target=es2015)`; also the baseline stem.
    pub name: String,
}

impl TestItem {
    pub fn id(&self) -> String {
        format!("{}/{}", self.suite, self.name)
    }
}

pub fn configured_name(basename: &str, config_name: &str) -> String {
    if !config_name.is_empty() {
        let extname = tspath::get_any_extension_from_path(basename, &[], false);
        let extensionless = &basename[..basename.len() - extname.len()];
        return format!("{extensionless}({config_name}){extname}");
    }
    basename.to_string()
}

pub fn baseline_stem(configured_name: &str) -> String {
    crate::tsbaseline::TS_EXTENSION.replace(configured_name, "").into_owned()
}

pub fn get_configurations(content: &str, table: &OptionTable) -> Result<Vec<NamedTestConfiguration>, String> {
    let settings = test_case_parser::extract_compiler_settings(content);
    harnessutil::get_file_based_test_configurations(&settings, table)
}

// Expands a test file into its variants (runTest). Skipped test files produce no items.
pub fn expand_test_file(suite: &str, path: &str, table: &OptionTable) -> Result<Vec<TestItem>, String> {
    let basename = tspath::get_base_file_name(path);
    if SKIPPED_TESTS.contains(&basename.as_str()) {
        return Ok(Vec::new());
    }
    let content = read_test_file(path);
    let configurations = get_configurations(&content, table)?;
    let item = |config: &str| TestItem {
        suite: suite.to_string(),
        path: path.to_string(),
        config: config.to_string(),
        name: baseline_stem(&configured_name(&basename, config)),
    };
    if configurations.is_empty() {
        return Ok(vec![item("")]);
    }
    Ok(configurations.iter().map(|c| item(&c.name)).collect())
}

pub fn find_configuration(content: &str, table: &OptionTable, config: &str) -> Result<Option<NamedTestConfiguration>, String> {
    let configurations = get_configurations(content, table)?;
    if config.is_empty() && configurations.len() <= 1 {
        return Ok(configurations.into_iter().next());
    }
    configurations.into_iter().find(|c| c.name == config).map(Some).ok_or_else(|| format!("configuration '{config}' not found"))
}

// What running one variant produced.
pub enum Outcome {
    // The generated error baseline text (baseline.NoContent when there were no diagnostics), and the
    // `.types`/`.symbols` baselines when they were requested and the test does not set @noTypesAndSymbols.
    // The third element is the `.js` baseline when `--baselines js` asked for it and the test emits (Go
    // verifyJavaScriptOutput: `hasNonDtsFiles` and not in `skippedEmitTests`); `Err` holds a panic message.
    Baseline(String, Option<TypesAndSymbols>, Option<Result<String, String>>),
    // SkipUnsupportedCompilerOptions
    Skip(String),
    // A harness-level failure that is not a panic (t.Fatalf in Go).
    Error(String),
}

// The generated `.types` and `.symbols` baselines; `Err` holds the panic message of a walk that panicked.
pub struct TypesAndSymbols {
    pub types: Result<String, String>,
    pub symbols: Result<String, String>,
}

fn create_harness_test_file(unit: &TestUnit, current_directory: &str) -> TestFile {
    TestFile { unit_name: tspath::get_normalized_absolute_path(&unit.name, current_directory), content: unit.content.clone() }
}

pub struct SplitUnits {
    pub current_directory: String,
    pub ts_config_files: Vec<TestFile>,
    // equivalent to the files that will be passed on the command line
    pub to_be_compiled: Vec<TestFile>,
    // equivalent to other files on the file system not directly passed to the compiler (ie things that are referenced by other files)
    pub other_files: Vec<TestFile>,
}

// The file-assembly part of newCompilerTest. `ts_config_file_names` is ParsedConfig.FileNames of the
// test's tsconfig.json, if it has one. May rewrite `baseurl` in the harness configuration.
pub fn split_units(payload: &TestCaseContent, harness_config: &mut TestConfiguration, ts_config_file_names: Option<&[String]>) -> SplitUnits {
    let current_directory = tspath::get_normalized_absolute_path(harness_config.get("currentdirectory").map_or("", String::as_str), SRC_FOLDER);
    let units = &payload.test_unit_data;
    let mut to_be_compiled = Vec::new();
    let mut other_files = Vec::new();
    let mut ts_config_files = Vec::new();
    if let (Some(file_names), Some(ts_config_unit)) = (ts_config_file_names, payload.ts_config_file_unit_data.as_ref()) {
        ts_config_files.push(create_harness_test_file(ts_config_unit, &current_directory));
        for unit in units {
            if file_names.contains(&tspath::get_normalized_absolute_path(&unit.name, &current_directory)) {
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
    SplitUnits { current_directory, ts_config_files, to_be_compiled, other_files }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Checks unit parsing and file assembly for every test variant against the input files the Go harness
    // produced (tools/oracle/testrunner `diags` output). Skipped when the oracle files are absent.
    #[test]
    fn units_match_go_harness() {
        let scratch = repo_root().join("target/scratch/testrunner");
        let (Ok(diags), Ok(table)) =
            (std::fs::read_to_string(scratch.join("diags.jsonl")), crate::oracle::load_option_table(&scratch.join("options.json").to_string_lossy()))
        else {
            eprintln!("oracle files missing; skipping");
            return;
        };
        let mut checked = 0;
        let mut mismatches = Vec::new();
        let mut paths: rustc_hash::FxHashMap<String, String> = Default::default();
        for suite in SUITES {
            for p in enumerate_test_files(suite) {
                paths.insert(format!("{suite}/{}", tspath::get_base_file_name(&p)), p);
            }
        }
        for line in diags.lines() {
            let v: serde_json::Value = serde_json::from_str(line).unwrap();
            if v.get("error").is_some() || v.get("files").is_none() {
                continue;
            }
            let suite = v["suite"].as_str().unwrap();
            let name = v["name"].as_str().unwrap();
            let file = regex::Regex::new(r"\(.*\)").unwrap().replace(name, "").into_owned();
            let path = &paths[&format!("{suite}/{file}")];
            let content = read_test_file(path);
            let stem = baseline_stem(name);
            let items = expand_test_file(suite, path, &table).unwrap();
            let item = items.iter().find(|i| i.name == stem).unwrap_or_else(|| panic!("no variant {stem}"));
            let mut config = find_configuration(&content, &table, &item.config).unwrap().map(|c| c.config).unwrap_or_default();
            let payload = test_case_parser::make_units_from_test(&content, path);
            if payload.global_options.get("runexternalcode").is_some_and(|v| v == "true") {
                // content-mapped files are baselined with their transformed text
                continue;
            }
            let all_names: Vec<String> =
                payload.test_unit_data.iter().map(|u| tspath::get_normalized_absolute_path(&u.name, &payload.current_directory)).collect();
            let has_ts_config = payload.ts_config_file_unit_data.is_some();
            let split = split_units(&payload, &mut config, if has_ts_config { Some(&all_names) } else { None });
            let mine: Vec<(String, String)> = split
                .ts_config_files
                .iter()
                .chain(&split.to_be_compiled)
                .chain(&split.other_files)
                .map(|f| (f.unit_name.clone(), f.content.clone()))
                .collect();
            let mut go: Vec<(String, String)> = v["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| (f["name"].as_str().unwrap().to_string(), f["content"].as_str().unwrap().to_string()))
                .collect();
            let mut mine = mine;
            if has_ts_config {
                mine[1..].sort();
                go[1..].sort();
            }
            checked += 1;
            if mine != go {
                mismatches.push(format!("{suite}/{name}"));
            }
        }
        assert!(mismatches.is_empty(), "{} of {checked} mismatched, e.g. {:?}", mismatches.len(), &mismatches[..mismatches.len().min(10)]);
        eprintln!("{checked} variants match");
    }
}
