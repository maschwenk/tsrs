use tsrs_core::{ScriptKind, TextChange, Tristate};

use super::*;
use crate::astnav::{parse_for_test, repo_root};
use crate::lsutil::{EditorSettings, FormatCodeSettings, IndentStyle};

// api_test.go:19
pub(crate) fn apply_bulk_edits(text: &str, edits: &[TextChange]) -> String {
    let mut b = String::with_capacity(text.len());
    let mut last_end = 0usize;
    for e in edits {
        let start = e
            .text_range
            .pos()
            .expect_source("format edits have source positions") as usize;
        if start != last_end {
            b.push_str(&text[last_end..start]);
        }
        b.push_str(&e.new_text);

        last_end = e
            .text_range
            .end()
            .expect_source("format edits have source positions") as usize;
    }
    b.push_str(&text[last_end..]);

    b
}

pub(crate) fn settings(
    tab_size: i32,
    indent_size: i32,
    base_indent_size: i32,
    convert_tabs_to_spaces: Tristate,
    customize: impl FnOnce(&mut FormatCodeSettings),
) -> FormatCodeSettings {
    let mut s = FormatCodeSettings {
        editor_settings: EditorSettings {
            tab_size,
            indent_size,
            base_indent_size,
            new_line_character: "\n".to_string(),
            convert_tabs_to_spaces,
            indent_style: IndentStyle::Smart,
            trim_trailing_whitespace: Tristate::True,
        },
        ..Default::default()
    };
    customize(&mut s);
    s
}

// api_test.go:37
#[test]
fn test_format_checker_ts() {
    let ctx = with_format_code_settings(
        &FormatContext::default(),
        settings(4, 4, 4, Tristate::True, |s| s.insert_space_before_type_annotation = Tristate::True),
        "\n",
    );
    let file_path = repo_root().join("ts-ref/tsc/testdata/fixtures/compiler/checker.ts");
    let text = std::fs::read_to_string(file_path).unwrap();
    let source_file = parse_for_test("/checker.ts", &text, ScriptKind::TS);
    let edits = format_document(&ctx, source_file);
    let new_text = apply_bulk_edits(&text, &edits);
    assert_ne!(new_text, "");
    assert_ne!(text, new_text);
}

// tsrs-only: a huge tab size (a client's `tabSize: 1e9`) indents by MAX_INDENT_SIZE columns per level instead of
// building gigabyte indentation strings.
#[test]
fn huge_indent_sizes_are_clamped() {
    let ctx = with_format_code_settings(&FormatContext::default(), settings(1_000_000_000, 1_000_000_000, 0, Tristate::True, |_| {}), "\n");
    let text = "function f() {\nif(a) {\nb();\n}\n}\n";
    let source_file = parse_for_test("/a.ts", text, ScriptKind::TS);
    let new_text = apply_bulk_edits(text, &format_document(&ctx, source_file));
    let pad = |level: usize| " ".repeat(level * MAX_INDENT_SIZE as usize);
    assert_eq!(new_text, format!("function f() {{\n{}if(a) {{\n{}b();\n{}}}\n}}\n", pad(1), pad(2), pad(1)));
}

// tsrs-only: a selection from a comment to inside the first token after more comments trims each line once. Go (and
// TypeScript) trimmed the lines after the first comment twice, sending the same delete edits twice.
#[test]
fn selection_trims_trailing_whitespace_once() {
    let ctx = with_format_code_settings(&FormatContext::default(), settings(4, 4, 0, Tristate::True, |_| {}), "\n");
    let text = "/* a\n*/  \n\n \t\n// b\ndeclare module \"x\" { }\n";
    let source_file = parse_for_test("/a.d.ts", text, ScriptKind::TS);
    let start = text.find("*/").unwrap() as u32;
    let end = text.find("declare").unwrap() as u32 + 4;
    let edits = format_selection(&ctx, source_file, start, end);
    let ranges: Vec<(u32, u32)> = edits.iter().map(|e| (e.pos(), e.end())).collect();
    let after_comment = text.find("*/").unwrap() as u32 + 2;
    let blank = text.find(" \t").unwrap() as u32;
    assert_eq!(ranges, vec![(after_comment, after_comment + 2), (blank, blank + 2)]);
    assert_eq!(apply_bulk_edits(text, &edits), "/* a\n*/\n\n\n// b\ndeclare module \"x\" { }\n");
}
