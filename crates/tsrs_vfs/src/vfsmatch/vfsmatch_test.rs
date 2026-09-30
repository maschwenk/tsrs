use super::*;
use crate::iovfs::IoVFS;
use crate::vfstest::{from_map, symlink, MapFS, MapFile};

// Test cases modeled after TypeScript's matchFiles tests in
// tsc/testdata/fixtures/testRunner/unittests/config/matchFiles.ts

type Host = IoVFS<MapFS>;

// caseInsensitiveHost simulates a Windows-like file system
fn case_insensitive_host() -> Host {
    from_map(
        [
            ("/dev/a.ts", ""),
            ("/dev/a.d.ts", ""),
            ("/dev/a.js", ""),
            ("/dev/b.ts", ""),
            ("/dev/b.js", ""),
            ("/dev/c.d.ts", ""),
            ("/dev/z/a.ts", ""),
            ("/dev/z/abz.ts", ""),
            ("/dev/z/aba.ts", ""),
            ("/dev/z/b.ts", ""),
            ("/dev/z/bbz.ts", ""),
            ("/dev/z/bba.ts", ""),
            ("/dev/x/a.ts", ""),
            ("/dev/x/aa.ts", ""),
            ("/dev/x/b.ts", ""),
            ("/dev/x/y/a.ts", ""),
            ("/dev/x/y/b.ts", ""),
            ("/dev/js/a.js", ""),
            ("/dev/js/b.js", ""),
            ("/dev/js/d.min.js", ""),
            ("/dev/js/ab.min.js", ""),
            ("/ext/ext.ts", ""),
            ("/ext/b/a..b.ts", ""),
        ],
        false,
    )
}

// caseSensitiveHost simulates a Unix-like case-sensitive file system
fn case_sensitive_host() -> Host {
    from_map(
        [
            ("/dev/a.ts", ""),
            ("/dev/a.d.ts", ""),
            ("/dev/a.js", ""),
            ("/dev/b.ts", ""),
            ("/dev/b.js", ""),
            ("/dev/A.ts", ""),
            ("/dev/B.ts", ""),
            ("/dev/c.d.ts", ""),
            ("/dev/z/a.ts", ""),
            ("/dev/z/abz.ts", ""),
            ("/dev/z/aba.ts", ""),
            ("/dev/z/b.ts", ""),
            ("/dev/z/bbz.ts", ""),
            ("/dev/z/bba.ts", ""),
            ("/dev/x/a.ts", ""),
            ("/dev/x/b.ts", ""),
            ("/dev/x/y/a.ts", ""),
            ("/dev/x/y/b.ts", ""),
            ("/dev/q/a/c/b/d.ts", ""),
            ("/dev/js/a.js", ""),
            ("/dev/js/b.js", ""),
            ("/dev/js/d.MIN.js", ""),
        ],
        true,
    )
}

// commonFoldersHost includes node_modules, bower_components, jspm_packages
fn common_folders_host() -> Host {
    from_map(
        [
            ("/dev/a.ts", ""),
            ("/dev/a.d.ts", ""),
            ("/dev/a.js", ""),
            ("/dev/b.ts", ""),
            ("/dev/x/a.ts", ""),
            ("/dev/node_modules/a.ts", ""),
            ("/dev/bower_components/a.ts", ""),
            ("/dev/jspm_packages/a.ts", ""),
        ],
        false,
    )
}

// dottedFoldersHost includes files and folders starting with a dot
fn dotted_folders_host() -> Host {
    from_map(
        [
            ("/dev/x/d.ts", ""),
            ("/dev/x/y/d.ts", ""),
            ("/dev/x/y/.e.ts", ""),
            ("/dev/x/.y/a.ts", ""),
            ("/dev/.z/.b.ts", ""),
            ("/dev/.z/c.ts", ""),
            ("/dev/w/.u/e.ts", ""),
            ("/dev/g.min.js/.g/g.ts", ""),
        ],
        false,
    )
}

// mixedExtensionHost has various file extensions
fn mixed_extension_host() -> Host {
    from_map(
        [
            ("/dev/a.ts", ""),
            ("/dev/a.d.ts", ""),
            ("/dev/a.js", ""),
            ("/dev/b.tsx", ""),
            ("/dev/b.d.ts", ""),
            ("/dev/b.jsx", ""),
            ("/dev/c.tsx", ""),
            ("/dev/c.js", ""),
            ("/dev/d.js", ""),
            ("/dev/e.jsx", ""),
            ("/dev/f.other", ""),
        ],
        false,
    )
}

// sameNamedDeclarationsHost has files with same names but different extensions
fn same_named_declarations_host() -> Host {
    from_map(
        [
            ("/dev/a.tsx", ""),
            ("/dev/a.d.ts", ""),
            ("/dev/b.tsx", ""),
            ("/dev/b.ts", ""),
            ("/dev/c.tsx", ""),
            ("/dev/m.ts", ""),
            ("/dev/m.d.ts", ""),
            ("/dev/n.tsx", ""),
            ("/dev/n.ts", ""),
            ("/dev/n.d.ts", ""),
            ("/dev/o.ts", ""),
            ("/dev/x.d.ts", ""),
        ],
        false,
    )
}

struct ReadDirTestCase {
    name: &'static str,
    host: fn() -> Host,
    current_dir: &'static str,
    path: &'static str,
    extensions: Vec<&'static str>,
    excludes: Vec<&'static str>,
    includes: Vec<&'static str>,
    depth: usize,
    expect: fn(&str, &[String]),
}

fn rd(name: &'static str, host: fn() -> Host, expect: fn(&str, &[String])) -> ReadDirTestCase {
    ReadDirTestCase {
        name,
        host,
        current_dir: "",
        path: "",
        extensions: Vec::new(),
        excludes: Vec::new(),
        includes: Vec::new(),
        depth: 0,
        expect,
    }
}

fn ts_exts() -> Vec<&'static str> {
    vec![".ts", ".tsx", ".d.ts"]
}

fn run_read_directory_case(tc: &ReadDirTestCase) {
    let current_dir = if tc.current_dir.is_empty() { "/" } else { tc.current_dir };
    let path = if tc.path.is_empty() { "/dev" } else { tc.path };
    let depth = if tc.depth == 0 { UNLIMITED_DEPTH } else { tc.depth };
    let host = (tc.host)();
    let got = match_files(path, &tc.extensions, &tc.excludes, &tc.includes, host.use_case_sensitive_file_names(), current_dir, depth, &host);
    (tc.expect)(tc.name, &got);
}

fn run_read_directory_cases(cases: &[ReadDirTestCase]) {
    for tc in cases {
        run_read_directory_case(tc);
    }
}

fn has(got: &[String], s: &str) -> bool {
    got.iter().any(|g| g == s)
}

#[track_caller]
fn assert_has(name: &str, got: &[String], s: &str) {
    assert!(has(got, s), "{name}: expected {s:?} in {got:?}");
}

#[track_caller]
fn assert_lacks(name: &str, got: &[String], s: &str) {
    assert!(!has(got, s), "{name}: unexpected {s:?} in {got:?}");
}

#[track_caller]
fn assert_list(name: &str, got: &[String], want: &[&str]) {
    assert_eq!(got, want, "{name}");
}

#[test]
fn test_read_directory() {
    let cases = vec![
        ReadDirTestCase {
            extensions: ts_exts(),
            ..rd("defaults include common package folders", common_folders_host, |name, got| {
                assert_has(name, got, "/dev/a.ts");
                assert_has(name, got, "/dev/b.ts");
                assert_has(name, got, "/dev/x/a.ts");
                assert_has(name, got, "/dev/node_modules/a.ts");
                assert_has(name, got, "/dev/bower_components/a.ts");
                assert_has(name, got, "/dev/jspm_packages/a.ts");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["a.ts", "b.ts"],
            ..rd("literal includes without exclusions", case_insensitive_host, |name, got| {
                assert_list(name, got, &["/dev/a.ts", "/dev/b.ts"]);
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["a.js", "b.js"],
            ..rd("literal includes with non ts extensions excluded", case_insensitive_host, |name, got| {
                assert_eq!(got.len(), 0, "{name}");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["z.ts", "x.ts"],
            ..rd("literal includes missing files excluded", case_insensitive_host, |name, got| {
                assert_eq!(got.len(), 0, "{name}");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            excludes: vec!["b.ts"],
            includes: vec!["a.ts", "b.ts"],
            ..rd("literal includes with literal excludes", case_insensitive_host, |name, got| {
                assert_list(name, got, &["/dev/a.ts"]);
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            excludes: vec!["*.ts", "z/??z.ts", "*/b.ts"],
            includes: vec!["a.ts", "b.ts", "z/a.ts", "z/abz.ts", "z/aba.ts", "x/b.ts"],
            ..rd("literal includes with wildcard excludes", case_insensitive_host, |name, got| {
                assert_list(name, got, &["/dev/z/a.ts", "/dev/z/aba.ts"]);
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            excludes: vec!["**/b.ts"],
            includes: vec!["a.ts", "b.ts", "x/a.ts", "x/b.ts", "x/y/a.ts", "x/y/b.ts"],
            ..rd("literal includes with recursive excludes", case_insensitive_host, |name, got| {
                assert_list(name, got, &["/dev/a.ts", "/dev/x/a.ts", "/dev/x/y/a.ts"]);
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            excludes: vec!["**/b.ts"],
            includes: vec!["B.ts"],
            ..rd("case sensitive exclude is respected", case_sensitive_host, |name, got| {
                assert_list(name, got, &["/dev/B.ts"]);
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["a.ts", "b.ts", "node_modules/a.ts", "bower_components/a.ts", "jspm_packages/a.ts"],
            ..rd("explicit includes keep common package folders", common_folders_host, |name, got| {
                assert_has(name, got, "/dev/a.ts");
                assert_has(name, got, "/dev/b.ts");
                assert_has(name, got, "/dev/node_modules/a.ts");
                assert_has(name, got, "/dev/bower_components/a.ts");
                assert_has(name, got, "/dev/jspm_packages/a.ts");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["z/*.ts", "x/*.ts"],
            ..rd("wildcard include sorted order", case_insensitive_host, |name, got| {
                let expected = [
                    "/dev/z/a.ts",
                    "/dev/z/aba.ts",
                    "/dev/z/abz.ts",
                    "/dev/z/b.ts",
                    "/dev/z/bba.ts",
                    "/dev/z/bbz.ts",
                    "/dev/x/a.ts",
                    "/dev/x/aa.ts",
                    "/dev/x/b.ts",
                ];
                assert_list(name, got, &expected);
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["*.ts"],
            ..rd("wildcard include same named declarations excluded", case_insensitive_host, |name, got| {
                assert_has(name, got, "/dev/a.ts");
                assert_has(name, got, "/dev/b.ts");
                assert_has(name, got, "/dev/a.d.ts");
                assert_has(name, got, "/dev/c.d.ts");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["*"],
            ..rd("wildcard star matches only ts files", case_insensitive_host, |name, got| {
                for f in got {
                    assert!(f.contains(".ts") || f.contains(".tsx") || f.contains(".d.ts"), "{name}: unexpected file: {f}");
                }
                assert_lacks(name, got, "/dev/a.js");
                assert_lacks(name, got, "/dev/b.js");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["x/?.ts"],
            ..rd("wildcard question mark single character", case_insensitive_host, |name, got| {
                assert_list(name, got, &["/dev/x/a.ts", "/dev/x/b.ts"]);
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["**/a.ts"],
            ..rd("wildcard recursive directory", case_insensitive_host, |name, got| {
                assert_has(name, got, "/dev/a.ts");
                assert_has(name, got, "/dev/z/a.ts");
                assert_has(name, got, "/dev/x/a.ts");
                assert_has(name, got, "/dev/x/y/a.ts");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["x/**/a.ts"],
            ..rd("double asterisk matches zero-or-more directories", case_insensitive_host, |name, got| {
                assert_eq!(got.len(), 2, "{name}");
                assert_has(name, got, "/dev/x/a.ts");
                assert_has(name, got, "/dev/x/y/a.ts");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["x/y/**/a.ts", "x/**/a.ts", "z/**/a.ts"],
            ..rd("wildcard multiple recursive directories", case_insensitive_host, |name, got| {
                assert!(!got.is_empty(), "{name}");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["**/A.ts"],
            ..rd("wildcard case sensitive matching", case_sensitive_host, |name, got| {
                assert_list(name, got, &["/dev/A.ts"]);
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["*/z.ts"],
            ..rd("wildcard missing files excluded", case_insensitive_host, |name, got| assert_eq!(got.len(), 0, "{name}"))
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            excludes: vec!["z", "x"],
            includes: vec!["**/*"],
            ..rd("exclude folders with wildcards", case_insensitive_host, |name, got| {
                for f in got {
                    assert!(!f.contains("/z/") && !f.contains("/x/"), "{name}: should not contain z or x: {f}");
                }
                assert_has(name, got, "/dev/a.ts");
                assert_has(name, got, "/dev/b.ts");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["*", "/ext/*"],
            ..rd("include paths outside project absolute", case_insensitive_host, |name, got| {
                assert_has(name, got, "/dev/a.ts");
                assert_has(name, got, "/ext/ext.ts");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            excludes: vec!["**"],
            includes: vec!["*", "../ext/*"],
            ..rd("include paths outside project relative", case_insensitive_host, |name, got| {
                assert_has(name, got, "/ext/ext.ts");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            excludes: vec!["**"],
            includes: vec!["/ext/b/a..b.ts"],
            ..rd("include files containing double dots", case_insensitive_host, |name, got| {
                assert_has(name, got, "/ext/b/a..b.ts");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            excludes: vec!["/ext/b/a..b.ts"],
            includes: vec!["/ext/**/*"],
            ..rd("exclude files containing double dots", case_insensitive_host, |name, got| {
                assert_has(name, got, "/ext/ext.ts");
                assert_lacks(name, got, "/ext/b/a..b.ts");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["**/a.ts"],
            ..rd("common package folders implicitly excluded", common_folders_host, |name, got| {
                assert_has(name, got, "/dev/a.ts");
                assert_has(name, got, "/dev/x/a.ts");
                assert_lacks(name, got, "/dev/node_modules/a.ts");
                assert_lacks(name, got, "/dev/bower_components/a.ts");
                assert_lacks(name, got, "/dev/jspm_packages/a.ts");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["**/a.ts", "**/node_modules/a.ts"],
            ..rd("common package folders explicit recursive include", common_folders_host, |name, got| {
                assert_has(name, got, "/dev/a.ts");
                assert_has(name, got, "/dev/node_modules/a.ts");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["*/a.ts"],
            ..rd("common package folders wildcard include", common_folders_host, |name, got| {
                assert_has(name, got, "/dev/x/a.ts");
                assert_lacks(name, got, "/dev/node_modules/a.ts");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["*/a.ts", "node_modules/a.ts"],
            ..rd("common package folders explicit wildcard include", common_folders_host, |name, got| {
                assert_has(name, got, "/dev/x/a.ts");
                assert_has(name, got, "/dev/node_modules/a.ts");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["x/**/*", "w/*/*"],
            ..rd("dotted folders not implicitly included", dotted_folders_host, |name, got| {
                assert_has(name, got, "/dev/x/d.ts");
                assert_has(name, got, "/dev/x/y/d.ts");
                assert_lacks(name, got, "/dev/x/.y/a.ts");
                assert_lacks(name, got, "/dev/x/y/.e.ts");
                assert_lacks(name, got, "/dev/w/.u/e.ts");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["x/.y/a.ts", "/dev/.z/.b.ts"],
            ..rd("dotted folders explicitly included", dotted_folders_host, |name, got| {
                assert_has(name, got, "/dev/x/.y/a.ts");
                assert_has(name, got, "/dev/.z/.b.ts");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["**/.*/*"],
            ..rd("dotted folders recursive wildcard matches directories", dotted_folders_host, |name, got| {
                assert_has(name, got, "/dev/x/.y/a.ts");
                assert_has(name, got, "/dev/.z/c.ts");
                assert_has(name, got, "/dev/w/.u/e.ts");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["**"],
            ..rd("trailing recursive include returns empty", case_insensitive_host, |name, got| assert_eq!(got.len(), 0, "{name}"))
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            excludes: vec!["**"],
            includes: vec!["**/*"],
            ..rd("trailing recursive exclude removes everything", case_insensitive_host, |name, got| assert_eq!(got.len(), 0, "{name}"))
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["**/x/**/*"],
            ..rd("multiple recursive directory patterns in includes", case_insensitive_host, |name, got| {
                assert_has(name, got, "/dev/x/a.ts");
                assert_has(name, got, "/dev/x/y/a.ts");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            excludes: vec!["**/x/**"],
            includes: vec!["**/a.ts"],
            ..rd("multiple recursive directory patterns in excludes", case_insensitive_host, |name, got| {
                assert_has(name, got, "/dev/a.ts");
                assert_has(name, got, "/dev/z/a.ts");
                assert_lacks(name, got, "/dev/x/a.ts");
                assert_lacks(name, got, "/dev/x/y/a.ts");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["z"],
            ..rd("implicit globbification expands directory", case_insensitive_host, |name, got| {
                assert_has(name, got, "/dev/z/a.ts");
                assert_has(name, got, "/dev/z/aba.ts");
                assert_has(name, got, "/dev/z/b.ts");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            excludes: vec!["**/x"],
            ..rd("exclude patterns starting with starstar", case_sensitive_host, |name, got| {
                for f in got {
                    assert!(!f.contains("/x/"), "{name}: should not contain /x/: {f}");
                }
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["**/x", "**/a/**/b"],
            ..rd("include patterns starting with starstar", case_sensitive_host, |name, got| {
                assert_has(name, got, "/dev/x/a.ts");
                assert_has(name, got, "/dev/q/a/c/b/d.ts");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            depth: 1,
            ..rd("depth limit one", case_insensitive_host, |name, got| {
                for f in got {
                    let suffix = &f["/dev/".len()..];
                    assert!(!suffix.contains('/'), "{name}: depth 1 should not include nested files: {f}");
                }
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            depth: 2,
            ..rd("depth limit two", case_insensitive_host, |name, got| {
                assert_has(name, got, "/dev/a.ts");
                assert_has(name, got, "/dev/z/a.ts");
                assert_lacks(name, got, "/dev/x/y/a.ts");
            })
        },
        ReadDirTestCase {
            extensions: vec![".ts"],
            ..rd("mixed extensions only ts", mixed_extension_host, |name, got| {
                for f in got {
                    assert!(f.ends_with(".ts"), "{name}: should only have .ts files: {f}");
                }
            })
        },
        ReadDirTestCase {
            extensions: vec![".ts", ".tsx"],
            ..rd("mixed extensions ts and tsx", mixed_extension_host, |name, got| {
                for f in got {
                    assert!(f.ends_with(".ts") || f.ends_with(".tsx"), "{name}: should only have .ts or .tsx files: {f}");
                }
            })
        },
        ReadDirTestCase {
            extensions: vec![".js", ".jsx"],
            ..rd("mixed extensions js and jsx", mixed_extension_host, |name, got| {
                for f in got {
                    assert!(f.ends_with(".js") || f.ends_with(".jsx"), "{name}: should only have .js or .jsx files: {f}");
                }
            })
        },
        ReadDirTestCase {
            extensions: vec![".js"],
            includes: vec!["js/*"],
            ..rd("min js files excluded by wildcard", case_insensitive_host, |name, got| {
                assert_has(name, got, "/dev/js/a.js");
                assert_has(name, got, "/dev/js/b.js");
                assert_lacks(name, got, "/dev/js/d.min.js");
                assert_lacks(name, got, "/dev/js/ab.min.js");
            })
        },
        ReadDirTestCase {
            extensions: vec![".js"],
            includes: vec!["js/*"],
            ..rd("min js exclusion is case-sensitive on case-sensitive FS", case_sensitive_host, |name, got| {
                assert_has(name, got, "/dev/js/a.js");
                assert_has(name, got, "/dev/js/b.js");
                // Legacy behavior: only lowercase ".min.js" is excluded by default when matching is case-sensitive.
                assert_has(name, got, "/dev/js/d.MIN.js");
            })
        },
        ReadDirTestCase {
            extensions: vec![".js"],
            includes: vec!["js/*.min.js"],
            ..rd("min js files explicitly included", case_insensitive_host, |name, got| {
                assert_has(name, got, "/dev/js/d.min.js");
                assert_has(name, got, "/dev/js/ab.min.js");
            })
        },
        ReadDirTestCase {
            extensions: vec![".js"],
            includes: vec!["js/*.min.*"],
            ..rd("min js files included when pattern mentions .min.", case_insensitive_host, |name, got| {
                assert_eq!(got.len(), 2, "{name}");
                assert_has(name, got, "/dev/js/d.min.js");
                assert_has(name, got, "/dev/js/ab.min.js");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            excludes: vec!["node_modules"],
            includes: vec!["**/*"],
            ..rd("exclude literal node_modules folder", common_folders_host, |name, got| {
                assert_has(name, got, "/dev/a.ts");
                assert_lacks(name, got, "/dev/node_modules/a.ts");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["*.ts"],
            ..rd("same named declarations include ts", same_named_declarations_host, |name, got| assert!(!got.is_empty(), "{name}"))
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["*.tsx"],
            ..rd("same named declarations include tsx", same_named_declarations_host, |name, got| {
                for f in got {
                    assert!(f.ends_with(".tsx"), "{name}: should only have .tsx files: {f}");
                }
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            ..rd("empty includes returns all matching files", case_insensitive_host, |name, got| {
                assert!(!got.is_empty(), "{name}");
                assert_has(name, got, "/dev/a.ts");
            })
        },
        rd("nil extensions returns all files", case_insensitive_host, |name, got| {
            assert_has(name, got, "/dev/a.ts");
            assert_has(name, got, "/dev/a.js");
        }),
        ReadDirTestCase {
            extensions: vec![],
            ..rd("empty extensions slice returns all files", case_insensitive_host, |name, got| {
                assert!(!got.is_empty(), "{name}: expected files to be returned")
            })
        },
    ];

    run_read_directory_cases(&cases);
}

#[test]
fn test_is_implicit_glob() {
    let tests = [
        ("simple", "foo", true),
        ("folder", "src", true),
        ("with extension", "foo.ts", false),
        ("trailing dot", "foo.", false),
        ("star", "*", false),
        ("question", "?", false),
        ("star suffix", "foo*", false),
        ("question suffix", "foo?", false),
        ("dot name", "foo.bar", false),
        ("empty", "", true),
    ];

    for (name, input, expected) in tests {
        let result = is_implicit_glob(input);
        assert_eq!(result, expected, "{name}");
    }
}

// Edge case tests for various pattern scenarios
#[test]
fn test_read_directory_edge_cases() {
    let cases = vec![
        ReadDirTestCase {
            extensions: vec![".ts"],
            includes: vec!["/dev/a.ts"],
            ..rd("rooted include path", case_insensitive_host, |name, got| assert_has(name, got, "/dev/a.ts"))
        },
        ReadDirTestCase {
            extensions: vec![".ts"],
            includes: vec!["a.ts"],
            ..rd("include with extension in path", case_insensitive_host, |name, got| assert_has(name, got, "/dev/a.ts"))
        },
        ReadDirTestCase {
            extensions: vec![".ts"],
            includes: vec!["file+test.ts"],
            ..rd(
                "special regex characters in path",
                || {
                    from_map(
                        [
                            ("/dev/file+test.ts", ""),
                            ("/dev/file[0].ts", ""),
                            ("/dev/file(1).ts", ""),
                            ("/dev/file$money.ts", ""),
                            ("/dev/file^start.ts", ""),
                            ("/dev/file|pipe.ts", ""),
                            ("/dev/file#hash.ts", ""),
                        ],
                        false,
                    )
                },
                |name, got| assert_has(name, got, "/dev/file+test.ts"),
            )
        },
        ReadDirTestCase {
            extensions: vec![".ts"],
            includes: vec!["?.ts"],
            ..rd("include pattern starting with question mark", case_insensitive_host, |name, got| {
                assert_has(name, got, "/dev/a.ts");
                assert_has(name, got, "/dev/b.ts");
            })
        },
        ReadDirTestCase {
            extensions: vec![".ts"],
            includes: vec!["*b.ts"],
            ..rd("include pattern starting with star", case_insensitive_host, |name, got| assert_has(name, got, "/dev/b.ts"))
        },
        ReadDirTestCase {
            extensions: vec![".ts"],
            includes: vec!["*.ts"],
            ..rd(
                "case insensitive file matching",
                || from_map([("/dev/File.ts", ""), ("/dev/FILE.ts", "")], true),
                |name, got| assert!(got.len() == 2, "{name}: {got:?}"),
            )
        },
        ReadDirTestCase {
            extensions: vec![".ts"],
            includes: vec!["q/a/c/b/d.ts"],
            ..rd("nested subdirectory base path", case_sensitive_host, |name, got| assert_has(name, got, "/dev/q/a/c/b/d.ts"))
        },
        ReadDirTestCase {
            extensions: vec![".ts"],
            includes: vec!["z/*.ts"],
            ..rd("current directory differs from path", case_insensitive_host, |name, got| assert!(!got.is_empty(), "{name}"))
        },
    ];

    run_read_directory_cases(&cases);
}

#[test]
fn test_read_directory_empty_includes() {
    let cases = vec![ReadDirTestCase {
        path: "/root",
        current_dir: "/",
        extensions: vec![".ts"],
        includes: vec![],
        ..rd(
            "empty includes slice behavior",
            || from_map([("/root/a.ts", "")], true),
            |name, got| {
                if got.is_empty() {
                    return;
                }
                assert_has(name, got, "/root/a.ts");
            },
        )
    }];

    run_read_directory_cases(&cases);
}

// TestReadDirectorySymlinkCycle tests that cyclic symlinks don't cause infinite loops.
// The cycle is detected by the vfs package using Realpath for cycle detection.
// This means directories with cyclic symlinks will be skipped during traversal.
#[test]
fn test_read_directory_symlink_cycle() {
    let cases = vec![ReadDirTestCase {
        path: "/root",
        current_dir: "/",
        extensions: vec![".ts"],
        includes: vec!["**/*"],
        ..rd(
            "detects and skips symlink cycles",
            || {
                from_map(
                    vec![
                        ("/root/file.ts", MapFile::from("")),
                        ("/root/a/file.ts", MapFile::from("")),
                        ("/root/a/b", symlink("/root/a")),
                    ],
                    true,
                )
            },
            |name, got| {
                let expected = ["/root/file.ts", "/root/a/file.ts"];
                assert_list(name, got, &expected);
            },
        )
    }];

    run_read_directory_cases(&cases);
}

// TestReadDirectoryMatchesTypeScriptBaselines contains tests that verify the Go implementation
// matches the promoted TypeScript baseline outputs
#[test]
fn test_read_directory_matches_type_script_baselines() {
    let cases = vec![
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["z/*.ts", "x/*.ts"],
            ..rd(
                "sorted in include order then alphabetical",
                || {
                    from_map(
                        [
                            ("/dev/z/a.ts", ""),
                            ("/dev/z/aba.ts", ""),
                            ("/dev/z/abz.ts", ""),
                            ("/dev/z/b.ts", ""),
                            ("/dev/z/bba.ts", ""),
                            ("/dev/z/bbz.ts", ""),
                            ("/dev/x/a.ts", ""),
                            ("/dev/x/aa.ts", ""),
                            ("/dev/x/b.ts", ""),
                        ],
                        false,
                    )
                },
                |name, got| {
                    let expected = [
                        "/dev/z/a.ts",
                        "/dev/z/aba.ts",
                        "/dev/z/abz.ts",
                        "/dev/z/b.ts",
                        "/dev/z/bba.ts",
                        "/dev/z/bbz.ts",
                        "/dev/x/a.ts",
                        "/dev/x/aa.ts",
                        "/dev/x/b.ts",
                    ];
                    assert_list(name, got, &expected);
                },
            )
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["**/.*/*"],
            ..rd(
                "recursive wildcards match dotted directories",
                || {
                    from_map(
                        [
                            ("/dev/x/d.ts", ""),
                            ("/dev/x/y/d.ts", ""),
                            ("/dev/x/y/.e.ts", ""),
                            ("/dev/x/.y/a.ts", ""),
                            ("/dev/.z/.b.ts", ""),
                            ("/dev/.z/c.ts", ""),
                            ("/dev/w/.u/e.ts", ""),
                            ("/dev/g.min.js/.g/g.ts", ""),
                        ],
                        false,
                    )
                },
                |name, got| {
                    let expected = ["/dev/.z/c.ts", "/dev/g.min.js/.g/g.ts", "/dev/w/.u/e.ts", "/dev/x/.y/a.ts"];
                    assert_eq!(got.len(), expected.len(), "{name}: {got:?}");
                    for want in expected {
                        assert_has(name, got, want);
                    }
                },
            )
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["**/a.ts"],
            ..rd(
                "common package folders implicitly excluded with wildcard",
                || {
                    from_map(
                        [
                            ("/dev/a.ts", ""),
                            ("/dev/a.d.ts", ""),
                            ("/dev/a.js", ""),
                            ("/dev/b.ts", ""),
                            ("/dev/x/a.ts", ""),
                            ("/dev/node_modules/a.ts", ""),
                            ("/dev/bower_components/a.ts", ""),
                            ("/dev/jspm_packages/a.ts", ""),
                        ],
                        false,
                    )
                },
                |name, got| assert_list(name, got, &["/dev/a.ts", "/dev/x/a.ts"]),
            )
        },
        ReadDirTestCase {
            extensions: vec![".js"],
            includes: vec!["js/*"],
            ..rd(
                "js wildcard excludes min js files",
                || from_map([("/dev/js/a.js", ""), ("/dev/js/b.js", ""), ("/dev/js/d.min.js", ""), ("/dev/js/ab.min.js", "")], false),
                |name, got| assert_list(name, got, &["/dev/js/a.js", "/dev/js/b.js"]),
            )
        },
        ReadDirTestCase {
            extensions: vec![".js"],
            includes: vec!["js/*.min.js"],
            ..rd(
                "explicit min js pattern includes min files",
                || from_map([("/dev/js/a.js", ""), ("/dev/js/b.js", ""), ("/dev/js/d.min.js", ""), ("/dev/js/ab.min.js", "")], false),
                |name, got| {
                    let expected = ["/dev/js/ab.min.js", "/dev/js/d.min.js"];
                    assert_eq!(got.len(), expected.len(), "{name}: {got:?}");
                    for want in expected {
                        assert_has(name, got, want);
                    }
                },
            )
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            excludes: vec!["b.ts"],
            includes: vec!["a.ts", "b.ts"],
            ..rd("literal excludes baseline", case_insensitive_host, |name, got| assert_list(name, got, &["/dev/a.ts"]))
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            excludes: vec!["*.ts", "z/??z.ts", "*/b.ts"],
            includes: vec!["a.ts", "b.ts", "z/a.ts", "z/abz.ts", "z/aba.ts", "x/b.ts"],
            ..rd("wildcard excludes baseline", case_insensitive_host, |name, got| assert_list(name, got, &["/dev/z/a.ts", "/dev/z/aba.ts"]))
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            excludes: vec!["**/b.ts"],
            includes: vec!["a.ts", "b.ts", "x/a.ts", "x/b.ts", "x/y/a.ts", "x/y/b.ts"],
            ..rd("recursive excludes baseline", case_insensitive_host, |name, got| {
                assert_list(name, got, &["/dev/a.ts", "/dev/x/a.ts", "/dev/x/y/a.ts"]);
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["x/?.ts"],
            ..rd("question mark baseline", case_insensitive_host, |name, got| assert_list(name, got, &["/dev/x/a.ts", "/dev/x/b.ts"]))
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["**/a.ts"],
            ..rd("recursive directory pattern baseline", case_insensitive_host, |name, got| {
                assert_list(name, got, &["/dev/a.ts", "/dev/x/a.ts", "/dev/x/y/a.ts", "/dev/z/a.ts"]);
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["**/A.ts"],
            ..rd("case sensitive baseline", case_sensitive_host, |name, got| assert_list(name, got, &["/dev/A.ts"]))
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            excludes: vec!["z", "x"],
            includes: vec!["**/*"],
            ..rd("exclude folders baseline", case_insensitive_host, |name, got| {
                for f in got {
                    assert!(!f.contains("/z/") && !f.contains("/x/"), "{name}: should not contain z or x: {f}");
                }
                assert_has(name, got, "/dev/a.ts");
                assert_has(name, got, "/dev/b.ts");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["z"],
            ..rd("implicit glob expansion baseline", case_insensitive_host, |name, got| {
                assert_list(name, got, &["/dev/z/a.ts", "/dev/z/aba.ts", "/dev/z/abz.ts", "/dev/z/b.ts", "/dev/z/bba.ts", "/dev/z/bbz.ts"]);
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["**"],
            ..rd("trailing recursive directory baseline", case_insensitive_host, |name, got| assert_eq!(got.len(), 0, "{name}"))
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            excludes: vec!["**"],
            includes: vec!["**/*"],
            ..rd("exclude trailing recursive directory baseline", case_insensitive_host, |name, got| assert_eq!(got.len(), 0, "{name}"))
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["**/x/**/*"],
            ..rd("multiple recursive directory patterns baseline", case_insensitive_host, |name, got| {
                assert_has(name, got, "/dev/x/a.ts");
                assert_has(name, got, "/dev/x/aa.ts");
                assert_has(name, got, "/dev/x/b.ts");
                assert_has(name, got, "/dev/x/y/a.ts");
                assert_has(name, got, "/dev/x/y/b.ts");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["**/x", "**/a/**/b"],
            ..rd("include dirs with starstar prefix baseline", case_sensitive_host, |name, got| {
                assert_has(name, got, "/dev/x/a.ts");
                assert_has(name, got, "/dev/x/b.ts");
                assert_has(name, got, "/dev/q/a/c/b/d.ts");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["x/**/*", "w/*/*"],
            ..rd("dotted folders not implicitly included baseline", dotted_folders_host, |name, got| {
                assert_has(name, got, "/dev/x/d.ts");
                assert_has(name, got, "/dev/x/y/d.ts");
                assert_lacks(name, got, "/dev/x/.y/a.ts");
                assert_lacks(name, got, "/dev/x/y/.e.ts");
                assert_lacks(name, got, "/dev/w/.u/e.ts");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            includes: vec!["*", "/ext/*"],
            ..rd("include paths outside project baseline", case_insensitive_host, |name, got| {
                assert_has(name, got, "/dev/a.ts");
                assert_has(name, got, "/ext/ext.ts");
            })
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            excludes: vec!["**"],
            includes: vec!["/ext/b/a..b.ts"],
            ..rd("include files with double dots baseline", case_insensitive_host, |name, got| assert_has(name, got, "/ext/b/a..b.ts"))
        },
        ReadDirTestCase {
            extensions: ts_exts(),
            excludes: vec!["/ext/b/a..b.ts"],
            includes: vec!["/ext/**/*"],
            ..rd("exclude files with double dots baseline", case_insensitive_host, |name, got| {
                assert_has(name, got, "/ext/ext.ts");
                assert_lacks(name, got, "/ext/b/a..b.ts");
            })
        },
    ];

    run_read_directory_cases(&cases);
}

struct SpecMatcherCase {
    name: &'static str,
    specs: Vec<&'static str>,
    base_path: &'static str,
    usage: Usage,
    use_case_sensitive_file_names: bool,
    matching_paths: Vec<&'static str>,
    non_matching_paths: Vec<&'static str>,
}

// TestSpecMatcher tests the SpecMatcher API
#[test]
fn test_spec_matcher() {
    let cases = vec![
        SpecMatcherCase {
            name: "simple wildcard",
            specs: vec!["*.ts"],
            base_path: "/project",
            usage: Usage::Files,
            use_case_sensitive_file_names: true,
            matching_paths: vec!["/project/a.ts", "/project/b.ts", "/project/foo.ts"],
            non_matching_paths: vec!["/project/a.js", "/project/sub/a.ts"],
        },
        SpecMatcherCase {
            name: "recursive wildcard",
            specs: vec!["**/*.ts"],
            base_path: "/project",
            usage: Usage::Files,
            use_case_sensitive_file_names: true,
            matching_paths: vec!["/project/a.ts", "/project/sub/a.ts", "/project/sub/deep/a.ts"],
            non_matching_paths: vec!["/project/a.js"],
        },
        SpecMatcherCase {
            name: "exclude pattern",
            specs: vec!["node_modules"],
            base_path: "/project",
            usage: Usage::Exclude,
            use_case_sensitive_file_names: true,
            matching_paths: vec!["/project/node_modules/foo"],
            non_matching_paths: vec!["/project/node_modules", "/project/src"],
        },
        SpecMatcherCase {
            name: "case insensitive",
            specs: vec!["*.ts"],
            base_path: "/project",
            usage: Usage::Files,
            use_case_sensitive_file_names: false,
            matching_paths: vec!["/project/A.TS", "/project/B.Ts"],
            non_matching_paths: vec!["/project/a.js"],
        },
        SpecMatcherCase {
            name: "multiple specs",
            specs: vec!["*.ts", "*.tsx"],
            base_path: "/project",
            usage: Usage::Files,
            use_case_sensitive_file_names: true,
            matching_paths: vec!["/project/a.ts", "/project/b.tsx"],
            non_matching_paths: vec!["/project/a.js"],
        },
    ];

    for tc in &cases {
        let name = tc.name;
        let Some(matcher) = new_spec_matcher(&tc.specs, tc.base_path, tc.usage, tc.use_case_sensitive_file_names) else {
            panic!("{name}: matcher should not be nil");
        };
        for path in &tc.matching_paths {
            assert!(matcher.match_string(path), "{name}: should match: {path}");
        }
        for path in &tc.non_matching_paths {
            assert!(!matcher.match_string(path), "{name}: should not match: {path}");
        }
    }
}

#[test]
fn test_spec_matcher_match_string() {
    struct Case {
        name: &'static str,
        specs: Vec<&'static str>,
        base_path: &'static str,
        usage: Usage,
        use_case_sensitive_file_names: bool,
        paths: Vec<&'static str>,
        expected: Vec<bool>,
    }
    let cases = vec![
        Case {
            name: "simple wildcard files",
            specs: vec!["*.ts"],
            base_path: "/project",
            usage: Usage::Files,
            use_case_sensitive_file_names: true,
            paths: vec!["/project/a.ts", "/project/sub/a.ts", "/project/a.js"],
            expected: vec![true, false, false],
        },
        Case {
            name: "recursive wildcard files",
            specs: vec!["**/*.ts"],
            base_path: "/project",
            usage: Usage::Files,
            use_case_sensitive_file_names: true,
            paths: vec!["/project/a.ts", "/project/sub/a.ts", "/project/a.js"],
            expected: vec![true, true, false],
        },
        Case {
            name: "exclude pattern matches prefix",
            specs: vec!["node_modules"],
            base_path: "/project",
            usage: Usage::Exclude,
            use_case_sensitive_file_names: true,
            paths: vec!["/project/node_modules", "/project/node_modules/foo", "/project/src"],
            expected: vec![false, true, false],
        },
    ];

    for tc in &cases {
        let name = tc.name;
        assert_eq!(tc.paths.len(), tc.expected.len(), "{name}");
        let m = new_spec_matcher(&tc.specs, tc.base_path, tc.usage, tc.use_case_sensitive_file_names);
        let Some(m) = m else { panic!("{name}: matcher should not be nil") };
        for (i, path) in tc.paths.iter().enumerate() {
            assert_eq!(m.match_string(path), tc.expected[i], "{name}: path: {path}");
        }
    }
}

#[test]
fn test_single_spec_matcher_match_string() {
    struct Case {
        name: &'static str,
        spec: &'static str,
        base_path: &'static str,
        usage: Usage,
        use_case_sensitive_file_names: bool,
        paths: Vec<&'static str>,
        expected: Vec<bool>,
    }
    let cases = vec![
        Case {
            name: "single spec wildcard",
            spec: "*.ts",
            base_path: "/project",
            usage: Usage::Files,
            use_case_sensitive_file_names: true,
            paths: vec!["/project/a.ts", "/project/sub/a.ts", "/project/a.js"],
            expected: vec![true, false, false],
        },
        Case {
            name: "single spec trailing starstar exclude allowed",
            spec: "**",
            base_path: "/project",
            usage: Usage::Exclude,
            use_case_sensitive_file_names: true,
            paths: vec!["/project/a.ts", "/project/sub/a.ts"],
            expected: vec![true, true],
        },
    ];

    for tc in &cases {
        let name = tc.name;
        assert_eq!(tc.paths.len(), tc.expected.len(), "{name}");
        let m = new_spec_matcher(&[tc.spec], tc.base_path, tc.usage, tc.use_case_sensitive_file_names);
        let Some(m) = m else { panic!("{name}: matcher should not be nil") };
        for (i, path) in tc.paths.iter().enumerate() {
            assert_eq!(m.match_string(path), tc.expected[i], "{name}: path: {path}");
        }
    }
}

#[test]
fn test_spec_matchers_match_index() {
    struct Case {
        name: &'static str,
        specs: Vec<&'static str>,
        base_path: &'static str,
        usage: Usage,
        use_case_sensitive_file_names: bool,
        paths: Vec<&'static str>,
        expected: Vec<i32>,
    }
    let cases = vec![
        Case {
            name: "index lookup prefers first match",
            specs: vec!["*.ts", "*.tsx"],
            base_path: "/project",
            usage: Usage::Files,
            use_case_sensitive_file_names: true,
            paths: vec!["/project/a.ts", "/project/a.tsx", "/project/a.js"],
            expected: vec![0, 1, -1],
        },
        Case {
            name: "exclude index lookup",
            specs: vec!["node_modules", "bower_components"],
            base_path: "/project",
            usage: Usage::Exclude,
            use_case_sensitive_file_names: true,
            paths: vec![
                "/project/node_modules",
                "/project/node_modules/foo",
                "/project/bower_components",
                "/project/bower_components/bar",
                "/project/src",
            ],
            expected: vec![-1, 0, -1, 1, -1],
        },
    ];

    for tc in &cases {
        let name = tc.name;
        assert_eq!(tc.paths.len(), tc.expected.len(), "{name}");
        let m = new_spec_matcher(&tc.specs, tc.base_path, tc.usage, tc.use_case_sensitive_file_names);
        let Some(m) = m else { panic!("{name}: matcher should not be nil") };
        for (i, path) in tc.paths.iter().enumerate() {
            assert_eq!(m.match_index(path), tc.expected[i], "{name}: path: {path}");
        }
    }
}

#[test]
fn test_single_spec_matcher() {
    struct Case {
        name: &'static str,
        spec: &'static str,
        base_path: &'static str,
        usage: Usage,
        use_case_sensitive_file_names: bool,
        expect_nil: bool,
        matching_paths: Vec<&'static str>,
        non_matching_paths: Vec<&'static str>,
    }
    let cases = vec![
        Case {
            name: "simple spec",
            spec: "*.ts",
            base_path: "/project",
            usage: Usage::Files,
            use_case_sensitive_file_names: true,
            expect_nil: false,
            matching_paths: vec!["/project/a.ts"],
            non_matching_paths: vec!["/project/a.js"],
        },
        Case {
            name: "trailing ** non-exclude returns nil",
            spec: "**",
            base_path: "/project",
            usage: Usage::Files,
            use_case_sensitive_file_names: true,
            expect_nil: true,
            matching_paths: vec![],
            non_matching_paths: vec![],
        },
        Case {
            name: "trailing ** exclude works",
            spec: "**",
            base_path: "/project",
            usage: Usage::Exclude,
            use_case_sensitive_file_names: true,
            expect_nil: false,
            matching_paths: vec!["/project/anything", "/project/deep/path"],
            non_matching_paths: vec![],
        },
    ];

    for tc in &cases {
        let name = tc.name;
        let matcher = new_spec_matcher(&[tc.spec], tc.base_path, tc.usage, tc.use_case_sensitive_file_names);
        if tc.expect_nil {
            assert!(matcher.is_none(), "{name}: should be nil");
            continue;
        }
        let Some(matcher) = matcher else { panic!("{name}: matcher should not be nil") };
        for path in &tc.matching_paths {
            assert!(matcher.match_string(path), "{name}: should match: {path}");
        }
        for path in &tc.non_matching_paths {
            assert!(!matcher.match_string(path), "{name}: should not match: {path}");
        }
    }
}

#[test]
fn test_spec_matchers() {
    struct Case {
        name: &'static str,
        specs: Vec<&'static str>,
        base_path: &'static str,
        usage: Usage,
        use_case_sensitive_file_names: bool,
        expect_nil: bool,
        path_to_index: Vec<(&'static str, i32)>,
    }
    let cases = vec![
        Case {
            name: "multiple specs return correct index",
            specs: vec!["*.ts", "*.tsx", "*.js"],
            base_path: "/project",
            usage: Usage::Files,
            use_case_sensitive_file_names: true,
            expect_nil: false,
            path_to_index: vec![
                ("/project/a.ts", 0),
                ("/project/b.tsx", 1),
                ("/project/c.js", 2),
                ("/project/d.css", -1), // no match
            ],
        },
        Case {
            name: "empty specs returns nil",
            specs: vec![],
            base_path: "/project",
            usage: Usage::Files,
            use_case_sensitive_file_names: true,
            expect_nil: true,
            path_to_index: vec![],
        },
    ];

    for tc in &cases {
        let name = tc.name;
        let matchers = new_spec_matcher(&tc.specs, tc.base_path, tc.usage, tc.use_case_sensitive_file_names);
        if tc.expect_nil {
            assert!(matchers.is_none(), "{name}: should be nil");
            continue;
        }
        let Some(matchers) = matchers else { panic!("{name}: matchers should not be nil") };
        for &(path, expected_index) in &tc.path_to_index {
            let got_index = matchers.match_index(path);
            assert_eq!(got_index, expected_index, "{name}: path: {path}");
        }
    }
}

// Returns the part as text sliced from the virtual string prefix+suffix, plus the next offset.
fn next_part(prefix: &str, suffix: &str, offset: usize) -> Option<(String, usize)> {
    let virt = format!("{prefix}{suffix}");
    next_path_part_parts(prefix, suffix, offset).map(|(start, end, next)| (virt[start..end].to_string(), next))
}

// TestGlobPatternInternals tests internal glob pattern matching logic
// to ensure edge cases are covered that may not be hit by ReadDirectory tests
#[test]
fn test_glob_pattern_internals_next_path_part_handles_consecutive_slashes() {
    // Test path with consecutive slashes
    let path = "/dev//foo///bar";

    // First call - returns empty for root
    let (part, offset) = next_part(path, "", 0).expect("ok");
    assert_eq!(part, "");
    assert_eq!(offset, 1);

    // Second call - should skip consecutive slashes after /dev
    let (part, offset) = next_part(path, "", 1).expect("ok");
    assert_eq!(part, "dev");

    // Third call - should skip the double slashes before foo
    let (part, offset) = next_part(path, "", offset).expect("ok");
    assert_eq!(part, "foo");

    // Fourth call - should skip the triple slashes before bar
    let (part, _) = next_part(path, "", offset).expect("ok");
    assert_eq!(part, "bar");
}

#[test]
fn test_glob_pattern_internals_next_path_part_handles_path_ending_with_slashes() {
    let path = "/dev/";

    // Skip to after "dev"
    let (_, offset) = next_part(path, "", 0).expect("ok"); // root
    let (_, offset) = next_part(path, "", offset).expect("ok"); // dev
    // Now at trailing slash, should return not ok
    assert!(next_part(path, "", offset).is_none());
}

#[test]
fn test_glob_pattern_internals_next_path_part_parts_handles_empty_prefix() {
    let path = "/dev//foo";

    let (part, offset) = next_part("", path, 0).expect("ok");
    assert_eq!(part, "");
    assert_eq!(offset, 1);

    let (part, offset) = next_part("", path, offset).expect("ok");
    assert_eq!(part, "dev");

    let (part, _) = next_part("", path, offset).expect("ok");
    assert_eq!(part, "foo");
}

#[test]
fn test_glob_pattern_internals_next_path_part_parts_returns_not_ok_when_only_slashes_remain() {
    let prefix = "/dev/";
    let suffix = "foo";

    let (_, offset) = next_part(prefix, suffix, 0).expect("ok"); // root

    let (part, offset) = next_part(prefix, suffix, offset).expect("ok"); // dev
    assert_eq!(part, "dev");

    let (part, offset) = next_part(prefix, suffix, offset).expect("ok"); // foo
    assert_eq!(part, "foo");
    assert_eq!(offset, prefix.len() + suffix.len());

    assert!(next_part(prefix, suffix, offset).is_none());
}

#[test]
fn test_glob_pattern_internals_next_path_part_parts_parses_from_suffix_region() {
    let prefix = "/";
    let suffix = "a";

    let (part, offset) = next_part(prefix, suffix, 0).expect("ok"); // root
    assert_eq!(part, "");
    assert_eq!(offset, 1);

    let (part, _) = next_part(prefix, suffix, offset).expect("ok");
    assert_eq!(part, "a");
}

#[test]
fn test_glob_pattern_internals_question_mark_segment_at_end_of_string() {
    // Create pattern with question mark that should fail when string is exhausted
    let p = compile_glob_pattern("a?", "/", Usage::Files, true).expect("ok");

    // Should match "ab"
    assert!(p.matches("/ab"));

    // Should NOT match "a" (question mark requires a character)
    assert!(!p.matches("/a"));
}

#[test]
fn test_glob_pattern_internals_star_segment_with_complex_pattern() {
    // Pattern like "a*b*c" requires backtracking in star matching
    let p = compile_glob_pattern("a*b*c", "/", Usage::Files, true).expect("ok");

    // Should match "abc"
    assert!(p.matches("/abc"));

    // Should match "aXbYc"
    assert!(p.matches("/aXbYc"));

    // Should match "aXXXbYYYc"
    assert!(p.matches("/aXXXbYYYc"));

    // Should NOT match "aXbY" (no trailing c)
    assert!(!p.matches("/aXbY"));
}

#[test]
fn test_glob_pattern_internals_ensure_trailing_slash_with_existing_slash() {
    // Test that ensureTrailingSlash doesn't double-add slashes
    let result = ensure_trailing_slash("/dev/");
    assert_eq!(result, "/dev/");

    let result = ensure_trailing_slash("/");
    assert_eq!(result, "/");
}

#[test]
fn test_glob_pattern_internals_ensure_trailing_slash_with_empty_string() {
    let result = ensure_trailing_slash("");
    assert_eq!(result, "");
}

#[test]
fn test_glob_pattern_internals_literal_component_with_package_folder_in_include() {
    // When a literal include path goes through a package folder,
    // the skipPackageFolders flag on literal components should not block it
    // because literal components in includes don't have skipPackageFolders=true
    let host = from_map([("/dev/node_modules/pkg/index.ts", "")], false);

    // Explicit literal path should work
    let got = match_files("/dev", &[".ts"], &[] as &[&str], &["node_modules/pkg/index.ts"], false, "/", UNLIMITED_DEPTH, &host);
    assert!(has(&got, "/dev/node_modules/pkg/index.ts"), "{got:?}");
}

// TestMatchSegmentsEdgeCases tests edge cases in the matchSegments function
#[test]
fn test_match_segments_edge_cases_question_mark_before_slash_in_string() {
    // This tests the case where question mark encounters a slash character
    // which should fail since ? doesn't match /
    let p = compile_glob_pattern("a?b", "/", Usage::Files, true).expect("ok");

    // "a/b" should not match "a?b" pattern since ? shouldn't match /
    // But this is a single component pattern, so / wouldn't be in the component
    // We need to test this within the segment matching

    // Create a pattern that will exercise question mark matching edge cases
    assert!(p.matches("/aXb")); // X matches ?
    assert!(!p.matches("/ab")); // nothing to match ?
    assert!(!p.matches("/aXYb")); // XY is too many chars for ?
}

#[test]
fn test_match_segments_edge_cases_star_with_no_trailing_content() {
    // Test that star can match to end of string
    let p = compile_glob_pattern("a*", "/", Usage::Files, true).expect("ok");

    assert!(p.matches("/a"));
    assert!(p.matches("/abc"));
    assert!(p.matches("/aXYZ"));
}

#[test]
fn test_match_segments_edge_cases_multiple_stars_in_pattern() {
    // Test patterns with multiple stars that require backtracking
    let p = compile_glob_pattern("*a*", "/", Usage::Files, true).expect("ok");

    assert!(p.matches("/a"));
    assert!(p.matches("/Xa"));
    assert!(p.matches("/aX"));
    assert!(p.matches("/XaY"));
    assert!(!p.matches("/XYZ")); // no 'a'
}

#[test]
fn test_match_segments_edge_cases_multiple_stars_requiring_backtracking() {
    // These patterns require proper backtracking to match correctly.
    // A naive greedy algorithm would fail on these.

    // Pattern: *a*a - must find two 'a' characters
    let p1 = compile_glob_pattern("*a*a", "/", Usage::Files, true).expect("ok");
    assert!(p1.matches("/aa")); // minimal: first * matches "", second * matches ""
    assert!(p1.matches("/Xaa")); // first * matches "X"
    assert!(p1.matches("/aXa")); // second * matches "X"
    assert!(p1.matches("/XaYa")); // both * match chars
    assert!(p1.matches("/aaaa")); // multiple a's
    assert!(!p1.matches("/a")); // only one 'a'
    assert!(!p1.matches("/Xa")); // only one 'a'
    assert!(!p1.matches("/aX")); // only one 'a', doesn't end with 'a'
    assert!(!p1.matches("/XaYaZ")); // doesn't end with 'a'

    // Pattern: *a*b*c - must find a, then b, then c in order
    let p2 = compile_glob_pattern("*a*b*c", "/", Usage::Files, true).expect("ok");
    assert!(p2.matches("/abc")); // minimal
    assert!(p2.matches("/XaYbZc")); // chars between
    assert!(p2.matches("/aXbYc")); // chars between
    assert!(p2.matches("/aaabbbccc")); // repeated chars
    assert!(!p2.matches("/ab")); // missing c
    assert!(!p2.matches("/ac")); // missing b
    assert!(!p2.matches("/cba")); // wrong order
    assert!(!p2.matches("/abcX")); // doesn't end with c

    // Pattern: *a*a*a - must find three 'a' characters
    let p3 = compile_glob_pattern("*a*a*a", "/", Usage::Files, true).expect("ok");
    assert!(p3.matches("/aaa"));
    assert!(p3.matches("/aXaYa"));
    assert!(p3.matches("/XaYaZa"));
    assert!(!p3.matches("/aa")); // only two 'a's
    assert!(!p3.matches("/aaX")); // doesn't end with 'a'

    // Pattern: a*b*a - starts with a, ends with a, has b in middle
    let p4 = compile_glob_pattern("a*b*a", "/", Usage::Files, true).expect("ok");
    assert!(p4.matches("/aba"));
    assert!(p4.matches("/aXbYa"));
    assert!(p4.matches("/abba")); // b appears, ends with a
    assert!(!p4.matches("/ab")); // doesn't end with a
    assert!(!p4.matches("/aba ")); // trailing space
    assert!(!p4.matches("/Xaba")); // doesn't start with a (hidden file rule may affect)
}

#[test]
fn test_match_segments_edge_cases_pathological_pattern_performance() {
    // This pattern could cause exponential backtracking in naive implementations.
    // Pattern: *a*a*a*a*b against "aaaaaaaaaaaaaaaa" (no b)
    // Should return false quickly, not hang.
    let p = compile_glob_pattern("*a*a*a*a*b", "/", Usage::Files, true).expect("ok");

    // These should complete quickly (not hang)
    assert!(!p.matches("/aaaaaaaaaaaaaaaa")); // no 'b' at end
    assert!(!p.matches("/aaaaaaaaaaaaaaaaX")); // ends with X not b
    assert!(p.matches("/aaaab")); // minimal match
    assert!(p.matches("/XaYaZaWab")); // complex match
}

#[test]
fn test_match_segments_edge_cases_literal_segment_not_matching() {
    // Test literal segment that's longer than remaining string
    let p = compile_glob_pattern("abcdefgh.ts", "/", Usage::Files, true).expect("ok");

    assert!(!p.matches("/abc.ts")); // different literal
    assert!(p.matches("/abcdefgh.ts")); // exact match
}

#[test]
fn test_match_segments_edge_cases_question_mark_matches_multi_byte_unicode_rune() {
    // ? should match one full Unicode codepoint, not one byte.
    // 'é' is 2 bytes in UTF-8, '🎉' is 4 bytes, '中' is 3 bytes.

    let p1 = compile_glob_pattern("?.ts", "/", Usage::Files, true).expect("ok");

    assert!(p1.matches("/a.ts")); // single ASCII char
    assert!(p1.matches("/é.ts")); // 2-byte rune
    assert!(p1.matches("/中.ts")); // 3-byte rune
    assert!(p1.matches("/🎉.ts")); // 4-byte rune (surrogate pair in UTF-16)
    assert!(!p1.matches("/.ts")); // empty - no char for ? to match
    assert!(!p1.matches("/ab.ts")); // two chars

    // Two question marks should match exactly two runes
    let p2 = compile_glob_pattern("??.ts", "/", Usage::Files, true).expect("ok");

    assert!(p2.matches("/ab.ts")); // two ASCII chars
    assert!(p2.matches("/é中.ts")); // two multi-byte runes
    assert!(p2.matches("/🎉é.ts")); // 4-byte + 2-byte runes
    assert!(!p2.matches("/a.ts")); // only one char
    assert!(!p2.matches("/abc.ts")); // three chars
}

#[test]
fn test_match_segments_edge_cases_star_matches_multi_byte_unicode_runes_correctly() {
    // * should advance by full runes during backtracking.

    // Pattern: *é.ts - anything ending in é.ts
    let p = compile_glob_pattern("*é.ts", "/", Usage::Files, true).expect("ok");

    assert!(p.matches("/é.ts"));
    assert!(p.matches("/café.ts"));
    assert!(!p.matches("/cafe.ts")); // 'e' != 'é'

    // Pattern: *🎉* - contains 🎉 somewhere
    let p2 = compile_glob_pattern("*🎉*", "/", Usage::Files, true).expect("ok");

    assert!(p2.matches("/🎉"));
    assert!(p2.matches("/a🎉b"));
    assert!(!p2.matches("/abc"));
}

// TestReadDirectoryConsecutiveSlashes tests handling of paths with consecutive slashes
#[test]
fn test_read_directory_consecutive_slashes() {
    let host = from_map([("/dev/a.ts", ""), ("/dev/x/b.ts", "")], false);

    // The matchFilesNoRegex function normalizes paths, but we can test internal handling
    let got = match_files("/dev", &[".ts"], &[] as &[&str], &["**/*.ts"], false, "/", UNLIMITED_DEPTH, &host);
    assert!(got.len() >= 2, "should find files");
    assert!(has(&got, "/dev/a.ts"), "{got:?}");
    assert!(has(&got, "/dev/x/b.ts"), "{got:?}");
}

// TestGlobPatternLiteralWithPackageFolders tests literal component behavior with package folders
#[test]
fn test_glob_pattern_literal_with_package_folders_wildcard_skips_package_folders() {
    // Wildcard patterns should skip node_modules
    let host = from_map([("/dev/a.ts", ""), ("/dev/node_modules/b.ts", "")], false);

    let got = match_files("/dev", &[".ts"], &[] as &[&str], &["*/*.ts"], false, "/", UNLIMITED_DEPTH, &host);
    assert!(!has(&got, "/dev/node_modules/b.ts"), "should skip node_modules with wildcard");
}

#[test]
fn test_glob_pattern_literal_with_package_folders_explicit_literal_includes_package_folder() {
    // Explicit literal paths should include package folders
    let host = from_map([("/dev/node_modules/b.ts", "")], false);

    let got = match_files("/dev", &[".ts"], &[] as &[&str], &["node_modules/b.ts"], false, "/", UNLIMITED_DEPTH, &host);
    assert!(has(&got, "/dev/node_modules/b.ts"), "should include explicit node_modules path");
}

// TestGetBasePathsCaseSensitivity verifies that getBasePaths uses the correct
// case-sensitivity when deduplicating base paths. On a case-sensitive file system,
// paths that differ only by case (e.g., "/Dev/src" and "/dev/src") are distinct
// and should not be deduplicated.
#[test]
fn test_get_base_paths_case_sensitivity_case_sensitive_does_not_dedup_differently_cased_paths() {
    // On a case-sensitive file system, /root/src/Dev and /root/src/dev are distinct directories.
    // When they're both included as base paths, they should not be deduplicated.
    // Use include patterns that point to directories outside the root path so the root
    // path doesn't subsume them via containsPath.
    let base_paths = get_base_paths("/root", &["../Other/**/*.ts", "../other/**/*.ts"], true /*caseSensitive*/);
    // Both /Other and /other should appear because they differ by case on a case-sensitive FS.
    assert!(has(&base_paths, "/Other"), "expected /Other in base paths: {base_paths:?}");
    assert!(has(&base_paths, "/other"), "expected /other in base paths: {base_paths:?}");
}

#[test]
fn test_get_base_paths_case_sensitivity_case_insensitive_dedups_differently_cased_paths() {
    // On a case-insensitive file system, /Other and /other refer to the same directory;
    // only one should appear.
    let base_paths = get_base_paths("/root", &["../Other/**/*.ts", "../other/**/*.ts"], false /*caseSensitive*/);
    let count = base_paths.iter().filter(|bp| *bp == "/Other" || *bp == "/other").count();
    assert!(count <= 1, "expected at most one of /Other or /other in base paths: {base_paths:?}");
}
