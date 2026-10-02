use tsrs_core::ScriptKind;

use super::*;
use crate::astnav::parse_for_test;
use crate::lsutil;

// indent_getindentation_test.go:12
#[test]
fn test_get_indentation_for_named_imports_position() {
    let text = "import {\n    type SomeInterface,\n} from \"./exports.js\";";
    // Position 9: \n
    // Position 10: first space of "    type SomeInterface"

    let source_file = parse_for_test("/test.ts", text, ScriptKind::TS);

    let options = lsutil::get_default_format_code_settings();

    // The line that contains "    type SomeInterface" starts at position 9 (the \n).
    // The getAdjustedStartPosition with LeadingTriviaOptionNone returns line start.
    // Let's test at position 9 (start of line containing the specifier)
    let line_start = get_line_start_position_for_position(14, source_file); // 14 is somewhere in "    type"

    let indent = get_indentation(line_start, source_file, &options, true);

    assert_eq!(indent, 4, "Expected indentation 4, got {indent} (lineStart={line_start})");
}
