// Port of Go's execute/tsctests/mock_watch_backend.go: records every directory watch, and routes events only through
// the watches whose paths match, so a scenario fails when the watcher does not register the watches it needs.

use std::sync::{Arc, Mutex};

use rustc_hash::FxHashMap;
use tsrs_core::tspath;
use tsrs_fswatch::{Error, ErrOverflow, Event, EventKind, IgnoreFunc, Watch, WatchCallback};

use crate::watchmanager::{WatchBackend, WatchDirectoryRequest};

struct mockWatch {
    path: String,
    callback: WatchCallback,
    recursive: bool,
    ignore: Option<IgnoreFunc>,
    closed: Arc<Mutex<bool>>,
}

struct mockCloser(Arc<Mutex<bool>>);

impl Watch for mockCloser {
    fn close(&self) -> Result<(), Error> {
        *self.0.lock().unwrap() = true;
        Ok(())
    }
}

/// Go `fsbaselineutil.FileChange`.
pub(crate) struct FileChange {
    pub(crate) path: String,
    pub(crate) deleted: bool,
}

pub(crate) struct MockWatchBackend {
    // The mock keeps closed watches (Go: `Dirs`), so "is watched" checks the closed flag.
    dirs: Mutex<FxHashMap<String, mockWatch>>,
    pub(crate) directory_exists: Box<dyn Fn(&str) -> bool + Send + Sync>,
    pub(crate) use_case_sensitive_file_names: bool,
}

impl MockWatchBackend {
    pub(crate) fn new(directory_exists: Box<dyn Fn(&str) -> bool + Send + Sync>, use_case_sensitive_file_names: bool) -> MockWatchBackend {
        MockWatchBackend { dirs: Mutex::new(FxHashMap::default()), directory_exists, use_case_sensitive_file_names }
    }

    pub(crate) fn has_watches(&self) -> bool {
        !self.dirs.lock().unwrap().is_empty()
    }

    /// Go `SendEvents`.
    pub(crate) fn send_events(&self, events: &[Event]) {
        // Snapshot callbacks under the lock, then invoke outside the lock (a callback may re-enter the mock).
        let mut targets: Vec<(String, WatchCallback, Vec<Event>)> = Vec::new();
        {
            let dirs = self.dirs.lock().unwrap();
            let mut order: Vec<&String> = dirs.keys().collect();
            order.sort();
            for e in events {
                for dir in &order {
                    let w = &dirs[*dir];
                    if *w.closed.lock().unwrap() {
                        continue;
                    }
                    if w.ignore.as_ref().is_some_and(|ignore| ignore(&e.path)) {
                        continue;
                    }
                    if !path_is_under(&e.path, &w.path, w.recursive, self.use_case_sensitive_file_names) {
                        continue;
                    }
                    match targets.iter_mut().find(|t| t.0 == w.path) {
                        Some(t) => t.2.push(e.clone()),
                        None => targets.push((w.path.clone(), Arc::clone(&w.callback), vec![e.clone()])),
                    }
                }
            }
        }
        for (_, callback, events) in targets {
            callback(events, None);
        }
    }

    /// Go `SendOverflow`.
    #[expect(dead_code, reason = "Go's mock API; the replayed scenarios do not overflow")]
    pub(crate) fn send_overflow(&self) {
        let callbacks: Vec<WatchCallback> =
            self.dirs.lock().unwrap().values().filter(|w| !*w.closed.lock().unwrap()).map(|w| Arc::clone(&w.callback)).collect();
        for callback in callbacks {
            callback(Vec::new(), Some(Error::wrap("overflow", &Error::new(ErrOverflow.0))));
        }
    }

    /// Go `SendChangedPaths`: an event per changed file, plus update events for its parent directories (real
    /// watchers report a new child directory to a non-recursive watch on its parent).
    pub(crate) fn send_changed_paths(&self, changes: &[FileChange]) {
        let mut events = Vec::with_capacity(changes.len() * 2);
        let mut seen_dirs: rustc_hash::FxHashSet<String> = rustc_hash::FxHashSet::default();
        for c in changes {
            let kind = if c.deleted { EventKind::Delete } else { EventKind::Update };
            events.push(Event::new(kind, c.path.clone()));
            let mut dir = path_dir(&c.path);
            while !dir.is_empty() && dir != "/" && dir != "." {
                if !seen_dirs.insert(dir.clone()) {
                    break;
                }
                events.push(Event::new(EventKind::Update, dir.clone()));
                let parent = path_dir(&dir);
                if parent == dir {
                    break;
                }
                dir = parent;
            }
        }
        self.send_events(&events);
    }

    /// Go `WatchState`.
    pub(crate) fn watch_state(&self) -> String {
        let dirs = self.dirs.lock().unwrap();
        let mut open: Vec<&mockWatch> = dirs.values().filter(|w| !*w.closed.lock().unwrap()).collect();
        open.sort_by(|a, b| a.path.cmp(&b.path));
        let mut b = String::from("Watch Registrations::\nDirectory watches::\n");
        if open.is_empty() {
            b.push_str("  (none)\n");
        }
        for w in open {
            if w.recursive {
                b.push_str(&format!("  {} (recursive)\n", w.path));
            } else {
                b.push_str(&format!("  {}\n", w.path));
            }
        }
        b
    }
}

impl WatchBackend for MockWatchBackend {
    fn watch_directories(&self, requests: Vec<WatchDirectoryRequest>) -> Result<Vec<Box<dyn Watch>>, Error> {
        let mut dirs = self.dirs.lock().unwrap();
        if let Some(missing) = requests.iter().find(|r| !(self.directory_exists)(&r.dir)) {
            return Err(Error::new(format!("directory does not exist: {}", missing.dir)));
        }
        let mut closers: Vec<Box<dyn Watch>> = Vec::with_capacity(requests.len());
        for r in requests {
            let closed = Arc::new(Mutex::new(false));
            closers.push(Box::new(mockCloser(Arc::clone(&closed))));
            dirs.insert(r.dir.clone(), mockWatch { path: r.dir, callback: r.callback, recursive: r.recursive, ignore: r.ignore, closed });
        }
        Ok(closers)
    }
}

// Go `path.Dir` for the absolute, slash-separated paths the harness uses.
fn path_dir(p: &str) -> String {
    match p.rfind('/') {
        Some(0) => "/".to_string(),
        Some(i) => p[..i].to_string(),
        None => ".".to_string(),
    }
}

// Go `pathIsUnder`: whether `event_path` is inside `dir` (a direct child only, unless recursive).
fn path_is_under(event_path: &str, dir: &str, recursive: bool, use_case_sensitive_file_names: bool) -> bool {
    let (event_path, dir) = if use_case_sensitive_file_names {
        (event_path.to_string(), dir.to_string())
    } else {
        (tspath::get_canonical_file_name(event_path, false), tspath::get_canonical_file_name(dir, false))
    };
    let Some(rest) = event_path.strip_prefix(dir.as_str()) else { return false };
    if rest.is_empty() || !rest.starts_with('/') {
        return false; // the dir itself, or a sibling like /foobar for /foo
    }
    recursive || !rest[1..].contains('/')
}
