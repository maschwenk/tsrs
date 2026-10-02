use std::fmt::Write as _;
use std::path::PathBuf;

use tsrs_ast::{Kind, Node, SourceFile, SourceFileParseOptions};
use tsrs_core::collections::OrderedMap;
use tsrs_core::json::{self, Value};
use tsrs_core::{position_to_line_and_byte_offset, ScriptKind, TextRange, P};

use super::*;

pub(crate) fn repo_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    while !dir.join("ts-ref").exists() {
        dir = dir.parent().expect("repository root with ts-ref").to_path_buf();
    }
    dir
}

fn test_files() -> Vec<PathBuf> {
    vec![repo_root().join("ts-ref/tsc/testdata/fixtures/services/mapCode.ts")]
}

pub(crate) fn parse(file_name: &str, text: &str, script_kind: ScriptKind) -> P<SourceFile> {
    tsrs_parser::parse_source_file(
        SourceFileParseOptions { file_name: file_name.to_string(), path: file_name.into(), ..Default::default() },
        text,
        script_kind,
    )
}

fn reference_baseline(name: &str) -> Option<String> {
    std::fs::read_to_string(repo_root().join("ts-ref/tsc/testdata/baselines/reference/astnav").join(name)).ok()
}

// baseline.Run: compare against the reference file; on mismatch write the local baseline for inspection.
fn check_baseline(name: &str, actual: &str) {
    let expected = reference_baseline(name).unwrap_or_else(|| "<no content>".to_string());
    if expected != actual {
        let dir = repo_root().join("target/scratch/lsfound/baselines/local/astnav");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), actual).unwrap();
        panic!("baseline mismatch for {name}: see target/scratch/lsfound/baselines/local/astnav/{name}");
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct TokenInfo {
    kind: String,
    pos: i32,
    end: i32,
}

// tokens_test.go:257
fn to_token_info(node: Option<P<Node>>) -> Option<TokenInfo> {
    let node = node?;
    let mut kind = format!("{:?}", node.kind());
    if kind == "EndOfFile" {
        kind = "EndOfFileToken".to_string();
    }
    Some(TokenInfo { kind, pos: node.pos(), end: node.end() })
}

struct TokenRun {
    start_pos: usize,
    end_pos: usize,
    kind: String,
    node_pos: i32,
    node_end: i32,
}

// tokens_test.go:196
fn baseline_go_tokens_json(test_name: &str, get_go_token: impl Fn(P<SourceFile>, i32) -> Option<TokenInfo>) {
    for file_name in test_files() {
        let file_text = std::fs::read_to_string(&file_name).unwrap();
        let file = parse("/file.ts", &file_text, ScriptKind::TS);

        let max_pos = file_text.len();
        let mut runs: Vec<TokenRun> = Vec::new();
        let mut current: Option<TokenRun> = None;

        for pos in 0..max_pos {
            let token = get_go_token(file, pos as i32);
            match (&mut current, &token) {
                (Some(c), Some(t)) if c.kind == t.kind && c.node_pos == t.pos && c.node_end == t.end => {
                    c.end_pos = pos;
                }
                _ => {
                    if let Some(c) = current.take() {
                        runs.push(c);
                    }
                    current = token.map(|t| TokenRun { start_pos: pos, end_pos: pos, kind: t.kind, node_pos: t.pos, node_end: t.end });
                }
            }
        }
        if let Some(c) = current {
            runs.push(c);
        }

        let values: Vec<Value> = runs
            .iter()
            .map(|r| {
                let mut m: OrderedMap<String, Value> = OrderedMap::default();
                m.insert("startPos".into(), Value::Number(r.start_pos as f64));
                m.insert("endPos".into(), Value::Number(r.end_pos as f64));
                m.insert("kind".into(), Value::String(r.kind.clone()));
                m.insert("nodePos".into(), Value::Number(r.node_pos as f64));
                m.insert("nodeEnd".into(), Value::Number(r.node_end as f64));
                Value::Object(m)
            })
            .collect();
        let output = if values.is_empty() { "null".to_string() } else { json::marshal_indent(&Value::Array(values), "", "  ").unwrap() };

        let base = file_name.file_name().unwrap().to_string_lossy().to_string();
        check_baseline(&format!("{test_name}.{base}.baseline.json"), &output);
    }
}

// Go's baselineTokens compares against tokens computed by the TypeScript (JS) implementation through node and
// `typescript` from the repository's node_modules (jstest.EvalNodeScriptWithTS). The TS side is reproduced here by the
// same script when `TSRS_TYPESCRIPT_JS` points at a typescript.js (6.0.3, the version in ts-ref/package-lock.json);
// without it, the test is skipped like Go's `jstest.SkipIfNoNodeJS`.
fn ts_tokens(function: &str, file_text: &str, positions: &[usize]) -> Option<Vec<Option<TokenInfo>>> {
    let ts_js = std::env::var("TSRS_TYPESCRIPT_JS").ok()?;
    let dir = repo_root().join(format!("target/scratch/lsfound/jstest/{function}"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("file.ts"), file_text).unwrap();
    let positions_json = format!("[{}]", positions.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(","));
    std::fs::write(dir.join("positions.json"), positions_json).unwrap();
    let call = match function {
        "GetTokenAtPosition" => "ts.getTokenAtPosition(file, position)",
        "GetTouchingPropertyName" => "ts.getTouchingPropertyName(file, position)",
        "FindPrecedingToken" => "ts.findPrecedingToken(position, file)",
        _ => unreachable!(),
    };
    let script = format!(
        r#"
const fs = require("fs");
const ts = require({ts_js:?});
const positions = JSON.parse(fs.readFileSync("positions.json", "utf8"));
const fileText = fs.readFileSync("file.ts", "utf8");
const file = ts.createSourceFile("file.ts", fileText, {{ languageVersion: ts.ScriptTarget.Latest, jsDocParsingMode: ts.JSDocParsingMode.ParseAll }}, true);
const result = positions.map(position => {{
    let token = {call};
    if (token === undefined) return null;
    if (token.kind === ts.SyntaxKind.SyntaxList) token = token.parent;
    return {{ kind: ts.Debug.formatSyntaxKind(token.kind), pos: token.pos, end: token.end }};
}});
fs.writeFileSync("result.json", JSON.stringify(result));
"#
    );
    std::fs::write(dir.join("script.cjs"), script).unwrap();
    let status = std::process::Command::new("node").arg("script.cjs").current_dir(&dir).status().ok()?;
    assert!(status.success(), "node script failed");
    let result = std::fs::read_to_string(dir.join("result.json")).unwrap();
    let Value::Array(items) = json::unmarshal(&result).unwrap() else {
        panic!("expected array");
    };
    Some(
        items
            .into_iter()
            .map(|item| match item {
                Value::Object(m) => Some(TokenInfo {
                    kind: match &m["kind"] {
                        Value::String(s) => s.clone(),
                        _ => panic!(),
                    },
                    pos: match m["pos"] {
                        Value::Number(n) => n as i32,
                        _ => panic!(),
                    },
                    end: match m["end"] {
                        Value::Number(n) => n as i32,
                        _ => panic!(),
                    },
                }),
                _ => None,
            })
            .collect(),
    )
}

#[derive(Clone, Default)]
struct TokenDiff {
    go_token: Option<TokenInfo>,
    ts_token: Option<TokenInfo>,
}

// tokens_test.go:140
fn baseline_tokens(test_name: &str, include_eof: bool, get_go_token: impl Fn(P<SourceFile>, i32) -> Option<TokenInfo>) {
    for file_name in test_files() {
        let file_text = std::fs::read_to_string(&file_name).unwrap();
        let positions: Vec<usize> = (0..file_text.len() + if include_eof { 1 } else { 0 }).collect();
        let Some(ts_tokens) = ts_tokens(test_name, &file_text, &positions) else {
            eprintln!("skipping {test_name} TS comparison: TSRS_TYPESCRIPT_JS not set");
            return;
        };
        let file = parse("/file.ts", &file_text, ScriptKind::TS);

        let mut output = String::new();
        let mut current_range = TextRange::new(0, 0);
        let mut current_diff = TokenDiff::default();

        for (pos, ts_token) in ts_tokens.iter().enumerate() {
            let go_token = get_go_token(file, pos as i32);
            let diff = TokenDiff { go_token, ts_token: ts_token.clone() };

            if !(current_diff.go_token == diff.go_token && current_diff.ts_token == diff.ts_token) {
                if current_diff.go_token != current_diff.ts_token {
                    write_range_diff(&mut output, file, &current_diff, current_range, pos as i32);
                }
                current_diff = diff;
                current_range = TextRange::new(pos as i32, pos as i32);
            }
            current_range = current_range.with_end(pos as i32);
        }

        if current_diff.go_token != current_diff.ts_token {
            write_range_diff(&mut output, file, &current_diff, current_range, ts_tokens.len() as i32 - 1);
        }

        let base = file_name.file_name().unwrap().to_string_lossy().to_string();
        check_baseline(&format!("{test_name}.{base}.baseline.txt"), if output.is_empty() { "<no content>" } else { &output });
    }
}

// tokens_test.go:381
fn write_range_diff(output: &mut String, file: P<SourceFile>, diff: &TokenDiff, rng: TextRange, position: i32) {
    let lines = file.ecma_line_map();

    let mut ts_token_pos = position;
    let mut go_token_pos = position;
    let mut ts_token_end = position;
    let mut go_token_end = position;
    if let Some(t) = &diff.ts_token {
        ts_token_pos = t.pos;
        ts_token_end = t.end;
    }
    if let Some(g) = &diff.go_token {
        go_token_pos = g.pos;
        go_token_end = g.end;
    }
    let (ts_start_line, _) = position_to_line_and_byte_offset(ts_token_pos, lines);
    let (ts_end_line, _) = position_to_line_and_byte_offset(ts_token_end, lines);
    let (go_start_line, _) = position_to_line_and_byte_offset(go_token_pos, lines);
    let (go_end_line, _) = position_to_line_and_byte_offset(go_token_end, lines);

    let context_lines = 2usize;
    let start_line = ts_start_line.min(go_start_line);
    let end_line = ts_end_line.max(go_end_line);
    let mut marker_lines = [ts_start_line, ts_end_line, go_start_line, go_end_line];
    marker_lines.sort();
    let context_start = start_line.saturating_sub(context_lines);
    let context_end = (lines.len() - 1).min(end_line + context_lines);
    let digits = (context_end).to_string().len();

    let should_truncate = |line: usize| -> (bool, usize) {
        let index = marker_lines.partition_point(|&m| m < line);
        if index == 0 || index == marker_lines.len() {
            return (false, 0);
        }
        let low = marker_lines[index - 1];
        let high = marker_lines[index];
        if line - low > 5 && high - line > 5 {
            return (true, high - 5);
        }
        (false, 0)
    };

    if !output.is_empty() {
        output.push_str("\n\n");
    }

    let _ = writeln!(output, "〚Positions: [{}, {}]〛", rng.pos(), rng.end());
    match &diff.ts_token {
        Some(t) => {
            let _ = writeln!(output, "【TS: {} [{}, {})】", t.kind, ts_token_pos, ts_token_end);
        }
        None => output.push_str("【TS: nil】\n"),
    }
    match &diff.go_token {
        Some(g) => {
            let _ = writeln!(output, "《Go: {} [{}, {})》", g.kind, go_token_pos, go_token_end);
        }
        None => output.push_str("《Go: nil》\n"),
    }
    let text = file.text().as_bytes();
    let mut out_bytes: Vec<u8> = Vec::new();
    let mut line = context_start;
    while line <= context_end {
        let (truncate, skip_to) = should_truncate(line);
        if truncate {
            out_bytes.extend_from_slice(format!("{} │........ {} lines omitted ........\n", " ".repeat(digits), skip_to - line + 1).as_bytes());
            line = skip_to;
        }
        out_bytes.extend_from_slice(format!("{:>digits$} │", line + 1).as_bytes());
        let end = if line < lines.len() - 1 { lines[line + 1] as i32 } else { text.len() as i32 + 1 };
        let mut pos = lines[line] as i32;
        while pos < end {
            if pos == rng.end() + 1 {
                out_bytes.extend_from_slice("〛".as_bytes());
            }
            if diff.ts_token.is_some() && pos == ts_token_end {
                out_bytes.extend_from_slice("】".as_bytes());
            }
            if diff.go_token.is_some() && pos == go_token_end {
                out_bytes.extend_from_slice("》".as_bytes());
            }
            if diff.go_token.is_some() && pos == go_token_pos {
                out_bytes.extend_from_slice("《".as_bytes());
            }
            if diff.ts_token.is_some() && pos == ts_token_pos {
                out_bytes.extend_from_slice("【".as_bytes());
            }
            if pos == rng.pos() {
                out_bytes.extend_from_slice("〚".as_bytes());
            }
            if (pos as usize) < text.len() {
                out_bytes.push(text[pos as usize]);
            }
            pos += 1;
        }
        line += 1;
    }
    output.push_str(&String::from_utf8_lossy(&out_bytes));
}

// tokens_test.go:25
#[test]
fn test_get_token_at_position_baseline() {
    baseline_tokens("GetTokenAtPosition", false /*includeEOF*/, |file, pos| to_token_info(Some(get_token_at_position(file, pos))));
}

#[test]
fn test_get_token_at_position_go_baseline_json() {
    baseline_go_tokens_json("GetTokenAtPosition", |file, pos| to_token_info(Some(get_token_at_position(file, pos))));
}

#[test]
fn test_get_token_at_position_jsdoc_type_assertion() {
    let file_text = "function foo(x) {\n    const s = /**@type {string}*/(x)\n}";
    let file = parse("/test.js", file_text, ScriptKind::JS);

    // Position of 'x' inside the parenthesized expression (position 52)
    let position = 52;

    // This should not panic - it previously panicked with:
    // "did not expect KindParenthesizedExpression to have KindIdentifier in its trivia"
    let token = get_touching_property_name(file, position);

    // The function may return either the identifier itself or the containing
    // parenthesized expression, depending on how the AST is structured
    assert!(
        token.kind() == Kind::Identifier || token.kind() == Kind::ParenthesizedExpression,
        "Expected identifier or parenthesized expression, got {:?}",
        token.kind()
    );
}

#[test]
fn test_get_token_at_position_jsdoc_type_assertion_with_comment() {
    // Exact code from the issue report
    let file_text = "function foo(x) {\n    const s = /**@type {string}*/(x)  // Go-to-definition on x causes panic\n}";
    let file = parse("/test.js", file_text, ScriptKind::JS);

    // Find position of 'x' in the type assertion
    let x_pos = 52; // Position of 'x' in (x)

    // This should not panic
    let _token = get_touching_property_name(file, x_pos);
}

#[test]
fn test_get_token_at_position_pointer_equality() {
    let file_text = "\n\t\t\tfunction foo() {\n\t\t\t\treturn 0;\n\t\t\t}\n\t\t";
    let file = parse("/file.ts", file_text, ScriptKind::TS);
    assert_eq!(get_token_at_position(file, 0), get_token_at_position(file, 0));
}

// tokens_test.go:121
#[test]
fn test_get_touching_property_name_baseline() {
    baseline_tokens("GetTouchingPropertyName", false /*includeEOF*/, |file, pos| to_token_info(Some(get_touching_property_name(file, pos))));
}

#[test]
fn test_get_touching_property_name_go_baseline_json() {
    baseline_go_tokens_json("GetTouchingPropertyName", |file, pos| to_token_info(Some(get_touching_property_name(file, pos))));
}

// tokens_test.go:478
#[test]
fn test_find_preceding_token_baseline() {
    baseline_tokens("FindPrecedingToken", true /*includeEOF*/, |file, pos| to_token_info(find_preceding_token(file, pos)));
}

#[test]
fn test_find_preceding_token_go_baseline_json() {
    baseline_go_tokens_json("FindPrecedingToken", |file, pos| to_token_info(find_preceding_token(file, pos)));
}

// tokens_test.go:504
#[test]
fn test_find_next_token_go_baseline_json() {
    baseline_go_tokens_json("FindNextToken", |file, pos| {
        // FindNextToken panics (like Go's assert) when the scanner finds trivia between
        // previousToken.End() and the next syntactic token. Catch those to avoid crashing
        // the baseline generator; those positions will be absent from the baseline.
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let token = get_token_at_position(file, pos);
            let next = find_next_token(token, file.as_node(), file);
            to_token_info(next)
        }))
        .unwrap_or(None)
    });
}

// tokens_test.go:524
#[test]
fn test_unit_find_preceding_token() {
    struct TestCase {
        name: &'static str,
        file_content: &'static str,
        position: i32,
        expected_kind: Kind,
    }
    let test_cases = [
        TestCase {
            name: "after dot in jsdoc",
            file_content: r#"import {
    CharacterCodes,
    compareStringsCaseInsensitive,
    compareStringsCaseSensitive,
    compareValues,
    Comparison,
    Debug,
    endsWith,
    equateStringsCaseInsensitive,
    equateStringsCaseSensitive,
    GetCanonicalFileName,
    getDeclarationFileExtension,
    getStringComparer,
    identity,
    lastOrUndefined,
    Path,
    some,
    startsWith,
} from "./_namespaces/ts.js";

/**
 * Internally, we represent paths as strings with '/' as the directory separator.
 * When we make system calls (eg: LanguageServiceHost.getDirectory()),
 * we expect the host to correctly handle paths in our specified format.
 *
 * @internal
 */
export const directorySeparator = "/";
/** @internal */
export const altDirectorySeparator = "\\";
const urlSchemeSeparator = "://";
const backslashRegExp = /\\/g;


backslashRegExp.

//Path Tests

/**
 * Determines whether a charCode corresponds to '/' or '\'.
 *
 * @internal
 */
export function isAnyDirectorySeparator(charCode: number): boolean {
    return charCode === CharacterCodes.slash || charCode === CharacterCodes.backslash;
}"#,
            position: 839,
            expected_kind: Kind::DotToken,
        },
        TestCase { name: "after comma in parameter list", file_content: "takesCb((n, s, ))", position: 15, expected_kind: Kind::CommaToken },
    ];
    for test_case in test_cases {
        let file = parse("/file.ts", test_case.file_content, ScriptKind::TS);
        let token = find_preceding_token(file, test_case.position).unwrap();
        assert_eq!(token.kind(), test_case.expected_kind, "{}", test_case.name);
    }
}
