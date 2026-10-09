// The subset of internal/diagnosticwriter that error baselines use, over an owned diagnostic snapshot
// (`Diag`) so that baseline rendering does not depend on the lifetime or API of compiler objects.

use std::fmt::Write as _;
use std::cmp::Ordering;
use std::rc::Rc;

use tsrs_core::tspath::{self, ComparePathsOptions};
use tsrs_core::{position_cmp, TextPos};
use tsrs_diagnostics::{self as diagnostics, Category};

pub struct FileLike {
    pub file_name: String,
    pub text: String,
    pub line_map: Vec<TextPos>,
}

impl FileLike {
    pub fn new(file_name: String, text: String) -> Rc<FileLike> {
        let line_map = compute_ecma_line_starts(&text);
        Rc::new(FileLike { file_name, text, line_map })
    }
}

#[derive(Clone)]
pub struct Diag {
    pub file: Option<Rc<FileLike>>,
    pub pos: TextPos,
    pub end: TextPos,
    pub code: i32,
    pub category: Category,
    // Custom prefix shown before the code instead of "TS" (empty means "TS").
    pub source: String,
    // Localized message text (Diagnostic.Localize(locale.Default)).
    pub message: String,
    // getDiagnosticMessageIdentity
    pub identity: String,
    pub args: Vec<String>,
    pub chain: Vec<Diag>,
    pub related: Vec<Diag>,
}

impl Diag {
    pub fn len(&self) -> u32 {
        self.end - self.pos
    }
}

fn diagnostic_path(d: &Diag) -> &str {
    d.file.as_ref().map_or("", |f| f.file_name.as_str())
}

fn compare_str(a: &str, b: &str) -> i32 {
    match a.cmp(b) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

fn compare_string_slices(a: &[String], b: &[String]) -> i32 {
    for (x, y) in a.iter().zip(b.iter()) {
        let c = compare_str(x, y);
        if c != 0 {
            return c;
        }
    }
    (a.len() as i32 - b.len() as i32).signum()
}

fn compare_message_chain_size(c1: &[Diag], c2: &[Diag]) -> i32 {
    let c = c2.len() as i32 - c1.len() as i32;
    if c != 0 {
        return c;
    }
    for i in 0..c1.len() {
        let c = compare_message_chain_size(&c1[i].chain, &c2[i].chain);
        if c != 0 {
            return c;
        }
    }
    0
}

fn compare_message_chain_content(c1: &[Diag], c2: &[Diag]) -> i32 {
    for i in 0..c1.len() {
        let c = compare_string_slices(&c1[i].args, &c2[i].args);
        if c != 0 {
            return c;
        }
        if !c1[i].chain.is_empty() {
            let c = compare_message_chain_content(&c1[i].chain, &c2[i].chain);
            if c != 0 {
                return c;
            }
        }
    }
    0
}

fn compare_related_info(r1: &[Diag], r2: &[Diag]) -> i32 {
    let c = r2.len() as i32 - r1.len() as i32;
    if c != 0 {
        return c;
    }
    for i in 0..r1.len() {
        let c = compare_diagnostics(&r1[i], &r2[i]);
        if c != 0 {
            return c;
        }
    }
    0
}

// ast.CompareDiagnostics
pub fn compare_diagnostics(d1: &Diag, d2: &Diag) -> i32 {
    let mut c = compare_str(diagnostic_path(d1), diagnostic_path(d2));
    if c != 0 {
        return c;
    }
    c = match position_cmp(d1.pos, d2.pos) { Ordering::Less => -1, Ordering::Equal => 0, Ordering::Greater => 1 };
    if c != 0 { return c; }
    c = match position_cmp(d1.end, d2.end) { Ordering::Less => -1, Ordering::Equal => 0, Ordering::Greater => 1 };
    if c != 0 { return c; }
    c = d1.code - d2.code;
    if c != 0 {
        return c;
    }
    c = d1.category as i32 - d2.category as i32;
    if c != 0 {
        return c;
    }
    c = compare_str(&d1.source, &d2.source);
    if c != 0 {
        return c;
    }
    c = compare_str(&d1.identity, &d2.identity);
    if c != 0 {
        return c;
    }
    c = compare_string_slices(&d1.args, &d2.args);
    if c != 0 {
        return c;
    }
    c = compare_message_chain_size(&d1.chain, &d2.chain);
    if c != 0 {
        return c;
    }
    c = compare_message_chain_content(&d1.chain, &d2.chain);
    if c != 0 {
        return c;
    }
    compare_related_info(&d1.related, &d2.related)
}

pub fn compute_ecma_line_starts(text: &str) -> Vec<TextPos> {
    let bytes = text.as_bytes();
    let mut result = Vec::with_capacity(bytes.iter().filter(|&&b| b == b'\n').count() + 1);
    let mut pos = 0usize;
    let mut line_start = 0usize;
    while pos < bytes.len() {
        let b = bytes[pos];
        if b < 0x80 {
            pos += 1;
            match b {
                b'\r' => {
                    if pos < bytes.len() && bytes[pos] == b'\n' {
                        pos += 1;
                    }
                    result.push(line_start as TextPos);
                    line_start = pos;
                }
                b'\n' => {
                    result.push(line_start as TextPos);
                    line_start = pos;
                }
                _ => {}
            }
        } else {
            let ch = text[pos..].chars().next().unwrap();
            pos += ch.len_utf8();
            if ch == '\u{2028}' || ch == '\u{2029}' {
                result.push(line_start as TextPos);
                line_start = pos;
            }
        }
    }
    result.push(line_start as TextPos);
    result
}

pub fn utf16_len(s: &str) -> i32 {
    s.chars().map(|c| c.len_utf16() as i32).sum()
}

// scanner.ComputeLineOfPosition
pub fn compute_line_of_position(line_starts: &[TextPos], pos: TextPos) -> usize {
    match line_starts.binary_search(&pos) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    }
}

// Go slices strings by byte offset without regard to char boundaries; clamp to a boundary so a bad
// position renders instead of panicking inside the harness.
fn byte_slice(text: &str, start: usize, end: usize) -> &str {
    let mut s = start.min(text.len());
    while !text.is_char_boundary(s) {
        s -= 1;
    }
    let mut e = end.min(text.len()).max(s);
    while !text.is_char_boundary(e) {
        e += 1;
    }
    &text[s..e]
}

pub fn get_ecma_line_and_utf16_character_of_position(file: &FileLike, pos: TextPos) -> (usize, i32) {
    let line = compute_line_of_position(&file.line_map, pos);
    let start = file.line_map[line] as usize;
    let character = utf16_len(byte_slice(&file.text, start, pos as usize));
    (line, character)
}

pub fn get_ecma_line_of_position(file: &FileLike, pos: TextPos) -> usize {
    compute_line_of_position(&file.line_map, pos)
}

pub struct FormattingOptions {
    pub compare_paths_options: ComparePathsOptions,
    pub new_line: &'static str,
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

fn file_appears_to_be_binary_code() -> i32 {
    diagnostics::File_appears_to_be_binary.code()
}

pub fn format_diagnostics_with_color_and_context(output: &mut String, diags: &[&Diag], format_opts: &FormattingOptions) {
    for (i, diagnostic) in diags.iter().enumerate() {
        if i > 0 {
            output.push_str(format_opts.new_line);
        }
        format_diagnostic_with_color_and_context(output, diagnostic, format_opts);
    }
}

pub fn format_diagnostic_with_color_and_context(output: &mut String, diagnostic: &Diag, format_opts: &FormattingOptions) {
    if let Some(file) = &diagnostic.file {
        write_location(output, file, diagnostic.pos, Some(format_opts), write_with_style_and_reset);
        output.push_str(" - ");
    }

    write_with_style_and_reset(output, diagnostic.category.name(), get_category_format(diagnostic.category));
    let _ = write!(output, "{} {}{}: {}", FOREGROUND_COLOR_ESCAPE_GREY, diagnostic_prefix(diagnostic), diagnostic.code, RESET_ESCAPE_SEQUENCE);
    write_flattened_diagnostic_message(output, diagnostic, format_opts.new_line);

    if let Some(file) = &diagnostic.file {
        if diagnostic.code != file_appears_to_be_binary_code() {
            output.push_str(format_opts.new_line);
            write_code_snippet(output, file, diagnostic.pos, diagnostic.len(), get_category_format(diagnostic.category), "", format_opts);
            output.push_str(format_opts.new_line);
        }
    }

    for related_information in &diagnostic.related {
        if let Some(file) = &related_information.file {
            output.push_str(format_opts.new_line);
            output.push_str("  ");
            let pos = related_information.pos;
            write_location(output, file, pos, Some(format_opts), write_with_style_and_reset);
            output.push_str(" - ");
            write_flattened_diagnostic_message(output, related_information, format_opts.new_line);
            write_code_snippet(output, file, pos, related_information.len(), FOREGROUND_COLOR_ESCAPE_CYAN, "    ", format_opts);
        }
        output.push_str(format_opts.new_line);
    }
}

fn is_go_space(c: char) -> bool {
    // unicode.IsSpace
    matches!(c, '\t' | '\n' | '\u{000B}' | '\u{000C}' | '\r' | ' ' | '\u{0085}' | '\u{00A0}') || (c > '\u{00FF}' && c.is_whitespace())
}

fn write_code_snippet(
    writer: &mut String,
    source_file: &FileLike,
    start: TextPos,
    length: u32,
    squiggle_color: &str,
    indent: &str,
    format_opts: &FormattingOptions,
) {
    let (first_line, first_line_char) = get_ecma_line_and_utf16_character_of_position(source_file, start);
    let (last_line, mut last_line_char) = get_ecma_line_and_utf16_character_of_position(source_file, start.checked_add(length).expect("diagnostic end exceeds TextPos"));
    if length == 0 {
        last_line_char += 1; // When length is zero, squiggle the character right after the start position.
    }

    let last_line_of_file = get_ecma_line_of_position(source_file, TextPos::try_from(source_file.text.len()).expect("source length exceeds TextPos"));

    let has_more_than_five_lines = last_line as i64 - first_line as i64 >= 4;
    let mut gutter_width = (last_line + 1).to_string().len();
    if has_more_than_five_lines {
        gutter_width = gutter_width.max(ELLIPSIS.len());
    }

    let mut i = first_line;
    while i <= last_line {
        writer.push_str(format_opts.new_line);

        // If the error spans over 5 lines, we'll only show the first 2 and last 2 lines,
        // so we'll skip ahead to the second-to-last line.
        if has_more_than_five_lines && first_line + 1 < i && i < last_line - 1 {
            writer.push_str(indent);
            writer.push_str(GUTTER_STYLE_SEQUENCE);
            let _ = write!(writer, "{:>width$}", ELLIPSIS, width = gutter_width);
            writer.push_str(RESET_ESCAPE_SEQUENCE);
            writer.push_str(GUTTER_SEPARATOR);
            writer.push_str(format_opts.new_line);
            i = last_line - 1;
        }

        let line_start = source_file.line_map[i] as usize;
        let line_end = if i < last_line_of_file { source_file.line_map[i + 1] as usize } else { source_file.text.len() };

        let line_content = byte_slice(&source_file.text, line_start, line_end).trim_end_matches(is_go_space); // trim from end
        let line_content = line_content.replace('\t', " "); // convert tabs to single spaces

        // Output the gutter and the actual contents of the line.
        writer.push_str(indent);
        writer.push_str(GUTTER_STYLE_SEQUENCE);
        let _ = write!(writer, "{:>width$}", i + 1, width = gutter_width);
        writer.push_str(RESET_ESCAPE_SEQUENCE);
        writer.push_str(GUTTER_SEPARATOR);
        writer.push_str(&line_content);
        writer.push_str(format_opts.new_line);

        // Output the gutter and the error span for the line using tildes.
        writer.push_str(indent);
        writer.push_str(GUTTER_STYLE_SEQUENCE);
        let _ = write!(writer, "{:>width$}", "", width = gutter_width);
        writer.push_str(RESET_ESCAPE_SEQUENCE);
        writer.push_str(GUTTER_SEPARATOR);
        writer.push_str(squiggle_color);
        if i == first_line {
            // If we're on the last line, then limit it to the last character of the last line.
            // Otherwise, we'll just squiggle the rest of the line, giving 'slice' no end position.
            let last_char_for_line = if i == last_line { last_line_char } else { utf16_len(&line_content) };

            // Fill with spaces until the first character,
            // then squiggle the remainder of the line.
            writer.push_str(&" ".repeat(first_line_char.max(0) as usize));
            writer.push_str(&"~".repeat((last_char_for_line - first_line_char).max(0) as usize));
        } else if i == last_line {
            // Squiggle until the final character.
            writer.push_str(&"~".repeat(last_line_char.max(0) as usize));
        } else {
            // Squiggle the entire line.
            writer.push_str(&"~".repeat(utf16_len(&line_content) as usize));
        }

        writer.push_str(RESET_ESCAPE_SEQUENCE);
        i += 1;
    }
}

pub fn flatten_diagnostic_message(d: &Diag, new_line: &str) -> String {
    let mut output = String::new();
    write_flattened_diagnostic_message(&mut output, d, new_line);
    output
}

pub fn write_flattened_diagnostic_message(writer: &mut String, diagnostic: &Diag, newline: &str) {
    writer.push_str(&diagnostic.message);
    for chain in &diagnostic.chain {
        flatten_diagnostic_message_chain(writer, chain, newline, 1);
    }
}

fn flatten_diagnostic_message_chain(writer: &mut String, chain: &Diag, new_line: &str, level: usize) {
    writer.push_str(new_line);
    for _ in 0..level {
        writer.push_str("  ");
    }
    writer.push_str(&chain.message);
    for child in &chain.chain {
        flatten_diagnostic_message_chain(writer, child, new_line, level + 1);
    }
}

fn diagnostic_prefix(diagnostic: &Diag) -> &str {
    if !diagnostic.source.is_empty() {
        return &diagnostic.source;
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

pub type FormattedWriter = fn(&mut String, &str, &str);

pub fn write_with_style_and_reset(output: &mut String, text: &str, format_style: &str) {
    output.push_str(format_style);
    output.push_str(text);
    output.push_str(RESET_ESCAPE_SEQUENCE);
}

pub fn write_location(output: &mut String, file: &FileLike, pos: TextPos, format_opts: Option<&FormattingOptions>, write_with_style_and_reset: FormattedWriter) {
    let (first_line, first_char) = get_ecma_line_and_utf16_character_of_position(file, pos);
    let relative_file_name = match format_opts {
        Some(o) => tspath::convert_to_relative_path(&file.file_name, &o.compare_paths_options),
        None => file.file_name.clone(),
    };

    write_with_style_and_reset(output, &relative_file_name, FOREGROUND_COLOR_ESCAPE_CYAN);
    output.push(':');
    write_with_style_and_reset(output, &(first_line + 1).to_string(), FOREGROUND_COLOR_ESCAPE_YELLOW);
    output.push(':');
    write_with_style_and_reset(output, &(first_char + 1).to_string(), FOREGROUND_COLOR_ESCAPE_YELLOW);
}

pub fn write_error_summary_text(output: &mut String, all_diagnostics: &[&Diag], format_opts: &FormattingOptions) {
    // Roughly corresponds to 'getErrorSummaryText' from watch.ts
    let mut total_error_count = 0;
    let mut global_errors = 0;
    // FileLike identity -> diagnostics, sorted by file name afterwards.
    let mut errors_by_file: Vec<(Rc<FileLike>, Vec<&Diag>)> = Vec::new();
    for diagnostic in all_diagnostics {
        if diagnostic.category != Category::Error {
            continue;
        }
        total_error_count += 1;
        match &diagnostic.file {
            None => global_errors += 1,
            Some(f) => match errors_by_file.iter_mut().find(|(g, _)| Rc::ptr_eq(g, f)) {
                Some((_, v)) => v.push(diagnostic),
                None => errors_by_file.push((Rc::clone(f), vec![diagnostic])),
            },
        }
    }
    if total_error_count == 0 {
        return;
    }
    errors_by_file.sort_by(|a, b| a.0.file_name.cmp(&b.0.file_name));

    let first_file_name = match errors_by_file.first() {
        Some((f, errs)) => pretty_path_for_file_error(f, errs, format_opts),
        None => String::new(),
    };
    let num_erroring_files = errors_by_file.len();

    let message = if total_error_count == 1 {
        // Special-case a single error.
        if global_errors > 0 || first_file_name.is_empty() {
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
    output.push_str(format_opts.new_line);
    output.push_str(&message);
    output.push_str(format_opts.new_line);
    output.push_str(format_opts.new_line);
    if num_erroring_files > 1 {
        write_tabular_errors_display(output, &errors_by_file, format_opts);
        output.push_str(format_opts.new_line);
    }
}

fn write_tabular_errors_display(output: &mut String, errors_by_file: &[(Rc<FileLike>, Vec<&Diag>)], format_opts: &FormattingOptions) {
    let max_errors = errors_by_file.iter().map(|(_, e)| e.len()).max().unwrap_or(0);

    let header_row = diagnostics::Errors_Files.localize(&[]);
    let left_column_heading_length = header_row.split(' ').next().unwrap().len();
    let length_of_biggest_error_count = max_errors.to_string().len();
    let left_padding_goal = left_column_heading_length.max(length_of_biggest_error_count);
    let header_padding = length_of_biggest_error_count.saturating_sub(left_column_heading_length);

    output.push_str(&" ".repeat(header_padding));
    output.push_str(&header_row);
    output.push_str(format_opts.new_line);

    for (file, file_errors) in errors_by_file {
        let _ = write!(output, "{:>width$}  ", file_errors.len(), width = left_padding_goal);
        output.push_str(&pretty_path_for_file_error(file, file_errors, format_opts));
        output.push_str(format_opts.new_line);
    }
}

fn pretty_path_for_file_error(file: &FileLike, file_errors: &[&Diag], format_opts: &FormattingOptions) -> String {
    if file_errors.is_empty() {
        return String::new();
    }
    let line = get_ecma_line_of_position(file, file_errors[0].pos);
    let mut file_name = file.file_name.clone();
    if tspath::path_is_absolute(&file_name) && tspath::path_is_absolute(&format_opts.compare_paths_options.current_directory) {
        file_name = tspath::convert_to_relative_path(&file.file_name, &format_opts.compare_paths_options);
    }
    format!("{}{}:{}{}", file_name, FOREGROUND_COLOR_ESCAPE_GREY, line + 1, RESET_ESCAPE_SEQUENCE)
}

pub fn write_format_diagnostics(output: &mut String, diagnostics: &[&Diag], format_opts: &FormattingOptions) {
    for diagnostic in diagnostics {
        write_format_diagnostic(output, diagnostic, format_opts);
    }
}

pub fn write_format_diagnostic(output: &mut String, diagnostic: &Diag, format_opts: &FormattingOptions) {
    if let Some(file) = &diagnostic.file {
        let (line, character) = get_ecma_line_and_utf16_character_of_position(file, diagnostic.pos);
        let relative_file_name = tspath::convert_to_relative_path(&file.file_name, &format_opts.compare_paths_options);
        let _ = write!(output, "{}({},{}): ", relative_file_name, line + 1, character + 1);
    }

    let _ = write!(output, "{} {}{}: ", diagnostic.category.name(), diagnostic_prefix(diagnostic), diagnostic.code);
    write_flattened_diagnostic_message(output, diagnostic, format_opts.new_line);
    output.push_str(format_opts.new_line);
}
