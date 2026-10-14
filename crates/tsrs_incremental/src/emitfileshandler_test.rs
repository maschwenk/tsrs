// The incremental emit (emitfileshandler.rs) emits the affected files with one `emit_each_file` per kind of emit; each
// file's result must still be its own, as with Go's per-file emits.

use std::sync::{Arc, Mutex};

use tsrs_compiler::{new_compiler_host, new_program, CompilerHost, Context, ProgramOptions};
use tsrs_core::P;
use tsrs_tsoptions::{self as tsoptions, ParseConfigHost};
use tsrs_vfs::{vfstest, FS};

use crate::emit::{EmitOptions, ProgramLike, WriteFileData};

struct configHost {
    fs: Arc<dyn FS>,
}

impl ParseConfigHost for configHost {
    fn fs(&self) -> &dyn FS {
        &*self.fs
    }
    fn get_current_directory(&self) -> &str {
        "/"
    }
}

const FILES: [(&str, &str); 4] = [
    (
        "/src/tsconfig.json",
        r#"{ "compilerOptions": { "incremental": true, "declaration": true, "target": "es2015", "types": [], "outDir": "/out", "tsBuildInfoFile": "/out/tsconfig.tsbuildinfo" } }"#,
    ),
    ("/src/a.ts", "export const a = 1;\n"),
    ("/src/b.ts", "import { a } from './a';\nexport const b = a + 1;\n"),
    // A declaration emit error (TS4094), reported by c.ts's emit only.
    ("/src/c.ts", "export const c = class { private p = 1; };\n"),
];

fn compiler_host() -> Arc<dyn CompilerHost> {
    let fs: Arc<dyn FS> = Arc::new(vfstest::from_map(FILES.iter().map(|&(k, v)| (k, v)), true));
    new_compiler_host("/", fs, "", None, None, None)
}

fn config() -> P<tsoptions::ParsedCommandLine> {
    let fs: Arc<dyn FS> = Arc::new(vfstest::from_map(FILES.iter().map(|&(k, v)| (k, v)), true));
    let host: &'static configHost = Box::leak(Box::new(configHost { fs }));
    let (config, diagnostics) = tsoptions::get_parsed_command_line_of_config_file("/src/tsconfig.json", None, None, host, None);
    assert!(diagnostics.is_empty());
    P::new(config.unwrap())
}

// Emits with `emit` and returns the written files (without the build info), sorted by name, and the codes and files
// of the emit diagnostics.
fn written_files(emit: impl FnOnce(&(dyn Fn(&str, &str, &mut WriteFileData) -> Result<(), String> + Sync)) -> Vec<(i32, String)>) -> (Vec<(String, String)>, Vec<(i32, String)>) {
    let written = Mutex::new(Vec::new());
    let write_file = |name: &str, text: &str, _: &mut WriteFileData| -> Result<(), String> {
        written.lock().unwrap().push((name.to_string(), text.to_string()));
        Ok(())
    };
    let mut diagnostics = emit(&write_file);
    diagnostics.sort();
    let mut written = written.into_inner().unwrap();
    written.retain(|(name, _)| !name.ends_with(".tsbuildinfo"));
    written.sort();
    (written, diagnostics)
}

fn codes_and_files(diagnostics: &[P<tsrs_ast::Diagnostic>]) -> Vec<(i32, String)> {
    diagnostics.iter().map(|d| (d.code(), d.file().map(|f| f.file_name().to_string()).unwrap_or_default())).collect()
}

#[test]
fn incremental_emit_matches_a_plain_emit_file_by_file() {
    let ctx = Context::default();
    let plain = new_program(ProgramOptions::new(config(), compiler_host()));
    let (plain_files, plain_diagnostics) = written_files(|write_file| {
        let result = plain.emit(&ctx, &EmitOptions { write_file: Some(write_file), ..Default::default() });
        codes_and_files(&result.diagnostics)
    });

    let host = compiler_host();
    let program = crate::new_program(new_program(ProgramOptions::new(config(), Arc::clone(&host))), None, crate::create_host(host), None, false);
    let incremental: &'static crate::Program = &*Box::leak(Box::new(program));
    let (files, diagnostics) = written_files(|write_file| {
        let result = incremental.emit(&ctx, EmitOptions { write_file: Some(write_file), ..Default::default() }).unwrap();
        codes_and_files(&result.diagnostics)
    });

    assert_eq!(files.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), // c.d.ts has an error, so it is not written.
        ["/out/a.d.ts", "/out/a.js", "/out/b.d.ts", "/out/b.js", "/out/c.js"]);
    assert_eq!(files, plain_files);
    assert_eq!(diagnostics, plain_diagnostics);
    assert_eq!(diagnostics.iter().map(|(code, file)| (*code, file.as_str())).collect::<Vec<_>>(), [(4094, "/src/c.ts")]);
    // The emit diagnostics are kept for the file that reported them only.
    let mut kept = incremental.snapshot.emit_diagnostics_per_file.keys();
    kept.sort();
    assert_eq!(kept.iter().map(|p| p.as_str()).collect::<Vec<_>>(), ["/src/c.ts"]);
}
