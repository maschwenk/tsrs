//! Opt-in resolver filesystem accounting. Build with `tsrs_vfs/resolver-fs-stats`, set
//! `TSRS_RESOLVER_FS_STATS=/absolute/report.json`, and pass `--extendedDiagnostics` to write a report.
//!
//! Physical rows count Rust filesystem API calls, not all kernel syscalls. In particular, a file read
//! or directory iteration can issue several syscalls, and `DirEntry::file_type` can need none. Rustix
//! rows count calls to its syscall wrappers, including explicit retries. Paths are recorded verbatim.

#[cfg(feature = "resolver-fs-stats")]
mod enabled;
#[cfg(feature = "resolver-fs-stats")]
pub use enabled::{cache, enabled, logical, physical, write_report};

#[cfg(not(feature = "resolver-fs-stats"))]
#[inline]
pub fn enabled() -> bool {
    false
}

#[cfg(not(feature = "resolver-fs-stats"))]
#[inline]
pub fn logical<T>(
    _site: &'static str,
    _operation: &'static str,
    _path: &str,
    f: impl FnOnce() -> T,
    _result: impl FnOnce(&T) -> &'static str,
) -> T {
    f()
}

#[cfg(not(feature = "resolver-fs-stats"))]
#[inline]
pub fn physical<T>(_operation: &'static str, _path: &str, f: impl FnOnce() -> T, _result: impl FnOnce(&T) -> &'static str) -> T {
    f()
}

#[cfg(not(feature = "resolver-fs-stats"))]
#[inline]
pub fn cache(_operation: &'static str, _outcome: &'static str) {}

#[cfg(not(feature = "resolver-fs-stats"))]
#[inline]
pub fn write_report() {}
