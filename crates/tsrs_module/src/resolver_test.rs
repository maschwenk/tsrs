use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{mpsc, Condvar, Mutex};
use std::time::{Duration, SystemTime};

use tsrs_core::collections::{new_ordered_map_with_size_hint, OrderedMapExt};
use tsrs_core::{CompilerOptions, ModuleKind, ModuleResolutionKind, ScriptTarget, P};
use tsrs_vfs::{vfstest, Entries, FileInfo, FS};

use crate::packagejson::InfoCacheEntryExt;
use crate::*;

struct ResolutionHostStub {
    fs: Box<dyn FS>,
    cwd: String,
}

impl ResolutionHost for ResolutionHostStub {
    fn fs(&self) -> &dyn FS {
        &*self.fs
    }
    fn get_current_directory(&self) -> &str {
        &self.cwd
    }
}

fn host(fs: impl FS + 'static, cwd: &str) -> &'static ResolutionHostStub {
    Box::leak(Box::new(ResolutionHostStub { fs: Box::new(fs), cwd: cwd.to_string() }))
}

fn bundler_options() -> P<CompilerOptions> {
    P::new(CompilerOptions {
        module_resolution: ModuleResolutionKind::Bundler,
        module: ModuleKind::ESNext,
        target: ScriptTarget::ESNext,
        ..Default::default()
    })
}

// Regression test for https://github.com/microsoft/TypeScript/tsc/issues/3526.
//
// Resolving a node_modules import with a trailing slash (e.g. `pkg/`) must
// produce the same result as without one.
#[test]
fn test_resolve_module_name_trailing_slash() {
    let fs = vfstest::from_map(
        [
            ("/repo/node_modules/pkg/package.json", r#"{"name":"pkg","main":"main.js","types":"main.d.ts"}"#),
            ("/repo/node_modules/pkg/main.d.ts", "export const x: number;"),
            ("/repo/node_modules/pkg/main.js", "exports.x = 1;"),
            ("/repo/src/file.ts", ""),
        ],
        true,
    );
    let resolver = new_resolver(ResolverOptions::new(host(fs, "/repo"), bundler_options()));

    for name in ["pkg", "pkg/"] {
        let (r, _) = resolver.resolve_module_name(name, "/repo/src/file.ts", ModuleKind::ESNext, None).unwrap();
        assert!(r.is_resolved(), "{name:?} failed to resolve");
    }
}

#[test]
fn test_resolution_data_caches() {
    let old_host = host(
        vfstest::from_map(
            [("/src/node_modules/pkg/package.json", r#"{"name":"pkg","types":"index.d.ts"}"#), ("/src/node_modules/pkg/index.d.ts", "export const value: number;")],
            true,
        ),
        "/",
    );
    let new_host = host(
        vfstest::from_map(
            [
                ("/src/node_modules/pkg/package.json", r#"{"name":"pkg","types":"index.d.ts"}"#),
                ("/src/node_modules/pkg/index.d.ts", "export const value: number;"),
                ("/new/package.json", r#"{"name":"new"}"#),
                ("/missing-first/package.json", r#"{"name":"missing-first"}"#),
            ],
            true,
        ),
        "/",
    );
    let resolver = new_resolver(ResolverOptions::new(old_host, P::new(CompilerOptions { module: ModuleKind::NodeNext, ..Default::default() })));
    let (resolved, _) = resolver.resolve_module_name("pkg", "/src/index.ts", ModuleKind::CommonJS, None).unwrap();
    assert!(resolved.is_resolved());
    let (cached, _) = resolver.resolve_module_name("pkg", "/src/index.ts", ModuleKind::CommonJS, None).unwrap();
    assert_eq!(cached, resolved);

    resolver.resolve_type_reference_directive("missing", "/src/index.ts", ModuleKind::CommonJS, None);
    let mut paths = new_ordered_map_with_size_hint(1);
    paths.set("alias/*".to_string(), vec!["./*".to_string()]);
    resolver.get_parsed_patterns_for_paths(&CompilerOptions { paths: Some(paths), ..Default::default() });
    assert_eq!(resolver.module_resolution_cache.size(), 1);
    assert_eq!(resolver.type_ref_directive_resolution_cache.size(), 1);
    assert_eq!(resolver.parsed_patterns_for_paths.size(), 1);

    let data = resolver.get_resolution_data();
    let rebound = data.get().new_resolver(old_host);
    assert_eq!(rebound.module_resolution_cache.size(), 0);
    assert_eq!(rebound.type_ref_directive_resolution_cache.size(), 0);
    assert_eq!(rebound.parsed_patterns_for_paths.size(), 0);

    let clone = data.clone_data().get().new_resolver(new_host);
    assert_eq!(clone.module_resolution_cache.size(), 0);
    assert_eq!(clone.type_ref_directive_resolution_cache.size(), 0);
    assert_eq!(clone.parsed_patterns_for_paths.size(), 0);
    assert_eq!(clone.get_package_scope_for_path("/src/node_modules/pkg"), resolver.get_package_scope_for_path("/src/node_modules/pkg"));
    assert!(clone.get_package_scope_for_path("/new").exists());
    assert!(!resolver.get_package_scope_for_path("/new").exists());
    assert!(!resolver.get_package_scope_for_path("/missing-first").exists());
    assert!(clone.get_package_scope_for_path("/missing-first").exists());
}

// A one-shot event, standing in for Go's `chan struct{}` that is closed once.
#[derive(Default)]
struct Gate {
    open: Mutex<bool>,
    cv: Condvar,
}

impl Gate {
    fn open(&self) {
        *self.open.lock().unwrap() = true;
        self.cv.notify_all();
    }

    fn wait(&self, description: &str) {
        let guard = self.open.lock().unwrap();
        let (guard, timeout) = self.cv.wait_timeout_while(guard, Duration::from_secs(10), |open| !*open).unwrap();
        if timeout.timed_out() && !*guard {
            panic!("timed out waiting for {description}");
        }
    }
}

// Delegates every method but `file_exists`/`read_file` to `fs`.
macro_rules! delegate_fs {
    () => {
        fn use_case_sensitive_file_names(&self) -> bool {
            self.fs.use_case_sensitive_file_names()
        }
        fn write_file(&self, path: &str, data: &str) -> Result<(), String> {
            self.fs.write_file(path, data)
        }
        fn append_file(&self, path: &str, data: &str) -> Result<(), String> {
            self.fs.append_file(path, data)
        }
        fn remove(&self, path: &str) -> Result<(), String> {
            self.fs.remove(path)
        }
        fn chtimes(&self, path: &str, a_time: SystemTime, m_time: SystemTime) -> Result<(), String> {
            self.fs.chtimes(path, a_time, m_time)
        }
        fn directory_exists(&self, path: &str) -> bool {
            self.fs.directory_exists(path)
        }
        fn get_accessible_entries(&self, path: &str) -> Entries {
            self.fs.get_accessible_entries(path)
        }
        fn stat(&self, path: &str) -> Option<FileInfo> {
            self.fs.stat(path)
        }
        fn realpath(&self, path: &str) -> String {
            self.fs.realpath(path)
        }
    };
}

// blockingFS wraps a vfs.FS and forces FileExists calls for `targetPath` to
// block on `gate` until released. Each caller sends on `arrived` when it
// reaches the gate.
struct BlockingFS<T: FS> {
    fs: T,
    target_path: String,
    gate: Gate,
    arrived: Mutex<mpsc::Sender<()>>,
}

impl<T: FS> FS for BlockingFS<T> {
    delegate_fs!();
    fn file_exists(&self, path: &str) -> bool {
        if path == self.target_path {
            self.arrived.lock().unwrap().send(()).unwrap();
            self.gate.wait("gate");
        }
        self.fs.file_exists(path)
    }
    fn read_file(&self, path: &str) -> Option<String> {
        self.fs.read_file(path)
    }
}

// flipFileExistsFS returns false for the first FileExists call to `targetPath`,
// then true for the second. ReadFile for the target path signals arrival then
// blocks, so the "file doesn't exist" Set completes before the "file exists" Set.
struct FlipFileExistsFS<T: FS> {
    fs: T,
    target_path: String,
    call_count: AtomicI32,
    first_arrived: Gate,
    second_arrived: Gate,
    first_gate: Gate,
    second_gate: Gate,
    read_arrived: Gate,
    read_gate: Gate,
}

impl<T: FS> FlipFileExistsFS<T> {
    fn new(fs: T, target_path: &str) -> FlipFileExistsFS<T> {
        FlipFileExistsFS {
            fs,
            target_path: target_path.to_string(),
            call_count: AtomicI32::new(0),
            first_arrived: Gate::default(),
            second_arrived: Gate::default(),
            first_gate: Gate::default(),
            second_gate: Gate::default(),
            read_arrived: Gate::default(),
            read_gate: Gate::default(),
        }
    }
}

impl<T: FS> FS for FlipFileExistsFS<T> {
    delegate_fs!();
    fn file_exists(&self, path: &str) -> bool {
        if path == self.target_path {
            let n = self.call_count.fetch_add(1, Ordering::SeqCst) + 1;
            if n == 1 {
                self.first_arrived.open();
                self.first_gate.wait("first gate");
                return false; // first caller: simulate "file not yet visible"
            }
            if n == 2 {
                self.second_arrived.open();
                self.second_gate.wait("second gate");
                return self.fs.file_exists(path); // second caller: file is visible
            }
        }
        self.fs.file_exists(path)
    }
    fn read_file(&self, path: &str) -> Option<String> {
        if path == self.target_path {
            self.read_arrived.open();
            self.read_gate.wait("read gate");
        }
        self.fs.read_file(path)
    }
}

// Regression test for https://github.com/microsoft/TypeScript/tsc/issues/3526.
//
// Two threads resolve the same package via specifiers that differ only by
// a trailing slash (`pkg` and `pkg/`), both held at the `FileExists` check for
// `package.json` after a cache miss; the loser of the insert race must still
// see its own `PackageDirectory`.
#[test]
fn test_resolve_module_name_trailing_slash_race() {
    const PKG_JSON_PATH: &str = "/repo/node_modules/pkg/package.json";
    let (arrived_tx, arrived_rx) = mpsc::channel();
    let fs: &'static BlockingFS<_> = Box::leak(Box::new(BlockingFS {
        fs: vfstest::from_map(
            [
                (PKG_JSON_PATH, r#"{"name":"pkg","types":"./typings/index.d.ts"}"#),
                ("/repo/node_modules/pkg/typings/index.d.ts", "export const x: number;"),
                ("/repo/src/a/file.ts", ""),
                ("/repo/src/b/file.ts", ""),
            ],
            true,
        ),
        target_path: PKG_JSON_PATH.to_string(),
        gate: Gate::default(),
        arrived: Mutex::new(arrived_tx),
    }));
    let resolver = new_resolver(ResolverOptions::new(host(fs, "/repo"), bundler_options()));

    std::thread::scope(|s| {
        let handles: Vec<_> = ["pkg", "pkg/"]
            .into_iter()
            .map(|name| {
                let containing_file = if name.ends_with('/') { "/repo/src/b/file.ts" } else { "/repo/src/a/file.ts" };
                let resolver = &resolver;
                s.spawn(move || (name, resolver.resolve_module_name(name, containing_file, ModuleKind::ESNext, None).unwrap().0.is_resolved()))
            })
            .collect();

        // Wait for both threads to reach the FileExists gate, guaranteeing
        // both have observed a package.json info-cache miss.
        arrived_rx.recv_timeout(Duration::from_secs(10)).expect("first FileExists gate arrival");
        arrived_rx.recv_timeout(Duration::from_secs(10)).expect("second FileExists gate arrival");
        fs.gate.open();

        for handle in handles {
            let (name, resolved) = handle.join().unwrap();
            assert!(resolved, "{name:?} failed to resolve");
        }
    });
}

// Regression test for https://github.com/microsoft/TypeScript/tsc/issues/1290.
//
// The second lookup of the root package.json receives the first lookup's
// nil-Contents cache entry; the typesVersions lookup must not dereference it.
#[test]
fn test_resolve_subpath_nil_contents_race() {
    const ROOT_PKG_JSON: &str = "/repo/node_modules/pkg/package.json";
    let fs: &'static FlipFileExistsFS<_> = Box::leak(Box::new(FlipFileExistsFS::new(
        vfstest::from_map(
            [
                (ROOT_PKG_JSON, r#"{"name":"pkg","version":"1.0.0"}"#),
                ("/repo/node_modules/pkg/sub/index.d.ts", "export declare const sub: number;"),
                ("/repo/node_modules/pkg/sub/index.js", "exports.sub = 1;"),
                ("/repo/src/a/file.ts", ""),
                ("/repo/src/b/file.ts", ""),
            ],
            true,
        ),
        ROOT_PKG_JSON,
    )));
    let resolver = new_resolver(ResolverOptions::new(host(fs, "/repo"), bundler_options()));

    std::thread::scope(|s| {
        let handles: Vec<_> = ["/repo/src/a/file.ts", "/repo/src/b/file.ts"]
            .into_iter()
            .map(|containing_file| {
                let resolver = &resolver;
                s.spawn(move || (containing_file, resolver.resolve_module_name("pkg/sub", containing_file, ModuleKind::ESNext, None).unwrap().0.is_resolved()))
            })
            .collect();

        fs.first_arrived.wait("first root package.json FileExists arrival");
        fs.second_arrived.wait("second root package.json FileExists arrival");
        fs.first_gate.open();
        fs.second_gate.open();
        fs.read_arrived.wait("root package.json ReadFile arrival");
        fs.read_gate.open();

        for handle in handles {
            let (containing_file, resolved) = handle.join().expect("resolver panicked due to nil Contents dereference");
            assert!(resolved, "{containing_file:?} failed to resolve pkg/sub");
        }
    });
}

#[test]
fn test_parse_node_module_from_path() {
    let tests = [
        ("file in package", "/a/node_modules/b/lib/index.d.ts", false, "/a/node_modules/b"),
        ("file in scoped package", "/a/node_modules/@scope/b/lib/index.d.ts", false, "/a/node_modules/@scope/b"),
        ("folder subpath", "/a/node_modules/b/lib/File", true, "/a/node_modules/b"),
        ("folder subpath scoped", "/a/node_modules/@scope/b/lib/File", true, "/a/node_modules/@scope/b"),
        ("package root folder", "/a/node_modules/b", true, "/a/node_modules/b"),
        ("scoped package root folder", "/a/node_modules/@scope/b", true, "/a/node_modules/@scope/b"),
        // A bare scope directory has no package name; must not panic (https://github.com/microsoft/TypeScript/tsc/issues/4373).
        ("scope-only folder", "/a/node_modules/@scope", true, "/a/node_modules/@scope"),
        ("types scope-only folder", "/a/node_modules/@types", true, "/a/node_modules/@types"),
        ("not in node_modules", "/a/src/index.ts", false, ""),
    ];
    for (name, path, is_folder, want) in tests {
        assert_eq!(parse_node_module_from_path(path, is_folder), want, "{name}");
    }
}

// Regression test for https://github.com/microsoft/TypeScript/tsc/issues/4478.
//
// The peer package.json lookup must not dereference a nil-Contents cache entry.
#[test]
fn test_resolve_peer_dependency_nil_contents_race() {
    const PEER_PKG_JSON: &str = "/repo/node_modules/peer/package.json";
    let fs: &'static FlipFileExistsFS<_> = Box::leak(Box::new(FlipFileExistsFS::new(
        vfstest::from_map(
            [
                ("/repo/node_modules/pkg/package.json", r#"{"name":"pkg","version":"1.0.0","types":"index.d.ts","peerDependencies":{"peer":"*"}}"#),
                ("/repo/node_modules/pkg/index.d.ts", "export declare const x: number;"),
                (PEER_PKG_JSON, r#"{"name":"peer","version":"2.0.0"}"#),
                ("/repo/src/a/file.ts", ""),
                ("/repo/src/b/file.ts", ""),
            ],
            true,
        ),
        PEER_PKG_JSON,
    )));
    let resolver = new_resolver(ResolverOptions::new(host(fs, "/repo"), bundler_options()));

    std::thread::scope(|s| {
        let (results_tx, results_rx) = mpsc::channel();
        for containing_file in ["/repo/src/a/file.ts", "/repo/src/b/file.ts"] {
            let resolver = &resolver;
            let results_tx = results_tx.clone();
            s.spawn(move || {
                let resolved = resolver.resolve_module_name("pkg", containing_file, ModuleKind::ESNext, None).unwrap().0.is_resolved();
                results_tx.send((containing_file, resolved)).unwrap();
            });
        }

        fs.first_arrived.wait("first peer package.json FileExists arrival");
        fs.second_arrived.wait("second peer package.json FileExists arrival");
        fs.first_gate.open();
        let first_result = results_rx.recv_timeout(Duration::from_secs(10)).expect("timed out waiting for first peer package.json lookup to finish");
        fs.second_gate.open();
        fs.read_arrived.wait("peer package.json ReadFile arrival");
        fs.read_gate.open();

        assert!(first_result.1, "{:?} failed to resolve pkg", first_result.0);
        let second_result = results_rx.recv_timeout(Duration::from_secs(10)).expect("resolver panicked due to nil Contents dereference");
        assert!(second_result.1, "{:?} failed to resolve pkg", second_result.0);
    });
}
