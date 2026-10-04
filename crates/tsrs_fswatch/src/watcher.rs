use std::any::Any;
use rustc_hash::{FxHashMap, FxHashSet};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex};

use crate::canonicalize::canonicalize_path;
use crate::debounce::debounce;
use crate::event::{eventList, Event, EventKind};
use crate::pathcompare::{comparisonCache, comparisonPath, pathComparer};

// ----- errors -----------------------------------------------------------------
//
// Go's sentinel errors and `fmt.Errorf("%w")` chains -> `Error`: a message plus the sentinels it wraps (for
// `errors.Is`) and the errno it wraps, if any (`errors.Is(err, unix.ENOENT)`).

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sentinel(pub &'static str);

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Error {
    message: String,
    wraps: Vec<Sentinel>,
    errno: Option<i32>,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Error {
        Error { message: message.into(), wraps: Vec::new(), errno: None }
    }

    pub fn from_errno(errno: i32) -> Error {
        Error { message: std::io::Error::from_raw_os_error(errno).to_string(), wraps: Vec::new(), errno: Some(errno) }
    }

    pub fn from_io(err: std::io::Error) -> Error {
        Error { message: err.to_string(), wraps: Vec::new(), errno: err.raw_os_error() }
    }

    // Go `&os.PathError{Op, Path, Err}`.
    pub(crate) fn path_error(op: &str, path: &str, errno: i32) -> Error {
        let inner = Error::from_errno(errno);
        Error { message: format!("{} {}: {}", op, path, inner.message), wraps: Vec::new(), errno: Some(errno) }
    }

    // `fmt.Errorf(format with one %w, inner)`: the message is `message`, the chain is inner's.
    pub fn wrap(message: impl Into<String>, inner: &Error) -> Error {
        Error { message: message.into(), wraps: inner.wraps.clone(), errno: inner.errno }
    }

    // `fmt.Errorf("%w: %w", a, b)` and similar: both chains.
    pub fn wrap2(message: impl Into<String>, a: &Error, b: &Error) -> Error {
        let mut wraps = a.wraps.clone();
        wraps.extend(b.wraps.iter().copied());
        Error { message: message.into(), wraps, errno: a.errno.or(b.errno) }
    }

    pub fn is(&self, s: Sentinel) -> bool {
        self.wraps.contains(&s)
    }

    pub fn is_errno(&self, errno: i32) -> bool {
        self.errno == Some(errno)
    }

    pub fn errno(&self) -> Option<i32> {
        self.errno
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl From<Sentinel> for Error {
    fn from(s: Sentinel) -> Error {
        Error { message: s.0.to_string(), wraps: vec![s], errno: None }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

pub(crate) static errNilCallback: Sentinel = Sentinel("fswatch: callback must not be nil");

// errRootPath is returned by WatchFile when the supplied path is a
// filesystem root with no parent directory to watch.
pub(crate) static errRootPath: Sentinel = Sentinel("fswatch: cannot watch a root path");

// errNotAbsolute is returned by [Watcher.WatchDirectory] and
// [Watcher.WatchFile] when the supplied path is not absolute.
pub(crate) static errNotAbsolute: Sentinel = Sentinel("fswatch: path must be absolute");

// ErrOverflow indicates that the kernel event queue overflowed and
// some filesystem changes were missed. The watch remains
// active; further events will continue to be delivered. Callers
// should treat this as a signal to rescan the watched directory.
pub static ErrOverflow: Sentinel = Sentinel("fswatch: event overflow; some changes were missed");

// ErrWatchTerminated indicates that the watch was terminated due to
// an unrecoverable error (e.g. the watched directory was deleted or
// the watch descriptor was revoked). No further events will be
// delivered. Call Close to release remaining state.
pub static ErrWatchTerminated: Sentinel = Sentinel("fswatch: watch terminated");

// ErrUnavailable indicates that a requested watcher is not
// available on the current platform.
pub static ErrUnavailable: Sentinel = Sentinel("fswatch: watcher not available on this platform");

// ErrFilesystemUnsupported indicates that the active watcher backend cannot
// operate on the target filesystem, even though the backend is available on
// the current platform.
pub static ErrFilesystemUnsupported: Sentinel = Sentinel("fswatch: watcher backend unsupported on this filesystem");

// `fmt.Errorf("%w: watched directory removed", ErrWatchTerminated)`.
pub(crate) fn err_watched_directory_removed() -> Error {
    Error::wrap(format!("{}: watched directory removed", ErrWatchTerminated.0), &ErrWatchTerminated.into())
}

// ----- public API ---------------------------------------------------------------

// WatchCallback receives batched filesystem events. Rapid changes
// are coalesced before delivery.
//
// For a given Watch, the callback is never invoked concurrently
// with itself. It runs on a library goroutine, not the caller's.
//
// When err is non-nil, use [errors.Is] to check for [ErrOverflow]
// (recoverable) or [ErrWatchTerminated] (terminal).
pub type WatchCallback = Arc<dyn Fn(Vec<Event>, Option<Error>) + Send + Sync>;

pub type IgnoreFunc = Arc<dyn Fn(&str) -> bool + Send + Sync>;

// Watcher represents a filesystem watching implementation.
// Use one of the constructor functions ([Inotify], [FSEvents], [Kqueue],
// [Windows]) to obtain a value, or [Default] for the platform default.
//
// All watchers exist on every platform. Subscribing with a watcher that
// is not supported on the current OS returns [ErrUnavailable].
pub trait Watcher: Send + Sync {
    // Name returns a stable identifier ("inotify", "fsevents", "kqueue",
    // "windows").
    fn name(&self) -> &str;
    // Available reports whether this watcher works on the current OS.
    fn available(&self) -> bool;
    // HasFastRecursiveBackend reports whether this watcher supports efficient
    // recursive watching without requiring a full userspace tree walk. This is
    // true for Windows (ReadDirectoryChangesW subtree mode) and macOS FSEvents
    // (inherently recursive), and false for all other backends.
    fn has_fast_recursive_backend(&self) -> bool;
    // WatchDirectory watches dir for changes, calling fn with batched
    // events. By default, only direct children are watched. Use
    // [WithRecursive] to watch the entire directory tree.
    // dir must be an absolute path to an existing directory. If dir is a
    // symlink or reparse point to a directory, the OS subscription follows
    // the target directory but delivered event paths remain rooted at dir.
    // Userspace recursive traversal does not follow symlinked descendant
    // directories.
    // Returns [ErrUnavailable] if the watcher is not supported on
    // the current platform.
    fn watch_directory(&self, dir: &str, f: WatchCallback, opts: Vec<WatchOption>) -> Result<Box<dyn Watch>, Error>;
    // WatchDirectories watches multiple directories as a batch. It has the
    // same semantics as calling [Watcher.WatchDirectory] for each request, but
    // lets backends arm the underlying OS watches once for the whole batch.
    // Returned watches are in the same order as requests.
    fn watch_directories(&self, requests: Vec<WatchDirectoryRequest>) -> Result<Vec<Box<dyn Watch>>, Error>;
    // WatchFile watches a single file for changes, calling fn with
    // batched events. path must be an absolute path. The file does not
    // need to exist at subscribe time; its creation will be reported.
    // The parent directory must exist.
    //
    // Multiple WatchFile calls for files in the same directory
    // share a single OS watch on the parent directory.
    //
    // If the parent directory is deleted, [ErrWatchTerminated] is
    // delivered and the watch is dead.
    fn watch_file(&self, path: &str, f: WatchCallback) -> Result<Box<dyn Watch>, Error>;
}

// WatchOption configures a watch. (Go: an interface with one implementation per option.)
#[derive(Clone)]
pub enum WatchOption {
    Ignore(IgnoreFunc),
    Recursive,
    // fileOption defers the file filter until the parent directory's comparer is
    // available, so WatchFile does not need a second filesystem query.
    File(String),
}

impl WatchOption {
    fn apply_watch_option(&self, opts: &mut watchOptions) {
        match self {
            WatchOption::Ignore(f) => opts.ignore = Some(Arc::clone(f)),
            WatchOption::Recursive => opts.recursive = true,
            WatchOption::File(path) => opts.file.clone_from(path),
        }
    }
}

// WatchDirectoryRequest describes one directory subscription in a
// [Watcher.WatchDirectories] batch.
#[derive(Clone)]
pub struct WatchDirectoryRequest {
    pub dir: String,
    pub callback: Option<WatchCallback>,
    pub options: Vec<WatchOption>,
}

#[derive(Default)]
struct watchOptions {
    ignore: Option<IgnoreFunc>,
    recursive: bool,
    file: String,
}

// watcher.go:164
// WithIgnore returns a [WatchOption] that filters events before delivery.
// If the function returns true for a path, events for that path are
// silently dropped. The filtering is per-subscriber; multiple watches
// on the same directory may have different ignore functions.
pub fn with_ignore(f: IgnoreFunc) -> WatchOption {
    WatchOption::Ignore(f)
}

// watcher.go:183
// WithRecursive returns a [WatchOption] that enables recursive watching
// of the entire directory tree. Without this option,
// [Watcher.WatchDirectory] watches only direct children of dir.
pub fn with_recursive() -> WatchOption {
    WatchOption::Recursive
}

// Watch represents a live watch. Close stops watching
// and releases resources. It is idempotent.
pub trait Watch: Send + Sync {
    fn close(&self) -> Result<(), Error>;
}

// Package-level watcher instances. Platform init() functions set the factory.
pub(crate) static inotifyWatcher: LazyLock<watcher> = LazyLock::new(|| {
    #[allow(unused_mut)]
    let mut w = watcher::new("inotify");
    #[cfg(target_os = "linux")]
    crate::inotify_linux::init(&mut w);
    w
});
pub(crate) static fseventsWatcher: LazyLock<watcher> = LazyLock::new(|| {
    #[allow(unused_mut)]
    let mut w = watcher::new("fsevents");
    #[cfg(target_os = "macos")]
    crate::fsevents_darwin::init(&mut w);
    w
});
pub(crate) static kqueueWatcher: LazyLock<watcher> = LazyLock::new(|| {
    #[allow(unused_mut)]
    let mut w = watcher::new("kqueue");
    #[cfg(any(target_os = "macos", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd", target_os = "dragonfly"))]
    crate::kqueue::init(&mut w);
    w
});
// windows.go (ReadDirectoryChangesW) is not ported: Windows builds get an unavailable watcher.
pub(crate) static windowsWatcher: LazyLock<watcher> = LazyLock::new(|| watcher::new("windows"));
// fanotify_linux.go is not ported: fanotify().Available() is false, so Linux uses inotify (Go's Default() falls
// back to inotify the same way on kernels without fanotify).
pub(crate) static fanotifyWatcher: LazyLock<watcher> = LazyLock::new(|| watcher::new("fanotify"));
pub(crate) static fanotifyFallbackWatcher: LazyLock<fallbackWatcher> =
    LazyLock::new(|| fallbackWatcher { primary: &*fanotifyWatcher, secondary: &*inotifyWatcher });
static unsupportedWatcher: LazyLock<watcher> = LazyLock::new(|| watcher::new("unsupported"));

// watcher.go:203
// AllWatchers returns a fresh slice listing every watcher backend the package
// knows about. Use [Watcher.Available] to check which ones work on the current
// OS.
pub fn all_watchers() -> Vec<&'static dyn Watcher> {
    vec![&*inotifyWatcher, &*fseventsWatcher, &*kqueueWatcher, &*windowsWatcher, &*fanotifyFallbackWatcher]
}

// watcher.go:214
// Inotify returns the inotify watcher (Linux and Android).
pub fn inotify() -> &'static dyn Watcher {
    &*inotifyWatcher
}

// watcher.go:217
// FSEvents returns the FSEvents watcher (macOS).
pub fn fsevents() -> &'static dyn Watcher {
    &*fseventsWatcher
}

// watcher.go:220
// Kqueue returns the kqueue watcher (macOS, FreeBSD, and other BSDs).
pub fn kqueue() -> &'static dyn Watcher {
    &*kqueueWatcher
}

// watcher.go:223
// Windows returns the ReadDirectoryChangesW watcher (Windows).
pub fn windows() -> &'static dyn Watcher {
    &*windowsWatcher
}

// watcher.go:227
// Fanotify returns the fanotify watcher (Linux, kernel ≥ 5.13). Directories on
// filesystems that don't support fanotify watches automatically use inotify instead.
pub fn fanotify() -> &'static dyn Watcher {
    &*fanotifyFallbackWatcher
}

// watcher.go:230
// Default returns the recommended watcher for the current OS.
pub fn default() -> &'static dyn Watcher {
    if cfg!(target_os = "linux") {
        if fanotify().available() {
            return fanotify();
        }
        inotify()
    } else if cfg!(target_os = "android") {
        inotify()
    } else if cfg!(target_os = "macos") {
        if fsevents().available() {
            return fsevents();
        }
        kqueue()
    } else if cfg!(target_os = "windows") {
        windows()
    } else if cfg!(any(target_os = "freebsd", target_os = "openbsd", target_os = "netbsd", target_os = "dragonfly")) {
        kqueue()
    } else {
        &*unsupportedWatcher
    }
}

// fallbackWatcher keeps the primary backend for supported filesystems while
// routing individual unsupported watches to the secondary backend.
pub(crate) struct fallbackWatcher {
    primary: &'static dyn Watcher,
    secondary: &'static dyn Watcher,
}

impl Watcher for fallbackWatcher {
    fn name(&self) -> &str {
        self.primary.name()
    }
    fn available(&self) -> bool {
        self.primary.available()
    }
    fn has_fast_recursive_backend(&self) -> bool {
        self.primary.has_fast_recursive_backend()
    }

    // watcher.go:263
    fn watch_directory(&self, dir: &str, f: WatchCallback, opts: Vec<WatchOption>) -> Result<Box<dyn Watch>, Error> {
        let mut watches = self.watch_directories(vec![WatchDirectoryRequest { dir: dir.to_string(), callback: Some(f), options: opts }])?;
        Ok(watches.remove(0))
    }

    // watcher.go:275
    fn watch_directories(&self, requests: Vec<WatchDirectoryRequest>) -> Result<Vec<Box<dyn Watch>>, Error> {
        match self.primary.watch_directories(requests.clone()) {
            Ok(watches) => return Ok(watches),
            Err(err) if !err.is(ErrFilesystemUnsupported) => return Err(err),
            Err(_) => {}
        }

        let mut watches: Vec<Box<dyn Watch>> = Vec::with_capacity(requests.len());
        for request in requests {
            let f = request.callback.clone().ok_or_else(|| Error::from(errNilCallback))?;
            let mut result = self.primary.watch_directory(&request.dir, Arc::clone(&f), request.options.clone());
            if matches!(&result, Err(err) if err.is(ErrFilesystemUnsupported)) {
                result = self.secondary.watch_directory(&request.dir, f, request.options.clone());
            }
            match result {
                Ok(watch) => watches.push(watch),
                Err(err) => {
                    for watch in watches.iter().rev() {
                        let _ = watch.close();
                    }
                    return Err(Error::wrap(format!("fswatch: failed to watch directory {:?}: {}", request.dir, err), &err));
                }
            }
        }
        Ok(watches)
    }

    // watcher.go:300
    fn watch_file(&self, path: &str, f: WatchCallback) -> Result<Box<dyn Watch>, Error> {
        let watch = self.primary.watch_file(path, Arc::clone(&f));
        if matches!(&watch, Err(err) if err.is(ErrFilesystemUnsupported)) {
            return self.secondary.watch_file(path, f);
        }
        watch
    }
}

// Pointer-identity key for `*dirWatch` map keys.
#[derive(Clone)]
pub(crate) struct dwKey(pub(crate) Arc<dirWatch>);

impl PartialEq for dwKey {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for dwKey {}
impl std::hash::Hash for dwKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (Arc::as_ptr(&self.0) as usize).hash(state)
    }
}

pub(crate) type watcherFactory = fn() -> Arc<dyn watcherImpl>;

// watcher is the concrete implementation of [Watcher]. Each platform
// watcher is a package-level *watcher whose factory is set by the
// platform's init() function.
pub(crate) struct watcher {
    pub(crate) name: &'static str,
    mu: Mutex<watcherState>,
    pub(crate) factory: Option<watcherFactory>, // nil if not available on this platform
    pub(crate) sequence: Option<fn() -> u64>,
}

#[derive(Default)]
struct watcherState {
    impl_: Option<Arc<dyn watcherImpl>>,
    dir_watches: FxHashMap<String, Arc<dirWatch>>,
    debounce: Option<Arc<debounce>>, // lazily created in getOrCreateDirWatch
}

const recursiveConsolidateThreshold: usize = 10;

impl watcher {
    fn new(name: &'static str) -> watcher {
        watcher { name, mu: Mutex::new(watcherState::default()), factory: None, sequence: None }
    }

    // watcher.go:340
    fn can_share_recursive_dir_watches(&self) -> bool {
        // TODO: Re-enable this for Windows once coalesced recursive watches have
        // more real-world bake time.
        self.name == "fsevents"
    }

    // watcher.go:346
    fn get_impl(&self) -> Result<Arc<dyn watcherImpl>, Error> {
        let st = self.mu.lock().unwrap();
        if let Some(impl_) = &st.impl_ {
            return Ok(Arc::clone(impl_));
        }
        let factory = self.factory;
        drop(st);

        let Some(factory) = factory else {
            return Err(ErrUnavailable.into());
        };

        let impl_ = factory();
        run(&impl_)?;

        let mut st = self.mu.lock().unwrap();
        if let Some(existing) = &st.impl_ {
            let existing = Arc::clone(existing);
            drop(st);
            impl_.shutdown();
            return Ok(existing);
        }
        st.impl_ = Some(Arc::clone(&impl_));
        Ok(impl_)
    }

    // watcher.go:377
    fn key_for_dir_watch(&self, dir: &str, recursive: bool) -> String {
        if recursive {
            return format!("{}\x00recursive", dir);
        }
        dir.to_string()
    }

    // watcher.go:384
    fn find_covering_recursive_watch_locked(st: &watcherState, dir: &str, physical_dir: &str, comparer: pathComparer) -> Option<Arc<dirWatch>> {
        let mut best: Option<&Arc<dirWatch>> = None;
        #[expect(clippy::iter_over_hash_type, reason = "the longest covering dir is unique: candidates are prefixes of one path, one key per dir; Go ranges the map too")]
        for dw in st.dir_watches.values() {
            if !dw.recursive || dw.comparer != comparer || !is_in_directory_or_self(&dw.dir, dir) || !is_in_directory_or_self(&dw.physical_dir, physical_dir) {
                continue;
            }
            if best.is_none_or(|b| dw.dir.len() > b.dir.len()) {
                best = Some(dw);
            }
        }
        best.cloned()
    }

    // watcher.go:397
    fn find_consolidation_dir_locked(&self, st: &watcherState, dir: &str, physical_dir: &str) -> String {
        if !self.can_share_recursive_dir_watches() {
            return String::new();
        }
        let mut dir = dir.to_string();
        let mut parent = filepath_dir(&dir);
        while parent != dir && parent != "." {
            if filepath_dir(&parent) == parent {
                break;
            }
            let physical_parent = physical_dir_for(&parent);
            if !is_in_directory_or_self(&physical_parent, physical_dir) {
                return String::new();
            }
            let mut count = 1;
            #[expect(clippy::iter_over_hash_type, reason = "only counts matches; returns `parent` whichever entries were counted; Go ranges the map too")]
            for dw in st.dir_watches.values() {
                if is_in_directory_or_self(&parent, &dw.dir) && is_in_directory_or_self(&physical_parent, &dw.physical_dir) {
                    count += 1;
                    if count >= recursiveConsolidateThreshold {
                        return parent;
                    }
                }
            }
            let next = filepath_dir(&parent);
            if next == parent {
                break;
            }
            dir = parent;
            parent = next;
        }
        String::new()
    }

    // watcher.go:429
    fn get_or_create_dir_watch(&self, dir: &str, physical_dir: &str, recursive: bool, comparer: pathComparer) -> Result<Arc<dirWatch>, Error> {
        let mut st = self.mu.lock().unwrap();
        if st.debounce.is_none() {
            st.debounce = Some(debounce::new());
        }
        let (mut dir, mut physical_dir, mut recursive) = (dir.to_string(), physical_dir.to_string(), recursive);

        if self.can_share_recursive_dir_watches() {
            if let Some(dw) = Self::find_covering_recursive_watch_locked(&st, &dir, &physical_dir, comparer) {
                return Ok(dw);
            }
            let consolidation_dir = self.find_consolidation_dir_locked(&st, &dir, &physical_dir);
            if !consolidation_dir.is_empty() {
                let parent_comparer = self.path_comparer(&consolidation_dir)?;
                if parent_comparer == comparer {
                    dir = consolidation_dir;
                    physical_dir = physical_dir_for(&dir);
                    recursive = true;
                    if let Some(dw) = Self::find_covering_recursive_watch_locked(&st, &dir, &physical_dir, comparer) {
                        return Ok(dw);
                    }
                }
            }
        }

        let key = self.key_for_dir_watch(&dir, recursive);
        if let Some(dw) = st.dir_watches.get(&key) {
            return Ok(Arc::clone(dw));
        }
        let dw = dirWatch::new(dir, physical_dir, st.debounce.as_ref().unwrap(), comparer, self.sequence, recursive);
        st.dir_watches.insert(key, Arc::clone(&dw));
        Ok(dw)
    }

    // watcher.go:471
    fn remove_dir_watch(&self, dw: &Arc<dirWatch>) {
        let mut st = self.mu.lock().unwrap();
        let key = self.key_for_dir_watch(&dw.dir, dw.recursive);
        if let Some(existing) = st.dir_watches.get(&key) {
            if Arc::ptr_eq(existing, dw) {
                st.dir_watches.remove(&key);
                dw.destroy_debounce();
            }
        }
    }
}

impl Watcher for watcher {
    fn name(&self) -> &str {
        self.name
    }

    fn available(&self) -> bool {
        self.factory.is_some()
    }

    // watcher.go:331
    // HasFastRecursiveBackend implements [Watcher.HasFastRecursiveBackend].
    fn has_fast_recursive_backend(&self) -> bool {
        matches!(self.name, "windows" | "fsevents")
    }

    // watcher.go:481
    fn watch_directory(&self, dir: &str, f: WatchCallback, opts: Vec<WatchOption>) -> Result<Box<dyn Watch>, Error> {
        let mut watches = self.watch_directories(vec![WatchDirectoryRequest { dir: dir.to_string(), callback: Some(f), options: opts }])?;
        Ok(watches.remove(0))
    }

    // watcher.go:493
    fn watch_directories(&self, requests: Vec<WatchDirectoryRequest>) -> Result<Vec<Box<dyn Watch>>, Error> {
        if !self.available() {
            return Err(ErrUnavailable.into());
        }
        if requests.is_empty() {
            return Ok(Vec::new());
        }

        struct preparedWatch {
            dw: Arc<dirWatch>,
            id: u64,
        }
        let mut prepared: Vec<preparedWatch> = Vec::with_capacity(requests.len());
        let mut unique_dir_watches: Vec<Arc<dirWatch>> = Vec::with_capacity(requests.len());
        let mut seen_dir_watches: FxHashSet<dwKey> = FxHashSet::with_capacity_and_hasher(requests.len(), Default::default());
        let rollback = |prepared: &[preparedWatch]| {
            for p in prepared.iter().rev() {
                p.dw.unwatch(p.id);
                p.dw.unref(self);
            }
        };

        for request in requests {
            let Some(f) = request.callback else {
                rollback(&prepared);
                return Err(errNilCallback.into());
            };
            let dir = filepath_clean(&request.dir);
            if !dir.starts_with('/') {
                rollback(&prepared);
                return Err(errNotAbsolute.into());
            }
            let dir = canonicalize_path(&dir);
            if self.can_share_recursive_dir_watches() {
                if let Err(err) = validate_watch_directory(&dir) {
                    rollback(&prepared);
                    return Err(err);
                }
            }
            let physical_dir = physical_dir_for(&dir);

            let mut sopts = watchOptions::default();
            for o in &request.options {
                o.apply_watch_option(&mut sopts);
            }

            let comparer = match self.path_comparer(&dir) {
                Ok(c) => c,
                Err(err) => {
                    rollback(&prepared);
                    return Err(err);
                }
            };
            let dw = match self.get_or_create_dir_watch(&dir, &physical_dir, sopts.recursive, comparer) {
                Ok(dw) => dw,
                Err(err) => {
                    rollback(&prepared);
                    return Err(err);
                }
            };
            let id = dw.add_callback(&dir, &physical_dir, sopts.recursive, f, sopts.ignore, &sopts.file);
            prepared.push(preparedWatch { dw: Arc::clone(&dw), id });
            if seen_dir_watches.insert(dwKey(Arc::clone(&dw))) {
                unique_dir_watches.push(dw);
            }
        }

        let impl_ = match self.get_impl() {
            Ok(impl_) => impl_,
            Err(err) => {
                rollback(&prepared);
                return Err(err);
            }
        };
        if let Err(err) = watch_add_many(&impl_, &unique_dir_watches) {
            rollback(&prepared);
            return Err(err);
        }

        let self_static: &'static watcher = self.as_static();
        Ok(prepared
            .into_iter()
            .map(|p| Box::new(watch { mu: Mutex::new(false), w: self_static, dw: p.dw, impl_: Arc::clone(&impl_), id: p.id }) as Box<dyn Watch>)
            .collect())
    }

    // watcher.go:589
    fn watch_file(&self, path: &str, f: WatchCallback) -> Result<Box<dyn Watch>, Error> {
        if !self.available() {
            return Err(ErrUnavailable.into());
        }
        let path = filepath_clean(path);
        if !path.starts_with('/') {
            return Err(errNotAbsolute.into());
        }
        let path = canonicalize_path(&path);
        let dir = filepath_dir(&path);
        if dir == path {
            return Err(errRootPath.into());
        }

        self.watch_directory(&dir, f, vec![WatchOption::File(path)])
    }
}

impl watcher {
    // Every `watcher` is one of the package-level statics.
    fn as_static(&self) -> &'static watcher {
        for w in [&*inotifyWatcher, &*fseventsWatcher, &*kqueueWatcher, &*windowsWatcher, &*fanotifyWatcher, &*unsupportedWatcher] {
            if std::ptr::eq(w, self) {
                return w;
            }
        }
        unreachable!("fswatch: watcher is not a package-level instance")
    }
}

// watcher.go:578
fn validate_watch_directory(dir: &str) -> Result<(), Error> {
    let md = std::fs::metadata(dir).map_err(|e| Error::path_error("stat", dir, e.raw_os_error().unwrap_or(0)))?;
    if !md.is_dir() {
        return Err(Error::from_errno(libc::ENOTDIR));
    }
    Ok(())
}

struct watch {
    mu: Mutex<bool>, // cancelled
    w: &'static watcher,
    dw: Arc<dirWatch>,
    impl_: Arc<dyn watcherImpl>,
    id: u64,
}

impl Watch for watch {
    // watcher.go:620
    fn close(&self) -> Result<(), Error> {
        let mut cancelled = self.mu.lock().unwrap();
        if *cancelled {
            return Ok(());
        }
        *cancelled = true;
        let last = self.dw.unwatch(self.id);
        if last {
            watch_remove(&self.impl_, &self.dw);
            self.dw.unref(self.w);
        }
        Ok(())
    }
}

// watcherImpl is the internal interface implemented by each platform watcher.
// Go embeds `watcherBase` (shared watch tracking and lifecycle) in each backend and dispatches through
// `self`; here `base()` returns the embedded value and the shared logic is the free functions below.
pub(crate) trait watcherImpl: Send + Sync + 'static {
    fn base(&self) -> &watcherBase;
    // Runs on its own thread (Go: a goroutine started by watcherBase.run) and returns when the backend stops.
    fn start(self: Arc<Self>) -> Result<(), Error>;
    fn shutdown(&self) {}
    fn subscribe(&self, w: &Arc<dirWatch>) -> Result<(), Error>;
    // Go: the optional `subscribeMany` interface; None = not implemented.
    fn subscribe_many(&self, _watches: &[Arc<dirWatch>]) -> Option<Result<(), Error>> {
        None
    }
    fn close_watch(&self, w: &Arc<dirWatch>) -> Result<(), Error>;
}

// watcherBase provides shared watch-tracking and lifecycle logic.
// Concrete backends embed it and override subscribe/closeWatch/start.
#[derive(Default)]
pub(crate) struct watcherBase {
    pub(crate) mu: Mutex<watcherBaseState>,
    started: Mutex<bool>,
    started_cv: Condvar,
}

#[derive(Default)]
pub(crate) struct watcherBaseState {
    subscriptions: FxHashSet<dwKey>,
    start_err: Option<Error>,
}

impl watcherBase {
    // watcher.go:669
    pub(crate) fn notify_started(&self) {
        let mut started = self.started.lock().unwrap();
        if !*started {
            *started = true;
            self.started_cv.notify_all();
        }
    }

    // watcher.go:701
    fn handle_start_error(&self, err: &Error) {
        let mut st = self.mu.lock().unwrap();
        st.start_err = Some(err.clone());
        let subs: Vec<Arc<dirWatch>> = st.subscriptions.iter().map(|k| Arc::clone(&k.0)).collect();
        drop(st);
        for w in subs {
            w.notify_error(&err);
        }
        self.notify_started();
    }
}

// watcher.go:680
fn run(impl_: &Arc<dyn watcherImpl>) -> Result<(), Error> {
    let starter = Arc::clone(impl_);
    std::thread::Builder::new()
        .name("fswatch-backend".to_string())
        .spawn(move || {
            let base_owner = Arc::clone(&starter);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || starter.start()));
            match result {
                Ok(Ok(())) => {}
                Ok(Err(err)) => base_owner.base().handle_start_error(&err),
                Err(panic) => {
                    let msg = panic.downcast_ref::<String>().cloned().or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
                    base_owner.base().handle_start_error(&Error::new(msg));
                }
            }
        })
        .expect("failed to spawn fswatch backend thread");
    let base = impl_.base();
    let started = base.started.lock().unwrap();
    let _started = base.started_cv.wait_while(started, |s| !*s).unwrap();
    base.mu.lock().unwrap().start_err.clone().map_or(Ok(()), Err)
}

// watcher.go:713
pub(crate) fn watch_add(impl_: &Arc<dyn watcherImpl>, w: &Arc<dirWatch>) -> Result<(), Error> {
    watch_add_many(impl_, std::slice::from_ref(w))
}

// watcher.go:717
pub(crate) fn watch_add_many(impl_: &Arc<dyn watcherImpl>, watches: &[Arc<dirWatch>]) -> Result<(), Error> {
    let mut st = impl_.base().mu.lock().unwrap();
    let to_add: Vec<Arc<dirWatch>> = watches.iter().filter(|w| !st.subscriptions.contains(&dwKey(Arc::clone(*w)))).cloned().collect();
    if to_add.is_empty() {
        return Ok(());
    }

    if let Some(result) = impl_.subscribe_many(&to_add) {
        result?;
        for w in to_add {
            st.subscriptions.insert(dwKey(w));
        }
        return Ok(());
    }

    let mut added: Vec<Arc<dirWatch>> = Vec::with_capacity(to_add.len());
    for w in to_add {
        if let Err(err) = impl_.subscribe(&w) {
            for added_watch in &added {
                st.subscriptions.remove(&dwKey(Arc::clone(added_watch)));
                let _ = impl_.close_watch(added_watch);
            }
            return Err(err);
        }
        st.subscriptions.insert(dwKey(Arc::clone(&w)));
        added.push(w);
    }
    Ok(())
}

// watcher.go:764
pub(crate) fn watch_remove(impl_: &Arc<dyn watcherImpl>, w: &Arc<dirWatch>) {
    let mut st = impl_.base().mu.lock().unwrap();
    if !st.subscriptions.remove(&dwKey(Arc::clone(w))) {
        return;
    }
    let _ = impl_.close_watch(w);
}

// watcher.go:775
pub(crate) fn handle_watcher_error(impl_: &Arc<dyn watcherImpl>, werr: &dirWatchError) {
    watch_remove(impl_, &werr.dir_watch);
    let err = Error::wrap2(format!("{}: {}", ErrWatchTerminated.0, werr.err), &ErrWatchTerminated.into(), &werr.err);
    werr.dir_watch.notify_error(&err);
}

// ----- dirWatch: per-directory watch state -------------------------

#[derive(Clone)]
pub(crate) struct callback {
    id: u64,
    dir: String,
    physical_dir: String,
    watch_dir: String,
    watch_physical_dir: String,
    recursive: bool,
    f: WatchCallback,
    ignore: Option<IgnoreFunc>,
    since_seq: u64,
    terminal: Option<Error>,
    delivered: bool,
    comparer: pathComparer,
    dir_comparison: comparisonPath<'static>,
    physical_comparison: comparisonPath<'static>,
    file_comparison: comparisonPath<'static>,
}

// dirWatchError associates an error with a specific directory watch.
pub(crate) struct dirWatchError {
    pub(crate) err: Error,
    pub(crate) dir_watch: Arc<dirWatch>,
}

impl From<dirWatchError> for Error {
    fn from(e: dirWatchError) -> Error {
        e.err
    }
}

// dirWatch holds per-directory state: pending events, registered callbacks,
// and a reference to the shared debouncer. Each watched directory has one.
pub(crate) struct dirWatch {
    // dir is the caller-visible watch root used in delivered event paths.
    pub(crate) dir: String,
    // physicalDir is the path passed to OS watcher APIs. It differs from dir
    // when dir or an ancestor is a symlink or reparse point to a directory.
    pub(crate) physical_dir: String,
    pub(crate) recursive: bool,
    pub(crate) events: eventList,
    pub(crate) comparer: pathComparer,
    pub(crate) dir_fold: String,
    pub(crate) physical_dir_fold: String,

    // state stores per-directory platform-specific bookkeeping (fsevents, windows).
    pub(crate) state: Mutex<Option<Arc<dyn Any + Send + Sync>>>,
    // sequence returns a backend event sequence cutoff for new logical callbacks.
    sequence: Option<fn() -> u64>,

    mu: Mutex<dirWatchState>,
    key: usize,
}

struct dirWatchState {
    callbacks: Vec<callback>,
    debounce: Option<Arc<debounce>>,
    next_cb_id: u64,
}

static nextDirWatchKey: AtomicU64 = AtomicU64::new(1);

impl dirWatch {
    // watcher.go:834 (newDirWatch + setComparer + the fields getOrCreateDirWatch assigns before publishing it)
    pub(crate) fn new(dir: String, physical_dir: String, db: &Arc<debounce>, comparer: pathComparer, sequence: Option<fn() -> u64>, recursive: bool) -> Arc<dirWatch> {
        let dir_fold = comparer.prepare(&dir).folded;
        let physical_dir_fold = if physical_dir == dir { dir_fold.clone() } else { comparer.prepare(&physical_dir).folded };
        let dw = Arc::new(dirWatch {
            dir,
            physical_dir,
            recursive,
            events: eventList::default(),
            comparer,
            dir_fold,
            physical_dir_fold,
            state: Mutex::new(None),
            sequence,
            mu: Mutex::new(dirWatchState { callbacks: Vec::new(), debounce: Some(Arc::clone(&db)), next_cb_id: 0 }),
            key: nextDirWatchKey.fetch_add(1, Ordering::Relaxed) as usize,
        });
        let target = Arc::clone(&dw);
        db.add(dw.key, Arc::new(move || target.trigger_callbacks()));
        dw
    }

    // watcher.go:866
    // displayPath maps a physical event path back under the caller-visible
    // watch root.
    pub(crate) fn display_path(&self, watch_path: &str) -> String {
        rebase_path(watch_path, &self.physical_dir, &self.dir)
    }

    // watcher.go:871
    // physicalPath maps a caller-visible path to the physical watched root.
    pub(crate) fn physical_path(&self, display_path: &str) -> String {
        rebase_path(display_path, &self.dir, &self.physical_dir)
    }

    // watcher.go:916
    fn destroy_debounce(&self) {
        let db = self.mu.lock().unwrap().debounce.take();
        if let Some(db) = db {
            db.remove(self.key);
        }
    }

    // watcher.go:926
    pub(crate) fn notify(&self) {
        let st = self.mu.lock().unwrap();
        let has_pending_cbs = st.callbacks.iter().any(|cb| !cb.delivered);
        let has_terminal = st.callbacks.iter().any(|cb| cb.terminal.is_some() && !cb.delivered);
        let has_events = self.events.size() > 0;
        let has_error = self.events.has_error();
        let db = st.debounce.clone();
        drop(st);

        if has_pending_cbs && (has_events || has_error || has_terminal) {
            if let Some(db) = db {
                db.trigger();
            }
        }
    }

    // watcher.go:944
    pub(crate) fn notify_error(&self, err: &Error) {
        let cbs = std::mem::take(&mut self.mu.lock().unwrap().callbacks);
        for cb in cbs {
            (cb.f)(Vec::new(), Some(err.clone()));
        }
    }

    // watcher.go:954
    fn trigger_callbacks(&self) {
        let mut st = self.mu.lock().unwrap();
        let has_error = self.events.has_error();
        let has_events = self.events.size() > 0;
        let mut cbs: Vec<callback> = Vec::with_capacity(st.callbacks.len());
        let mut has_terminal = false;
        for cb in &st.callbacks {
            if cb.delivered {
                continue;
            }
            if cb.terminal.is_some() {
                has_terminal = true;
            }
            cbs.push(cb.clone());
        }
        if cbs.is_empty() {
            if has_events || has_error {
                let _ = self.events.drain();
            }
            return;
        }
        if !has_events && !has_error && !has_terminal {
            return;
        }
        let start_seqs: Vec<u64> = cbs.iter().map(|cb| cb.since_seq).collect();
        let (events_by_callback, err) = self.events.drain_for_sequences(&start_seqs);
        for cb in &cbs {
            if cb.terminal.is_none() {
                continue;
            }
            if let Some(c) = st.callbacks.iter_mut().find(|c| c.id == cb.id) {
                c.delivered = true;
            }
        }
        drop(st);

        let comparisons = Mutex::new(comparisonCache::default());
        for (i, cb) in cbs.iter().enumerate() {
            let mut cb_events = events_by_callback[i].clone();
            if cb.ignore.is_some() || !cb.recursive || cb.dir != self.dir || !cb.file_comparison.path.is_empty() {
                let mut filtered = Vec::with_capacity(cb_events.len());
                for e in cb_events {
                    let mut e = cb.map_event_cached(e, Some(&comparisons));
                    if !cb.file_comparison.path.is_empty() {
                        let mut path = comparisonPath { path: e.path.clone(), cache: Some(&comparisons), ..Default::default() };
                        match cb.comparer.suffix_prepared(&cb.file_comparison, &mut path) {
                            Some(suffix) if suffix.is_empty() => {}
                            _ => continue,
                        }
                        e.path.clone_from(&cb.file_comparison.path);
                    }
                    if let Some(ignore) = &cb.ignore {
                        if ignore(&e.path) {
                            continue;
                        }
                    }
                    if cb.dir != self.dir && !e.included_watch_root && e.path == cb.dir && e.kind == EventKind::Update {
                        continue;
                    }
                    if cb.recursive {
                        if cb.dir != self.dir && !is_in_directory_or_self(&cb.dir, &e.path) {
                            continue;
                        }
                    } else if !is_direct_child(&cb.dir, &e.path) && !(cb.dir != self.dir && e.path == cb.dir) {
                        continue;
                    }
                    filtered.push(e);
                }
                cb_events = filtered;
            }
            let cb_err = cb.terminal.clone().or_else(|| err.clone());
            if !cb_events.is_empty() || cb_err.is_some() {
                (cb.f)(cb_events, cb_err);
            }
        }
    }

    // watcher.go:1066
    pub(crate) fn terminate_callbacks_for_deleted_root(&self, path: &str, seq: u64, err: &Error) -> bool {
        let mut st = self.mu.lock().unwrap();
        let mut changed = false;
        let comparisons = Mutex::new(comparisonCache::default());
        let deleted = comparisonPath { path: path.to_string(), cache: Some(&comparisons), ..Default::default() };
        for cb in st.callbacks.iter_mut() {
            if cb.delivered || cb.terminal.is_some() || cb.since_seq >= seq {
                continue;
            }
            let physical_path = comparisonPath { path: cb.event_physical_path(path), cache: Some(&comparisons), ..Default::default() };
            let (mut dir, mut physical) = (cb.dir_comparison.clone(), cb.physical_comparison.clone());
            let logical_match = cb.comparer.suffix_prepared(&deleted, &mut dir).is_some();
            let physical_match = cb.comparer.suffix_prepared(&physical_path, &mut physical).is_some();
            if logical_match || physical_match {
                cb.terminal = Some(err.clone());
                changed = true;
            }
        }
        changed
    }

    // watcher.go:1124
    pub(crate) fn watch(&self, dir: &str, physical_dir: &str, recursive: bool, f: WatchCallback, ignore: Option<IgnoreFunc>) -> u64 {
        self.add_callback(dir, physical_dir, recursive, f, ignore, "")
    }

    // watcher.go:1128
    pub(crate) fn add_callback(&self, dir: &str, physical_dir: &str, recursive: bool, f: WatchCallback, ignore: Option<IgnoreFunc>, file: &str) -> u64 {
        let mut st = self.mu.lock().unwrap();
        st.next_cb_id += 1;
        let id = st.next_cb_id;
        let mut since_seq = self.events.sequence();
        if let Some(sequence) = self.sequence {
            since_seq = sequence();
        }
        st.callbacks.push(callback {
            id,
            dir: dir.to_string(),
            physical_dir: physical_dir.to_string(),
            watch_dir: self.dir.clone(),
            watch_physical_dir: self.physical_dir.clone(),
            recursive,
            f,
            ignore,
            since_seq,
            terminal: None,
            delivered: false,
            comparer: self.comparer,
            dir_comparison: self.comparer.prepare(dir),
            physical_comparison: self.comparer.prepare(physical_dir),
            file_comparison: self.comparer.prepare(file),
        });
        id
    }

    // watcher.go:1146
    pub(crate) fn unwatch(&self, id: u64) -> bool {
        let mut st = self.mu.lock().unwrap();
        if let Some(i) = st.callbacks.iter().position(|cb| cb.id == id) {
            st.callbacks.remove(i);
            return st.callbacks.is_empty();
        }
        false
    }

    // watcher.go:1158
    fn unref(self: &Arc<Self>, w: &watcher) {
        let empty = self.mu.lock().unwrap().callbacks.is_empty();
        if empty {
            w.remove_dir_watch(self);
        }
    }

    #[cfg(test)]
    pub(crate) fn trigger_callbacks_for_test(&self) {
        self.trigger_callbacks();
    }

    #[cfg(test)]
    pub(crate) fn destroy_debounce_for_test(&self) {
        self.destroy_debounce();
    }
}

impl callback {
    // watcher.go:1046
    pub(crate) fn map_event(&self, e: Event) -> Event {
        self.map_event_cached(e, None)
    }

    // watcher.go:1050
    fn map_event_cached(&self, mut e: Event, cache: Option<&Mutex<comparisonCache>>) -> Event {
        if !self.physical_dir.is_empty() && (self.physical_dir != self.dir || self.comparer.ignore_case) {
            let mut physical_path = comparisonPath { path: self.event_physical_path(&e.path), cache, ..Default::default() };
            let mut root = self.physical_comparison.clone();
            if root.path.is_empty() {
                root.path.clone_from(&self.physical_dir);
            }
            if let Some(path) = self.comparer.rebase_prepared(&mut physical_path, &root, &self.dir) {
                e.path = path;
            }
        }
        e
    }

    // watcher.go:1062
    fn event_physical_path(&self, path: &str) -> String {
        if !self.watch_physical_dir.is_empty() && !self.watch_dir.is_empty() && self.watch_physical_dir != self.watch_dir && is_in_directory_or_self(&self.watch_dir, path) {
            return rebase_path(path, &self.watch_dir, &self.watch_physical_dir);
        }
        path.to_string()
    }
}

// watcher.go:850
// physicalDirFor returns the physical path to watch for dir. If dir, or an
// ancestor of dir, is a symlink or reparse point, events are subscribed on its
// realpath while callbacks still use dir.
pub(crate) fn physical_dir_for(dir: &str) -> String {
    let Ok(realpath) = std::fs::canonicalize(dir) else {
        return dir.to_string();
    };
    let realpath = realpath.to_string_lossy().into_owned();
    if realpath == dir {
        return dir.to_string();
    }
    canonicalize_path(&filepath_clean(&realpath))
}

// watcher.go:878
// rebasePath replaces the from root in path with to, preserving any child
// suffix. Prefix matches must end at a path separator so sibling paths like
// "/foo2" are not rebased from "/foo".
pub(crate) fn rebase_path(path: &str, from: &str, to: &str) -> String {
    if from == to {
        return path.to_string();
    }
    if path == from {
        return to.to_string();
    }
    if !path.starts_with(from) {
        return path.to_string();
    }
    let suffix = &path[from.len()..];
    if from.ends_with('/') {
        return join_path_suffix(to, suffix);
    }
    if !suffix.starts_with('/') {
        return path.to_string();
    }
    join_path_suffix(to, suffix)
}

// watcher.go:898
pub(crate) fn join_path_suffix(root: &str, suffix: &str) -> String {
    if suffix.is_empty() {
        return root.to_string();
    }
    if let Some(rest) = suffix.strip_prefix('/') {
        if root.ends_with('/') {
            return format!("{}{}", root, rest);
        }
        return format!("{}{}", root, suffix);
    }
    if root.ends_with('/') {
        return format!("{}{}", root, suffix);
    }
    format!("{}/{}", root, suffix)
}

// watcher.go:1087
pub(crate) fn is_in_directory_or_self(dir: &str, path: &str) -> bool {
    if dir.is_empty() {
        return false;
    }
    if path == dir {
        return true;
    }
    if !path.starts_with(dir) {
        return false;
    }
    let rest = &path[dir.len()..];
    if rest.is_empty() {
        return false;
    }
    if dir.ends_with('/') {
        return true;
    }
    rest.starts_with('/')
}

// watcher.go:1108
// isDirectChild reports whether path is an immediate child of dir.
// Both paths must be absolute. Returns false for path == dir.
pub(crate) fn is_direct_child(dir: &str, path: &str) -> bool {
    if !path.starts_with(dir) {
        return false;
    }
    let rest = &path[dir.len()..];
    if rest.is_empty() {
        return false;
    }
    if !rest.starts_with('/') {
        return false;
    }
    let rest = &rest[1..];
    !rest.is_empty() && !rest.contains('/')
}

// Go `filepath.Clean` (Unix).
pub(crate) fn filepath_clean(path: &str) -> String {
    if path.is_empty() {
        return ".".to_string();
    }
    let rooted = path.starts_with('/');
    let mut out: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if out.last().is_some_and(|p| *p != "..") {
                    out.pop();
                } else if !rooted {
                    out.push("..");
                }
            }
            p => out.push(p),
        }
    }
    let joined = out.join("/");
    if rooted {
        format!("/{}", joined)
    } else if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    }
}

// Go `filepath.Dir` (Unix).
pub(crate) fn filepath_dir(path: &str) -> String {
    let i = path.rfind('/').map(|i| i + 1).unwrap_or(0);
    filepath_clean(&path[..i])
}
