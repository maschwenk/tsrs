use rustc_hash::FxHashMap;
use std::sync::Mutex;

use crate::watcher::Error;

// EventKind classifies a filesystem change.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct EventKind(pub i32);

impl EventKind {
    pub const Update: EventKind = EventKind(1);
    pub const Delete: EventKind = EventKind(2);
}

impl std::fmt::Display for EventKind {
    // event.go:13
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            EventKind::Update => f.write_str("update"),
            EventKind::Delete => f.write_str("delete"),
            _ => f.write_str("unknown"),
        }
    }
}

// Event describes a single filesystem change.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Event {
    pub kind: EventKind,
    pub path: String,
    pub(crate) included_watch_root: bool,
}

impl Event {
    // Go `fswatch.Event{Kind: k, Path: p}` (the unexported field stays zero).
    pub fn new(kind: EventKind, path: impl Into<String>) -> Event {
        Event { kind, path: path.into(), included_watch_root: false }
    }
}

// eventEntry tracks coalescing state during a debounce batch.
#[derive(Clone, Copy, Default)]
pub(crate) struct eventEntry {
    created_seq: u64,
    updated_seq: u64,
    deleted_seq: u64,
    included_watch_root: bool,
}

// eventList coalesces filesystem events by path within a debounce window.
//   - create after delete → update (rapid delete+recreate)
//   - getEvents skips entries that were both created and deleted
#[derive(Default)]
pub(crate) struct eventList {
    mu: Mutex<eventListState>,
}

#[derive(Default)]
struct eventListState {
    entries: FxHashMap<String, eventEntry>,
    err: Option<Error>,
    seq: u64,
}

impl eventList {
    // event.go:51
    // create records a new-file event for path. Both create and update
    // produce EventUpdate externally; sequence state tracks coalescing
    // (create+delete within a batch cancels out).
    #[cfg(any(unix, test))]
    pub(crate) fn create(&self, path: &str) {
        let mut el = self.mu.lock().unwrap();
        let seq = el.next_seq_locked();
        el.create_locked(path, seq);
    }

    // event.go:79
    // update records an update event for path.
    #[cfg(any(unix, test))]
    pub(crate) fn update(&self, path: &str) {
        let mut el = self.mu.lock().unwrap();
        let seq = el.next_seq_locked();
        el.update_locked(path, seq);
    }

    // event.go:86
    #[cfg(target_os = "macos")] // only the FSEvents backend reports its own sequence numbers
    pub(crate) fn update_at(&self, path: &str, seq: u64) {
        let mut el = self.mu.lock().unwrap();
        el.advance_seq_locked(seq);
        el.update_locked(path, seq);
    }

    // event.go:93
    #[cfg(target_os = "macos")] // only the FSEvents backend reports its own sequence numbers
    pub(crate) fn update_watch_root_at(&self, path: &str, seq: u64) {
        let mut el = self.mu.lock().unwrap();
        el.advance_seq_locked(seq);
        el.update_locked(path, seq);
        el.get_or_create(path).included_watch_root = true;
    }

    // event.go:106
    // remove records a delete event for path.
    #[cfg(any(unix, test))]
    pub(crate) fn remove(&self, path: &str) {
        let mut el = self.mu.lock().unwrap();
        let seq = el.next_seq_locked();
        el.remove_locked(path, seq);
    }

    // event.go:113
    #[expect(dead_code, reason = "its only Go caller, windows.go (windowsSubscription.processOne), is not ported")]
    pub(crate) fn remove_and_get_sequence(&self, path: &str) -> u64 {
        let mut el = self.mu.lock().unwrap();
        let seq = el.next_seq_locked();
        el.remove_locked(path, seq);
        seq
    }

    // event.go:121
    #[cfg(target_os = "macos")] // only the FSEvents backend reports its own sequence numbers
    pub(crate) fn remove_at(&self, path: &str, seq: u64) {
        let mut el = self.mu.lock().unwrap();
        el.advance_seq_locked(seq);
        el.remove_locked(path, seq);
    }

    // event.go:128
    #[cfg(target_os = "macos")] // only the FSEvents backend reports its own sequence numbers
    pub(crate) fn remove_watch_root_at(&self, path: &str, seq: u64) {
        let mut el = self.mu.lock().unwrap();
        el.advance_seq_locked(seq);
        el.remove_locked(path, seq);
        el.get_or_create(path).included_watch_root = true;
    }

    // event.go:142
    // size returns the number of tracked entries (including ones that may
    // cancel out in getEvents).
    pub(crate) fn size(&self) -> usize {
        self.mu.lock().unwrap().entries.len()
    }

    // event.go:168
    // getEvents returns a snapshot of events, skipping entries that were both
    // created and deleted. Order is not guaranteed.
    #[cfg(test)] // only tests call it, in Go too
    pub(crate) fn get_events(&self) -> Vec<Event> {
        self.mu.lock().unwrap().snapshot_locked()
    }

    // event.go:177
    // drain atomically snapshots all pending events and the stored error,
    // then clears the list. This prevents events added between a separate
    // getEvents+clear from being silently dropped.
    pub(crate) fn drain(&self) -> (Vec<Event>, Option<Error>) {
        let mut el = self.mu.lock().unwrap();
        let out = el.snapshot_locked();
        let err = el.err.take();
        el.entries = FxHashMap::default();
        (out, err)
    }

    // event.go:187
    #[cfg(test)] // only its ported test calls it: trigger_callbacks makes the snapshots outside the lock (`drain_entries`)
    pub(crate) fn drain_for_sequences(&self, start_seqs: &[u64]) -> (Vec<Vec<Event>>, Option<Error>) {
        let mut el = self.mu.lock().unwrap();
        let out = start_seqs.iter().map(|&start_seq| el.snapshot_since_locked(start_seq)).collect();
        let err = el.err.take();
        el.entries = FxHashMap::default();
        (out, err)
    }

    // tsrs-only: `drain_for_sequences` without the per-callback snapshots, which the caller makes outside the lock
    // (`dirWatch::trigger_callbacks`): the pending entries and the stored error, and the list cleared.
    pub(crate) fn drain_entries(&self) -> (Vec<(String, eventEntry)>, Option<Error>) {
        let mut el = self.mu.lock().unwrap();
        let entries = std::mem::take(&mut el.entries);
        let err = el.err.take();
        drop(el);
        // order-independent: one entry per path; Go ranges the map too ("Order is not guaranteed")
        let entries = entries.into_iter().collect();
        (entries, err)
    }

    // event.go:201
    // setError stores the first error encountered (later errors are ignored).
    #[cfg(any(unix, test))]
    pub(crate) fn set_error(&self, err: Error) {
        let mut el = self.mu.lock().unwrap();
        if el.err.is_none() {
            el.err = Some(err);
        }
    }

    // event.go:210
    // hasError reports whether an error has been recorded.
    pub(crate) fn has_error(&self) -> bool {
        self.mu.lock().unwrap().err.is_some()
    }

    // event.go:217
    // getError returns the stored error (or nil if none).
    #[cfg(test)] // only tests call it, in Go too
    pub(crate) fn get_error(&self) -> Option<Error> {
        self.mu.lock().unwrap().err.clone()
    }

    // event.go:235
    pub(crate) fn sequence(&self) -> u64 {
        self.mu.lock().unwrap().seq
    }
}

impl eventListState {
    // event.go:65
    #[cfg(any(unix, test))]
    fn create_locked(&mut self, path: &str, seq: u64) {
        let entry = self.get_or_create(path);
        if entry.is_deleted() {
            // Rapid delete+recreate: clear both flags so the entry
            // emits EventUpdate (the default for non-deleted entries).
            // https://github.com/parcel-bundler/watcher/issues/72
            entry.deleted_seq = 0;
            entry.created_seq = 0;
            entry.updated_seq = seq;
        } else {
            entry.created_seq = seq;
        }
    }

    // event.go:101
    #[cfg(any(unix, test))]
    fn update_locked(&mut self, path: &str, seq: u64) {
        self.get_or_create(path).updated_seq = seq;
    }

    // event.go:135
    fn remove_locked(&mut self, path: &str, seq: u64) {
        let entry = self.get_or_create(path);
        entry.deleted_seq = seq;
    }

    // event.go:150
    // snapshotLocked returns the current set of pending events with
    // create+delete pairs filtered out. Caller must hold el.mu.
    fn snapshot_locked(&self) -> Vec<Event> {
        self.snapshot_since_locked(0)
    }

    // event.go:154
    fn snapshot_since_locked(&self, start_seq: u64) -> Vec<Event> {
        let mut out = Vec::with_capacity(self.entries.len());
        // order-independent: entries are keyed by path, so a batch has at most one event per path, and Go ranges the map too (getEvents: "Order is not guaranteed")
        out.extend(self.entries.iter().filter_map(|(path, e)| {
            let Some(kind) = e.kind_since(start_seq) else {
                return None;
            };
            Some(Event { kind, path: path.clone(), included_watch_root: e.included_watch_root })
        }));
        out
    }

    // event.go:223
    fn get_or_create(&mut self, path: &str) -> &mut eventEntry {
        if !self.entries.contains_key(path) {
            self.entries.insert(path.to_string(), eventEntry::default());
        }
        self.entries.get_mut(path).unwrap()
    }

    // event.go:241
    fn next_seq_locked(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    // event.go:246
    #[cfg(target_os = "macos")] // only the FSEvents backend reports its own sequence numbers
    fn advance_seq_locked(&mut self, seq: u64) {
        if seq > self.seq {
            self.seq = seq;
        }
    }
}

impl eventEntry {
    // event.go:252
    #[cfg(any(unix, test))]
    fn is_deleted(&self) -> bool {
        self.deleted_seq > self.created_seq && self.deleted_seq > self.updated_seq
    }

    // event.go:256
    // tsrs-only: the event `snapshot_since_locked` makes of this entry.
    pub(crate) fn event_since(&self, path: &str, start_seq: u64) -> Option<Event> {
        let kind = self.kind_since(start_seq)?;
        Some(Event { kind, path: path.to_string(), included_watch_root: self.included_watch_root })
    }

    fn kind_since(&self, start_seq: u64) -> Option<EventKind> {
        if self.deleted_seq > start_seq {
            if self.created_seq > start_seq && self.created_seq < self.deleted_seq && self.updated_seq < self.deleted_seq {
                return None;
            }
            return Some(EventKind::Delete);
        }
        let seq = self.created_seq.max(self.updated_seq);
        if seq > start_seq {
            return Some(EventKind::Update);
        }
        None
    }
}
