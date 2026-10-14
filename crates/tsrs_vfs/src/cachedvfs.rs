#[expect(clippy::module_inception, reason = "The module path preserves the upstream Go package and file layout.")]
mod cachedvfs;

pub use self::cachedvfs::*;
