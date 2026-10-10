use super::*;
use crate::tspath::*;

#[test]
fn test_normalize_slashes() {
    assert_eq!(normalize_slashes("a"), "a");
    assert_eq!(normalize_slashes("a/b"), "a/b");
    assert_eq!(normalize_slashes("a\\b"), "a/b");
    assert_eq!(normalize_slashes("\\\\server\\path"), "//server/path");
}

#[test]
fn test_get_root_length() {
    assert_eq!(get_root_length("a"), 0);
    assert_eq!(get_root_length("/"), 1);
    assert_eq!(get_root_length("/path"), 1);
    assert_eq!(get_root_length("c:"), 2);
    assert_eq!(get_root_length("c:d"), 0);
    assert_eq!(get_root_length("c:/"), 3);
    assert_eq!(get_root_length("c:\\"), 3);
    assert_eq!(get_root_length("//server"), 8);
    assert_eq!(get_root_length("//server/share"), 9);
    assert_eq!(get_root_length("\\\\server"), 8);
    assert_eq!(get_root_length("\\\\server\\share"), 9);
    assert_eq!(get_root_length("file:///"), 8);
    assert_eq!(get_root_length("file:///path"), 8);
    assert_eq!(get_root_length("file:///c:"), 10);
    assert_eq!(get_root_length("file:///c:d"), 8);
    assert_eq!(get_root_length("file:///c:/path"), 11);
    assert_eq!(get_root_length("file:///c%3a"), 12);
    assert_eq!(get_root_length("file:///c%3ad"), 8);
    assert_eq!(get_root_length("file:///c%3a/path"), 13);
    assert_eq!(get_root_length("file:///c%3A"), 12);
    assert_eq!(get_root_length("file:///c%3Ad"), 8);
    assert_eq!(get_root_length("file:///c%3A/path"), 13);
    assert_eq!(get_root_length("file://localhost"), 16);
    assert_eq!(get_root_length("file://localhost/"), 17);
    assert_eq!(get_root_length("file://localhost/path"), 17);
    assert_eq!(get_root_length("file://localhost/c:"), 19);
    assert_eq!(get_root_length("file://localhost/c:d"), 17);
    assert_eq!(get_root_length("file://localhost/c:/path"), 20);
    assert_eq!(get_root_length("file://localhost/c%3a"), 21);
    assert_eq!(get_root_length("file://localhost/c%3ad"), 17);
    assert_eq!(get_root_length("file://localhost/c%3a/path"), 22);
    assert_eq!(get_root_length("file://localhost/c%3A"), 21);
    assert_eq!(get_root_length("file://localhost/c%3Ad"), 17);
    assert_eq!(get_root_length("file://localhost/c%3A/path"), 22);
    assert_eq!(get_root_length("file://server"), 13);
    assert_eq!(get_root_length("file://server/"), 14);
    assert_eq!(get_root_length("file://server/path"), 14);
    assert_eq!(get_root_length("file://server/c:"), 14);
    assert_eq!(get_root_length("file://server/c:d"), 14);
    assert_eq!(get_root_length("file://server/c:/d"), 14);
    assert_eq!(get_root_length("file://server/c%3a"), 14);
    assert_eq!(get_root_length("file://server/c%3ad"), 14);
    assert_eq!(get_root_length("file://server/c%3a/d"), 14);
    assert_eq!(get_root_length("file://server/c%3A"), 14);
    assert_eq!(get_root_length("file://server/c%3Ad"), 14);
    assert_eq!(get_root_length("file://server/c%3A/d"), 14);
    assert_eq!(get_root_length("http://server"), 13);
    assert_eq!(get_root_length("http://server/path"), 14);
}

#[test]
fn test_path_is_absolute() {
    // POSIX
    assert_eq!(path_is_absolute("/path/to/file.ext"), true);
    // DOS
    assert_eq!(path_is_absolute("c:/path/to/file.ext"), true);
    // URL
    assert_eq!(path_is_absolute("file:///path/to/file.ext"), true);
    // Non-absolute
    assert_eq!(path_is_absolute("path/to/file.ext"), false);
    assert_eq!(path_is_absolute("./path/to/file.ext"), false);
}

#[test]
fn test_is_url() {
    assert_eq!(is_url("a"), false);
    assert_eq!(is_url("/"), false);
    assert_eq!(is_url("c:"), false);
    assert_eq!(is_url("c:d"), false);
    assert_eq!(is_url("c:/"), false);
    assert_eq!(is_url("c:\\"), false);
    assert_eq!(is_url("//server"), false);
    assert_eq!(is_url("//server/share"), false);
    assert_eq!(is_url("\\\\server"), false);
    assert_eq!(is_url("\\\\server\\share"), false);
    assert_eq!(is_url("file:///path"), true);
    assert_eq!(is_url("file:///c:"), true);
    assert_eq!(is_url("file:///c:d"), true);
    assert_eq!(is_url("file:///c:/path"), true);
    assert_eq!(is_url("file://server"), true);
    assert_eq!(is_url("file://server/path"), true);
    assert_eq!(is_url("http://server"), true);
    assert_eq!(is_url("http://server/path"), true);
}

#[test]
fn test_is_rooted_disk_path() {
    assert_eq!(is_rooted_disk_path("a"), false);
    assert_eq!(is_rooted_disk_path("/"), true);
    assert_eq!(is_rooted_disk_path("c:"), true);
    assert_eq!(is_rooted_disk_path("c:d"), false);
    assert_eq!(is_rooted_disk_path("c:/"), true);
    assert_eq!(is_rooted_disk_path("c:\\"), true);
    assert_eq!(is_rooted_disk_path("//server"), true);
    assert_eq!(is_rooted_disk_path("//server/share"), true);
    assert_eq!(is_rooted_disk_path("\\\\server"), true);
    assert_eq!(is_rooted_disk_path("\\\\server\\share"), true);
    assert_eq!(is_rooted_disk_path("file:///path"), false);
    assert_eq!(is_rooted_disk_path("file:///c:"), false);
    assert_eq!(is_rooted_disk_path("file:///c:d"), false);
    assert_eq!(is_rooted_disk_path("file:///c:/path"), false);
    assert_eq!(is_rooted_disk_path("file://server"), false);
    assert_eq!(is_rooted_disk_path("file://server/path"), false);
    assert_eq!(is_rooted_disk_path("http://server"), false);
    assert_eq!(is_rooted_disk_path("http://server/path"), false);
}

#[test]
fn test_get_directory_path() {
    assert_eq!(get_directory_path(""), "");
    assert_eq!(get_directory_path("a"), "");
    assert_eq!(get_directory_path("a/b"), "a");
    assert_eq!(get_directory_path("/"), "/");
    assert_eq!(get_directory_path("/a"), "/");
    assert_eq!(get_directory_path("/a/"), "/");
    assert_eq!(get_directory_path("/a/b"), "/a");
    assert_eq!(get_directory_path("/a/b/"), "/a");
    assert_eq!(get_directory_path("c:"), "c:");
    assert_eq!(get_directory_path("c:d"), "");
    assert_eq!(get_directory_path("c:/"), "c:/");
    assert_eq!(get_directory_path("c:/path"), "c:/");
    assert_eq!(get_directory_path("c:/path/"), "c:/");
    assert_eq!(get_directory_path("//server"), "//server");
    assert_eq!(get_directory_path("//server/"), "//server/");
    assert_eq!(get_directory_path("//server/share"), "//server/");
    assert_eq!(get_directory_path("//server/share/"), "//server/");
    assert_eq!(get_directory_path("\\\\server"), "//server");
    assert_eq!(get_directory_path("\\\\server\\"), "//server/");
    assert_eq!(get_directory_path("\\\\server\\share"), "//server/");
    assert_eq!(get_directory_path("\\\\server\\share\\"), "//server/");
    assert_eq!(get_directory_path("file:///"), "file:///");
    assert_eq!(get_directory_path("file:///path"), "file:///");
    assert_eq!(get_directory_path("file:///path/"), "file:///");
    assert_eq!(get_directory_path("file:///c:"), "file:///c:");
    assert_eq!(get_directory_path("file:///c:d"), "file:///");
    assert_eq!(get_directory_path("file:///c:/"), "file:///c:/");
    assert_eq!(get_directory_path("file:///c:/path"), "file:///c:/");
    assert_eq!(get_directory_path("file:///c:/path/"), "file:///c:/");
    assert_eq!(get_directory_path("file://server"), "file://server");
    assert_eq!(get_directory_path("file://server/"), "file://server/");
    assert_eq!(get_directory_path("file://server/path"), "file://server/");
    assert_eq!(get_directory_path("file://server/path/"), "file://server/");
    assert_eq!(get_directory_path("http://server"), "http://server");
    assert_eq!(get_directory_path("http://server/"), "http://server/");
    assert_eq!(get_directory_path("http://server/path"), "http://server/");
    assert_eq!(get_directory_path("http://server/path/"), "http://server/");
}

#[test]
fn test_remove_any_file_extension() {
    assert_eq!(remove_any_file_extension("/src/Component.vue"), "/src/Component");
    assert_eq!(remove_any_file_extension("/src/Component.d.ts"), "/src/Component");
    assert_eq!(remove_any_file_extension("/src/Component"), "/src/Component");
}

#[test]
fn test_get_path_components() {
    assert_eq!(get_path_components("", ""), vec![""]);
    assert_eq!(get_path_components("a", ""), vec!["", "a"]);
    assert_eq!(get_path_components("./a", ""), vec!["", ".", "a"]);
    assert_eq!(get_path_components("/", ""), vec!["/"]);
    assert_eq!(get_path_components("/a", ""), vec!["/", "a"]);
    assert_eq!(get_path_components("/a/", ""), vec!["/", "a"]);
    assert_eq!(get_path_components("c:", ""), vec!["c:"]);
    assert_eq!(get_path_components("c:d", ""), vec!["", "c:d"]);
    assert_eq!(get_path_components("c:/", ""), vec!["c:/"]);
    assert_eq!(get_path_components("c:/path", ""), vec!["c:/", "path"]);
    assert_eq!(get_path_components("//server", ""), vec!["//server"]);
    assert_eq!(get_path_components("//server/", ""), vec!["//server/"]);
    assert_eq!(get_path_components("//server/share", ""), vec!["//server/", "share"]);
    assert_eq!(get_path_components("file:///", ""), vec!["file:///"]);
    assert_eq!(get_path_components("file:///path", ""), vec!["file:///", "path"]);
    assert_eq!(get_path_components("file:///c:", ""), vec!["file:///c:"]);
    assert_eq!(get_path_components("file:///c:d", ""), vec!["file:///", "c:d"]);
    assert_eq!(get_path_components("file:///c:/", ""), vec!["file:///c:/"]);
    assert_eq!(get_path_components("file:///c:/path", ""), vec!["file:///c:/", "path"]);
    assert_eq!(get_path_components("file://server", ""), vec!["file://server"]);
    assert_eq!(get_path_components("file://server/", ""), vec!["file://server/"]);
    assert_eq!(get_path_components("file://server/path", ""), vec!["file://server/", "path"]);
    assert_eq!(get_path_components("http://server", ""), vec!["http://server"]);
    assert_eq!(get_path_components("http://server/", ""), vec!["http://server/"]);
    assert_eq!(get_path_components("http://server/path", ""), vec!["http://server/", "path"]);
}

#[test]
fn test_reduce_path_components() {
    assert_eq!(reduce_path_components(&vec![""].iter().map(|s| s.to_string()).collect::<Vec<_>>()), vec![""]);
    assert_eq!(reduce_path_components(&vec!["", "."].iter().map(|s| s.to_string()).collect::<Vec<_>>()), vec![""]);
    assert_eq!(reduce_path_components(&vec!["", ".", "a"].iter().map(|s| s.to_string()).collect::<Vec<_>>()), vec!["", "a"]);
    assert_eq!(reduce_path_components(&vec!["", "a", "."].iter().map(|s| s.to_string()).collect::<Vec<_>>()), vec!["", "a"]);
    assert_eq!(reduce_path_components(&vec!["", ".."].iter().map(|s| s.to_string()).collect::<Vec<_>>()), vec!["", ".."]);
    assert_eq!(reduce_path_components(&vec!["", "..", ".."].iter().map(|s| s.to_string()).collect::<Vec<_>>()), vec!["", "..", ".."]);
    assert_eq!(reduce_path_components(&vec!["", "..", ".", ".."].iter().map(|s| s.to_string()).collect::<Vec<_>>()), vec!["", "..", ".."]);
    assert_eq!(reduce_path_components(&vec!["", "a", ".."].iter().map(|s| s.to_string()).collect::<Vec<_>>()), vec![""]);
    assert_eq!(reduce_path_components(&vec!["", "..", "a"].iter().map(|s| s.to_string()).collect::<Vec<_>>()), vec!["", "..", "a"]);
    assert_eq!(reduce_path_components(&vec!["/"].iter().map(|s| s.to_string()).collect::<Vec<_>>()), vec!["/"]);
    assert_eq!(reduce_path_components(&vec!["/", "."].iter().map(|s| s.to_string()).collect::<Vec<_>>()), vec!["/"]);
    assert_eq!(reduce_path_components(&vec!["/", ".."].iter().map(|s| s.to_string()).collect::<Vec<_>>()), vec!["/"]);
    assert_eq!(reduce_path_components(&vec!["/", "a", ".."].iter().map(|s| s.to_string()).collect::<Vec<_>>()), vec!["/"]);
}

#[test]
fn test_combine_paths() {
    // Non-rooted
    assert_eq!(combine_paths("path", &["to", "file.ext"]), "path/to/file.ext");
    assert_eq!(combine_paths("path", &["dir", "..", "to", "file.ext"]), "path/dir/../to/file.ext");
    // POSIX
    assert_eq!(combine_paths("/path", &["to", "file.ext"]), "/path/to/file.ext");
    assert_eq!(combine_paths("/path", &["/to", "file.ext"]), "/to/file.ext");
    // DOS
    assert_eq!(combine_paths("c:/path", &["to", "file.ext"]), "c:/path/to/file.ext");
    assert_eq!(combine_paths("c:/path", &["c:/to", "file.ext"]), "c:/to/file.ext");
    // URL
    assert_eq!(combine_paths("file:///path", &["to", "file.ext"]), "file:///path/to/file.ext");
    assert_eq!(combine_paths("file:///path", &["file:///to", "file.ext"]), "file:///to/file.ext");
    assert_eq!(combine_paths("/", &["/node_modules/@types"]), "/node_modules/@types");
    assert_eq!(combine_paths("/a/..", &[""]), "/a/..");
    assert_eq!(combine_paths("/a/..", &["b"]), "/a/../b");
    assert_eq!(combine_paths("/a/..", &["b/"]), "/a/../b/");
    assert_eq!(combine_paths("/a/..", &["/"]), "/");
    assert_eq!(combine_paths("/a/..", &["/b"]), "/b");
}

#[test]
fn test_resolve_path() {
    assert_eq!(resolve_path("", &[]), "");
    assert_eq!(resolve_path(".", &[]), "");
    assert_eq!(resolve_path("./", &[]), "");
    assert_eq!(resolve_path("..", &[]), "..");
    assert_eq!(resolve_path("../", &[]), "../");
    assert_eq!(resolve_path("/", &[]), "/");
    assert_eq!(resolve_path("/.", &[]), "/");
    assert_eq!(resolve_path("/./", &[]), "/");
    assert_eq!(resolve_path("/../", &[]), "/");
    assert_eq!(resolve_path("/a", &[]), "/a");
    assert_eq!(resolve_path("/a/", &[]), "/a/");
    assert_eq!(resolve_path("/a/.", &[]), "/a");
    assert_eq!(resolve_path("/a/./", &[]), "/a/");
    assert_eq!(resolve_path("/a/./b", &[]), "/a/b");
    assert_eq!(resolve_path("/a/./b/", &[]), "/a/b/");
    assert_eq!(resolve_path("/a/..", &[]), "/");
    assert_eq!(resolve_path("/a/../", &[]), "/");
    assert_eq!(resolve_path("/a/../b", &[]), "/b");
    assert_eq!(resolve_path("/a/../b/", &[]), "/b/");
    assert_eq!(resolve_path("/a/..", &["b"]), "/b");
    assert_eq!(resolve_path("/a/..", &["/"]), "/");
    assert_eq!(resolve_path("/a/..", &["b/"]), "/b/");
    assert_eq!(resolve_path("/a/..", &["/b"]), "/b");
    assert_eq!(resolve_path("/a/.", &["b"]), "/a/b");
    assert_eq!(resolve_path("/a/.", &["."]), "/a");
    assert_eq!(resolve_path("a", &["b", "c"]), "a/b/c");
    assert_eq!(resolve_path("a", &["b", "/c"]), "/c");
    assert_eq!(resolve_path("a", &["b", "../c"]), "a/c");
}

#[test]
fn test_get_normalized_absolute_path() {
    assert_eq!(get_normalized_absolute_path("/", ""), "/");
    assert_eq!(get_normalized_absolute_path("/.", ""), "/");
    assert_eq!(get_normalized_absolute_path("/./", ""), "/");
    assert_eq!(get_normalized_absolute_path("/../", ""), "/");
    assert_eq!(get_normalized_absolute_path("/a", ""), "/a");
    assert_eq!(get_normalized_absolute_path("/a/", ""), "/a");
    assert_eq!(get_normalized_absolute_path("/a/.", ""), "/a");
    assert_eq!(get_normalized_absolute_path("/a/foo.", ""), "/a/foo.");
    assert_eq!(get_normalized_absolute_path("/a/./", ""), "/a");
    assert_eq!(get_normalized_absolute_path("/a/./b", ""), "/a/b");
    assert_eq!(get_normalized_absolute_path("/a/./b/", ""), "/a/b");
    assert_eq!(get_normalized_absolute_path("/a/..", ""), "/");
    assert_eq!(get_normalized_absolute_path("/a/../", ""), "/");
    assert_eq!(get_normalized_absolute_path("/a/../", ""), "/");
    assert_eq!(get_normalized_absolute_path("/a/../b", ""), "/b");
    assert_eq!(get_normalized_absolute_path("/a/../b/", ""), "/b");
    assert_eq!(get_normalized_absolute_path("/a/..", ""), "/");
    assert_eq!(get_normalized_absolute_path("/a/..", "/"), "/");
    assert_eq!(get_normalized_absolute_path("/a/..", "b/"), "/");
    assert_eq!(get_normalized_absolute_path("/a/..", "/b"), "/");
    assert_eq!(get_normalized_absolute_path("/a/.", "b"), "/a");
    assert_eq!(get_normalized_absolute_path("/a/.", "."), "/a");
    // Tests as above, but with backslashes.
    assert_eq!(get_normalized_absolute_path("\\", ""), "/");
    assert_eq!(get_normalized_absolute_path("\\.", ""), "/");
    assert_eq!(get_normalized_absolute_path("\\.\\", ""), "/");
    assert_eq!(get_normalized_absolute_path("\\..\\", ""), "/");
    assert_eq!(get_normalized_absolute_path("\\a\\.\\", ""), "/a");
    assert_eq!(get_normalized_absolute_path("\\a\\.\\b", ""), "/a/b");
    assert_eq!(get_normalized_absolute_path("\\a\\.\\b\\", ""), "/a/b");
    assert_eq!(get_normalized_absolute_path("\\a\\..", ""), "/");
    assert_eq!(get_normalized_absolute_path("\\a\\..\\", ""), "/");
    assert_eq!(get_normalized_absolute_path("\\a\\..\\", ""), "/");
    assert_eq!(get_normalized_absolute_path("\\a\\..\\b", ""), "/b");
    assert_eq!(get_normalized_absolute_path("\\a\\..\\b\\", ""), "/b");
    assert_eq!(get_normalized_absolute_path("\\a\\..", ""), "/");
    assert_eq!(get_normalized_absolute_path("\\a\\..", "\\"), "/");
    assert_eq!(get_normalized_absolute_path("\\a\\..", "b\\"), "/");
    assert_eq!(get_normalized_absolute_path("\\a\\..", "\\b"), "/");
    assert_eq!(get_normalized_absolute_path("\\a\\.", "b"), "/a");
    assert_eq!(get_normalized_absolute_path("\\a\\.", "."), "/a");
    // Relative paths on an empty currentDirectory.
    assert_eq!(get_normalized_absolute_path("", ""), "");
    assert_eq!(get_normalized_absolute_path(".", ""), "");
    assert_eq!(get_normalized_absolute_path("./", ""), "");
    // Strangely, these do not normalize to the empty string.
    assert_eq!(get_normalized_absolute_path("..", ""), "..");
    assert_eq!(get_normalized_absolute_path("../", ""), "..");
    // Interaction between relative paths and currentDirectory.
    assert_eq!(get_normalized_absolute_path("", "/home"), "/home");
    assert_eq!(get_normalized_absolute_path(".", "/home"), "/home");
    assert_eq!(get_normalized_absolute_path("./", "/home"), "/home");
    assert_eq!(get_normalized_absolute_path("..", "/home"), "/");
    assert_eq!(get_normalized_absolute_path("../", "/home"), "/");
    assert_eq!(get_normalized_absolute_path("a", "b"), "b/a");
    assert_eq!(get_normalized_absolute_path("a", "b/c"), "b/c/a");
    // Base names starting or ending with a dot do not affect normalization.
    assert_eq!(get_normalized_absolute_path(".a", ""), ".a");
    assert_eq!(get_normalized_absolute_path("..a", ""), "..a");
    assert_eq!(get_normalized_absolute_path("a.", ""), "a.");
    assert_eq!(get_normalized_absolute_path("a..", ""), "a..");
    assert_eq!(get_normalized_absolute_path("/base/./.a", ""), "/base/.a");
    assert_eq!(get_normalized_absolute_path("/base/../.a", ""), "/.a");
    assert_eq!(get_normalized_absolute_path("/base/./..a", ""), "/base/..a");
    assert_eq!(get_normalized_absolute_path("/base/../..a", ""), "/..a");
    assert_eq!(get_normalized_absolute_path("/base/./..a/b", ""), "/base/..a/b");
    assert_eq!(get_normalized_absolute_path("/base/../..a/b", ""), "/..a/b");
    assert_eq!(get_normalized_absolute_path("/base/./a.", ""), "/base/a.");
    assert_eq!(get_normalized_absolute_path("/base/../a.", ""), "/a.");
    assert_eq!(get_normalized_absolute_path("/base/./a..", ""), "/base/a..");
    assert_eq!(get_normalized_absolute_path("/base/../a..", ""), "/a..");
    assert_eq!(get_normalized_absolute_path("/base/./a../b", ""), "/base/a../b");
    assert_eq!(get_normalized_absolute_path("/base/../a../b", ""), "/a../b");
    assert_eq!(get_normalized_absolute_path("a/..", ""), "");
    assert_eq!(get_normalized_absolute_path("/a//", ""), "/a");
    assert_eq!(get_normalized_absolute_path("//a", "a"), "//a/");
    assert_eq!(get_normalized_absolute_path("/\\", ""), "//");
    assert_eq!(get_normalized_absolute_path("a///", "a"), "a/a");
    assert_eq!(get_normalized_absolute_path("/.//", ""), "/");
    assert_eq!(get_normalized_absolute_path("//\\\\", ""), "///");
    assert_eq!(get_normalized_absolute_path(".//a", "."), "a");
    assert_eq!(get_normalized_absolute_path("a/../..", ""), "..");
    assert_eq!(get_normalized_absolute_path("../..", "\\a"), "/");
    assert_eq!(get_normalized_absolute_path("a:", "b"), "a:/");
    assert_eq!(get_normalized_absolute_path("a/../..", ".."), "../..");
    assert_eq!(get_normalized_absolute_path("a/../..", "b"), "");
    assert_eq!(get_normalized_absolute_path("a//../..", ".."), "../..");
    // Consecutive intermediate slashes are normalized to a single slash.
    assert_eq!(get_normalized_absolute_path("a//b", ""), "a/b");
    assert_eq!(get_normalized_absolute_path("a///b", ""), "a/b");
    assert_eq!(get_normalized_absolute_path("a/b//c", ""), "a/b/c");
    assert_eq!(get_normalized_absolute_path("/a/b//c", ""), "/a/b/c");
    assert_eq!(get_normalized_absolute_path("//a/b//c", ""), "//a/b/c");
    // Backslashes are converted to slashes,
    // and then consecutive intermediate slashes are normalized to a single slash
    assert_eq!(get_normalized_absolute_path("a\\\\b", ""), "a/b");
    assert_eq!(get_normalized_absolute_path("a\\\\\\b", ""), "a/b");
    assert_eq!(get_normalized_absolute_path("a\\b\\\\c", ""), "a/b/c");
    assert_eq!(get_normalized_absolute_path("\\a\\b\\\\c", ""), "/a/b/c");
    assert_eq!(get_normalized_absolute_path("\\\\a\\b\\\\c", ""), "//a/b/c");
    // The same occurs for mixed slashes.
    assert_eq!(get_normalized_absolute_path("a/\\b", ""), "a/b");
    assert_eq!(get_normalized_absolute_path("a\\/b", ""), "a/b");
    assert_eq!(get_normalized_absolute_path("a\\/\\b", ""), "a/b");
    assert_eq!(get_normalized_absolute_path("a\\b//c", ""), "a/b/c");
    assert_eq!(get_normalized_absolute_path("\\a\\b\\\\c", ""), "/a/b/c");
    assert_eq!(get_normalized_absolute_path("\\\\a\\b\\\\c", ""), "//a/b/c");
}

#[test]
fn test_get_normalized_absolute_path_without_root() {
    assert_eq!(get_normalized_absolute_path_without_root("/a/b/c.txt", "/a/b"), "a/b/c.txt");
    assert_eq!(get_normalized_absolute_path_without_root("c:/work/hello.txt", "c:/work"), "work/hello.txt");
    assert_eq!(get_normalized_absolute_path_without_root("c:/work/hello.txt", "d:/worspaces"), "work/hello.txt");
}

#[test]
fn test_get_relative_path_to_directory_or_url() {
    // !!!
    // Based on tests for `getRelativePathFromDirectory`.
    assert_eq!(get_relative_path_to_directory_or_url("/", "/", false, &ComparePathsOptions::default()), "");
    assert_eq!(get_relative_path_to_directory_or_url("/a", "/a", false, &ComparePathsOptions::default()), "");
    assert_eq!(get_relative_path_to_directory_or_url("/a/", "/a", false, &ComparePathsOptions::default()), "");
    assert_eq!(get_relative_path_to_directory_or_url("/a", "/", false, &ComparePathsOptions::default()), "..");
    assert_eq!(get_relative_path_to_directory_or_url("/a", "/b", false, &ComparePathsOptions::default()), "../b");
    assert_eq!(get_relative_path_to_directory_or_url("/a/b", "/b", false, &ComparePathsOptions::default()), "../../b");
    assert_eq!(get_relative_path_to_directory_or_url("/a/b/c", "/b", false, &ComparePathsOptions::default()), "../../../b");
    assert_eq!(get_relative_path_to_directory_or_url("/a/b/c", "/b/c", false, &ComparePathsOptions::default()), "../../../b/c");
    assert_eq!(get_relative_path_to_directory_or_url("/a/b/c", "/a/b", false, &ComparePathsOptions::default()), "..");
    assert_eq!(get_relative_path_to_directory_or_url("c:", "d:", false, &ComparePathsOptions::default()), "d:/");
    assert_eq!(get_relative_path_to_directory_or_url("file:///", "file:///", false, &ComparePathsOptions::default()), "");
    assert_eq!(get_relative_path_to_directory_or_url("file:///a", "file:///a", false, &ComparePathsOptions::default()), "");
    assert_eq!(get_relative_path_to_directory_or_url("file:///a/", "file:///a", false, &ComparePathsOptions::default()), "");
    assert_eq!(get_relative_path_to_directory_or_url("file:///a", "file:///", false, &ComparePathsOptions::default()), "..");
    assert_eq!(get_relative_path_to_directory_or_url("file:///a", "file:///b", false, &ComparePathsOptions::default()), "../b");
    assert_eq!(get_relative_path_to_directory_or_url("file:///a/b", "file:///b", false, &ComparePathsOptions::default()), "../../b");
    assert_eq!(get_relative_path_to_directory_or_url("file:///a/b/c", "file:///b", false, &ComparePathsOptions::default()), "../../../b");
    assert_eq!(get_relative_path_to_directory_or_url("file:///a/b/c", "file:///b/c", false, &ComparePathsOptions::default()), "../../../b/c");
    assert_eq!(get_relative_path_to_directory_or_url("file:///a/b/c", "file:///a/b", false, &ComparePathsOptions::default()), "..");
    assert_eq!(get_relative_path_to_directory_or_url("file:///c:", "file:///d:", false, &ComparePathsOptions::default()), "file:///d:/");
}

#[test]
fn test_to_file_name_lower_case() {
    assert_eq!(to_file_name_lower_case("/user/UserName/projects/Project/file.ts"), "/user/username/projects/project/file.ts");
    assert_eq!(to_file_name_lower_case("/user/UserName/projects/projectß/file.ts"), "/user/username/projects/projectß/file.ts");
    assert_eq!(to_file_name_lower_case("/user/UserName/projects/İproject/file.ts"), "/user/username/projects/İproject/file.ts");
    assert_eq!(to_file_name_lower_case("/user/UserName/projects/ı/file.ts"), "/user/username/projects/ı/file.ts");
}

#[test]
fn test_to_path() {
    assert_eq!(to_path("file.ext", "path/to", false).as_str(), "path/to/file.ext");
    assert_eq!(to_path("file.ext", "/path/to", true).as_str(), "/path/to/file.ext");
    assert_eq!(to_path("/path/to/../file.ext", "path/to", true).as_str(), "/path/file.ext");
}

#[test]
fn test_get_longest_extension_from_path() {
    let extensions = &[".z", ".y.z", ".other"];
    assert_eq!(get_longest_extension_from_path("/src/Component.y.z", extensions, false), ".y.z");
    assert_eq!(get_longest_extension_from_path("/src/Component.z", extensions, false), ".z");
    assert_eq!(get_longest_extension_from_path("/src/Component.y.Z", extensions, false), "");
    assert_eq!(get_longest_extension_from_path("/src/Component.y.Z", extensions, true), ".y.Z");
}

#[test]
fn test_trim_file_path_prefix() {
    assert_eq!(trim_file_path_prefix("/project/src/file.ts", "/project/src", true), Some("/file.ts"));
    assert_eq!(trim_file_path_prefix("/project/SRC/file.ts", "/project/src", true), None);
    assert_eq!(trim_file_path_prefix("/project/SRC/file.ts", "/project/src", false), Some("/file.ts"));
    assert_eq!(trim_file_path_prefix("/other/file.ts", "/project/src", false), None);
    // Each Kelvin sign '\u212A' case-folds to the single-byte 'k'.
    assert_eq!(trim_file_path_prefix("/kkk/a.ts", "/\u{212A}\u{212A}\u{212A}", false), Some("/a.ts"));
    assert_eq!(trim_file_path_prefix("/project/src", "/project/src", true), Some(""));
}

#[test]
fn test_has_relative_path_segment() {
    // Expected values from the old regex `//|(?:^|/)\.\.?(?:$|/)`.
    let tests: Vec<(String, bool)> = vec![
        ("//".into(), true),
        ("foo/bar/baz".into(), false),
        ("foo/./baz".into(), true),
        ("foo/../baz".into(), true),
        ("foo/bar/baz/.".into(), true),
        ("./some/path".into(), true),
        ("/foo//bar/".into(), true),
        ("/foo/./bar/../../.".into(), true),
        ("foo/".repeat(100) + "..", true),
        ("foo.".into(), false),
        (".foo".into(), false),
        ("a/...".into(), false),
    ];
    for (p, expected) in tests {
        assert_eq!(has_relative_path_segment(&p), expected, "{p}");
    }
}

#[test]
fn test_has_relative_path_segment_matches_segment_split() {
    // Every string of up to 11 bytes over '/', '.', 'a' (so every word and tail position of the eight-byte scan),
    // also behind a 5-byte prefix, against splitting on '/'.
    fn by_segments(p: &str) -> bool {
        let segments: Vec<&str> = p.split('/').collect();
        segments.iter().enumerate().any(|(i, s)| *s == "." || *s == ".." || s.is_empty() && i > 0 && i + 1 < segments.len())
    }
    let alphabet = [b'/', b'.', b'a'];
    let mut bytes = Vec::new();
    for len in 0..=11 {
        for mut code in 0..3usize.pow(len) {
            bytes.clear();
            for _ in 0..len {
                bytes.push(alphabet[code % 3]);
                code /= 3;
            }
            let p = std::str::from_utf8(&bytes).unwrap();
            assert_eq!(has_relative_path_segment(p), by_segments(p), "{p}");
            let prefixed = format!("/ab/c{p}");
            assert_eq!(has_relative_path_segment(&prefixed), by_segments(&prefixed), "{prefixed}");
        }
    }
}

#[test]
fn test_path_is_relative() {
    let mut tests: Vec<(String, bool)> = vec![
        // relative
        (".".into(), true),
        ("..".into(), true),
        ("./".into(), true),
        ("../".into(), true),
        ("./foo/bar".into(), true),
        ("../foo/bar".into(), true),
        ("../".to_string() + &"foo/".repeat(100), true),
        // non-relative
        ("".into(), false),
        ("foo".into(), false),
        ("foo/bar".into(), false),
        ("/foo/bar".into(), false),
        ("c:/foo/bar".into(), false),
    ];
    let old = tests.clone();
    for (p, r) in old {
        tests.push((p.replace('/', "\\"), r));
    }
    for (p, expected) in tests {
        assert_eq!(path_is_relative(&p), expected, "{p}");
    }
}

#[test]
fn test_get_common_parents() {
    let opts = ComparePathsOptions::default();
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<String>>();
    let run = |paths: &[&str], min: usize| get_common_parents(&s(paths), min, get_path_components, &opts);

    let (got, ignored) = run(&[], 1);
    assert_eq!(ignored.len(), 0);
    assert!(got.is_empty());

    let (got, ignored) = run(&["/a/b/c/d"], 1);
    assert_eq!(ignored.len(), 0);
    assert_eq!(got, vec!["/a/b/c/d"]);

    let (got, ignored) = run(&["/a/b/c/d", "/a/b/c/e", "/a/b/f/g", "/x/y"], 4);
    assert_eq!(ignored, ["/x/y".to_string()].into_iter().collect());
    assert_eq!(got, vec!["/a/b/c", "/a/b/f/g"]);

    let (got, ignored) = run(&["/a/b/c/d", "/a/b/c/e", "/a/b/f/g"], 1);
    assert_eq!(ignored.len(), 0);
    assert_eq!(got, vec!["/a/b"]);

    let (got, _) = run(&["/a/b/c/d", "/a/b/c/e", "/a/b/f/g", "/x/y/z"], 1);
    assert_eq!(got, vec!["/"]);

    let (got, _) = run(&["/a/b/c/d", "/a/b/c/e", "/a/b/f/g", "/x/y/z"], 3);
    assert_eq!(got, vec!["/a/b", "/x/y/z"]);

    let (got, _) = run(&["c:/a/b/c/d", "d:/a/b/c/d"], 1);
    assert_eq!(got, vec!["c:/a/b/c/d", "d:/a/b/c/d"]);

    let (got, _) = run(&["/a/b/c/d", "/a/b/c/d"], 1);
    assert_eq!(got, vec!["/a/b/c/d"]);

    let (got, _) = run(&["/a/b/c/d", "/x/y"], 2);
    assert_eq!(got, vec!["/a/b/c/d", "/x/y"]);

    let (got, _) = run(&["/a/b/c/d", "/a/z/c/e", "/a/aaa/f/g", "/x/y/z"], 2);
    assert_eq!(got, vec!["/a", "/x/y/z"]);

    let (got, _) = run(&["/a/b/", "/a/b/c"], 1);
    assert_eq!(got, vec!["/a/b"]);

    let (got, _) = run(&["/a/x/1/p", "/a/x/2/q", "/a/y/3/r"], 4);
    assert_eq!(got, vec!["/a/x/1/p", "/a/x/2/q", "/a/y/3/r"]);
}

#[test]
fn test_untitled_path_handling() {
    let untitled_path = "^/untitled/ts-nul-authority/Untitled-2";
    assert_eq!(get_encoded_root_length(untitled_path), 2);
    assert!(is_rooted_disk_path(untitled_path));
    let current_dir = "/home/user/project";
    assert_eq!(to_path(untitled_path, current_dir, true).as_str(), "^/untitled/ts-nul-authority/Untitled-2");
    assert_eq!(get_normalized_absolute_path(untitled_path, current_dir), "^/untitled/ts-nul-authority/Untitled-2");
}

#[test]
fn test_untitled_path_edge_cases() {
    let cases = [
        ("^/", 2, true),
        ("^/untitled/ts-nul-authority/test", 2, true),
        ("^", 0, false),
        ("^x", 0, false),
        ("^^/", 0, false),
        ("x^/", 0, false),
        ("^/untitled/ts-nul-authority/path/with/deeper/structure", 2, true),
    ];
    for (path, expected, is_rooted) in cases {
        assert_eq!(get_encoded_root_length(path), expected, "{path}");
        assert_eq!(is_rooted_disk_path(path), is_rooted, "{path}");
    }
}

#[test]
fn test_starts_with_directory() {
    let tests = [
        ("exact match case sensitive", "/project/src/file.ts", "/project/src", true, true),
        ("exact match case insensitive", "/project/src/file.ts", "/PROJECT/SRC", false, true),
        ("case sensitive mismatch", "/project/src/file.ts", "/PROJECT/SRC", true, false),
        ("file not in directory", "/project/lib/file.ts", "/project/src", true, false),
        ("file in subdirectory", "/project/src/components/Button.tsx", "/project/src", true, true),
        ("file in parent directory", "/project/file.ts", "/project/src", true, false),
        ("windows style separators", "C:\\project\\src\\file.ts", "C:\\project\\src", true, true),
        ("mixed separators", "/project/src/file.ts", "\\project\\src", true, false),
        ("empty directory name", "/project/src/file.ts", "", true, false),
        ("empty file name", "", "/project/src", true, false),
        ("identical paths", "/project/src", "/project/src", true, false),
        ("directory with trailing separator", "/project/src/file.ts", "/project/src/", true, true),
        ("unicode characters", "/project/测试/file.ts", "/project/测试", true, true),
        ("unicode case insensitive", "/project/测试/file.ts", "/PROJECT/测试", false, true),
        ("file name shorter than directory", "/proj", "/project", true, false),
        ("file name starts with directory but no separator", "/projectsrc/file.ts", "/project", true, false),
        ("relative paths", "src/file.ts", "src", true, true),
        ("absolute vs relative", "/project/src/file.ts", "project/src", true, false),
    ];
    for (name, file_name, directory_name, use_case_sensitive_file_names, expected) in tests {
        assert_eq!(starts_with_directory(file_name, directory_name, use_case_sensitive_file_names), expected, "{name}");
    }
}

#[test]
fn test_contains_ignored_path() {
    let tests = [
        ("node_modules dot path", "/project/node_modules/.pnpm/file.ts", true),
        ("git directory", "/project/.git/hooks/pre-commit", true),
        ("emacs lock file", "/project/src/file.ts.#", true),
        ("regular file path", "/project/src/file.ts", false),
        ("node_modules without dot", "/project/node_modules/lodash/index.js", false),
        ("empty path", "", false),
        ("path with multiple ignored patterns", "/project/node_modules/.pnpm/.git/.#file.ts", true),
        ("case sensitive test", "/project/NODE_MODULES/.PNPM/file.ts", false),
        ("path with ignored pattern in middle", "/project/src/node_modules/.pnpm/dist/file.js", true),
        ("path with ignored pattern at end", "/project/src/file.ts.#", true),
        ("pattern at start", "/node_modules./file.ts", false),
        ("pattern at end", "/project/file.ts.#", true),
        ("multiple occurrences", "/project/.git/node_modules./.git/file.ts", true),
        ("no slashes", "node_modules.file.ts", false),
        ("single slash", "/file.ts", false),
    ];
    for (name, path, expected) in tests {
        assert_eq!(contains_ignored_path(path), expected, "{name}");
    }
    for pattern in ["/node_modules/.", "/.git", ".#"] {
        assert!(contains_ignored_path(&format!("/test{pattern}/file.ts")));
    }
}

// Case-insensitive path components compare with Go's strings.EqualFold (SimpleFold orbits). Expected values are
// tsgo's ContainsPath / GetRelativePathFromDirectory.
#[test]
fn test_ignore_case_paths_use_simple_fold() {
    let ci = ComparePathsOptions { use_case_sensitive_file_names: false, current_directory: "/".to_string() };
    assert!(!contains_path("/dir/I", "/dir/\u{131}/a.ts", &ci));
    assert_eq!(get_relative_path_from_directory("/dir/I", "/dir/\u{131}/a.ts", &ci), "../\u{131}/a.ts");
    assert!(!contains_path("/dir/i", "/dir/\u{130}/a.ts", &ci));
    assert_eq!(get_relative_path_from_directory("/dir/i", "/dir/\u{130}/a.ts", &ci), "../\u{130}/a.ts");
    assert!(contains_path("/dir/\u{3D1}", "/dir/\u{3F4}/a.ts", &ci));
    assert_eq!(get_relative_path_from_directory("/dir/\u{3D1}", "/dir/\u{3F4}/a.ts", &ci), "a.ts");
    assert!(contains_path("/dir/\u{FB05}", "/dir/\u{FB06}/a.ts", &ci));
    assert_eq!(get_relative_path_from_directory("/dir/\u{FB05}", "/dir/\u{FB06}/a.ts", &ci), "a.ts");
    assert!(contains_path("/dir/\u{1F80}", "/dir/\u{1F88}/a.ts", &ci));
}
