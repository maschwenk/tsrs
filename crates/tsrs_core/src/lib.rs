pub mod ptr;
pub use ptr::{alloc, alloc_slice, alloc_str, alloc_vec, P};

pub mod collections;
pub mod glob;
pub mod jsnum;
pub mod semver;
pub mod stringutil;
pub mod tspath;

mod compileroptions;
#[path = "core.rs"]
mod core_go;
mod languagevariant;
mod scriptkind;
mod text;
mod tristate;

pub use compileroptions::*;
pub use core_go::*;
pub use languagevariant::*;
pub use scriptkind::*;
pub use text::*;
pub use tristate::*;
