// Port of specifiers_test.go.

use tsrs_ast::{Node, SourceFile};
use tsrs_core::tspath::{self, Path};
use tsrs_core::{CompilerOptions, ResolutionMode, P, RESOLUTION_MODE_NONE};
use tsrs_module::packagejson;
use tsrs_module::symlinks::{new_known_symlink, KnownDirectoryLink, KnownSymlinks};
use tsrs_module::ResolvedModule;
use tsrs_tsoptions::outputpaths::OutputPathsHost;
use tsrs_tsoptions::SourceOutputAndProjectReference;

use super::*;

// Mock host for testing
#[derive(Default)]
struct mockModuleSpecifierGenerationHost {
    current_dir: String,
    content_mapper_extensions: Vec<String>,
    use_case_sensitive_file_names: bool,
    symlink_cache: Option<P<KnownSymlinks>>,
}

impl OutputPathsHost for mockModuleSpecifierGenerationHost {
    fn common_source_directory(&self) -> String {
        self.current_dir.clone()
    }
    fn content_mapper_extensions(&self) -> Vec<String> {
        self.content_mapper_extensions.clone()
    }
    fn get_current_directory(&self) -> &str {
        &self.current_dir
    }
    fn use_case_sensitive_file_names(&self) -> bool {
        self.use_case_sensitive_file_names
    }
}

impl ModuleSpecifierGenerationHost for mockModuleSpecifierGenerationHost {
    fn get_symlink_cache(&self) -> P<KnownSymlinks> {
        self.symlink_cache.unwrap()
    }
    fn get_global_typings_cache_location(&self) -> String {
        String::new()
    }
    fn get_project_reference_from_source(&self, _path: &Path) -> Option<P<SourceOutputAndProjectReference>> {
        None
    }
    fn get_redirect_targets(&self, _path: &Path) -> Vec<String> {
        Vec::new()
    }
    fn get_source_of_project_reference_if_output_included(&self, file: P<SourceFile>) -> String {
        file.file_name().to_string()
    }
    fn file_exists(&self, _path: &str) -> bool {
        true // Mock implementation
    }
    fn get_nearest_ancestor_directory_with_package_json(&self, _dirname: &str) -> String {
        String::new()
    }
    fn get_package_json_info(&self, _pkg_json_path: &str) -> Option<P<packagejson::InfoCacheEntry>> {
        None
    }
    fn get_default_resolution_mode_for_file(&self, _file: P<SourceFile>) -> ResolutionMode {
        RESOLUTION_MODE_NONE
    }
    fn get_resolved_module_from_module_specifier(&self, _file: P<SourceFile>, _module_specifier: P<Node>) -> Option<P<ResolvedModule>> {
        None
    }
    fn get_mode_for_usage_location(&self, _file: P<SourceFile>, _module_specifier: P<Node>) -> ResolutionMode {
        RESOLUTION_MODE_NONE
    }
}

fn project_host() -> mockModuleSpecifierGenerationHost {
    mockModuleSpecifierGenerationHost {
        current_dir: "/project".to_string(),
        use_case_sensitive_file_names: true,
        symlink_cache: Some(P::new(new_known_symlink("/project", true))),
        ..Default::default()
    }
}

#[test]
fn test_get_each_file_name_of_module() {
    struct Case {
        name: &'static str,
        importing_file: &'static str,
        imported_file: &'static str,
        prefer_symlinks: bool,
        expected_count: usize,
        expected_paths: Option<&'static [&'static str]>,
    }
    let tests = [
        Case {
            name: "basic file path",
            importing_file: "/project/src/main.ts",
            imported_file: "/project/lib/utils.ts",
            prefer_symlinks: false,
            expected_count: 1,
            expected_paths: Some(&["/project/lib/utils.ts"]),
        },
        Case {
            name: "symlink preference false",
            importing_file: "/project/src/main.ts",
            imported_file: "/project/lib/utils.ts",
            prefer_symlinks: false,
            expected_count: 1,
            expected_paths: None,
        },
        Case {
            name: "symlink preference true",
            importing_file: "/project/src/main.ts",
            imported_file: "/project/lib/utils.ts",
            prefer_symlinks: true,
            expected_count: 1,
            expected_paths: None,
        },
        Case {
            name: "ignored path with no alternatives",
            importing_file: "/project/src/main.ts",
            imported_file: "/project/node_modules/.pnpm/file.ts",
            prefer_symlinks: false,
            expected_count: 1, // Should return 1 because there's no better option (all paths are ignored)
            expected_paths: None,
        },
    ];

    for tt in &tests {
        let host = project_host();
        let result = get_each_file_name_of_module(tt.importing_file, tt.imported_file, &host, tt.prefer_symlinks);
        assert_eq!(result.len(), tt.expected_count, "{}", tt.name);
        if let Some(expected_paths) = tt.expected_paths {
            for (i, expected_path) in expected_paths.iter().enumerate() {
                assert_eq!(result[i].file_name, *expected_path, "{}", tt.name);
            }
        }
        for path in &result {
            assert!(!path.file_name.is_empty(), "{}", tt.name);
        }
    }
}

#[test]
fn test_get_each_file_name_of_module_with_symlinks() {
    let host = project_host();

    let symlink_path = tspath::to_path("/project/symlink", "/project", true).ensure_trailing_directory_separator();
    let real_directory = KnownDirectoryLink {
        real: "/real/path/".to_string(),
        real_path: tspath::to_path("/real/path", "/project", true).ensure_trailing_directory_separator(),
    };
    host.symlink_cache.unwrap().set_directory("/project/symlink", symlink_path, Some(P::new(real_directory)));

    let result = get_each_file_name_of_module("/project/src/main.ts", "/real/path/file.ts", &host, true);

    // Should find the symlink path
    assert!(result.iter().any(|p| p.file_name == "/project/symlink/file.ts"), "Expected to find symlink path /project/symlink/file.ts");
    // Symlinks come first when preferSymlinks is set.
    assert_eq!(result.iter().map(|p| p.file_name.as_str()).collect::<Vec<_>>(), vec!["/project/symlink/file.ts", "/real/path/file.ts"]);
}

#[test]
fn test_contains_node_modules() {
    let tests = [
        ("contains node_modules", "/project/node_modules/lodash/index.js", true),
        ("does not contain node_modules", "/project/src/utils.ts", false),
        ("node_modules in middle", "/project/packages/node_modules/pkg/file.js", true),
        ("empty path", "", false),
    ];
    for (name, path, expected) in tests {
        assert_eq!(contains_node_modules(path), expected, "{name}");
    }
}

#[test]
fn test_contains_ignored_path() {
    let tests = [("ignored path", "/project/node_modules/.pnpm/file.ts", true), ("not ignored path", "/project/src/file.ts", false)];
    for (name, path, expected) in tests {
        assert_eq!(contains_ignored_path(path), expected, "{name}");
    }
}

#[test]
fn test_try_get_real_file_name_for_non_js_declaration_file_name() {
    let tests = [
        ("json declaration file", "/project/foo.d.json.ts", "/project/foo.json"),
        ("multi-dot source extension declaration file", "/project/foo.module.d.css.ts", "/project/foo.module.css"),
        ("plain dts file ignored", "/project/foo.d.ts", ""),
    ];
    for (name, file_name, expected) in tests {
        assert_eq!(try_get_real_file_name_for_non_js_declaration_file_name(file_name), expected, "{name}");
    }
}

#[test]
fn test_try_get_module_name_from_exports_or_imports() {
    // with exports pattern
    let tests = [
        ("match", "/pkg/src/things/thing1/index.ts", "./src/things/thing1"),
        ("mismatch with matching leading and trailing strings", "/pkg/src/things/index.ts", ""),
    ];
    let exports = packagejson::parse(r#"{"exports": "./src/things/*/index.js"}"#).unwrap().exports;
    assert_eq!(exports.type_(), packagejson::JSONValueType::String);
    for (name, target_file_path, expected) in tests {
        let result = try_get_module_name_from_exports_or_imports(
            &CompilerOptions::default(),
            &mockModuleSpecifierGenerationHost::default(),
            target_file_path,
            "/pkg",
            "./src/things/*",
            &exports,
            &[],
            MatchingMode::Pattern,
            false,
            false,
        );
        assert_eq!(result, expected, "{name}");
    }
}

// Not in the Go tests: pure helpers with behavior pinned by the Go source.

#[test]
fn test_get_node_module_path_parts() {
    let parts = get_node_module_path_parts("/base/path/node_modules/@scope/pkg/sub/file.js").unwrap();
    assert_eq!(parts.top_level_node_modules_index, 10);
    assert_eq!(parts.top_level_package_name_index, 23);
    assert_eq!(parts.package_root_index, 34);
    assert_eq!(parts.file_name_index, 38);
    assert!(get_node_module_path_parts("/base/path/src/file.js").is_none());
    // A bare scope directory still parses (state Scope > NodeModules), as in Go.
    assert!(get_node_module_path_parts("/base/node_modules/@scope").is_some());
    assert!(get_node_module_path_parts("/base/node_modules").is_none());
}

#[test]
fn test_get_package_name_from_directory() {
    assert_eq!(get_package_name_from_directory("/a/node_modules/pkg/lib/x.js"), "pkg");
    assert_eq!(get_package_name_from_directory("/a/node_modules/@s/pkg/lib/x.js"), "@s/pkg");
    assert_eq!(get_package_name_from_directory("/a/node_modules/@s/pkg"), "@s/pkg");
    assert_eq!(get_package_name_from_directory("/a/node_modules/.pnpm/x"), "");
    assert_eq!(get_package_name_from_directory("/a/src/x"), "");
}

#[test]
fn test_is_excluded_by_regex() {
    assert!(is_excluded_by_regex("lodash/fp", &["^lodash/".to_string()]));
    assert!(is_excluded_by_regex("Lodash/fp", &["/^lodash\\//i".to_string()]));
    assert!(!is_excluded_by_regex("Lodash/fp", &["/^lodash\\//".to_string()]));
    assert!(!is_excluded_by_regex("x", &["(".to_string()]));
}

#[test]
fn test_count_path_components() {
    assert_eq!(count_path_components("./a/b"), 1);
    assert_eq!(count_path_components("../a/b"), 2);
    assert_eq!(count_path_components("a"), 0);
}
