// Go internal/ls (+ lsconv, lsutil, change), astnav, format, sourcemap (docs/LSP.md).

pub mod astnav;
pub mod autoimport;
mod completions;
mod crossproject;
mod definition;
mod diagnostics;
mod displaypartswriter;
mod findallreferences;
pub mod format;
mod host;
mod hover;
mod hovericon;
mod jsdoc;
mod languageservice;
mod lsformat;
pub mod lsconv;
pub mod lsutil;
pub mod sourcemap;
mod source_map;
mod sourcedefinition;
pub mod spanmap;
mod utilities;

pub use completions::{err_needs_auto_imports, is_err_needs_auto_imports, ERR_NEEDS_AUTO_IMPORTS};
pub use crossproject::*;
pub use host::*;
pub use jsdoc::{get_symbol_documentation_comment, get_symbol_jsdoc_tags, JSDocTagInfo};
pub use languageservice::*;
pub use utilities::{is_in_string, range_contains_range};
