// Port of Go's `diagnosticwriter` package for ast diagnostics (the LSP diagnostic abstraction and
// content-mapper rendering are not ported).

use std::io::{self, Write};

use tsrs_ast::{Diagnostic, SourceFile};
use tsrs_core::tspath::{self, ComparePathsOptions};
use tsrs_core::{text_pos_from_len, TextPos, P};
use tsrs_diagnostics::{self as diagnostics, Category};

pub struct FormattingOptions {
    pub compare_paths_options: ComparePathsOptions,
    pub new_line: String,
}

const FOREGROUND_COLOR_ESCAPE_GREY: &str = "\u{001b}[90m";
const FOREGROUND_COLOR_ESCAPE_RED: &str = "\u{001b}[91m";
const FOREGROUND_COLOR_ESCAPE_YELLOW: &str = "\u{001b}[93m";
const FOREGROUND_COLOR_ESCAPE_BLUE: &str = "\u{001b}[94m";
const FOREGROUND_COLOR_ESCAPE_CYAN: &str = "\u{001b}[96m";

const GUTTER_STYLE_SEQUENCE: &str = "\u{001b}[7m";
const GUTTER_SEPARATOR: &str = " ";
const RESET_ESCAPE_SEQUENCE: &str = "\u{001b}[0m";
const ELLIPSIS: &str = "...";

// Go writes through an io.Writer and ignores write errors; so do we.
macro_rules! w {
    ($out:expr_2021, $($arg:tt)*) => {{
        let _ = write!($out, $($arg)*);
    }};
}

pub fn format_diagnostics_with_color_and_context(output: &mut dyn Write, diags: &[P<Diagnostic>], format_opts: &FormattingOptions) {
    if diags.is_empty() {
        return;
    }
    for (i, &diagnostic) in diags.iter().enumerate() {
        if i > 0 {
            w!(output, "{}", format_opts.new_line);
        }
        format_diagnostic_with_color_and_context(output, diagnostic, format_opts);
    }
}

pub fn format_diagnostic_with_color_and_context(output: &mut dyn Write, diagnostic: P<Diagnostic>, format_opts: &FormattingOptions) {
    if let Some(file) = diagnostic.file() {
        let pos = diagnostic.pos();
        write_location(output, file, pos, Some(format_opts), &write_with_style_and_reset);
        w!(output, " - ");
    }

    write_with_style_and_reset(output, diagnostic.category().name(), get_category_format(diagnostic.category()));
    w!(output, "{} {}{}: {}", FOREGROUND_COLOR_ESCAPE_GREY, diagnostic_prefix(diagnostic), diagnostic.code(), RESET_ESCAPE_SEQUENCE);
    write_flattened_diagnostic_message(output, diagnostic, &format_opts.new_line);

    if let Some(file) = diagnostic.file() {
        if diagnostic.code() != diagnostics::File_appears_to_be_binary.code() {
            w!(output, "{}", format_opts.new_line);
            write_code_snippet(output, file, diagnostic.pos(), diagnostic.len(), get_category_format(diagnostic.category()), "", format_opts);
            w!(output, "{}", format_opts.new_line);
        }
    }

    let related = diagnostic.related_information();
    if !related.is_empty() {
        for &related_information in related {
            if let Some(file) = related_information.file() {
                w!(output, "{}", format_opts.new_line);
                w!(output, "  ");
                let pos = related_information.pos();
                write_location(output, file, pos, Some(format_opts), &write_with_style_and_reset);
                w!(output, " - ");
                write_flattened_diagnostic_message(output, related_information, &format_opts.new_line);
                write_code_snippet(output, file, pos, related_information.len(), FOREGROUND_COLOR_ESCAPE_CYAN, "    ", format_opts);
            }
            w!(output, "{}", format_opts.new_line);
        }
    }
}

fn is_go_space(c: char) -> bool {
    // unicode.IsSpace
    matches!(c, '\t' | '\n' | '\u{000B}' | '\u{000C}' | '\r' | ' ' | '\u{0085}' | '\u{00A0}') || (c as u32 > 0xFF && c.is_whitespace())
}

fn utf16_len(s: &str) -> usize {
    s.chars().map(|c| c.len_utf16()).sum()
}

fn write_code_snippet(
    writer: &mut dyn Write,
    source_file: P<SourceFile>,
    start: TextPos,
    length: u32,
    squiggle_color: &str,
    indent: &str,
    format_opts: &FormattingOptions,
) {
    let (first_line, first_line_char) = tsrs_scanner::get_ecma_line_and_utf16_character_of_position(&*source_file, start);
    let end = start.checked_add(length).expect("diagnostic range end exceeds u32::MAX");
    let (last_line, mut last_line_char) = tsrs_scanner::get_ecma_line_and_utf16_character_of_position(&*source_file, end);
    if length == 0 {
        last_line_char += 1; // When length is zero, squiggle the character right after the start position.
    }

    let text = source_file.text();
    let last_line_of_file = tsrs_scanner::get_ecma_line_of_position(&*source_file, text_pos_from_len(text.len()));

    let has_more_than_five_lines = last_line - first_line >= 4;
    let mut gutter_width = (last_line + 1).to_string().len();
    if has_more_than_five_lines {
        gutter_width = gutter_width.max(ELLIPSIS.len());
    }

    let mut i = first_line;
    while i <= last_line {
        w!(writer, "{}", format_opts.new_line);

        // If the error spans over 5 lines, we'll only show the first 2 and last 2 lines,
        // so we'll skip ahead to the second-to-last line.
        if has_more_than_five_lines && first_line + 1 < i && i < last_line - 1 {
            w!(writer, "{}", indent);
            w!(writer, "{}", GUTTER_STYLE_SEQUENCE);
            w!(writer, "{:>width$}", ELLIPSIS, width = gutter_width);
            w!(writer, "{}", RESET_ESCAPE_SEQUENCE);
            w!(writer, "{}", GUTTER_SEPARATOR);
            w!(writer, "{}", format_opts.new_line);
            i = last_line - 1;
        }

        let line_start = tsrs_scanner::get_ecma_position_of_line_and_byte_offset(&*source_file, i, 0) as usize;
        let line_end = if i < last_line_of_file {
            tsrs_scanner::get_ecma_position_of_line_and_byte_offset(&*source_file, i + 1, 0) as usize
        } else {
            text.len()
        };

        let line_content = text[line_start..line_end].trim_end_matches(is_go_space); // trim from end
        let line_content = line_content.replace('\t', " "); // convert tabs to single spaces

        // Output the gutter and the actual contents of the line.
        w!(writer, "{}", indent);
        w!(writer, "{}", GUTTER_STYLE_SEQUENCE);
        w!(writer, "{:>width$}", i + 1, width = gutter_width);
        w!(writer, "{}", RESET_ESCAPE_SEQUENCE);
        w!(writer, "{}", GUTTER_SEPARATOR);
        w!(writer, "{}", line_content);
        w!(writer, "{}", format_opts.new_line);

        // Output the gutter and the error span for the line using tildes.
        w!(writer, "{}", indent);
        w!(writer, "{}", GUTTER_STYLE_SEQUENCE);
        w!(writer, "{:>width$}", "", width = gutter_width);
        w!(writer, "{}", RESET_ESCAPE_SEQUENCE);
        w!(writer, "{}", GUTTER_SEPARATOR);
        w!(writer, "{}", squiggle_color);
        if i == first_line {
            // If we're on the last line, then limit it to the last character of the last line.
            // Otherwise, we'll just squiggle the rest of the line, giving 'slice' no end position.
            let last_char_for_line = if i == last_line { last_line_char as i64 } else { utf16_len(&line_content) as i64 };

            // Fill with spaces until the first character,
            // then squiggle the remainder of the line.
            w!(writer, "{}", " ".repeat(first_line_char as usize));
            w!(writer, "{}", "~".repeat(go_repeat_count(last_char_for_line - first_line_char as i64)));
        } else if i == last_line {
            // Squiggle until the final character.
            w!(writer, "{}", "~".repeat(go_repeat_count(last_line_char as i64)));
        } else {
            // Squiggle the entire line.
            w!(writer, "{}", "~".repeat(utf16_len(&line_content)));
        }

        w!(writer, "{}", RESET_ESCAPE_SEQUENCE);
        i += 1;
    }
}

// strings.Repeat panics on a negative count; the Go code never passes one for valid diagnostics.
fn go_repeat_count(n: i64) -> usize {
    if n < 0 {
        panic!("strings: negative Repeat count");
    }
    n as usize
}

pub fn flatten_diagnostic_message(d: P<Diagnostic>, new_line: &str) -> String {
    let mut output = Vec::new();
    write_flattened_diagnostic_message(&mut output, d, new_line);
    String::from_utf8(output).unwrap()
}

pub fn write_flattened_diagnostic_message(writer: &mut dyn Write, diagnostic: P<Diagnostic>, newline: &str) {
    w!(writer, "{}", diagnostic.localize());

    for &chain in diagnostic.message_chain() {
        flatten_diagnostic_message_chain(writer, chain, newline, 1 /*level*/);
    }
}

fn flatten_diagnostic_message_chain(writer: &mut dyn Write, chain: P<Diagnostic>, new_line: &str, level: usize) {
    w!(writer, "{}", new_line);
    for _ in 0..level {
        w!(writer, "  ");
    }

    w!(writer, "{}", chain.localize());
    for &child in chain.message_chain() {
        flatten_diagnostic_message_chain(writer, child, new_line, level + 1);
    }
}

// diagnosticPrefix returns the prefix shown before a diagnostic's code, e.g. "TS" for compiler
// diagnostics or an external source's custom prefix.
fn diagnostic_prefix(diagnostic: P<Diagnostic>) -> &'static str {
    let source = diagnostic.source();
    if !source.is_empty() {
        return source;
    }
    "TS"
}

fn get_category_format(category: Category) -> &'static str {
    match category {
        Category::Error => FOREGROUND_COLOR_ESCAPE_RED,
        Category::Warning => FOREGROUND_COLOR_ESCAPE_YELLOW,
        Category::Suggestion => FOREGROUND_COLOR_ESCAPE_GREY,
        Category::Message => FOREGROUND_COLOR_ESCAPE_BLUE,
    }
}

pub type FormattedWriter = dyn Fn(&mut dyn Write, &str, &str);

fn write_with_style_and_reset(output: &mut dyn Write, text: &str, format_style: &str) {
    w!(output, "{}", format_style);
    w!(output, "{}", text);
    w!(output, "{}", RESET_ESCAPE_SEQUENCE);
}

pub fn write_location(
    output: &mut dyn Write,
    file: P<SourceFile>,
    pos: TextPos,
    format_opts: Option<&FormattingOptions>,
    write_with_style_and_reset: &FormattedWriter,
) {
    let (first_line, first_char) = tsrs_scanner::get_ecma_line_and_utf16_character_of_position(&*file, pos);
    let relative_file_name = match format_opts {
        Some(opts) => tspath::convert_to_relative_path(file.file_name(), &opts.compare_paths_options),
        None => file.file_name().to_string(),
    };

    write_with_style_and_reset(output, &relative_file_name, FOREGROUND_COLOR_ESCAPE_CYAN);
    w!(output, ":");
    write_with_style_and_reset(output, &(first_line + 1).to_string(), FOREGROUND_COLOR_ESCAPE_YELLOW);
    w!(output, ":");
    write_with_style_and_reset(output, &(first_char + 1).to_string(), FOREGROUND_COLOR_ESCAPE_YELLOW);
}

// Some of these lived in watch.ts, but they're not specific to the watch API.

struct ErrorSummary {
    total_error_count: usize,
    global_errors: Vec<P<Diagnostic>>,
    errors_by_file: Vec<(P<SourceFile>, Vec<P<Diagnostic>>)>,
}

pub fn write_error_summary_text(output: &mut dyn Write, all_diagnostics: &[P<Diagnostic>], format_opts: &FormattingOptions) {
    // Roughly corresponds to 'getErrorSummaryText' from watch.ts

    let error_summary = get_error_summary(all_diagnostics);
    let total_error_count = error_summary.total_error_count;
    if total_error_count == 0 {
        return;
    }

    let first_file = error_summary.errors_by_file.first();
    let first_file_name = match first_file {
        Some((file, errors)) => pretty_path_for_file_error(Some(*file), errors, format_opts),
        None => String::new(),
    };
    let num_erroring_files = error_summary.errors_by_file.len();

    let message = if total_error_count == 1 {
        // Special-case a single error.
        if !error_summary.global_errors.is_empty() || first_file_name.is_empty() {
            diagnostics::Found_1_error.localize(&[])
        } else {
            diagnostics::Found_1_error_in_0.localize(&[&first_file_name])
        }
    } else {
        match num_erroring_files {
            // No file-specific errors.
            0 => diagnostics::Found_0_errors.localize(&[&total_error_count]),
            // One file with errors.
            1 => diagnostics::Found_0_errors_in_the_same_file_starting_at_Colon_1.localize(&[&total_error_count, &first_file_name]),
            // Multiple files with errors.
            _ => diagnostics::Found_0_errors_in_1_files.localize(&[&total_error_count, &num_erroring_files]),
        }
    };
    w!(output, "{}", format_opts.new_line);
    w!(output, "{}", message);
    w!(output, "{}", format_opts.new_line);
    w!(output, "{}", format_opts.new_line);
    if num_erroring_files > 1 {
        write_tabular_errors_display(output, &error_summary, format_opts);
        w!(output, "{}", format_opts.new_line);
    }
}

fn get_error_summary(diags: &[P<Diagnostic>]) -> ErrorSummary {
    let mut total_error_count = 0;
    let mut global_errors = Vec::new();
    let mut errors_by_file: Vec<(P<SourceFile>, Vec<P<Diagnostic>>)> = Vec::new();
    let mut index_by_file: rustc_hash::FxHashMap<P<SourceFile>, usize> = rustc_hash::FxHashMap::default();

    for &diagnostic in diags {
        if diagnostic.category() != Category::Error {
            continue;
        }

        total_error_count += 1;
        match diagnostic.file() {
            None => global_errors.push(diagnostic),
            Some(file) => {
                let idx = *index_by_file.entry(file).or_insert_with(|| {
                    errors_by_file.push((file, Vec::new()));
                    errors_by_file.len() - 1
                });
                errors_by_file[idx].1.push(diagnostic);
            }
        }
    }

    // !!!
    // Need an ordered map here, but sorting for consistency.
    errors_by_file.sort_by(|a, b| a.0.file_name().cmp(b.0.file_name()));

    ErrorSummary { total_error_count, global_errors, errors_by_file }
}

fn write_tabular_errors_display(output: &mut dyn Write, error_summary: &ErrorSummary, format_opts: &FormattingOptions) {
    let max_errors = error_summary.errors_by_file.iter().map(|(_, e)| e.len()).max().unwrap_or(0);

    // !!!
    // TODO (drosen): This was never localized.
    // Should make this better.
    let header_row = diagnostics::Errors_Files.localize(&[]);
    let left_column_heading_length = header_row.split(' ').next().unwrap_or("").len();
    let length_of_biggest_error_count = max_errors.to_string().len();
    let left_padding_goal = left_column_heading_length.max(length_of_biggest_error_count);
    let header_padding = length_of_biggest_error_count.saturating_sub(left_column_heading_length);

    w!(output, "{}", " ".repeat(header_padding));
    w!(output, "{}", header_row);
    w!(output, "{}", format_opts.new_line);

    for (file, file_errors) in &error_summary.errors_by_file {
        let error_count = file_errors.len();

        w!(output, "{:>width$}  ", error_count, width = left_padding_goal);
        w!(output, "{}", pretty_path_for_file_error(Some(*file), file_errors, format_opts));
        w!(output, "{}", format_opts.new_line);
    }
}

fn pretty_path_for_file_error(file: Option<P<SourceFile>>, file_errors: &[P<Diagnostic>], format_opts: &FormattingOptions) -> String {
    let Some(file) = file else {
        return String::new();
    };
    if file_errors.is_empty() {
        return String::new();
    }
    let line = tsrs_scanner::get_ecma_line_of_position(&*file, file_errors[0].pos());
    let mut file_name = file.file_name().to_string();
    if tspath::path_is_absolute(&file_name) && tspath::path_is_absolute(&format_opts.compare_paths_options.current_directory) {
        file_name = tspath::convert_to_relative_path(file.file_name(), &format_opts.compare_paths_options);
    }
    format!("{}{}:{}{}", file_name, FOREGROUND_COLOR_ESCAPE_GREY, line + 1, RESET_ESCAPE_SEQUENCE)
}

pub fn write_format_diagnostics(output: &mut dyn Write, diagnostics: &[P<Diagnostic>], format_opts: &FormattingOptions) {
    for &diagnostic in diagnostics {
        write_format_diagnostic(output, diagnostic, format_opts);
    }
}

pub fn write_format_diagnostic(output: &mut dyn Write, diagnostic: P<Diagnostic>, format_opts: &FormattingOptions) {
    if let Some(file) = diagnostic.file() {
        let (line, character) = tsrs_scanner::get_ecma_line_and_utf16_character_of_position(&*file, diagnostic.pos());
        let file_name = file.file_name();
        let relative_file_name = tspath::convert_to_relative_path(file_name, &format_opts.compare_paths_options);
        w!(output, "{}({},{}): ", relative_file_name, line + 1, character + 1);
    }

    w!(output, "{} {}{}: ", diagnostic.category().name(), diagnostic_prefix(diagnostic), diagnostic.code());
    write_flattened_diagnostic_message(output, diagnostic, &format_opts.new_line);
    w!(output, "{}", format_opts.new_line);
}

pub fn format_diagnostic_to_string(diagnostic: P<Diagnostic>, format_opts: &FormattingOptions) -> io::Result<String> {
    let mut out = Vec::new();
    write_format_diagnostic(&mut out, diagnostic, format_opts);
    Ok(String::from_utf8_lossy(&out).into_owned())
}

// diagnosticwriter.go:584
pub fn format_diagnostics_status_with_color_and_time(output: &mut dyn Write, time: &str, diag: P<Diagnostic>, format_opts: &FormattingOptions) {
    w!(output, "[");
    write_with_style_and_reset(output, time, FOREGROUND_COLOR_ESCAPE_GREY);
    w!(output, "] ");
    write_flattened_diagnostic_message(output, diag, &format_opts.new_line);
}

// diagnosticwriter.go:591
pub fn format_diagnostics_status_and_time(output: &mut dyn Write, time: &str, diag: P<Diagnostic>, format_opts: &FormattingOptions) {
    w!(output, "{} - ", time);
    write_flattened_diagnostic_message(output, diag, &format_opts.new_line);
}
