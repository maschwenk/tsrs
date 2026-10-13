// program_to_snapshot takes shortcuts when the new program shares files with the old state's program (versions of
// shared files, the deleted-file and global-scope passes, reference sets compared by their shared part once). Each
// test checks that the snapshot is the one the full computation gives.

use std::sync::{Arc, Mutex};

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

const FILES: [(&str, &str); 8] = [
    ("/src/tsconfig.json", r#"{ "compilerOptions": { "noLib": true, "types": [], "checkers": 2 } }"#),
    ("/src/a.ts", "export const a = 1;\n"),
    ("/src/b.ts", "/// <reference path=\"./g.ts\" />\nimport { a } from './a';\nexport const b = a;\n"),
    // A script: it affects the global scope.
    ("/src/g.ts", "declare var g: number;\n"),
    // Ambient modules: every file references the files declaring them.
    ("/src/m.d.ts", "declare module \"m\" { export const x: number; }\n"),
    ("/src/c.ts", "import { x } from \"m\";\nexport const c = x;\n"),
    ("/src/d.ts", "import { b } from './b';\nexport const d = b;\n"),
    // A module augmentation: the importers of a.ts reference it.
    ("/src/aug.ts", "import './a';\ndeclare module './a' { interface I { x: number } }\nexport {};\n"),
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
    compiler_program_with(files, compiler_host(files))
}

fn compiler_program_with(files: &[(&'static str, &'static str)], host: Arc<dyn CompilerHost>) -> &'static CompilerProgram {
    let fs: Arc<dyn FS> = Arc::new(vfstest::from_map(files.iter().copied(), true));
    let config_host: &'static configHost = Box::leak(Box::new(configHost { fs }));
    let (config, diagnostics) = tsoptions::get_parsed_command_line_of_config_file("/src/tsconfig.json", None, None, config_host, None);
    assert!(diagnostics.is_empty());
    new_program(ProgramOptions::new(P::new(config.unwrap()), host))
}

// A host that hands out the file it parsed first for each path, like the watch's source file cache.
struct cachingHost {
    inner: Arc<dyn CompilerHost>,
    cache: Arc<Mutex<rustc_hash::FxHashMap<Path, P<tsrs_ast::SourceFile>>>>,
}

impl CompilerHost for cachingHost {
    fn fs(&self) -> &dyn FS {
        self.inner.fs()
    }
    fn default_library_path(&self) -> &str {
        self.inner.default_library_path()
    }
    fn get_current_directory(&self) -> &str {
        self.inner.get_current_directory()
    }
    fn trace(&self, msg: &'static tsrs_diagnostics::Message, args: &[&dyn std::fmt::Display]) {
        self.inner.trace(msg, args);
    }
    fn get_source_file(&self, opts: tsrs_ast::SourceFileParseOptions) -> Option<P<tsrs_ast::SourceFile>> {
        if let Some(&file) = self.cache.lock().unwrap().get(&opts.path) {
            return Some(file);
        }
        let path = opts.path.clone();
        let file = self.inner.get_source_file(opts)?;
        self.cache.lock().unwrap().insert(path, file);
        Some(file)
    }
    fn get_content_mapped_source_files(
        &self,
        parse_options: tsrs_ast::SourceFileParseOptions,
        mapper: &tsrs_tsoptions::Mapper,
    ) -> Result<tsrs_contentmapper::SourceFiles, tsrs_contentmapper::Error> {
        self.inner.get_content_mapped_source_files(parse_options, mapper)
    }
    fn content_mapper_project(&self) -> Option<Arc<dyn tsrs_contentmapper::Project>> {
        self.inner.content_mapper_project()
    }
    fn get_resolved_project_reference(&self, file_name: &str, path: Path) -> Option<P<tsoptions::ParsedCommandLine>> {
        self.inner.get_resolved_project_reference(file_name, path)
    }
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
    assert_eq!(b.3, ["/src/a.ts", "/src/aug.ts", "/src/g.ts", "/src/m.d.ts"]);
}

// A reused program keeps the references of the files it did not reparse (programtosnapshot.rs `keeps_references`):
// they are the ones a fresh snapshot computes when the reparsed file declares module augmentations or ambient modules.
#[test]
fn a_reused_program_keeps_the_references_of_the_other_files() {
    for (name, text) in [
        ("/src/aug.ts", "import './a';\ndeclare module './a' { interface I { y: string } }\nexport {};\n"),
        ("/src/m.d.ts", "declare module \"m\" { export const x: string; export const z: number; }\n"),
        ("/src/g.ts", "declare var g: string;\n"),
    ] {
        let old_program = compiler_program(&FILES);
        let old = incremental(old_program, None);
        let edited = with(&[(name, Some(text))]);
        let (reused, _, ok) = old_program.reuse_program(&Path::from(name), compiler_host(&edited), None, None);
        assert!(ok, "{name}");
        let reused = incremental(reused.unwrap(), Some(old));
        assert_eq!(file_states(&reused), file_states(&incremental(compiler_program(&edited), None)), "{name}");
        assert_eq!(changed_files(&reused), [name]);
    }
}

#[test]
fn deleting_a_global_file_changes_every_file() {
    let old = incremental(compiler_program(&FILES), None);
    let new = incremental(compiler_program(&with(&[("/src/g.ts", None)])), Some(old));
    assert_eq!(changed_files(&new), ["/src/a.ts", "/src/aug.ts", "/src/b.ts", "/src/c.ts", "/src/d.ts", "/src/m.d.ts"]);
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
    assert_eq!(changed_files(&new), ["/src/a.ts", "/src/aug.ts", "/src/b.ts", "/src/c.ts", "/src/d.ts", "/src/g.ts", "/src/m.d.ts"]);
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

// A program built in full keeps no references, even for the files it shares with the old state's program: the
// module resolution may differ (here a file that was missing appears).
#[test]
fn a_full_rebuild_that_shares_files_looks_up_their_references_again() {
    let cache = Arc::new(Mutex::new(rustc_hash::FxHashMap::default()));
    let before = with(&[("/src/e.ts", Some("import { x } from './x';\nexport const e = x;\n"))]);
    let after = with(&[("/src/e.ts", Some("import { x } from './x';\nexport const e = x;\n")), ("/src/x.ts", Some("export const x = 1;\n"))]);
    let old_program = compiler_program_with(&before, Arc::new(cachingHost { inner: compiler_host(&before), cache: Arc::clone(&cache) }));
    let old = incremental(old_program, None);
    let new_program = compiler_program_with(&after, Arc::new(cachingHost { inner: compiler_host(&after), cache }));
    let e = Path::from("/src/e.ts");
    assert_eq!(old_program.get_source_file_by_path(&e), new_program.get_source_file_by_path(&e), "e.ts is shared");
    let new = incremental(new_program, Some(old));
    assert_eq!(file_states(&new), file_states(&incremental(new_program, None)));
    let e = file_states(&new).into_iter().find(|(p, ..)| p.as_str() == "/src/e.ts").unwrap();
    assert_eq!(e.3, ["/src/m.d.ts", "/src/x.ts"]);
}
