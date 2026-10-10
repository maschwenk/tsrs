// Test-only helpers: Go json.Marshal of the option structs (field order, omitzero) and the subset of
// diagnosticwriter the tsoptions baselines use.

use std::fmt::Write;

use tsrs_ast::{Diagnostic, SourceFile};
use tsrs_core::json::{self, Value};
use tsrs_core::tspath::{self, ComparePathsOptions};
use tsrs_core::{BuildOptions, CompilerOptions, TypeAcquisition, P};
use tsrs_diagnostics::Category;

use crate::commandlineoption::CompilerOptionsValue;
use crate::gojson;

// Go json.Marshal of core.CompilerOptions, core.BuildOptions and core.TypeAcquisition: the gojson module's
// encoding, which contentmapper.MarshalDeclaredOptions also uses.
pub(crate) fn compiler_options_to_json(options: &CompilerOptions) -> Value {
    gojson::compiler_options_to_go_json(options)
}

pub(crate) fn build_options_to_json(options: &BuildOptions) -> Value {
    gojson::build_options_to_go_json(options)
}

pub(crate) fn type_acquisition_to_json(options: &TypeAcquisition) -> Value {
    gojson::type_acquisition_to_go_json(options)
}

pub(crate) fn option_value_to_json(value: &CompilerOptionsValue) -> Value {
    json::unmarshal(&crate::tsconfigparsing::stringify_json(value)).unwrap()
}

fn flattened_message(d: P<Diagnostic>, new_line: &str, indent: usize, out: &mut String) {
    out.push_str(&d.localize());
    for chain in d.message_chain() {
        out.push_str(new_line);
        for _ in 0..=indent {
            out.push_str("  ");
        }
        flattened_message(*chain, new_line, indent + 1, out);
    }
}

// diagnosticwriter.WriteFormatDiagnostics
pub(crate) fn write_format_diagnostics(diagnostics: &[P<Diagnostic>], new_line: &str) -> String {
    let mut out = String::new();
    for d in diagnostics {
        if let Some(file) = d.file() {
            let (line, character) = line_and_character(file, d.pos());
            let relative_file_name = tspath::convert_to_relative_path(file.file_name(), &ComparePathsOptions::default());
            let _ = write!(out, "{}({},{}): ", relative_file_name, line + 1, character + 1);
        }
        let _ = write!(out, "{} TS{}: ", d.category().name(), d.code());
        flattened_message(*d, new_line, 0, &mut out);
        out.push_str(new_line);
    }
    out
}

fn line_starts(text: &str) -> Vec<i32> {
    tsrs_core::compute_ecma_line_starts(text)
}

fn line_and_character(file: P<SourceFile>, pos: i32) -> (usize, usize) {
    let starts = line_starts(file.text());
    let line = match starts.binary_search(&pos) {
        Ok(i) => i,
        Err(i) => i - 1,
    };
    let text = &file.text()[starts[line] as usize..pos as usize];
    (line, text.encode_utf16().count())
}

const FOREGROUND_COLOR_ESCAPE_GREY: &str = "\u{1b}[90m";
const FOREGROUND_COLOR_ESCAPE_RED: &str = "\u{1b}[91m";
const FOREGROUND_COLOR_ESCAPE_YELLOW: &str = "\u{1b}[93m";
const FOREGROUND_COLOR_ESCAPE_BLUE: &str = "\u{1b}[94m";
const FOREGROUND_COLOR_ESCAPE_CYAN: &str = "\u{1b}[96m";
const GUTTER_STYLE_SEQUENCE: &str = "\u{1b}[7m";
const GUTTER_SEPARATOR: &str = " ";
const RESET_ESCAPE_SEQUENCE: &str = "\u{1b}[0m";
const ELLIPSIS: &str = "...";

fn with_style(out: &mut String, text: &str, style: &str) {
    out.push_str(style);
    out.push_str(text);
    out.push_str(RESET_ESCAPE_SEQUENCE);
}

fn category_color(category: Category) -> &'static str {
    match category {
        Category::Error => FOREGROUND_COLOR_ESCAPE_RED,
        Category::Warning => FOREGROUND_COLOR_ESCAPE_YELLOW,
        Category::Suggestion => FOREGROUND_COLOR_ESCAPE_GREY,
        Category::Message => FOREGROUND_COLOR_ESCAPE_BLUE,
    }
}

fn write_location(out: &mut String, file: P<SourceFile>, pos: i32, options: &ComparePathsOptions) {
    let (line, character) = line_and_character(file, pos);
    let relative_file_name = tspath::convert_to_relative_path(file.file_name(), options);
    with_style(out, &relative_file_name, FOREGROUND_COLOR_ESCAPE_CYAN);
    out.push(':');
    with_style(out, &(line + 1).to_string(), FOREGROUND_COLOR_ESCAPE_YELLOW);
    out.push(':');
    with_style(out, &(character + 1).to_string(), FOREGROUND_COLOR_ESCAPE_YELLOW);
}

fn write_code_snippet(out: &mut String, file: P<SourceFile>, start: i32, length: i32, squiggle_color: &str, indent: &str, new_line: &str) {
    let starts = line_starts(file.text());
    let text = file.text();
    let line_of = |pos: i32| match starts.binary_search(&pos) {
        Ok(i) => i,
        Err(i) => i - 1,
    };
    let first_line = line_of(start);
    let first_line_char = text[starts[first_line] as usize..start as usize].encode_utf16().count();
    let end = start + length;
    let last_line = line_of(end);
    let mut last_line_char = text[starts[last_line] as usize..end as usize].encode_utf16().count();
    if length == 0 {
        last_line_char += 1; // When length is zero, squiggle the character right after the start position.
    }
    let last_line_of_file = line_of(text.len() as i32);
    let has_more_than_five_lines = last_line - first_line >= 4;
    let mut gutter_width = (last_line + 1).to_string().len();
    if has_more_than_five_lines {
        gutter_width = gutter_width.max(ELLIPSIS.len());
    }

    let mut i = first_line;
    while i <= last_line {
        out.push_str(new_line);
        // If the error spans over 5 lines, we'll only show the first 2 and last 2 lines,
        // so we'll skip ahead to the second-to-last line.
        if has_more_than_five_lines && first_line + 1 < i && i < last_line - 1 {
            out.push_str(indent);
            with_style(out, &format!("{:>width$}", ELLIPSIS, width = gutter_width), GUTTER_STYLE_SEQUENCE);
            out.push_str(GUTTER_SEPARATOR);
            out.push_str(new_line);
            i = last_line - 1;
        }

        let line_start = starts[i] as usize;
        let line_end = if i < last_line_of_file { starts[i + 1] as usize } else { text.len() };
        let line_content = text[line_start..line_end].trim_end_matches(|c: char| c.is_whitespace());
        let line_content = line_content.replace('\t', " ");

        // Output the gutter and the actual contents of the line.
        out.push_str(indent);
        with_style(out, &format!("{:>width$}", i + 1, width = gutter_width), GUTTER_STYLE_SEQUENCE);
        out.push_str(GUTTER_SEPARATOR);
        out.push_str(&line_content);
        out.push_str(new_line);

        // Output the gutter and the error span for the line using tildes.
        out.push_str(indent);
        with_style(out, &format!("{:>width$}", "", width = gutter_width), GUTTER_STYLE_SEQUENCE);
        out.push_str(GUTTER_SEPARATOR);
        out.push_str(squiggle_color);
        let content_len = line_content.encode_utf16().count();
        if i == first_line {
            // If we're on the last line, then limit it to the last character of the last line.
            // Otherwise, we'll just squiggle the rest of the line, giving 'slice' no end position.
            let last_char_for_line = if i == last_line { last_line_char } else { content_len };
            // Fill with spaces until the first character,
            // then squiggle the remainder of the line.
            out.push_str(&" ".repeat(first_line_char));
            out.push_str(&"~".repeat(last_char_for_line.saturating_sub(first_line_char)));
        } else if i == last_line {
            // Squiggle until the final character.
            out.push_str(&"~".repeat(last_line_char));
        } else {
            // Squiggle the entire line.
            out.push_str(&"~".repeat(content_len));
        }
        out.push_str(RESET_ESCAPE_SEQUENCE);
        i += 1;
    }
}

// diagnosticwriter.FormatDiagnosticsWithColorAndContext (no related information in these baselines).
pub(crate) fn format_diagnostics_with_color_and_context(
    diagnostics: &[P<Diagnostic>],
    new_line: &str,
    options: &ComparePathsOptions,
) -> String {
    let mut out = String::new();
    if diagnostics.is_empty() {
        return out;
    }
    for (i, diagnostic) in diagnostics.iter().enumerate() {
        if i > 0 {
            out.push_str(new_line);
        }
        if let Some(file) = diagnostic.file() {
            write_location(&mut out, file, diagnostic.pos(), options);
            out.push_str(" - ");
        }
        with_style(&mut out, diagnostic.category().name(), category_color(diagnostic.category()));
        let _ = write!(out, "{} TS{}: {}", FOREGROUND_COLOR_ESCAPE_GREY, diagnostic.code(), RESET_ESCAPE_SEQUENCE);
        flattened_message(*diagnostic, new_line, 0, &mut out);

        if let Some(file) = diagnostic.file() {
            if diagnostic.code() != tsrs_diagnostics::File_appears_to_be_binary.code() {
                out.push_str(new_line);
                write_code_snippet(
                    &mut out,
                    file,
                    diagnostic.pos(),
                    diagnostic.end() - diagnostic.pos(),
                    category_color(diagnostic.category()),
                    "",
                    new_line,
                );
                out.push_str(new_line);
            }
        }
    }
    out
}

#[cfg(test)]
mod line_start_tests {
    use super::line_starts;

    #[test]
    fn unicode_line_starts_are_byte_offsets() {
        assert_eq!(line_starts(""), [0]);
        assert_eq!(line_starts("é😀\u{2027}\u{202A}"), [0]);
        assert_eq!(line_starts("é\r\n😀\u{2028}\u{2029}x\r"), [0, 4, 11, 14, 16]);
        assert_eq!(line_starts("\n\r\r\n"), [0, 1, 2, 4]);
    }
}
