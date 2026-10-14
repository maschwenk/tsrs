#[expect(clippy::module_inception, reason = "The module path preserves the upstream Go package and file layout.")]
mod glob;

pub use glob::*;
