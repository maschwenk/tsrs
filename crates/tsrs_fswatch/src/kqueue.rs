// kqueue.go: kqueue backend (macOS, FreeBSD, OpenBSD, NetBSD, DragonFlyBSD)
//
// Uses the kernel's kqueue/kevent mechanism to watch individual files and
// directories via EVFILT_VNODE. Unlike inotify, kqueue requires an open file
// descriptor per watched path, not just per directory. On macOS, O_EVTONLY
// opens files for event monitoring only; on other BSDs, O_RDONLY is used.
//
// Event dispatch (on the start thread):
//   - NOTE_WRITE on a directory → compareDir: re-read the directory from
//     disk, diff against the in-memory tree, emit update events for new
//     entries (opening + watching them) and delete events for removed ones
//     (closing their fds).
//   - NOTE_DELETE / NOTE_RENAME / NOTE_REVOKE → close the stale fd. For a
//     pure NOTE_DELETE on a file, tryRewatchLocked checks whether the path
//     was immediately recreated (atomic-save pattern) and emits update
//     instead of delete if so. Otherwise emit delete and remove from the
//     tree.
//   - NOTE_WRITE / NOTE_ATTRIB / NOTE_EXTEND on a file → emit update.
//
// Shutdown:
//   Write a byte to pipe[1] → kevent sees the pipe fd → loop exits →
//   close all tracked fds, the kqueue fd, and the pipe.
//
// Representation: Go's `*dirEntry` values are shared between a subscription's entries map and `fdToEntry`. Here
// every subscription's entries map lives in `kqState.entry_maps` (keyed by a map id; all subscriptions created by
// one `subscribe` share the map, as in Go), and `fd_to_entry` maps an fd to (map id, path). `compareDir` holds
// `mu` for its whole run (Go releases it around directory reads and re-acquires it in `watchPath`); only the
// event-loop thread mutates entry maps after `subscribe`, so this changes blocking, not results.

use rustc_hash::FxHashMap;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use crate::walkdir_unix::walk_dir;
use crate::watcher::{dirWatch, dwKey, err_watched_directory_removed, watcher, watcherBase, watcherImpl, Error};

// kqueue.go:85
// openForEvents opens a path for kqueue event monitoring. On darwin, O_EVTONLY
// opens the file for event notification without granting read access. On other
// BSDs, falls back to O_RDONLY.
fn open_for_events(path: &str) -> Result<i32, Error> {
    #[cfg(target_os = "macos")]
    let flags = libc::O_EVTONLY;
    #[cfg(not(target_os = "macos"))]
    let flags = libc::O_RDONLY;
    let cpath = std::ffi::CString::new(path).map_err(|_| Error::from_errno(libc::EINVAL))?;
    // SAFETY: `cpath` is NUL-terminated and outlives the call.
    let fd = unsafe { libc::open(cpath.as_ptr(), flags) };
    if fd < 0 {
        return Err(Error::from_io(std::io::Error::last_os_error()));
    }
    Ok(fd)
}

fn close_fd(fd: i32) {
    // SAFETY: closing an fd this backend opened and still tracks.
    unsafe { libc::close(fd) };
}

const vnodeFflags: u32 = libc::NOTE_DELETE | libc::NOTE_WRITE | libc::NOTE_EXTEND | libc::NOTE_ATTRIB | libc::NOTE_RENAME | libc::NOTE_REVOKE;

fn new_kevent(fd: i32, filter: i16, flags: u16, fflags: u32) -> libc::kevent {
    // SAFETY: `kevent` is a plain C struct; all-zero is a valid value.
    let mut ev: libc::kevent = unsafe { std::mem::zeroed() };
    ev.ident = fd as _;
    ev.filter = filter as _;
    ev.flags = flags as _;
    ev.fflags = fflags as _;
    ev
}

// Go `unix.Kevent(kq, changes, nil, nil)`.
fn kevent_register(kq: i32, ev: &libc::kevent) -> Result<(), Error> {
    // SAFETY: one valid change, no event list.
    let r = unsafe { libc::kevent(kq, ev, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
    if r < 0 {
        return Err(Error::from_io(std::io::Error::last_os_error()));
    }
    Ok(())
}

// dirEntry tracks a watched path for kqueue's fd↔path mapping.
#[derive(Clone)]
struct dirEntry {
    path: String,
    watch_path: String,
    is_dir: bool,
    state: Option<i32>, // stores the open fd
}

type entriesID = u64;

#[derive(Clone)]
struct kqueueSubscription {
    id: u64,
    dir_watch: Arc<dirWatch>,
    path: String,
    entries: entriesID,
    fd: i32,
}

#[derive(Default)]
struct kqState {
    subs_by_path: FxHashMap<String, Vec<kqueueSubscription>>, // multimap<path, sub>
    fd_to_entry: FxHashMap<i32, (entriesID, String)>,
    entry_maps: FxHashMap<entriesID, FxHashMap<String, dirEntry>>,
}

pub(crate) struct kqueueBackend {
    base: watcherBase,

    mu: Mutex<kqState>, // local lock for kqueue-specific maps
    kq: AtomicI32,
    // pipeFDs[0] is read in the start thread only. pipeFDs[1] is written
    // by shutdown (any thread) to wake the loop, so it lives in
    // pipeWriteFD with a sentinel of -1 once closed.
    pipe_read_fd: AtomicI32,
    pipe_write_fd: AtomicI32,
    ended: Mutex<bool>,
    ended_cv: Condvar,
    next_id: AtomicU64,
}

// kqueue.go:129 (init)
pub(crate) fn init(mut w: watcher) -> watcher {
    w.factory = Some(|| Arc::new(new_kqueue_backend()) as Arc<dyn watcherImpl>);
    w
}

// kqueue.go:133
fn new_kqueue_backend() -> kqueueBackend {
    kqueueBackend {
        base: watcherBase::default(),
        mu: Mutex::new(kqState::default()),
        kq: AtomicI32::new(-1),
        pipe_read_fd: AtomicI32::new(-1),
        pipe_write_fd: AtomicI32::new(-1),
        ended: Mutex::new(false),
        ended_cv: Condvar::new(),
        next_id: AtomicU64::new(1),
    }
}

impl kqueueBackend {
    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn run_loop(&self) -> Result<(), Error> {
        // SAFETY: plain syscall.
        let kq = unsafe { libc::kqueue() };
        if kq < 0 {
            return Err(Error::new(format!("unable to open kqueue: {}", std::io::Error::last_os_error())));
        }
        self.kq.store(kq, Ordering::SeqCst);

        let mut pipe_fds = [-1i32; 2];
        // SAFETY: `pipe_fds` has room for two fds.
        if unsafe { libc::pipe(pipe_fds.as_mut_ptr()) } < 0 {
            return Err(Error::new(format!("unable to open pipe: {}", std::io::Error::last_os_error())));
        }
        self.pipe_read_fd.store(pipe_fds[0], Ordering::SeqCst);
        self.pipe_write_fd.store(pipe_fds[1], Ordering::SeqCst);

        // Watch the read side of the pipe so we can break the loop on shutdown.
        let pipe_ev = new_kevent(pipe_fds[0], libc::EVFILT_READ, libc::EV_ADD | libc::EV_CLEAR, 0);
        if let Err(err) = kevent_register(kq, &pipe_ev) {
            return Err(Error::new(format!("unable to watch pipe: {}", err)));
        }

        self.base.notify_started();

        // SAFETY: zeroed kevent structs are valid.
        let mut events: Vec<libc::kevent> = vec![unsafe { std::mem::zeroed() }; 128];
        let mut watchers_touched: Vec<Arc<dirWatch>> = Vec::new();
        loop {
            // SAFETY: `events` has room for `events.len()` results.
            let n = unsafe { libc::kevent(kq, std::ptr::null(), 0, events.as_mut_ptr(), events.len() as i32, std::ptr::null()) };
            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                return Err(Error::new(format!("kevent error: {}", err)));
            }

            let mut stop = false;
            for ev in &events[..n as usize] {
                let mut fflags = ev.fflags as u32;
                let flags = ev.flags as u16;
                let fd = ev.ident as i32;
                if fd == pipe_fds[0] {
                    stop = true;
                    break;
                }

                // EV_ERROR indicates kevent couldn't apply a changelist
                // entry or that the kernel rejected the registration.
                if flags & libc::EV_ERROR != 0 {
                    continue;
                }

                let st = self.mu.lock().unwrap();
                let entry = st.fd_to_entry.get(&fd).and_then(|(map, path)| st.entry_maps.get(map).and_then(|m| m.get(path)).map(|e| (*map, e.clone())));
                drop(st);
                let Some((map_id, entry)) = entry else {
                    continue;
                };

                if fflags & libc::NOTE_WRITE != 0 && entry.is_dir {
                    self.compare_dir(fd, &entry.path, &mut watchers_touched);
                    // NOTE_WRITE on a dir already ran compareDir above.
                    // On DragonFlyBSD, rename-over coalesces NOTE_DELETE
                    // with NOTE_WRITE on the parent directory (rather than
                    // firing NOTE_DELETE on the replaced file's fd).
                    // Skip handleFileEvent so we don't misinterpret the
                    // coalesced NOTE_DELETE as the directory itself being
                    // removed.
                    fflags &= !libc::NOTE_DELETE;
                }
                if fflags & !libc::NOTE_WRITE != 0 || !entry.is_dir {
                    self.handle_file_event(fflags, map_id, &entry.path, &mut watchers_touched);
                }
            }

            for w in watchers_touched.drain(..) {
                w.notify();
            }
            if stop {
                break;
            }
        }
        Ok(())
    }

    // kqueue.go:219
    fn close_fds(&self) {
        let r = self.pipe_read_fd.swap(-1, Ordering::SeqCst);
        if r >= 0 {
            close_fd(r);
        }
        let w = self.pipe_write_fd.swap(-1, Ordering::SeqCst);
        if w >= 0 {
            close_fd(w);
        }
        let kq = self.kq.swap(-1, Ordering::SeqCst);
        if kq >= 0 {
            close_fd(kq);
        }
    }

    // kqueue.go:234
    fn close_subscriptions(&self) {
        let mut st = self.mu.lock().unwrap();
        let mut seen_fds = rustc_hash::FxHashSet::default();
        #[expect(clippy::iter_over_hash_type, reason = "closes each distinct fd once, then clears the maps; Go ranges the map too")]
        for list in st.subs_by_path.values() {
            for sub in list {
                if sub.fd < 0 {
                    continue;
                }
                if !seen_fds.insert(sub.fd) {
                    continue;
                }
                close_fd(sub.fd);
            }
        }
        st.subs_by_path = FxHashMap::default();
        st.fd_to_entry = FxHashMap::default();
    }

    // kqueue.go:266
    fn handle_file_event(&self, fflags: u32, map_id: entriesID, entry_path: &str, touched: &mut Vec<Arc<dirWatch>>) {
        let mut st = self.mu.lock().unwrap();
        let subs = st.find_subscriptions_locked(entry_path);
        let Some(entry) = st.entry_maps.get(&map_id).and_then(|m| m.get(entry_path)).cloned() else {
            return;
        };

        if fflags & (libc::NOTE_DELETE | libc::NOTE_RENAME | libc::NOTE_REVOKE) != 0 {
            // Close the stale fd; the watched inode is gone.
            if let Some(old_fd) = entry.state {
                close_fd(old_fd);
                st.fd_to_entry.remove(&old_fd);
                st.set_entry_state(map_id, entry_path, None);
            }

            let mut recreated = false;
            if fflags & libc::NOTE_DELETE != 0 && fflags & (libc::NOTE_RENAME | libc::NOTE_REVOKE) == 0 && !entry.is_dir {
                recreated = self.try_rewatch_locked(&mut st, map_id, &entry);
            }

            for sub in &subs {
                touch(touched, &sub.dir_watch);
                if recreated {
                    sub.dir_watch.events.update(&sub.path);
                } else {
                    sub.dir_watch.events.remove(&sub.path);
                    // If we lost a directory, walk the entries map and
                    // close every fd we had open for descendants. Some
                    // kernels (OpenBSD in particular) deliver only the
                    // parent's NOTE_DELETE/NOTE_RENAME and never fire
                    // NOTE_DELETE on the children; without this cleanup,
                    // modifying a file inside the moved tree later
                    // surfaces an event against the descendant's stale
                    // (pre-rename) path. We also emit a delete for each
                    // descendant we close, so callers don't miss those
                    // removals if the kernel didn't fire per-child events.
                    if entry.is_dir {
                        st.close_descendant_fds_locked(&sub.dir_watch, sub.entries, &sub.path);
                    }
                    if let Some(m) = st.entry_maps.get_mut(&sub.entries) {
                        remove_entry_and_descendants(m, &sub.path);
                    }
                    // Root-of-watch deletion: no more events can fire
                    // for this dirWatch. Tell the caller.
                    if sub.path == sub.dir_watch.dir {
                        sub.dir_watch.events.set_error(err_watched_directory_removed());
                    }
                }
            }
            if !recreated {
                st.subs_by_path.remove(entry_path);
            }
            return;
        }

        for sub in &subs {
            touch(touched, &sub.dir_watch);
            if fflags & (libc::NOTE_WRITE | libc::NOTE_ATTRIB | libc::NOTE_EXTEND) != 0 {
                sub.dir_watch.events.update(&sub.path);
            }
        }
    }

    // kqueue.go:357
    // tryRewatchLocked checks whether a deleted path was immediately recreated
    // with the same type. If so, it opens a new fd, registers a kqueue watch,
    // and returns true. The caller should emit update instead of delete.
    fn try_rewatch_locked(&self, st: &mut kqState, map_id: entriesID, entry: &dirEntry) -> bool {
        let Ok(md) = std::fs::symlink_metadata(&entry.watch_path) else {
            return false;
        };

        // Only fast-path when the recreated path has the same type;
        // a file→dir change needs a full tree rebuild via compareDir.
        let new_is_dir = md.is_dir();
        if new_is_dir != entry.is_dir {
            return false;
        }

        let Ok(fd) = open_for_events(&entry.watch_path) else {
            return false;
        };

        let ev = new_kevent(fd, libc::EVFILT_VNODE, libc::EV_ADD | libc::EV_CLEAR | libc::EV_ENABLE, vnodeFflags);
        if kevent_register(self.kq.load(Ordering::SeqCst), &ev).is_err() {
            close_fd(fd);
            return false;
        }

        st.set_entry_state(map_id, &entry.path, Some(fd));
        st.fd_to_entry.insert(fd, (map_id, entry.path.clone()));
        true
    }

    // kqueue.go:479
    // watchPath corresponds to `kqueueBackend::watchDir`.
    fn watch_path_locked(&self, st: &mut kqState, w: &Arc<dirWatch>, path: &str, map_id: entriesID) -> bool {
        let Some(entry) = st.entry_maps.get(&map_id).and_then(|m| m.get(path)).cloned() else {
            return false;
        };

        let fd = match entry.state {
            Some(fd) => fd,
            None => {
                let Ok(fd) = open_for_events(&entry.watch_path) else {
                    return false;
                };
                let ev = new_kevent(fd, libc::EVFILT_VNODE, libc::EV_ADD | libc::EV_CLEAR | libc::EV_ENABLE, vnodeFflags);
                if kevent_register(self.kq.load(Ordering::SeqCst), &ev).is_err() {
                    close_fd(fd);
                    return false;
                }
                st.set_entry_state(map_id, path, Some(fd));
                st.fd_to_entry.insert(fd, (map_id, path.to_string()));
                fd
            }
        };
        let sub = kqueueSubscription { id: self.next_id(), dir_watch: Arc::clone(w), path: path.to_string(), entries: map_id, fd };
        st.subs_by_path.entry(path.to_string()).or_default().push(sub);
        true
    }

    // kqueue.go:509
    // compareDir mirrors `kqueueBackend::compareDir`. Triggered when a watched
    // directory has NOTE_WRITE: list the dir, diff against the tree, emit
    // create/remove events.
    fn compare_dir(&self, _fd: i32, path: &str, touched: &mut Vec<Arc<dirWatch>>) -> bool {
        let mut st = self.mu.lock().unwrap();
        let subs = st.find_subscriptions_locked(path);

        // For non-recursive subscriptions, only compareDir on the root dir.
        // NOTE_WRITE on a child dir means something changed inside it, but
        // non-recursive mode shouldn't report those changes. Emit an update
        // for the child dir itself (its metadata changed) and return.
        let mut filtered_subs = Vec::new();
        for s in subs {
            if !s.dir_watch.recursive && path != s.dir_watch.dir {
                s.dir_watch.events.update(path);
                touch(touched, &s.dir_watch);
            } else {
                filtered_subs.push(s);
            }
        }
        if filtered_subs.is_empty() {
            return true;
        }
        let subs = filtered_subs;

        let dir_start = format!("{}/", path);
        struct diskSnapshot {
            entries: Vec<(String, bool)>,
            current_display_paths: rustc_hash::FxHashSet<String>,
        }
        let mut snapshots: FxHashMap<String, Arc<diskSnapshot>> = FxHashMap::default();

        // Each subscription has its own entries map (built in subscribe).
        // Multiple subs at the same path arise from multiple dirWatches
        // covering overlapping subtrees; their maps are always distinct, so
        // we iterate subs directly rather than trying to dedup by map identity.
        for sub in &subs {
            let Some(base_entry) = st.entry_maps.get(&sub.entries).and_then(|m| m.get(path)).cloned() else {
                continue;
            };
            let watch_path = base_entry.watch_path.clone();
            let watch_dir_start = format!("{}/", watch_path);

            let snapshot = match snapshots.get(&watch_path) {
                Some(s) => Arc::clone(s),
                None => {
                    let Ok(disk_entries) = read_entries(&watch_path) else {
                        continue;
                    };
                    let current_display_paths = disk_entries.iter().map(|(name, _)| format!("{}{}", dir_start, name)).collect();
                    let s = Arc::new(diskSnapshot { entries: disk_entries, current_display_paths });
                    snapshots.insert(watch_path.clone(), Arc::clone(&s));
                    s
                }
            };

            let map_id = sub.entries;
            for (name, ent_is_dir) in &snapshot.entries {
                let full_path = format!("{}{}", dir_start, name);
                let full_watch_path = format!("{}{}", watch_dir_start, name);

                let existing = st.entry_maps.get(&map_id).and_then(|m| m.get(&full_path)).cloned();
                if let Some(existing) = existing {
                    let mut existing_state = existing.state;
                    if let Some(fd) = existing.state {
                        // Check if the fd still refers to the same inode as
                        // the path on disk. On DragonFlyBSD, rename-over
                        // doesn't fire NOTE_DELETE on the replaced file's fd,
                        // leaving a stale entry whose fd points to the old
                        // (now unlinked) inode.
                        if let (Some(fd_st), Ok(path_st)) = (fstat(fd), std::fs::symlink_metadata(&full_watch_path)) {
                            use std::os::unix::fs::MetadataExt;
                            if fd_st.0 != path_st.dev() || fd_st.1 != path_st.ino() {
                                // Inode changed: path was replaced.
                                st.close_entry_locked(map_id, &full_path);
                                st.remove_subs_for_entries_locked(&full_path, sub.id);
                                if existing.is_dir {
                                    st.remove_entry_and_descendants_locked(map_id, &full_path, false, sub.id);
                                }
                                if let Some(e) = st.entry_maps.get_mut(&map_id).and_then(|m| m.get_mut(&full_path)) {
                                    e.is_dir = *ent_is_dir;
                                }
                                existing_state = None;
                            }
                        }
                    }
                    if existing_state.is_some() {
                        continue;
                    }
                    // Entry exists but fd is stale: the file was replaced.
                    // Re-watch it and emit an update.
                    if !self.watch_path_locked(&mut st, &sub.dir_watch, &full_path, map_id) {
                        continue;
                    }
                    sub.dir_watch.events.update(&full_path);
                    touch(touched, &sub.dir_watch);
                    if *ent_is_dir && sub.dir_watch.recursive {
                        self.walk_new_subtree_locked(&mut st, sub, map_id, &full_watch_path);
                    }
                    continue;
                }
                let e = dirEntry { path: full_path.clone(), watch_path: full_watch_path.clone(), is_dir: *ent_is_dir, state: None };
                st.entry_maps.entry(map_id).or_default().insert(full_path.clone(), e);
                if !self.watch_path_locked(&mut st, &sub.dir_watch, &full_path, map_id) {
                    if let Some(m) = st.entry_maps.get_mut(&map_id) {
                        m.remove(&full_path);
                    }
                    continue;
                }
                sub.dir_watch.events.create(&full_path);
                touch(touched, &sub.dir_watch);

                // For recursive subscriptions, walk into the new directory
                // to catch pre-populated subdirectories (e.g. a directory
                // tree moved into the watched area).
                if *ent_is_dir && sub.dir_watch.recursive {
                    self.walk_new_subtree_locked(&mut st, sub, map_id, &full_watch_path);
                }
            }

            // Detect removals: entries directly under dirStart that no longer
            // exist on disk.
            let mut to_remove = Vec::new();
            if let Some(m) = st.entry_maps.get(&map_id) {
                #[expect(clippy::iter_over_hash_type, reason = "stale children are removed from disjoint subtrees and from per-path event entries; Go ranges the map too")]
                for p in m.keys() {
                    let Some(rest) = p.strip_prefix(&dir_start) else {
                        continue;
                    };
                    if rest.contains('/') {
                        continue;
                    }
                    if snapshot.current_display_paths.contains(p) {
                        continue;
                    }
                    to_remove.push(p.clone());
                }
            }
            for p in to_remove {
                sub.dir_watch.events.remove(&p);
                touch(touched, &sub.dir_watch);
                let descendants: Vec<(String, Option<i32>)> = st
                    .entry_maps
                    .get(&map_id)
                    .map(|m| m.iter().filter(|(d, _)| *d == &p || is_descendant(d, &p)).map(|(d, e)| (d.clone(), e.state)).collect())
                    .unwrap_or_default();
                for (descendant, fd) in descendants {
                    if let Some(fd) = fd {
                        close_fd(fd);
                        st.fd_to_entry.remove(&fd);
                    }
                    st.subs_by_path.remove(&descendant);
                }
                if let Some(m) = st.entry_maps.get_mut(&map_id) {
                    remove_entry_and_descendants(m, &p);
                }
            }
        }
        true
    }

    // The `walkDir(fullWatchPath, true, ...)` closures of compareDir.
    fn walk_new_subtree_locked(&self, st: &mut kqState, sub: &kqueueSubscription, map_id: entriesID, full_watch_path: &str) {
        let mut found = Vec::new();
        let _ = walk_dir(full_watch_path, true, &mut |p, p_is_dir| {
            if p == full_watch_path {
                return Ok(()); // already handled above
            }
            found.push((p.to_string(), p_is_dir));
            Ok(())
        });
        for (p, p_is_dir) in found {
            let display_path = sub.dir_watch.display_path(&p);
            let e = dirEntry { path: display_path.clone(), watch_path: p, is_dir: p_is_dir, state: None };
            st.entry_maps.entry(map_id).or_default().insert(display_path.clone(), e);
            sub.dir_watch.events.create(&display_path);
            self.watch_path_locked(st, &sub.dir_watch, &display_path, map_id);
        }
    }
}

fn touch(touched: &mut Vec<Arc<dirWatch>>, w: &Arc<dirWatch>) {
    if !touched.iter().any(|t| Arc::ptr_eq(t, w)) {
        touched.push(Arc::clone(w));
    }
}

fn is_descendant(descendant: &str, path: &str) -> bool {
    descendant.len() > path.len() && descendant.as_bytes()[path.len()] == b'/' && descendant.starts_with(path)
}

// (dev, ino) of an open fd (Go `unix.Fstat`).
fn fstat(fd: i32) -> Option<(u64, u64)> {
    // SAFETY: zeroed `stat` is a valid out-parameter; `fd` is an fd this backend owns.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `st` is a live, writable `stat` out-parameter for the call; `fd` is an fd this backend owns.
    if unsafe { libc::fstat(fd, &raw mut st) } != 0 {
        return None;
    }
    Some((st.st_dev as u64, st.st_ino as u64))
}

impl kqState {
    fn set_entry_state(&mut self, map_id: entriesID, path: &str, state: Option<i32>) {
        if let Some(e) = self.entry_maps.get_mut(&map_id).and_then(|m| m.get_mut(path)) {
            e.state = state;
        }
    }

    // kqueue.go:337
    // closeDescendantFDsLocked closes every fd attached to an entry whose
    // path lives strictly under root, removing the kevent registration and
    // the corresponding b.subsByPath / b.fdToEntry bookkeeping, and emits a
    // delete event for each.
    fn close_descendant_fds_locked(&mut self, w: &Arc<dirWatch>, map_id: entriesID, root: &str) {
        let prefix = format!("{}/", root);
        let Some(m) = self.entry_maps.get_mut(&map_id) else {
            return;
        };
        let mut closed = Vec::new();
        #[expect(clippy::iter_over_hash_type, reason = "each matching entry is closed and removed once; event entries are per path; Go ranges the map too")]
        for (path, e) in m.iter_mut() {
            if !path.starts_with(&prefix) {
                continue;
            }
            if let Some(fd) = e.state.take() {
                close_fd(fd);
                closed.push(fd);
            }
            w.events.remove(path);
            self.subs_by_path.remove(path);
        }
        for fd in closed {
            self.fd_to_entry.remove(&fd);
        }
    }

    // kqueue.go:408
    fn close_entry_locked(&mut self, map_id: entriesID, path: &str) {
        let Some(e) = self.entry_maps.get_mut(&map_id).and_then(|m| m.get_mut(path)) else {
            return;
        };
        if let Some(fd) = e.state.take() {
            close_fd(fd);
            self.fd_to_entry.remove(&fd);
        }
    }

    // kqueue.go:416
    // Go compares `&sub.entries == entriesPtr`: the address of each subscription's own field, so only the
    // subscription the caller passes matches. That subscription is never listed under a different path, so this
    // removes nothing; it only drops an empty list (kept as Go).
    fn remove_subs_for_entries_locked(&mut self, path: &str, sub_id: u64) {
        let Some(list) = self.subs_by_path.get_mut(path) else {
            self.subs_by_path.remove(path);
            return;
        };
        list.retain(|s| s.id != sub_id);
        if list.is_empty() {
            self.subs_by_path.remove(path);
        }
    }

    // kqueue.go:432
    fn remove_entry_and_descendants_locked(&mut self, map_id: entriesID, path: &str, include_root: bool, sub_id: u64) {
        let paths: Vec<String> = self
            .entry_maps
            .get(&map_id)
            .map(|m| m.keys().filter(|d| if *d == path { include_root } else { is_descendant(d, path) }).cloned().collect())
            .unwrap_or_default();
        for descendant in paths {
            self.close_entry_locked(map_id, &descendant);
            self.remove_subs_for_entries_locked(&descendant, sub_id);
            if let Some(m) = self.entry_maps.get_mut(&map_id) {
                m.remove(&descendant);
            }
        }
    }

    // kqueue.go:449
    fn find_subscriptions_locked(&self, path: &str) -> Vec<kqueueSubscription> {
        self.subs_by_path.get(path).cloned().unwrap_or_default()
    }

    // kqueue.go:468
    // cleanupEntriesLocked closes fds for all entries that have been opened.
    // Called on subscribe failure to avoid fd leaks. Must be called under b.mu.
    fn cleanup_entries_locked(&mut self, entries: &mut FxHashMap<String, dirEntry>) {
        #[expect(clippy::iter_over_hash_type, reason = "closes every opened fd and drops its fd_to_entry key; Go ranges the map too")]
        for e in entries.values_mut() {
            if let Some(fd) = e.state.take() {
                close_fd(fd);
                self.fd_to_entry.remove(&fd);
            }
        }
    }
}

// kqueue.go:732
// readEntries lists directory entries (excluding "." and "..") at path. (Go `os.ReadDir` sorts by name.)
fn read_entries(path: &str) -> Result<Vec<(String, bool)>, Error> {
    let mut out = Vec::new();
    for ent in std::fs::read_dir(path).map_err(Error::from_io)? {
        let ent = ent.map_err(Error::from_io)?;
        let is_dir = ent.file_type().map(|t| t.is_dir()).unwrap_or(false);
        out.push((ent.file_name().to_string_lossy().into_owned(), is_dir));
    }
    out.sort();
    Ok(out)
}

// kqueue.go:774
// removeEntryAndDescendants removes path and all paths prefixed with
// path + separator from the entries map.
fn remove_entry_and_descendants(entries: &mut FxHashMap<String, dirEntry>, path: &str) {
    entries.remove(path);
    entries.retain(|k, _| !is_descendant(k, path));
}

impl watcherImpl for kqueueBackend {
    fn base(&self) -> &watcherBase {
        &self.base
    }

    // kqueue.go:146
    fn start(self: Arc<Self>) -> Result<(), Error> {
        let result = self.run_loop();
        self.close_subscriptions();
        self.close_fds();
        *self.ended.lock().unwrap() = true;
        self.ended_cv.notify_all();
        result
    }

    // kqueue.go:256
    fn shutdown(&self) {
        let fd = self.pipe_write_fd.load(Ordering::SeqCst);
        if fd < 0 {
            return;
        }
        // SAFETY: writing one byte from a live buffer to our own pipe.
        unsafe { libc::write(fd, b"X".as_ptr().cast::<libc::c_void>(), 1) };
        let ended = self.ended.lock().unwrap();
        let _ended = self.ended_cv.wait_while(ended, |e| !*e).unwrap();
    }

    // kqueue.go:455
    // subscribe mirrors `kqueueBackend::subscribe`. Called under watcherBase.mu
    // via watchAdd.
    fn subscribe(&self, w: &Arc<dirWatch>) -> Result<(), Error> {
        // Build the entries map without registering any watches or
        // subscriptions, so the event loop never sees a partially built map.
        let mut entries: FxHashMap<String, dirEntry> = FxHashMap::default();
        walk_dir(&w.physical_dir, w.recursive, &mut |watch_path, is_dir| {
            let path = w.display_path(watch_path);
            entries.insert(path.clone(), dirEntry { path, watch_path: watch_path.to_string(), is_dir, state: None });
            Ok(())
        })?;

        // Open fds, register kevents, and publish subscriptions under b.mu.
        let mut st = self.mu.lock().unwrap();
        let map_id = self.next_id();
        let kq = self.kq.load(Ordering::SeqCst);

        let paths: Vec<String> = entries.keys().cloned().collect();
        for path in paths {
            let watch_path = entries[&path].watch_path.clone();
            let fd = match open_for_events(&watch_path) {
                Ok(fd) => fd,
                Err(err) => {
                    if path == w.dir {
                        st.cleanup_entries_locked(&mut entries);
                        return Err(Error::wrap(format!("error watching {}: {}", w.dir, err), &err));
                    }
                    entries.remove(&path);
                    continue;
                }
            };
            let ev = new_kevent(fd, libc::EVFILT_VNODE, libc::EV_ADD | libc::EV_CLEAR | libc::EV_ENABLE, vnodeFflags);
            if let Err(err) = kevent_register(kq, &ev) {
                close_fd(fd);
                if path == w.dir {
                    st.cleanup_entries_locked(&mut entries);
                    return Err(Error::wrap(format!("error watching {}: {}", w.dir, err), &err));
                }
                entries.remove(&path);
                continue;
            }
            entries.get_mut(&path).unwrap().state = Some(fd);
            st.fd_to_entry.insert(fd, (map_id, path));
        }

        #[expect(clippy::iter_over_hash_type, reason = "one subscription per distinct path; subscription ids are only compared for equality; Go ranges the map too")]
        for (path, entry) in &entries {
            let fd = entry.state.unwrap();
            let sub = kqueueSubscription { id: self.next_id(), dir_watch: Arc::clone(w), path: path.clone(), entries: map_id, fd };
            st.subs_by_path.entry(path.clone()).or_default().push(sub);
        }
        st.entry_maps.insert(map_id, entries);
        Ok(())
    }

    // kqueue.go:737
    // closeWatch mirrors `kqueueBackend::closeWatch`.
    fn close_watch(&self, w: &Arc<dirWatch>) -> Result<(), Error> {
        let mut st = self.mu.lock().unwrap();
        let key = dwKey(Arc::clone(w));
        let mut maps_of_watch = rustc_hash::FxHashSet::default();
        let paths: Vec<String> = st.subs_by_path.keys().cloned().collect();
        for path in paths {
            let list = st.subs_by_path.get(&path).unwrap();
            let removed_any = list.iter().any(|s| dwKey(Arc::clone(&s.dir_watch)) == key);
            if !removed_any {
                continue;
            }
            for s in list.iter().filter(|s| dwKey(Arc::clone(&s.dir_watch)) == key) {
                maps_of_watch.insert(s.entries);
            }
            let first_fd = list[0].fd;
            let kept: Vec<kqueueSubscription> = list.iter().filter(|s| dwKey(Arc::clone(&s.dir_watch)) != key).cloned().collect();
            if kept.is_empty() {
                // Closing the file descriptor automatically unwatches it in kqueue.
                close_fd(first_fd);
                st.fd_to_entry.remove(&first_fd);
                st.subs_by_path.remove(&path);
            } else {
                st.subs_by_path.insert(path, kept);
            }
        }
        // Go drops the entries map with its last subscription (GC).
        #[expect(clippy::iter_over_hash_type, reason = "drops each entry map that no subscription references; the removals are independent")]
        for map_id in maps_of_watch {
            let still_used = st.subs_by_path.values().any(|l| l.iter().any(|s| s.entries == map_id));
            if !still_used {
                st.entry_maps.remove(&map_id);
            }
        }
        Ok(())
    }
}
