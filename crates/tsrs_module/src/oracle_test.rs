// Differential test against the Go resolver (tools/oracle/module). Generate the inputs with
//   python3 tools/oracle/module/gen.py > target/scratch/module/scenarios.jsonl
//   bin/tsrs-oracle-module < target/scratch/module/scenarios.jsonl > target/scratch/module/expected.jsonl
// and run `cargo test -p tsrs_module oracle -- --ignored`. `testdata/oracle` holds the handcrafted subset
// (`grep handcrafted`) of both files.

use tsrs_core::collections::{new_ordered_map_with_size_hint, OrderedMapExt};
use tsrs_core::{CompilerOptions, JsxEmit, ModuleKind, ModuleResolutionKind, Tristate, P};
use tsrs_vfs::{vfstest, FS};

use crate::packagejson::json::{self, Json};
use crate::*;

struct Host {
    fs: Box<dyn FS>,
    cwd: String,
}

impl ResolutionHost for Host {
    fn fs(&self) -> &dyn FS {
        &*self.fs
    }
    fn get_current_directory(&self) -> &str {
        &self.cwd
    }
}

fn get<'a>(value: &'a Json, name: &str) -> Option<&'a Json> {
    let Json::Object(members) = value else { return None };
    members.iter().rev().find(|(n, _)| n == name).map(|(_, v)| v)
}

fn str_of(value: Option<&Json>) -> String {
    match value {
        Some(Json::String(s)) => s.clone(),
        _ => String::new(),
    }
}

fn num_of(value: Option<&Json>) -> i32 {
    match value {
        Some(Json::Number(n)) => *n as i32,
        _ => 0,
    }
}

fn bool_of(value: Option<&Json>) -> bool {
    matches!(value, Some(Json::Bool(true)))
}

fn tristate_of(value: Option<&Json>) -> Tristate {
    match value {
        Some(Json::Bool(true)) => Tristate::True,
        Some(Json::Bool(false)) => Tristate::False,
        _ => Tristate::Unknown,
    }
}

fn strings_of(value: Option<&Json>) -> Option<Vec<String>> {
    match value {
        Some(Json::Array(items)) => Some(items.iter().map(|i| str_of(Some(i))).collect()),
        _ => None,
    }
}

fn module_kind(n: i32) -> ModuleKind {
    match n {
        1 => ModuleKind::CommonJS,
        2 => ModuleKind::AMD,
        3 => ModuleKind::UMD,
        4 => ModuleKind::System,
        5 => ModuleKind::ES2015,
        6 => ModuleKind::ES2020,
        7 => ModuleKind::ES2022,
        99 => ModuleKind::ESNext,
        100 => ModuleKind::Node16,
        101 => ModuleKind::Node18,
        102 => ModuleKind::Node20,
        199 => ModuleKind::NodeNext,
        200 => ModuleKind::Preserve,
        _ => ModuleKind::None,
    }
}

fn module_resolution_kind(n: i32) -> ModuleResolutionKind {
    match n {
        1 => ModuleResolutionKind::Classic,
        2 => ModuleResolutionKind::Node10,
        3 => ModuleResolutionKind::Node16,
        99 => ModuleResolutionKind::NodeNext,
        100 => ModuleResolutionKind::Bundler,
        _ => ModuleResolutionKind::Unknown,
    }
}

fn jsx(n: i32) -> JsxEmit {
    match n {
        1 => JsxEmit::Preserve,
        2 => JsxEmit::React,
        3 => JsxEmit::ReactNative,
        4 => JsxEmit::ReactJSX,
        5 => JsxEmit::ReactJSXDev,
        _ => JsxEmit::None,
    }
}

fn format_trace(d: &DiagAndArgs) -> String {
    let mut parts = vec![d.message.key().to_string()];
    parts.extend(d.args.iter().cloned());
    parts.join("|")
}

fn run(scenario: &Json) -> Vec<(String, String, String)> {
    let mut files: Vec<(String, vfstest::MapFile)> = Vec::new();
    if let Some(Json::Object(members)) = get(scenario, "files") {
        for (path, value) in members {
            match value {
                Json::String(text) => files.push((path.clone(), text.into())),
                _ => files.push((path.clone(), vfstest::symlink(&str_of(get(value, "symlink"))))),
            }
        }
    }
    let host: &'static Host = Box::leak(Box::new(Host {
        fs: Box::new(vfstest::from_map(files, bool_of(get(scenario, "caseSensitive")))),
        cwd: str_of(get(scenario, "cwd")),
    }));
    let o = get(scenario, "options").unwrap();
    let mut options = CompilerOptions {
        module: module_kind(num_of(get(o, "module"))),
        module_resolution: module_resolution_kind(num_of(get(o, "moduleResolution"))),
        jsx: jsx(num_of(get(o, "jsx"))),
        resolve_json_module: tristate_of(get(o, "resolveJsonModule")),
        allow_js: tristate_of(get(o, "allowJs")),
        no_dts_resolution: tristate_of(get(o, "noDtsResolution")),
        preserve_symlinks: tristate_of(get(o, "preserveSymlinks")),
        resolve_package_json_exports: tristate_of(get(o, "resolvePackageJsonExports")),
        resolve_package_json_imports: tristate_of(get(o, "resolvePackageJsonImports")),
        allow_arbitrary_extensions: tristate_of(get(o, "allowArbitraryExtensions")),
        custom_conditions: strings_of(get(o, "customConditions")),
        module_suffixes: strings_of(get(o, "moduleSuffixes")),
        root_dirs: strings_of(get(o, "rootDirs")),
        type_roots: strings_of(get(o, "typeRoots")),
        types: strings_of(get(o, "types")),
        paths_base_path: str_of(get(o, "pathsBasePath")),
        out_dir: str_of(get(o, "outDir")),
        declaration_dir: str_of(get(o, "declarationDir")),
        root_dir: str_of(get(o, "rootDir")),
        config_file_path: str_of(get(o, "configFilePath")),
        trace_resolution: Tristate::True,
        ..Default::default()
    };
    if let Some(Json::Array(entries)) = get(o, "paths") {
        let mut paths = new_ordered_map_with_size_hint(entries.len());
        for entry in entries {
            let Json::Array(pair) = entry else { panic!("bad paths entry") };
            paths.set(str_of(Some(&pair[0])), strings_of(Some(&pair[1])).unwrap_or_default());
        }
        options.paths = Some(paths);
    }
    let options = P::new(options);
    let resolver = new_resolver(ResolverOptions::new(host, options));
    let expected_results = match get(scenario, "expected").and_then(|e| get(e, "results")) {
        Some(Json::Array(results)) => results.clone(),
        _ => Vec::new(),
    };
    let mut mismatches = Vec::new();
    let Some(Json::Array(requests)) = get(scenario, "requests") else { return mismatches };
    for (request, expected) in requests.iter().zip(expected_results.iter()) {
        let kind = str_of(get(request, "kind"));
        let name = str_of(get(request, "name"));
        let file = str_of(get(request, "file"));
        let mode = module_kind(num_of(get(request, "mode")));
        let label = format!("{kind} {name:?} from {file} mode={}", num_of(get(request, "mode")));
        let mut actual: Vec<(&str, String)> = Vec::new();
        let traces;
        if kind == "module" {
            let (m, t) = resolver.resolve_module_name(&name, &file, mode, None).unwrap();
            traces = t;
            actual.push(("resolvedFileName", m.resolved_file_name.to_string()));
            actual.push(("originalPath", m.original_path.to_string()));
            actual.push(("extension", m.extension.to_string()));
            actual.push(("packageId", if m.package_id.name.is_empty() { String::new() } else { m.package_id.to_string() }));
            actual.push(("isExternalLibraryImport", m.is_external_library_import.to_string()));
            actual.push(("resolvedUsingTsExtension", m.resolved_using_ts_extension.to_string()));
            actual.push(("resolvedUsingExtraExtensions", m.resolved_using_extra_extensions.to_string()));
            actual.push(("alternateResult", m.alternate_result.to_string()));
            actual.push(("diagnostics", m.resolution_diagnostics.iter().map(|d| d.code().to_string()).collect::<Vec<_>>().join(",")));
        } else {
            let (m, t) = resolver.resolve_type_reference_directive(&name, &file, mode, None);
            traces = t;
            actual.push(("resolvedFileName", m.resolved_file_name.to_string()));
            actual.push(("originalPath", m.original_path.to_string()));
            actual.push(("packageId", if m.package_id.name.is_empty() { String::new() } else { m.package_id.to_string() }));
            actual.push(("isExternalLibraryImport", m.is_external_library_import.to_string()));
            actual.push(("primary", m.primary.to_string()));
            actual.push(("diagnostics", m.resolution_diagnostics.iter().map(|d| d.code().to_string()).collect::<Vec<_>>().join(",")));
        }
        for (field, value) in &actual {
            let want = match get(expected, field) {
                Some(Json::String(s)) => s.clone(),
                Some(Json::Bool(b)) => b.to_string(),
                Some(Json::Array(items)) if *field == "diagnostics" => {
                    items.iter().map(|i| str_of(Some(i)).split(':').next().unwrap_or("").to_string()).collect::<Vec<_>>().join(",")
                }
                _ => String::new(),
            };
            if &want != value {
                mismatches.push((label.clone(), field.to_string(), format!("want {want:?}, got {value:?}")));
            }
        }
        let want_traces: Vec<String> = match get(expected, "traces") {
            Some(Json::Array(items)) => items.iter().map(|i| str_of(Some(i))).collect(),
            _ => Vec::new(),
        };
        let got_traces: Vec<String> = traces.iter().map(format_trace).collect();
        if want_traces != got_traces {
            let i = want_traces.iter().zip(got_traces.iter()).position(|(a, b)| a != b).unwrap_or(want_traces.len().min(got_traces.len()));
            mismatches.push((
                label.clone(),
                "traces".to_string(),
                format!("first difference at #{i}: want {:?}, got {:?}", want_traces.get(i), got_traces.get(i)),
            ));
        }
    }
    let want_types: Vec<String> = match get(scenario, "expected").and_then(|e| get(e, "automaticTypes")) {
        Some(Json::Array(items)) => items.iter().map(|i| str_of(Some(i))).collect(),
        _ => Vec::new(),
    };
    let mut got_types = get_automatic_type_directive_names(&options, host);
    got_types.sort();
    if want_types != got_types {
        mismatches.push(("automaticTypes".to_string(), "automaticTypes".to_string(), format!("want {want_types:?}, got {got_types:?}")));
    }
    mismatches
}

// The hand-written scenarios from gen.py, with the Go results checked in.
#[test]
fn oracle_handcrafted() {
    run_oracle_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/oracle"));
}

// The full corpus-derived set, generated locally (see the top of this file).
#[test]
#[ignore]
fn oracle_scenarios() {
    let dir = std::env::var("TSRS_MODULE_ORACLE_DIR").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/scratch/module").to_string());
    run_oracle_dir(&dir);
}

fn run_oracle_dir(dir: &str) {
    let scenarios = std::fs::read_to_string(format!("{dir}/scenarios.jsonl")).expect("scenarios.jsonl");
    let expected = std::fs::read_to_string(format!("{dir}/expected.jsonl")).expect("expected.jsonl");
    let mut failed = 0;
    let mut total = 0;
    for (scenario_line, expected_line) in scenarios.lines().zip(expected.lines()) {
        let expected = json::parse(expected_line).unwrap();
        if get(&expected, "error").is_some() {
            continue;
        }
        let Json::Object(mut members) = json::parse(scenario_line).unwrap() else { panic!() };
        members.push(("expected".to_string(), expected));
        let scenario = Json::Object(members);
        total += 1;
        let name = str_of(get(&scenario, "name"));
        let result = std::panic::catch_unwind(|| run(&scenario));
        match result {
            Ok(mismatches) if mismatches.is_empty() => {}
            Ok(mismatches) => {
                failed += 1;
                if failed <= 40 {
                    eprintln!("MISMATCH {name}");
                    for (label, field, detail) in mismatches.iter().take(3) {
                        eprintln!("    {label}: {field}: {detail}");
                    }
                }
            }
            Err(_) => {
                failed += 1;
                eprintln!("PANIC {name}");
            }
        }
    }
    eprintln!("{} / {} scenarios match", total - failed, total);
    assert_eq!(failed, 0);
}
