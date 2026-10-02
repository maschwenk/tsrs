// Port of Go `internal/fswatch` (itself a Go port of @parcel/watcher): in-process filesystem watching with
// FSEvents and kqueue (macOS / BSD) and inotify (Linux). Not ported: `fanotify_linux.go` (Linux uses inotify, the
// same fallback Go takes on kernels without fanotify), `windows.go` (Windows builds get an unavailable watcher),
// `walkDirGeneric` (only used where the unix walker does not exist).

mod canonicalize;
mod debounce;
mod event;
#[cfg(target_os = "macos")]
mod fsevents_darwin;
#[cfg(target_os = "macos")]
mod fsevents_darwin_ffi;
#[cfg(target_os = "linux")]
mod inotify_linux;
#[cfg(any(target_os = "macos", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd", target_os = "dragonfly"))]
mod kqueue;
mod pathcompare;
mod pathkey;
#[cfg(unix)]
mod walkdir_unix;
mod watcher;

#[cfg(test)]
mod testutil_test;
#[cfg(test)]
mod eventlist_test;
#[cfg(test)]
mod watcher_test;

pub use canonicalize::path_comparer_for_path;
pub use event::{Event, EventKind};
pub use pathkey::{NativePathComparisonAvailable, PathComparer};
pub use watcher::{
    all_watchers, default, fanotify, fsevents, inotify, kqueue, windows, with_ignore, with_recursive, Error, ErrFilesystemUnsupported, ErrOverflow, ErrUnavailable, ErrWatchTerminated, IgnoreFunc, Sentinel, Watch, WatchCallback, WatchDirectoryRequest, WatchOption, Watcher,
};
