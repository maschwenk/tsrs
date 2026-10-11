use tsrs_core::tspath;

use super::os_test::{mklink, temp_dir};
use super::*;

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

#[cfg(not(windows))]
#[test]
fn test_walk_symlinks_callbacks_preserve_physical_parent() {
    let mut probed = Vec::new();
    let actual = walk_symlinks_with(
        "/repo/link/../file.ts",
        |path| {
            probed.push(path.to_string());
            Ok(match path {
                "/repo/link" => FileMode::Symlink,
                "/repo/real/file.ts" => FileMode::None,
                "/repo" | "/repo/real" | "/repo/real/sub" => FileMode::Dir,
                _ => return Err(io::Error::from(io::ErrorKind::NotFound)),
            })
        },
        |path| {
            assert_eq!(path, "/repo/link");
            Ok("real/sub".to_string())
        },
    )
    .unwrap();
    assert_eq!(actual, "/repo/real/file.ts");
    assert_eq!(probed, ["/repo", "/repo/link", "/repo/real", "/repo/real/sub", "/repo/real/file.ts"]);
}

#[cfg(not(windows))]
#[test]
fn test_walk_symlinks_callbacks_keep_link_limit_and_errors() {
    let mut metadata_calls = 0;
    let mut link_calls = 0;
    let err = walk_symlinks_with(
        "/loop",
        |_| {
            metadata_calls += 1;
            Ok(FileMode::Symlink)
        },
        |_| {
            link_calls += 1;
            Ok("loop".to_string())
        },
    )
    .unwrap_err();
    assert_eq!(err.to_string(), "EvalSymlinks: too many links");
    assert_eq!(metadata_calls, 256);
    assert_eq!(link_calls, 255);

    for path in ["/file/child", "/file/"] {
        let err = walk_symlinks_with(path, |_| Ok(FileMode::None), |_| panic!("not a symlink")).unwrap_err();
        assert_eq!(err.raw_os_error(), Some(20));
    }
    let err = walk_symlinks_with("/missing", |_| Err(io::Error::from(io::ErrorKind::NotFound)), |_| panic!("missing entry")).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
}

#[cfg(unix)]
#[test]
fn test_realpath_file_links_and_broken_links() {
    let tmp = temp_dir();
    let target = format!("{tmp}/file.ts");
    fs::write(&target, "").unwrap();
    let link = format!("{tmp}/link.ts");
    mklink("file.ts", &link, false);
    assert_eq!(fs().realpath(&link), fs().realpath(&target));

    let missing_link = format!("{tmp}/missing.ts");
    mklink("does-not-exist.ts", &missing_link, false);
    assert_eq!(fs().realpath(&missing_link), missing_link);

    let loop_link = format!("{tmp}/loop.ts");
    mklink("loop.ts", &loop_link, false);
    assert_eq!(fs().realpath(&loop_link), loop_link);
    fs::remove_dir_all(tmp).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn test_linux_realpath_long_path_and_procfs_fallback() {
    let tmp = temp_dir();
    let mut directory = std::path::PathBuf::from(&tmp);
    for i in 0..6 {
        directory.push(format!("{i}{}", "a".repeat(49)));
    }
    fs::create_dir_all(&directory).unwrap();
    let target = directory.join("file.ts");
    fs::write(&target, "").unwrap();
    let link = format!("{tmp}/link.ts");
    mklink(target.to_str().unwrap(), &link, false);
    let expected = fs::canonicalize(&target).unwrap().to_string_lossy().into_owned();
    assert!(expected.len() > 256);

    // Exercise the no-procfs branch without requiring a privileged chroot.
    for has_proc in [false, true] {
        if has_proc && fs::metadata("/proc/self/fd/").is_err() {
            continue;
        }
        assert_eq!(linux_realpath(&link, has_proc).unwrap(), expected);
        assert_eq!(linux_realpath(target.to_str().unwrap(), has_proc).unwrap(), expected);
        let missing = format!("{tmp}/missing.ts");
        assert_eq!(linux_realpath(&missing, has_proc).unwrap_err().kind(), io::ErrorKind::NotFound);
    }
    fs::remove_dir_all(tmp).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn test_linux_realpath_retries_interrupted_syscalls() {
    let mut calls = 0;
    let result = ignoring_eintr(|| {
        calls += 1;
        if calls < 3 { Err(rustix::io::Errno::INTR) } else { Ok(42) }
    });
    assert_eq!(result.unwrap(), 42);
    assert_eq!(calls, 3);
    let err = ignoring_eintr::<()>(|| Err(rustix::io::Errno::NOENT)).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
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
