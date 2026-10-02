// Go internal/ls (+ lsconv, lsutil, change), astnav, format, sourcemap (docs/LSP.md).

mod api;
pub mod astnav;
pub mod autoimport;
mod completions;
mod constants;
mod crossproject;
mod definition;
mod diagnostics;
mod displaypartswriter;
mod findallreferences;
mod folding;
pub mod format;
mod host;
mod hover;
mod hovericon;
mod jsdoc;
mod languageservice;
#[cfg(test)]
mod ls_smoke_test;
mod lsformat;
pub mod lsconv;
pub mod lsutil;
pub mod sourcemap;
mod source_map;
mod sourcedefinition;
pub mod spanmap;
mod symbols;
mod utilities;

pub use api::{ERR_NO_SOURCE_FILE, ERR_NO_TOKEN_AT_POSITION};
pub use completions::{err_needs_auto_imports, is_err_needs_auto_imports, ERR_NEEDS_AUTO_IMPORTS};
pub use crossproject::*;
pub use host::*;
pub use jsdoc::{get_symbol_documentation_comment, get_symbol_jsdoc_tags, JSDocTagInfo};
pub use languageservice::*;
pub use symbols::provide_workspace_symbols;
pub use utilities::{is_in_string, range_contains_range};
