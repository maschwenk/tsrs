mod bundled;
mod embed;
mod embed_generated;
mod libs_generated;

pub use self::bundled::*;
pub use self::embed::{is_bundled, WrappedFS};
pub use self::libs_generated::*;

#[cfg(test)]
#[path = "bundled/bundled_test.rs"]
mod bundled_test;
