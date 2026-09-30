use std::path::PathBuf;

use rustc_hash::FxHashSet;
use tsrs_ast::Diagnostic;
use tsrs_core::collections::OrderedMap;
use tsrs_core::json;
use tsrs_core::tspath;
use tsrs_core::P;
use tsrs_diagnostics as diagnostics;
use tsrs_vfs::FS;

use crate::commandlineoption::{CommandLineOption, CommandLineOptionKind, CompilerOptionsValue, DefaultValueDescription};
use crate::commandlineparser::{parse_build_command_line, parse_command_line, CommandLineParser};
use crate::declscompiler::OPTIONS_DECLARATIONS;
use crate::diagnostics::{get_parse_command_line_worker_diagnostics, ParseCommandLineWorkerDiagnostics, COMPILER_OPTIONS_DID_YOU_MEAN_DIAGNOSTICS};
use crate::namemap::get_name_map_from_list;
use crate::testutil::{build_options_to_json, compiler_options_to_json, write_format_diagnostics};
use crate::tsconfigparsing::stringify_json;
use crate::tsoptionstest::new_vfs_parse_config_host;

pub(crate) fn repo_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    while !dir.join("ts-ref").exists() {
        dir = dir.parent().expect("repository root with ts-ref").to_path_buf();
    }
    dir
}

pub(crate) fn reference_baseline(subfolder: &str, name: &str) -> String {
    let path = repo_root().join("ts-ref/tsc/testdata/baselines/reference").join(subfolder).join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|_| "<no content>".to_string())
}

// baseline.Run: compares against the reference baseline, collecting mismatches instead of failing fast.
pub(crate) fn check_baseline(failures: &mut Vec<String>, subfolder: &str, name: &str, actual: &str) {
    let expected = reference_baseline(subfolder, name);
    if expected != actual {
        let local = repo_root().join("target/scratch/tsoptions/baselines/local").join(subfolder).join(name);
        std::fs::create_dir_all(local.parent().unwrap()).unwrap();
        std::fs::write(&local, actual).unwrap();
        failures.push(name.to_string());
    }
}

pub(crate) struct TestCommandLineParser {
    pub(crate) file_names: Vec<String>,
    pub(crate) options: OrderedMap<String, CompilerOptionsValue>,
    pub(crate) errors: Vec<P<Diagnostic>>,
}

fn get_test_parse_command_line_worker_diagnostics(decls: &[&'static CommandLineOption]) -> &'static ParseCommandLineWorkerDiagnostics {
    if decls.is_empty() {
        return &COMPILER_OPTIONS_DID_YOU_MEAN_DIAGNOSTICS;
    }
    P::new(get_parse_command_line_worker_diagnostics(decls)).get()
}

pub(crate) fn parse_command_line_test_worker(
    decls: &[&'static CommandLineOption],
    command_line: &[String],
    fs: Option<&dyn FS>,
    current_directory: &str,
) -> TestCommandLineParser {
    let mut parser = CommandLineParser {
        current_directory: current_directory.to_string(),
        worker_diagnostics: &COMPILER_OPTIONS_DID_YOU_MEAN_DIAGNOSTICS,
        options_map: get_name_map_from_list(&[]),
        file_names: Vec::new(),
        options: OrderedMap::default(),
        errors: Vec::new(),
        response_file_stack: FxHashSet::default(),
    };
    if !decls.is_empty() {
        parser.worker_diagnostics = get_test_parse_command_line_worker_diagnostics(decls);
    }

    parser.options_map = get_name_map_from_list(parser.options_declarations());
    parser.parse_strings(command_line, fs);
    TestCommandLineParser { file_names: parser.file_names, options: parser.options, errors: parser.errors }
}

fn args(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

fn format_new_baseline(command_line: &[String], opts: &str, file_names: &str, errors: &str) -> String {
    let mut formatted = String::new();
    formatted.push_str("Args::\n");
    formatted.push('[');
    for (i, arg) in command_line.iter().enumerate() {
        if i > 0 {
            formatted.push_str(", ");
        }
        formatted.push('"');
        formatted.push_str(arg);
        formatted.push('"');
    }
    formatted.push(']');
    formatted.push_str("\n\nCompilerOptions::\n");
    formatted.push_str(opts);
    // todo: watch options not implemented
    // formatted.WriteString("WatchOptions::\n")
    formatted.push_str("\n\nFileNames::\n");
    formatted.push_str(file_names);
    formatted.push_str("\n\nErrors::\n");
    formatted.push_str(errors);
    formatted
}

fn format_new_baseline_build(command_line: &[String], opts: &str, compiler_opts: &str, projects: &str, errors: &str) -> String {
    let mut formatted = String::new();
    formatted.push_str("Args::\n");
    formatted.push('[');
    for (i, arg) in command_line.iter().enumerate() {
        if i > 0 {
            formatted.push_str(", ");
        }
        formatted.push('"');
        formatted.push_str(arg);
        formatted.push('"');
    }
    formatted.push(']');
    formatted.push_str("\n\nbuildOptions::\n");
    formatted.push_str(opts);
    formatted.push_str("\n\ncompilerOptions::\n");
    formatted.push_str(compiler_opts);
    // todo: watch options not implemented
    // formatted.WriteString("WatchOptions::\n")
    formatted.push_str("\n\nProjects::\n");
    formatted.push_str(projects);
    formatted.push_str("\n\nErrors::\n");
    formatted.push_str(errors);
    formatted
}

fn assert_parse_result(failures: &mut Vec<String>, scenario_kind: &str, name: &str, command_line: &[String], opt_decls: &[&'static CommandLineOption]) {
    let test_name = format!("{scenario_kind}/{name}");
    let cwd = std::env::temp_dir();
    let parsed = parse_command_line_test_worker(opt_decls, command_line, Some(tsrs_vfs::osvfs::fs()), &tspath::normalize_slashes(&cwd.to_string_lossy()));

    let new_baseline_file_names = parsed.file_names.join(",");
    let o = stringify_json(&CompilerOptionsValue::Object(parsed.options));
    let new_baseline_errors = write_format_diagnostics(&parsed.errors, "\n");
    check_baseline(
        failures,
        "tsoptions/commandLineParsing",
        &format!("{test_name}.js"),
        &format_new_baseline(command_line, &o, &new_baseline_file_names, &new_baseline_errors),
    );
}

fn assert_build_parse_result(failures: &mut Vec<String>, name: &str, command_line: &[String]) {
    let test_name = format!("parseBuildOptions/{name}");
    let testdata = repo_root().join("ts-ref/tsc/testdata");
    let host = new_vfs_parse_config_host(&[], &tspath::normalize_slashes(&testdata.to_string_lossy()), true);
    let parsed = parse_build_command_line(command_line, host);

    let new_baseline_projects = parsed.projects.join(",");
    let o = json::marshal(&build_options_to_json(&parsed.build_options)).unwrap();
    let compiler_opts = json::marshal(&compiler_options_to_json(&parsed.compiler_options)).unwrap();
    let new_baseline_errors = write_format_diagnostics(&parsed.errors, "\n");
    check_baseline(
        failures,
        "tsoptions/commandLineParsing",
        &format!("{test_name}.js"),
        &format_new_baseline_build(command_line, &o, &compiler_opts, &new_baseline_projects, &new_baseline_errors),
    );
}

#[test]
fn test_command_line_parse_result() {
    let parse_command_line_sub_scenarios: Vec<(&str, Vec<String>)> = vec![
        // --lib es6 0.ts
        ("Parse single option of library flag", args(&["--lib", "es6", "0.ts"])),
        ("Handles may only be used with --build flags", args(&["--build", "--clean", "--dry", "--force", "--verbose"])),
        // --declarations --allowTS
        ("Handles did you mean for misspelt flags", args(&["--declarations", "--allowTS"])),
        // --lib es5,es2015.symbol.wellknown 0.ts
        ("Parse multiple options of library flags", args(&["--lib", "es5,es2015.symbol.wellknown", "0.ts"])),
        // --lib es5,invalidOption 0.ts
        ("Parse invalid option of library flags", args(&["--lib", "es5,invalidOption", "0.ts"])),
        // 0.ts --jsx
        ("Parse empty options of --jsx", args(&["0.ts", "--jsx"])),
        // 0.ts --
        ("Parse empty options of --module", args(&["0.ts", "--module"])),
        // 0.ts --newLine
        ("Parse empty options of --newLine", args(&["0.ts", "--newLine"])),
        // 0.ts --target
        ("Parse empty options of --target", args(&["0.ts", "--target"])),
        // 0.ts --moduleResolution
        ("Parse empty options of --moduleResolution", args(&["0.ts", "--moduleResolution"])),
        // 0.ts --lib
        ("Parse empty options of --lib", args(&["0.ts", "--lib"])),
        // 0.ts --lib
        // This test is an error because the empty string is falsey
        ("Parse empty string of --lib", args(&["0.ts", "--lib", ""])),
        // 0.ts --lib
        ("Parse immediately following command line argument of --lib", args(&["0.ts", "--lib", "--sourcemap"])),
        // --lib es5, es7 0.ts
        ("Parse --lib option with extra comma", args(&["--lib", "es5,", "es7", "0.ts"])),
        // --lib es5, es7 0.ts
        ("Parse --lib option with trailing white-space", args(&["--lib", "es5, ", "es7", "0.ts"])),
        // --lib es5,es2015.symbol.wellknown --target es5 0.ts
        (
            "Parse multiple compiler flags with input files at the end",
            args(&["--lib", "es5,es2015.symbol.wellknown", "--target", "es5", "0.ts"]),
        ),
        // --module commonjs --target es5 0.ts --lib es5,es2015.symbol.wellknown
        (
            "Parse multiple compiler flags with input files in the middle",
            args(&["--module", "commonjs", "--target", "es5", "0.ts", "--lib", "es5,es2015.symbol.wellknown"]),
        ),
        // --module commonjs --target es5 --lib es5 0.ts --library es2015.array,es2015.symbol.wellknown
        (
            "Parse multiple library compiler flags ",
            args(&["--module", "commonjs", "--target", "es5", "--lib", "es5", "0.ts", "--lib", "es2015.core, es2015.symbol.wellknown "]),
        ),
        ("Parse explicit boolean flag value", args(&["--strictNullChecks", "false", "0.ts"])),
        ("Parse non boolean argument after boolean flag", args(&["--noImplicitAny", "t", "0.ts"])),
        ("Parse implicit boolean flag value", args(&["--strictNullChecks"])),
        ("parse --incremental", args(&["--incremental", "0.ts"])),
        ("parse --tsBuildInfoFile", args(&["--tsBuildInfoFile", "build.tsbuildinfo", "0.ts"])),
        ("allows tsconfig only option to be set to null", args(&["--composite", "null", "-tsBuildInfoFile", "null", "0.ts"])),
        // ****** Watch Options ******
        ("parse --watchFile", args(&["--watchFile", "UseFsEvents", "0.ts"])),
        ("parse --watchDirectory", args(&["--watchDirectory", "FixedPollingInterval", "0.ts"])),
        ("parse --fallbackPolling", args(&["--fallbackPolling", "PriorityInterval", "0.ts"])),
        ("parse --synchronousWatchDirectory", args(&["--synchronousWatchDirectory", "0.ts"])),
        ("errors on missing argument to --fallbackPolling", args(&["0.ts", "--fallbackPolling"])),
        ("parse --excludeDirectories", args(&["--excludeDirectories", "**/temp", "0.ts"])),
        ("errors on invalid excludeDirectories", args(&["--excludeDirectories", "**/../*", "0.ts"])),
        ("parse --excludeFiles", args(&["--excludeFiles", "**/temp/*.ts", "0.ts"])),
        ("errors on invalid excludeFiles", args(&["--excludeFiles", "**/../*", "0.ts"])),
    ];

    let mut failures = Vec::new();
    for (name, command_line) in &parse_command_line_sub_scenarios {
        assert_parse_result(&mut failures, "parseCommandLine", name, command_line, &[]);
    }
    assert!(failures.is_empty(), "baseline mismatches: {failures:#?}");
}

#[test]
fn test_response_file_does_not_panic() {
    // Passing `@` with an empty or relative filename should not panic.
    // It should produce a diagnostic error instead.
    let cwd = tspath::normalize_slashes(&std::env::temp_dir().to_string_lossy());
    let parsed = parse_command_line_test_worker(&[], &args(&["@"]), Some(tsrs_vfs::osvfs::fs()), &cwd);
    assert!(!parsed.errors.is_empty(), "expected an error for empty response file name");
    let parsed = parse_command_line_test_worker(&[], &args(&["@blah"]), Some(tsrs_vfs::osvfs::fs()), &cwd);
    assert!(!parsed.errors.is_empty(), "expected an error for non-existent response file");
}

#[test]
fn test_response_file_parsing() {
    // final token without trailing whitespace
    let host = new_vfs_parse_config_host(&[("/project/args.txt", "--strict --outDir dist")], "/project", true);
    let parsed = parse_command_line(&args(&["@args.txt"]), host);
    assert_eq!(parsed.errors.len(), 0);
    assert!(parsed.compiler_options().unwrap().strict.is_true());
    assert_eq!(parsed.compiler_options().unwrap().out_dir, "/project/dist");

    // cyclic response files
    let host = new_vfs_parse_config_host(
        &[("/project/a.txt", "@/project/b.txt --strict"), ("/project/b.txt", "@/project/a.txt --outDir dist")],
        "/project",
        true,
    );
    let parsed = parse_command_line(&args(&["@a.txt"]), host);
    assert_eq!(parsed.errors.len(), 0);
    assert!(parsed.compiler_options().unwrap().strict.is_true());
    assert_eq!(parsed.compiler_options().unwrap().out_dir, "/project/dist");
}

#[test]
fn test_parse_command_line_type_roots_relative_path() {
    let host = new_vfs_parse_config_host(&[("/home/project/bug.ts", "let x = 1;")], "/home/project", true);
    let cmd_line = parse_command_line(&args(&["--typeRoots", "t", "bug.ts"]), host);
    let options = cmd_line.compiler_options().unwrap();
    let type_roots = options.type_roots.as_ref().expect("typeRoots should not be nil");
    assert_eq!(type_roots.len(), 1);
    assert!(tspath::is_rooted_disk_path(&type_roots[0]), "typeRoots entry should be an absolute path, got: {}", type_roots[0]);
    assert!(type_roots[0].ends_with("/t"), "typeRoots entry should end with '/t', got: {}", type_roots[0]);
}

struct VerifyNull {
    sub_scenario: &'static str,
    option_name: &'static str,
    non_null_value: &'static str,
    opt_decls: Vec<&'static CommandLineOption>,
}

fn create_verify_null_for_non_null_included(sub_scenario: &'static str, kind: CommandLineOptionKind, non_null_value: &'static str) -> VerifyNull {
    let extra: &'static CommandLineOption = P::new(CommandLineOption {
        name: "optionName",
        kind,
        is_tsconfig_only: true,
        category: Some(&diagnostics::Backwards_Compatibility),
        description: Some(&diagnostics::Enable_project_compilation),
        default_value_description: DefaultValueDescription::None,
        ..CommandLineOption::DEFAULT
    })
    .get();
    let mut opt_decls = OPTIONS_DECLARATIONS.clone();
    opt_decls.push(extra);
    VerifyNull { sub_scenario, option_name: "optionName", non_null_value, opt_decls }
}

#[test]
fn test_parse_command_line_verify_null() {
    let mut failures = Vec::new();
    // run test for boolean
    assert_parse_result(
        &mut failures,
        "parseCommandLine",
        "allows setting option type boolean to false",
        &args(&["--composite", "false", "0.ts"]),
        &[],
    );

    let verify_null_sub_scenarios = vec![
        VerifyNull { sub_scenario: "option of type boolean", option_name: "composite", non_null_value: "true", opt_decls: Vec::new() },
        VerifyNull { sub_scenario: "option of type object", option_name: "paths", non_null_value: "", opt_decls: Vec::new() },
        VerifyNull { sub_scenario: "option of type list", option_name: "rootDirs", non_null_value: "abc,xyz", opt_decls: Vec::new() },
        create_verify_null_for_non_null_included("option of type string", CommandLineOptionKind::String, "hello"),
        create_verify_null_for_non_null_included("option of type number", CommandLineOptionKind::Number, "10"),
        // todo: make the following work for tests -- currently it is difficult to do extra options of enum type
        // createVerifyNullForNonNullIncluded("option of type custom map", CommandLineOptionTypeEnum, "node"),
    ];

    for verify_null_case in &verify_null_sub_scenarios {
        let option = format!("--{}", verify_null_case.option_name);
        assert_parse_result(
            &mut failures,
            "parseCommandLine",
            &format!("{} allows setting it to null", verify_null_case.sub_scenario),
            &args(&[&option, "null", "0.ts"]),
            &verify_null_case.opt_decls,
        );

        if !verify_null_case.non_null_value.is_empty() {
            assert_parse_result(
                &mut failures,
                "parseCommandLine",
                &format!("{} errors if non null value is passed", verify_null_case.sub_scenario),
                &args(&[&option, verify_null_case.non_null_value, "0.ts"]),
                &verify_null_case.opt_decls,
            );
        }

        assert_parse_result(
            &mut failures,
            "parseCommandLine",
            &format!("{} errors if its followed by another option", verify_null_case.sub_scenario),
            &args(&["0.ts", "--strictNullChecks", &option]),
            &verify_null_case.opt_decls,
        );

        assert_parse_result(
            &mut failures,
            "parseCommandLine",
            &format!("{} errors if its last option", verify_null_case.sub_scenario),
            &args(&["0.ts", &option]),
            &verify_null_case.opt_decls,
        );
    }
    assert!(failures.is_empty(), "baseline mismatches: {failures:#?}");
}

#[test]
fn test_parse_build_command_line() {
    let parse_command_line_sub_scenarios: Vec<(&str, Vec<String>)> = vec![
        ("parse build without any options ", args(&[])),
        ("Parse multiple options", args(&["--verbose", "--force", "tests"])),
        ("Parse option with invalid option", args(&["--verbose", "--invalidOption"])),
        ("Parse multiple flags with input projects at the end", args(&["--force", "--verbose", "src", "tests"])),
        ("Parse multiple flags with input projects in the middle", args(&["--force", "src", "tests", "--verbose"])),
        ("Parse multiple flags with input projects in the beginning", args(&["src", "tests", "--force", "--verbose"])),
        ("parse build with --incremental", args(&["--incremental", "tests"])),
        ("parse build with --locale en-us", args(&["--locale", "en-us", "src"])),
        ("parse build with --tsBuildInfoFile", args(&["--tsBuildInfoFile", "build.tsbuildinfo", "tests"])),
        ("reports other common may not be used with --build flags", args(&["--strict"])),
        ("--clean and --force together is invalid", args(&["--clean", "--force"])),
        ("--clean and --verbose together is invalid", args(&["--clean", "--verbose"])),
        ("--clean and --watch together is invalid", args(&["--clean", "--watch"])),
        ("--watch and --dry together is invalid", args(&["--watch", "--dry"])),
        ("parse --watchFile", args(&["--watchFile", "UseFsEvents", "--verbose"])),
        ("parse --watchDirectory", args(&["--watchDirectory", "FixedPollingInterval", "--verbose"])),
        ("parse --fallbackPolling", args(&["--fallbackPolling", "PriorityInterval", "--verbose"])),
        ("parse --synchronousWatchDirectory", args(&["--synchronousWatchDirectory", "--verbose"])),
        ("errors on missing argument", args(&["--verbose", "--fallbackPolling"])),
        ("errors on invalid excludeDirectories", args(&["--excludeDirectories", "**/../*"])),
        ("parse --excludeFiles", args(&["--excludeFiles", "**/temp/*.ts"])),
        ("errors on invalid excludeFiles", args(&["--excludeFiles", "**/../*"])),
        ("parse --builders", args(&["--builders", "2"])),
        ("--singleThreaded and --builders together", args(&["--singleThreaded", "--builders", "2"])),
        ("reports error when --builders is 0", args(&["--builders", "0"])),
        ("reports error when --builders is negative", args(&["--builders", "-1"])),
        ("reports error when --builders is invalid type", args(&["--builders", "invalid"])),
    ];

    let mut failures = Vec::new();
    for (name, command_line) in &parse_command_line_sub_scenarios {
        assert_build_parse_result(&mut failures, name, command_line);
    }
    assert!(failures.is_empty(), "baseline mismatches: {failures:#?}");
}

#[test]
fn test_affects_build_info() {
    // should have affectsBuildInfo true for every option with affectsSemanticDiagnostics
    for option in OPTIONS_DECLARATIONS.iter() {
        if option.affects_semantic_diagnostics {
            // semantic diagnostics affect the build info, so ensure they're included
            assert!(option.affects_build_info);
        }
    }
}
