use tsrs_core::{ScriptKind, Tristate};

use super::api_test::{apply_bulk_edits, settings};
use super::*;
use crate::astnav::parse_for_test;

// format_test.go:15
#[test]
fn test_format_no_trailing_space() {
    let test_cases = [
        ("simple statement without trailing newline", "1;"),
        ("function call without trailing newline", "console.log('hello');"),
        ("if block on single line", "if (true) { }"),
        ("class declaration", "class A {\n    // Class Contents Go Here\n}"),
        ("class declaration with trailing newline", "class A {\n    // Class Contents Go Here\n}\n"),
        ("empty block", "if (true) {}"),
        ("module declaration", "module M { }"),
        ("enum declaration", "enum E { A, B }"),
    ];

    for (name, text) in test_cases {
        let ctx = with_format_code_settings(&FormatContext::default(), settings(4, 4, 0, Tristate::True, |_| {}), "\n");
        let source_file = parse_for_test("/test.ts", text, ScriptKind::TS);
        let edits = format_document(&ctx, source_file);
        let new_text = apply_bulk_edits(text, &edits);
        // Formatting should not add trailing whitespace at end of file
        for (i, line) in new_text.split('\n').enumerate() {
            let trimmed = line.trim_end_matches([' ', '\t']);
            assert_eq!(line, trimmed, "{name}: Formatter should not add trailing whitespace on line {}", i + 1);
        }
    }
}
