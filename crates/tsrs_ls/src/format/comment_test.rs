use tsrs_core::{ScriptKind, Tristate};

use super::api_test::{apply_bulk_edits, settings};
use super::*;
use crate::astnav::parse_for_test;

fn type_annotation(s: &mut crate::lsutil::FormatCodeSettings) {
    s.insert_space_before_type_annotation = Tristate::True;
}

// comment_test.go:15
#[test]
fn test_comment_formatting_issue_reproduction() {
    let ctx = with_format_code_settings(&FormatContext::default(), settings(4, 4, 4, Tristate::True, type_annotation), "\n");

    // Original code that causes the bug
    let original_text = "class C {\n    /**\n     *\n    */\n    async x() {}\n}";

    let source_file = parse_for_test("/test.ts", original_text, ScriptKind::TS);

    // Apply formatting once
    let edits = format_document(&ctx, source_file);
    let first_formatted = apply_bulk_edits(original_text, &edits);

    // Check that the asterisk is not corrupted
    assert!(!first_formatted.contains("*/\n   /"), "should not corrupt */ to /");
    assert!(first_formatted.contains("*/"), "should preserve */ token");
    assert!(first_formatted.contains("async"), "should preserve async keyword");

    // Apply formatting a second time to test stability
    let source_file2 = parse_for_test("/test.ts", &first_formatted, ScriptKind::TS);

    let edits2 = format_document(&ctx, source_file2);
    let second_formatted = apply_bulk_edits(&first_formatted, &edits2);

    // Check that second formatting doesn't introduce corruption
    assert!(!second_formatted.contains(" sync x()"), "should not corrupt async to sync");
    assert!(second_formatted.contains("async"), "should preserve async keyword on second pass");
}

#[test]
fn test_comment_formatting_jsdoc_with_tab_indentation() {
    let ctx = with_format_code_settings(&FormatContext::default(), settings(4, 4, 0, Tristate::False, type_annotation), "\n");

    // Original code with tab indentation (tabs represented as \t)
    let original_text =
        "class Foo {\n\t/**\n\t * @param {string} argument - This is a param description.\n\t */\n\texample(argument) {\nconsole.log(argument);\n\t}\n}";

    let source_file = parse_for_test("/test.ts", original_text, ScriptKind::TS);

    // Apply formatting
    let edits = format_document(&ctx, source_file);
    let formatted = apply_bulk_edits(original_text, &edits);

    // Check that tabs come before spaces (not spaces before tabs)
    // The comment lines should have format: tab followed by space and asterisk
    // NOT: space followed by tab and asterisk
    assert!(!formatted.contains(" \t*"), "should not have space before tab before asterisk");
    assert!(formatted.contains("\t *"), "should have tab before space before asterisk");

    // Verify console.log is properly indented with tabs
    assert!(formatted.contains("\t\tconsole.log"), "console.log should be indented with two tabs");
}

#[test]
fn test_comment_formatting_inside_multi_line_argument_list() {
    let ctx = with_format_code_settings(&FormatContext::default(), settings(4, 4, 0, Tristate::False, type_annotation), "\n");

    // Original code with proper indentation
    let original_text = "console.log(\n\t\"a\",\n\t// the second arg\n\t\"b\"\n);";

    let source_file = parse_for_test("/test.ts", original_text, ScriptKind::TS);

    // Apply formatting
    let edits = format_document(&ctx, source_file);
    let formatted = apply_bulk_edits(original_text, &edits);

    // The comment should remain indented with a tab
    assert!(formatted.contains("\t// the second arg"), "comment should be indented with tab");
    // The comment should not lose its indentation
    assert!(!formatted.contains("\n// the second arg"), "comment should not lose indentation");
}

#[test]
fn test_comment_formatting_in_chained_method_calls() {
    let ctx = with_format_code_settings(&FormatContext::default(), settings(4, 4, 0, Tristate::False, type_annotation), "\n");

    // Original code with proper indentation
    let original_text = "foo\n\t.bar()\n\t// A second call\n\t.baz();";

    let source_file = parse_for_test("/test.ts", original_text, ScriptKind::TS);

    // Apply formatting
    let edits = format_document(&ctx, source_file);
    let formatted = apply_bulk_edits(original_text, &edits);

    // The comment should remain indented
    assert!(formatted.contains("\t// A second call") || formatted.contains("   // A second call"), "comment should be indented");
    // The comment should not lose its indentation
    assert!(!formatted.contains("\n// A second call"), "comment should not lose indentation");
}

// Regression test for issue #1928 - panic when formatting chained method call with comment
#[test]
fn test_comment_formatting_chained_method_call_with_comment_issue_1928() {
    let ctx = with_format_code_settings(&FormatContext::default(), settings(4, 4, 0, Tristate::False, type_annotation), "\n");

    // This code previously caused a panic with "strings: negative Repeat count"
    // because tokenIndentation was -1 and was being used directly for indentation
    let original_text = "foo\n\t.bar()\n\t// A second call\n\t.baz();";

    let source_file = parse_for_test("/test.ts", original_text, ScriptKind::TS);

    // Apply formatting - should not panic
    let edits = format_document(&ctx, source_file);
    let formatted = apply_bulk_edits(original_text, &edits);

    // Verify the comment maintains proper indentation and doesn't lose it
    assert!(formatted.contains("\t// A second call") || formatted.contains("   // A second call"), "comment should be indented");
    assert!(!formatted.contains("\n// A second call"), "comment should not be at column 0");
}

#[test]
fn test_comment_formatting_multiline_comment_inside_block_issue_2649() {
    let ctx = with_format_code_settings(&FormatContext::default(), settings(4, 4, 0, Tristate::False, |_| {}), "\n");

    let original_text = "document.addEventListener('DOMContentLoaded', () => {\n    /** @type {NodeListOf<HTMLSpanElement>} */\n    const elements = document.querySelectorAll('.test')\n});";

    let source_file = parse_for_test("/test.js", original_text, ScriptKind::JS);

    let edits = format_document(&ctx, source_file);
    let formatted = apply_bulk_edits(original_text, &edits);
    assert!(!formatted.is_empty(), "formatted text should not be empty");
}

#[test]
fn test_comment_formatting_single_line_comment_inside_block_issue_2649() {
    let ctx = with_format_code_settings(&FormatContext::default(), settings(4, 4, 0, Tristate::False, |_| {}), "\n");

    let original_text = "document.addEventListener('DOMContentLoaded', () => {\n    // a comment\n    const x = 1\n});";

    let source_file = parse_for_test("/test.ts", original_text, ScriptKind::TS);

    let edits = format_document(&ctx, source_file);
    let formatted = apply_bulk_edits(original_text, &edits);
    assert!(!formatted.is_empty(), "formatted text should not be empty");
}

// comment_test.go:251
#[test]
fn test_format_selection_preserves_comments_selection_ends_inside_comment() {
    let ctx = with_format_code_settings(&FormatContext::default(), settings(4, 4, 0, Tristate::True, |_| {}), "\n");

    // Reproduce: const test/* comment */=5;
    // When selecting a range that ends inside the comment (before */), format selection should not delete the comment.
    let original_text = "const test/* comment */=5;";

    let source_file = parse_for_test("/test.ts", original_text, ScriptKind::TS);

    // Select a range that starts at the beginning of the line and ends inside the block comment.
    // This covers `const test/* comment`, stopping before the closing `*/`.
    let comment_start = original_text.find("/*").unwrap();
    let selection_end = comment_start + "/* comment".len(); // ends inside the comment, before the closing `*/`

    let edits = format_selection(&ctx, source_file, 0, selection_end as i32);
    let formatted = apply_bulk_edits(original_text, &edits);

    // The entire statement should be preserved unchanged
    assert_eq!(formatted, original_text, "format selection should not delete the block comment or alter the statement");
}

#[test]
fn test_format_selection_preserves_comments_selection_starts_inside_comment() {
    let ctx = with_format_code_settings(&FormatContext::default(), settings(4, 4, 0, Tristate::True, |_| {}), "\n");

    let original_text = "const test/* comment */=5;";

    let source_file = parse_for_test("/test.ts", original_text, ScriptKind::TS);

    // Select from inside the comment to the end
    let comment_start = original_text.find("/*").unwrap();
    let selection_start = comment_start + 3; // inside the comment

    let edits = format_selection(&ctx, source_file, selection_start as i32, original_text.len() as i32);
    let formatted = apply_bulk_edits(original_text, &edits);

    // The entire statement should be preserved unchanged
    assert_eq!(formatted, original_text, "format selection should not delete the block comment or alter the statement");
}

#[test]
fn test_format_selection_preserves_comments_full_document() {
    let ctx = with_format_code_settings(
        &FormatContext::default(),
        settings(4, 4, 0, Tristate::True, |s| s.insert_space_before_and_after_binary_operators = Tristate::True),
        "\n",
    );

    let original_text = "const test/* comment */=5;";

    let source_file = parse_for_test("/test.ts", original_text, ScriptKind::TS);

    let edits = format_document(&ctx, source_file);
    let formatted = apply_bulk_edits(original_text, &edits);

    // Full document format should preserve the comment and add spaces around `=`
    assert_eq!("const test/* comment */ = 5;", formatted, "full format should preserve the block comment and add spaces");
}

// comment_test.go:345
#[test]
fn test_slice_bounds_panic() {
    let ctx = with_format_code_settings(&FormatContext::default(), settings(4, 4, 4, Tristate::True, type_annotation), "\n");

    // Code from the issue that causes slice bounds panic
    let original_text = "const _enableDisposeWithListenerWarning = false\n\t// || Boolean(\"TRUE\") // causes a linter warning so that it cannot be pushed\n\t;\n";

    let source_file = parse_for_test("/test.ts", original_text, ScriptKind::TS);

    // This should not panic
    let edits = format_document(&ctx, source_file);
    let formatted = apply_bulk_edits(original_text, &edits);

    // Basic sanity checks
    assert!(!formatted.is_empty(), "formatted text should not be empty");
    assert!(formatted.contains("_enableDisposeWithListenerWarning"), "should preserve variable name");
}
