// Go internal/ls (+ lsconv, lsutil, change), astnav, format, sourcemap (docs/LSP.md).

pub mod astnav;
pub mod autoimport;
mod completions;
mod crossproject;
mod findallreferences;
pub mod format;
mod host;
mod languageservice;
mod lsformat;
pub mod lsconv;
pub mod lsutil;
pub mod sourcemap;
pub mod spanmap;
mod utilities;

pub use completions::{err_needs_auto_imports, is_err_needs_auto_imports, ERR_NEEDS_AUTO_IMPORTS};
pub use crossproject::*;
pub use host::*;
pub use languageservice::*;
pub use utilities::{is_in_string, range_contains_range};
