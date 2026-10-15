pub mod ast;
pub mod checkflags;
pub mod deepclone;
pub mod diagnostic;
pub mod flow;
pub mod functionflags;
pub mod generated;
pub mod identifier;
pub mod ids;
pub mod kind;
pub mod lazylist;
pub mod modifierflags;
pub mod nodeflags;
pub mod parseoptions;
pub mod positionmap;
#[cfg(feature = "ast-sizing")]
pub mod sizing; // TOOL, opt-in `--features tsrs_ast/ast-sizing` (notes/mem-compact-ast-sizing.md)
#[cfg(not(feature = "ast-sizing"))]
pub mod sizing {
    /// Sizing is compiled out unless the `ast-sizing` feature is enabled.
    #[inline]
    pub fn run(_phase: &str, _files: &[tsrs_core::P<crate::SourceFile>], _skip_freed_leaves: bool) {}
}
pub mod precedence;
pub mod subtreefacts;
pub mod symbol;
pub mod symbolflags;
pub mod tokenflags;
pub mod utilities_1;
pub mod utilities_2;
pub mod utilities_3;
pub mod visitor;

pub use ast::*;
pub use checkflags::*;
pub use diagnostic::*;
pub use flow::*;
pub use functionflags::*;
pub use generated::*;
pub use identifier::*;
pub use ids::*;
pub use kind::*;
pub use modifierflags::*;
pub use nodeflags::*;
pub use parseoptions::*;
pub use positionmap::*;
pub use precedence::*;
pub use subtreefacts::*;
pub use symbol::*;
pub use symbolflags::*;
pub use tokenflags::*;
pub use utilities_1::*;
pub use utilities_2::*;
pub use utilities_3::*;
pub use visitor::*;
