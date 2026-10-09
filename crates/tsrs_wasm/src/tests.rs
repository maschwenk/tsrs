// `run()` end to end on native targets, over the in-memory `host::test_host`.

use std::io::Write;
use std::sync::{Arc, Mutex, MutexGuard};

use tsrs_ast::{new_diagnostic_from_text, Diagnostic};
use tsrs_core::{undefined_text_range, P};
use tsrs_diagnostics::Category;
use tsrs_execute::tsc::System;

use super::*;

// The test host is one process-wide map: one test at a time.
static LOCK: Mutex<()> = Mutex::new(());

struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn setup(files: &[(&str, &str)]) -> MutexGuard<'static, ()> {
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset(files);
    guard
}

fn reset(files: &[(&str, &str)]) {
    let mut map = host::test_host::FILES.lock().unwrap();
    map.clear();
    for (path, text) in files {
        map.insert(path.to_string(), text.as_bytes().to_vec());
    }
    host::test_host::FAIL_WRITES.lock().unwrap().clear();
}

fn request(cwd: &str, flags: u32, args: &[&str]) -> Vec<u8> {
    let mut fields = vec![cwd.to_string(), flags.to_string()];
    fields.extend(args.iter().map(|a| a.to_string()));
    fields.join("\0").into_bytes()
}

/// (status, reply, stdout)
fn run_captured(cwd: &str, flags: u32, args: &[&str]) -> (i32, Vec<u8>, String) {
    let out = Arc::new(Mutex::new(Vec::new()));
    let (status, reply) = run_with_output(&request(cwd, flags, args), Box::new(Capture(Arc::clone(&out))));
    let stdout = String::from_utf8(out.lock().unwrap().clone()).unwrap();
    (status, reply, stdout)
}

const ERRORS: &str = "const s: string = 1;\n// é and 😀 move the UTF-16 offsets\nconst n: number = 'x';\nlet u = undefinedName;\n";

// Regression: the module's JSON encoder drifts from the API's DiagnosticResponse wire format (field order, UTF-16
// positions, source lines, file-less diagnostics), so `diagnostics: "json"` callers get a different shape than
// the Node API returns.
#[test]
fn json_reply_matches_the_api_encoder() {
    let _guard = setup(&[("/p/a.ts", ERRORS)]);
    let list = Arc::new(Mutex::new(Vec::<P<Diagnostic>>::new()));
    let sys: &'static sys::WasmSys = Box::leak(Box::new(sys::WasmSys::new("/p", true, false, Some(Arc::clone(&list)), Box::new(std::io::sink()))));
    let result = tsrs_execute::execute::command_line(sys, vec!["a.ts".into(), "--noEmit".into(), "--unknownOption".into()]);
    tsrs_execute::execute::command_line(sys, vec!["a.ts".into(), "--noEmit".into()]);
    sys.flush();
    let mut diagnostics = list.lock().unwrap().clone();
    assert!(!diagnostics.is_empty(), "the bad option is reported");
    let file = diagnostics.iter().find_map(|d| d.file()).expect("type errors have a source file");
    diagnostics.push(new_diagnostic_from_text(
        Some(file),
        undefined_text_range(),
        9999,
        Category::Error,
        "synthetic location",
        &[],
        &[],
        false,
        false,
    ));
    let ours = json::encode(&diagnostics);
    let api = tsrs_core::json::marshal(&tsrs_api::diagnostics::diagnostic_responses(&diagnostics)).unwrap();
    assert_eq!(String::from_utf8(ours).unwrap(), api);
    assert!(api.contains("\"pos\":0,\"end\":0"), "file diagnostics with synthetic locations clamp to the start of the file: {api}");
    assert_eq!(result.status as i32, 1);

    reset(&[("/p/a.ts", ERRORS)]);
    let (status, reply, stdout) = run_captured("/p", REQUEST_JSON_DIAGNOSTICS, &["a.ts", "--noEmit"]);
    assert_eq!(status, 2);
    assert_eq!(stdout, "", "diagnostics go to the reply, not stdout");
    let reply = String::from_utf8(reply).unwrap();
    assert_eq!(reply.matches("\"code\":2322").count(), 2, "{reply}");
    assert!(reply.contains("\"code\":2552"), "{reply}");
}

// Regression: emit through the host loses or changes files (writes, BOM, the declaration and map files) compared
// with native tsrs over the OS file system.
#[test]
fn emit_through_the_host_matches_native() {
    let dir = std::env::temp_dir().join(format!("tsrs-wasm-emit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let root = tsrs_core::tspath::normalize_path(&dir.canonicalize().unwrap().to_string_lossy());
    let config = r#"{ "compilerOptions": { "rootDir": "src", "outDir": "out", "declaration": true, "sourceMap": true, "declarationMap": true, "emitBOM": true }, "include": ["src"] }"#;
    let a = "export const a: number = 1;\nexport function f(x: string) { return x.length; }\n";
    let b = "import { f } from './a';\nexport default f('é');\n";
    std::fs::write(dir.join("tsconfig.json"), config).unwrap();
    std::fs::write(dir.join("src/a.ts"), a).unwrap();
    std::fs::write(dir.join("src/b.ts"), b).unwrap();

    struct OsSys {
        fs: Arc<dyn tsrs_vfs::FS>,
        cwd: String,
        start: std::time::Instant,
    }
    impl System for OsSys {
        fn fs(&self) -> Arc<dyn tsrs_vfs::FS> {
            Arc::clone(&self.fs)
        }
        fn default_library_path(&self) -> &str {
            Box::leak(tsrs_vfs::bundled::lib_path().into_boxed_str())
        }
        fn get_current_directory(&self) -> &str {
            &self.cwd
        }
        fn write(&self, _: &str) {}
        fn flush(&self) {}
        fn write_output_is_tty(&self) -> bool {
            false
        }
        fn get_environment_variable(&self, _: &str) -> Option<String> {
            None
        }
        fn now(&self) -> std::time::Instant {
            std::time::Instant::now()
        }
        fn since_start(&self) -> std::time::Duration {
            self.start.elapsed()
        }
    }
    let _guard = setup(&[(&format!("{root}/tsconfig.json"), config), (&format!("{root}/src/a.ts"), a), (&format!("{root}/src/b.ts"), b)]);
    let native: &'static OsSys = Box::leak(Box::new(OsSys { fs: Arc::new(tsrs_vfs::bundled::wrap_fs(tsrs_vfs::osvfs::fs())), cwd: root.clone(), start: std::time::Instant::now() }));
    let native_status = tsrs_execute::execute::command_line(native, vec!["-p".into(), ".".into()]).status as i32;
    let (status, _, stdout) = run_captured(&root, 0, &["-p", "."]);
    let listing: Vec<_> = std::fs::read_dir(dir.join("out")).map(|r| r.map(|e| e.unwrap().file_name()).collect()).unwrap_or_default();
    assert_eq!((status, native_status), (0, 0), "{stdout} {listing:?}");

    let mut expected = Vec::new();
    for name in ["a.js", "a.js.map", "a.d.ts", "a.d.ts.map", "b.js", "b.js.map", "b.d.ts", "b.d.ts.map"] {
        expected.push((format!("{root}/out/{name}"), std::fs::read(dir.join("out").join(name)).unwrap()));
    }
    expected.sort();
    let files = host::test_host::FILES.lock().unwrap();
    let written: Vec<(String, Vec<u8>)> = files.iter().filter(|(k, _)| k.contains("/out/")).map(|(k, v)| (k.clone(), v.clone())).collect();
    assert!(written == expected, "{:?} != {:?}", written.iter().map(|w| &w.0).collect::<Vec<_>>(), expected.iter().map(|w| &w.0).collect::<Vec<_>>());
    assert!(written[0].1.starts_with(&[0xEF, 0xBB, 0xBF]), "--emitBOM keeps the BOM");
    drop(files);
    let _ = std::fs::remove_dir_all(&dir);
}

// Regression: a host write error is swallowed or rewritten, so TS5033 no longer shows native tsrs's text.
#[test]
fn host_write_error_reaches_ts5033() {
    let _guard = setup(&[("/p/a.ts", "export const a = 1;\n")]);
    host::test_host::FAIL_WRITES.lock().unwrap().push(("/p/out".to_string(), "Permission denied (os error 13)".to_string()));
    let (status, _, stdout) = run_captured("/p", 0, &["a.ts", "--outDir", "out", "--pretty", "false"]);
    assert_eq!(stdout, "error TS5033: Could not write file '/p/out/a.js': Permission denied (os error 13).\n");
    assert_eq!(status, 2);
}

// Regression: `tsc -b` with JSON diagnostics runs and prints build-mode text that the JSON reply cannot carry.
#[test]
fn build_with_json_is_refused() {
    let _guard = setup(&[("/p/tsconfig.json", "{}"), ("/p/a.ts", "export const a = 1;\n")]);
    let (status, reply, stdout) = run_captured("/p", REQUEST_JSON_DIAGNOSTICS, &["-b"]);
    assert_eq!((status, stdout.as_str()), (1, "error: diagnostics as JSON are not supported with --build\n"));
    assert_eq!(reply, b"[]");
}
