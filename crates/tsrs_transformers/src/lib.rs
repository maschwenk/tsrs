//! Port of `internal/transformers` (TypeScript 7): the transformer base (transformer.go, chain.go, utilities.go,
//! modifiervisitor.go), the script transformers that `compiler.getScriptTransformers` chains together
//! (`tstransforms`, `estransforms`, `moduletransforms`, `jsxtransforms`, `inliners`, one module per Go package and
//! one file per Go file; large Go files are split by line range into `<file>_N.rs`), and `Resolver`, the Rust form
//! of Go's `printer.EmitResolver` interface value.
//! The declaration transformer (`transformers/declarations`) lives in `tsrs_declarations`, which depends on this
//! crate and re-exports its base types.
//!
//! Handles (docs/EMIT.md section 10): a Go `*transformers.Transformer` is a `P<Transformer>` that points at the
//! `base` field of an arena-allocated concrete transformer (`P<XTransformer>`); the visit closure captures the
//! `P<XTransformer>`. Factories (`TransformerFactory`) are plain `fn(&TransformOptions) -> Option<P<Transformer>>`.

pub(crate) use std::cell::{Cell, OnceCell, RefCell};
pub(crate) use std::rc::Rc;

pub(crate) use rustc_hash::{FxHashMap, FxHashSet};
pub(crate) use tsrs_ast::{self as ast, *};
pub(crate) use tsrs_checker::{self as checker, Checker, CheckerSlot, EmitResolver, SymbolAccessibilityResult, SymbolTracker};
pub(crate) use tsrs_core::{alloc_slice, alloc_str, alloc_vec, CompilerOptions, ModuleKind, ScriptTarget, P, SP};
pub(crate) use tsrs_printer::{self as printer, EmitContext, EmitFlags};
pub(crate) use tsrs_scanner as scanner;
pub(crate) use tsrs_core::collections::{MultiMap, OrderedSet, OrderedSetExt, Set};
pub(crate) use tsrs_core::jsnum::{self, Number};
pub(crate) use tsrs_core::{tspath, TextRange};
pub(crate) use tsrs_printer::{AssignedNameOptions, AutoGenerateOptions, EmitHelper, GeneratedIdentifierFlags, NameOptions};

/// Go package `nodebuilder` (flags only).
pub mod nodebuilder {
    pub use tsrs_checker::{Flags, InternalFlags};
}

mod chain;
mod emithost;
mod modifiervisitor;
mod resolver;
mod transformer;
mod utilities;

pub use chain::*;
pub use emithost::*;
pub use modifiervisitor::*;
pub use resolver::*;
pub use transformer::*;
pub use utilities::*;

mod destructuring;

pub use destructuring::*;

pub mod estransforms;
pub mod inliners;
pub mod jsxtransforms;
pub mod moduletransforms;
pub mod tstransforms;

#[cfg(test)]
mod emit_test;
