// fsevents_darwin.go: macOS FSEvents backend (event processing and stream lifecycle). The FFI plumbing is in
// fsevents_darwin_ffi.rs (see the deviation note there: the C callback classifies events on the stream's GCD
// queue thread instead of handing a payload to a goroutine through a pipe).
//
// Event classification (fsEventsCallback):
//   Each batch delivers arrays of paths, flags, and event IDs. The flags
//   bitmask may combine multiple states (created + modified + renamed).
//   Pure removes emit EventDelete with no syscalls. Renames and
//   remove+create combos do one Lstat to check existence (the kernel
//   reports some deletions as renames). Everything else emits EventUpdate
//   with no syscalls.
//
// Overflow:
//   flagMustScanSubDirs → ErrOverflow with detail (user/kernel/too-many).
//
// Root deletion:
//   Detected in the callback; the logical watch is marked terminated and
//   receives ErrWatchTerminated. The shared stream remains active for other
//   watches until the owner closes or reconciles the terminated watch.

use rustc_hash::{FxHashMap, FxHashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::fsevents_darwin_ffi::{create_and_start_stream, fs_events_get_current_event_id, fsEventStream, fsEventsCallbackPayload, new_stream_callback, streamCallback, streamStartError, teardown_stream};
use crate::pathcompare::comparisonPath;
use crate::watcher::{dirWatch, dwKey, err_watched_directory_removed, watcher, watcherBase, watcherImpl, Error, ErrOverflow};

// ----- FSEvents flag bits (from FSEvents.h) ------------------------------

const flagMustScanSubDirs: u32 = 0x00000001;
const flagUserDropped: u32 = 0x00000002;
const flagKernelDropped: u32 = 0x00000004;
const flagHistoryDone: u32 = 0x00000010;

const flagItemCreated: u32 = 0x00000100;
const flagItemRemoved: u32 = 0x00000200;
const flagItemInodeMetaMod: u32 = 0x00000400;
const flagItemRenamed: u32 = 0x00000800;
const flagItemModified: u32 = 0x00001000;
const flagItemFinderInfoMod: u32 = 0x00002000;
const flagItemChangeOwner: u32 = 0x00004000;
const flagItemXattrMod: u32 = 0x00008000;
const flagItemIsFile: u32 = 0x00010000;
const flagItemIsDir: u32 = 0x00020000;
const flagItemIsSymlink: u32 = 0x00040000;
const flagItemIsHardlink: u32 = 0x00100000;
const flagItemIsLastHardlink: u32 = 0x00200000;
const flagItemCloned: u32 = 0x00400000;

const ignoredFlags: u32 = flagItemIsHardlink | flagItemIsLastHardlink | flagItemIsSymlink | flagItemIsDir | flagItemIsFile | flagItemCloned;

#[derive(Default)]
pub(crate) struct fseventsState {
    terminated: AtomicBool,
}

pub(crate) struct fseventsStream {
    stream: Mutex<Option<fsEventStream>>,
    cb: Mutex<Option<Box<streamCallback>>>,
}

// ----- the watcherImpl -------------------------------------------------------

pub(crate) struct fsEventsBackend {
    base: watcherBase,
    mu: Mutex<fsEventsState>,
}

#[derive(Default)]
struct fsEventsState {
    watches: FxHashMap<dwKey, Arc<fseventsState>>,
    streams: Vec<Arc<fseventsStream>>,
}

// fsevents_darwin.go:161 (init)
pub(crate) fn init(w: &mut watcher) {
    w.factory = Some(|| Arc::new(new_fs_events_backend()) as Arc<dyn watcherImpl>);
    w.sequence = Some(fs_events_get_current_event_id);
}

// fsevents_darwin.go:166
fn new_fs_events_backend() -> fsEventsBackend {
    fsEventsBackend { base: watcherBase::default(), mu: Mutex::new(fsEventsState::default()) }
}

// fsevents_darwin.go:179
// checkWatcher mirrors the helper of the same name.
fn check_watcher(w: &Arc<dirWatch>) -> Result<(), Error> {
    let md = std::fs::metadata(&w.physical_dir).map_err(|e| Error::path_error("stat", &w.physical_dir, e.raw_os_error().unwrap_or(0)))?;
    if !md.is_dir() {
        return Err(Error::from_errno(libc::ENOTDIR));
    }
    Ok(())
}

fn err_fsevents_user_dropped() -> Error {
    Error::wrap(format!("events were dropped by the FSEvents client: {}", ErrOverflow.0), &ErrOverflow.into())
}
fn err_fsevents_kernel_dropped() -> Error {
    Error::wrap(format!("events were dropped by the kernel: {}", ErrOverflow.0), &ErrOverflow.into())
}
fn err_fsevents_too_many() -> Error {
    Error::wrap(format!("too many events: {}", ErrOverflow.0), &ErrOverflow.into())
}

fn stream_start_error(e: streamStartError) -> Error {
    match e {
        streamStartError::CFStringCreateNull => Error::new("CFStringCreate returned NULL"),
        streamStartError::CFArrayCreateNull => Error::new("CFArrayCreate returned NULL"),
        streamStartError::StreamCreateNull => Error::new("FSEventStreamCreate returned NULL"),
        streamStartError::StreamStartFailed => Error::new("error starting FSEvents stream"),
    }
}

const fseventsPathsPerStream: usize = 512;

#[derive(Clone)]
pub(crate) struct fseventsWatchSnapshot {
    pub(crate) w: Arc<dirWatch>,
    state: Arc<fseventsState>,
}

impl fsEventsState {
    // fsevents_darwin.go:215
    fn active_watches_locked(&self) -> Vec<fseventsWatchSnapshot> {
        let mut watches = Vec::with_capacity(self.watches.len());
        #[expect(clippy::iter_over_hash_type, reason = "stream paths are sorted after; each watch's dispatch only touches that watch's own state; Go ranges the map too")]
        for (w, state) in &self.watches {
            if state.terminated.load(Ordering::SeqCst) {
                continue;
            }
            watches.push(fseventsWatchSnapshot { w: Arc::clone(&w.0), state: Arc::clone(state) });
        }
        watches
    }
}

impl fsEventsBackend {
    // fsevents_darwin.go:226
    fn start_streams(&self, watches: &[fseventsWatchSnapshot]) -> Result<Vec<Arc<fseventsStream>>, Error> {
        start_fs_events_streams(watches, &|paths, watches| self.start_stream(paths, watches))
    }

    // fsevents_darwin.go:278
    // startStream creates and starts one FSEventStream watching all supplied paths.
    fn start_stream(&self, paths: &[String], watches: &[fseventsWatchSnapshot]) -> Result<Option<Arc<fseventsStream>>, Error> {
        if paths.is_empty() {
            return Ok(None);
        }
        let Some(mut cb) = new_stream_callback(watches) else {
            return Err(stream_start_error(streamStartError::StreamCreateNull));
        };
        let stream = create_and_start_stream(paths, &mut cb).map_err(stream_start_error)?;
        Ok(Some(Arc::new(fseventsStream { stream: Mutex::new(Some(stream)), cb: Mutex::new(Some(cb)) })))
    }
}

// fsevents_darwin.go:230
fn start_fs_events_streams(
    watches: &[fseventsWatchSnapshot],
    start_stream: &dyn Fn(&[String], &[fseventsWatchSnapshot]) -> Result<Option<Arc<fseventsStream>>, Error>,
) -> Result<Vec<Arc<fseventsStream>>, Error> {
    if watches.is_empty() {
        return Ok(Vec::new());
    }
    let mut seen = FxHashSet::with_capacity_and_hasher(watches.len(), Default::default());
    let mut paths = Vec::with_capacity(watches.len());
    for watch in watches {
        let path = watch.w.physical_dir.clone();
        if !seen.insert(path.clone()) {
            continue;
        }
        paths.push(path);
    }
    paths.sort();

    if let Ok(stream) = start_stream(&paths, watches) {
        return Ok(stream.into_iter().collect());
    }

    let mut streams = Vec::with_capacity(paths.len().div_ceil(fseventsPathsPerStream));
    let mut remaining_paths = &paths[..];
    while !remaining_paths.is_empty() {
        let chunk_len = remaining_paths.len().min(fseventsPathsPerStream);
        let chunk_paths = &remaining_paths[..chunk_len];
        match start_stream(chunk_paths, &watches_for_fs_events_paths(watches, chunk_paths)) {
            Ok(stream) => streams.extend(stream),
            Err(err) => {
                stop_fs_events_streams(&streams);
                return Err(err);
            }
        }
        remaining_paths = &remaining_paths[chunk_len..];
    }
    Ok(streams)
}

// fsevents_darwin.go:265
fn watches_for_fs_events_paths(watches: &[fseventsWatchSnapshot], paths: &[String]) -> Vec<fseventsWatchSnapshot> {
    if paths.is_empty() {
        return Vec::new();
    }
    watches.iter().filter(|watch| paths.binary_search(&watch.w.physical_dir).is_ok()).cloned().collect()
}

// fsevents_darwin.go:367
// The atomic Swap gates teardown so concurrent or repeated calls are safe:
// only the goroutine that observes a non-zero stream performs the cleanup.
fn stop_fs_events_streams(streams: &[Arc<fseventsStream>]) {
    for stream in streams {
        stop_fs_events_stream(stream);
    }
}

// fsevents_darwin.go:373
fn stop_fs_events_stream(state: &fseventsStream) {
    let Some(stream) = state.stream.lock().unwrap().take() else {
        return;
    };
    let mut cb = state.cb.lock().unwrap().take();
    teardown_stream(stream, cb.as_mut());
}

impl watcherImpl for fsEventsBackend {
    fn base(&self) -> &watcherBase {
        &self.base
    }

    // fsevents_darwin.go:174
    fn start(self: Arc<Self>) -> Result<(), Error> {
        self.base.notify_started();
        Ok(())
    }

    // fsevents_darwin.go:387
    // subscribe mirrors `fsEventsBackend::subscribe`.
    fn subscribe(&self, w: &Arc<dirWatch>) -> Result<(), Error> {
        self.subscribe_many(std::slice::from_ref(w)).unwrap()
    }

    // fsevents_darwin.go:391
    fn subscribe_many(&self, watches_to_add: &[Arc<dirWatch>]) -> Option<Result<(), Error>> {
        if watches_to_add.is_empty() {
            return Some(Ok(()));
        }
        let mut states: Vec<(Arc<dirWatch>, Arc<fseventsState>)> = Vec::with_capacity(watches_to_add.len());
        for w in watches_to_add {
            if let Err(err) = check_watcher(w) {
                return Some(Err(err));
            }
            states.push((Arc::clone(w), Arc::new(fseventsState::default())));
        }

        let mut st = self.mu.lock().unwrap();
        for (w, state) in &states {
            *w.state.lock().unwrap() = Some(Arc::<fseventsState>::clone(state));
            st.watches.insert(dwKey(Arc::clone(w)), Arc::clone(state));
        }
        let watches = st.active_watches_locked();
        drop(st);

        let streams = match self.start_streams(&watches) {
            Ok(streams) => streams,
            Err(err) => {
                let mut st = self.mu.lock().unwrap();
                for (w, state) in &states {
                    let key = dwKey(Arc::clone(w));
                    if st.watches.get(&key).is_some_and(|s| Arc::ptr_eq(s, state)) {
                        st.watches.remove(&key);
                        *w.state.lock().unwrap() = None;
                    }
                }
                return Some(Err(err));
            }
        };

        let mut st = self.mu.lock().unwrap();
        let old_streams = std::mem::replace(&mut st.streams, streams);
        drop(st);
        stop_fs_events_streams(&old_streams);
        Some(Ok(()))
    }

    // fsevents_darwin.go:437
    // closeWatch mirrors `fsEventsBackend::closeWatch`.
    fn close_watch(&self, w: &Arc<dirWatch>) -> Result<(), Error> {
        let state = w.state.lock().unwrap().take().and_then(|s| s.downcast::<fseventsState>().ok());
        let Some(state) = state else {
            return Ok(());
        };
        state.terminated.store(true, Ordering::SeqCst);

        let mut st = self.mu.lock().unwrap();
        st.watches.remove(&dwKey(Arc::clone(w)));
        let watches = st.active_watches_locked();
        drop(st);

        let streams = self.start_streams(&watches)?;

        let mut st = self.mu.lock().unwrap();
        let old_streams = std::mem::replace(&mut st.streams, streams);
        drop(st);
        stop_fs_events_streams(&old_streams);
        Ok(())
    }
}

// fsevents_darwin.go:467
// fsEventsCallback processes a batch of FSEvents (on the stream's serial dispatch queue thread).
pub(crate) fn fs_events_callback(cb: &streamCallback, payload: &fsEventsCallbackPayload) {
    if payload.is_empty() {
        return;
    }

    let num_events = payload.num_events;
    let watches = &cb.watches;
    let mut touched: Vec<Arc<dirWatch>> = Vec::new();
    let mut touched_set: FxHashSet<dwKey> = FxHashSet::default();
    let mut touch = |w: &Arc<dirWatch>| {
        if touched_set.insert(dwKey(Arc::clone(w))) {
            touched.push(Arc::clone(w));
        }
    };

    for i in 0..num_events {
        let flag = payload.flag(i);
        let event_id = payload.id(i);
        let path = payload.path_nfc(i);
        if path.is_empty() {
            continue;
        }
        let mut comparison = comparisonPath::new(&path);

        let is_removed = flag & flagItemRemoved != 0;
        let is_renamed = flag & flagItemRenamed != 0;
        let is_created = flag & flagItemCreated != 0;
        let is_done = flag & flagHistoryDone != 0;

        if flag & flagMustScanSubDirs != 0 {
            let overflow = if flag & flagUserDropped != 0 {
                err_fsevents_user_dropped()
            } else if flag & flagKernelDropped != 0 {
                err_fsevents_kernel_dropped()
            } else {
                err_fsevents_too_many()
            };
            for watch in watches {
                if watch.state.terminated.load(Ordering::SeqCst) {
                    continue;
                }
                if fsevents_overflow_matches_prepared(&watch.w, &mut comparison) {
                    watch.w.events.set_error(overflow.clone());
                    touch(&watch.w);
                }
            }
        }

        if is_done {
            break;
        }

        if flag & !ignoredFlags == 0 {
            continue;
        }

        let raw_path = &path;
        let mut path_exists = false;
        let mut path_exists_known = false;

        for watch in watches {
            if watch.state.terminated.load(Ordering::SeqCst) {
                continue;
            }
            let w = &watch.w;
            let Some(display_path) = fsevents_display_path_prepared(w, &mut comparison) else {
                continue;
            };

            // Skip events for the watched directory itself unless it's been
            // removed. fseventsd reports a change on the watched dir when a
            // child is added or removed; subscribers observe changes *within*
            // the directory, not the dir's own metadata churn.
            // (A removal of the dir is still propagated because Watcher
            // relies on it to tear down the stream.)
            if display_path == w.dir && !is_removed && !is_renamed {
                continue;
            }

            if is_removed && !is_created {
                if display_path == w.dir {
                    w.events.remove_watch_root_at(&display_path, event_id);
                } else {
                    w.events.remove_at(&display_path, event_id);
                }
                if w.terminate_callbacks_for_deleted_root(&display_path, event_id, &err_watched_directory_removed()) {
                    touch(w);
                }
                if display_path == w.dir {
                    watch.state.terminated.store(true, Ordering::SeqCst);
                    w.events.set_error(err_watched_directory_removed());
                }
            } else if is_renamed || (is_removed && is_created) {
                if !path_exists_known {
                    path_exists = std::fs::symlink_metadata(raw_path).is_ok();
                    path_exists_known = true;
                }
                if path_exists {
                    if display_path == w.dir {
                        w.events.update_watch_root_at(&display_path, event_id);
                    } else {
                        w.events.update_at(&display_path, event_id);
                    }
                } else {
                    if display_path == w.dir {
                        w.events.remove_watch_root_at(&display_path, event_id);
                    } else {
                        w.events.remove_at(&display_path, event_id);
                    }
                    if w.terminate_callbacks_for_deleted_root(&display_path, event_id, &err_watched_directory_removed()) {
                        touch(w);
                    }
                    if display_path == w.dir {
                        watch.state.terminated.store(true, Ordering::SeqCst);
                        w.events.set_error(err_watched_directory_removed());
                    }
                }
            } else if display_path == w.dir {
                w.events.update_watch_root_at(&display_path, event_id);
            } else {
                w.events.update_at(&display_path, event_id);
            }
            touch(w);
        }
    }

    for w in touched {
        w.notify();
    }
}

// fsevents_darwin.go:609
fn fsevents_display_path(w: &dirWatch, raw_path: &str) -> Option<String> {
    let mut path = comparisonPath::new(raw_path);
    fsevents_display_path_prepared(w, &mut path)
}

// fsevents_darwin.go:614
fn fsevents_display_path_prepared(w: &dirWatch, raw_path: &mut comparisonPath) -> Option<String> {
    let physical = comparisonPath { path: w.physical_dir.clone(), folded: w.physical_dir_fold.clone(), ready: !w.physical_dir_fold.is_empty(), cache: None };
    if let Some(path) = w.comparer.rebase_prepared(raw_path, &physical, &w.dir) {
        return Some(path);
    }
    if w.physical_dir != w.dir {
        let logical = comparisonPath { path: w.dir.clone(), folded: w.dir_fold.clone(), ready: !w.dir_fold.is_empty(), cache: None };
        return w.comparer.rebase_prepared(raw_path, &logical, &w.dir);
    }
    None
}

// fsevents_darwin.go:626
fn fsevents_overflow_matches(w: &dirWatch, raw_path: &str) -> bool {
    let mut path = comparisonPath::new(raw_path);
    fsevents_overflow_matches_prepared(w, &mut path)
}

// fsevents_darwin.go:631
fn fsevents_overflow_matches_prepared(w: &dirWatch, raw_path: &mut comparisonPath) -> bool {
    let mut physical = comparisonPath { path: w.physical_dir.clone(), folded: w.physical_dir_fold.clone(), ready: !w.physical_dir_fold.is_empty(), cache: None };
    if w.comparer.suffix_prepared(&physical, raw_path).is_some() {
        return true;
    }
    if w.comparer.suffix_prepared(raw_path, &mut physical).is_some() {
        return true;
    }
    if w.physical_dir != w.dir {
        let mut logical = comparisonPath { path: w.dir.clone(), folded: w.dir_fold.clone(), ready: !w.dir_fold.is_empty(), cache: None };
        if w.comparer.suffix_prepared(&logical, raw_path).is_some() {
            return true;
        }
        return w.comparer.suffix_prepared(raw_path, &mut logical).is_some();
    }
    false
}
