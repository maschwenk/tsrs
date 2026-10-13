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

/// The default library directory of the CLI, the language server and the API server. Go's embedded build reads the
/// libraries from `bundled:///libs` and its `noembed` build (the npm `tsgo`) from its executable's directory
/// (bundled/noembed.go `libPath`). The npm
/// packages ship the libraries next to the binary, so a binary with a `lib.d.ts` beside it reads them from there like
/// the npm `tsgo`: file listings, diagnostics in lib files and tsbuildinfo then name the same real paths. Otherwise
/// (a `cargo` build) the embedded libraries. tsrs-only: `TSRS_LIB_PATH=<dir containing lib.d.ts>` overrides both.
pub fn default_lib_path() -> String {
    let override_dir = std::env::var("TSRS_LIB_PATH").ok().filter(|dir| !dir.is_empty());
    // Realpath'd like Go's executableDir, so a symlinked install (pnpm) names the store directory as the npm tsgo does.
    let exe_dir = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .ok()
        .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf));
    default_lib_path_from(override_dir, exe_dir)
}

pub(crate) fn default_lib_path_from(override_dir: Option<String>, exe_dir: Option<std::path::PathBuf>) -> String {
    if let Some(dir) = override_dir {
        return tspath::normalize_path(&dir);
    }
    match exe_dir {
        Some(dir) if dir.join("lib.d.ts").is_file() => tspath::normalize_path(&dir.to_string_lossy()),
        _ => lib_path(),
    }
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
