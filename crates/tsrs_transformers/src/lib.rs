//! Port of `internal/transformers` (TypeScript 7): the script transformers that turn a TypeScript source file into
//! JavaScript before printing.
//!
//! Layout (one Rust file per Go file, one module per Go package):
//! - crate root: package `transformers` (destructuring.rs; transformer.go, chain.go, utilities.go and
//!   modifiervisitor.go are wave E1's, see todo_core.rs).
//! - `tstransforms`: typeeraser, importelision, runtimesyntax, utilities.
//! - `moduletransforms`: commonjsmodule, esmodule, externalmoduleinfo, impliedmodule, utilities.
//! - `estransforms`: usestrict.
//! - `inliners`: constenum.
//!
//! Transformers are arena objects (`P<XTransformer>`, `&self` methods, `Cell`/`RefCell` fields). Their visitor
//! closures capture the handle, exactly like `tsrs_declarations::DeclarationTransformer`. A factory returns the
//! embedded `Transformer` base as `P<Transformer>` (Go `*transformers.Transformer`).

pub(crate) use std::cell::{Cell, OnceCell, RefCell};
pub(crate) use std::rc::Rc;

pub(crate) use rustc_hash::{FxHashMap, FxHashSet};
pub(crate) use tsrs_ast::{self as ast, *};
pub(crate) use tsrs_core::collections::{MultiMap, OrderedSet, Set};
pub(crate) use tsrs_core::jsnum::{self, Number};
pub(crate) use tsrs_core::tspath;
pub(crate) use tsrs_core::{alloc_slice, alloc_str, alloc_vec, CompilerOptions, ModuleKind, ScriptKind, ScriptTarget, TextRange, Tristate, P};
pub(crate) use tsrs_printer::{self as printer, AutoGenerateOptions, EmitContext, EmitFlags, EmitHelper, GeneratedIdentifierFlags};
pub(crate) use tsrs_scanner as scanner;

mod todo_core;
pub use todo_core::*;

mod destructuring;
pub use destructuring::*;

pub mod estransforms;
pub mod inliners;
pub mod moduletransforms;
pub mod tstransforms;

#[cfg(test)]
mod tests;
