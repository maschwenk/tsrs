// lsp/lspwatcher/lspwatcher_test.go

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rustc_hash::FxHashMap;
use tsrs_core::tspath;
use tsrs_fswatch as fswatch;
use tsrs_lsproto as lsproto;
use tsrs_project::logging::{new_logger, Logger};
use tsrs_vfs::{bundled, osvfs, FS};

use crate::lspwatcher::*;

fn os_fs() -> Arc<dyn FS> {
    Arc::new(bundled::wrap_fs(osvfs::fs()))
}

fn stderr_logger() -> Arc<dyn Logger> {
    new_logger(Box::new(std::io::stderr()))
}

struct tempDir(String);

impl tempDir {
    fn new() -> tempDir {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("tsrs-lspwatcher-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        // Go's t.TempDir() path is not resolved either; the watcher works on the requested spelling.
        tempDir(dir.to_string_lossy().into_owned())
    }

    fn join(&self, rel: &str) -> String {
        format!("{}/{}", self.0, rel)
    }
}

impl Drop for tempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// lspwatcher_test.go:21
fn wait_for(cond: impl Fn() -> bool, msg: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if cond() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {}", msg);
}

fn fsw(pattern: &str, kind: Option<lsproto::WatchKind>) -> lsproto::FileSystemWatcher {
    lsproto::FileSystemWatcher { glob_pattern: lsproto::PatternOrRelativePattern { pattern: Some(pattern.to_string()), relative_pattern: None }, kind }
}

fn all_kinds() -> Option<lsproto::WatchKind> {
    Some(lsproto::WatchKind(lsproto::WatchKind::Create.0 | lsproto::WatchKind::Change.0 | lsproto::WatchKind::Delete.0))
}

type sink = Arc<Mutex<Vec<lsproto::FileEvent>>>;

fn recorder() -> (sink, OnChanges) {
    let got: sink = Arc::new(Mutex::new(Vec::new()));
    let g = got.clone();
    (got, Arc::new(move |changes: Vec<lsproto::FileEvent>| g.lock().unwrap().extend(changes)))
}

fn update(path: &str) -> fswatch::Event {
    fswatch::Event::new(fswatch::EventKind::Update, path)
}

fn delete(path: &str) -> fswatch::Event {
    fswatch::Event::new(fswatch::EventKind::Delete, path)
}

fn terminated_error(detail: &str) -> fswatch::Error {
    // Go: errors.Join(fswatch.ErrWatchTerminated, errors.New(detail)).
    fswatch::Error::wrap(format!("{}\n{}", fswatch::ErrWatchTerminated.0, detail), &fswatch::ErrWatchTerminated.into())
}

// lspwatcher_test.go:33
#[test]
fn watcher_create_change_delete() {
    let dir = tempDir::new();
    let (got, on_changes) = recorder();
    let w = new(os_fs(), on_changes, stderr_logger());

    let pattern = format!("{}/**/*", tspath::normalize_slashes(&dir.0));
    w.watch_files("test", &[fsw(&pattern, all_kinds())]).unwrap();

    std::thread::sleep(Duration::from_millis(200));

    let file = dir.join("a.ts");
    std::fs::write(&file, "export {}").unwrap();
    wait_for(|| got.lock().unwrap().iter().any(|e| e.type_ == lsproto::FileChangeType::Changed), "update event");

    std::fs::remove_file(&file).unwrap();
    wait_for(|| got.lock().unwrap().iter().any(|e| e.type_ == lsproto::FileChangeType::Deleted), "delete event");

    w.unwatch_files("test").unwrap();
    w.close();
}

// lspwatcher_test.go:100
#[test]
fn watcher_kind_filter() {
    let dir = tempDir::new();
    let dir_norm = tspath::normalize_slashes(&dir.0);
    let (got, on_changes) = recorder();
    let backend = fakeBackend::new();
    let w = new_with_backend(os_fs(), Box::new(backend.clone()), on_changes, stderr_logger());

    let pattern = format!("{}/**/*", dir_norm);
    w.watch_files("test", &[fsw(&pattern, Some(lsproto::WatchKind::Delete))]).unwrap();
    let x = format!("{}/x.ts", dir_norm);
    backend.emit_all(vec![update(&x), delete(&x)], None);

    wait_for(|| got.lock().unwrap().iter().any(|e| e.type_ == lsproto::FileChangeType::Deleted), "delete event");
    for e in got.lock().unwrap().iter() {
        assert_eq!(e.type_, lsproto::FileChangeType::Deleted, "unexpected non-delete event: {:?}", e.uri);
    }
    w.close();
}

// lspwatcher_test.go:150
#[test]
fn root_from_glob_cases() {
    for (pattern, want) in [("/abs/path/**/*", "/abs/path"), ("/abs/path/", "/abs/path"), ("/abs/path/?.ts", "/abs/path"), ("/abs/path/{a,b}/*", "/abs/path")] {
        assert_eq!(root_from_glob_for_test(pattern), want, "rootFromGlob({:?})", pattern);
    }
}

#[derive(Default)]
struct fakeBackendState {
    by_dir: FxHashMap<String, fswatch::WatchCallback>,
    closed: FxHashMap<String, i32>,
    opt_count: FxHashMap<String, usize>,
    fail_dirs: FxHashMap<String, fswatch::Error>,
}

#[derive(Clone)]
struct fakeBackend(Arc<Mutex<fakeBackendState>>);

struct fakeWatch(Box<dyn Fn() + Send + Sync>);

impl fswatch::Watch for fakeWatch {
    fn close(&self) -> Result<(), fswatch::Error> {
        (self.0)();
        Ok(())
    }
}

impl fakeBackend {
    fn new() -> fakeBackend {
        fakeBackend(Arc::new(Mutex::new(fakeBackendState::default())))
    }

    // watchedDirs returns the directories currently subscribed, for assertions.
    fn watched_dirs(&self) -> Vec<String> {
        self.0.lock().unwrap().by_dir.keys().cloned().collect()
    }

    fn is_watching(&self, dir: &str) -> bool {
        self.0.lock().unwrap().by_dir.contains_key(dir)
    }

    fn emit(&self, dir: &str, events: Vec<fswatch::Event>, err: Option<fswatch::Error>) {
        let cb = self.0.lock().unwrap().by_dir.get(dir).cloned();
        if let Some(cb) = cb {
            cb(events, err);
        }
    }

    fn emit_all(&self, events: Vec<fswatch::Event>, err: Option<fswatch::Error>) {
        let cbs: Vec<fswatch::WatchCallback> = self.0.lock().unwrap().by_dir.values().cloned().collect();
        for cb in cbs {
            cb(events.clone(), err.clone());
        }
    }
}

impl watcherBackend for fakeBackend {
    // lspwatcher_test.go:185
    fn watch_directory(&self, dir: &str, f: fswatch::WatchCallback, opts: Vec<fswatch::WatchOption>) -> Result<Box<dyn fswatch::Watch>, fswatch::Error> {
        let mut st = self.0.lock().unwrap();
        if let Some(err) = st.fail_dirs.get(dir) {
            return Err(err.clone());
        }
        st.by_dir.insert(dir.to_string(), f);
        st.opt_count.insert(dir.to_string(), opts.len());
        let state = self.0.clone();
        let dir = dir.to_string();
        Ok(Box::new(fakeWatch(Box::new(move || {
            let mut st = state.lock().unwrap();
            st.by_dir.remove(&dir);
            *st.closed.entry(dir.clone()).or_default() += 1;
        }))))
    }
}

// lspwatcher_test.go:245
#[test]
fn watcher_bookkeeping_and_overflow() {
    let dir = tempDir::new();
    let dir_norm = tspath::normalize_slashes(&dir.0);
    let pattern = format!("{}/**/*", dir_norm);
    let backend = fakeBackend::new();
    let (got, on_changes) = recorder();
    let w = new_with_backend(os_fs(), Box::new(backend.clone()), on_changes, stderr_logger());

    w.watch_files("id", &[fsw(&pattern, None)]).unwrap();
    assert!(w.watch_files("id", &[fsw(&pattern, None)]).is_err(), "expected duplicate-id error");

    backend.emit_all(vec![update(&format!("{}/a.ts", dir_norm))], Some(fswatch::ErrOverflow.into()));
    wait_for(|| !got.lock().unwrap().is_empty(), "events after overflow");

    assert!(w.unwatch_files("missing").is_err(), "expected unknown-id error");
    w.unwatch_files("id").unwrap();
    w.watch_files("id2", &[]).unwrap();
    w.close();
    assert!(w.watch_files("id3", &[]).is_err(), "expected closed error");
}

// lspwatcher_test.go:299
#[test]
fn watcher_non_recursive_glob_is_not_recursive() {
    let dir = tempDir::new();
    let dir_norm = tspath::normalize_slashes(&dir.0);
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    let sub_norm = tspath::normalize_slashes(&dir.join("sub"));

    let backend = fakeBackend::new();
    let w = new_with_backend(os_fs(), Box::new(backend.clone()), Arc::new(|_| {}), stderr_logger());

    let recursive = format!("{}/**/*", dir_norm);
    let non_recursive = format!("{}/*", sub_norm);
    w.watch_files("id", &[fsw(&recursive, None), fsw(&non_recursive, None)]).unwrap();

    let st = backend.0.lock().unwrap();
    assert_eq!(st.opt_count.get(&dir_norm).copied(), Some(1), "recursive glob: expected 1 watch option (WithRecursive)");
    assert_eq!(st.opt_count.get(&sub_norm).copied(), Some(0), "non-recursive glob: expected 0 watch options");
    drop(st);
    w.close();
}

// lspwatcher_test.go:333
#[test]
fn watcher_real_backend_missing_then_create() {
    let base = tempDir::new();
    let (got, on_changes) = recorder();
    let w = new(os_fs(), on_changes, stderr_logger());

    // Watch a directory that does not exist yet.
    let target = tspath::normalize_slashes(&base.join("pkg"));
    let pattern = format!("{}/*", target);
    w.watch_files("test", &[fsw(&pattern, all_kinds())]).unwrap();

    // Give the ancestor watch time to install.
    std::thread::sleep(Duration::from_millis(200));

    // Create the target directory and a file inside it. The real backend's
    // ancestor watch should fire, promote to the target, and
    // the file should ultimately surface.
    std::fs::create_dir_all(base.join("pkg")).unwrap();
    std::fs::write(base.join("pkg/index.ts"), "export {}").unwrap();

    wait_for(|| got.lock().unwrap().iter().any(|e| e.uri.0.ends_with("/pkg/index.ts")), "event for file created in a previously-missing directory");
    w.close();
}

// lspwatcher_test.go:393
#[test]
fn watcher_missing_directory_tracks_ancestor() {
    let backend = fakeBackend::new();
    let w = new_with_backend(os_fs(), Box::new(backend.clone()), Arc::new(|_| {}), stderr_logger());

    let base = tempDir::new();
    let base_norm = tspath::normalize_slashes(&base.0);
    let pattern = format!("{}/*", tspath::normalize_slashes(&base.join("pkg")));
    w.watch_files("id", &[fsw(&pattern, None)]).unwrap();

    // A missing target directory installs an ancestor watch on the nearest
    // existing ancestor (the base dir), not on the target.
    assert!(backend.is_watching(&base_norm), "expected ancestor watch on ancestor {:?}, watched: {:?}", base_norm, backend.watched_dirs());
    assert_eq!(backend.watched_dirs().len(), 1, "expected exactly one (ancestor) watch, got {:?}", backend.watched_dirs());

    w.unwatch_files("id").unwrap();
    assert!(backend.watched_dirs().is_empty(), "expected all watches closed after unwatch, got {:?}", backend.watched_dirs());
    w.close();
}

// lspwatcher_test.go:429
#[test]
fn watcher_missing_directory_promotes_on_create() {
    let backend = fakeBackend::new();
    let (got, on_changes) = recorder();
    let w = new_with_backend(os_fs(), Box::new(backend.clone()), on_changes, stderr_logger());

    let base = tempDir::new();
    let base_norm = tspath::normalize_slashes(&base.0);
    let target = tspath::normalize_slashes(&base.join("pkg"));
    let pattern = format!("{}/*", target);
    w.watch_files("id", &[fsw(&pattern, all_kinds())]).unwrap();

    // Create the target directory with a file, then notify the ancestor watch.
    std::fs::create_dir_all(base.join("pkg")).unwrap();
    std::fs::write(base.join("pkg/index.ts"), "export {}").unwrap();
    backend.emit(&base_norm, vec![update(&base.join("pkg"))], None);

    wait_for(|| backend.is_watching(&target), "promotion to target watch");

    // Synthetic creates must cover the target dir and its immediate child so
    // the session re-resolves files created before the watch was installed.
    wait_for(
        || {
            let got = got.lock().unwrap();
            let created = got.iter().filter(|e| e.type_ == lsproto::FileChangeType::Created);
            let uris: Vec<&str> = created.map(|e| e.uri.0.as_str()).collect();
            uris.iter().any(|s| s.ends_with("/pkg")) && uris.iter().any(|s| s.ends_with("/pkg/index.ts"))
        },
        "synthetic create events for target and child",
    );
    w.close();
}

// lspwatcher_test.go:493
#[test]
fn watcher_multi_level_descend() {
    let backend = fakeBackend::new();
    let w = new_with_backend(os_fs(), Box::new(backend.clone()), Arc::new(|_| {}), stderr_logger());

    let base = tempDir::new();
    let base_norm = tspath::normalize_slashes(&base.0);
    let pattern = format!("{}/*", tspath::normalize_slashes(&base.join("a/b/c")));
    w.watch_files("id", &[fsw(&pattern, None)]).unwrap();
    assert!(backend.is_watching(&base_norm), "expected initial ancestor watch on {:?}, got {:?}", base_norm, backend.watched_dirs());

    // Reveal one path component at a time; the ancestor watch should descend.
    let mkdir_and_path = |rel: &str| {
        let p = base.join(rel);
        std::fs::create_dir_all(&p).unwrap();
        tspath::normalize_slashes(&p)
    };

    let a_dir = mkdir_and_path("a");
    backend.emit(&base_norm, vec![update(&base.join("a"))], None);
    wait_for(|| backend.is_watching(&a_dir), "descend to a");

    let ab_dir = mkdir_and_path("a/b");
    backend.emit(&a_dir, vec![update(&base.join("a/b"))], None);
    wait_for(|| backend.is_watching(&ab_dir), "descend to a/b");

    let abc_dir = mkdir_and_path("a/b/c");
    backend.emit(&ab_dir, vec![update(&base.join("a/b/c"))], None);
    wait_for(|| backend.is_watching(&abc_dir), "promote to target a/b/c");
    w.close();
}

// lspwatcher_test.go:537
#[test]
fn watcher_atomic_tree_create_race() {
    let backend = fakeBackend::new();
    let w = new_with_backend(os_fs(), Box::new(backend.clone()), Arc::new(|_| {}), stderr_logger());

    let base = tempDir::new();
    let base_norm = tspath::normalize_slashes(&base.0);
    let target = tspath::normalize_slashes(&base.join("a/b/c"));
    let pattern = format!("{}/*", target);
    w.watch_files("id", &[fsw(&pattern, None)]).unwrap();

    // The whole tree appears at once (e.g. an extraction/symlink). A single
    // notification on the base ancestor watch must descend all the way and
    // promote to the target in one reconcile pass.
    std::fs::create_dir_all(base.join("a/b/c")).unwrap();
    backend.emit(&base_norm, vec![update(&base.join("a"))], None);

    wait_for(|| backend.is_watching(&target), "promote to target in one pass");
    w.close();
}

// lspwatcher_test.go:567
#[test]
fn watcher_synthetic_create_depth() {
    for recursive in [false, true] {
        let backend = fakeBackend::new();
        let (got, on_changes) = recorder();
        let w = new_with_backend(os_fs(), Box::new(backend.clone()), on_changes, stderr_logger());

        let base = tempDir::new();
        let base_norm = tspath::normalize_slashes(&base.0);
        let target = tspath::normalize_slashes(&base.join("pkg"));
        let pattern = if recursive { format!("{}/**/*", target) } else { format!("{}/*", target) };
        w.watch_files("id", &[fsw(&pattern, all_kinds())]).unwrap();

        // Materialize the target with a nested file under a subdirectory.
        std::fs::create_dir_all(base.join("pkg/sub")).unwrap();
        std::fs::write(base.join("pkg/top.ts"), "export {}").unwrap();
        std::fs::write(base.join("pkg/sub/deep.ts"), "export {}").unwrap();
        backend.emit(&base_norm, vec![update(&base.join("pkg"))], None);

        wait_for(|| backend.is_watching(&target), "promotion");

        let created = || -> Vec<String> { got.lock().unwrap().iter().filter(|e| e.type_ == lsproto::FileChangeType::Created).map(|e| e.uri.0.clone()).collect() };

        // Both modes must synthesize the immediate child.
        wait_for(|| created().iter().any(|s| s.ends_with("/pkg/top.ts")), "synthetic create for immediate child");

        // Only the recursive watch should synthesize the deep descendant.
        let saw_deep = || created().iter().any(|s| s.ends_with("/pkg/sub/deep.ts"));
        if recursive {
            let deadline = Instant::now() + Duration::from_secs(1);
            while Instant::now() < deadline && !saw_deep() {
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(saw_deep(), "recursive watch should synthesize deep descendant; got {:?}", created());
        } else {
            std::thread::sleep(Duration::from_millis(300)); // allow any erroneous deep event to arrive
            assert!(!saw_deep(), "non-recursive watch must not synthesize deep descendant; got {:?}", created());
        }
        w.close();
    }
}

// lspwatcher_test.go:667
#[test]
fn watcher_terminated_falls_back_and_recovers() {
    let backend = fakeBackend::new();
    let (got, on_changes) = recorder();
    let w = new_with_backend(os_fs(), Box::new(backend.clone()), on_changes, stderr_logger());

    let base = tempDir::new();
    let base_norm = tspath::normalize_slashes(&base.0);
    let target = tspath::normalize_slashes(&base.join("pkg"));
    std::fs::create_dir_all(base.join("pkg")).unwrap();
    let pattern = format!("{}/*", target);
    w.watch_files("id", &[fsw(&pattern, all_kinds())]).unwrap();
    assert!(backend.is_watching(&target), "expected target watch on {:?}, got {:?}", target, backend.watched_dirs());

    // Delete the directory and deliver ErrWatchTerminated together with the
    // directory's own delete event (as the real backends do).
    std::fs::remove_dir_all(base.join("pkg")).unwrap();
    backend.emit(&target, vec![delete(&base.join("pkg"))], Some(terminated_error("removed")));

    // The delete must be forwarded, and the watch must fall back to watching
    // the ancestor.
    wait_for(|| got.lock().unwrap().iter().any(|e| e.type_ == lsproto::FileChangeType::Deleted && e.uri.0.ends_with("/pkg")), "forwarded delete of terminated dir");
    wait_for(|| backend.is_watching(&base_norm) && !backend.is_watching(&target), "fallback to ancestor watch");

    // Recreate the directory; the ancestor watch must promote back to target.
    std::fs::create_dir_all(base.join("pkg")).unwrap();
    backend.emit(&base_norm, vec![update(&base.join("pkg"))], None);
    wait_for(|| backend.is_watching(&target), "recovery to target watch after recreation");
    w.close();
}

// lspwatcher_test.go:733
#[test]
fn watcher_genuine_failure_rolls_back_for_retry() {
    let backend = fakeBackend::new();
    let w = new_with_backend(os_fs(), Box::new(backend.clone()), Arc::new(|_| {}), stderr_logger());

    let dir = tempDir::new();
    let dir_norm = tspath::normalize_slashes(&dir.0);
    let pattern = format!("{}/*", dir_norm);

    // Inject a genuine backend failure for the existing directory.
    backend.0.lock().unwrap().fail_dirs.insert(dir_norm.clone(), fswatch::Error::new("too many open files"));

    assert!(w.watch_files("id", &[fsw(&pattern, None)]).is_err(), "expected error from genuine backend failure");

    // The id must have been rolled back so a retry can re-register cleanly
    // (rather than hitting the duplicate-id error). Clear the injected failure
    // to simulate the resource pressure easing on retry.
    backend.0.lock().unwrap().fail_dirs.remove(&dir_norm);

    if let Err(err) = w.watch_files("id", &[fsw(&pattern, None)]) {
        panic!("retry after rollback should succeed, got {}", err);
    }
    assert!(backend.is_watching(&dir_norm), "expected watch on {:?} after successful retry, got {:?}", dir_norm, backend.watched_dirs());
    w.close();
}

// lspwatcher_test.go:774
#[test]
fn watcher_watch_terminated_does_not_drop_events() {
    let dir = tempDir::new();
    let dir_norm = tspath::normalize_slashes(&dir.0);
    let backend = fakeBackend::new();
    let (got, on_changes) = recorder();
    let w = new_with_backend(os_fs(), Box::new(backend.clone()), on_changes, stderr_logger());

    let pattern = format!("{}/**/*", dir_norm);
    w.watch_files("id", &[fsw(&pattern, None)]).unwrap();

    backend.emit_all(vec![update(&format!("{}/b.ts", dir_norm))], Some(terminated_error("simulated")));

    wait_for(|| !got.lock().unwrap().is_empty(), "events with watch-terminated error");
    w.close();
}

struct blockingBackend {
    entered: Mutex<Option<mpsc::Sender<()>>>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl watcherBackend for blockingBackend {
    // lspwatcher_test.go:812
    fn watch_directory(&self, _dir: &str, _f: fswatch::WatchCallback, _opts: Vec<fswatch::WatchOption>) -> Result<Box<dyn fswatch::Watch>, fswatch::Error> {
        if let Some(entered) = self.entered.lock().unwrap().take() {
            let _ = entered.send(());
        }
        let _ = self.release.lock().unwrap().recv();
        Ok(Box::new(fakeWatch(Box::new(|| {}))))
    }
}

// lspwatcher_test.go:822
#[test]
fn watcher_close_while_watch_files_reconciles() {
    let dir = tempDir::new();
    let pattern = format!("{}/**/*", tspath::normalize_slashes(&dir.0));
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let backend = blockingBackend { entered: Mutex::new(Some(entered_tx)), release: Mutex::new(release_rx) };
    let w = new_with_backend(os_fs(), Box::new(backend), Arc::new(|_| {}), stderr_logger());

    let w2 = w.clone();
    let done = std::thread::spawn(move || w2.watch_files("id", &[fsw(&pattern, None)]));

    entered_rx.recv().unwrap();
    w.close();
    drop(release_tx);

    assert!(done.join().unwrap().is_err(), "expected WatchFiles to report that the watcher was closed");
}
