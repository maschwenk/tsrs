//! Port of `internal/pseudochecker`: a limited "checker" that returns pseudo-"types" of expressions (mostly those
//! which trivially have type nodes). It has no dependency on the checker; the checker's node builder consumes it
//! (pseudotypenodebuilder.rs). `checker.go`, `type.go` (data model) and `lookup.go` are hand-ported (lookup.rs
//! was scaffolded as stubs from tools/gosig/pseudochecker.json; signatures in docs/sigs/pseudochecker*.txt).

mod checker;
mod lookup;
mod type_;

pub use checker::*;
pub use lookup::*;
pub use type_::*;
