// Differential test of tsrs_modulespecifiers + `Program`'s ModuleSpecifierGenerationHost (incl. GetSymlinkCache)
// against the Go implementation (tools/oracle/modulespecifiers). Regenerate the data with
//   python3 tools/oracle/modulespecifiers/gen.py > crates/tsrs_compiler/testdata/modulespecifiers_oracle/scenarios.jsonl
//   bin/tsrs-oracle-modulespecifiers < .../scenarios.jsonl > .../expected.jsonl

use std::sync::Arc;

use tsrs_core::collections::OrderedMap;
use tsrs_core::json::{self, Value};
use tsrs_core::{CompilerOptions, ModuleKind, Tristate, P};
use tsrs_modulespecifiers as modulespecifiers;
use tsrs_tsoptions::{self as tsoptions, ParseConfigHost};
use tsrs_vfs::{bundled, vfstest, FS};

use crate::{new_compiler_host, new_program, ProgramOptions};

struct parseConfigHost {
    fs: Arc<dyn FS>,
    cwd: String,
}

impl ParseConfigHost for parseConfigHost {
    fn fs(&self) -> &dyn FS {
        &*self.fs
    }
    fn get_current_directory(&self) -> &str {
        &self.cwd
    }
}

fn field<'a>(v: &'a Value, name: &str) -> &'a Value {
    match v {
        Value::Object(m) => m.get(name).unwrap_or(&Value::Null),
        _ => &Value::Null,
    }
}

fn str_field(v: &Value, name: &str) -> String {
    match field(v, name) {
        Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

fn object(entries: Vec<(&str, Value)>) -> Value {
    let mut m = OrderedMap::default();
    for (k, v) in entries {
        m.insert(k.to_string(), v);
    }
    Value::Object(m)
}

fn strings(v: impl IntoIterator<Item = String>) -> Value {
    Value::Array(v.into_iter().map(Value::String).collect())
}

fn run(s: &Value) -> Value {
    let cwd = str_field(s, "cwd");
    let case_sensitive = matches!(field(s, "caseSensitive"), Value::Bool(true));
    let mut files: Vec<(String, vfstest::MapFile)> = Vec::new();
    if let Value::Object(m) = field(s, "files") {
        for (k, v) in m {
            let Value::String(text) = v else { unreachable!() };
            files.push((k.clone(), vfstest::MapFile::from(text.as_str())));
        }
    }
    if let Value::Object(m) = field(s, "symlinks") {
        for (k, v) in m {
            let Value::String(target) = v else { unreachable!() };
            files.push((k.clone(), vfstest::symlink(target)));
        }
    }
    let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(vfstest::from_map(files, case_sensitive)));
    let host = new_compiler_host(&cwd, fs.clone(), &bundled::lib_path(), None, None);
    let parse_host: &'static parseConfigHost = Box::leak(Box::new(parseConfigHost { fs, cwd: cwd.clone() }));
    let (config, errors) =
        tsoptions::get_parsed_command_line_of_config_file(&str_field(s, "config"), Some(&CompilerOptions::default()), None, parse_host, None);
    let name = Value::String(str_field(s, "name"));
    let Some(config) = config.filter(|_| errors.is_empty()) else {
        return object(vec![("name", name), ("symlinks", strings(["config error".to_string()])), ("results", Value::Array(Vec::new()))]);
    };
    let mut opts = ProgramOptions::new(P::new(config), host);
    opts.single_threaded = Tristate::True;
    let program = new_program(opts);

    let mut symlinks = Vec::new();
    program.get_symlink_cache().directories_by_realpath().range(|realpath, set| {
        for k in set.to_slice() {
            symlinks.push(format!("{} <- {}", realpath.as_str(), k));
        }
        true
    });
    symlinks.sort();

    let mut results = Vec::new();
    if let Value::Array(requests) = field(s, "requests") {
        for r in requests {
            let from = str_field(r, "from");
            let to = str_field(r, "to");
            let ending = str_field(r, "ending");
            let pref = str_field(r, "pref");
            let mode = match field(r, "mode") {
                Value::Number(n) => *n as i32,
                _ => 0,
            };
            let mut entries = vec![
                ("from", Value::String(from.clone())),
                ("to", Value::String(to.clone())),
                ("ending", Value::String(ending.clone())),
                ("mode", Value::Number(mode as f64)),
                ("pref", Value::String(pref.clone())),
            ];
            let Some(file) = program.get_source_file(&from) else {
                entries.extend([("specifiers", Value::Array(Vec::new())), ("kind", Value::Number(0.0)), ("error", Value::String("no importing file".into()))]);
                results.push(object(entries));
                continue;
            };
            let mut prefs = modulespecifiers::UserPreferences {
                import_module_specifier_preference: modulespecifiers::ImportModuleSpecifierPreference::ProjectRelative,
                ..Default::default()
            };
            prefs.import_module_specifier_preference = match pref.as_str() {
                "" => modulespecifiers::ImportModuleSpecifierPreference::ProjectRelative,
                "shortest" => modulespecifiers::ImportModuleSpecifierPreference::Shortest,
                "relative" => modulespecifiers::ImportModuleSpecifierPreference::Relative,
                "non-relative" => modulespecifiers::ImportModuleSpecifierPreference::NonRelative,
                _ => panic!("unknown pref {pref}"),
            };
            if ending == "js" {
                prefs.import_module_specifier_ending = modulespecifiers::ImportModuleSpecifierEndingPreference::Js;
            }
            let override_import_mode = match mode {
                0 => ModuleKind::None,
                1 => ModuleKind::CommonJS,
                99 => ModuleKind::ESNext,
                _ => panic!("unknown mode {mode}"),
            };
            let options = program.options();
            let (specifiers, kind) = modulespecifiers::get_module_specifiers_for_file_with_info(
                file,
                &to,
                &options,
                program,
                prefs,
                modulespecifiers::ModuleSpecifierOptions { override_import_mode },
                false,
            );
            entries.extend([("specifiers", strings(specifiers)), ("kind", Value::Number(kind as u8 as f64))]);
            results.push(object(entries));
        }
    }
    object(vec![("name", name), ("symlinks", strings(symlinks)), ("results", Value::Array(results))])
}

#[test]
fn modulespecifiers_oracle() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/modulespecifiers_oracle");
    let scenarios = std::fs::read_to_string(format!("{dir}/scenarios.jsonl")).unwrap();
    let expected = std::fs::read_to_string(format!("{dir}/expected.jsonl")).unwrap();
    let mut failures = Vec::new();
    let mut count = 0;
    for (s, e) in scenarios.lines().zip(expected.lines()) {
        let s = json::unmarshal(s).unwrap();
        let e = json::unmarshal(e).unwrap();
        let actual = run(&s);
        let (Value::Array(actual_results), Value::Array(expected_results)) = (field(&actual, "results"), field(&e, "results")) else {
            unreachable!()
        };
        if field(&actual, "symlinks") != field(&e, "symlinks") {
            failures.push(format!(
                "{}: symlinks\n  actual:   {}\n  expected: {}",
                str_field(&e, "name"),
                json::marshal(field(&actual, "symlinks")).unwrap(),
                json::marshal(field(&e, "symlinks")).unwrap()
            ));
        }
        assert_eq!(actual_results.len(), expected_results.len());
        for (a, x) in actual_results.iter().zip(expected_results) {
            count += 1;
            if field(a, "specifiers") != field(x, "specifiers") || field(a, "kind") != field(x, "kind") || field(a, "error") != field(x, "error") {
                failures.push(format!(
                    "{}: {} -> {} (ending {:?}, mode {}, pref {:?})\n  actual:   {} kind {}\n  expected: {} kind {}",
                    str_field(&e, "name"),
                    str_field(x, "from"),
                    str_field(x, "to"),
                    str_field(x, "ending"),
                    json::marshal(field(x, "mode")).unwrap(),
                    str_field(x, "pref"),
                    json::marshal(field(a, "specifiers")).unwrap(),
                    json::marshal(field(a, "kind")).unwrap(),
                    json::marshal(field(x, "specifiers")).unwrap(),
                    json::marshal(field(x, "kind")).unwrap(),
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{} of {count} requests differ:\n{}", failures.len(), failures.join("\n"));
}
