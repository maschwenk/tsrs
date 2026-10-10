use std::sync::Arc;
use tsrs_vfs::vfstest;
use tsrs_vfs::FS;

use super::util::{get_package_realpath_funcs, word_indices};

// util_test.go:11
#[test]
fn test_word_indices() {
    let tests: &[(&str, &[&str])] = &[
        // Basic camelCase
        ("camelCase", &["camelCase", "Case"]),
        // snake_case
        ("snake_case", &["snake_case", "case"]),
        // ParseURL - uppercase sequence followed by lowercase
        ("ParseURL", &["ParseURL", "URL"]),
        // XMLHttpRequest - multiple uppercase sequences
        ("XMLHttpRequest", &["XMLHttpRequest", "HttpRequest", "Request"]),
        // Single word lowercase
        ("hello", &["hello"]),
        // Single word uppercase
        ("HELLO", &["HELLO"]),
        // Mixed with numbers
        ("parseHTML5Parser", &["parseHTML5Parser", "HTML5Parser", "Parser"]),
        // Underscore variations
        ("__proto__", &["__proto__", "proto__"]),
        ("_private_member", &["_private_member", "member"]),
        // Single character
        ("a", &["a"]),
        ("A", &["A"]),
        // Consecutive underscores
        ("test__double__underscore", &["test__double__underscore", "double__underscore", "underscore"]),
    ];

    for &(input, expected_words) in tests {
        let indices = word_indices(input);

        // Convert indices to actual word slices for comparison
        let actual_words: Vec<&str> = indices.iter().map(|&idx| &input[idx..]).collect();

        assert_eq!(actual_words, expected_words, "wordIndices({:?})", input);
    }
}

fn owned_fs(fs: impl FS + 'static) -> Arc<dyn FS> {
    Arc::new(fs)
}

// util_test.go:99
// TestGetPackageRealpathFuncs_FollowsNodeModulesSymlinks tests that toRealpath correctly
// follows symlinks for files outside the package directory (e.g. node_modules entries).
#[test]
fn test_get_package_realpath_funcs_follows_node_modules_symlinks() {
    let fs = owned_fs(vfstest::from_map(
        [
            ("/symlink-bin/pkg", vfstest::symlink("/real/bin/pkg")),
            ("/real/bin/pkg/index.d.ts", "export declare const a: number;".into()),
            ("/real/bin/pkg/node_modules/dep", vfstest::symlink("/real/dep")),
            ("/real/dep/index.d.ts", "export declare const b: number;".into()),
            ("/real/dep/src/utils/helper.d.ts", "export declare const c: number;".into()),
        ],
        true,
    ));

    let (to_realpath, _) = get_package_realpath_funcs(Arc::clone(&fs), "/symlink-bin/pkg");

    // Files inside the package should be converted via string replacement (fast path).
    assert_eq!(to_realpath("/symlink-bin/pkg/index.d.ts"), "/real/bin/pkg/index.d.ts", "package files should be converted via prefix replacement");

    // Files outside the package (e.g. node_modules symlinks) should be resolved via
    // fs.Realpath so the cache key is the canonical realpath, not the symlink path.
    assert_eq!(
        to_realpath("/real/bin/pkg/node_modules/dep/index.d.ts"),
        "/real/dep/index.d.ts",
        "node_modules symlinks must be followed so the same file gets a consistent cache key"
    );

    // Files in subdirectories of an already-resolved external package should
    // use the cached prefix mapping without additional realpath calls.
    assert_eq!(
        to_realpath("/real/bin/pkg/node_modules/dep/src/utils/helper.d.ts"),
        "/real/dep/src/utils/helper.d.ts",
        "subdirectories of a resolved external package should use cached prefix mapping"
    );
}

// util_test.go:153
#[test]
fn test_get_package_realpath_funcs_duplicate_cache_keys() {
    let fs = owned_fs(vfstest::from_map(
        [
            ("/workspace/packages/app-a", vfstest::symlink("/store/app-a")),
            ("/workspace/packages/app-b", vfstest::symlink("/store/app-b")),
            ("/store/app-a/index.d.ts", "export declare const a: number;".into()),
            ("/store/app-b/index.d.ts", "export declare const b: number;".into()),
            ("/store/app-a/node_modules/shared-lib", vfstest::symlink("/store/shared-lib")),
            ("/store/app-b/node_modules/shared-lib", vfstest::symlink("/store/shared-lib")),
            ("/store/shared-lib/index.d.ts", "export declare const shared: string;".into()),
        ],
        true,
    ));

    let (to_realpath_a, _) = get_package_realpath_funcs(Arc::clone(&fs), "/workspace/packages/app-a");
    let (to_realpath_b, _) = get_package_realpath_funcs(Arc::clone(&fs), "/workspace/packages/app-b");

    let resolved_a = to_realpath_a("/store/app-a/node_modules/shared-lib/index.d.ts");
    let resolved_b = to_realpath_b("/store/app-b/node_modules/shared-lib/index.d.ts");

    // Both should resolve to the same canonical realpath so the module resolver
    // uses a single cache key for the shared dependency, avoiding duplicate loads.
    let expected_realpath = "/store/shared-lib/index.d.ts";
    assert_eq!(resolved_a, expected_realpath, "app-a's toRealpath should follow the node_modules symlink to the realpath");
    assert_eq!(resolved_b, expected_realpath, "app-b's toRealpath should follow the node_modules symlink to the realpath");
}

// util_test.go:192
#[test]
fn test_get_package_realpath_funcs_non_symlinked_package_with_symlinked_deps() {
    let fs = owned_fs(vfstest::from_map(
        [
            ("/real/my-pkg/index.d.ts", "export declare const a: number;".into()),
            ("/real/my-pkg/node_modules/dep", vfstest::symlink("/real/dep")),
            ("/real/dep/index.d.ts", "export declare const b: number;".into()),
        ],
        true,
    ));

    let (to_realpath, _) = get_package_realpath_funcs(Arc::clone(&fs), "/real/my-pkg");

    // Files inside the (non-symlinked) package should be returned unchanged.
    assert_eq!(to_realpath("/real/my-pkg/index.d.ts"), "/real/my-pkg/index.d.ts");

    // Files outside the package reached via symlinked node_modules should still be resolved.
    assert_eq!(
        to_realpath("/real/my-pkg/node_modules/dep/index.d.ts"),
        "/real/dep/index.d.ts",
        "symlinked deps must be resolved even when the package dir itself is not a symlink"
    );
}
