// program_to_snapshot takes shortcuts when the new program shares files with the old state's program (versions of
// shared files, the deleted-file and global-scope passes, reference sets compared by their shared part once). Each
// test checks that the snapshot is the one the full computation gives.

use std::sync::Arc;

use tsrs_compiler::{new_compiler_host, new_program, CompilerHost, Program as CompilerProgram, ProgramOptions};
use tsrs_core::tspath::Path;
use tsrs_core::P;
use tsrs_tsoptions::{self as tsoptions, ParseConfigHost};
use tsrs_vfs::{vfstest, FS};

use crate::program::Program;

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

const FILES: [(&str, &str); 7] = [
    ("/src/tsconfig.json", r#"{ "compilerOptions": { "noLib": true, "types": [], "checkers": 2 } }"#),
    ("/src/a.ts", "export const a = 1;\n"),
    ("/src/b.ts", "/// <reference path=\"./g.ts\" />\nimport { a } from './a';\nexport const b = a;\n"),
    // A script: it affects the global scope.
    ("/src/g.ts", "declare var g: number;\n"),
    // Ambient modules: every file references the files declaring them.
    ("/src/m.d.ts", "declare module \"m\" { export const x: number; }\n"),
    ("/src/c.ts", "import { x } from \"m\";\nexport const c = x;\n"),
    ("/src/d.ts", "import { b } from './b';\nexport const d = b;\n"),
];

fn with(changes: &[(&'static str, Option<&'static str>)]) -> Vec<(&'static str, &'static str)> {
    let mut files: Vec<(&str, &str)> = FILES.to_vec();
    for &(name, text) in changes {
        files.retain(|&(n, _)| n != name);
        if let Some(text) = text {
            files.push((name, text));
        }
    }
    files
}

fn compiler_host(files: &[(&'static str, &'static str)]) -> Arc<dyn CompilerHost> {
    let fs: Arc<dyn FS> = Arc::new(vfstest::from_map(files.iter().copied(), true));
    new_compiler_host("/", fs, "", None, None, None)
}

fn compiler_program(files: &[(&'static str, &'static str)]) -> &'static CompilerProgram {
    let fs: Arc<dyn FS> = Arc::new(vfstest::from_map(files.iter().copied(), true));
    let host: &'static configHost = Box::leak(Box::new(configHost { fs }));
    let (config, diagnostics) = tsoptions::get_parsed_command_line_of_config_file("/src/tsconfig.json", None, None, host, None);
    assert!(diagnostics.is_empty());
    new_program(ProgramOptions::new(P::new(config.unwrap()), compiler_host(files)))
}

fn incremental(program: &'static CompilerProgram, old: Option<P<Program>>) -> P<Program> {
    crate::new_program(program, old, crate::create_host(compiler_host(&FILES)), None, false)
}

// Per file: the version, whether it affects the global scope, and its references, sorted.
fn file_states(program: &Program) -> Vec<(Path, String, bool, Vec<String>)> {
    let snapshot = program.snapshot;
    let mut paths = snapshot.file_infos.keys();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let info = snapshot.file_infos.load(&path).unwrap();
            let mut references: Vec<String> =
                snapshot.referenced_map.get_references(&path).map(|refs| refs.iter().map(|p| p.to_string()).collect()).unwrap_or_default();
            references.sort();
            (path, info.version, info.affects_global_scope, references)
        })
        .collect()
}

fn changed_files(program: &Program) -> Vec<String> {
    let mut changed: Vec<String> = program.snapshot.changed_files_set.keys().iter().map(|p| p.to_string()).collect();
    changed.sort();
    changed
}

#[test]
fn a_reused_program_has_the_snapshot_of_a_fresh_one() {
    let old_program = compiler_program(&FILES);
    let old = incremental(old_program, None);
    let edited = with(&[("/src/a.ts", Some("export const a = 1;\nconst local = 2;\n"))]);
    let (reused, _, ok) = old_program.reuse_program(&Path::from("/src/a.ts"), compiler_host(&edited), None, None);
    assert!(ok);
    let reused = incremental(reused.unwrap(), Some(old));

    // The same files, parsed again: nothing is shared with the old state's program.
    let fresh_program = compiler_program(&edited);
    let fresh = incremental(fresh_program, Some(old));
    let from_scratch = incremental(fresh_program, None);

    assert_eq!(file_states(&reused), file_states(&from_scratch));
    assert_eq!(file_states(&fresh), file_states(&from_scratch));
    assert_eq!(changed_files(&reused), ["/src/a.ts"]);
    assert_eq!(changed_files(&fresh), ["/src/a.ts"]);
    let b = file_states(&reused).into_iter().find(|(p, ..)| p.as_str() == "/src/b.ts").unwrap();
    assert_eq!(b.3, ["/src/a.ts", "/src/g.ts", "/src/m.d.ts"]);
}

#[test]
fn deleting_a_global_file_changes_every_file() {
    let old = incremental(compiler_program(&FILES), None);
    let new = incremental(compiler_program(&with(&[("/src/g.ts", None)])), Some(old));
    assert_eq!(changed_files(&new), ["/src/a.ts", "/src/b.ts", "/src/c.ts", "/src/d.ts", "/src/m.d.ts"]);
}

#[test]
fn deleting_a_module_changes_its_importers_only() {
    let old = incremental(compiler_program(&FILES), None);
    let new = incremental(compiler_program(&with(&[("/src/d.ts", None)])), Some(old));
    assert!(changed_files(&new).is_empty());
    assert!(new.snapshot.build_info_emit_pending.get());
}

#[test]
fn a_file_that_stops_affecting_the_global_scope_changes_every_file() {
    let old = incremental(compiler_program(&FILES), None);
    let new = incremental(compiler_program(&with(&[("/src/g.ts", Some("export {};\ndeclare var g: number;\n"))])), Some(old));
    assert_eq!(changed_files(&new), ["/src/a.ts", "/src/b.ts", "/src/c.ts", "/src/d.ts", "/src/g.ts", "/src/m.d.ts"]);
}

#[test]
fn a_new_ambient_module_changes_the_shared_references() {
    let old_program = compiler_program(&FILES);
    let old = incremental(old_program, None);
    let new_program = compiler_program(&with(&[("/src/n.d.ts", Some("declare module \"n\" { export const y: number; }\n"))]));
    let new = incremental(new_program, Some(old));
    assert_eq!(file_states(&new), file_states(&incremental(new_program, None)));
    let a = file_states(&new).into_iter().find(|(p, ..)| p.as_str() == "/src/a.ts").unwrap();
    assert_eq!(a.3, ["/src/m.d.ts", "/src/n.d.ts"]);
}
