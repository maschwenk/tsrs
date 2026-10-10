use super::index::{Index, Named};

#[derive(Clone)]
struct testEntry {
    name: String,
    package_: String,
}

impl Named for testEntry {
    fn name(&self) -> &str {
        &self.name
    }
}

fn entry(name: &str, package_: &str) -> testEntry {
    testEntry { name: name.to_string(), package_: package_.to_string() }
}

// index_test.go:16
#[test]
fn test_index_clone_filters_entries_by_package() {
    let mut idx: Index<testEntry> = Index::default();
    idx.insert_as_words(entry("fooBar", "pkg-a"));
    idx.insert_as_words(entry("bazQux", "pkg-b"));
    idx.insert_as_words(entry("fooQux", "pkg-a"));

    // Clone excluding pkg-b
    let cloned = idx.clone_filtered(|e| e.package_ != "pkg-b");

    // Original should have all 3 entries
    assert_eq!(idx.entries.len(), 3);

    // Cloned should have 2 entries (only pkg-a)
    assert_eq!(cloned.entries.len(), 2);

    // Search should work on cloned index
    let results = cloned.find("fooBar", true);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "fooBar");

    // bazQux should not be in cloned index
    let results = cloned.find("bazQux", true);
    assert_eq!(results.len(), 0);

    // Word prefix search should work
    let results = cloned.search_word_prefix("foo");
    assert_eq!(results.len(), 2);
}

// index_test.go:51 ("handles nil index": a nil *Index is `Option::None`, which Go's Clone maps to nil)
#[test]
fn test_index_clone_handles_nil_index() {
    let idx: Option<Index<testEntry>> = None;
    let cloned = idx.as_ref().map(|i| i.clone_filtered(|_| true));
    assert!(cloned.is_none());
}

// index_test.go:59
#[test]
fn test_index_clone_handles_empty_index() {
    let idx: Index<testEntry> = Index::default();
    let cloned = idx.clone_filtered(|_| true);
    assert_eq!(cloned.entries.len(), 0);
}

// index_test.go:67
#[test]
fn test_index_clone_filters_all_entries() {
    let mut idx: Index<testEntry> = Index::default();
    idx.insert_as_words(entry("fooBar", "pkg-a"));
    idx.insert_as_words(entry("bazQux", "pkg-b"));

    let cloned = idx.clone_filtered(|_| false);
    assert_eq!(cloned.entries.len(), 0);
    assert_eq!(cloned.index_len(), 0);
}

// Not in Go: names are keyed by unicode.ToUpper of their first rune (simple mapping: U+1F80 -> U+1F88), so a
// case-insensitive lookup starting with U+1F80 finds a name starting with U+1F88. Results checked against tsgo.
#[test]
fn test_index_keys_by_simple_uppercase() {
    let mut idx: Index<testEntry> = Index::default();
    idx.insert_as_words(entry("\u{1F88}bc", "pkg"));
    idx.insert_as_words(entry("foo\u{1F80}bc", "pkg"));
    assert_eq!(idx.find("\u{1F80}bc", false).len(), 1);
    let results = idx.search_word_prefix("\u{1F80}");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "\u{1F88}bc");
}
