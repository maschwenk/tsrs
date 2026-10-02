use super::try_remove_directory_prefix;

// TestTryRemoveDirectoryPrefixCaseFoldingShrinksPrefix reproduces a panic that used to occur
// when tryRemoveDirectoryPrefix confirmed a case-insensitive directory match via
// GetCanonicalFileName, then sliced the raw (non-canonicalized) path using the raw byte length
// of prefix. Each Kelvin sign 'K' below case-folds to the single-byte 'k', so the raw
// prefix is longer in bytes (15) than path (12), even though path's canonical form is
// case-insensitively prefixed by prefix's canonical form. Slicing path[len(prefix):] used to
// panic with "slice bounds out of range [15:12]"; tryRemoveDirectoryPrefix must instead trim by
// rune count via tspath.TrimFilePathPrefix.
// string_completions_test.go:17
#[test]
fn test_try_remove_directory_prefix_case_folding_shrinks_prefix() {
    let prefix = "/a/\u{212A}\u{212A}\u{212A}\u{212A}";
    let path = "/a/kkkk/x.ts";
    let actual = try_remove_directory_prefix(path, prefix, false /*useCaseSensitiveFileNames*/);
    let Some(actual) = actual else {
        panic!("expected a non-nil result");
    };
    assert_eq!(actual, "x.ts");
}
