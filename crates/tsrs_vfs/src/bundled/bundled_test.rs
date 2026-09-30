use super::*;
use crate::osvfs;
use crate::FS;

#[test]
fn test_testing_lib_path() {
    let p = testing_lib_path();

    std::fs::metadata(&p).unwrap();

    let libdts = format!("{}/lib.d.ts", p);

    std::fs::metadata(libdts).unwrap();
}

#[test]
fn test_embedded_libs() {
    let fs = wrap_fs(osvfs::fs());

    let mut files = fs.get_accessible_entries(&lib_path()).files;
    files.sort();
    assert_eq!(files, LIB_NAMES);
}

#[test]
fn test_embedded_contents_match_source() {
    let fs = wrap_fs(osvfs::fs());
    let os = osvfs::fs();
    let lib = lib_path();
    assert_eq!(lib, "bundled:///libs");
    assert!(is_bundled(&lib));
    assert!(fs.directory_exists(&lib));
    for name in LIB_NAMES {
        let path = format!("{}/{}", lib, name);
        assert!(fs.file_exists(&path));
        let contents = fs.read_file(&path).unwrap();
        let expected = os.read_file(&format!("{}/{}", testing_lib_path(), name)).unwrap();
        assert_eq!(contents, expected, "{}", name);
        let stat = fs.stat(&path).unwrap();
        assert_eq!(stat.name(), *name);
        assert_eq!(stat.size(), contents.len() as i64);
        assert_eq!(fs.realpath(&path), path);
    }
    assert!(!fs.file_exists(&format!("{}/lib.nope.d.ts", lib)));
    assert_eq!(fs.get_accessible_entries("bundled:///").directories, vec!["libs"]);
}
