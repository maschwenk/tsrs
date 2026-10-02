// PLACEHOLDER for Go `internal/ls/autoimport` (auto-imports are phase 3; docs/LSP.md "Known gaps").
//
// Only the types and functions that `internal/project` and the phase-1 `ls` files call exist, with Go's
// signatures. They behave like an empty registry that is never prepared for any file: `Clone` returns an empty
// registry, `IsPreparedForImportingFile` is always false, `NodeModulesDirectories` is empty. Replace this module
// with the real port in phase 3.

mod registry;
mod view;
pub use registry::*;
pub use view::*;
