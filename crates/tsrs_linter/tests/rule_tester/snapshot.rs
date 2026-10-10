// Adapted from tsgolint's internal/rule_tester/snapshot.go (MIT).
use std::fmt::Write;

use tsrs_ast::SourceFile;
use tsrs_core::{P, TextRange};
use tsrs_linter::RuleDiagnostic;
use tsrs_scanner::get_ecma_line_and_utf16_character_of_position;

fn trim_start(code: &str, range: TextRange) -> i32 {
    let mut start = range.pos();
    while start < range.end() && (start as usize) < code.len() {
        if matches!(
            code.as_bytes()[start as usize],
            b' ' | b'\t' | b'\n' | b'\r'
        ) {
            start += 1;
        } else {
            break;
        }
    }
    start
}

fn adjust_for_tabs(line: &str, column: usize) -> usize {
    line.as_bytes()
        .iter()
        .take(column)
        .map(|&ch| if ch == b'\t' { 4 } else { 1 })
        .sum()
}

fn render_source_annotation(
    code: &str,
    file: P<SourceFile>,
    range: TextRange,
    marker: char,
    label: &str,
) -> String {
    let lines: Vec<_> = code.split('\n').collect();
    let (sl, sc) =
        get_ecma_line_and_utf16_character_of_position(file.get(), trim_start(code, range));
    let (el, ec) = get_ecma_line_and_utf16_character_of_position(file.get(), range.end());
    let start_line = (sl - 1).max(0) as usize;
    let end_line = ((el + 1) as usize).min(lines.len() - 1);
    let width = (end_line + 1).to_string().len().max(2);
    let mut output = String::new();
    for (index, &text) in lines.iter().enumerate().take(end_line + 1).skip(start_line) {
        writeln!(
            output,
            "  {:>width$} | {}",
            index + 1,
            text.replace('\t', "    ")
        )
        .unwrap();
        if (index as i32) < sl || (index as i32) > el {
            continue;
        }
        let start = if index as i32 == sl { sc as usize } else { 0 };
        let end = if index as i32 == el {
            ec as usize
        } else {
            text.len()
        };
        let start = adjust_for_tabs(text, start);
        let end = adjust_for_tabs(text, end);
        if end <= start {
            continue;
        }
        write!(
            output,
            "  {:>width$} | {}{}",
            "",
            " ".repeat(start),
            marker.to_string().repeat(end - start)
        )
        .unwrap();
        if !label.is_empty() {
            write!(output, " {label}").unwrap();
        }
        output.push('\n');
    }
    output
}

fn display_range(code: &str, file: P<SourceFile>, range: TextRange) -> (i32, i32, i32, i32) {
    let start = trim_start(code, range);
    let end = if range.end() > start {
        range.end() - 1
    } else {
        range.end()
    };
    let (line, column) = get_ecma_line_and_utf16_character_of_position(file.get(), start);
    let (end_line, end_column) = get_ecma_line_and_utf16_character_of_position(file.get(), end);
    (line + 1, column + 1, end_line + 1, end_column + 1)
}

fn format_diagnostics_snapshot(code: &str, diagnostics: &[RuleDiagnostic]) -> String {
    if diagnostics.is_empty() {
        return "No diagnostics".to_string();
    }
    let mut output = String::new();
    for (index, diagnostic) in diagnostics.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        let range = diagnostic.range;
        let has_range = range.pos() != range.end() || (range.pos() != 0 && range.pos() != -1);
        if has_range {
            let (line, column, end_line, end_column) =
                display_range(code, diagnostic.source_file, range);
            writeln!(
                output,
                "Diagnostic {}: {} ({line}:{column} - {end_line}:{end_column})",
                index + 1,
                diagnostic.message.id
            )
            .unwrap();
            writeln!(output, "Message: {}", diagnostic.message.description).unwrap();
            if let Some(help) = diagnostic
                .message
                .help
                .as_deref()
                .filter(|help| !help.is_empty())
            {
                writeln!(output, "Help: {help}").unwrap();
            }
            output.push_str(&render_source_annotation(
                code,
                diagnostic.source_file,
                range,
                '~',
                "",
            ));
        } else {
            writeln!(
                output,
                "Diagnostic {}: {}",
                index + 1,
                diagnostic.message.id
            )
            .unwrap();
            writeln!(output, "Message: {}", diagnostic.message.description).unwrap();
        }
        for label in &diagnostic.labeled_ranges {
            let (line, column, end_line, end_column) =
                display_range(code, diagnostic.source_file, label.range);
            writeln!(
                output,
                "  Label: {} ({line}:{column} - {end_line}:{end_column})",
                label.label
            )
            .unwrap();
            output.push_str(&render_source_annotation(
                code,
                diagnostic.source_file,
                label.range,
                '^',
                &label.label,
            ));
        }
        for (index, suggestion) in diagnostic.suggestions.iter().enumerate() {
            writeln!(
                output,
                "  Suggestion {}: [{}] {}",
                index + 1,
                suggestion.message.id,
                suggestion.message.description
            )
            .unwrap();
        }
    }
    output.trim_end_matches('\n').to_string()
}

pub fn match_snapshot(name: &str, code: &str, diagnostics: &[RuleDiagnostic], snapshots: &str) {
    let key = format!("[{name} - 1]");
    let expected = snapshots
        .split("---\n")
        .find_map(|block| {
            let (header, body) = block.trim_start_matches('\n').split_once('\n')?;
            (header == key).then(|| body.trim_end_matches('\n'))
        })
        .unwrap_or_else(|| panic!("missing upstream snapshot for {name}"));
    let actual = format_diagnostics_snapshot(code, diagnostics);
    assert_eq!(actual, expected, "{name}: upstream diagnostic snapshot");
}
