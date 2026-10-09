use std::sync::atomic::{AtomicUsize, Ordering};

use tsrs_core::tspath;

use super::*;

// A fresh directory under the system temp dir (t.TempDir()).
pub(super) fn temp_dir() -> String {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!("tsrs_vfs_test_{}_{}", std::process::id(), COUNTER.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

pub(super) fn mklink(target: &str, link: &str, is_dir: bool) {
    #[cfg(unix)]
    {
        let _ = is_dir;
        std::os::unix::fs::symlink(target, link).unwrap();
    }
    #[cfg(windows)]
    {
        if is_dir {
            std::os::windows::fs::symlink_dir(target, link).unwrap();
        } else {
            std::os::windows::fs::symlink_file(target, link).unwrap();
        }
    }
}

#[test]
fn test_os_read_file() {
    let fs = fs();

    let cargo_toml = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let cargo_toml_path = tspath::normalize_path(cargo_toml);

    let expected = std::fs::read_to_string(cargo_toml).unwrap();

    let contents = fs.read_file(&cargo_toml_path);
    assert_eq!(contents.as_deref(), Some(expected.as_str()));
}

#[test]
fn test_os_realpath() {
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let home = tspath::normalize_path(&home.to_string_lossy());

    let mut expected = home.clone();
    if cfg!(windows) {
        // Windows drive letters can be lowercase, but realpath will always return uppercase.
        expected = format!("{}{}", expected[..1].to_uppercase(), &expected[1..]);
    }
    let realpath = fs().realpath(&home);
    assert_eq!(realpath, expected);
}

#[test]
fn test_os_use_case_sensitive_file_names() {
    let fs = fs();

    // Just check that it works.
    fs.use_case_sensitive_file_names();

    if cfg!(windows) {
        assert!(!fs.use_case_sensitive_file_names());
    } else if cfg!(target_os = "linux") {
        assert!(fs.use_case_sensitive_file_names());
    }
}

#[test]
fn test_os_bom_and_directories() {
    let tmp = tspath::normalize_slashes(&temp_dir());
    let fs = fs();

    let file = tspath::combine_paths(&tmp, &["a/b/c.ts"]);
    fs.write_file(&file, "\u{FEFF}hello").unwrap();
    assert_eq!(fs.read_file(&file).as_deref(), Some("hello"));
    assert!(fs.file_exists(&file));
    assert!(!fs.directory_exists(&file));
    assert!(fs.directory_exists(&tspath::combine_paths(&tmp, &["a/b"])));
    assert_eq!(fs.read_file(&tspath::combine_paths(&tmp, &["a"])), None);

    fs.append_file(&file, " world").unwrap();
    assert_eq!(fs.read_file(&file).as_deref(), Some("hello world"));

    let stat = fs.stat(&file).unwrap();
    assert_eq!(stat.name(), "c.ts");
    assert!(stat.mode().is_regular());

    fs.remove(&tspath::combine_paths(&tmp, &["a"])).unwrap();
    assert!(!fs.file_exists(&file));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_clean() {
    for (input, want) in [
        ("", "."),
        ("abc", "abc"),
        ("abc/def", "abc/def"),
        ("a/b/c", "a/b/c"),
        (".", "."),
        ("..", ".."),
        ("../..", "../.."),
        ("../../abc", "../../abc"),
        ("/abc", "/abc"),
        ("/", "/"),
        ("abc/", "abc"),
        ("abc/def/", "abc/def"),
        ("/abc/", "/abc"),
        ("abc//def//ghi", "abc/def/ghi"),
        ("//abc", "/abc"),
        ("abc/./def", "abc/def"),
        ("/./abc/def", "/abc/def"),
        ("abc/def/ghi/../jkl", "abc/def/jkl"),
        ("abc/def/../ghi/../jkl", "abc/jkl"),
        ("abc/def/..", "abc"),
        ("abc/def/../..", "."),
        ("/abc/def/../..", "/"),
        ("abc/def/../../..", ".."),
        ("/abc/def/../../..", "/"),
        ("abc/def/../../../ghi/jkl/../../../mno", "../../mno"),
        ("abc/./../def", "def"),
        ("abc//./../def", "def"),
        ("abc/../../././../def", "../../def"),
    ] {
        assert_eq!(clean(input), want, "clean({:?})", input);
    }
}
