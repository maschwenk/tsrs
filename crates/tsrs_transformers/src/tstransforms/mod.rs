//! Package `tstransforms`: TypeScript-specific transforms (type erasure, import elision, runtime syntax).

use crate::*;

mod importelision;
mod runtimesyntax;
mod typeeraser;
mod utilities;

pub use importelision::*;
pub use runtimesyntax::*;
pub use typeeraser::*;
pub(crate) use utilities::*;
pub(crate) use runtimesyntax::get_innermost_module_declaration_from_dotted_module;
