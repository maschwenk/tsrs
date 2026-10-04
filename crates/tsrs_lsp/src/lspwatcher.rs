// Package lspwatcher implements an in-process file watcher used as a
// drop-in replacement for LSP-based file watching when the client does not
// support dynamic registration of file watchers.

use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use indexmap::IndexMap;
use rustc_hash::FxHashMap;
use tsrs_core::tspath;
use tsrs_fswatch as fswatch;
use tsrs_ls::lsconv;
use tsrs_lsproto as lsproto;
use tsrs_project::background::{after_func, Timer};
use tsrs_project::logging::Logger;
use tsrs_vfs::{walk_dir, FS};

// throttleWindow mirrors VS Code's parcel watcher integration: give the
// first batch a short grace window so adjacent filesystem bursts coalesce.
const throttleWindow: Duration = Duration::from_millis(75);

pub(crate) trait watcherBackend: Send + Sync {
    fn watch_directory(&self, dir: &str, f: fswatch::WatchCallback, opts: Vec<fswatch::WatchOption>) -> Result<Box<dyn fswatch::Watch>, fswatch::Error>;
}

struct defaultWatcherBackend {
    watcher: &'static dyn fswatch::Watcher,
}

impl watcherBackend for defaultWatcherBackend {
    // lspwatcher.go:34
    fn watch_directory(&self, dir: &str, f: fswatch::WatchCallback, opts: Vec<fswatch::WatchOption>) -> Result<Box<dyn fswatch::Watch>, fswatch::Error> {
        self.watcher.watch_directory(dir, f, opts)
    }
}

pub type OnChanges = Arc<dyn Fn(Vec<lsproto::FileEvent>) + Send + Sync>;

// Watcher manages a set of file system subscriptions identified by
// WatcherID strings (matching the LSP server's project.WatcherID type).
// Events are delivered to onChanges in batches as `*lsproto.FileEvent`,
// shaped exactly like a `workspace/didChangeWatchedFiles` notification.
pub struct Watcher {
    fs: Arc<dyn FS>,
    backend: Box<dyn watcherBackend>,
    on_changes: OnChanges,
    logger: Arc<dyn Logger>,

    mu: Mutex<watcherState>,
    this: Weak<Watcher>,
}

#[derive(Default)]
struct watcherState {
    // watches holds the watches associated with each LSP WatcherID. A single id
    // may map to more than one watch because each FileSystemWatcher in the
    // registration becomes its own watch (different roots and kinds).
    // (Go sets the map to nil on Close.)
    watches: Option<FxHashMap<String, Vec<Arc<watch>>>>,
    closed: bool,

    // Pending batch state, protected by mu.
    pending: Option<IndexMap<String, lsproto::FileEvent>>,
    flush_timer: Option<Timer>,
}

// watch represents one FileSystemWatcher from the LSP registration.
//
// The directory the session asks to watch may not exist yet (common in
// granular mode, where each probed-but-missing package directory becomes a
// watch) or may be deleted while watched. To honor the watch across those
// transitions, a watch maintains either:
//
//   - a "target" subscription rooted directly at the requested directory, once
//     it exists, or
//   - an "ancestor" subscription on the nearest existing ancestor
//     (non-recursive), used to detect the requested directory — or an
//     intermediate path component — being created, after which the watch
//     descends toward and eventually promotes to the target.
//
// When the target materializes, synthetic create events are emitted for it
// (and, depending on whether the watch is recursive, its immediate children or
// its whole subtree) so the session re-resolves files that appeared in the gap
// before the real subscription was installed.
//
// All path fields are tspath-style (forward-slash) absolute paths.
struct watch {
    watcher: Weak<Watcher>,
    requested_directory: String, // directory requested by the LSP layer (possibly a symlink)
    kind: lsproto::WatchKind,
    recursive: bool, // whether the target subscription should be recursive

    mu: Mutex<watchState>,
    this: Weak<watch>,
}

#[derive(Default)]
struct watchState {
    subscription: Option<Box<dyn fswatch::Watch>>, // current subscription (target or ancestor); nil if none
    watched_directory: String,                     // canonicalized directory 'subscription' is rooted at
    watching_target: bool,                         // whether 'subscription' is rooted at the target directory
    closed: bool,
}

// lspwatcher.go:91
// New constructs a Watcher backed by internal/fswatch's platform-default
// watcher implementation.
pub fn new(fs: Arc<dyn FS>, on_changes: OnChanges, logger: Arc<dyn Logger>) -> Arc<Watcher> {
    new_with_fs_watcher(fs, fswatch::default(), on_changes, logger)
}

// lspwatcher.go:98
// NewWithFSWatcher constructs a Watcher backed by the provided fswatch.Watcher.
// Use this to select a specific backend (e.g. fswatch.Kqueue()) instead of the
// platform default.
pub fn new_with_fs_watcher(fs: Arc<dyn FS>, watcher: &'static dyn fswatch::Watcher, on_changes: OnChanges, logger: Arc<dyn Logger>) -> Arc<Watcher> {
    new_with_backend(fs, Box::new(defaultWatcherBackend { watcher }), on_changes, logger)
}

// lspwatcher.go:102
pub(crate) fn new_with_backend(fs: Arc<dyn FS>, backend: Box<dyn watcherBackend>, on_changes: OnChanges, logger: Arc<dyn Logger>) -> Arc<Watcher> {
    Arc::new_cyclic(|this| Watcher {
        fs,
        backend,
        on_changes,
        logger,
        mu: Mutex::new(watcherState { watches: Some(FxHashMap::default()), ..Default::default() }),
        this: Weak::clone(this),
    })
}

impl Watcher {
    // lspwatcher.go:120
    // WatchFiles subscribes to each FileSystemWatcher under the given id.
    //
    // A watcher whose directory does not exist yet is not an error: an ancestor
    // watch is installed on the nearest existing ancestor and the subscription is
    // reported as successful, so the session's notion of "this watcher is alive"
    // stays true for the subscription's whole lifetime. Only a genuine backend
    // failure (e.g. resource exhaustion while watching an existing directory)
    // causes WatchFiles to roll back the whole id and return an error, so the
    // session's pending/retry path re-registers it on the next reevaluation.
    pub fn watch_files(&self, id: &str, file_system_watchers: &[lsproto::FileSystemWatcher]) -> Result<(), String> {
        let mut st = self.mu.lock().unwrap();
        if st.closed {
            return Err("lspwatcher: closed".to_string());
        }
        let watches = st.watches.get_or_insert_with(FxHashMap::default);
        if watches.contains_key(id) {
            return Err(format!("lspwatcher: watcher {:?} already exists", id));
        }
        // Mark the id as existing before installing any watches so a concurrent
        // WatchFiles for the same id is rejected above.
        watches.insert(id.to_string(), Vec::new());
        drop(st);

        let mut failed = false;
        for file_system_watcher in file_system_watchers {
            let directory = match watch_root(file_system_watcher) {
                Some(d) if !d.is_empty() => d,
                _ => {
                    self.logger.logf(format_args!("lspwatcher: skipping watcher {:?}: unrecognized pattern {:?}", id, watch_pattern_string(file_system_watcher)));
                    continue;
                }
            };
            let new_watch = Arc::new_cyclic(|this| watch {
                watcher: Weak::clone(&self.this),
                requested_directory: directory.clone(),
                kind: effective_kind(file_system_watcher),
                recursive: is_recursive_glob(file_system_watcher),
                mu: Mutex::new(watchState::default()),
                this: Weak::clone(this),
            });
            if let Err(err) = new_watch.reconcile(false /*emitSynthetic*/) {
                self.logger.logf(format_args!("lspwatcher: failed to register watcher {:?} for {:?}: {}", id, directory, err));
                new_watch.close();
                failed = true;
                break;
            }
            let mut st = self.mu.lock().unwrap();
            if st.closed {
                drop(st);
                new_watch.close();
                return Err("lspwatcher: closed".to_string());
            }
            st.watches.get_or_insert_with(FxHashMap::default).entry(id.to_string()).or_default().push(new_watch);
        }

        if failed {
            // Roll back the whole id so the session's retry (MarkPending) can
            // cleanly re-register it. The session treats an id as a single unit.
            let _ = self.unwatch_files(id);
            return Err(format!("lspwatcher: failed to register one or more watchers for {:?}", id));
        }
        Ok(())
    }

    // lspwatcher.go:178
    // UnwatchFiles tears down all subscriptions associated with id.
    pub fn unwatch_files(&self, id: &str) -> Result<(), String> {
        let mut st = self.mu.lock().unwrap();
        let Some(watches) = st.watches.as_mut().and_then(|w| w.remove(id)) else {
            return Err(format!("lspwatcher: no watcher with id {:?}", id));
        };
        drop(st);
        for watch in watches {
            watch.close();
        }
        Ok(())
    }

    // lspwatcher.go:194
    // Close removes every subscription. Safe to call multiple times.
    pub fn close(&self) {
        let mut st = self.mu.lock().unwrap();
        if st.closed {
            return;
        }
        st.closed = true;
        let watches_by_id = st.watches.take();
        if let Some(timer) = st.flush_timer.take() {
            timer.stop();
        }
        st.pending = None;
        drop(st);
        for watches in watches_by_id.into_iter().flat_map(|m| m.into_values()) {
            for watch in watches {
                watch.close();
            }
        }
    }

    // lspwatcher.go:394
    // forwardEvents translates fswatch events into LSP file events and enqueues
    // them for the next debounced flush.
    fn forward_events(&self, kind: lsproto::WatchKind, events: &[fswatch::Event]) {
        let mut st = self.mu.lock().unwrap();
        if st.closed {
            return;
        }
        let pending = st.pending.get_or_insert_with(|| IndexMap::with_capacity(events.len()));
        for event in events {
            let change_type = match event.kind {
                fswatch::EventKind::Update => {
                    // fswatch intentionally doesn't distinguish create vs update.
                    // For LSP consumers this is fine: callers infer create/update
                    // from their own cache and both should invalidate stale state.
                    if kind.0 & (lsproto::WatchKind::Create.0 | lsproto::WatchKind::Change.0) == 0 {
                        continue;
                    }
                    lsproto::FileChangeType::Changed
                }
                fswatch::EventKind::Delete => {
                    if kind.0 & lsproto::WatchKind::Delete.0 == 0 {
                        continue;
                    }
                    lsproto::FileChangeType::Deleted
                }
                _ => continue,
            };

            let path = tspath::normalize_slashes(&event.path);
            let uri = lsconv::file_name_to_document_uri(&path);
            pending.insert(uri.0.clone(), lsproto::FileEvent { uri, type_: change_type });
        }
        self.schedule_flush_locked(&mut st);
    }

    // lspwatcher.go:440
    // emitSyntheticCreates enqueues synthetic create events after a target watch is
    // (re)installed following a missing→present transition, so the session
    // re-resolves files that appeared before the real watch existed. The target
    // directory itself is always included; for a non-recursive watch its immediate
    // children are added, and for a recursive watch its whole subtree is walked.
    // Nothing is emitted if the watch doesn't request create notifications.
    fn emit_synthetic_creates(&self, directory: &str, kind: lsproto::WatchKind, recursive: bool) {
        if kind.0 & lsproto::WatchKind::Create.0 == 0 {
            return;
        }
        let mut paths = vec![directory.to_string()];
        if recursive {
            let _ = walk_dir(&*self.fs, directory, &mut |path, _entry, err| {
                if err.is_none() && path != directory {
                    paths.push(path.to_string());
                }
                Ok(())
            });
        } else {
            let entries = self.fs.get_accessible_entries(directory);
            for name in &entries.files {
                paths.push(tspath::combine_paths(directory, &[name]));
            }
            for name in &entries.directories {
                paths.push(tspath::combine_paths(directory, &[name]));
            }
        }
        self.enqueue_synthetic_creates(&paths);
    }

    // lspwatcher.go:466
    // enqueueSyntheticCreates adds synthetic create events for paths, without
    // clobbering a more specific event already pending for the same path (e.g. a
    // real delete).
    fn enqueue_synthetic_creates(&self, paths: &[String]) {
        let mut st = self.mu.lock().unwrap();
        if st.closed {
            return;
        }
        let pending = st.pending.get_or_insert_with(|| IndexMap::with_capacity(paths.len()));
        for path in paths {
            let uri = lsconv::file_name_to_document_uri(path);
            if pending.contains_key(&uri.0) {
                continue;
            }
            pending.insert(uri.0.clone(), lsproto::FileEvent { uri, type_: lsproto::FileChangeType::Created });
        }
        self.schedule_flush_locked(&mut st);
    }

    // lspwatcher.go:490
    // scheduleFlushLocked arms the debounce flush timer if it isn't already armed.
    // Callers must hold w.mu.
    fn schedule_flush_locked(&self, st: &mut watcherState) {
        if st.flush_timer.is_none() {
            let this = Weak::clone(&self.this);
            st.flush_timer = Some(after_func(throttleWindow, move || {
                if let Some(w) = this.upgrade() {
                    w.flush();
                }
            }));
        }
    }

    // lspwatcher.go:496
    fn flush(&self) {
        let mut st = self.mu.lock().unwrap();
        if st.closed {
            return;
        }
        let pending = st.pending.take();
        st.flush_timer = None;
        drop(st);

        let Some(pending) = pending.filter(|p| !p.is_empty()) else {
            return;
        };
        let changes: Vec<lsproto::FileEvent> = pending.into_values().collect();
        (self.on_changes)(changes);
    }
}

impl watch {
    fn watcher(&self) -> Option<Arc<Watcher>> {
        self.watcher.upgrade()
    }

    // lspwatcher.go:218
    // close tears down the watch's current subscription and prevents any in-flight
    // reconcile from reinstalling one.
    fn close(&self) {
        let mut st = self.mu.lock().unwrap();
        st.closed = true;
        let subscription = st.subscription.take();
        st.watched_directory = String::new();
        drop(st);
        if let Some(subscription) = subscription {
            let _ = subscription.close();
        }
    }

    // lspwatcher.go:244
    // reconcile installs or advances this watch toward the target directory based
    // on the current filesystem state. It is called at registration, whenever a
    // ancestor watch observes activity, and after a target watch is terminated by
    // deletion.
    //
    // emitSynthetic controls whether promoting to the target emits synthetic
    // create events: false for the initial install when the target already exists
    // (the session already knows about those files), true for any missing→present
    // recovery.
    //
    // It returns a non-nil error only on a genuine backend failure to install a
    // watch; a missing target directory is handled by installing an ancestor watch
    // and returns nil.
    fn reconcile(&self, mut emit_synthetic_creates: bool) -> Result<(), fswatch::Error> {
        let mut st = self.mu.lock().unwrap();
        let Some(watcher) = self.watcher() else {
            return Ok(());
        };
        loop {
            if st.closed {
                return Ok(());
            }
            if watcher.fs.directory_exists(&self.requested_directory) {
                if st.watching_target && st.subscription.is_some() {
                    return Ok(()); // already watching the target
                }
                let target_directory = self.requested_directory.clone();
                let mut options = Vec::new();
                if self.recursive {
                    options.push(fswatch::with_recursive());
                }
                let subscription = watcher.backend.watch_directory(&target_directory, self.target_callback(&target_directory), options)?;
                let previous = st.subscription.replace(subscription);
                st.watched_directory = target_directory.clone();
                st.watching_target = true;
                if let Some(previous) = previous {
                    let _ = previous.close();
                }
                if emit_synthetic_creates {
                    watcher.emit_synthetic_creates(&target_directory, self.kind, self.recursive);
                }
                return Ok(());
            }

            let Some(ancestor) = nearest_existing_ancestor(&*watcher.fs, &self.requested_directory) else {
                // Nothing exists to watch (even the root is gone); drop any subscription.
                if let Some(previous) = st.subscription.take() {
                    st.watched_directory = String::new();
                    st.watching_target = false;
                    let _ = previous.close();
                }
                return Ok(());
            };
            let ancestor_directory = ancestor;
            if !st.watching_target && st.subscription.is_some() && st.watched_directory == ancestor_directory {
                return Ok(()); // already watching the correct ancestor
            }
            let subscription = watcher.backend.watch_directory(&ancestor_directory, self.ancestor_callback(), Vec::new())?;
            let previous = st.subscription.replace(subscription);
            st.watched_directory = ancestor_directory;
            st.watching_target = false;
            if let Some(previous) = previous {
                let _ = previous.close();
            }
            // The target may have appeared between the DirectoryExists check above
            // and installing this ancestor subscription (e.g. an atomic tree
            // creation), so loop to descend further or promote immediately. Any
            // promotion from here on is a missing→present transition, so synthesize
            // creates.
            emit_synthetic_creates = true;
        }
    }

    // lspwatcher.go:316
    // targetCallback returns the fswatch callback for a target watch rooted at
    // watchedReal. It forwards events to the session and, on ErrWatchTerminated
    // (the watched directory was deleted), falls back to watching the nearest
    // existing ancestor so the watch re-attaches when the directory is recreated.
    fn target_callback(&self, watched_directory: &str) -> fswatch::WatchCallback {
        let w = Weak::clone(&self.this);
        let watched_directory = watched_directory.to_string();
        Arc::new(move |events: Vec<fswatch::Event>, err: Option<fswatch::Error>| {
            let Some(w) = w.upgrade() else {
                return;
            };
            let Some(watcher) = w.watcher() else {
                return;
            };
            let mut terminated = false;
            if let Some(err) = err {
                if err.is(fswatch::ErrOverflow) {
                    watcher.logger.logf(format_args!("lspwatcher: watch overflow in {:?} (some events may have been dropped): {}", watched_directory, err));
                } else if err.is(fswatch::ErrWatchTerminated) {
                    terminated = true;
                    watcher.logger.logf(format_args!("lspwatcher: watch terminated in {:?} (directory removed): {}", watched_directory, err));
                } else {
                    watcher.logger.logf(format_args!("lspwatcher: watch error in {:?}: {}", watched_directory, err));
                }
            }
            if !events.is_empty() {
                watcher.forward_events(w.kind, &events);
            }
            if terminated {
                // The delete event for the directory was forwarded above; now
                // re-attach to the nearest existing ancestor.
                w.handle_terminated();
            }
        })
    }

    // lspwatcher.go:353
    // handleTerminated clears the dead target watch (the backend has already
    // removed it) and re-evaluates, falling back to an ancestor watch on the nearest
    // existing ancestor so the watch re-attaches when the directory reappears.
    // Clearing the state first is essential: reconcile would otherwise see
    // watchingTarget && subscription != nil and conclude the target is already
    // watched, even though the subscription is dead — losing recovery if the
    // directory is recreated before reconcile runs.
    fn handle_terminated(&self) {
        let mut st = self.mu.lock().unwrap();
        if st.closed {
            return;
        }
        let previous = st.subscription.take();
        st.watched_directory = String::new();
        st.watching_target = false;
        drop(st);
        if let Some(previous) = previous {
            let _ = previous.close();
        }
        let _ = self.reconcile(true /*emitSyntheticCreates*/);
    }

    // lspwatcher.go:374
    // ancestorCallback returns the fswatch callback for an ancestor watch. Ancestor
    // watches exist only to detect the target — or an intermediate path component —
    // being created; their events are about ancestor directories the session
    // doesn't track, so they are ignored and the watch is simply re-evaluated.
    fn ancestor_callback(&self) -> fswatch::WatchCallback {
        let w = Weak::clone(&self.this);
        Arc::new(move |_events: Vec<fswatch::Event>, _err: Option<fswatch::Error>| {
            if let Some(w) = w.upgrade() {
                let _ = w.reconcile(true /*emitSyntheticCreates*/);
            }
        })
    }
}

// lspwatcher.go:380
// nearestExistingAncestor returns the deepest existing directory that is dir or
// an ancestor of dir, walking upward. None only if nothing in the chain
// (including the root) exists.
fn nearest_existing_ancestor(fs: &dyn FS, dir: &str) -> Option<String> {
    let mut dir = dir.to_string();
    loop {
        if fs.directory_exists(&dir) {
            return Some(dir);
        }
        let parent = tspath::get_directory_path(&dir);
        if parent == dir {
            return None;
        }
        dir = parent;
    }
}

// lspwatcher.go:525
// watchRoot extracts the directory the fswatch subscription should be
// rooted at from a FileSystemWatcher. The patterns the project layer
// produces are of the form `<dir>/**/*` (recursive) or `<dir>/*`
// (non-recursive, used by granular watch mode), either as a Pattern
// with a fully-qualified directory or as a RelativePattern with a
// file:// BaseUri, so the heuristic of "everything before the first
// glob meta character" is reliable. Use [isRecursiveGlob] to determine
// whether the subscription should be recursive.
//
// Returned roots are tspath-normalized (forward-slash) absolute paths.
fn watch_root(file_system_watcher: &lsproto::FileSystemWatcher) -> Option<String> {
    if let Some(pattern) = &file_system_watcher.glob_pattern.pattern {
        return Some(root_from_glob(pattern));
    }
    if let Some(relative_pattern) = &file_system_watcher.glob_pattern.relative_pattern {
        let base = match &relative_pattern.base_uri.uri {
            Some(uri) => lsproto::DocumentUri(uri.0.clone()).file_name(),
            None => return None,
        };
        let pattern = tspath::combine_paths(&base, &[&relative_pattern.pattern]);
        return Some(root_from_glob(&pattern));
    }
    None
}

// lspwatcher.go:541
fn root_from_glob(pattern: &str) -> String {
    let pattern = tspath::normalize_slashes(pattern);
    let meta_index = pattern.bytes().position(|b| matches!(b, b'*' | b'?' | b'[' | b'{'));
    let Some(meta_index) = meta_index else {
        return tspath::normalize_path(pattern.trim_end_matches('/'));
    };
    let directory = pattern[..meta_index].trim_end_matches('/');
    if directory.is_empty() {
        return String::new();
    }
    tspath::normalize_path(directory)
}

// lspwatcher.go:563
fn watch_pattern_string(file_system_watcher: &lsproto::FileSystemWatcher) -> String {
    if let Some(pattern) = &file_system_watcher.glob_pattern.pattern {
        return pattern.clone();
    }
    if let Some(relative_pattern) = &file_system_watcher.glob_pattern.relative_pattern {
        let base = relative_pattern.base_uri.uri.as_ref().map(|u| u.0.clone()).unwrap_or_default();
        return format!("{}/{}", base, relative_pattern.pattern);
    }
    String::new()
}

// lspwatcher.go:580
// isRecursiveGlob reports whether a FileSystemWatcher's pattern requests
// recursive watching (contains a `**` segment). Granular watch mode emits
// non-recursive `<dir>/*` patterns, which watch only the immediate directory.
fn is_recursive_glob(file_system_watcher: &lsproto::FileSystemWatcher) -> bool {
    watch_pattern_string(file_system_watcher).contains("**")
}

// lspwatcher.go:584
fn effective_kind(file_system_watcher: &lsproto::FileSystemWatcher) -> lsproto::WatchKind {
    if let Some(kind) = file_system_watcher.kind {
        return kind;
    }
    lsproto::WatchKind(lsproto::WatchKind::Create.0 | lsproto::WatchKind::Change.0 | lsproto::WatchKind::Delete.0)
}

#[cfg(test)]
pub(crate) fn root_from_glob_for_test(pattern: &str) -> String {
    root_from_glob(pattern)
}
