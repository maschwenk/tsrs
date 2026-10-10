// Adapted from tsgolint's internal/rule_tester (MIT); see ../fixtures/tsgolint/LICENSE.
mod snapshot;

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;
use tsrs_core::tspath;
use tsrs_linter::{
    FileConfig, Fixes, LintConfig, RequestedRule, RuleDiagnostic, RuleFix, RunLinterOptions,
    TypeErrors, Workload, run_linter,
};
use tsrs_scanner::get_ecma_line_and_utf16_character_of_position;
use tsrs_vfs::{FS, bundled, vfstest};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suite {
    pub revision: String,
    files: BTreeMap<String, String>,
    pub cases: Vec<TestCase>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
pub struct TestCase {
    pub name: String,
    valid: bool,
    config: String,
    data: TestCaseData,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
struct TestCaseData {
    code: String,
    only: bool,
    skip: bool,
    file_name: String,
    options: Value,
    #[serde(rename = "TSConfig")]
    tsconfig: String,
    tsx: bool,
    files: Option<BTreeMap<String, String>>,
    output: Option<Vec<String>>,
    errors: Option<Vec<ExpectedError>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
struct ExpectedError {
    message_id: String,
    line: i32,
    column: i32,
    end_line: i32,
    end_column: i32,
    suggestions: Option<Vec<ExpectedSuggestion>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase", deny_unknown_fields)]
struct ExpectedSuggestion {
    message_id: String,
    output: String,
}

fn lint(
    suite: &Suite,
    case: &TestCase,
    code: &str,
    rule_name: &'static str,
) -> Vec<RuleDiagnostic> {
    const ROOT: &str = "/fixtures";
    let data = &case.data;
    let file_name = if data.file_name.is_empty() {
        if data.tsx { "react.tsx" } else { "file.ts" }
    } else {
        &data.file_name
    };
    let file_name = tspath::get_normalized_absolute_path(file_name, ROOT);
    let mut files: BTreeMap<_, _> = suite
        .files
        .iter()
        .map(|(path, text)| {
            (
                tspath::get_normalized_absolute_path(path, ROOT),
                text.clone(),
            )
        })
        .collect();
    files.insert(file_name.clone(), code.to_string());
    for (path, text) in data.files.iter().flatten() {
        files.insert(
            tspath::get_normalized_absolute_path(path, ROOT),
            text.clone(),
        );
    }
    let config = if data.tsconfig.is_empty() {
        &case.config
    } else {
        &data.tsconfig
    };
    let fs: Arc<dyn FS> = Arc::new(bundled::wrap_fs(vfstest::from_map(files, true)));
    let lint = Arc::new(
        LintConfig::new(
            &[FileConfig {
                file_paths: vec![file_name.clone()],
                rules: vec![RequestedRule {
                    name: rule_name.into(),
                    options: data.options.clone(),
                }],
            }],
            ROOT,
            true,
            Fixes {
                fix: true,
                fix_suggestions: true,
            },
            false,
        )
        .unwrap(),
    );
    let result = run_linter(&RunLinterOptions {
        current_directory: ROOT.to_string(),
        workload: Workload {
            programs: BTreeMap::from([(
                tspath::get_normalized_absolute_path(config, ROOT),
                vec![file_name],
            )]),
            unmatched_files: Vec::new(),
        },
        fs,
        lint,
        type_errors: TypeErrors::default(),
        suppress_program_diagnostics: false,
    })
    .unwrap_or_else(|error| panic!("{}: {error}", case.name));
    assert!(
        result.diagnostics.is_empty(),
        "unexpected internal diagnostics: {:?}",
        result.diagnostics
    );
    result.lint.diagnostics
}

// source_code_fixer.go: ApplyRuleFixes. Keep overlapping diagnostic fixes together.
fn apply_rule_fixes<'a>(
    code: &str,
    diagnostics: impl IntoIterator<Item = &'a [RuleFix]>,
) -> (String, bool) {
    let mut with_fixes: Vec<_> = diagnostics
        .into_iter()
        .filter(|fixes| !fixes.is_empty())
        .map(|fixes| {
            let mut fixes: Vec<_> = fixes.iter().collect();
            fixes.sort_by_key(|fix| (fix.range.pos(), fix.range.end()));
            fixes
        })
        .collect();
    with_fixes.sort_by_key(|fixes| (fixes[0].range.pos(), fixes.last().unwrap().range.end()));
    let mut output = String::new();
    let mut last_fix_end = 0;
    let mut fixed = false;
    for fixes in with_fixes {
        if last_fix_end > fixes[0].range.pos() as usize {
            continue;
        }
        for fix in fixes {
            fixed = true;
            output.push_str(&code[last_fix_end..fix.range.pos() as usize]);
            output.push_str(&fix.text);
            last_fix_end = fix.range.end() as usize;
        }
    }
    output.push_str(&code[last_fix_end..]);
    (output, fixed)
}

pub fn run_case(suite: &Suite, case: &TestCase, rule_name: &'static str, snapshots: &str) {
    let data = &case.data;
    assert!(
        !data.only,
        "upstream .Only would leave other cases untested"
    );
    if data.skip {
        eprintln!("explicitly running upstream skipped case {}", case.name);
    }
    if case.valid {
        let diagnostics = lint(suite, case, &data.code, rule_name);
        assert!(
            diagnostics.is_empty(),
            "{}: {diagnostics:#?}\n{}",
            case.name,
            data.code
        );
        return;
    }
    let mut initial_diagnostics = Vec::new();
    let mut outputs = Vec::new();
    let mut code = data.code.clone();
    for iteration in 0..10 {
        let diagnostics = lint(suite, case, &code, rule_name);
        let (fixed_code, fixed) =
            apply_rule_fixes(&code, diagnostics.iter().map(|d| d.fixes.as_slice()));
        if iteration == 0 {
            initial_diagnostics = diagnostics;
        }
        if !fixed {
            break;
        }
        code = fixed_code;
        outputs.push(code.clone());
    }
    snapshot::match_snapshot(&case.name, &data.code, &initial_diagnostics, snapshots);
    assert_eq!(
        outputs,
        data.output.as_deref().unwrap_or_default(),
        "{}: fixed output",
        case.name
    );
    let errors = data
        .errors
        .as_deref()
        .expect("invalid case must specify errors");
    assert_eq!(
        initial_diagnostics.len(),
        errors.len(),
        "{}: diagnostic count",
        case.name
    );
    for (diagnostic, expected) in initial_diagnostics.iter().zip(errors) {
        assert_eq!(
            diagnostic.message.id, expected.message_id,
            "{}: message",
            case.name
        );
        let (line, column) = get_ecma_line_and_utf16_character_of_position(
            diagnostic.source_file.get(),
            diagnostic.range.pos(),
        );
        let (end_line, end_column) = get_ecma_line_and_utf16_character_of_position(
            diagnostic.source_file.get(),
            diagnostic.range.end(),
        );
        for (label, wanted, actual) in [
            ("line", expected.line, line + 1),
            ("column", expected.column, column + 1),
            ("end line", expected.end_line, end_line + 1),
            ("end column", expected.end_column, end_column + 1),
        ] {
            if wanted != 0 {
                assert_eq!(actual, wanted, "{}: {label}", case.name);
            }
        }
        let suggestions = expected.suggestions.as_deref().unwrap_or_default();
        assert_eq!(
            diagnostic.suggestions.len(),
            suggestions.len(),
            "{}: suggestion count",
            case.name
        );
        for (suggestion, expected) in diagnostic.suggestions.iter().zip(suggestions) {
            assert_eq!(
                suggestion.message.id, expected.message_id,
                "{}: suggestion id",
                case.name
            );
            let (output, _) = apply_rule_fixes(&data.code, [suggestion.fixes.as_slice()]);
            assert_eq!(output, expected.output, "{}: suggestion output", case.name);
        }
    }
}
