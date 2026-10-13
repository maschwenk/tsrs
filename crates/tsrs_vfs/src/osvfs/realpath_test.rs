use tsrs_core::tspath;

use super::os_test::{mklink, temp_dir};
use super::*;

fn setup_symlinks() -> (String, String) {
    let tmp = temp_dir();

    let target = format!("{tmp}/target");
    let target_file = format!("{target}/file");

    let link = format!("{tmp}/link");
    let link_file = format!("{link}/file");

    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(&target_file, "hello").unwrap();

    mklink(&target, &link, true);

    (target_file, link_file)
}

#[test]
fn test_symlink_realpath() {
    let (target_file, link_file) = setup_symlinks();

    let got_contents = std::fs::read_to_string(&link_file).unwrap();
    assert_eq!(got_contents, "hello");

    let fs = fs();

    let target_realpath = fs.realpath(&tspath::normalize_path(&target_file));
    let link_realpath = fs.realpath(&tspath::normalize_path(&link_file));

    assert_eq!(target_realpath, link_realpath, "expected realpath of target and link to be equal");
}

#[test]
fn test_realpath_relative_symlink_and_missing() {
    let tmp = tspath::normalize_slashes(&temp_dir());
    std::fs::create_dir_all(format!("{tmp}/real/sub")).unwrap();
    std::fs::write(format!("{tmp}/real/sub/f.ts"), "").unwrap();
    mklink("real/sub", &format!("{tmp}/rel"), true);

    let fs = fs();
    let real = fs.realpath(&format!("{tmp}/real/sub/f.ts"));
    assert_eq!(fs.realpath(&format!("{tmp}/rel/f.ts")), real);
    // ".." after a symlink is physical (EvalSymlinks semantics): rel/.. is real/, so this does not exist.
    let dotdot = format!("{tmp}/rel/../real/sub/f.ts");
    assert_eq!(fs.realpath(&dotdot), dotdot);
    assert_eq!(fs.realpath(&format!("{tmp}/rel/../sub/f.ts")), real);

    // A path that does not exist is returned unchanged.
    let missing = format!("{tmp}/does/not/exist.ts");
    assert_eq!(fs.realpath(&missing), missing);
}

#[test]
fn test_get_accessible_entries() {
    let tmp = temp_dir();
    let target = format!("{tmp}/target");
    let link = format!("{tmp}/link");

    std::fs::create_dir_all(&target).unwrap();
    std::fs::create_dir_all(&link).unwrap();

    let target_file1 = format!("{target}/file1");
    let target_file2 = format!("{target}/file2");

    std::fs::write(&target_file1, "hello").unwrap();
    std::fs::write(&target_file2, "world").unwrap();

    let target_dir1 = format!("{target}/dir1");
    let target_dir2 = format!("{target}/dir2");

    std::fs::create_dir_all(&target_dir1).unwrap();
    std::fs::create_dir_all(&target_dir2).unwrap();

    mklink(&target_file1, &format!("{link}/file1"), false);
    mklink(&target_file2, &format!("{link}/file2"), false);
    mklink(&target_dir1, &format!("{link}/dir1"), true);
    mklink(&target_dir2, &format!("{link}/dir2"), true);

    let fs = fs();

    let entries = fs.get_accessible_entries(&tspath::normalize_path(&link));

    assert_eq!(entries.directories, vec!["dir1", "dir2"]);
    assert_eq!(entries.files, vec!["file1", "file2"]);
    let symlinks = entries.symlinks.expect("expected Symlinks to be set for directory with symlinks");
    assert_eq!(symlinks.len(), 4);
    for name in ["file1", "file2", "dir1", "dir2"] {
        assert!(symlinks.contains(name), "expected {name:?} to be in Symlinks");
    }

    // Non-symlink directory should have empty Symlinks.
    let entries = fs.get_accessible_entries(&tspath::normalize_path(&target));
    assert_eq!(entries.directories, vec!["dir1", "dir2"]);
    assert_eq!(entries.files, vec!["file1", "file2"]);
    let symlinks = entries.symlinks.expect("expected Symlinks to be non-nil for directory without symlinks");
    assert_eq!(symlinks.len(), 0);
}
