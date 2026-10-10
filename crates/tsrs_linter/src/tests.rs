mod no_floating_promises {
    use std::sync::Arc;

    use serde_json::{Value, json};
    use tsrs_vfs::{FS, bundled, vfstest};

    use crate::{
        FileConfig, Fixes, LintConfig, RequestedRule, RunLinterOptions, TypeErrors, run_linter,
    };

    use crate::NO_FLOATING_PROMISES;

    struct Diagnostics(crate::LinterResult);

    impl std::ops::Deref for Diagnostics {
        type Target = [crate::RuleDiagnostic];
        fn deref(&self) -> &Self::Target {
            &self.0.lint().diagnostics
        }
    }

    fn lint(code: &str, options: Value, suggestions: bool) -> Diagnostics {
        lint_files(&[("/file.ts", code)], options, suggestions)
    }

    fn lint_files(files: &[(&str, &str)], options: Value, suggestions: bool) -> Diagnostics {
        let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(vfstest::from_map(
            files.iter().copied(),
            true,
        )));
        let lint = Arc::new(
            LintConfig::new(
                &[FileConfig {
                    file_paths: vec!["/file.ts".into()],
                    rules: vec![RequestedRule {
                        name: NO_FLOATING_PROMISES.into(),
                        options,
                    }],
                }],
                "/",
                true,
                Fixes {
                    fix: false,
                    fix_suggestions: suggestions,
                },
                false,
            )
            .unwrap(),
        );
        let result = run_linter(&RunLinterOptions {
            current_directory: "/".to_string(),
            file_names: vec!["/file.ts".to_string()],
            fs,
            lint,
            type_errors: TypeErrors::default(),
            suppress_program_diagnostics: false,
        })
        .unwrap();
        assert!(
            result.diagnostics().is_empty(),
            "unexpected internal diagnostics: {:?}",
            result.diagnostics()
        );
        Diagnostics(result)
    }

    fn ids(code: &str, options: Value) -> Vec<String> {
        lint(code, options, false)
            .iter()
            .map(|diagnostic| diagnostic.message.id.clone())
            .collect()
    }

    #[test]
    fn handles_await_catch_then_void_and_assignments() {
        let code = r#"
async function test() {
  await Promise.resolve();
  Promise.resolve().catch(() => {});
  Promise.resolve().then(() => {}, () => {});
  void Promise.resolve();
  const value = Promise.resolve();
  return Promise.resolve();
}
"#;
        assert!(ids(code, Value::Null).is_empty());
    }

    #[test]
    fn reports_floating_promises_and_arrays() {
        let code = r#"
Promise.resolve();
declare const promises: Array<Promise<void>>;
promises;
Promise.reject().catch(undefined);
"#;
        assert_eq!(
            ids(code, Value::Null),
            [
                "floatingVoid",
                "floatingPromiseArrayVoid",
                "floatingUselessRejectionHandlerVoid"
            ]
        );
        assert_eq!(
            ids(code, json!({ "ignoreVoid": false })),
            [
                "floating",
                "floatingPromiseArray",
                "floatingUselessRejectionHandler"
            ]
        );
    }

    #[test]
    fn recognizes_derived_union_array_and_tuple_promises() {
        let code = r#"
declare class DerivedPromise extends Promise<void> {}
declare const derived: DerivedPromise;
declare const union: Promise<void> | number;
declare const array: Promise<void>[];
declare const tuple: [number, Promise<void>];
derived;
union;
array;
tuple;
"#;
        assert_eq!(
            ids(code, Value::Null),
            [
                "floatingVoid",
                "floatingVoid",
                "floatingPromiseArrayVoid",
                "floatingPromiseArrayVoid"
            ]
        );
    }

    #[test]
    fn checks_each_promise_branch() {
        let code = r#"
(Promise.resolve(), 1);
true ? 1 : Promise.resolve();
false || Promise.resolve();
"#;
        assert_eq!(
            ids(code, Value::Null),
            ["floatingVoid", "floatingVoid", "floatingVoid"]
        );
    }

    #[test]
    fn thenables_are_opt_in() {
        let code = r#"
declare const thenable: {
  then(onfulfilled: () => void, onrejected: () => void): unknown;
};
thenable;
"#;
        assert!(ids(code, Value::Null).is_empty());
        assert_eq!(
            ids(code, json!({ "checkThenables": true })),
            ["floatingVoid"]
        );
    }

    #[test]
    fn supports_safe_promise_and_call_specifiers() {
        let code = r#"
type SafePromise = Promise<void> & { readonly safe: unique symbol };
declare const safe: SafePromise;
declare function fire(): Promise<void>;
safe;
fire();
"#;
        let options = json!({
            "allowForKnownSafePromises": [{ "from": "file", "name": "SafePromise" }],
            "allowForKnownSafeCalls": [{ "from": "file", "name": "fire", "path": "/file.ts" }]
        });
        assert!(ids(code, options).is_empty());
    }

    #[test]
    fn supports_safe_call_specifiers_from_packages() {
        let files = [
            (
                "/file.ts",
                "import { fire } from 'safe-package';\nfire();\n",
            ),
            (
                "/node_modules/safe-package/package.json",
                r#"{ "name": "safe-package", "types": "index.d.ts" }"#,
            ),
            (
                "/node_modules/safe-package/index.d.ts",
                "export declare function fire(): Promise<void>;",
            ),
        ];
        assert_eq!(
            lint_files(&files, Value::Null, false)
                .iter()
                .map(|diagnostic| diagnostic.message.id.clone())
                .collect::<Vec<_>>(),
            ["floatingVoid"]
        );
        let options = json!({ "allowForKnownSafeCalls": [{ "from": "package", "name": "fire", "package": "safe-package" }] });
        assert!(lint_files(&files, options, false).is_empty());
    }

    #[test]
    fn computed_bindings_do_not_match_safe_call_declaration_sources() {
        let code = r#"
declare const key: "fire";
declare const api: { fire: () => Promise<void> };
const { [key]: trusted } = api;
trusted();
"#;
        assert_eq!(
            ids(
                code,
                json!({ "allowForKnownSafeCalls": [{ "from": "file", "path": "/file.ts", "name": "trusted" }] }),
            ),
            ["floatingVoid"]
        );
        assert!(
            ids(
                code,
                json!({ "allowForKnownSafeCalls": [{ "from": "file", "path": "/file.ts", "name": "trusted" }, "trusted"] }),
            )
            .is_empty()
        );
    }

    #[test]
    fn safe_call_package_sources_stop_at_named_modules_but_skip_namespaces() {
        let files = [
            (
                "/file.ts",
                "/// <reference path='./safe.d.ts' />\nimport { InnerModule, InnerNamespace } from 'safe-package';\nInnerModule.fire();\nInnerNamespace.fire();\n",
            ),
            (
                "/safe.d.ts",
                r#"
declare module "safe-package" {
    export module InnerModule { function fire(): Promise<void>; }
    export namespace InnerNamespace { function fire(): Promise<void>; }
}
"#,
            ),
        ];
        let options = json!({ "allowForKnownSafeCalls": [{ "from": "package", "name": "fire", "package": "safe-package" }] });
        let diagnostics = lint_files(&files, options, false);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].message.id, "floatingVoid");
        let range = diagnostics[0].range;
        assert_eq!(
            &files[0].1[range.pos() as usize..range.end() as usize],
            "InnerModule.fire()"
        );
    }

    #[test]
    fn safe_call_package_sources_fall_back_when_package_name_is_empty() {
        let files = [
            (
                "/file.ts",
                "import { fire } from 'safe-package';\nfire();\n",
            ),
            (
                "/node_modules/safe-package/package.json",
                r#"{ "name": "", "types": "index.d.ts" }"#,
            ),
            (
                "/node_modules/safe-package/index.d.ts",
                "export declare function fire(): Promise<void>;",
            ),
        ];
        assert_eq!(lint_files(&files, Value::Null, false).len(), 1);
        let options = json!({ "allowForKnownSafeCalls": [{ "from": "package", "name": "fire", "package": "safe-package" }] });
        assert!(lint_files(&files, options, false).is_empty());
    }

    #[test]
    fn ignores_iifes_when_configured() {
        let code = "(async () => {})();";
        assert_eq!(ids(code, Value::Null), ["floatingVoid"]);
        assert!(ids(code, json!({ "ignoreIIFE": true })).is_empty());
    }

    #[test]
    fn suggestions_match_the_headless_fix_contract() {
        let diagnostics = lint("Promise.resolve();", Value::Null, true);
        assert_eq!(diagnostics.len(), 1);
        let suggestions = &diagnostics[0].suggestions;
        assert_eq!(
            suggestions
                .iter()
                .map(|suggestion| suggestion.message.id.as_str())
                .collect::<Vec<_>>(),
            ["floatingFixVoid", "floatingFixAwait"]
        );
        assert_eq!(suggestions[0].fixes[0].text, "void ");
        assert_eq!(suggestions[1].fixes[0].text, "await ");
        assert_eq!(suggestions[0].fixes[0].range.pos(), 0);
        assert_eq!(suggestions[0].fixes[0].range.end(), 0);
    }
}
