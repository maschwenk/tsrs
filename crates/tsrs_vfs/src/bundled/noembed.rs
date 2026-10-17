use tsrs_core::tspath;

use crate::FS;

pub(crate) const EMBEDDED: bool = false;

pub type WrappedFS<T> = T;

pub(crate) fn wrap_fs<T: FS>(fs: T) -> WrappedFS<T> {
    fs
}

pub(crate) fn lib_path() -> String {
    if cfg!(test) {
        return super::testing_lib_path();
    }
    let exe = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .expect("could not get executable path");
    let dir = exe.parent().expect("executable has no directory");
    assert!(
        dir.join("lib.d.ts").is_file(),
        "bundled: {} does not exist; this executable may be misplaced",
        dir.join("lib.d.ts").display()
    );
    tspath::normalize_path(&dir.to_string_lossy())
}

// The suite runners still mount bundled paths when training a noembed compiler.
// Recognize their URI scheme without making any embedded contents reachable from the CLI.
pub fn is_bundled(path: &str) -> bool {
    super::embed::is_bundled(path)
}

pub fn embedded_file(_path: &str) -> Option<&'static str> {
    None
}
