use std::sync::Arc;

use tsrs_compiler::{new_compiler_host, Context, ProgramOptions};
use tsrs_core::{CompilerOptions, ModuleKind, Tristate, P};
use tsrs_incremental::emit::ProgramLike;
use tsrs_incremental::{create_host, new_program, Program, SemanticDiagnosticsState};

// The public incremental API must preserve useful in-memory reuse, but not a cached absence after an export
// alias changes the virtual checker slots. This uses the real OS filesystem, resolver, binder and checkers.
#[test]
fn in_memory_interface_diagnostics_require_unchanged_report_slots() {
    let dir = std::env::temp_dir().join(format!("tsrs-in-memory-merged-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("node_modules/p")).unwrap();
    let dir = dir.canonicalize().unwrap();
    let files = ["node_modules/p/a.ts", "node_modules/p/b.ts", "c.ts", "d.ts", "e.ts", "f.ts", "g.ts", "h.ts"];
    for (i, file) in files.iter().enumerate() {
        let imports = if i == 4 { "import 'p/a';\nimport 'p/b';\nimport 'p/alias';\n" } else { "import 'missing/a';\nimport 'missing/b';\nimport 'missing/c';\n" };
        let (body, nodes) = match i {
            0 => ("declare global { interface Array<T> { length: number; [n: number]: T } interface Boolean {} interface CallableFunction {} interface Function {} interface IArguments {} interface NewableFunction {} interface Number {} interface Object {} interface RegExp {} interface String {} interface Left { kind: 'l' } interface Right { kind: 'r' } interface M extends Left, Right {} }\n", 64),
            4 => ("declare global { interface M { extra?: number } }\n", 18),
            _ => ("", 8),
        };
        let text = format!("{imports}{body}");
        let padding = (100 - nodes) * 100 - text.len() - 4;
        std::fs::write(dir.join(file), format!("{text}/*{}*/", "x".repeat(padding))).unwrap();
    }
    let package = |target: &str| {
        std::fs::write(dir.join("node_modules/p/package.json"), format!(r#"{{"name":"p","version":"1.0.0","type":"module","exports":{{"./a":"./a.ts","./b":"./b.ts","./alias":"./{target}.ts"}}}}"#)).unwrap();
    };
    let build = |old: Option<P<Program>>| {
        let host = new_compiler_host(dir.to_str().unwrap(), Arc::new(tsrs_vfs::osvfs::fs()), "", None, None, None);
        let options = CompilerOptions { incremental: Tristate::True, no_emit: Tristate::True, no_lib: Tristate::True, module: ModuleKind::NodeNext, ..Default::default() };
        let config = P::new(tsrs_tsoptions::new_parsed_command_line(P::new(options), files.iter().map(|file| dir.join(file).to_str().unwrap().to_string()).collect(), Vec::new(), Default::default()));
        let compiler = tsrs_compiler::new_program(ProgramOptions::new(config, Arc::clone(&host)));
        new_program(compiler, old, create_host(host), None, true)
    };
    let sites = |program: P<Program>| {
        program.get().get_semantic_diagnostics(&Context::default(), None).into_iter().map(|diag| (diag.code(), diag.file().unwrap().file_name().to_string(), diag.pos(), diag.len(), diag.to_string())).collect::<Vec<_>>()
    };
    let state = |program: P<Program>, file: &str| {
        let file = program.get_program().get_source_file(dir.join(file).to_str().unwrap()).unwrap();
        program.testing_semantic_diagnostics_state(file.path())
    };
    package("a");
    let initial = build(None);
    let expected = sites(initial);
    assert_eq!(expected.iter().filter(|diagnostic| diagnostic.0 == 2320).count(), 1, "{expected:#?}");
    let same = new_program(initial.get_program(), Some(initial), create_host(Arc::clone(initial.get_program().host())), None, true);
    assert_eq!(sites(same), expected, "same compiler program fast path");
    assert_eq!(state(same, "e.ts"), SemanticDiagnosticsState::Reused);
    let unchanged = build(Some(same));
    assert_eq!(state(unchanged, "e.ts"), SemanticDiagnosticsState::Reused, "an unchanged new program must reuse cached absence");
    assert_eq!(sites(unchanged), expected);
    package("b");
    let changed = build(Some(unchanged));
    assert_eq!(state(changed, "e.ts"), SemanticDiagnosticsState::NotCached, "changed report slots must invalidate cached absence");
    assert_eq!(state(changed, "f.ts"), SemanticDiagnosticsState::Reused, "non-interface diagnostics remain reusable");
    let fresh = sites(build(None));
    assert_eq!(fresh.iter().filter(|diagnostic| diagnostic.0 == 2320).count(), 2, "{fresh:#?}");
    assert_eq!(sites(changed), fresh);
    package("a");
    let reversed = build(Some(changed));
    assert_eq!(state(reversed, "e.ts"), SemanticDiagnosticsState::NotCached, "changed report slots must invalidate cached errors");
    assert_eq!(sites(reversed), sites(build(None)));
    std::fs::remove_dir_all(dir).unwrap();
}
