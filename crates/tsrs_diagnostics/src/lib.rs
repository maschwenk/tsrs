//! Diagnostic messages (Go `internal/diagnostics`).
//!
//! Every Go message variable is a `pub static` with the same identifier, re-exported at the crate
//! root: Go `diagnostics.Type_0_is_not_assignable_to_type_1` is
//! `tsrs_diagnostics::Type_0_is_not_assignable_to_type_1` (pass it as `&'static Message`).
//! `generated.rs` is produced by `tools/gen-diagnostics/gen.py`; do not edit it by hand.

mod diagnostics;
mod generated;

pub use diagnostics::*;
pub use generated::*;

#[cfg(test)]
mod tests;
