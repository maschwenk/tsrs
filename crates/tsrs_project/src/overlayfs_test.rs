use std::sync::Arc;

use rustc_hash::FxHashMap;
use tsrs_core::tspath::Path;
use tsrs_core::ScriptKind;
use tsrs_lsproto as lsproto;
use tsrs_vfs::{vfstest, FS};

use super::*;
use crate::filechange::{FileChange, FileChangeKind};
use crate::snapshotfs::FileHandleSource;

// Helper to create test overlayFS
fn create_overlay_fs() -> overlayFS {
    let test_fs = vfstest::from_map(
        [("/test1.ts", "// existing content"), ("/test2.ts", "// existing content"), ("/script", "// extensionless content")],
        false, /* useCaseSensitiveFileNames */
    );
    new_overlay_fs(FsRef::Host(Arc::new(test_fs)), Arc::default(), lsproto::PositionEncodingKind::UTF16, Arc::new(|file_name: &str| Path(file_name.to_string())))
}

fn test_uri1() -> lsproto::DocumentUri {
    lsproto::DocumentUri("file:///test1.ts".to_string())
}

fn test_uri2() -> lsproto::DocumentUri {
    lsproto::DocumentUri("file:///test2.ts".to_string())
}

fn open(uri: lsproto::DocumentUri, version: i32, content: &str, language_kind: lsproto::LanguageKind) -> FileChange {
    FileChange { kind: FileChangeKind::Open, uri, version, content: content.to_string(), language_kind, changes: Vec::new() }
}

fn change(kind: FileChangeKind, uri: lsproto::DocumentUri) -> FileChange {
    FileChange { kind, uri, ..Default::default() }
}

// overlayfs_test.go:39 TestProcessChanges/multiple opens should panic
#[test]
fn process_changes_multiple_opens_should_panic() {
    let fs = create_overlay_fs();

    let changes = vec![
        open(test_uri1(), 1, "const x = 1;", lsproto::LanguageKind::TypeScript),
        open(test_uri2(), 1, "const y = 2;", lsproto::LanguageKind::TypeScript),
    ];

    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        fs.process_changes(&changes);
    }))
    .is_err();
    assert!(panicked);
}

// overlayfs_test.go:70 TestProcessChanges/watch create then delete becomes nothing
#[test]
fn process_changes_watch_create_then_delete_becomes_nothing() {
    let fs = create_overlay_fs();
    let (result, _) = fs.process_changes(&[change(FileChangeKind::WatchCreate, test_uri1()), change(FileChangeKind::WatchDelete, test_uri1())]);
    assert!(result.is_empty());
}

// overlayfs_test.go:89 TestProcessChanges/watch delete then create becomes change
#[test]
fn process_changes_watch_delete_then_create_becomes_change() {
    let fs = create_overlay_fs();
    let (result, _) = fs.process_changes(&[change(FileChangeKind::WatchDelete, test_uri1()), change(FileChangeKind::WatchCreate, test_uri1())]);

    assert_eq!(result.created.len(), 0);
    assert_eq!(result.deleted.len(), 0);
    assert!(result.changed.has(&test_uri1()));
}

// overlayfs_test.go:112 TestProcessChanges/multiple watch changes deduplicated
#[test]
fn process_changes_multiple_watch_changes_deduplicated() {
    let fs = create_overlay_fs();
    let (result, _) = fs.process_changes(&[
        change(FileChangeKind::WatchChange, test_uri1()),
        change(FileChangeKind::WatchChange, test_uri1()),
        change(FileChangeKind::WatchChange, test_uri1()),
    ]);

    assert!(result.changed.has(&test_uri1()));
    assert_eq!(result.changed.len(), 1);
}

// overlayfs_test.go:137 TestProcessChanges/save marks overlay as matching disk
#[test]
fn process_changes_save_marks_overlay_as_matching_disk() {
    let fs = create_overlay_fs();

    // First create an overlay
    fs.process_changes(&[open(test_uri1(), 1, "const x = 1;", lsproto::LanguageKind::TypeScript)]);
    // Then save
    let (result, _) = fs.process_changes(&[change(FileChangeKind::Save, test_uri1())]);
    // We don't observe saves for snapshot changes,
    // so they're not included in the summary
    assert!(result.is_empty());

    // Check that the overlay is marked as matching disk text
    let fh = fs.get_file(&test_uri1().file_name()).unwrap();
    assert!(fh.matches_disk_text());
}

// overlayfs_test.go:166 TestProcessChanges/open falls back to file extension for unknown language kind
#[test]
fn process_changes_open_falls_back_to_file_extension_for_unknown_language_kind() {
    let fs = create_overlay_fs();
    let uri = lsproto::DocumentUri("file:///test1.mts".to_string());

    fs.process_changes(&[open(uri.clone(), 1, "export const x = 1;", lsproto::LanguageKind("mts"))]);

    let fh = fs.get_file(&uri.file_name()).unwrap();
    assert_eq!(fh.kind(), ScriptKind::TS);
}

// overlayfs_test.go:186 TestProcessChanges/open extensionless file preserves unknown script kind
#[test]
fn process_changes_open_extensionless_file_preserves_unknown_script_kind() {
    let fs = create_overlay_fs();
    let uri = lsproto::DocumentUri("file:///script".to_string());

    fs.process_changes(&[open(uri.clone(), 1, "const x = 1;", lsproto::LanguageKind("plaintext"))]);

    let fh = fs.get_file(&uri.file_name()).unwrap();
    assert_eq!(fh.kind(), ScriptKind::Unknown);
}

// overlayfs_test.go:206 TestProcessChanges/extensionless disk file preserves unknown script kind
#[test]
fn process_changes_extensionless_disk_file_preserves_unknown_script_kind() {
    let fs = create_overlay_fs();

    let fh = fs.get_file("/script").unwrap();
    assert_eq!(fh.kind(), ScriptKind::Unknown);
}

// overlayfs_test.go:216 TestProcessChanges/watch change on overlay marks as not matching disk
#[test]
fn process_changes_watch_change_on_overlay_marks_as_not_matching_disk() {
    let fs = create_overlay_fs();

    // First create an overlay
    fs.process_changes(&[open(test_uri1(), 1, "const x = 1;", lsproto::LanguageKind::TypeScript)]);
    assert!(!fs.get_file(&test_uri1().file_name()).unwrap().matches_disk_text());

    // Then save
    fs.process_changes(&[change(FileChangeKind::Save, test_uri1())]);
    assert!(fs.get_file(&test_uri1().file_name()).unwrap().matches_disk_text());

    // Now process a watch change
    fs.process_changes(&[change(FileChangeKind::WatchChange, test_uri1())]);
    assert!(!fs.get_file(&test_uri1().file_name()).unwrap().matches_disk_text());
}

// overlayfs_test.go:252 TestProcessChanges/save without overlay should not panic
#[test]
fn process_changes_save_without_overlay_should_not_panic() {
    let fs = create_overlay_fs();

    // Save a file that was never opened (no overlay exists).
    // This can happen when an editor sends didSave for a file
    // that is not managed by the LSP server (e.g., package.json).
    let (result, _) = fs.process_changes(&[change(FileChangeKind::Save, test_uri1())]);
    // Should be treated as a disk change
    assert!(result.changed.has(&test_uri1()));
}

// overlayfs_test.go:269 TestProcessChanges/close and change without overlay should not panic
#[test]
fn process_changes_close_and_change_without_overlay_should_not_panic() {
    let fs = create_overlay_fs();

    fs.process_changes(&[open(test_uri1(), 1, "const x = 1;", lsproto::LanguageKind::TypeScript)]);
    fs.process_changes(&[change(FileChangeKind::Close, test_uri1())]);

    let (result, _) = fs.process_changes(&[change(FileChangeKind::Close, test_uri1())]);

    assert!(result.is_empty());

    let (result, _) = fs.process_changes(&[FileChange {
        kind: FileChangeKind::Change,
        uri: test_uri1(),
        version: 2,
        changes: vec![lsproto::TextDocumentContentChangePartialOrWholeDocument {
            whole_document: Some(lsproto::TextDocumentContentChangeWholeDocument { text: "const x = 1;".to_string() }),
            ..Default::default()
        }],
        ..Default::default()
    }]);

    assert!(result.is_empty());
}

// overlayfs_test.go:314 TestProcessChanges/close then open in same batch marks as changed
#[test]
fn process_changes_close_then_open_in_same_batch_marks_as_changed() {
    let fs = create_overlay_fs();

    // First create an overlay
    fs.process_changes(&[open(test_uri1(), 1, "const x = 1;", lsproto::LanguageKind::TypeScript)]);

    // Now close and reopen in the same batch (like Neovim does for file reload)
    let (result, _) = fs.process_changes(&[change(FileChangeKind::Close, test_uri1()), open(test_uri1(), 0, "const x = 2;", lsproto::LanguageKind::TypeScript)]);

    // Should not be marked as opened since it was already open
    assert!(result.opened.0.is_empty(), "close then open should not mark as opened");
    // Should also be marked as changed since it was closed and reopened
    assert!(result.changed.has(&test_uri1()), "close then open should mark as changed");
    // Should have the new content
    let fh = fs.get_file(&test_uri1().file_name()).unwrap();
    assert_eq!(fh.content(), "const x = 2;");
}

// overlayfs_test.go:357 TestOverlayFSFileSystem
#[test]
fn overlay_fs_file_system() {
    let host = vfstest::from_map([("/virtual", "host file")], false /* useCaseSensitiveFileNames */);
    let to_path: ToPath = Arc::new(|file_name: &str| Path(file_name.to_string()));
    let mut overlays: FxHashMap<Path, Arc<Overlay>> = FxHashMap::default();
    overlays.insert(
        Path("/virtual/nested/file.ts".to_string()),
        Arc::new(new_overlay("/virtual/nested/file.ts".to_string(), "overlay".to_string(), 1, ScriptKind::TS)),
    );
    let file_system = new_overlay_fs(FsRef::Host(Arc::new(host)), Arc::new(overlays), lsproto::PositionEncodingKind::UTF16, to_path);

    assert!(file_system.directory_exists("/virtual"));
    assert!(!file_system.file_exists("/virtual"));
    assert!(file_system.stat("/virtual").unwrap().is_dir());
    let content = file_system.read_file("/virtual/nested/file.ts").unwrap();
    assert_eq!(content, "overlay");

    let root_entries = file_system.get_accessible_entries("/");
    assert!(root_entries.directories.contains(&"virtual".to_string()));
    assert!(!root_entries.files.contains(&"virtual".to_string()));
}
