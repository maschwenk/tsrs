use super::*;

#[test]
fn full_compilation_replays_after_project_is_deleted() {
    std::thread::Builder::new().stack_size(512 << 20).spawn(|| {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().canonicalize().unwrap().to_str().unwrap().to_string();
        let root = Path::new(&cwd);
        std::fs::create_dir_all(root.join("node_modules/dep")).unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(root.join("base.json"), r#"{"compilerOptions":{"strict":true,"target":"es2020","module":"esnext","moduleResolution":"bundler"}}"#).unwrap();
        std::fs::write(root.join("tsconfig.json"), r#"{"extends":"./base.json","include":["src/**/*.ts"]}"#).unwrap();
        std::fs::write(root.join("node_modules/dep/package.json"), r#"{"name":"dep","types":"index.d.ts"}"#).unwrap();
        std::fs::write(root.join("node_modules/dep/index.d.ts"), "export declare const value: number;").unwrap();
        std::fs::write(root.join("src/a.ts"), "import { value } from 'dep';\nconst x: string = value;\n").unwrap();

        let config = format!("{cwd}/tsconfig.json");
        let project = PreparedProject::capture(cwd, &config, bundled::lib_path(), Arc::new(bundled::wrap_fs(osvfs::fs())));
        assert!(project.expected.output.contains("error TS2322:"), "{}", project.expected.output);
        assert_eq!(project.expected.output.lines().count(), 1, "{}", project.expected.output);
        dir.close().unwrap();

        // Every call constructs new config, module resolution, parser, binder, and checker state.
        for _ in 0..2 {
            let (sys, args) = project.invocation();
            project.verify(&sys.compile(args));
        }
    }).unwrap().join().unwrap();
}

#[test]
#[should_panic(expected = "missing benchmark config")]
fn missing_project_does_not_produce_a_successful_benchmark() {
    PreparedProject::capture(
        "/".to_string(),
        "/missing/tsconfig.json",
        bundled::lib_path(),
        Arc::new(tsrs_vfs::vfstest::from_map(
            std::iter::empty::<(&str, &str)>(),
            true,
        )),
    );
}

#[test]
#[should_panic(expected = "unknown TSRS_BENCH_PROJECT")]
fn unknown_project_is_rejected() {
    project_config("unknown");
}
