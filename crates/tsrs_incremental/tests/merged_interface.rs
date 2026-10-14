use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tsrs_compiler::{new_compiler_host, Context, ProgramOptions};
use tsrs_core::{CompilerOptions, ModuleKind, Tristate, P};
use tsrs_incremental::emit::{EmitOptions, ProgramLike, WriteFileData};
use tsrs_incremental::{
    create_host, new_build_info_reader_with_canonical_diagnostics, new_program, read_build_info_program, BuildInfo, Program, SemanticDiagnosticsState,
};

// In-memory and loaded incremental caches must reuse unchanged interfaces, but not stale report sites after
// an export alias change. This uses the real OS filesystem, resolver, binder, checkers and build-info writer.
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
    let build_info_path = dir.join("tsconfig.tsbuildinfo");
    let options = CompilerOptions { incremental: Tristate::True, no_emit: Tristate::True, no_lib: Tristate::True, module: ModuleKind::NodeNext, ts_build_info_file: build_info_path.to_str().unwrap().to_string(), ..Default::default() };
    let config = P::new(tsrs_tsoptions::new_parsed_command_line(P::new(options), files.iter().map(|file| dir.join(file).to_str().unwrap().to_string()).collect(), Vec::new(), Default::default()));
    let create_with_policy = |old: Option<P<Program>>, loaded: bool, canonical_diagnostics: bool| {
        let host = new_compiler_host(dir.to_str().unwrap(), Arc::new(tsrs_vfs::osvfs::fs()), "", None, None, None);
        let old = if loaded {
            read_build_info_program(config, &*new_build_info_reader_with_canonical_diagnostics(Arc::clone(&host), canonical_diagnostics), &*host)
        } else {
            old
        };
        let mut opts = ProgramOptions::new(config, Arc::clone(&host));
        opts.canonical_diagnostics = canonical_diagnostics;
        let compiler = tsrs_compiler::new_program(opts);
        new_program(compiler, old, create_host(host), None, true)
    };
    let create = |old, loaded| create_with_policy(old, loaded, true);
    let build = |old| create(old, false);
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
    let writes = AtomicUsize::new(0);
    let writer = |path: &str, text: &str, data: &mut WriteFileData| {
        assert_eq!(data.build_info.as_ref().unwrap().downcast_ref::<BuildInfo>().unwrap().marshal(), text, "custom writer must receive the serialized build info");
        writes.fetch_add(1, Ordering::Relaxed);
        std::fs::write(path, text).map_err(|err| err.to_string())
    };
    let emit = |program: P<Program>| program.get().emit(&Context::default(), EmitOptions { write_file: Some(&writer), ..Default::default() });
    emit(unchanged);
    let serialized = std::fs::read_to_string(&build_info_path).unwrap();
    let modified = std::fs::metadata(&build_info_path).unwrap().modified().unwrap();
    for _ in 0..2 {
        let loaded = create(None, true);
        assert_eq!(state(loaded, "e.ts"), SemanticDiagnosticsState::Reused, "loaded cache must reuse the interface diagnostic entry");
        assert_eq!(sites(loaded), expected);
        emit(loaded);
    }
    assert_eq!(writes.load(Ordering::Relaxed), 1, "an unchanged loaded cache must not emit");
    assert_eq!(std::fs::read_to_string(&build_info_path).unwrap(), serialized);
    assert_eq!(std::fs::metadata(&build_info_path).unwrap().modified().unwrap(), modified);
    for signature in ["", "go4-v2:0123456789abcdef0123456789abcdef", "go4-v1:bad"] {
        let mut legacy = BuildInfo::unmarshal(&serialized).unwrap();
        legacy.tsrs_canonical_check_signature = signature.to_string();
        std::fs::write(&build_info_path, legacy.marshal()).unwrap();
        let migrating = create(None, true);
        assert_eq!(state(migrating, "e.ts"), SemanticDiagnosticsState::NotCached, "legacy or invalid proof must refresh interfaces");
        assert_eq!(state(migrating, "f.ts"), SemanticDiagnosticsState::Reused, "migration must preserve other caches");
        assert_eq!(sites(migrating), expected);
        let before = writes.load(Ordering::Relaxed);
        emit(migrating);
        assert_eq!(writes.load(Ordering::Relaxed), before + 1, "migration must emit through custom writers");
        assert_eq!(std::fs::read_to_string(&build_info_path).unwrap(), serialized, "migration must upgrade once");
        let upgraded = create(None, true);
        assert_eq!(state(upgraded, "e.ts"), SemanticDiagnosticsState::Reused);
        assert_eq!(sites(upgraded), expected);
        emit(upgraded);
        assert_eq!(writes.load(Ordering::Relaxed), before + 1);
    }
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

    let api_expected = sites(create_with_policy(None, false, false));
    assert_eq!(api_expected.iter().filter(|diagnostic| diagnostic.0 == 2320).count(), 2);
    let api = create_with_policy(Some(unchanged), false, false);
    assert_eq!(state(api, "e.ts"), SemanticDiagnosticsState::NotCached, "API must invalidate canonical cached absence in memory");
    assert_eq!(state(api, "f.ts"), SemanticDiagnosticsState::Reused);
    assert_eq!(sites(api), api_expected);
    emit(api);
    let api_serialized = std::fs::read_to_string(&build_info_path).unwrap();
    assert!(!api_serialized.contains("tsrsCanonicalCheckSignature"));
    let api_writes = writes.load(Ordering::Relaxed);
    for _ in 0..2 {
        let loaded = create_with_policy(None, true, false);
        assert_eq!(state(loaded, "e.ts"), SemanticDiagnosticsState::Reused);
        assert_eq!(sites(loaded), api_expected);
        emit(loaded);
    }
    assert_eq!(writes.load(Ordering::Relaxed), api_writes, "unchanged API caches must not emit");
    let canonical = create_with_policy(Some(api), false, true);
    assert_eq!(state(canonical, "e.ts"), SemanticDiagnosticsState::NotCached, "canonical policy must invalidate API cached errors in memory");
    assert_eq!(state(canonical, "f.ts"), SemanticDiagnosticsState::Reused);
    assert_eq!(sites(canonical), expected);
    let loaded = create(None, true);
    assert_eq!(state(loaded, "e.ts"), SemanticDiagnosticsState::NotCached, "CLI must refresh an API-shaped disk cache");
    assert_eq!(state(loaded, "f.ts"), SemanticDiagnosticsState::Reused);
    assert_eq!(sites(loaded), expected);
    emit(loaded);
    assert_eq!(std::fs::read_to_string(&build_info_path).unwrap(), serialized);

    let key = "\"tsrsCanonicalCheckSignature\":";
    let start = serialized.find(key).unwrap() + key.len();
    let end = start + serialized[start..].find(',').unwrap();
    for value in [
        "\"go4-v1:0123456789abcdef0123456789abcdef\"",
        "\"\"",
        "\"go4-v1:bad\"",
        "\"go4-v2:0123456789abcdef0123456789abcdef\"",
        "null",
        "123",
        "false",
        "{}",
        "[]",
    ] {
        std::fs::write(&build_info_path, format!("{}{value}{}", &serialized[..start], &serialized[end..])).unwrap();
        let loaded = create_with_policy(None, true, false);
        assert_eq!(state(loaded, "e.ts"), SemanticDiagnosticsState::NotCached, "API must reject any present CLI tag: {value}");
        assert_eq!(state(loaded, "f.ts"), SemanticDiagnosticsState::NotCached);
        assert_eq!(sites(loaded), api_expected);
        let before = writes.load(Ordering::Relaxed);
        emit(loaded);
        assert_eq!(writes.load(Ordering::Relaxed), before + 1);
        assert_eq!(std::fs::read_to_string(&build_info_path).unwrap(), api_serialized);
        let warm = create_with_policy(None, true, false);
        assert_eq!(state(warm, "e.ts"), SemanticDiagnosticsState::Reused);
        assert_eq!(sites(warm), api_expected);
        emit(warm);
        assert_eq!(writes.load(Ordering::Relaxed), before + 1);
    }

    std::fs::write(&build_info_path, &serialized).unwrap();
    tsrs_core::compat::set_go_compatible_history(true);
    let go = create(None, true);
    assert_eq!(state(go, "e.ts"), SemanticDiagnosticsState::NotCached, "Go history must reject a tagged default cache");
    let go_expected = sites(build(None));
    assert_eq!(sites(go), go_expected);
    emit(go);
    let go_serialized = std::fs::read_to_string(&build_info_path).unwrap();
    assert!(!go_serialized.contains("tsrsCanonicalCheckSignature"));
    let go_modified = std::fs::metadata(&build_info_path).unwrap().modified().unwrap();
    let go_writes = writes.load(Ordering::Relaxed);
    for _ in 0..2 {
        let loaded = create(None, true);
        assert_eq!(state(loaded, "e.ts"), SemanticDiagnosticsState::Reused, "native Go interface caches must remain reusable");
        assert_eq!(state(loaded, "f.ts"), SemanticDiagnosticsState::Reused);
        assert_eq!(sites(loaded), go_expected);
        emit(loaded);
    }
    assert_eq!(writes.load(Ordering::Relaxed), go_writes);
    assert_eq!(std::fs::read_to_string(&build_info_path).unwrap(), go_serialized);
    assert_eq!(std::fs::metadata(&build_info_path).unwrap().modified().unwrap(), go_modified);
    tsrs_core::compat::set_go_compatible_history(false);
    let default = create(None, true);
    assert_eq!(state(default, "e.ts"), SemanticDiagnosticsState::NotCached, "default history must refresh an untagged Go cache");
    assert_eq!(state(default, "f.ts"), SemanticDiagnosticsState::Reused);
    assert_eq!(sites(default), expected);
    emit(default);
    assert_eq!(std::fs::read_to_string(&build_info_path).unwrap(), serialized);
    let restored = create(None, true);
    assert_eq!(state(restored, "e.ts"), SemanticDiagnosticsState::Reused, "restored default history must reuse the interface cache");
    assert_eq!(sites(restored), expected);
    emit(restored);
    assert_eq!(writes.load(Ordering::Relaxed), go_writes + 1);

    // An in-memory edit (a watch cycle) refreshes interface caches only when the changed file declares an interface,
    // before or after the edit: only that can move a merged interface's report site.
    let f = dir.join("f.ts");
    let f_text = std::fs::read_to_string(&f).unwrap();
    let mut previous = build(None);
    sites(previous);
    // The fixture's padding comment sets each file's size, which the virtual checker slots depend on: a comment
    // edit that keeps the size changes neither the slots nor any interface.
    let comment_edit = |from: &str| f_text.replacen("xx", from, 1);
    for (text, refreshed, what) in [
        (comment_edit("yy"), false, "a comment edit in a file without interfaces"),
        (format!("{f_text}\ndeclare global {{ interface M {{ other?: string }} }}\n"), true, "adding a declaration of the merged interface"),
        (comment_edit("yy"), true, "removing it again"),
        (comment_edit("zz"), false, "another comment edit"),
    ] {
        std::fs::write(&f, &text).unwrap();
        let edited = build(Some(previous));
        let expected_state = if refreshed { SemanticDiagnosticsState::NotCached } else { SemanticDiagnosticsState::Reused };
        assert_eq!(state(edited, "e.ts"), expected_state, "{what}");
        assert_eq!(sites(edited), sites(build(None)), "{what}");
        previous = edited;
    }
    std::fs::write(&f, f_text).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}
