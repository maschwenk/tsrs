// PLACEHOLDER for Go `internal/ls/autoimport` (auto-imports are phase 3; docs/LSP.md "Known gaps").
//
// Only the types and functions that `internal/project` and the phase-1 `ls` files call exist, with Go's
// signatures. They behave like a registry that never finds any export: `Clone` builds an empty bucket for the
// requested file's default project (so `IsPreparedForImportingFile` becomes true for that project, as after Go's
// `GetLanguageServiceWithAutoImports`), `View.GetCompletions` returns nothing, the import adder finds no fixes,
// `NodeModulesDirectories` is empty. Replace this module with the real port in phase 3.

mod export;
mod fix;
mod import_adder;
mod registry;
mod view;
pub use export::*;
pub use fix::*;
pub use import_adder::*;
pub use registry::*;
pub use view::*;
