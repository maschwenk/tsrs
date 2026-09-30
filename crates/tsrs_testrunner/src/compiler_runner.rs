// internal/testrunner/compiler_runner.go: test enumeration, configurations and per-variant execution.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use tsrs_core::tspath;

use crate::harnessutil::{self, NamedTestConfiguration, OptionTable};
use crate::test_case_parser;

// Posix-style path to sources under test
pub const SRC_FOLDER: &str = "/.src";

pub const SUITES: [&str; 2] = ["compiler", "conformance"];

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
    // The generated error baseline text (baseline.NoContent when there were no diagnostics).
    Baseline(String),
    // SkipUnsupportedCompilerOptions
    Skip(String),
    // A harness-level failure that is not a panic (t.Fatalf in Go).
    Error(String),
}
