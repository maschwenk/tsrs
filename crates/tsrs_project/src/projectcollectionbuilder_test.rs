use std::sync::Arc;

use tsrs_core::context::Context;
use tsrs_core::tspath::Path;
use tsrs_lsproto as lsproto;

use crate::dirty::Shared;
use crate::project::{Kind, Project};
use crate::projecttestutil::setup;
use crate::session::Session;

fn uri(s: &str) -> lsproto::DocumentUri {
    lsproto::DocumentUri(s.to_string())
}

fn ctx() -> Context {
    Context::background()
}

fn p(s: &str) -> Path {
    Path::from(s)
}

type Files = Vec<(String, String)>;

fn refs(files: &Files) -> Vec<(&str, &str)> {
    files.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
}

fn get(files: &Files, name: &str) -> String {
    files.iter().find(|(k, _)| k == name).unwrap().1.clone()
}

fn set(files: &mut Files, name: &str, content: &str) {
    files.retain(|(k, _)| k != name);
    files.push((name.to_string(), content.to_string()));
}

// projectcollectionbuilder_test.go:601
fn files_for_solution_config_file(solution_refs: &[&str], compiler_options: &str, own_files: &[&str]) -> Files {
    let mut compiler_options_str = String::new();
    if !compiler_options.is_empty() {
        compiler_options_str = format!("\"compilerOptions\": {{\n\t\t\t{compiler_options}\n\t\t}},");
    }
    let own_files_str = own_files.join(",");
    let references: Vec<String> = solution_refs.iter().map(|r| format!(r#"{{ "path": "{r}" }}"#)).collect();
    vec![
        (
            "/user/username/projects/myproject/tsconfig.json".to_string(),
            format!("{{\n\t\t\t{}\n\t\t\t\"files\": [{}],\n\t\t\t\"references\": [\n\t\t\t\t{}\n\t\t\t]\n\t\t}}", compiler_options_str, own_files_str, references.join(",")),
        ),
        (
            "/user/username/projects/myproject/tsconfig-src.json".to_string(),
            r#"{
			"compilerOptions": {
				"composite": true,
				"outDir": "./target",
			},
			"include": ["./src/**/*"]
		}"#
            .to_string(),
        ),
        (
            "/user/username/projects/myproject/src/main.ts".to_string(),
            "\n\t\t\timport { foo } from './helpers/functions';\n\t\t\texport { foo };".to_string(),
        ),
        ("/user/username/projects/myproject/src/helpers/functions.ts".to_string(), "export const foo = 1;".to_string()),
    ]
}

// projectcollectionbuilder_test.go:640
fn apply_indirect_project_files(files: &mut Files, project_index: usize, compiler_options: &str) {
    for (k, v) in files_for_indirect_project(project_index, compiler_options) {
        set(files, &k, &v);
    }
}

// projectcollectionbuilder_test.go:644
fn files_for_indirect_project(project_index: usize, compiler_options: &str) -> Files {
    vec![
        (
            format!("/user/username/projects/myproject/tsconfig-indirect{project_index}.json"),
            format!(
                r#"{{
			"compilerOptions": {{
				"composite": true,
				"outDir": "./target/",
				{compiler_options}
			}},
			"files": [
				"./indirect{project_index}/main.ts"
			],
			"references": [
				{{
				"path": "./tsconfig-src.json"
				}}
			]
		}}"#
            ),
        ),
        (format!("/user/username/projects/myproject/indirect{project_index}/main.ts"), "export const indirect = 1;".to_string()),
    ]
}

const main_uri: &str = "file:///user/username/projects/myproject/src/main.ts";

fn open_main(session: &Session, files: &Files) {
    session.did_open_file(&ctx(), uri(main_uri), 1, get(files, "/user/username/projects/myproject/src/main.ts"), lsproto::LanguageKind::TypeScript);
}

// Close the file and open one in an inferred project.
fn close_and_open_dummy(session: &Session, uri_to_close: &str) {
    session.did_close_file(&ctx(), uri(uri_to_close));
    session.did_open_file(&ctx(), uri("file:///user/username/workspaces/dummy/dummy.ts"), 1, "const x = 1;".to_string(), lsproto::LanguageKind::TypeScript);
    let snapshot = session.snapshot();
    assert_eq!(snapshot.project_collection.projects().len(), 1);
    assert!(snapshot.project_collection.inferred_project().is_some());
}

fn has_config(session: &Session, path: &str) -> bool {
    session.snapshot().config_file_registry.get_config(&p(path)).is_some()
}

fn same(a: &Option<Shared<Project>>, b: &Option<Shared<Project>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Shared::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

// projectcollectionbuilder_test.go:26
#[test]
fn when_project_found_is_solution_referencing_default_project_directly() {
    let files = files_for_solution_config_file(&["./tsconfig-src.json"], "", &[]);
    let (session, _) = setup(&refs(&files));

    // Ensure configured project is found for open file
    open_main(&session, &files);
    let snapshot = session.snapshot();
    assert_eq!(snapshot.project_collection.projects().len(), 1);
    assert!(snapshot.project_collection.configured_project(&p("/user/username/projects/myproject/tsconfig-src.json")).is_some());

    // Ensure request can use existing snapshot
    session.get_language_service(&ctx(), &uri(main_uri)).unwrap();
    let request_snapshot = session.snapshot();
    assert!(Arc::ptr_eq(&request_snapshot, &snapshot));

    // Searched configs should be present while file is open
    assert!(has_config(&session, "/user/username/projects/myproject/tsconfig.json"), "solution config should be present");
    assert!(has_config(&session, "/user/username/projects/myproject/tsconfig-src.json"), "direct reference should be present");

    close_and_open_dummy(&session, main_uri);

    // Config files should have been released
    assert!(!has_config(&session, "/user/username/projects/myproject/tsconfig.json"));
    assert!(!has_config(&session, "/user/username/projects/myproject/tsconfig-src.json"));
}

// projectcollectionbuilder_test.go:62
#[test]
fn when_project_found_is_solution_referencing_default_project_indirectly() {
    let mut files = files_for_solution_config_file(&["./tsconfig-indirect1.json", "./tsconfig-indirect2.json"], "", &[]);
    apply_indirect_project_files(&mut files, 1, "");
    apply_indirect_project_files(&mut files, 2, "");
    let (session, _) = setup(&refs(&files));

    // Ensure configured project is found for open file
    open_main(&session, &files);
    let snapshot = session.snapshot();
    assert_eq!(snapshot.project_collection.projects().len(), 1);
    let src_project = snapshot.project_collection.configured_project(&p("/user/username/projects/myproject/tsconfig-src.json"));
    assert!(src_project.is_some());

    // Verify the default project is the source project
    assert!(same(&snapshot.get_default_project(&uri(main_uri)), &src_project));

    // Searched configs should be present while file is open
    assert!(has_config(&session, "/user/username/projects/myproject/tsconfig.json"), "solution config should be present");
    assert!(has_config(&session, "/user/username/projects/myproject/tsconfig-indirect1.json"), "direct reference should be present");
    assert!(has_config(&session, "/user/username/projects/myproject/tsconfig-src.json"), "indirect reference should be present");

    close_and_open_dummy(&session, main_uri);

    // Config files should be released
    for config in ["tsconfig.json", "tsconfig-src.json", "tsconfig-indirect1.json", "tsconfig-indirect2.json"] {
        assert!(!has_config(&session, &format!("/user/username/projects/myproject/{config}")), "{config}");
    }
}

// projectcollectionbuilder_test.go:102
#[test]
fn when_project_found_is_solution_with_disable_referenced_project_load_referencing_default_project_directly() {
    let files = files_for_solution_config_file(&["./tsconfig-src.json"], r#""disableReferencedProjectLoad": true"#, &[]);
    let (session, _) = setup(&refs(&files));

    // Ensure no configured project is created due to disableReferencedProjectLoad
    open_main(&session, &files);
    let snapshot = session.snapshot();
    assert_eq!(snapshot.project_collection.projects().len(), 1);
    assert!(snapshot.project_collection.configured_project(&p("/user/username/projects/myproject/tsconfig-src.json")).is_none());

    // Should use inferred project instead
    let default_project = snapshot.get_default_project(&uri(main_uri)).unwrap();
    assert_eq!(default_project.kind, Kind::Inferred);

    // Searched configs should be present while file is open
    assert!(has_config(&session, "/user/username/projects/myproject/tsconfig.json"), "solution config should be present");
    assert!(!has_config(&session, "/user/username/projects/myproject/tsconfig-src.json"), "direct reference should not be present");

    close_and_open_dummy(&session, main_uri);

    // Config files should be released
    assert!(!has_config(&session, "/user/username/projects/myproject/tsconfig.json"));
    assert!(!has_config(&session, "/user/username/projects/myproject/tsconfig-src.json"));
}

// projectcollectionbuilder_test.go:137
#[test]
fn when_project_found_is_solution_referencing_default_project_indirectly_through_disable_referenced_project_load() {
    let mut files = files_for_solution_config_file(&["./tsconfig-indirect1.json"], "", &[]);
    apply_indirect_project_files(&mut files, 1, r#""disableReferencedProjectLoad": true"#);
    let (session, _) = setup(&refs(&files));

    // Ensure no configured project is created due to disableReferencedProjectLoad in indirect project
    open_main(&session, &files);
    let snapshot = session.snapshot();
    assert_eq!(snapshot.project_collection.projects().len(), 1);
    assert!(snapshot.project_collection.configured_project(&p("/user/username/projects/myproject/tsconfig-src.json")).is_none());

    // Should use inferred project instead
    assert_eq!(snapshot.get_default_project(&uri(main_uri)).unwrap().kind, Kind::Inferred);

    // Searched configs should be present while file is open
    assert!(has_config(&session, "/user/username/projects/myproject/tsconfig.json"), "solution config should be present");
    assert!(has_config(&session, "/user/username/projects/myproject/tsconfig-indirect1.json"), "solution direct reference should be present");
    assert!(!has_config(&session, "/user/username/projects/myproject/tsconfig-src.json"), "indirect reference should not be present");

    close_and_open_dummy(&session, main_uri);

    // Config files should be released
    for config in ["tsconfig.json", "tsconfig-src.json", "tsconfig-indirect1.json"] {
        assert!(!has_config(&session, &format!("/user/username/projects/myproject/{config}")), "{config}");
    }
}

// projectcollectionbuilder_test.go:175
#[test]
fn when_project_found_is_solution_referencing_default_project_indirectly_through_disable_referenced_project_load_in_one_but_without_it_in_another() {
    let mut files = files_for_solution_config_file(&["./tsconfig-indirect1.json", "./tsconfig-indirect2.json"], "", &[]);
    apply_indirect_project_files(&mut files, 1, r#""disableReferencedProjectLoad": true"#);
    apply_indirect_project_files(&mut files, 2, "");
    let (session, _) = setup(&refs(&files));

    // Ensure configured project is found through the indirect project without disableReferencedProjectLoad
    open_main(&session, &files);
    let snapshot = session.snapshot();
    assert_eq!(snapshot.project_collection.projects().len(), 1);
    let src_project = snapshot.project_collection.configured_project(&p("/user/username/projects/myproject/tsconfig-src.json"));
    assert!(src_project.is_some());

    // Verify the default project is the source project (found through indirect2, not indirect1)
    assert!(same(&snapshot.get_default_project(&uri(main_uri)), &src_project));

    // Searched configs should be present while file is open
    for config in ["tsconfig.json", "tsconfig-indirect1.json", "tsconfig-indirect2.json", "tsconfig-src.json"] {
        assert!(has_config(&session, &format!("/user/username/projects/myproject/{config}")), "{config} should be present");
    }

    close_and_open_dummy(&session, main_uri);

    // Config files should be released
    for config in ["tsconfig.json", "tsconfig-src.json", "tsconfig-indirect1.json", "tsconfig-indirect2.json"] {
        assert!(!has_config(&session, &format!("/user/username/projects/myproject/{config}")), "{config}");
    }
}

// projectcollectionbuilder_test.go:216
#[test]
#[ignore = "needs project references in tsrs_compiler (not ported: programs never resolve references)"]
fn when_project_found_is_project_with_own_files_referencing_the_file_from_referenced_project() {
    let mut files = files_for_solution_config_file(&["./tsconfig-src.json"], "", &[r#""./own/main.ts""#]);
    set(&mut files, "/user/username/projects/myproject/own/main.ts", "\n\t\t\timport { foo } from '../src/main';\n\t\t\tfoo;\n\t\t\texport function bar() {}\n\t\t");
    let (session, _) = setup(&refs(&files));

    // Ensure configured project is found for open file - should load both projects
    open_main(&session, &files);
    let snapshot = session.snapshot();
    assert_eq!(snapshot.project_collection.projects().len(), 2);
    let src_project = snapshot.project_collection.configured_project(&p("/user/username/projects/myproject/tsconfig-src.json"));
    assert!(src_project.is_some());
    assert!(snapshot.project_collection.configured_project(&p("/user/username/projects/myproject/tsconfig.json")).is_some());

    // Verify the default project is the source project
    assert!(same(&snapshot.get_default_project(&uri(main_uri)), &src_project));

    // Searched configs should be present while file is open
    assert!(has_config(&session, "/user/username/projects/myproject/tsconfig.json"), "solution config should be present");
    assert!(has_config(&session, "/user/username/projects/myproject/tsconfig-src.json"), "direct reference should be present");

    close_and_open_dummy(&session, main_uri);

    // Config files should be released
    assert!(!has_config(&session, "/user/username/projects/myproject/tsconfig.json"));
    assert!(!has_config(&session, "/user/username/projects/myproject/tsconfig-src.json"));
}

// projectcollectionbuilder_test.go:258
#[test]
fn when_file_is_not_part_of_first_config_tree_found_looks_into_ancestor_folder_and_its_references_to_find_default_project() {
    let files: Files = [
        (
            "/home/src/projects/project/app/Component-demos.ts",
            "\n                import * as helpers from 'demos/helpers';\n                export const demo = () => {\n                    helpers;\n                }\n            ",
        ),
        ("/home/src/projects/project/app/Component.ts", "export const Component = () => {}"),
        (
            "/home/src/projects/project/app/tsconfig.json",
            r#"{
				"compilerOptions": {
					"composite": true,
					"outDir": "../app-dist/",
				},
				"include": ["**/*"],
				"exclude": ["**/*-demos.*"],
			}"#,
        ),
        ("/home/src/projects/project/demos/helpers.ts", "export const foo = 1;"),
        (
            "/home/src/projects/project/demos/tsconfig.json",
            r#"{
				"compilerOptions": {
					"composite": true,
					"rootDir": "../",
					"outDir": "../demos-dist/",
					"paths": {
						"demos/*": ["./*"],
					},
				},
				"include": [
					"**/*",
					"../app/**/*-demos.*",
				],
			}"#,
        ),
        (
            "/home/src/projects/project/tsconfig.json",
            r#"{
				"compilerOptions": {
					"outDir": "./dist/",
				},
				"references": [
					{ "path": "./demos/tsconfig.json" },
					{ "path": "./app/tsconfig.json" },
				],
				"files": []
			}"#,
        ),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    let (session, _) = setup(&refs(&files));
    let u = "file:///home/src/projects/project/app/Component-demos.ts";

    // Ensure configured project is found for open file
    session.did_open_file(&ctx(), uri(u), 1, get(&files, "/home/src/projects/project/app/Component-demos.ts"), lsproto::LanguageKind::TypeScript);
    let snapshot = session.snapshot();
    assert_eq!(snapshot.project_collection.projects().len(), 2);
    let demo_project = snapshot.project_collection.configured_project(&p("/home/src/projects/project/demos/tsconfig.json"));
    assert!(demo_project.is_some());
    assert!(snapshot.project_collection.configured_project(&p("/home/src/projects/project/tsconfig.json")).is_some());

    // Verify the default project is the demos project (not the app project that excludes demos files)
    assert!(same(&snapshot.get_default_project(&uri(u)), &demo_project));

    // Searched configs should be present while file is open
    assert!(has_config(&session, "/home/src/projects/project/app/tsconfig.json"), "app config should be present");
    assert!(has_config(&session, "/home/src/projects/project/demos/tsconfig.json"), "demos config should be present");
    assert!(has_config(&session, "/home/src/projects/project/tsconfig.json"), "solution config should be present");

    close_and_open_dummy(&session, u);

    // Config files should be released
    assert!(!has_config(&session, "/home/src/projects/project/app/tsconfig.json"));
    assert!(!has_config(&session, "/home/src/projects/project/demos/tsconfig.json"));
    assert!(!has_config(&session, "/home/src/projects/project/tsconfig.json"));
}

// projectcollectionbuilder_test.go:338
#[test]
#[ignore = "needs project references in tsrs_compiler (not ported: programs never resolve references)"]
fn when_dts_file_is_next_to_ts_file_and_included_as_root_in_referenced_project() {
    let files: Files = [
        (
            "/home/src/projects/project/src/index.d.ts",
            "\n                 declare global {\n                    interface Window {\n                        electron: ElectronAPI\n                        api: unknown\n                    }\n                }\n            ",
        ),
        ("/home/src/projects/project/src/index.ts", "const api = {}"),
        (
            "/home/src/projects/project/tsconfig.json",
            r#"{
				"include": [
					"src/*.d.ts",
				],
				"references": [{ "path": "./tsconfig.node.json" }],
			}"#,
        ),
        (
            "/home/src/projects/project/tsconfig.node.json",
            r#"{
				"include": ["src/**/*"],
                "compilerOptions": {
                    "composite": true,
                },
			}"#,
        ),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    let (session, _) = setup(&refs(&files));
    let u = "file:///home/src/projects/project/src/index.d.ts";

    // Ensure configured projects are found for open file
    session.did_open_file(&ctx(), uri(u), 1, get(&files, "/home/src/projects/project/src/index.d.ts"), lsproto::LanguageKind::TypeScript);
    let snapshot = session.snapshot();
    assert_eq!(snapshot.project_collection.projects().len(), 2);
    assert!(snapshot.project_collection.configured_project(&p("/home/src/projects/project/tsconfig.json")).is_some());

    // Verify the default project is inferred
    assert_eq!(snapshot.get_default_project(&uri(u)).unwrap().kind, Kind::Inferred);

    // Searched configs should be present while file is open
    assert!(has_config(&session, "/home/src/projects/project/tsconfig.json"), "root config should be present");
    assert!(has_config(&session, "/home/src/projects/project/tsconfig.node.json"), "node config should be present");

    close_and_open_dummy(&session, u);

    // Config files should be released
    assert!(!has_config(&session, "/home/src/projects/project/tsconfig.json"));
    assert!(!has_config(&session, "/home/src/projects/project/tsconfig.node.json"));
}

// projectcollectionbuilder_test.go:396
#[test]
fn issue_1630() {
    let files: &[(&str, &str)] = &[
        ("/project/lib/tsconfig.json", r#"{
				"files": ["a.ts"]
			}"#),
        ("/project/lib/a.ts", "export const a = 1;"),
        ("/project/lib/b.ts", "export const b = 1;"),
        (
            "/project/tsconfig.json",
            r#"{
				"files": [],
				"references": [{ "path": "./lib" }],
				"compilerOptions": {
					"disableReferencedProjectLoad": true
				}
			}"#,
        ),
        ("/project/index.ts", ""),
    ];

    let (session, _) = setup(files);

    // opening b.ts puts /project/lib/tsconfig.json in the config file registry and creates the project,
    // but the project is ultimately not a match
    session.did_open_file(&ctx(), uri("file:///project/lib/b.ts"), 1, "export const b = 1;".to_string(), lsproto::LanguageKind::TypeScript);
    // opening an unrelated file triggers cleanup of /project/lib/tsconfig.json since no open file is part of that project,
    // but will keep the config file in the registry since lib/b.ts is still open
    session.did_open_file(&ctx(), uri("untitled:Untitled-1"), 1, String::new(), lsproto::LanguageKind::TypeScript);
    // Opening index.ts searches /project/tsconfig.json and then checks /project/lib/tsconfig.json without opening it.
    session.did_open_file(&ctx(), uri("file:///project/index.ts"), 1, String::new(), lsproto::LanguageKind::TypeScript);
}

// projectcollectionbuilder_test.go:428
#[test]
fn inferred_project_root_files_are_in_stable_order() {
    let files: &[(&str, &str)] = &[("/project/a.ts", "export const a = 1;"), ("/project/b.ts", "export const b = 1;"), ("/project/c.ts", "export const c = 1;")];

    let (session, _) = setup(files);

    // b, c, a
    for name in ["b", "c", "a"] {
        session.did_open_file(&ctx(), uri(&format!("file:///project/{name}.ts")), 1, format!("export const {name} = 1;"), lsproto::LanguageKind::TypeScript);
    }

    let snapshot = session.snapshot();
    assert_eq!(snapshot.project_collection.projects().len(), 1);
    let inferred_project = snapshot.project_collection.inferred_project().unwrap();
    assert_eq!(inferred_project.program.unwrap().command_line().file_names(), &["/project/a.ts", "/project/b.ts", "/project/c.ts"]);
}

// projectcollectionbuilder_test.go:457
#[test]
fn project_lookup_terminates() {
    let files: &[(&str, &str)] = &[
        (
            "/tsconfig.json",
            r#"{
				"files": [],
				"references": [
					{
						"path": "./packages/pkg1"
					},
					{
						"path": "./packages/pkg2"
					},
				]
			}"#,
        ),
        (
            "/packages/pkg1/tsconfig.json",
            r#"{
				"include": ["src/**/*.ts"],
				"compilerOptions": {
					"composite": true,
				},
				"references": [
					{
						"path": "../pkg2"
					},
				]
			}"#,
        ),
        (
            "/packages/pkg2/tsconfig.json",
            r#"{
				"include": ["src/**/*.ts"],
				"compilerOptions": {
					"composite": true,
				},
				"references": [
					{
						"path": "../pkg1"
					},
				]
			}"#,
        ),
        ("/script.ts", "export const a = 1;"),
    ];
    let (session, _) = setup(files);
    session.did_open_file(&ctx(), uri("file:///script.ts"), 1, "export const a = 1;".to_string(), lsproto::LanguageKind::TypeScript);
    // Test should terminate
}

// projectcollectionbuilder_test.go:500
#[test]
fn file_moves_to_inferred_project_after_import_is_deleted() {
    let files: &[(&str, &str)] = &[
        ("/project/tsconfig.json", r#"{"compilerOptions": {"strict": true}}"#),
        ("/project/index.ts", r#"import { helper } from "./node_modules/dep/index";"#),
        ("/project/node_modules/dep/index.d.ts", "export declare function helper(): void;"),
    ];
    let (session, _) = setup(files);

    // Step 1: Open the project root file
    let root_uri = uri("file:///project/index.ts");
    session.did_open_file(&ctx(), root_uri.clone(), 1, files[1].1.to_string(), lsproto::LanguageKind::TypeScript);
    session.get_language_service(&ctx(), &root_uri).unwrap();

    // Step 2: Open the node_modules dependency file - should be in the configured project
    let dep_uri = uri("file:///project/node_modules/dep/index.d.ts");
    session.did_open_file(&ctx(), dep_uri.clone(), 1, files[2].1.to_string(), lsproto::LanguageKind::TypeScript);

    let snapshot = session.snapshot();
    let configured_project = snapshot.project_collection.configured_project(&p("/project/tsconfig.json"));
    assert!(configured_project.is_some(), "configured project should exist");
    assert!(same(&snapshot.get_default_project(&dep_uri), &configured_project), "dependency should be in the configured project initially");

    // Step 3: Delete the import from the root file
    session.did_change_file(
        &ctx(),
        root_uri,
        2,
        vec![lsproto::TextDocumentContentChangePartialOrWholeDocument {
            whole_document: Some(lsproto::TextDocumentContentChangeWholeDocument { text: "// import removed".to_string() }),
            ..Default::default()
        }],
    );

    // Step 4: Request language service for the dependency - it should now be in an inferred project
    session.get_language_service(&ctx(), &dep_uri).unwrap();

    let snapshot = session.snapshot();
    let default_project = snapshot.get_default_project(&dep_uri).expect("dependency should have a default project");
    assert_eq!(default_project.kind, Kind::Inferred, "dependency should be in an inferred project after import is deleted");
}

// projectcollectionbuilder_test.go:544
#[test]
fn should_update_project_on_package_json_change() {
    let files: &[(&str, &str)] = &[
        (
            "/home/projects/myproject/tsconfig.json",
            r#"{
				"compilerOptions": {
					"module": "nodenext",
					"moduleResolution": "nodenext",
					"noLib": true,
					"noEmit": true
				}
			}"#,
        ),
        (
            "/home/projects/myproject/package.json",
            r##"{
				"name": "myproject",
				"type": "module",
				"imports": {
					"#utils": "./src/utils.ts"
				}
			}"##,
        ),
        ("/home/projects/myproject/src/index.ts", r##"import { add } from "#utils";"##),
        ("/home/projects/myproject/src/utils.ts", "export function add(a: number, b: number) { return a + b; }"),
    ];

    let (session, utils) = setup(files);
    let index_uri = uri("file:///home/projects/myproject/src/index.ts");
    session.did_open_file(&ctx(), index_uri.clone(), 1, files[2].1.to_string(), lsproto::LanguageKind::TypeScript);

    // Verify initial state: #utils resolves to utils.ts, so utils.ts is in the program
    let program = session.get_language_service(&ctx(), &index_uri).unwrap().get_program();
    assert_eq!(program.get_semantic_diagnostics(&ctx(), None).len(), 0, "should have no diagnostics with correct package.json");

    // Now change the package.json to point #utils at a non-existent file
    utils
        .fs()
        .write_file(
            "/home/projects/myproject/package.json",
            r##"{
			"name": "myproject",
			"type": "module",
			"imports": {
				"#utils": "./src/nonexistent.ts"
			}
		}"##,
        )
        .unwrap();
    session.did_change_watched_files(
        &ctx(),
        &[lsproto::FileEvent { uri: uri("file:///home/projects/myproject/package.json"), type_: lsproto::FileChangeType::Changed }],
    );

    let updated_program = session.get_language_service(&ctx(), &index_uri).unwrap().get_program();
    assert_eq!(updated_program.get_semantic_diagnostics(&ctx(), None).len(), 1, "should have diagnostics after package.json change");
}
