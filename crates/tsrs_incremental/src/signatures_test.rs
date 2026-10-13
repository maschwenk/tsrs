// Program::compute_placeholder_signatures replaces the version stand-ins of declaration signatures, which changes
// which files a later edit invalidates. A file whose dependency changed shape must still be checked again: after
// random edits to a chain of modules (inferred export types, a re-export, a global, syntax errors), the diagnostics of
// each incremental program must be those of a fresh build, whether or not the signatures were computed in between.

use std::collections::BTreeMap;
use std::sync::Arc;

use tsrs_ast::Diagnostic;
use tsrs_compiler::{new_compiler_host, new_program, CompilerHost, Context, Program as CompilerProgram, ProgramOptions};
use tsrs_core::tspath::Path;
use tsrs_core::P;
use tsrs_tsoptions::{self as tsoptions, ParseConfigHost};
use tsrs_vfs::{vfstest, FS};

use crate::emit::{EmitOptions, ProgramLike, WriteFileData};
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

// With noEmit and a syntax error, neither the semantic pass nor the emit handles the changed files: the change stays
// pending into the next program.
const CONFIGS: [&str; 2] = [
    r#"{ "compilerOptions": { "noLib": true, "types": [], "outDir": "/out" } }"#,
    r#"{ "compilerOptions": { "noLib": true, "types": [], "noEmit": true } }"#,
];

// The texts each file takes; the first is its initial text.
const VARIANTS: [(&str, &[&str]); 8] = [
    (
        "/src/a.ts",
        &[
            "export function f() { return 1; }\n",
            "export function f() { return 's'; }\n",
            "export function f() { return 1; }\n// a comment\n",
            "export function f() { return 1; }\nconst bad: string = 1;\n",
            "export function f() { return 1; }\nexport const extra = 2;\n",
        ],
    ),
    (
        "/src/b.ts",
        &[
            "import { f } from './a';\nexport const g = f();\n",
            "import { f } from './a';\nexport const g = f();\n// b comment\n",
            "import { f } from './a';\nexport const g: number = f();\n",
        ],
    ),
    ("/src/c.ts", &["import { g } from './b';\nexport const h: number = g;\n", "import { g } from './b';\nexport const h: string = g;\n"]),
    ("/src/d.ts", &["export * from './b';\n", "export * from './b';\nexport const dd = 1;\n"]),
    ("/src/e.ts", &["import { g } from './d';\nexport const k = [g];\n", "import { g } from './d';\nexport const k: boolean = g;\n"]),
    ("/src/f.ts", &["import { k } from './e';\nconst z: number[] = k;\n", "import { k } from './e';\nconst z: string[] = k;\n"]),
    ("/src/glob.ts", &["declare var G: number;\n", "declare var G: string;\n"]),
    ("/src/useglob.ts", &["export const u: number = G;\n", "export const u: number = G;\n// u comment\n"]),
];

// The global types the checker needs (noLib): without them every program has global errors, and tsc's passes stop
// before the semantic diagnostics.
const LIB: &str = "interface Array<T> { length: number; [n: number]: T; }\ninterface Boolean {}\ninterface Function {}\ninterface CallableFunction {}\ninterface NewableFunction {}\ninterface IArguments {}\ninterface Number {}\ninterface Object {}\ninterface RegExp {}\ninterface String {}\n";

fn files_of(config: &str, texts: &BTreeMap<&'static str, String>) -> Vec<(&'static str, String)> {
    let mut files = vec![("/src/tsconfig.json", config.to_string()), ("/src/lib.d.ts", LIB.to_string())];
    files.extend(texts.iter().map(|(&name, text)| (name, text.clone())));
    files
}

fn compiler_host(files: &[(&'static str, String)]) -> Arc<dyn CompilerHost> {
    let fs: Arc<dyn FS> = Arc::new(vfstest::from_map(files.iter().map(|(k, v)| (*k, v.as_str())), true));
    new_compiler_host("/", fs, "", None, None, None)
}

fn compiler_program(files: &[(&'static str, String)]) -> &'static CompilerProgram {
    let fs: Arc<dyn FS> = Arc::new(vfstest::from_map(files.iter().map(|(k, v)| (*k, v.as_str())), true));
    let config_host: &'static configHost = Box::leak(Box::new(configHost { fs }));
    let (config, diagnostics) = tsoptions::get_parsed_command_line_of_config_file("/src/tsconfig.json", None, None, config_host, None);
    assert!(diagnostics.is_empty());
    new_program(ProgramOptions::new(P::new(config.unwrap()), compiler_host(files)))
}

fn describe(diagnostics: &[P<Diagnostic>]) -> Vec<String> {
    let mut lines: Vec<String> = diagnostics
        .iter()
        .map(|d| format!("{}:{:?} TS{} {}", d.file().map(|f| f.file_name().to_string()).unwrap_or_default(), d.pos(), d.code(), d.message_text()))
        .collect();
    lines.sort();
    lines
}

// What the watch reports for an incremental program: tsc's diagnostics passes, then the emit.
fn incremental_diagnostics(ctx: &Context, program: &'static Program) -> Vec<String> {
    let mut diagnostics = crate::emit::get_diagnostics_of_any_program(
        ctx,
        &program,
        None,
        false,
        &mut |ctx, file| program.get_bind_diagnostics(ctx, file),
        &mut |ctx, file| program.get_semantic_diagnostics(ctx, file),
    );
    let write_file = |_: &str, _: &str, _: &mut WriteFileData| -> Result<(), String> { Ok(()) };
    if let Some(result) = program.emit(ctx, EmitOptions { write_file: Some(&write_file), ..Default::default() }) {
        diagnostics.extend(result.diagnostics.iter().copied());
    }
    describe(&diagnostics)
}

fn fresh_diagnostics(ctx: &Context, program: &'static CompilerProgram) -> Vec<String> {
    let mut diagnostics = tsrs_compiler::get_diagnostics_of_any_program(
        ctx,
        program,
        None,
        false,
        &mut |ctx, file| program.get_bind_diagnostics(ctx, file),
        &mut |ctx, file| program.get_semantic_diagnostics(ctx, file),
    );
    let write_file = |_: &str, _: &str, _: &mut WriteFileData| -> Result<(), String> { Ok(()) };
    diagnostics.extend(program.emit(ctx, &tsrs_compiler::EmitOptions { write_file: Some(&write_file), ..Default::default() }).diagnostics.iter().copied());
    describe(&diagnostics)
}

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

// Returns the number of signatures computed and the number of steps whose diagnostics include a type error.
fn run(config: &str, seed: u64, steps: usize) -> (usize, usize) {
    let ctx = Context::default();
    let mut rng = seed;
    let mut texts: BTreeMap<&'static str, String> = VARIANTS.iter().map(|&(name, variants)| (name, variants[0].to_string())).collect();
    let mut files = files_of(config, &texts);
    let mut compiler = compiler_program(&files);
    let mut program: &'static Program = Box::leak(Box::new(crate::new_program(compiler, None, crate::create_host(compiler_host(&files)), None, false))).get();
    assert_eq!(incremental_diagnostics(&ctx, program), fresh_diagnostics(&ctx, compiler_program(&files)));
    let mut stored = program.compute_placeholder_signatures(&ctx, 2);
    let mut type_errors = 0;
    for step in 0..steps {
        let (name, variants) = VARIANTS[(splitmix(&mut rng) % VARIANTS.len() as u64) as usize];
        let mut text = variants[(splitmix(&mut rng) % variants.len() as u64) as usize].to_string();
        if splitmix(&mut rng) % 7 == 0 {
            text.push_str("const = ;\n");
        }
        texts.insert(name, text);
        files = files_of(config, &texts);
        // Like the watch: reuse the program when the edit allows it, else build it in full.
        let host = compiler_host(&files);
        compiler = match compiler.reuse_program(&Path::from(name), Arc::clone(&host), None, None) {
            (Some(reused), _, true) => reused,
            _ => compiler_program(&files),
        };
        program = Box::leak(Box::new(crate::new_program(compiler, Some(P::from_static(program)), crate::create_host(host), None, false))).get();
        let fresh = fresh_diagnostics(&ctx, compiler_program(&files));
        type_errors += usize::from(fresh.iter().any(|d| d.contains(" TS2322 ")));
        assert_eq!(incremental_diagnostics(&ctx, program), fresh, "{config}\nseed {seed}, step {step}: {name}\n{texts:#?}");
        // The watch computes the signatures while it waits for the next change, unless the change comes first.
        if splitmix(&mut rng) % 3 != 0 {
            stored += program.compute_placeholder_signatures(&ctx, 2);
        }
    }
    (stored, type_errors)
}

#[test]
fn edits_invalidate_the_same_files_with_computed_signatures() {
    let (mut stored, mut type_errors) = (0, 0);
    for config in CONFIGS {
        for seed in 1..=6 {
            let (s, t) = run(config, seed, 150);
            stored += s;
            type_errors += t;
        }
    }
    assert!(stored > 100, "the signatures were computed ({stored})");
    assert!(type_errors > 300, "the semantic diagnostics were compared ({type_errors} steps with type errors)");
}

#[test]
fn computed_signatures_replace_only_stand_ins() {
    let ctx = Context::default();
    let texts: BTreeMap<&'static str, String> = VARIANTS.iter().map(|&(name, variants)| (name, variants[0].to_string())).collect();
    let files = files_of(CONFIGS[0], &texts);
    let program: &'static Program =
        Box::leak(Box::new(crate::new_program(compiler_program(&files), None, crate::create_host(compiler_host(&files)), None, false))).get();
    let a = Path::from("/src/a.ts");
    let info = program.snapshot.file_infos.load(&a).unwrap();
    assert_eq!(info.signature, info.version, "the initial build stores the version");
    // Every source file but none of the declaration files (there are none here) gets its declaration signature.
    assert_eq!(program.compute_placeholder_signatures(&ctx, 3), VARIANTS.len());
    let info = program.snapshot.file_infos.load(&a).unwrap();
    assert_ne!(info.signature, info.version);
    // glob.ts and d.ts are their own declaration output: their signature is their version, and is computed again.
    assert_eq!(program.compute_placeholder_signatures(&ctx, 3), 2);
}

#[test]
fn signatures_wait_for_pending_changes() {
    let ctx = Context::default();
    let texts: BTreeMap<&'static str, String> = VARIANTS.iter().map(|&(name, variants)| (name, variants[0].to_string())).collect();
    let files = files_of(CONFIGS[0], &texts);
    let old: &'static Program =
        Box::leak(Box::new(crate::new_program(compiler_program(&files), None, crate::create_host(compiler_host(&files)), None, false))).get();
    let mut edited = texts.clone();
    edited.insert("/src/a.ts", VARIANTS[0].1[1].to_string());
    let edited = files_of(CONFIGS[0], &edited);
    let new = crate::new_program(compiler_program(&edited), Some(P::from_static(old)), crate::create_host(compiler_host(&edited)), None, false);
    assert_eq!(new.snapshot.changed_files_set.size(), 1);
    assert_eq!(new.compute_placeholder_signatures(&ctx, 3), 0);
}
