// canonicalize_darwin.go and canonicalize_other.go (selected by `cfg` like Go's build tags).

use crate::pathkey::PathComparer;
use crate::watcher::{watcher, Error};
use crate::pathcompare::pathComparer;

#[cfg(target_os = "macos")]
pub(crate) use crate::fsevents_darwin_ffi::{fold_native_path, nativePathFolding};

#[cfg(target_os = "macos")]
// canonicalize_darwin.go:15
// canonicalizePath normalizes watch keys, subscribed filenames, and incoming
// FSEvents paths to NFC. kqueue retains on-disk child spellings for its fd
// bookkeeping and directory events; on case-insensitive volumes, the native
// path comparer handles normalization differences when filtering WatchFile.
pub(crate) fn canonicalize_path(p: &str) -> String {
    crate::fsevents_darwin_ffi::normalize_nfc(p)
}

#[cfg(target_os = "macos")]
impl watcher {
    // canonicalize_darwin.go:17
    pub(crate) fn path_comparer(&self, dir: &str) -> Result<pathComparer, Error> {
        if self.name != "fsevents" && self.name != "kqueue" {
            return Ok(pathComparer::default());
        }
        let c = path_comparer_for_path(dir)?;
        Ok(c.comparer)
    }
}

#[cfg(target_os = "macos")]
// canonicalize_darwin.go:27
// path_comparer_for_path queries an existing path's volume. Errors are returned to
// the caller; a failed query must not silently enable or disable native folding.
pub fn path_comparer_for_path(path: &str) -> Result<PathComparer, Error> {
    // _PC_CASE_SENSITIVE from sys/unistd.h. Query the watched volume rather
    // than assuming every volume mounted on macOS is case-insensitive.
    const pcCaseSensitive: libc::c_int = 11;
    let cpath = std::ffi::CString::new(path).map_err(|_| Error::from_errno(libc::EINVAL))?;
    // SAFETY: `cpath` is a valid NUL-terminated string for the duration of the call.
    let sensitive = unsafe { libc::pathconf(cpath.as_ptr(), pcCaseSensitive) };
    if sensitive < 0 {
        let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        return Err(Error::path_error("pathconf", path, errno));
    }
    Ok(PathComparer { comparer: pathComparer { ignore_case: sensitive == 0 } })
}

#[cfg(not(target_os = "macos"))]
pub(crate) const nativePathFolding: bool = false;

#[cfg(not(target_os = "macos"))]
// canonicalize_other.go:7
pub(crate) fn fold_native_path(_: &str) -> String {
    panic!("fswatch: native path folding is only available on Darwin");
}

#[cfg(not(target_os = "macos"))]
// canonicalize_other.go:14
// canonicalizePath is a no-op on platforms whose watchers report paths
// using the same bytes the caller provided. See canonicalize_darwin.go
// for the rationale on macOS.
pub(crate) fn canonicalize_path(p: &str) -> String {
    p.to_string()
}

#[cfg(not(target_os = "macos"))]
impl watcher {
    // canonicalize_other.go:16
    pub(crate) fn path_comparer(&self, _dir: &str) -> Result<pathComparer, Error> {
        Ok(pathComparer::default())
    }
}

#[cfg(not(target_os = "macos"))]
// canonicalize_other.go:22
// path_comparer_for_path returns exact comparison on platforms without native
// Darwin watch aliases. It does not inspect the host filesystem.
pub fn path_comparer_for_path(_path: &str) -> Result<PathComparer, Error> {
    Ok(PathComparer::default())
}
