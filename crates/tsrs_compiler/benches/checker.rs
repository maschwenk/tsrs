//! Program construction and type checking of a small in-memory project (benches/fixtures) against the bundled ES5
//! lib, single-threaded.
//!
//! Each iteration builds its program inside its own arena region, freed afterwards (as the API's transpile does), so
//! repeated iterations do not grow the process. Single-threaded so that every allocation lands in that region.
//!
//! `cargo bench -p tsrs_compiler`; under CodSpeed: `cargo codspeed build -p tsrs_compiler && cargo codspeed run`.

use std::sync::Arc;

use tsrs_compiler::{free_unshared_program, new_compiler_host, new_program, CompilerHost, Context, Program, ProgramOptions};
use tsrs_core::arena::Region;
use tsrs_core::{tspath, CompilerOptions, ModuleKind, ScriptTarget, Tristate, P};
use tsrs_tsoptions as tsoptions;
use tsrs_vfs::{bundled, vfstest, FS};

const MODELS_TS: &str = include_str!("fixtures/models.ts");
const STORE_TS: &str = include_str!("fixtures/store.ts");

fn main() {
    divan::main();
}

/// Builds the program in the current region (host, module resolution, parse and bind of the project and lib files).
/// Equivalent to a tsconfig with `strict`, `target: es2015`, `module: commonjs`, `lib: ["es5"]`, `types: []`.
fn build_program() -> &'static Program {
    let files = [("/project/models.ts", MODELS_TS), ("/project/store.ts", STORE_TS)];
    let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(vfstest::from_map(files, true)));
    let options = CompilerOptions {
        strict: Tristate::True,
        target: ScriptTarget::ES2015,
        module: ModuleKind::CommonJS,
        lib: Some(vec!["lib.es5.d.ts".to_string()]),
        types: Some(Vec::new()),
        no_emit: Tristate::True,
        ..Default::default()
    };
    let roots = files.iter().map(|(name, _)| (*name).to_string()).collect();
    let config = tsoptions::new_parsed_command_line(P::new(options), roots, Vec::new(), tspath::ComparePathsOptions::default());
    let host: Arc<dyn CompilerHost> = new_compiler_host("/project", fs, &bundled::lib_path(), None, None, None);
    let mut program_options = ProgramOptions::new(P::new(config), host);
    program_options.single_threaded = Tristate::True;
    new_program(program_options)
}

/// Runs `f` on a fresh program inside its own region, then frees both.
fn with_program<R>(f: impl FnOnce(&'static Program) -> R) -> R {
    let region = Region::new(1 << 20);
    let result = {
        let _scope = region.enter();
        let program = build_program();
        let result = f(program);
        // SAFETY: the program was made by `new_program` above and shares nothing with another program; `f` returns
        // only plain values, so nothing derived from it is used after this.
        unsafe { free_unshared_program(program) };
        result
    };
    drop(region);
    result
}

/// Program construction only: module resolution, parse and bind of every file (project and lib).
#[divan::bench]
fn create_program() -> usize {
    with_program(|program| program.get_source_files().len())
}

/// Program construction followed by the full set of diagnostics a `tsc --noEmit` run reports.
#[divan::bench]
fn check_project() -> usize {
    with_program(|program| {
        let ctx = Context::background();
        let mut count = program.get_syntactic_diagnostics(&ctx, None).len();
        count += program.get_bind_diagnostics(&ctx, None).len();
        count += program.get_semantic_diagnostics(&ctx, None).len();
        count += program.get_global_diagnostics(&ctx).len();
        count
    })
}
