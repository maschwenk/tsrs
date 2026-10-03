//! Port of `internal/transformers` (TypeScript 7): the script transformers that run before the printer on the emit
//! path (docs/EMIT.md section 5). One module per Go package, one Rust file per Go file.
//!
//! This branch (`emit/jsx-decorators`) holds `jsxtransforms/jsx.go` and the legacy decorator transforms from
//! `tstransforms` (legacydecorators.go, metadata.go, typeserializer.go). The transformer base (`Transformer`,
//! `TransformOptions`, `Chain`) and the printer factory helpers belong to `emit/core`; the minimum this wave needs
//! is in `emit_stubs.rs` behind `TODO(emit/core)` until that branch lands.

pub(crate) use std::cell::{Cell, OnceCell, RefCell};
pub(crate) use std::rc::Rc;

pub(crate) use rustc_hash::{FxHashMap, FxHashSet};
pub(crate) use tsrs_ast::{self as ast, *};
pub(crate) use tsrs_core::collections::OrderedMap;
pub(crate) use tsrs_core::{alloc_slice, alloc_str, alloc_vec, CompilerOptions, ScriptTarget, TextRange, P};
pub(crate) use tsrs_printer::{self as printer, EmitContext, EmitFlags};
pub(crate) use tsrs_scanner as scanner;

mod emit_stubs;
pub use emit_stubs::*;

pub mod jsxtransforms;
pub mod tstransforms;

