#[expect(clippy::module_inception, reason = "The module path preserves the upstream Go package and file layout.")]
mod bundled;
// The test runners use this explicitly, including when training noembed release builds.
// Its library data is unreferenced and removed by the linker in a noembed CLI.
pub mod embed;
mod embed_generated;
#[cfg(feature = "noembed")]
mod noembed;
mod libs_generated;

pub use self::bundled::*;
#[cfg(not(feature = "noembed"))]
pub use self::embed::{is_bundled, WrappedFS};
#[cfg(feature = "noembed")]
pub use self::noembed::{is_bundled, WrappedFS};
pub use self::libs_generated::*;

#[cfg(test)]
#[path = "bundled/bundled_test.rs"]
mod bundled_test;
