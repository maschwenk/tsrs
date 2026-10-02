// Go internal/ls (+ lsconv, lsutil, change), astnav, format, sourcemap (docs/LSP.md).

mod api;
pub mod astnav;
pub mod autoimport;
mod autoinsert;
mod callhierarchy;
mod codelens;
pub mod change;
mod codeactions;
mod codeactions_missingmemberfixer;
mod completions;
mod completions_2;
mod completions_3;
mod completions_4;
#[cfg(test)]
mod completions_smoke_test;
mod constants;
mod crossproject;
mod definition;
mod diagnostics;
mod displaypartswriter;
mod documenthighlights;
mod findallreferences;
mod folding;
#[cfg(test)]
mod findallreferences_test;
pub mod format;
mod host;
mod hover;
mod hovericon;
mod import_tracker;
mod inlay_hints;
mod jsdoc;
mod jsdoc_snippet;
mod languageservice;
mod linkedediting;
#[cfg(test)]
mod ls_smoke_test;
mod lsformat;
mod organizeimports;
#[cfg(test)]
mod refs_smoke_test;
mod rename;
pub mod lsconv;
pub mod lsutil;
pub mod sourcemap;
mod selectionranges;
mod semantictokens;
mod source_map;
mod signaturehelp;
mod sourcedefinition;
mod string_completions;
pub mod spanmap;
mod symbols;
mod utilities;

pub use api::{ERR_NO_SOURCE_FILE, ERR_NO_TOKEN_AT_POSITION};
pub use findallreferences::{Definition, DefinitionKind, ReferenceEntry, SignatureUsage, SymbolAndEntries, SymbolAndEntriesData};
pub use rename::{client_supports_document_changes, client_supports_rename_resource_operations, client_supports_will_rename_files, RenameInfo};
pub use codeactions::{CodeAction, CodeFixContext, CodeFixProvider, CombinedCodeActions};
pub use completions::{
    deprecate_sort_text, err_needs_auto_imports, is_err_needs_auto_imports, object_literal_property_sort_text, sort_below, CompletionItem, CompletionKind, CompletionList,
    KeywordCompletionFilters, SortText, COMPLETION_TRIGGER_CHARACTERS, ERR_NEEDS_AUTO_IMPORTS, SORT_TEXT_AUTO_IMPORT_SUGGESTIONS, SORT_TEXT_CLASS_MEMBER_SNIPPETS,
    SORT_TEXT_GLOBALS_OR_KEYWORDS, SORT_TEXT_JAVASCRIPT_IDENTIFIERS, SORT_TEXT_LOCAL_DECLARATION_PRIORITY, SORT_TEXT_LOCATION_PRIORITY,
    SORT_TEXT_MEMBER_DECLARED_BY_SPREAD_ASSIGNMENT, SORT_TEXT_OPTIONAL_MEMBER, SORT_TEXT_SUGGESTED_CLASS_MEMBERS,
};
pub use completions_3::{
    compare_completion_entries, SOURCE_CLASS_MEMBER_SNIPPET, SOURCE_OBJECT_LITERAL_MEMBER_WITH_COMMA, SOURCE_OBJECT_LITERAL_METHOD_SNIPPET, SOURCE_SWITCH_CASES,
    SOURCE_THIS_PROPERTY, SOURCE_TYPE_ONLY_ALIAS,
};
pub use crossproject::*;
pub use signaturehelp::{SIGNATURE_HELP_RETRIGGER_CHARACTERS, SIGNATURE_HELP_TRIGGER_CHARACTERS};
pub use host::*;
pub use jsdoc::{get_symbol_documentation_comment, get_symbol_jsdoc_tags, JSDocTagInfo};
pub use languageservice::*;
pub use semantictokens::semantic_tokens_legend;
pub use symbols::provide_workspace_symbols;
pub use utilities::{is_in_string, range_contains_range};
