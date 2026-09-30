use std::panic::{catch_unwind, AssertUnwindSafe};

use super::*;
use crate::internal::IoFS;
use crate::iovfs::RealpathFS;
use crate::FS;

fn assert_panics(f: impl FnOnce(), expected: &str) {
    let err = catch_unwind(AssertUnwindSafe(f)).expect_err("expected panic");
    let msg = if let Some(s) = err.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = err.downcast_ref::<&str>() {
        s.to_string()
    } else {
        String::new()
    };
    assert_eq!(msg, expected);
}

fn map_fs(entries: &[(&str, &str)]) -> Vec<(String, MapFile)> {
    entries.iter().map(|(k, v)| (k.to_string(), MapFile::from(*v))).collect()
}

fn dir_entries_to_names(entries: &[IoDirEntry]) -> Vec<String> {
    entries.iter().map(|e| e.name.clone()).collect()
}

#[test]
fn test_insensitive() {
    let contents = b"bar".to_vec();

    let vfs = convert_map_fs(map_fs(&[("foo/bar/baz", "bar"), ("foo/bar2/baz2", "bar"), ("foo/bar3/baz3", "bar")]), false);

    let sensitive = vfs.read_file("foo/bar/baz").unwrap();
    assert_eq!(sensitive, contents);
    vfs.stat("foo/bar/baz").unwrap();
    let sensitive_real_path = vfs.realpath("foo/bar/baz").unwrap();
    assert_eq!(sensitive_real_path, "foo/bar/baz");
    let entries = vfs.read_dir("foo").unwrap();
    assert_eq!(dir_entries_to_names(&entries), vec!["bar", "bar2", "bar3"]);

    let err = vfs.realpath("does/not/exist").unwrap_err();
    assert!(err.to_string().contains("file does not exist"));
    let err = vfs.stat("does/not/exist").unwrap_err();
    assert!(err.to_string().contains("file does not exist"));

    let insensitive = vfs.read_file("Foo/Bar/Baz").unwrap();
    assert_eq!(insensitive, contents);
    vfs.stat("Foo/Bar/Baz").unwrap();
    let insensitive_real_path = vfs.realpath("Foo/Bar/Baz").unwrap();
    assert_eq!(insensitive_real_path, "foo/bar/baz");
    let entries = vfs.read_dir("Foo").unwrap();
    assert_eq!(dir_entries_to_names(&entries), vec!["bar", "bar2", "bar3"]);

    let err = vfs.realpath("Does/Not/Exist").unwrap_err();
    assert!(err.to_string().contains("file does not exist"));
    let err = vfs.stat("Does/Not/Exist").unwrap_err();
    assert!(err.to_string().contains("file does not exist"));
}

#[test]
fn test_insensitive_upper() {
    let contents = b"bar".to_vec();

    let vfs = convert_map_fs(map_fs(&[("Foo/Bar/Baz", "bar"), ("Foo/Bar2/Baz2", "bar"), ("Foo/Bar3/Baz3", "bar")]), false);

    let sensitive = vfs.read_file("foo/bar/baz").unwrap();
    assert_eq!(sensitive, contents);
    vfs.stat("foo/bar/baz").unwrap();
    let entries = vfs.read_dir("foo").unwrap();
    assert_eq!(dir_entries_to_names(&entries), vec!["Bar", "Bar2", "Bar3"]);

    let insensitive = vfs.read_file("Foo/Bar/Baz").unwrap();
    assert_eq!(insensitive, contents);
    vfs.stat("Foo/Bar/Baz").unwrap();
    let entries = vfs.read_dir("Foo").unwrap();
    assert_eq!(dir_entries_to_names(&entries), vec!["Bar", "Bar2", "Bar3"]);
}

#[test]
fn test_sensitive() {
    let contents = b"bar".to_vec();

    let vfs = convert_map_fs(map_fs(&[("foo/bar/baz", "bar"), ("foo/bar2/baz2", "bar"), ("foo/bar3/baz3", "bar")]), true);

    let sensitive = vfs.read_file("foo/bar/baz").unwrap();
    assert_eq!(sensitive, contents);
    vfs.stat("foo/bar/baz").unwrap();

    let err = vfs.read_file("Foo/Bar/Baz").unwrap_err();
    assert!(err.to_string().contains("file does not exist"));
}

#[test]
fn test_sensitive_duplicate_path() {
    assert_panics(
        || {
            convert_map_fs(map_fs(&[("foo", "bar"), ("Foo", "baz")]), false);
        },
        r#"duplicate path: "Foo" and "foo" have the same canonical path"#,
    );
}

#[test]
fn test_insensitive_duplicate_path() {
    convert_map_fs(map_fs(&[("foo", "bar"), ("Foo", "baz")]), true);
}

#[test]
fn test_writable_fs() {
    let fs = from_map(Vec::<(&str, &str)>::new(), false);

    fs.write_file("/foo/bar/baz", "hello, world").unwrap();

    assert_eq!(fs.read_file("/foo/bar/baz").as_deref(), Some("hello, world"));

    fs.write_file("/foo/bar/baz", "goodbye, world").unwrap();

    assert_eq!(fs.read_file("/foo/bar/baz").as_deref(), Some("goodbye, world"));

    let err = fs.write_file("/foo/bar/baz/oops", "goodbye, world").unwrap_err();
    assert!(err.contains(r#"mkdir "foo/bar/baz": path exists but is not a directory"#), "{}", err);
}

#[test]
fn test_writable_fs_delete() {
    let fs = from_map(Vec::<(&str, &str)>::new(), false);

    let _ = fs.write_file("/foo/bar/file.ts", "remove");
    assert!(fs.file_exists("/foo/bar/file.ts"));
    fs.remove("/foo/bar/file.ts").unwrap();
    assert!(!fs.file_exists("/foo/bar/file.ts"));

    let _ = fs.write_file("/foo/bar/test/remove2.ts", "remove2");
    assert!(fs.directory_exists("/foo/bar/test"));
    fs.remove("/foo/bar/test").unwrap();
    assert!(!fs.file_exists("/foo/bar/test/remove2.ts"));
    assert!(!fs.directory_exists("/foo/bar/test"));

    // no errors when removing file/dir that does not exist
    fs.remove("/foo/bar/test").unwrap();
    fs.remove("/foo/bar/file.ts").unwrap();

    let _ = fs.write_file("/foo/barbar", "remove2");
    let _ = fs.remove("/foo/bar");
    assert!(fs.file_exists("/foo/barbar"));
}

#[test]
fn test_stress() {
    let fs = from_map(Vec::<(&str, &str)>::new(), false);

    let ops: Vec<fn(&IoVFS<MapFS>)> = vec![
        |fs| {
            let _ = fs.write_file("/foo/bar/baz.txt", "hello, world");
        },
        |fs| {
            fs.read_file("/foo/bar/baz.txt");
        },
        |fs| {
            fs.directory_exists("/foo/bar");
        },
        |fs| {
            fs.file_exists("/foo/bar");
        },
        |fs| {
            fs.file_exists("/foo/bar/baz.txt");
        },
        |fs| {
            fs.get_accessible_entries("/foo/bar");
        },
        |fs| {
            fs.realpath("/foo/bar/baz.txt");
        },
        |fs| {
            fs.stat("/foo/bar/baz.txt");
        },
    ];

    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    std::thread::scope(|s| {
        for t in 0..threads {
            let fs = &fs;
            let ops = &ops;
            s.spawn(move || {
                let mut random_ops = ops.clone();
                random_ops.rotate_left(t % ops.len());
                for i in 0..10000 {
                    random_ops[i % random_ops.len()](fs);
                }
            });
        }
    });
}

#[test]
fn test_parent_dir_file() {
    assert_panics(
        || {
            convert_map_fs(map_fs(&[("foo", "bar"), ("foo/oops", "baz")]), false);
        },
        r#"failed to create intermediate directories for "foo/oops": mkdir "foo": path exists but is not a directory"#,
    );
}

#[test]
fn test_from_map_posix() {
    let fs = from_map(
        vec![
            ("/string", MapFile::from("hello, world")),
            ("/bytes", MapFile::from(b"hello, world".to_vec())),
            (
                "/mapfile",
                MapFile {
                    data: b"hello, world".to_vec(),
                    ..Default::default()
                },
            ),
        ],
        false,
    );

    assert_eq!(fs.read_file("/string").as_deref(), Some("hello, world"));
    assert_eq!(fs.read_file("/bytes").as_deref(), Some("hello, world"));
    assert_eq!(fs.read_file("/mapfile").as_deref(), Some("hello, world"));
}

#[test]
fn test_from_map_windows() {
    let fs = from_map(
        vec![
            ("c:/string", MapFile::from("hello, world")),
            ("d:/bytes", MapFile::from(b"hello, world".to_vec())),
            (
                "e:/mapfile",
                MapFile {
                    data: b"hello, world".to_vec(),
                    ..Default::default()
                },
            ),
        ],
        false,
    );

    assert_eq!(fs.read_file("c:/string").as_deref(), Some("hello, world"));
    assert_eq!(fs.read_file("d:/bytes").as_deref(), Some("hello, world"));
    assert_eq!(fs.read_file("e:/mapfile").as_deref(), Some("hello, world"));
}

#[test]
fn test_from_map_mixed() {
    assert_panics(
        || {
            from_map([("/string", "hello, world"), ("c:/bytes", "hello, world")], false);
        },
        "mixed posix and windows paths",
    );
}

#[test]
fn test_from_map_non_rooted() {
    assert_panics(
        || {
            from_map([("string", "hello, world")], false);
        },
        r#"non-rooted path "string""#,
    );
}

#[test]
fn test_from_map_non_normalized() {
    assert_panics(
        || {
            from_map([("/string/", "hello, world")], false);
        },
        r#"non-normalized path "/string/""#,
    );
}

#[test]
fn test_from_map_non_normalized2() {
    assert_panics(
        || {
            from_map([("/string/../foo", "hello, world")], false);
        },
        r#"non-normalized path "/string/../foo""#,
    );
}

#[test]
fn test_vfstest_map_fs() {
    let fs = from_map(
        [
            ("/foo.ts", "hello, world"),
            ("/dir1/file1.ts", "export const foo = 42;"),
            ("/dir1/file2.ts", "export const foo = 42;"),
            ("/dir2/file1.ts", "export const foo = 42;"),
        ],
        false,
    );

    // ReadFile
    assert_eq!(fs.read_file("/foo.ts").as_deref(), Some("hello, world"));
    assert_eq!(fs.read_file("/does/not/exist.ts"), None);

    // Realpath
    assert_eq!(FS::realpath(&fs, "/foo.ts"), "/foo.ts");
    assert_eq!(FS::realpath(&fs, "/Foo.ts"), "/foo.ts");
    assert_eq!(FS::realpath(&fs, "/does/not/exist.ts"), "/does/not/exist.ts");

    // UseCaseSensitiveFileNames
    assert!(!fs.use_case_sensitive_file_names());
}

#[test]
fn test_vfstest_map_fs_windows() {
    let fs = from_map(
        [
            ("c:/foo.ts", "hello, world"),
            ("c:/dir1/file1.ts", "export const foo = 42;"),
            ("c:/dir1/file2.ts", "export const foo = 42;"),
            ("c:/dir2/file1.ts", "export const foo = 42;"),
        ],
        false,
    );

    // ReadFile
    assert_eq!(fs.read_file("c:/foo.ts").as_deref(), Some("hello, world"));
    assert_eq!(fs.read_file("c:/does/not/exist.ts"), None);

    // Realpath
    assert_eq!(FS::realpath(&fs, "c:/foo.ts"), "c:/foo.ts");
    assert_eq!(FS::realpath(&fs, "c:/Foo.ts"), "c:/foo.ts");
    assert_eq!(FS::realpath(&fs, "c:/does/not/exist.ts"), "c:/does/not/exist.ts");
}

#[test]
fn test_bom() {
    const EXPECTED: &str = "hello, world";

    for (name, big_endian) in [("BigEndian", true), ("LittleEndian", false)] {
        let mut buf: Vec<u8> = if big_endian { vec![0xFE, 0xFF] } else { vec![0xFF, 0xFE] };
        for r in EXPECTED.encode_utf16() {
            if big_endian {
                buf.extend_from_slice(&r.to_be_bytes());
            } else {
                buf.extend_from_slice(&r.to_le_bytes());
            }
        }

        let fs = from_map([("/foo.ts", buf)], true);

        assert_eq!(fs.read_file("/foo.ts").as_deref(), Some(EXPECTED), "{}", name);
    }

    // UTF8
    let mut buf = b"\xEF\xBB\xBF".to_vec();
    buf.extend_from_slice(EXPECTED.as_bytes());
    let fs = from_map([("/foo.ts", buf)], true);

    assert_eq!(fs.read_file("/foo.ts").as_deref(), Some(EXPECTED));
}

#[test]
fn test_symlink() {
    let fs = from_map(
        vec![
            ("/foo.ts", MapFile::from("hello, world")),
            ("/symlink.ts", symlink("/foo.ts")),
            ("/some/dir/file.ts", MapFile::from("hello, world")),
            ("/some/dirlink", symlink("/some/dir")),
            ("/a", symlink("/b")),
            ("/b", symlink("/c")),
            ("/c", symlink("/d")),
            ("/d/existing.ts", MapFile::from("this is existing.ts")),
        ],
        false,
    );

    // ReadFile
    assert_eq!(fs.read_file("/symlink.ts").as_deref(), Some("hello, world"));
    assert_eq!(fs.read_file("/some/dirlink/file.ts").as_deref(), Some("hello, world"));
    assert_eq!(fs.read_file("/a/existing.ts").as_deref(), Some("this is existing.ts"));

    // Realpath
    assert_eq!(FS::realpath(&fs, "/symlink.ts"), "/foo.ts");
    assert_eq!(FS::realpath(&fs, "/some/dirlink"), "/some/dir");
    assert_eq!(FS::realpath(&fs, "/some/dirlink/file.ts"), "/some/dir/file.ts");

    // FileExists
    assert!(fs.file_exists("/symlink.ts"));
    assert!(fs.file_exists("/some/dirlink/file.ts"));
    assert!(fs.file_exists("/a/existing.ts"));

    // DirectoryExists
    assert!(fs.directory_exists("/some/dirlink"));
    assert!(fs.directory_exists("/d"));
    assert!(fs.directory_exists("/c"));
    assert!(fs.directory_exists("/b"));
    assert!(fs.directory_exists("/a"));

    // GetAccessibleEntries reports followed symlinks.
    let entries = fs.get_accessible_entries("/some");
    assert_eq!(entries.directories, vec!["dir", "dirlink"]);
    assert!(entries.symlinks.as_ref().unwrap().contains("dirlink"));
    let entries = fs.get_accessible_entries("/some/dirlink");
    assert_eq!(entries.files, vec!["file.ts"]);
}

#[test]
fn test_writable_fs_symlink() {
    let fs = from_map(
        vec![
            ("/some/dir/other.ts", MapFile::from("NOTHING")),
            ("/other.ts", symlink("/some/dir/other.ts")),
            ("/some/dirlink", symlink("/some/dir")),
            ("/brokenlink", symlink("/does/not/exist")),
            ("/a", symlink("/b")),
            ("/b", symlink("/c")),
            ("/c", symlink("/d")),
            ("/d/existing.ts", MapFile::from("hello, world")),
        ],
        false,
    );

    fs.write_file("/some/dirlink/file.ts", "hello, world").unwrap();

    assert_eq!(fs.read_file("/some/dirlink/file.ts").as_deref(), Some("hello, world"));
    assert_eq!(fs.read_file("/some/dir/file.ts").as_deref(), Some("hello, world"));

    fs.write_file("/some/dirlink/file.ts", "goodbye, world").unwrap();

    assert_eq!(fs.read_file("/some/dirlink/file.ts").as_deref(), Some("goodbye, world"));

    fs.write_file("/other.ts", "hello, world").unwrap();

    assert_eq!(fs.read_file("/other.ts").as_deref(), Some("hello, world"));
    assert_eq!(fs.read_file("/some/dir/other.ts").as_deref(), Some("hello, world"));

    let err = fs.write_file("/some/dirlink", "hello, world").unwrap_err();
    assert_eq!(err, r#"write "some/dirlink": path exists but is not a regular file"#);

    // Can't write inside a broken dir symlink
    let err = fs.write_file("/brokenlink/file.ts", "hello, world").unwrap_err();
    assert_eq!(err, r#"broken symlink "brokenlink" -> "does/not/exist""#);

    let err = fs.write_file("/brokenlink/also/wrong/file.ts", "hello, world").unwrap_err();
    assert_eq!(err, r#"broken symlink "brokenlink" -> "does/not/exist""#);

    // But we can write to a broken file symlink
    fs.write_file("/brokenlink", "hello, world").unwrap();
    assert_eq!(fs.read_file("/brokenlink").as_deref(), Some("hello, world"));
    assert_eq!(fs.read_file("/does/not/exist").as_deref(), Some("hello, world"));
}

#[test]
fn test_writable_fs_symlink_chain() {
    let fs = from_map(
        vec![
            ("/a", symlink("/b")),
            ("/b", symlink("/c")),
            ("/c", symlink("/d")),
            ("/d/existing.ts", MapFile::from("hello, world")),
        ],
        false,
    );

    fs.write_file("/a/foo/bar/new.ts", "this is new.ts").unwrap();
    assert_eq!(fs.read_file("/a/foo/bar/new.ts").as_deref(), Some("this is new.ts"));
    assert_eq!(fs.read_file("/b/foo/bar/new.ts").as_deref(), Some("this is new.ts"));
    assert_eq!(fs.read_file("/d/foo/bar/new.ts").as_deref(), Some("this is new.ts"));
}

#[test]
fn test_writable_fs_symlink_chain_not_dir() {
    let fs = from_map(
        vec![("/a", symlink("/b")), ("/b", symlink("/c")), ("/c", symlink("/d")), ("/d", MapFile::from("hello, world"))],
        false,
    );

    let err = fs.write_file("/a/foo/bar/new.ts", "this is new.ts").unwrap_err();
    assert_eq!(err, r#"mkdir "d": path exists but is not a directory"#);
}

#[test]
fn test_writable_fs_symlink_delete() {
    let fs = from_map(
        vec![
            ("/some/dir/other.ts", MapFile::from("NOTHING")),
            ("/other.ts", symlink("/some/dir/other.ts")),
            ("/some/dirlink", symlink("/some/dir")),
            ("/brokenlink", symlink("/does/not/exist")),
            ("/a", symlink("/b")),
            ("/b", symlink("/c")),
            ("/c", symlink("/d")),
            ("/d/existing.ts", MapFile::from("hello, world")),
        ],
        false,
    );

    fs.remove("/a").unwrap();
    assert!(!fs.directory_exists("/a"));
    assert!(fs.directory_exists("/b"));
    assert!(fs.directory_exists("/c"));
    assert!(fs.file_exists("/d/existing.ts"));

    // symlinks should still exist even if underlying file/dir is deleted
    fs.remove("/d").unwrap();
    assert!(!fs.directory_exists("/b"));
    assert!(!fs.directory_exists("/c"));
    assert!(!fs.directory_exists("/d"));
    assert!(!fs.file_exists("/d/again.ts"));
    fs.write_file("/d/again.ts", "d exists again").unwrap();
    assert!(fs.directory_exists("/b"));
    assert!(fs.directory_exists("/c"));
    assert_eq!(fs.read_file("/b/again.ts").as_deref(), Some("d exists again"));

    assert!(!fs.file_exists("/brokenlink"));
    assert!(!fs.directory_exists("/brokenlink"));
    fs.remove("/does/not/exist").unwrap(); // should do nothing
    assert!(!fs.file_exists("/brokenlink"));
    assert!(!fs.directory_exists("/brokenlink"));
    fs.write_file("/does/not/exist", "hello, world").unwrap();
    assert!(fs.file_exists("/brokenlink"));
}
