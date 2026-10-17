use std::sync::LazyLock;

use tsrs_core::tspath;

use crate::FS;

pub(crate) fn wrap_fs<T: FS>(fs: T) -> T {
    fs
}

// bundled/noembed.go: libPath is evaluated once, including the executable realpath
// and the misplaced-executable check. Tests use the source libraries, like Go.
static LIB_PATH: LazyLock<String> = LazyLock::new(|| {
    if cfg!(test) {
        return super::testing_lib_path();
    }
    let exe = std::env::current_exe().and_then(std::fs::canonicalize).expect("bundled: failed to get executable path");
    let dir = exe.parent().expect("bundled: executable has no directory");
    let libdts = dir.join("lib.d.ts");
    assert!(libdts.exists(), "bundled: {} does not exist; this executable may be misplaced", libdts.display());
    tspath::normalize_slashes(&dir.to_string_lossy())
});

pub(crate) fn lib_path() -> String {
    LIB_PATH.clone()
}

// The suite runners still mount bundled paths when training a noembed compiler.
// Recognize their URI scheme without making any embedded contents reachable from the CLI.
pub fn is_bundled(path: &str) -> bool {
    super::embed::is_bundled(path)
}

pub fn embedded_file(_path: &str) -> Option<&'static str> {
    None
}
