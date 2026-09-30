use tsrs_core::tspath::{self, Path};
use tsrs_core::{ResolutionMode, P, RESOLUTION_MODE_NONE};

use super::*;
use crate::types::{ResolvedModule, ResolvedTypeReferenceDirective};

#[test]
fn test_new_known_symlink() {
    let cache = new_known_symlink("/test/dir", true);
    assert_eq!(cache.cwd, "/test/dir");
    assert!(cache.use_case_sensitive_file_names);
}

#[test]
fn test_set_directory() {
    let cache = new_known_symlink("/test/dir", true);
    let symlink_path = tspath::to_path("/test/symlink", "/test/dir", true).ensure_trailing_directory_separator();
    let real_directory = P::new(KnownDirectoryLink {
        real: "/real/path/".to_string(),
        real_path: tspath::to_path("/real/path", "/test/dir", true).ensure_trailing_directory_separator(),
    });

    cache.set_directory("/test/symlink", symlink_path.clone(), Some(real_directory));

    // Check that directory was stored
    let stored = cache.directories().load(&symlink_path).expect("Expected directory to be stored").unwrap();
    assert_eq!(stored.real, real_directory.real);
    assert_eq!(stored.real_path, real_directory.real_path);

    // Check that realpath mapping was created
    let set = cache.directories_by_realpath().load(&real_directory.real_path).expect("Expected realpath mapping to be created");
    assert!(set.has(&"/test/symlink".to_string()));
}

#[test]
fn test_set_file() {
    let cache = new_known_symlink("/test/dir", true);
    let symlink = "/test/symlink/file.ts";
    let symlink_path = tspath::to_path(symlink, "/test/dir", true);
    let realpath = "/real/path/file.ts";

    cache.set_file(symlink, symlink_path.clone(), realpath);

    assert_eq!(cache.files().load(&symlink_path).expect("Expected file to be stored"), realpath);
}

#[test]
fn test_process_resolution() {
    let cache = new_known_symlink("/test/dir", true);

    // Test with empty paths
    cache.process_resolution("", "");
    cache.process_resolution("original", "");
    cache.process_resolution("", "resolved");

    // Test with valid paths
    let original_path = "/test/original/file.ts";
    let resolved_path = "/test/resolved/file.ts";
    cache.process_resolution(original_path, resolved_path);

    let symlink_path = tspath::to_path(original_path, "/test/dir", true);
    assert_eq!(cache.files().load(&symlink_path).expect("Expected file to be stored"), resolved_path);
}

#[test]
fn test_guess_directory_symlink() {
    let cache = new_known_symlink("/test/dir", true);
    let tests = [
        ("identical paths", "/test/path/file.ts", "/test/path/file.ts", "/test/dir", ["/", "/"]),
        ("different files same directory", "/test/path/file1.ts", "/test/path/file2.ts", "/test/dir", ["", ""]),
        ("different directories", "/test/path1/file.ts", "/test/path2/file.ts", "/test/dir", ["/test/path1", "/test/path2"]),
        ("node_modules paths", "/test/node_modules/pkg/file.ts", "/test/node_modules/pkg/file.ts", "/test/dir", ["/test/node_modules/pkg", "/test/node_modules/pkg"]),
        (
            "scoped package paths",
            "/test/node_modules/@scope/pkg/file.ts",
            "/test/node_modules/@scope/pkg/file.ts",
            "/test/dir",
            ["/test/node_modules/@scope/pkg", "/test/node_modules/@scope/pkg"],
        ),
    ];
    for (name, a, b, cwd, expected) in tests {
        let (common_resolved, common_original) = cache.guess_directory_symlink(a, b, cwd);
        assert_eq!(common_resolved, expected[0], "{name}");
        assert_eq!(common_original, expected[1], "{name}");
    }
}

#[test]
fn test_is_node_modules_or_scoped_package_directory() {
    let cache = new_known_symlink("/test/dir", true);
    let tests = [
        ("node_modules", "node_modules", true),
        ("scoped package", "@scope", true),
        ("regular directory", "src", false),
        ("empty string", "", false),
        ("case insensitive node_modules", "NODE_MODULES", false), // The function is case sensitive
        ("case insensitive scoped", "@SCOPE", true),
    ];
    for (name, dir, expected) in tests {
        assert_eq!(cache.is_node_modules_or_scoped_package_directory(dir), expected, "{name}");
    }
}

#[test]
fn test_set_symlinks_from_resolutions() {
    let cache = new_known_symlink("/test/dir", true);
    let resolved_modules = [
        ("/test/original/file1.ts", "/test/resolved/file1.ts", "module1", RESOLUTION_MODE_NONE, tspath::to_path("/test/source.ts", "/test/dir", true)),
        ("/test/original/file2.ts", "/test/resolved/file2.ts", "module2", RESOLUTION_MODE_NONE, tspath::to_path("/test/source.ts", "/test/dir", true)),
    ];

    cache.set_symlinks_from_resolutions(
        |callback: &mut dyn FnMut(&ResolvedModule, &str, ResolutionMode, &Path), _file| {
            for (original_path, resolved_path, module_name, mode, file_path) in &resolved_modules {
                let resolution = ResolvedModule { original_path, resolved_file_name: resolved_path, ..Default::default() };
                callback(&resolution, module_name, *mode, file_path);
            }
        },
        |_callback: &mut dyn FnMut(&ResolvedTypeReferenceDirective, &str, ResolutionMode, &Path), _file| {
            // No type reference directives for this test
        },
    );

    for (original_path, resolved_path, ..) in &resolved_modules {
        let symlink_path = tspath::to_path(original_path, "/test/dir", true);
        assert_eq!(cache.files().load(&symlink_path).as_deref(), Some(*resolved_path));
    }
}

#[test]
fn test_known_symlinks_thread_safety() {
    let cache = new_known_symlink("/test/dir", true);
    std::thread::scope(|s| {
        for id in 0..10u32 {
            let cache = &cache;
            s.spawn(move || {
                let c = char::from_u32(id).unwrap();
                let symlink_path = tspath::to_path(&format!("/test/symlink{c}"), "/test/dir", true).ensure_trailing_directory_separator();
                let real_directory = P::new(KnownDirectoryLink {
                    real: format!("/real/path{c}/"),
                    real_path: tspath::to_path(&format!("/real/path{c}"), "/test/dir", true).ensure_trailing_directory_separator(),
                });
                cache.set_directory(&format!("/test/symlink{c}"), symlink_path.clone(), Some(real_directory));

                // Read back
                let stored = cache.directories().load(&symlink_path).expect("Expected directory to be stored").unwrap();
                assert_eq!(stored.real, real_directory.real);
            });
        }
    });
    assert_eq!(cache.directories().size(), 10);
}
