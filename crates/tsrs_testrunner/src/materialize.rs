// `tsrs-test materialize`: writes conformance cases to disk as tsc command lines, the input of the WebAssembly
// differential (tools/wasm/diff.mjs runs native tsrs and the module on the same directory and compares their output).
// It reuses the runner's parse, configuration and split path, so a case holds the same files as in the runner. It is
// a differential input, not a baseline match: options the command line cannot carry (harness-only options,
// tsconfig-only options) are dropped and counted, which costs coverage, not correctness.
//
// Per item, `<out>/<NNNNN>_<id>/`:
//   root/<path>     every unit at its virtual path (`/.src/a.ts` -> `root/.src/a.ts`); `@link` / `@symlink` as
//                   relative symlinks; the test libraries in `root/.lib` when the case uses them
//   case.json       { "id", "cwd" (relative to root), "args", "dropped", "skipped"? }; `${ROOT}` in an argument
//                   stands for the absolute path of the directory that holds `root/`'s copy at run time

use std::collections::BTreeMap;
use std::path::Path;

use tsrs_core::json::Value;
use tsrs_core::tspath;

use crate::compiler_runner::{self, TestItem};
use crate::harnessutil::{OptionTable, TestConfiguration, TEST_LIB_FOLDER};
use crate::test_case_parser;

pub struct Materialized {
    pub skipped: bool,
    pub dropped: Vec<String>,
}

// Harness settings the layout itself carries (units, cwd, the lib copy, links) rather than dropped.
const HANDLED: [&str; 7] = ["currentdirectory", "filename", "libfiles", "link", "noimplicitreferences", "symlink", "typescriptversion"];

fn rooted(path: &str) -> String {
    format!("${{ROOT}}{path}")
}

fn write_file(root: &Path, virtual_path: &str, content: &str) -> std::io::Result<()> {
    let dest = root.join(virtual_path.trim_start_matches('/'));
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(dest, content)
}

fn relative(from_dir: &str, to: &str) -> String {
    let from: Vec<&str> = from_dir.split('/').filter(|s| !s.is_empty()).collect();
    let to_parts: Vec<&str> = to.split('/').filter(|s| !s.is_empty()).collect();
    let common = from.iter().zip(&to_parts).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<&str> = vec![".."; from.len() - common];
    parts.extend(&to_parts[common..]);
    if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    }
}

fn copy_test_libs(root: &Path) -> std::io::Result<()> {
    let src = compiler_runner::testdata_path().join("tests/lib");
    fn walk(dir: &Path, base: &Path, dest: &Path) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            let target = dest.join(path.strip_prefix(base).unwrap());
            if path.is_dir() {
                walk(&path, base, dest)?;
            } else {
                std::fs::create_dir_all(target.parent().unwrap())?;
                std::fs::copy(&path, &target)?;
            }
        }
        Ok(())
    }
    walk(&src, &src, &root.join(TEST_LIB_FOLDER.trim_start_matches('/')))
}

/// Writes one item under `dir`.
pub fn materialize(item: &TestItem, table: &OptionTable, dir: &Path) -> Result<Materialized, String> {
    let root = dir.join("root");
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let content = compiler_runner::read_test_file(&item.path);
    let named = compiler_runner::find_configuration(&content, table, &item.config)?;
    let payload = test_case_parser::make_units_from_test(&content, &item.path);
    let mut case = BTreeMap::new();
    case.insert("id".to_string(), Value::String(item.id()));
    if payload.global_options.get("runexternalcode").is_some_and(|v| v == "true") {
        case.insert("skipped".to_string(), Value::String("runExternalCode".to_string()));
        std::fs::write(dir.join("case.json"), tsrs_core::json::marshal(&Value::Object(case.into_iter().collect())).unwrap()).map_err(|e| e.to_string())?;
        return Ok(Materialized { skipped: true, dropped: Vec::new() });
    }
    let ts_config = crate::options::parse_test_ts_config(&payload);
    let mut harness_config: TestConfiguration = named.map(|c| c.config).unwrap_or_default();
    let split = compiler_runner::split_units(&payload, &mut harness_config, ts_config.map(|t| t.get().parsed_config.file_names.as_slice()));
    let cwd = split.current_directory.clone();

    for file in split.ts_config_files.iter().chain(&split.to_be_compiled).chain(&split.other_files) {
        write_file(&root, &tspath::get_normalized_absolute_path(&file.unit_name, &cwd), &file.content).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(root.join(cwd.trim_start_matches('/'))).map_err(|e| e.to_string())?;
    for (link, target) in &payload.symlinks {
        let link = tspath::get_normalized_absolute_path(link, &cwd);
        let target = tspath::get_normalized_absolute_path(target, &cwd);
        let dest = root.join(link.trim_start_matches('/'));
        std::fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(relative(&tspath::get_directory_path(&link), &target), &dest).map_err(|e| e.to_string())?;
    }

    let lib_files: Vec<String> = harness_config.get("libfiles").map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default();
    let no_lib = harness_config.get("nolib").is_some_and(|v| v == "true");
    let uses_libs = !lib_files.is_empty() || split.to_be_compiled.iter().any(|f| f.content.contains(&format!("{TEST_LIB_FOLDER}/")));
    if uses_libs {
        copy_test_libs(&root).map_err(|e| e.to_string())?;
    }

    let mut args = Vec::new();
    let mut dropped = Vec::new();
    if let Some(tsconfig) = &split.ts_config_files.first() {
        args.push("-p".to_string());
        args.push(rooted(&tspath::get_normalized_absolute_path(&tsconfig.unit_name, &cwd)));
    }
    for (name, value) in &harness_config {
        if HANDLED.contains(&name.as_str()) {
            continue;
        }
        match crate::options::get_command_line_option(name) {
            Some(option) if !option.is_tsconfig_only => {
                args.push(format!("--{}", option.name));
                let value = value.split(',').map(|v| if v.trim().starts_with('/') { rooted(v.trim()) } else { v.to_string() }).collect::<Vec<_>>().join(",");
                args.push(value);
            }
            _ => dropped.push(name.clone()),
        }
    }
    if split.ts_config_files.is_empty() {
        for file in &split.to_be_compiled {
            let path = tspath::get_normalized_absolute_path(&file.unit_name, &cwd);
            if !tspath::file_extension_is(&path, tspath::EXTENSION_JSON) && !tspath::file_extension_is(&path, tspath::EXTENSION_TS_BUILD_INFO) {
                args.push(rooted(&path));
            }
        }
        for lib in &lib_files {
            if lib == "lib.d.ts" && !no_lib {
                continue;
            }
            args.push(rooted(&tspath::combine_paths(TEST_LIB_FOLDER, &[lib])));
        }
    }
    args.push("--pretty".to_string());
    args.push("false".to_string());

    case.insert("cwd".to_string(), Value::String(cwd.trim_start_matches('/').to_string()));
    case.insert("args".to_string(), Value::Array(args.into_iter().map(Value::String).collect()));
    case.insert("dropped".to_string(), Value::Array(dropped.iter().cloned().map(Value::String).collect()));
    std::fs::write(dir.join("case.json"), tsrs_core::json::marshal(&Value::Object(case.into_iter().collect())).unwrap()).map_err(|e| e.to_string())?;
    Ok(Materialized { skipped: false, dropped })
}

/// A directory name for an item: its index (unique) plus a readable form of its id.
pub fn dir_name(index: usize, item: &TestItem) -> String {
    let id: String = item.id().chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' { c } else { '_' }).collect();
    format!("{index:05}_{}", &id[..id.len().min(120)])
}
