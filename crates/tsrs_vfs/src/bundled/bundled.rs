use tsrs_core::tspath;

use super::embed::{self, WrappedFS};
use crate::FS;

// Embedded is true if the bundled files are implemented through an embedded FS.
pub const EMBEDDED: bool = embed::EMBEDDED;

// WrapFS returns an FS which redirects embedded paths to the embedded file system.
pub fn wrap_fs<T: FS>(fs: T) -> WrappedFS<T> {
    embed::wrap_fs(fs)
}

// LibPath returns the path to the directory containing the bundled lib.d.ts files.
pub fn lib_path() -> String {
    embed::lib_path()
}

// The embedded text of a bundled lib path without a copy (tsrs-only, see embed::embedded_file).
pub fn embedded_file(path: &str) -> Option<&'static str> {
    embed::embedded_file(path)
}

// TestingLibPath returns the path to the source bundled libs directory.
// It's only valid to use where the source code is available.
pub fn testing_lib_path() -> String {
    tspath::normalize_slashes(concat!(env!("CARGO_MANIFEST_DIR"), "/libs"))
}
