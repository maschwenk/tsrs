use std::panic::{AssertUnwindSafe, catch_unwind};

use tsrs_vfs::{osvfs, vfstest};

use super::*;

#[test]
fn replay_survives_removal_of_files_and_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_str().unwrap();
    let file = format!("{root}/file.ts");
    let missing = format!("{root}/missing");
    std::fs::write(&file, "export const value = 1;").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("file.ts", dir.path().join("link.ts")).unwrap();

    let recording = RecordingFs::new(Arc::new(osvfs::fs()));
    let content = recording.read_file(&file);
    let metadata = recording.stat(&file);
    let entries = recording.get_accessible_entries(root);
    assert!(recording.file_exists(&file));
    assert!(recording.directory_exists(root));
    assert!(!recording.directory_exists(&file));
    assert!(!recording.file_exists(&missing));
    assert!(!recording.directory_exists(&missing));
    assert_eq!(recording.read_file(&missing), None);
    assert_eq!(recording.stat(&missing), None);
    let missing_entries = recording.get_accessible_entries(&missing);
    let missing_realpath = recording.realpath(&missing);
    let realpath = recording.realpath(&file);
    let case_sensitive = recording.use_case_sensitive_file_names();
    #[cfg(unix)]
    let link = {
        let link = format!("{root}/link.ts");
        assert_eq!(recording.read_file(&link), content);
        assert_eq!(recording.realpath(&link), realpath);
        assert!(entries.symlinks.as_ref().unwrap().contains("link.ts"));
        link
    };
    let snapshot = recording.freeze();
    let root = root.to_string();
    dir.close().unwrap();

    assert_eq!(snapshot.read_file(&file), content);
    assert_eq!(snapshot.stat(&file), metadata);
    assert_eq!(snapshot.get_accessible_entries(&root), entries);
    assert!(snapshot.file_exists(&file));
    assert!(snapshot.directory_exists(&root));
    assert!(!snapshot.directory_exists(&file));
    assert!(!snapshot.file_exists(&missing));
    assert!(!snapshot.directory_exists(&missing));
    assert_eq!(snapshot.read_file(&missing), None);
    assert_eq!(snapshot.stat(&missing), None);
    assert_eq!(snapshot.get_accessible_entries(&missing), missing_entries);
    assert_eq!(snapshot.realpath(&missing), missing_realpath);
    assert_eq!(snapshot.realpath(&file), realpath);
    assert_eq!(snapshot.use_case_sensitive_file_names(), case_sensitive);
    #[cfg(unix)]
    {
        assert_eq!(snapshot.read_file(&link), content);
        assert_eq!(snapshot.realpath(&link), realpath);
    }
}

#[test]
fn replay_keeps_case_sensitivity_and_negative_results() {
    for case_sensitive in [false, true] {
        let recording = RecordingFs::new(Arc::new(vfstest::from_map(
            [("/A.ts", "const x = 1;")],
            case_sensitive,
        )));
        let lower = recording.read_file("/a.ts");
        let exists = recording.file_exists("/a.ts");
        let snapshot = recording.freeze();
        assert_eq!(snapshot.use_case_sensitive_file_names(), case_sensitive);
        assert_eq!(snapshot.read_file("/a.ts"), lower);
        assert_eq!(snapshot.file_exists("/a.ts"), exists);
        assert_eq!(exists, !case_sensitive);
    }
}

#[test]
fn unknown_requests_are_not_treated_as_missing_paths() {
    let snapshot = SnapshotFs::default();
    let requests: &[fn(&SnapshotFs)] = &[
        |fs| {
            fs.read_file("/unrecorded");
        },
        |fs| {
            fs.file_exists("/unrecorded");
        },
        |fs| {
            fs.directory_exists("/unrecorded");
        },
        |fs| {
            fs.get_accessible_entries("/unrecorded");
        },
        |fs| {
            fs.stat("/unrecorded");
        },
        |fs| {
            fs.realpath("/unrecorded");
        },
    ];
    for request in requests {
        assert!(catch_unwind(|| request(&snapshot)).is_err());
    }
}

#[test]
fn capture_and_replay_reject_mutations() {
    let recording = RecordingFs::new(Arc::new(vfstest::from_map([("/a.ts", "")], true)));
    let snapshot = recording.freeze();
    for fs in [&recording as &dyn FS, &snapshot as &dyn FS] {
        assert!(catch_unwind(AssertUnwindSafe(|| fs.write_file("/a.ts", "changed"))).is_err());
        assert!(catch_unwind(AssertUnwindSafe(|| fs.append_file("/a.ts", "changed"))).is_err());
        assert!(catch_unwind(AssertUnwindSafe(|| fs.remove("/a.ts"))).is_err());
        assert!(
            catch_unwind(AssertUnwindSafe(|| fs.chtimes(
                "/a.ts",
                SystemTime::UNIX_EPOCH,
                SystemTime::UNIX_EPOCH
            )))
            .is_err()
        );
    }
}
