use tsrs_core::{ScriptKind, TextChange, Tristate};

use super::*;
use crate::astnav::{parse_for_test, repo_root};
use crate::lsutil::{EditorSettings, FormatCodeSettings, IndentStyle};

// api_test.go:19
pub(crate) fn apply_bulk_edits(text: &str, edits: &[TextChange]) -> String {
    let mut b = String::with_capacity(text.len());
    let mut last_end = 0usize;
    for e in edits {
        let start = e.text_range.pos() as usize;
        if start != last_end {
            b.push_str(&text[last_end..start]);
        }
        b.push_str(&e.new_text);

        last_end = e.text_range.end() as usize;
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
    assert!(!new_text.is_empty());
    assert!(text != new_text);
}
