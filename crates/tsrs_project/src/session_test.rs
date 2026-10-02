use std::sync::Arc;

use tsrs_core::context::Context;
use tsrs_core::tspath::Path;
use tsrs_core::ScriptKind;
use tsrs_lsproto as lsproto;

use crate::projecttestutil::{self, setup};

fn uri(s: &str) -> lsproto::DocumentUri {
    lsproto::DocumentUri(s.to_string())
}

fn ctx() -> Context {
    Context::background()
}

const default_files: &[(&str, &str)] = &[
    (
        "/home/projects/TS/p1/tsconfig.json",
        r#"{
			"compilerOptions": {
				"noLib": true,
				"module": "nodenext",
				"strict": true
			},
			"include": ["src"]
		}"#,
    ),
    ("/home/projects/TS/p1/src/index.ts", r#"import { x } from "./x";"#),
    ("/home/projects/TS/p1/src/x.ts", "export const x = 1;"),
    ("/home/projects/TS/p1/config.ts", "let x = 1, y = 2;"),
];

fn file(files: &[(&str, &str)], name: &str) -> String {
    files.iter().find(|(k, _)| *k == name).unwrap().1.to_string()
}

fn partial(line: u32, start: u32, end: u32, text: &str) -> lsproto::TextDocumentContentChangePartialOrWholeDocument {
    lsproto::TextDocumentContentChangePartialOrWholeDocument {
        partial: Some(lsproto::TextDocumentContentChangePartial {
            range: lsproto::Range { start: lsproto::Position { line, character: start }, end: lsproto::Position { line, character: end } },
            text: text.to_string(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

// session_test.go:45 TestSession/DidOpenFile/create configured project
#[test]
fn did_open_file_create_configured_project() {
    let (session, _) = setup(default_files);
    let snapshot = session.snapshot();
    assert_eq!(snapshot.project_collection.projects().len(), 0);

    session.did_open_file(
        &ctx(),
        uri("file:///home/projects/TS/p1/src/index.ts"),
        1,
        file(default_files, "/home/projects/TS/p1/src/index.ts"),
        lsproto::LanguageKind::TypeScript,
    );

    let snapshot = session.snapshot();
    assert_eq!(snapshot.project_collection.projects().len(), 1);

    let configured_project = snapshot.project_collection.configured_project(&Path("/home/projects/ts/p1/tsconfig.json".to_string()));
    assert!(configured_project.is_some());

    // Get language service to access the program
    let ls = session.get_language_service(&ctx(), &uri("file:///home/projects/TS/p1/src/index.ts")).unwrap();
    let program = ls.get_program();
    assert!(program.get_source_file("/home/projects/TS/p1/src/x.ts").is_some());
    assert_eq!(program.get_source_file("/home/projects/TS/p1/src/x.ts").unwrap().text(), "export const x = 1;");
}

// session_test.go:67 TestSession/DidOpenFile/create inferred project
#[test]
fn did_open_file_create_inferred_project() {
    let (session, _) = setup(default_files);

    session.did_open_file(&ctx(), uri("file:///home/projects/TS/p1/config.ts"), 1, file(default_files, "/home/projects/TS/p1/config.ts"), lsproto::LanguageKind::TypeScript);

    // Find tsconfig, load, notice config.ts is not included, create inferred project
    let snapshot = session.snapshot();
    assert_eq!(snapshot.project_collection.projects().len(), 2);

    // Should have both configured project (for tsconfig.json) and inferred project
    let configured_project = snapshot.project_collection.configured_project(&Path("/home/projects/ts/p1/tsconfig.json".to_string()));
    let inferred_project = snapshot.project_collection.inferred_project();
    assert!(configured_project.is_some());
    assert!(inferred_project.is_some());
}

// session_test.go:84 TestSession/DidOpenFile/inferred project for in-memory files
#[test]
fn did_open_file_inferred_project_for_in_memory_files() {
    let (session, _) = setup(default_files);

    session.did_open_file(&ctx(), uri("file:///home/projects/TS/p1/config.ts"), 1, file(default_files, "/home/projects/TS/p1/config.ts"), lsproto::LanguageKind::TypeScript);
    session.did_open_file(&ctx(), uri("untitled:Untitled-1"), 1, "x".to_string(), lsproto::LanguageKind::TypeScript);
    session.did_open_file(&ctx(), uri("untitled:Untitled-2"), 1, "y".to_string(), lsproto::LanguageKind::TypeScript);

    let snapshot = session.snapshot();

    assert_eq!(snapshot.project_collection.projects().len(), 1);
    assert!(snapshot.project_collection.inferred_project().is_some());
}

// session_test.go:98 TestSession/DidOpenFile/inferred project JS file
#[test]
fn did_open_file_inferred_project_js_file() {
    let js_files: &[(&str, &str)] = &[("/home/projects/TS/p1/index.js", r#"import { x } from "./x";"#)];
    let (session, _) = setup(js_files);

    session.did_open_file(&ctx(), uri("file:///home/projects/TS/p1/index.js"), 1, file(js_files, "/home/projects/TS/p1/index.js"), lsproto::LanguageKind::JavaScript);

    let snapshot = session.snapshot();
    assert_eq!(snapshot.project_collection.projects().len(), 1);

    let ls = session.get_language_service(&ctx(), &uri("file:///home/projects/TS/p1/index.js")).unwrap();
    let program = ls.get_program();
    assert!(program.get_source_file("/home/projects/TS/p1/index.js").is_some());
}

// session_test.go:116 TestSession/DidOpenFile/inferred project extensionless file
#[test]
fn did_open_file_inferred_project_extensionless_file() {
    let files: &[(&str, &str)] = &[("/home/projects/TS/p1/script", "const x = 1;")];
    let (session, _) = setup(files);

    session.did_open_file(&ctx(), uri("file:///home/projects/TS/p1/script"), 1, file(files, "/home/projects/TS/p1/script"), lsproto::LanguageKind("plaintext"));

    let snapshot = session.snapshot();
    assert_eq!(snapshot.project_collection.projects().len(), 1);
    assert!(snapshot.project_collection.inferred_project().is_some());

    let ls = session.get_language_service(&ctx(), &uri("file:///home/projects/TS/p1/script")).unwrap();
    let program = ls.get_program();
    let f = program.get_source_file("/home/projects/TS/p1/script").unwrap();
    assert_eq!(f.script_kind(), ScriptKind::TS);
}

// session_test.go:138 TestSession/watchChange and didOpen in same batch rebuilds program
#[test]
fn watch_change_and_did_open_in_same_batch_rebuilds_program() {
    let files: &[(&str, &str)] = &[
        (
            "/home/projects/TS/p1/tsconfig.json",
            r#"{
				"compilerOptions": {
					"noLib": true,
					"strict": true
				}
			}"#,
        ),
        ("/home/projects/TS/p1/src/a.ts", "export const a = 1;\n"),
        ("/home/projects/TS/p1/src/b.ts", "export const b = 1;\n"),
    ];
    let (session, utils) = setup(files);
    let old_content = file(files, "/home/projects/TS/p1/src/a.ts");

    // Open b.ts to create the project; a.ts is included via tsconfig.
    session.did_open_file(&ctx(), uri("file:///home/projects/TS/p1/src/b.ts"), 1, file(files, "/home/projects/TS/p1/src/b.ts"), lsproto::LanguageKind::TypeScript);

    // Verify a.ts is in the program with the original content.
    let ls = session.get_language_service(&ctx(), &uri("file:///home/projects/TS/p1/src/b.ts")).unwrap();
    assert_eq!(ls.get_program().get_source_file("/home/projects/TS/p1/src/a.ts").unwrap().text(), old_content);

    // Modify a.ts on disk (simulate a build tool or git checkout).
    let new_content = "export const a = 2;\nexport const extra = true;\n";
    utils.fs().write_file("/home/projects/TS/p1/src/a.ts", new_content).unwrap();

    // Queue a watch event for the disk change (not flushed yet).
    session.did_change_watched_files(&ctx(), &[lsproto::FileEvent { type_: lsproto::FileChangeType::Changed, uri: uri("file:///home/projects/TS/p1/src/a.ts") }]);

    // Open a.ts in the editor—flushes both watch event and didOpen together.
    session.did_open_file(&ctx(), uri("file:///home/projects/TS/p1/src/a.ts"), 1, new_content.to_string(), lsproto::LanguageKind::TypeScript);

    // The program's SourceFile must reflect the overlay (new) content.
    let ls = session.get_language_service(&ctx(), &uri("file:///home/projects/TS/p1/src/a.ts")).unwrap();
    assert_eq!(ls.get_program().get_source_file("/home/projects/TS/p1/src/a.ts").unwrap().text(), new_content);
}

// session_test.go:184 TestSession/DidChangeFile/update file and program
#[test]
fn did_change_file_update_file_and_program() {
    let (session, _) = setup(default_files);

    session.did_open_file(&ctx(), uri("file:///home/projects/TS/p1/src/x.ts"), 1, file(default_files, "/home/projects/TS/p1/src/x.ts"), lsproto::LanguageKind::TypeScript);

    let ls_before = session.get_language_service(&ctx(), &uri("file:///home/projects/TS/p1/src/x.ts")).unwrap();
    let program_before = ls_before.get_program();

    session.did_change_file(&ctx(), uri("file:///home/projects/TS/p1/src/x.ts"), 2, vec![partial(0, 17, 18, "2")]);

    let ls_after = session.get_language_service(&ctx(), &uri("file:///home/projects/TS/p1/src/x.ts")).unwrap();
    let program_after = ls_after.get_program();

    // Program should change due to the file content change
    assert!(!std::ptr::eq(program_after, program_before));
    assert_eq!(program_after.get_source_file("/home/projects/TS/p1/src/x.ts").unwrap().text(), "export const x = 2;");
}

// session_test.go:221 TestSession/DidChangeFile/update untitled file
#[test]
fn did_change_file_update_untitled_file() {
    let (session, _) = setup(default_files);

    session.did_open_file(&ctx(), uri("untitled:Untitled-1"), 1, "let x = 1;".to_string(), lsproto::LanguageKind::TypeScript);

    let ls_before = session.get_language_service(&ctx(), &uri("untitled:Untitled-1")).unwrap();
    let program_before = ls_before.get_program();
    let untitled_file_name = uri("untitled:Untitled-1").file_name();
    assert_eq!(program_before.get_source_file(&untitled_file_name).unwrap().text(), "let x = 1;");

    session.did_change_file(&ctx(), uri("untitled:Untitled-1"), 2, vec![partial(0, 8, 9, "2")]);

    let ls_after = session.get_language_service(&ctx(), &uri("untitled:Untitled-1")).unwrap();
    let program_after = ls_after.get_program();

    assert!(!std::ptr::eq(program_after, program_before));
    assert_eq!(program_after.get_source_file(&untitled_file_name).unwrap().text(), "let x = 2;");
}

// session_test.go:259 TestSession/DidChangeFile/unchanged source files are reused
#[test]
fn did_change_file_unchanged_source_files_are_reused() {
    let (session, _) = setup(default_files);

    session.did_open_file(&ctx(), uri("file:///home/projects/TS/p1/src/x.ts"), 1, file(default_files, "/home/projects/TS/p1/src/x.ts"), lsproto::LanguageKind::TypeScript);

    let ls_before = session.get_language_service(&ctx(), &uri("file:///home/projects/TS/p1/src/x.ts")).unwrap();
    let program_before = ls_before.get_program();
    let index_file_before = program_before.get_source_file("/home/projects/TS/p1/src/index.ts");

    session.did_change_file(&ctx(), uri("file:///home/projects/TS/p1/src/x.ts"), 2, vec![partial(0, 0, 0, ";")]);

    let ls_after = session.get_language_service(&ctx(), &uri("file:///home/projects/TS/p1/src/x.ts")).unwrap();
    let program_after = ls_after.get_program();

    // Unchanged file should be reused
    assert_eq!(program_after.get_source_file("/home/projects/TS/p1/src/index.ts"), index_file_before);
}

// session_test.go:296 TestSession/DidChangeFile/change can pull in new files
#[test]
fn did_change_file_change_can_pull_in_new_files() {
    let mut files: Vec<(&str, &str)> = default_files.to_vec();
    files.push(("/home/projects/TS/p1/y.ts", "export const y = 2;"));
    let (session, _) = setup(&files);

    session.did_open_file(&ctx(), uri("file:///home/projects/TS/p1/src/index.ts"), 1, file(&files, "/home/projects/TS/p1/src/index.ts"), lsproto::LanguageKind::TypeScript);

    // Verify y.ts is not initially in the program
    let ls_before = session.get_language_service(&ctx(), &uri("file:///home/projects/TS/p1/src/index.ts")).unwrap();
    assert!(ls_before.get_program().get_source_file("/home/projects/TS/p1/y.ts").is_none());

    session.did_change_file(&ctx(), uri("file:///home/projects/TS/p1/src/index.ts"), 2, vec![partial(0, 0, 0, "import { y } from \"../y\";\n")]);

    let ls_after = session.get_language_service(&ctx(), &uri("file:///home/projects/TS/p1/src/index.ts")).unwrap();
    // y.ts should now be included in the program
    assert!(ls_after.get_program().get_source_file("/home/projects/TS/p1/y.ts").is_some());
}

// session_test.go:336 TestSession/DidChangeFile/single-file change followed by config change reloads program
#[test]
fn did_change_file_single_file_change_followed_by_config_change_reloads_program() {
    let mut files: Vec<(&str, &str)> = default_files.to_vec();
    files[0].1 = r#"{
				"compilerOptions": {
					"noLib": true,
					"module": "nodenext",
					"strict": true
				},
				"include": ["src/index.ts"]
			}"#;
    let (session, utils) = setup(&files);

    session.did_open_file(&ctx(), uri("file:///home/projects/TS/p1/src/index.ts"), 1, file(&files, "/home/projects/TS/p1/src/index.ts"), lsproto::LanguageKind::TypeScript);

    let ls_before = session.get_language_service(&ctx(), &uri("file:///home/projects/TS/p1/src/index.ts")).unwrap();
    assert_eq!(ls_before.get_program().get_source_files().len(), 2);

    session.did_change_file(&ctx(), uri("file:///home/projects/TS/p1/src/index.ts"), 2, vec![partial(0, 0, 0, "\n")]);

    utils
        .fs()
        .write_file(
            "/home/projects/TS/p1/tsconfig.json",
            r#"{
				"compilerOptions": {
					"noLib": true,
					"module": "nodenext",
					"strict": true
				},
				"include": ["./**/*"]
			}"#,
        )
        .unwrap();

    session.did_change_watched_files(&ctx(), &[lsproto::FileEvent { type_: lsproto::FileChangeType::Changed, uri: uri("file:///home/projects/TS/p1/tsconfig.json") }]);

    let ls_after = session.get_language_service(&ctx(), &uri("file:///home/projects/TS/p1/src/index.ts")).unwrap();
    assert_eq!(ls_after.get_program().get_source_files().len(), 3);
}

// tsrs-only: snapshots, programs and parse cache entries are released when the session moves on.
#[test]
fn snapshot_refs_release_parse_cache() {
    let (session, _) = setup(default_files);
    session.did_open_file(&ctx(), uri("file:///home/projects/TS/p1/src/x.ts"), 1, file(default_files, "/home/projects/TS/p1/src/x.ts"), lsproto::LanguageKind::TypeScript);
    let _ = session.get_language_service(&ctx(), &uri("file:///home/projects/TS/p1/src/x.ts")).unwrap();
    session.wait_for_background_tasks();
    let entries = session.snapshot_host().parse_cache.len();
    assert!(entries >= 2, "{entries}");
    for v in 2..6 {
        session.did_change_file(&ctx(), uri("file:///home/projects/TS/p1/src/x.ts"), v, vec![partial(0, 0, 0, ";")]);
        let _ = session.get_language_service(&ctx(), &uri("file:///home/projects/TS/p1/src/x.ts")).unwrap();
    }
    session.wait_for_background_tasks();
    // Only the current versions of the files stay cached.
    assert_eq!(session.snapshot_host().parse_cache.len(), entries);
    assert_eq!(session.snapshot().ref_count(), 1);
    let _ = Arc::strong_count(&session);
    let _ = projecttestutil::TestTypingsLocation;
}

fn open(session: &crate::session::Session, files: &[(&str, &str)], name: &str) {
    session.did_open_file(&ctx(), uri(&format!("file://{name}")), 1, file(files, name), lsproto::LanguageKind::TypeScript);
}

fn ls_program(session: &crate::session::Session, name: &str) -> &'static tsrs_compiler::Program {
    session.get_language_service(&ctx(), &uri(&format!("file://{name}"))).unwrap().get_program()
}

fn watch_changed(session: &crate::session::Session, name: &str, type_: lsproto::FileChangeType) {
    session.did_change_watched_files(&ctx(), &[lsproto::FileEvent { type_, uri: uri(&format!("file://{name}")) }]);
}

fn delete_close_recreate(files: &[(&str, &str)]) {
    let (session, utils) = setup(files);

    open(&session, files, "/home/projects/TS/p1/src/x.ts");
    open(&session, files, "/home/projects/TS/p1/src/index.ts");

    utils.fs().remove("/home/projects/TS/p1/src/x.ts").unwrap();

    session.did_close_file(&ctx(), uri("file:///home/projects/TS/p1/src/x.ts"));
    let program = ls_program(&session, "/home/projects/TS/p1/src/index.ts");
    assert!(program.get_source_file("/home/projects/TS/p1/src/x.ts").is_none());

    utils.fs().write_file("/home/projects/TS/p1/src/x.ts", "").unwrap();

    session.did_open_file(&ctx(), uri("file:///home/projects/TS/p1/src/x.ts"), 1, String::new(), lsproto::LanguageKind::TypeScript);

    let program = ls_program(&session, "/home/projects/TS/p1/src/x.ts");
    assert_eq!(program.get_source_file("/home/projects/TS/p1/src/x.ts").unwrap().text(), "");
}

// session_test.go:402 TestSession/DidCloseFile/Configured projects/delete a file, close it, recreate it
#[test]
fn did_close_file_configured_delete_close_recreate() {
    delete_close_recreate(default_files);
}

// session_test.go:433 TestSession/DidCloseFile/Inferred projects/delete a file, close it, recreate it
#[test]
fn did_close_file_inferred_delete_close_recreate() {
    let files: Vec<(&str, &str)> = default_files.iter().copied().filter(|(k, _)| *k != "/home/projects/TS/p1/tsconfig.json").collect();
    delete_close_recreate(&files);
}

// session_test.go:464 TestSession/DidCloseFile/Inferred projects/close untitled file
#[test]
fn did_close_file_close_untitled_file() {
    let (session, _) = setup(default_files);

    session.did_open_file(&ctx(), uri("untitled:Untitled-1"), 1, "let x = 1;".to_string(), lsproto::LanguageKind::TypeScript);
    session.did_close_file(&ctx(), uri("untitled:Untitled-1"));
    session.did_open_file(&ctx(), uri("untitled:Untitled-2"), 1, String::new(), lsproto::LanguageKind::TypeScript);
}

fn did_save_file(save_first: bool) {
    let (session, _) = setup(default_files);
    open(&session, default_files, "/home/projects/TS/p1/src/index.ts");

    assert_eq!(session.snapshot().id(), 1);

    if save_first {
        session.did_save_file(&ctx(), uri("file:///home/projects/TS/p1/src/index.ts"));
        watch_changed(&session, "/home/projects/TS/p1/src/index.ts", lsproto::FileChangeType::Changed);
    } else {
        watch_changed(&session, "/home/projects/TS/p1/src/index.ts", lsproto::FileChangeType::Changed);
        session.did_save_file(&ctx(), uri("file:///home/projects/TS/p1/src/index.ts"));
    }

    session.wait_for_background_tasks();
    // We didn't need a snapshot change, but the session overlays should be updated.
    assert_eq!(session.snapshot().id(), 1);

    // Open another file to force a snapshot update so we can see the changes.
    open(&session, default_files, "/home/projects/TS/p1/src/x.ts");
    assert!(session.snapshot().get_file("/home/projects/TS/p1/src/index.ts").unwrap().matches_disk_text());
}

// session_test.go:477 TestSession/DidSaveFile/save event first
#[test]
fn did_save_file_save_event_first() {
    did_save_file(true);
}

// session_test.go:504 TestSession/DidSaveFile/watch event first
#[test]
fn did_save_file_watch_event_first() {
    did_save_file(false);
}

// session_test.go:534 TestSession/Source file sharing/projects with similar options share source files
#[test]
fn source_file_sharing_similar_options() {
    let mut files: Vec<(&str, &str)> = default_files.to_vec();
    files.push((
        "/home/projects/TS/p2/tsconfig.json",
        r#"{
				"compilerOptions": {
					"noLib": true,
					"module": "nodenext",
					"strict": true,
					"noCheck": true
				}
			}"#,
    ));
    files.push(("/home/projects/TS/p2/src/index.ts", r#"import { x } from "../../p1/src/x";"#));
    let (session, _) = setup(&files);

    open(&session, &files, "/home/projects/TS/p1/src/index.ts");
    open(&session, &files, "/home/projects/TS/p2/src/index.ts");

    assert_eq!(session.snapshot().project_collection.projects().len(), 2);

    let program1 = ls_program(&session, "/home/projects/TS/p1/src/index.ts");
    let program2 = ls_program(&session, "/home/projects/TS/p2/src/index.ts");

    assert_eq!(program1.get_source_file("/home/projects/TS/p1/src/x.ts"), program2.get_source_file("/home/projects/TS/p1/src/x.ts"));
}

// session_test.go:569 TestSession/Source file sharing/projects with different options do not share source files
#[test]
fn source_file_sharing_different_options() {
    let mut files: Vec<(&str, &str)> = default_files.to_vec();
    files.push((
        "/home/projects/TS/p2/tsconfig.json",
        r#"{
				"compilerOptions": {
					"noLib": true,
					"module": "nodenext",
					"strict": true,
					"moduleDetection": "auto"
				},
				"include": ["src"]
			}"#,
    ));
    files.push(("/home/projects/TS/p2/src/index.ts", r#"import { x } from "../../p1/src/x";"#));
    let (session, _) = setup(&files);

    open(&session, &files, "/home/projects/TS/p1/src/index.ts");
    open(&session, &files, "/home/projects/TS/p2/src/index.ts");

    assert_eq!(session.snapshot().project_collection.projects().len(), 2);

    let x1 = ls_program(&session, "/home/projects/TS/p1/src/index.ts").get_source_file("/home/projects/TS/p1/src/x.ts").unwrap();
    let x2 = ls_program(&session, "/home/projects/TS/p2/src/index.ts").get_source_file("/home/projects/TS/p1/src/x.ts").unwrap();
    assert_ne!(x1, x2);
}

// session_test.go:608 TestSession/DidChangeWatchedFiles/change open file
#[test]
fn did_change_watched_files_change_open_file() {
    let (session, utils) = setup(default_files);

    open(&session, default_files, "/home/projects/TS/p1/src/x.ts");
    open(&session, default_files, "/home/projects/TS/p1/src/index.ts");

    let program_before = ls_program(&session, "/home/projects/TS/p1/src/index.ts");

    utils.fs().write_file("/home/projects/TS/p1/src/x.ts", "export const x = 2;").unwrap();

    watch_changed(&session, "/home/projects/TS/p1/src/x.ts", lsproto::FileChangeType::Changed);

    // Program should remain the same since the file is open and changes are handled through DidChangeTextDocument
    assert!(std::ptr::eq(program_before, ls_program(&session, "/home/projects/TS/p1/src/index.ts")));
}

// session_test.go:636 TestSession/DidChangeWatchedFiles/change closed program file
#[test]
fn did_change_watched_files_change_closed_program_file() {
    let (session, utils) = setup(default_files);

    open(&session, default_files, "/home/projects/TS/p1/src/index.ts");

    let program_before = ls_program(&session, "/home/projects/TS/p1/src/index.ts");

    utils.fs().write_file("/home/projects/TS/p1/src/x.ts", "export const x = 2;").unwrap();

    watch_changed(&session, "/home/projects/TS/p1/src/x.ts", lsproto::FileChangeType::Changed);

    assert!(!std::ptr::eq(program_before, ls_program(&session, "/home/projects/TS/p1/src/index.ts")));
}

// projecttestutil.go:281 WithRequestID(t.Context())
fn request_ctx() -> Context {
    let (ctx, cancel) = Context::background().with_cancel();
    std::mem::forget(cancel);
    tsrs_core::context::with_request_id(&ctx, "0")
}

fn semantic_diagnostics_count(program: &'static tsrs_compiler::Program, file_name: &str) -> usize {
    program.get_semantic_diagnostics(&request_ctx(), program.get_source_file(file_name)).len()
}

fn root_file_names(program: &'static tsrs_compiler::Program) -> Vec<String> {
    program.command_line().parsed_config.file_names.clone()
}

// session_test.go:713 TestSession/DidChangeWatchedFiles/change config file
#[test]
fn did_change_watched_files_change_config_file() {
    let files: &[(&str, &str)] = &[
        (
            "/home/projects/TS/p1/tsconfig.json",
            r#"{
					"compilerOptions": {
						"noLib": true,
						"strict": false
					}
				}"#,
        ),
        ("/home/projects/TS/p1/src/x.ts", "export declare const x: number | undefined;"),
        (
            "/home/projects/TS/p1/src/index.ts",
            r#"
					import { x } from "./x";
					let y: number = x;"#,
        ),
    ];

    let (session, utils) = setup(files);
    open(&session, files, "/home/projects/TS/p1/src/index.ts");

    let program = ls_program(&session, "/home/projects/TS/p1/src/index.ts");
    assert_eq!(semantic_diagnostics_count(program, "/home/projects/TS/p1/src/index.ts"), 0);

    utils
        .fs()
        .write_file(
            "/home/projects/TS/p1/tsconfig.json",
            r#"{
				"compilerOptions": {
					"noLib": false,
					"strict": true
				}
			}"#,
        )
        .unwrap();

    watch_changed(&session, "/home/projects/TS/p1/tsconfig.json", lsproto::FileChangeType::Changed);

    let program = ls_program(&session, "/home/projects/TS/p1/src/index.ts");
    assert_eq!(semantic_diagnostics_count(program, "/home/projects/TS/p1/src/index.ts"), 1);
}

// session_test.go:757 TestSession/DidChangeWatchedFiles/delete explicitly included file
#[test]
fn did_change_watched_files_delete_explicitly_included_file() {
    let files: &[(&str, &str)] = &[
        (
            "/home/projects/TS/p1/tsconfig.json",
            r#"{
					"compilerOptions": {
						"noLib": true
					},
					"files": ["src/index.ts", "src/x.ts"]
				}"#,
        ),
        ("/home/projects/TS/p1/src/x.ts", "export declare const x: number | undefined;"),
        ("/home/projects/TS/p1/src/index.ts", r#"import { x } from "./x";"#),
    ];
    let (session, utils) = setup(files);
    open(&session, files, "/home/projects/TS/p1/src/index.ts");

    let program = ls_program(&session, "/home/projects/TS/p1/src/index.ts");
    assert!(root_file_names(program).contains(&"/home/projects/TS/p1/src/x.ts".to_string()));
    assert_eq!(semantic_diagnostics_count(program, "/home/projects/TS/p1/src/index.ts"), 0);

    utils.fs().remove("/home/projects/TS/p1/src/x.ts").unwrap();

    watch_changed(&session, "/home/projects/TS/p1/src/x.ts", lsproto::FileChangeType::Deleted);

    let program = ls_program(&session, "/home/projects/TS/p1/src/index.ts");
    // File name is still in the command line, was explicitly included
    assert!(root_file_names(program).contains(&"/home/projects/TS/p1/src/x.ts".to_string()));
    assert_eq!(semantic_diagnostics_count(program, "/home/projects/TS/p1/src/index.ts"), 1);
    assert!(program.get_source_file("/home/projects/TS/p1/src/x.ts").is_none());

    // Open file to trigger cleanup
    session.did_open_file(&ctx(), uri("untitled:Untitled-1"), 1, String::new(), lsproto::LanguageKind::TypeScript);
    assert!(session.snapshot().get_file("/home/projects/TS/p1/src/x.ts").is_none());
}

// session_test.go:802 TestSession/DidChangeWatchedFiles/delete wildcard included file
#[test]
fn did_change_watched_files_delete_wildcard_included_file() {
    let files: &[(&str, &str)] = &[
        (
            "/home/projects/TS/p1/tsconfig.json",
            r#"{
					"compilerOptions": {
						"noLib": true
					},
					"include": ["src"]
				}"#,
        ),
        ("/home/projects/TS/p1/src/index.ts", "let x = 2;"),
        ("/home/projects/TS/p1/src/x.ts", "let y = x;"),
    ];
    let (session, utils) = setup(files);
    open(&session, files, "/home/projects/TS/p1/src/x.ts");

    let program = ls_program(&session, "/home/projects/TS/p1/src/x.ts");
    assert!(root_file_names(program).contains(&"/home/projects/TS/p1/src/index.ts".to_string()));
    assert_eq!(semantic_diagnostics_count(program, "/home/projects/TS/p1/src/x.ts"), 0);

    utils.fs().remove("/home/projects/TS/p1/src/index.ts").unwrap();

    watch_changed(&session, "/home/projects/TS/p1/src/index.ts", lsproto::FileChangeType::Deleted);

    let program = ls_program(&session, "/home/projects/TS/p1/src/x.ts");
    // File name is gone from the command line, was originally included via wildcard
    assert!(!root_file_names(program).contains(&"/home/projects/TS/p1/src/index.ts".to_string()));
    assert_eq!(semantic_diagnostics_count(program, "/home/projects/TS/p1/src/x.ts"), 1);

    // Open file to trigger cleanup
    session.did_open_file(&ctx(), uri("untitled:Untitled-1"), 1, String::new(), lsproto::LanguageKind::TypeScript);
    assert!(session.snapshot().get_file("/home/projects/TS/p1/src/index.ts").is_none());
}

fn delete_directory(files_spec: &str, check_root_names: bool) {
    let files: &[(&str, &str)] = &[
        ("/home/projects/TS/p1/tsconfig.json", files_spec),
        ("/home/projects/TS/p1/src/index.ts", r#"import { x } from "./sub/x";"#),
        ("/home/projects/TS/p1/src/sub/x.ts", "export const x = 1;"),
    ];
    let (session, utils) = setup(files);
    open(&session, files, "/home/projects/TS/p1/src/index.ts");

    let program = ls_program(&session, "/home/projects/TS/p1/src/index.ts");
    if check_root_names {
        assert!(root_file_names(program).contains(&"/home/projects/TS/p1/src/sub/x.ts".to_string()));
    } else {
        assert!(root_file_names(program).contains(&"/home/projects/TS/p1/src/index.ts".to_string()));
        // x.ts is not in "files" but is pulled in via the import.
        assert!(program.get_source_file("/home/projects/TS/p1/src/sub/x.ts").is_some());
    }
    assert_eq!(semantic_diagnostics_count(program, "/home/projects/TS/p1/src/index.ts"), 0);

    // Delete the entire subdirectory from the file system.
    utils.fs().remove("/home/projects/TS/p1/src/sub").unwrap();

    // Send a delete event for the directory URI.
    watch_changed(&session, "/home/projects/TS/p1/src/sub", lsproto::FileChangeType::Deleted);

    let program = ls_program(&session, "/home/projects/TS/p1/src/index.ts");
    if check_root_names {
        // The directory was deleted, so the file should no longer be in the program.
        assert!(!root_file_names(program).contains(&"/home/projects/TS/p1/src/sub/x.ts".to_string()));
    } else {
        // The directory was deleted, so the file should no longer be resolvable.
        assert!(program.get_source_file("/home/projects/TS/p1/src/sub/x.ts").is_none());
    }
    // The import should now be an error since the module is missing.
    assert_eq!(semantic_diagnostics_count(program, "/home/projects/TS/p1/src/index.ts"), 1);
}

// session_test.go:846 TestSession/DidChangeWatchedFiles/delete directory with wildcard included files
#[test]
fn did_change_watched_files_delete_directory_with_wildcard_included_files() {
    delete_directory(
        r#"{
					"compilerOptions": {
						"noLib": true
					},
					"include": ["src"]
				}"#,
        true,
    );
}

// session_test.go:892 TestSession/DidChangeWatchedFiles/delete directory with program-only files
#[test]
fn did_change_watched_files_delete_directory_with_program_only_files() {
    delete_directory(
        r#"{
					"compilerOptions": {
						"noLib": true
					},
					"files": ["src/index.ts"]
				}"#,
        false,
    );
}

fn delete_sibling_folder(open_third: bool) {
    let files: &[(&str, &str)] = &[
        (
            "/home/projects/TS/p1/tsconfig.json",
            if open_third {
                r#"{
					"compilerOptions": {
						"noLib": true
					},
					"files": ["index.ts", "third.ts"]
				}"#
            } else {
                r#"{
					"compilerOptions": {
						"noLib": true
					},
					"files": ["index.ts"]
				}"#
            },
        ),
        ("/home/projects/TS/p1/index.ts", "import { content } from \"./f/content\";\n\nexport const value = content;"),
        ("/home/projects/TS/p1/f/content.ts", "export const content = 1;"),
        ("/home/projects/TS/p1/third.ts", "export const third = 3;"),
    ];
    let (session, utils) = setup(files);
    let content_uri = uri("file:///home/projects/TS/p1/f/content.ts");
    open(&session, files, "/home/projects/TS/p1/index.ts");
    open(&session, files, "/home/projects/TS/p1/f/content.ts");

    let _ = ls_program(&session, "/home/projects/TS/p1/index.ts");
    session.wait_for_background_tasks();

    let baseline_refresh_count = utils.client().refresh_diagnostics_calls();

    utils.fs().remove("/home/projects/TS/p1/f").unwrap();

    watch_changed(&session, "/home/projects/TS/p1/f", lsproto::FileChangeType::Deleted);
    if open_third {
        open(&session, files, "/home/projects/TS/p1/third.ts");
    } else {
        session.did_close_file(&ctx(), content_uri);
    }
    // The debounced refresh runs after the session's (zero) debounce delay.
    session.wait_for_background_tasks();

    let refresh_count = utils.client().refresh_diagnostics_calls();
    assert!(refresh_count > baseline_refresh_count, "expected RefreshDiagnostics to be called, got {refresh_count} calls (baseline {baseline_refresh_count})");
}

// session_test.go:936 TestSession/DidChangeWatchedFiles/delete sibling folder schedules diagnostics refresh
#[test]
fn did_change_watched_files_delete_sibling_folder_schedules_diagnostics_refresh() {
    delete_sibling_folder(false);
}

// session_test.go:979 TestSession/DidChangeWatchedFiles/delete sibling folder schedules diagnostics refresh after opening third file
#[test]
fn did_change_watched_files_delete_sibling_folder_after_opening_third_file() {
    delete_sibling_folder(true);
}

// session_test.go:1024 TestSession/DidChangeWatchedFiles/create explicitly included file
#[test]
fn did_change_watched_files_create_explicitly_included_file() {
    let files: &[(&str, &str)] = &[
        (
            "/home/projects/TS/p1/tsconfig.json",
            r#"{
					"compilerOptions": {
						"noLib": true
					},
					"files": ["src/index.ts", "src/y.ts"]
				}"#,
        ),
        ("/home/projects/TS/p1/src/index.ts", r#"import { y } from "./y";"#),
    ];
    let (session, utils) = setup(files);
    open(&session, files, "/home/projects/TS/p1/src/index.ts");

    // Initially should have an error because y.ts is missing
    let program = ls_program(&session, "/home/projects/TS/p1/src/index.ts");
    assert_eq!(semantic_diagnostics_count(program, "/home/projects/TS/p1/src/index.ts"), 1);

    // Add the missing file
    utils.fs().write_file("/home/projects/TS/p1/src/y.ts", "export const y = 1;").unwrap();

    watch_changed(&session, "/home/projects/TS/p1/src/y.ts", lsproto::FileChangeType::Created);

    // Error should be resolved
    let program = ls_program(&session, "/home/projects/TS/p1/src/index.ts");
    assert_eq!(semantic_diagnostics_count(program, "/home/projects/TS/p1/src/index.ts"), 0);
}

// session_test.go:1064 TestSession/DidChangeWatchedFiles/create failed lookup location
#[test]
fn did_change_watched_files_create_failed_lookup_location() {
    let files: &[(&str, &str)] = &[
        (
            "/home/projects/TS/p1/tsconfig.json",
            r#"{
					"compilerOptions": {
						"noLib": true
					},
					"files": ["src/index.ts"]
				}"#,
        ),
        ("/home/projects/TS/p1/src/index.ts", r#"import { z } from "./z";"#),
    ];
    let (session, utils) = setup(files);
    open(&session, files, "/home/projects/TS/p1/src/index.ts");

    // Initially should have an error because z.ts is missing
    let program = ls_program(&session, "/home/projects/TS/p1/src/index.ts");
    assert_eq!(semantic_diagnostics_count(program, "/home/projects/TS/p1/src/index.ts"), 1);

    // Add a new file through failed lookup watch
    utils.fs().write_file("/home/projects/TS/p1/src/z.ts", "export const z = 1;").unwrap();
    watch_changed(&session, "/home/projects/TS/p1/src/z.ts", lsproto::FileChangeType::Created);

    // Error should be resolved and the new file should be included in the program
    let program = ls_program(&session, "/home/projects/TS/p1/src/index.ts");
    assert_eq!(semantic_diagnostics_count(program, "/home/projects/TS/p1/src/index.ts"), 0);
    assert!(program.get_source_file("/home/projects/TS/p1/src/z.ts").is_some());
}

// session_test.go:1104 TestSession/DidChangeWatchedFiles/create wildcard included file
#[test]
fn did_change_watched_files_create_wildcard_included_file() {
    let files: &[(&str, &str)] = &[
        (
            "/home/projects/TS/p1/tsconfig.json",
            r#"{
					"compilerOptions": {
						"noLib": true
					},
					"include": ["src"]
				}"#,
        ),
        ("/home/projects/TS/p1/src/index.ts", "a;"),
    ];
    let (session, utils) = setup(files);
    open(&session, files, "/home/projects/TS/p1/src/index.ts");

    // Initially should have an error because declaration for 'a' is missing
    let program = ls_program(&session, "/home/projects/TS/p1/src/index.ts");
    assert_eq!(semantic_diagnostics_count(program, "/home/projects/TS/p1/src/index.ts"), 1);

    // Add a new file through wildcard watch
    utils.fs().write_file("/home/projects/TS/p1/src/a.ts", "const a = 1;").unwrap();
    watch_changed(&session, "/home/projects/TS/p1/src/a.ts", lsproto::FileChangeType::Created);

    // Error should be resolved and the new file should be included in the program
    let program = ls_program(&session, "/home/projects/TS/p1/src/index.ts");
    assert_eq!(semantic_diagnostics_count(program, "/home/projects/TS/p1/src/index.ts"), 0);
    assert!(program.get_source_file("/home/projects/TS/p1/src/a.ts").is_some());
}

// session_test.go:1144 TestSession/DidChangeWatchedFiles/irrelevant extension changes are filtered out
#[test]
fn did_change_watched_files_irrelevant_extension_changes_are_filtered_out() {
    let files: &[(&str, &str)] = &[
        (
            "/home/projects/TS/p1/tsconfig.json",
            r#"{
					"compilerOptions": {
						"noLib": true
					},
					"include": ["src"]
				}"#,
        ),
        ("/home/projects/TS/p1/src/index.ts", "export const x = 1;"),
        ("/home/projects/TS/p1/src/data.txt", "some text"),
    ];
    let (session, utils) = setup(files);
    open(&session, files, "/home/projects/TS/p1/src/index.ts");

    let program = ls_program(&session, "/home/projects/TS/p1/src/index.ts");
    assert_eq!(semantic_diagnostics_count(program, "/home/projects/TS/p1/src/index.ts"), 0);
    let old_program = program;

    // Modify an irrelevant file and send change/create events for files with
    // extensions that are not relevant to TypeScript compilation.
    utils.fs().write_file("/home/projects/TS/p1/src/data.txt", "updated text").unwrap();

    session.did_change_watched_files(
        &ctx(),
        &[
            lsproto::FileEvent { type_: lsproto::FileChangeType::Changed, uri: uri("file:///home/projects/TS/p1/src/data.txt") },
            lsproto::FileEvent { type_: lsproto::FileChangeType::Created, uri: uri("file:///home/projects/TS/p1/src/styles.css") },
            lsproto::FileEvent { type_: lsproto::FileChangeType::Created, uri: uri("file:///home/projects/TS/p1/src/image.png") },
        ],
    );

    // The program should not have been rebuilt since all events had irrelevant extensions.
    let program = ls_program(&session, "/home/projects/TS/p1/src/index.ts");
    assert!(std::ptr::eq(program, old_program), "program should not be rebuilt for irrelevant extension changes");
}

// session_test.go:1192 TestSession/DidChangeWatchedFiles/pnpm install links local package
#[test]
fn did_change_watched_files_pnpm_install_links_local_package() {
    let files: &[(&str, &str)] = &[
        ("/home/projects/pnpm/pnpm-workspace.yaml", "packages:\n  - 'packages/*'"),
        ("/home/projects/pnpm/packages/alpha/package.json", r#"{ "name": "@repo/alpha", "main": "index.ts" }"#),
        (
            "/home/projects/pnpm/packages/alpha/tsconfig.json",
            r#"{
					"compilerOptions": { "noLib": true, "composite": true }
				}"#,
        ),
        ("/home/projects/pnpm/packages/alpha/index.ts", "export const alpha = 1;"),
        ("/home/projects/pnpm/packages/beta/package.json", r#"{ "name": "@repo/beta" }"#),
        (
            "/home/projects/pnpm/packages/beta/tsconfig.json",
            r#"{
					"compilerOptions": { "noLib": true }
				}"#,
        ),
        ("/home/projects/pnpm/packages/beta/index.ts", r#"import { alpha } from "@repo/alpha";"#),
    ];
    let (session, utils) = setup(files);
    open(&session, files, "/home/projects/pnpm/packages/beta/index.ts");

    // Before pnpm install: the import is unresolved because node_modules/@repo/alpha doesn't exist.
    let program = ls_program(&session, "/home/projects/pnpm/packages/beta/index.ts");
    assert_eq!(semantic_diagnostics_count(program, "/home/projects/pnpm/packages/beta/index.ts"), 1);

    // Simulate pnpm install: create a symlink from beta's node_modules/@repo/alpha to packages/alpha.
    utils.map_fs().mkdir_all("home/projects/pnpm/packages/beta/node_modules/@repo", tsrs_vfs::FileMode::Perm).unwrap();
    utils.map_fs().add_symlink("home/projects/pnpm/packages/beta/node_modules/@repo/alpha", "home/projects/pnpm/packages/alpha");

    // Fire watch events mimicking what VS Code sends for a pnpm install.
    let ev = |type_, u: &str| lsproto::FileEvent { type_, uri: uri(u) };
    session.did_change_watched_files(
        &ctx(),
        &[
            ev(lsproto::FileChangeType::Created, "file:///home/projects/pnpm/packages/beta/node_modules"),
            ev(lsproto::FileChangeType::Created, "file:///home/projects/pnpm/packages/beta/node_modules/%40repo"),
            ev(lsproto::FileChangeType::Created, "file:///home/projects/pnpm/packages/beta/node_modules/%40repo/alpha"),
            ev(lsproto::FileChangeType::Created, "file:///home/projects/pnpm/pnpm-lock.yaml"),
            ev(lsproto::FileChangeType::Changed, "file:///home/projects/pnpm/packages/beta/node_modules/.bin/tsc"),
            ev(lsproto::FileChangeType::Changed, "file:///home/projects/pnpm/packages/beta/node_modules/.bin/tsserver"),
        ],
    );

    // After pnpm install: the import should resolve.
    let program = ls_program(&session, "/home/projects/pnpm/packages/beta/index.ts");
    assert_eq!(semantic_diagnostics_count(program, "/home/projects/pnpm/packages/beta/index.ts"), 0);
}

// session_test.go:1244 TestSession/DidChangeWatchedFiles/symlinked node_modules package.json change invalidates resolution
#[test]
fn did_change_watched_files_symlinked_node_modules_package_json_change_invalidates_resolution() {
    let files: &[(&str, &str)] = &[
        (
            "/home/projects/myproject/tsconfig.json",
            r#"{
					"compilerOptions": {
						"noLib": true,
						"module": "nodenext",
						"moduleResolution": "nodenext"
					},
					"files": ["src/index.ts"]
				}"#,
        ),
        ("/home/projects/myproject/src/index.ts", r#"import { foo } from "mylib";"#),
        // The real package lives as a sibling directory
        (
            "/home/projects/mylib/package.json",
            r#"{
					"name": "mylib",
					"main": "dist/index.js"
				}"#,
        ),
        ("/home/projects/mylib/dist/index.js", "exports.foo = function() { return 1; };"),
        ("/home/projects/mylib/dist/index.d.ts", "export declare function foo(): number;"),
        // node_modules/mylib is a symlink to the sibling
        ("/home/projects/myproject/node_modules/mylib", "symlink:/home/projects/mylib"),
    ];

    let mut options = projecttestutil::default_session_options();
    options.current_directory = "/home/projects/myproject".to_string();
    options.push_diagnostics_enabled = false;
    let (session, utils) = projecttestutil::setup_with_options(files, Some(options));
    open(&session, files, "/home/projects/myproject/src/index.ts");

    // Initial state: import resolves successfully via package.json main -> dist/index.d.ts
    let program = ls_program(&session, "/home/projects/myproject/src/index.ts");
    session.wait_for_background_tasks();
    assert_eq!(semantic_diagnostics_count(program, "/home/projects/myproject/src/index.ts"), 0, "import should resolve initially");

    // Assert: watched file globs cover the realpath of package.json and dist/index.d.ts.
    assert!(utils.watches_file("/home/projects/mylib/package.json"), "realpath of package.json should be watched");
    assert!(utils.watches_file("/home/projects/mylib/dist/index.d.ts"), "realpath of dist/index.d.ts should be watched");

    // Edit package.json to remove "main" field
    utils.fs().write_file("/home/projects/mylib/package.json", "{\n\t\t\t\t\"name\": \"mylib\"\n\t\t\t}").unwrap();

    // Fire watch event for the realpath of the changed package.json.
    watch_changed(&session, "/home/projects/mylib/package.json", lsproto::FileChangeType::Changed);

    // After removing "main" from package.json, the import should no longer resolve.
    let program = ls_program(&session, "/home/projects/myproject/src/index.ts");
    assert!(semantic_diagnostics_count(program, "/home/projects/myproject/src/index.ts") > 0, "import should fail after removing main from package.json");
}

// session_test.go:1318 TestSession/DidChangeWatchedFiles/create file in non-existent directory
#[test]
fn did_change_watched_files_create_file_in_non_existent_directory() {
    let files: &[(&str, &str)] = &[
        (
            "/home/projects/TS/p1/tsconfig.json",
            r#"{
					"compilerOptions": {
						"noLib": true
					},
					"files": ["src/index.ts"]
				}"#,
        ),
        ("/home/projects/TS/p1/src/index.ts", r#"import { helper } from "./lib/helper";"#),
    ];
    let (session, utils) = setup(files);
    open(&session, files, "/home/projects/TS/p1/src/index.ts");

    // Initially should have an error because lib/helper.ts doesn't exist
    // and src/lib/ directory doesn't exist either.
    let program = ls_program(&session, "/home/projects/TS/p1/src/index.ts");
    assert_eq!(semantic_diagnostics_count(program, "/home/projects/TS/p1/src/index.ts"), 1);

    // Create the directory and file.
    utils.fs().write_file("/home/projects/TS/p1/src/lib/helper.ts", "export const helper = 1;").unwrap();
    watch_changed(&session, "/home/projects/TS/p1/src/lib/helper.ts", lsproto::FileChangeType::Created);

    // Error should be resolved.
    let program = ls_program(&session, "/home/projects/TS/p1/src/index.ts");
    assert_eq!(semantic_diagnostics_count(program, "/home/projects/TS/p1/src/index.ts"), 0);
    assert!(program.get_source_file("/home/projects/TS/p1/src/lib/helper.ts").is_some());
}

// session_test.go:1358 TestSession/DidChangeWatchedFiles/create symlink directory matching include pattern
#[test]
fn did_change_watched_files_create_symlink_directory_matching_include_pattern() {
    let files: &[(&str, &str)] = &[
        (
            "/home/projects/TS/p1/tsconfig.json",
            r#"{
					"compilerOptions": {
						"noLib": true
					},
					"include": ["src"]
				}"#,
        ),
        ("/home/projects/TS/p1/src/index.ts", "export const x = 1;"),
        ("/home/projects/TS/shared/utils.ts", r#"export const util = "hello";"#),
        ("/home/projects/TS/shared/helpers.ts", "export const helper = 42;"),
    ];
    let (session, utils) = setup(files);
    open(&session, files, "/home/projects/TS/p1/src/index.ts");

    let program = ls_program(&session, "/home/projects/TS/p1/src/index.ts");

    // Initially, project only has the one file in src/.
    let names = root_file_names(program);
    assert!(names.contains(&"/home/projects/TS/p1/src/index.ts".to_string()));
    assert!(!names.contains(&"/home/projects/TS/p1/src/linked/utils.ts".to_string()));
    assert!(!names.contains(&"/home/projects/TS/p1/src/linked/helpers.ts".to_string()));

    // Create a symlink directory inside src/ that points to the shared directory.
    utils.map_fs().add_symlink("home/projects/TS/p1/src/linked", "home/projects/TS/shared");

    // Send directory creation event (what VS Code sends when a symlink directory appears).
    watch_changed(&session, "/home/projects/TS/p1/src/linked", lsproto::FileChangeType::Created);

    // After the symlink directory is created, the files inside it should be
    // picked up by the wildcard include pattern.
    let program = ls_program(&session, "/home/projects/TS/p1/src/index.ts");
    let names = root_file_names(program);
    assert!(names.contains(&"/home/projects/TS/p1/src/index.ts".to_string()));
    assert!(names.contains(&"/home/projects/TS/p1/src/linked/utils.ts".to_string()));
    assert!(names.contains(&"/home/projects/TS/p1/src/linked/helpers.ts".to_string()));
}

// session_test.go:1405 TestSession/DidChangeWatchedFiles/skips irrelevant extensions
#[test]
fn did_change_watched_files_skips_irrelevant_extensions() {
    let files: &[(&str, &str)] = &[
        (
            "/home/projects/TS/p1/tsconfig.json",
            r#"{
					"compilerOptions": {},
					"include": ["src"]
				}"#,
        ),
        ("/home/projects/TS/p1/src/index.ts", "export const x = 1;"),
    ];
    let (session, utils) = setup(files);

    open(&session, files, "/home/projects/TS/p1/src/index.ts");
    session.wait_for_background_tasks();

    let refreshes = || utils.client().refresh_diagnostics_calls();
    let send = |events: &[(lsproto::FileChangeType, &str)]| {
        let events: Vec<lsproto::FileEvent> = events.iter().map(|(t, u)| lsproto::FileEvent { type_: *t, uri: uri(u) }).collect();
        session.did_change_watched_files(&ctx(), &events);
        session.wait_for_background_tasks();
    };
    let mut baseline_refresh_count = refreshes();

    // Scenario A: irrelevant .svg
    send(&[(lsproto::FileChangeType::Created, "file:///home/projects/TS/p1/icon.svg")]);
    assert_eq!(refreshes(), baseline_refresh_count, "irrelevant .svg should not trigger refresh");

    // Scenario B: relevant .ts
    send(&[(lsproto::FileChangeType::Created, "file:///home/projects/TS/p1/src/new.ts")]);
    assert!(refreshes() > baseline_refresh_count, "relevant .ts should trigger refresh");
    baseline_refresh_count = refreshes();

    // Scenario C: tsconfig.json
    send(&[(lsproto::FileChangeType::Changed, "file:///home/projects/TS/p1/tsconfig.json")]);
    assert!(refreshes() > baseline_refresh_count, "tsconfig.json should trigger refresh");
    baseline_refresh_count = refreshes();

    // Scenario D: directory creation (no extension)
    utils.map_fs().mkdir_all("home/projects/TS/p1/node_modules/@types", tsrs_vfs::FileMode::Perm).unwrap();
    send(&[(lsproto::FileChangeType::Created, "file:///home/projects/TS/p1/node_modules/@types")]);
    assert!(refreshes() > baseline_refresh_count, "directory change should trigger refresh");
    baseline_refresh_count = refreshes();

    // Scenario E: mixed batch
    send(&[
        (lsproto::FileChangeType::Created, "file:///home/projects/TS/p1/icon.png"),
        (lsproto::FileChangeType::Changed, "file:///home/projects/TS/p1/src/index.ts"),
    ]);
    assert!(refreshes() > baseline_refresh_count, "mixed batch with relevant file should trigger refresh");
    baseline_refresh_count = refreshes();

    // Scenario F: package install noise
    send(&[
        (lsproto::FileChangeType::Created, "file:///home/projects/TS/p1/node_modules/pkg/LICENSE"),
        (lsproto::FileChangeType::Created, "file:///home/projects/TS/p1/README.md"),
        (lsproto::FileChangeType::Created, "file:///home/projects/TS/p1/LICENSE.txt"),
        (lsproto::FileChangeType::Created, "file:///home/projects/TS/p1/style.css"),
    ]);
    assert_eq!(refreshes(), baseline_refresh_count, "package install noise should not trigger refresh");
}

// session_test.go:1481 TestSession/refreshes code lenses and inlay hints when relevant user preferences change
#[test]
fn refreshes_code_lenses_and_inlay_hints_when_relevant_user_preferences_change() {
    let files: &[(&str, &str)] = &[("/src/tsconfig.json", "{}"), ("/src/index.ts", "export const x = 1;")];
    let (session, utils) = setup(files);
    open(&session, files, "/src/index.ts");
    let _ = ls_program(&session, "/src/index.ts");

    session.configure(tsrs_ls::lsutil::new_default_user_preferences());
    // Change user preferences for code lens and inlay hints.
    let mut new_prefs = session.config();
    new_prefs.code_lens.references_code_lens_enabled = tsrs_core::Tristate::True;
    new_prefs.inlay_hints.include_inlay_function_like_return_type_hints = tsrs_core::Tristate::True;

    session.configure(new_prefs);

    assert_eq!(*utils.client().refresh_code_lens_calls.lock().unwrap(), 1, "expected one RefreshCodeLens call after code lens preference change");
    assert_eq!(*utils.client().refresh_inlay_hints_calls.lock().unwrap(), 1, "expected one RefreshInlayHints call after inlay hints preference change");
}

// session_test.go:1506 TestSession/sets locale when configured
#[test]
fn sets_locale_when_configured() {
    let (session, utils) = setup(&[]);
    let mut prefs = tsrs_ls::lsutil::new_default_user_preferences();
    prefs.locale = "fr".to_string();

    session.configure(prefs);

    let set_locale_calls = utils.client().set_locale_calls.lock().unwrap().clone();
    assert_eq!(set_locale_calls, vec!["fr".to_string()]);
}

// session_test.go:1519 TestSession/locale change invalidates programs
#[test]
fn locale_change_invalidates_programs() {
    let files: &[(&str, &str)] = &[("/src/tsconfig.json", "{}"), ("/src/index.ts", "export const x = 1;")];
    let (session, _) = setup(files);
    let config_path = Path("/src/tsconfig.json".to_string());
    open(&session, files, "/src/index.ts");
    let _ = ls_program(&session, "/src/index.ts");
    let program_of = || session.snapshot().project_collection.configured_project(&config_path).unwrap().program.unwrap();
    let initial_program = program_of();

    let mut preferences = session.config();
    preferences.code_lens.references_code_lens_enabled = tsrs_core::Tristate::True;
    session.configure(preferences.clone());
    let _ = ls_program(&session, "/src/index.ts");
    assert!(std::ptr::eq(program_of(), initial_program));

    preferences.locale = "fr".to_string();
    session.configure(preferences);
    let _ = ls_program(&session, "/src/index.ts");
    assert!(!std::ptr::eq(program_of(), initial_program));
    session.close();
}

// session_test.go:1551 TestSession/adds locale to background contexts
#[test]
fn adds_locale_to_background_contexts() {
    let (session, utils) = setup(&[]);
    *utils.client().locale.lock().unwrap() = "fr".to_string();
    *utils.client().refresh_code_lens_func.lock().unwrap() = Some(Box::new(|ctx: &Context| {
        assert_eq!(tsrs_core::context::locale_from_context(ctx).0, "fr");
    }));
    let mut prefs = tsrs_ls::lsutil::new_default_user_preferences();
    prefs.code_lens.references_code_lens_enabled = tsrs_core::Tristate::True;

    session.configure(prefs);

    assert_eq!(*utils.client().refresh_code_lens_calls.lock().unwrap(), 1);
}

// session_test.go:1571 TestSession/schedules diagnostics refresh when reportStyleChecksAsWarnings changes
#[test]
fn schedules_diagnostics_refresh_when_report_style_checks_as_warnings_changes() {
    let files: &[(&str, &str)] = &[("/src/tsconfig.json", "{}"), ("/src/index.ts", "export const x = 1;")];
    let (session, utils) = setup(files);
    open(&session, files, "/src/index.ts");
    let _ = ls_program(&session, "/src/index.ts");
    session.wait_for_background_tasks();

    // Record the baseline count of RefreshDiagnostics calls.
    let baseline_refresh_count = utils.client().refresh_diagnostics_calls();

    // Toggle reportStyleChecksAsWarnings (default is true, so set it to false).
    let mut prefs = tsrs_ls::lsutil::new_default_user_preferences();
    prefs.report_style_checks_as_warnings = tsrs_core::Tristate::False;
    session.configure(prefs);
    session.wait_for_background_tasks();

    assert!(utils.client().refresh_diagnostics_calls() > baseline_refresh_count);
}

// session_test.go:1598 TestSession/config parsing
#[test]
fn config_parsing() {
    use tsrs_ls::lsutil::{new_default_user_preferences, parse_user_preferences, OrganizeImportsSort, QuotePreference};
    let files: &[(&str, &str)] = &[("/src/tsconfig.json", "{}"), ("/src/index.ts", "export const x = 1;")];
    let (session, _) = setup(files);
    open(&session, files, "/src/index.ts");
    let _ = ls_program(&session, "/src/index.ts");

    let parse = |text: &str| match tsrs_core::json::unmarshal(text).unwrap() {
        tsrs_core::json::Value::Object(map) => parse_user_preferences(&map),
        _ => unreachable!(),
    };

    session.configure(parse(r#"{"js/ts": {"preferences": {"useAliasesForRenames": true, "quoteStyle": "single"}, "unstable": {"organizeImportsSort": "ordinalIgnoreCase"}}}"#));
    let mut expected_prefs1 = new_default_user_preferences();
    expected_prefs1.use_aliases_for_rename = tsrs_core::Tristate::True;
    expected_prefs1.quote_preference = QuotePreference::Single;
    expected_prefs1.organize_imports_sort = OrganizeImportsSort::OrdinalIgnoreCase;
    assert_eq!(session.config(), expected_prefs1);

    session.configure(parse(r#"{"js/ts": {"preferences": {"useAliasesForRenames": false, "quoteStyle": "double"}, "unstable": {"organizeImportsSort": "ordinal"}}}"#));
    let mut expected_prefs2 = new_default_user_preferences();
    expected_prefs2.use_aliases_for_rename = tsrs_core::Tristate::False;
    expected_prefs2.quote_preference = QuotePreference::Double;
    expected_prefs2.organize_imports_sort = OrganizeImportsSort::Ordinal;
    assert_eq!(session.config(), expected_prefs2);
}

// session_test.go:1649 TestSession/language service for closed files/closed file in configured project not yet opened
#[test]
fn language_service_for_closed_file_in_configured_project() {
    let files: &[(&str, &str)] = &[
        (
            "/home/projects/TS/p1/tsconfig.json",
            r#"{
					"compilerOptions": {
						"noLib": true,
						"strict": true
					},
					"include": ["src"]
				}"#,
        ),
        ("/home/projects/TS/p1/src/index.ts", "export const x: number = 1;"),
    ];
    let (session, _) = setup(files);

    // Do NOT open any file. Directly request language service for a closed file
    // that belongs to the configured project.
    let program = ls_program(&session, "/home/projects/TS/p1/src/index.ts");
    assert_eq!(program.get_source_file("/home/projects/TS/p1/src/index.ts").unwrap().text(), "export const x: number = 1;");
}

// session_test.go:1678 TestSession/language service for closed files/closed file with no configured project creates inferred project
#[test]
fn language_service_for_closed_file_without_configured_project() {
    let files: &[(&str, &str)] = &[("/home/projects/TS/loose/index.ts", r#"const greeting: string = "hello";"#)];
    let (session, _) = setup(files);

    let program = ls_program(&session, "/home/projects/TS/loose/index.ts");
    assert_eq!(program.get_source_file("/home/projects/TS/loose/index.ts").unwrap().text(), r#"const greeting: string = "hello";"#);
}

// session_test.go:1700 TestSession/jsconfig.json used for JS files when tsconfig.json exists in same directory
#[test]
fn jsconfig_used_for_js_files_when_tsconfig_exists_in_same_directory() {
    let files: &[(&str, &str)] = &[
        (
            "/home/projects/TS/p1/tsconfig.json",
            r#"{
				"compilerOptions": {
					"noLib": true,
					"strict": true
				}
			}"#,
        ),
        (
            "/home/projects/TS/p1/jsconfig.json",
            r#"{
				"compilerOptions": {
					"noLib": true,
					"checkJs": true
				}
			}"#,
        ),
        ("/home/projects/TS/p1/index.ts", "export const x: number = 1;"),
        ("/home/projects/TS/p1/app.js", r#"/** @type {number} */ var y = "not a number";"#),
    ];
    let (session, _) = setup(files);

    // Open the JS file - it should be assigned to the jsconfig.json project, not tsconfig.json
    session.did_open_file(&ctx(), uri("file:///home/projects/TS/p1/app.js"), 1, file(files, "/home/projects/TS/p1/app.js"), lsproto::LanguageKind::JavaScript);

    let default_project = session.snapshot().get_default_project(&uri("file:///home/projects/TS/p1/app.js")).expect("JS file should have a default project");
    assert_eq!(default_project.config_file_name(), "/home/projects/TS/p1/jsconfig.json", "JS file should belong to jsconfig.json project, not tsconfig.json");

    // Open the TS file - it should be assigned to tsconfig.json project
    open(&session, files, "/home/projects/TS/p1/index.ts");

    let default_ts_project = session.snapshot().get_default_project(&uri("file:///home/projects/TS/p1/index.ts")).expect("TS file should have a default project");
    assert_eq!(default_ts_project.config_file_name(), "/home/projects/TS/p1/tsconfig.json", "TS file should belong to tsconfig.json project");
}

// session_test.go:662 TestSession/DidChangeWatchedFiles/change program file not in tsconfig root files
#[test]
fn did_change_watched_files_change_program_file_not_in_tsconfig_root_files() {
    for workspace_dir in ["/", "/home/projects/TS/p1", "/somewhere/else/entirely"] {
        let files: &[(&str, &str)] = &[
            (
                "/home/projects/TS/p1/tsconfig.json",
                r#"{
							"compilerOptions": {
								"noLib": true,
								"module": "nodenext",
								"strict": true
							},
							"files": ["src/index.ts"]
						}"#,
            ),
            ("/home/projects/TS/p1/src/index.ts", r#"import { x } from "../../x";"#),
            ("/home/projects/TS/x.ts", "export const x = 1;"),
        ];

        let mut options = projecttestutil::default_session_options();
        options.current_directory = workspace_dir.to_string();
        options.push_diagnostics_enabled = false;
        let (session, utils) = projecttestutil::setup_with_options(files, Some(options));
        open(&session, files, "/home/projects/TS/p1/src/index.ts");
        let program_before = ls_program(&session, "/home/projects/TS/p1/src/index.ts");
        session.wait_for_background_tasks();

        assert!(utils.watches_file("/home/projects/ts/x.ts"), "workspaceDir={workspace_dir}");

        utils.fs().write_file("/home/projects/TS/x.ts", "export const x = 2;").unwrap();

        watch_changed(&session, "/home/projects/TS/x.ts", lsproto::FileChangeType::Changed);

        assert!(!std::ptr::eq(ls_program(&session, "/home/projects/TS/p1/src/index.ts"), program_before), "workspaceDir={workspace_dir}");
    }
}
