use tsrs_core::collections::OrderedMap;
use tsrs_core::tspath::ComparePathsOptions;

use crate::wildcarddirectories::get_wildcard_directories;

fn strings(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

#[test]
fn test_get_wildcard_directories_dot_prefixed_include_with_dot_dir_exclude() {
    // https://github.com/microsoft/TypeScript/tsc/issues/3733
    // "./"-prefixed include specs must be fully normalized before being tested
    // against exclude patterns; otherwise the leftover literal "." path segment
    // matches dot-directory excludes like "**/.*/", silently dropping every
    // wildcard directory (and with them, root file watching for the config).
    let result = get_wildcard_directories(
        &strings(&["./app/**/*.ts", "./app/**/*.tsx"]),
        &strings(&["**/node_modules", "**/.*/", "./build"]),
        &ComparePathsOptions { current_directory: "/home/projects/monorepo/apps/web".to_string(), use_case_sensitive_file_names: true },
    );
    let mut expected = OrderedMap::default();
    expected.insert("/home/projects/monorepo/apps/web/app".to_string(), true);
    assert_eq!(result, Some(expected));
}

#[test]
fn test_get_wildcard_directories_non_ascii_characters() {
    let tests: &[(&str, &[&str], &[&str], &str, bool)] = &[
        (
            "Norwegian character æ in path",
            &["src/**/*.test.ts", "src/**/*.stories.ts", "src/**/*.mdx"],
            &["node_modules"],
            "C:/Users/TobiasLægreid/dev/app/frontend/packages/react",
            false,
        ),
        ("Japanese characters in path", &["src/**/*.ts"], &["テスト"], "/Users/ユーザー/プロジェクト", true),
        ("Chinese characters in path", &["源代码/**/*.js"], &["节点模块"], "/home/用户/项目", true),
        ("Various Unicode characters", &["src/**/*.ts"], &["node_modules"], "/Users/Müller/café/naïve/résumé", false),
    ];

    for (name, include, exclude, current_directory, use_case_sensitive_file_names) in tests {
        let compare_paths_options = ComparePathsOptions {
            current_directory: current_directory.to_string(),
            use_case_sensitive_file_names: *use_case_sensitive_file_names,
        };
        let result = get_wildcard_directories(&strings(include), &strings(exclude), &compare_paths_options);
        assert!(result.is_some(), "{name}: expected non-nil result");
    }
}
