// The completion sort texts from Go's ls/completions.go that the tests reference (`ls.SortText*`). The `ls`
// package's completions are not ported yet; when they are, generated code should use them instead (this module
// is what `ls::` resolves to in tests/prelude.rs).

// completions.go:269 (`type SortText string`)
pub type SortText = String;

// completions.go:272
pub const SORT_TEXT_LOCAL_DECLARATION_PRIORITY: &str = "10";
pub const SORT_TEXT_LOCATION_PRIORITY: &str = "11";
pub const SORT_TEXT_OPTIONAL_MEMBER: &str = "12";
pub const SORT_TEXT_MEMBER_DECLARED_BY_SPREAD_ASSIGNMENT: &str = "13";
pub const SORT_TEXT_SUGGESTED_CLASS_MEMBERS: &str = "14";
pub const SORT_TEXT_GLOBALS_OR_KEYWORDS: &str = "15";
pub const SORT_TEXT_AUTO_IMPORT_SUGGESTIONS: &str = "16";
pub const SORT_TEXT_CLASS_MEMBER_SNIPPETS: &str = "17";
pub const SORT_TEXT_JAVASCRIPT_IDENTIFIERS: &str = "18";

// completions.go:283
pub fn deprecate_sort_text(original: &str) -> SortText {
    format!("z{original}")
}

// completions.go:287
pub fn object_literal_property_sort_text(preset_sort_text: &str, symbol_display_name: &str) -> SortText {
    format!("{preset_sort_text}\x00{symbol_display_name}\x00")
}

// completions.go:291
pub fn sort_below(original: &str) -> SortText {
    format!("{original}1")
}
