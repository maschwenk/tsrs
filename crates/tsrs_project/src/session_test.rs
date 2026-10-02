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
