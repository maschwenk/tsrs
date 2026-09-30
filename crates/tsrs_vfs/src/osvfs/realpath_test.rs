use tsrs_core::tspath;

use super::os_test::{mklink, temp_dir};
use super::*;
use crate::FS;

fn setup_symlinks() -> (String, String) {
    let tmp = temp_dir();

    let target = format!("{}/target", tmp);
    let target_file = format!("{}/file", target);

    let link = format!("{}/link", tmp);
    let link_file = format!("{}/file", link);

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
    std::fs::create_dir_all(format!("{}/real/sub", tmp)).unwrap();
    std::fs::write(format!("{}/real/sub/f.ts", tmp), "").unwrap();
    mklink("real/sub", &format!("{}/rel", tmp), true);

    let fs = fs();
    let real = fs.realpath(&format!("{}/real/sub/f.ts", tmp));
    assert_eq!(fs.realpath(&format!("{}/rel/f.ts", tmp)), real);
    // ".." after a symlink is physical (EvalSymlinks semantics): rel/.. is real/, so this does not exist.
    let dotdot = format!("{}/rel/../real/sub/f.ts", tmp);
    assert_eq!(fs.realpath(&dotdot), dotdot);
    assert_eq!(fs.realpath(&format!("{}/rel/../sub/f.ts", tmp)), real);

    // A path that does not exist is returned unchanged.
    let missing = format!("{}/does/not/exist.ts", tmp);
    assert_eq!(fs.realpath(&missing), missing);
}

#[test]
fn test_get_accessible_entries() {
    let tmp = temp_dir();
    let target = format!("{}/target", tmp);
    let link = format!("{}/link", tmp);

    std::fs::create_dir_all(&target).unwrap();
    std::fs::create_dir_all(&link).unwrap();

    let target_file1 = format!("{}/file1", target);
    let target_file2 = format!("{}/file2", target);

    std::fs::write(&target_file1, "hello").unwrap();
    std::fs::write(&target_file2, "world").unwrap();

    let target_dir1 = format!("{}/dir1", target);
    let target_dir2 = format!("{}/dir2", target);

    std::fs::create_dir_all(&target_dir1).unwrap();
    std::fs::create_dir_all(&target_dir2).unwrap();

    mklink(&target_file1, &format!("{}/file1", link), false);
    mklink(&target_file2, &format!("{}/file2", link), false);
    mklink(&target_dir1, &format!("{}/dir1", link), true);
    mklink(&target_dir2, &format!("{}/dir2", link), true);

    let fs = fs();

    let entries = fs.get_accessible_entries(&tspath::normalize_path(&link));

    assert_eq!(entries.directories, vec!["dir1", "dir2"]);
    assert_eq!(entries.files, vec!["file1", "file2"]);
    let symlinks = entries.symlinks.expect("expected Symlinks to be set for directory with symlinks");
    assert_eq!(symlinks.len(), 4);
    for name in ["file1", "file2", "dir1", "dir2"] {
        assert!(symlinks.contains(name), "expected {:?} to be in Symlinks", name);
    }

    // Non-symlink directory should have empty Symlinks.
    let entries = fs.get_accessible_entries(&tspath::normalize_path(&target));
    assert_eq!(entries.directories, vec!["dir1", "dir2"]);
    assert_eq!(entries.files, vec!["file1", "file2"]);
    let symlinks = entries.symlinks.expect("expected Symlinks to be non-nil for directory without symlinks");
    assert_eq!(symlinks.len(), 0);
}
