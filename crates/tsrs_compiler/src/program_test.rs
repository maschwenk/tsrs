// program_test.go (the parts about program reuse and pluggable checker pools; tracing is not ported).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tsrs_core::tspath::Path;
use tsrs_core::P;
use tsrs_tsoptions::{self as tsoptions, ParseConfigHost};
use tsrs_vfs::{vfstest, FS};

use crate::checkerpool::checkerPool;
use crate::{new_compiler_host, new_program, CheckerHandle, PooledChecker, CheckerPool, CompilerHost, Context, CreateCheckerPool, Program, ProgramOptions};

struct parseConfigHost {
    fs: Arc<dyn FS>,
}

impl ParseConfigHost for parseConfigHost {
    fn fs(&self) -> &dyn FS {
        &*self.fs
    }
    fn get_current_directory(&self) -> &str {
        "/"
    }
}

fn host_for(files: &[(&str, &str)]) -> Arc<dyn CompilerHost> {
    let fs: Arc<dyn FS> = Arc::new(vfstest::from_map(files.iter().map(|&(k, v)| (k, v)), true));
    new_compiler_host("/", fs, "", None, None)
}

fn config_for(files: &[(&str, &str)]) -> P<tsoptions::ParsedCommandLine> {
    let fs: Arc<dyn FS> = Arc::new(vfstest::from_map(files.iter().map(|&(k, v)| (k, v)), true));
    let host: &'static parseConfigHost = Box::leak(Box::new(parseConfigHost { fs }));
    let (config, diagnostics) = tsoptions::get_parsed_command_line_of_config_file("/src/tsconfig.json", None, None, host, None);
    assert!(diagnostics.is_empty());
    P::new(config.unwrap())
}

// A pool outside the built-in one: hands out checkers through `CheckerHandle::from_raw` and takes them back on
// release, like the project system's pool.
struct testPool {
    program: &'static Program,
    idle: Arc<Mutex<Vec<PooledChecker>>>,
    acquired: Arc<AtomicUsize>,
}

impl CheckerPool for testPool {
    fn get_checker(&self, _ctx: &Context, _file: Option<P<tsrs_ast::SourceFile>>) -> CheckerHandle {
        self.acquired.fetch_add(1, Ordering::Relaxed);
        let mut checker = self.idle.lock().unwrap().pop().unwrap_or_else(|| PooledChecker::new(tsrs_checker::new_checker(self.program)));
        let ptr = checker.as_non_null();
        let idle = self.idle.clone();
        // SAFETY: the checker is out of the idle list (held) until the release function puts it back.
        unsafe { CheckerHandle::from_raw(ptr, move || idle.lock().unwrap().push(checker)) }
    }
}

// program_test.go:95 TestProgramHostsAndFactories
#[test]
fn program_hosts_and_factories() {
    let index = "/// <reference types=\"dep\" />\nimport { value } from \"./dep.js\"; export const result = value;";
    let files = [
        ("/src/tsconfig.json", r#"{"compilerOptions":{"noLib":true,"module":"nodenext"},"files":["index.ts"]}"#),
        ("/src/index.ts", index),
        ("/src/dep.ts", "export const value = 1;"),
        ("/src/node_modules/@types/dep/index.d.ts", "export {};"),
    ];
    let config = config_for(&files);
    let pools = Arc::new(AtomicUsize::new(0));
    let resolvers = Arc::new(AtomicUsize::new(0));
    let mut opts = ProgramOptions::new(config, host_for(&files));
    let pools_c = pools.clone();
    opts.create_checker_pool = Some(Arc::new(move |p: &'static Program| -> Box<dyn CheckerPool> {
        pools_c.fetch_add(1, Ordering::Relaxed);
        Box::new(checkerPool::new(p))
    }));
    let resolvers_c = resolvers.clone();
    opts.create_module_resolver = Some(Arc::new(move |options: tsrs_module::ResolverOptions| -> Box<dyn tsrs_module::Resolver> {
        resolvers_c.fetch_add(1, Ordering::Relaxed);
        Box::new(tsrs_module::new_resolver(options))
    }));
    let p = new_program(opts);
    assert_eq!(pools.load(Ordering::Relaxed), 1);
    assert_eq!(resolvers.load(Ordering::Relaxed), 1);
    let old_file = p.get_source_file("/src/index.ts").unwrap();
    let resolved = p.get_resolved_module_from_module_specifier(old_file, old_file.imports()[0]).unwrap();
    assert!(resolved.is_resolved());
    let resolved_type_ref = p
        .get_resolved_type_reference_directive_from_type_reference_directive(old_file.type_reference_directives.get()[0], old_file)
        .unwrap();
    assert!(resolved_type_ref.is_resolved());

    let new_index = format!("\n{index}");
    let new_files = [files[0], ("/src/index.ts", new_index.as_str()), files[2], files[3], ("/probe/package.json", r#"{"name":"new-host"}"#)];
    let new_host = host_for(&new_files);
    let pools_c = pools.clone();
    let new_host_c = new_host.clone();
    let create: CreateCheckerPool = Arc::new(move |p: &'static Program| -> Box<dyn CheckerPool> {
        pools_c.fetch_add(1, Ordering::Relaxed);
        assert!(Arc::ptr_eq(p.host(), &new_host_c));
        assert_eq!(p.get_source_file("/src/index.ts").unwrap().text(), format!("\n{index}"));
        Box::new(checkerPool::new(p))
    });
    let (cloned, changed, reused) = p.reuse_program(
        &Path::from("/src/index.ts"),
        new_host.clone(),
        Some(create),
        Some(Arc::new(|_| panic!("cloning must reuse resolution data without invoking construction callbacks"))),
    );
    assert!(reused);
    let cloned = cloned.unwrap();
    let changed = changed.unwrap();
    assert_eq!(Some(changed), cloned.get_source_file("/src/index.ts"));
    assert_eq!(Some(old_file), p.get_source_file("/src/index.ts"));
    assert_eq!(cloned.get_resolved_module_from_module_specifier(changed, changed.imports()[0]), Some(resolved));
    assert_eq!(
        cloned.get_resolved_type_reference_directive_from_type_reference_directive(changed.type_reference_directives.get()[0], changed),
        Some(resolved_type_ref)
    );
    assert!(cloned.resolution_data != p.resolution_data);
    assert!(cloned.get_package_json_info("/probe/package.json").is_some());
    assert!(p.get_package_json_info("/probe/package.json").is_none(), "new lazy lookups must not populate the old generation's cache");
    assert_eq!(pools.load(Ordering::Relaxed), 2);
    assert_eq!(resolvers.load(Ordering::Relaxed), 1);

    let (defaults, _, reused) = cloned.reuse_program(&Path::from("/src/index.ts"), new_host.clone(), None, None);
    assert!(reused);
    assert!(defaults.unwrap().compiler_checker_pool().is_some());
    assert_eq!(pools.load(Ordering::Relaxed), 2);

    let rebuild_files = [files[0], ("/src/index.ts", "import \"./other.js\";"), files[2], files[3], ("/src/other.ts", "export {};")];
    let rebuild_host = host_for(&rebuild_files);
    let (rebuilt, _, reused) = p.update_program(&Path::from("/src/index.ts"), rebuild_host.clone(), None, None);
    assert!(!reused);
    assert!(rebuilt.compiler_checker_pool().is_some());
    assert!(Arc::ptr_eq(rebuilt.host(), &rebuild_host));
    assert!(rebuilt.get_source_file("/src/other.ts").is_some());
    assert_eq!(pools.load(Ordering::Relaxed), 2);
    assert_eq!(resolvers.load(Ordering::Relaxed), 1);
}

// tsrs-only: the diagnostics entry points give the same results through an external pool (Go's per-file
// `GetChecker` path) as through the built-in grouped pool, and every acquired checker is released.
#[test]
fn external_checker_pool_diagnostics() {
    let files = [
        ("/src/tsconfig.json", r#"{"compilerOptions":{"noLib":true,"strict":true},"files":["a.ts","b.ts"]}"#),
        ("/src/a.ts", "export const a: number = \"x\";\nexport function f(x) { return x; }"),
        ("/src/b.ts", "import { a } from \"./a\";\nconst b: string = a;\n// @ts-expect-error\nconst ok = 1;"),
    ];
    let config = config_for(&files);
    let builtin = new_program(ProgramOptions::new(config, host_for(&files)));
    let idle = Arc::new(Mutex::new(Vec::new()));
    let acquired = Arc::new(AtomicUsize::new(0));
    let mut opts = ProgramOptions::new(config, host_for(&files));
    let (idle_c, acquired_c) = (idle.clone(), acquired.clone());
    opts.create_checker_pool = Some(Arc::new(move |program: &'static Program| -> Box<dyn CheckerPool> {
        Box::new(testPool { program, idle: idle_c.clone(), acquired: acquired_c.clone() })
    }));
    let external = new_program(opts);
    assert!(external.compiler_checker_pool().is_none());

    let ctx = Context::default();
    let messages = |program: &'static Program| -> Vec<String> {
        program
            .get_semantic_diagnostics(&ctx, None)
            .iter()
            .chain(program.get_suggestion_diagnostics(&ctx, None).iter())
            .map(|d| format!("{}:{}:{}", d.file().unwrap().file_name(), d.pos(), d.code()))
            .collect()
    };
    let expected = messages(builtin);
    assert!(expected.len() >= 4, "{expected:?}");
    assert_eq!(messages(external), expected);
    let file = external.get_source_file("/src/b.ts").unwrap();
    let single: Vec<i32> = external.get_semantic_diagnostics(&ctx, Some(file)).iter().map(|d| d.code()).collect();
    assert_eq!(single, vec![2322, 2578]);
    assert!(external.get_global_diagnostics(&ctx).is_empty());
    {
        let mut c = external.get_type_checker_for_file(&ctx, file);
        assert!(!c.was_canceled());
    }
    assert_eq!(acquired.load(Ordering::Relaxed), 6);
    assert!(!idle.lock().unwrap().is_empty());
}
