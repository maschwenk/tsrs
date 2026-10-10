// internal/testutil/tsbaseline: error_baseline.go and util.go.

use std::fmt::Write as _;
use std::sync::LazyLock;

use regex::Regex;
use tsrs_core::tspath::{self, ComparePathsOptions};

use crate::baseline::NO_CONTENT;
use crate::diagnosticwriter::{self as dw, compute_ecma_line_starts, Diag, FormattingOptions};
use crate::harnessutil::TestFile;

// IO
const HARNESS_NEW_LINE: &str = "\r\n";

fn format_opts() -> FormattingOptions {
    FormattingOptions { compare_paths_options: ComparePathsOptions::default(), new_line: HARNESS_NEW_LINE }
}

static DIAGNOSTICS_LOCATION_PREFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?im)^(lib.*\.d\.ts)\([0-9]+,[0-9]+\)").unwrap());
static DIAGNOSTICS_LOCATION_PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(lib.*\.d\.ts):[0-9]+:[0-9]+").unwrap());
static LINE_DELIMITER: LazyLock<Regex> = LazyLock::new(|| Regex::new("\r?\n").unwrap());
pub(crate) static TS_EXTENSION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.tsx?$").unwrap());

// Returns the error baseline text for the given diagnostics (`baseline.NoContent` when there are none), as
// `DoErrorBaseline` passes it to `baseline.Run`.
pub fn do_error_baseline(input_files: &[TestFile], errors: &[Diag], pretty: bool) -> String {
    if !errors.is_empty() {
        get_error_baseline(input_files, errors, pretty)
    } else {
        NO_CONTENT.to_string()
    }
}

fn minimal_diagnostics_to_string(diagnostics: &[&Diag], pretty: bool) -> String {
    let mut output = String::new();
    if pretty {
        dw::format_diagnostics_with_color_and_context(&mut output, diagnostics, &format_opts());
    } else {
        dw::write_format_diagnostics(&mut output, diagnostics, &format_opts());
    }
    output
}

pub fn get_error_baseline(input_files: &[TestFile], diagnostics: &[Diag], pretty: bool) -> String {
    let mut output_lines = iterate_error_baseline(input_files, diagnostics, pretty);

    if pretty {
        let mut summary_builder = String::new();
        // Go passes the unsorted input slice here.
        let unsorted: Vec<&Diag> = diagnostics.iter().collect();
        dw::write_error_summary_text(&mut summary_builder, &unsorted, &format_opts());
        let summary = remove_test_path_prefixes(&summary_builder, false);
        output_lines.push(summary);
    }
    output_lines.concat()
}

fn iterate_error_baseline(input_files: &[TestFile], input_diagnostics: &[Diag], pretty: bool) -> Vec<String> {
    let mut diagnostics: Vec<&Diag> = input_diagnostics.iter().collect();
    // slices.SortFunc is not stable; diagnostics that compare equal are indistinguishable in the output.
    diagnostics.sort_by(|a, b| dw::compare_diagnostics(a, b).cmp(&0));

    let mut output_lines = String::new();
    let mut first_line = true;
    let mut new_line = || {
        if first_line {
            first_line = false;
            return "";
        }
        "\r\n"
    };

    let mut result = Vec::new();

    let output_error_text = |output_lines: &mut String, new_line: &mut dyn FnMut() -> &'static str, diag: &Diag| {
        let message = dw::flatten_diagnostic_message(diag, HARNESS_NEW_LINE);

        let mut err_lines = Vec::new();
        for line in remove_test_path_prefixes(&message, false).split('\n') {
            let line = line.strip_suffix('\r').unwrap_or(line);
            if line.is_empty() {
                continue;
            }
            err_lines.push(format!("!!! {} TS{}: {}", diag.category.name(), diag.code, line));
        }

        for info in &diag.related {
            let mut location = String::new();
            if let Some(file) = &info.file {
                location = format!(" {}", format_location(file, info.pos, &format_opts(), |output, text, _| output.push_str(text)));
            }
            location = remove_test_path_prefixes(&location, false);
            if !location.is_empty() && is_default_library_file(&info.file.as_ref().unwrap().file_name) {
                location = DIAGNOSTICS_LOCATION_PATTERN.replace_all(&location, "$1:--:--").into_owned();
            }
            err_lines.push(format!("!!! related TS{}{}: {}", info.code, location, dw::flatten_diagnostic_message(info, HARNESS_NEW_LINE)));
        }

        for e in err_lines {
            output_lines.push_str(new_line());
            output_lines.push_str(&e);
        }
    };

    let top_diagnostics = minimal_diagnostics_to_string(&diagnostics, pretty);
    let top_diagnostics = remove_test_path_prefixes(&top_diagnostics, false);
    let top_diagnostics = DIAGNOSTICS_LOCATION_PREFIX.replace_all(&top_diagnostics, "$1(--,--)");

    result.push(format!("{top_diagnostics}{HARNESS_NEW_LINE}{HARNESS_NEW_LINE}"));

    // Report global errors
    for error in &diagnostics {
        if error.file.is_none() {
            output_error_text(&mut output_lines, &mut new_line, error);
        }
    }

    result.push(std::mem::take(&mut output_lines));

    // 'merge' the lines of each input file with any errors associated with it
    for input_file in input_files {
        // Filter down to the errors in the file
        let unit = remove_test_path_prefixes(&input_file.unit_name, false);
        let file_errors: Vec<&Diag> = diagnostics
            .iter()
            .copied()
            .filter(|e| {
                e.file.as_ref().is_some_and(|f| {
                    tspath::compare_paths(&remove_test_path_prefixes(&f.file_name, false), &unit, &ComparePathsOptions::default()) == 0
                })
            })
            .collect();

        // Header
        let _ = write!(output_lines, "{}==== {} ({} errors) ====", new_line(), unit, file_errors.len());

        // For each line, emit the line followed by any error squiggles matching this line
        let content = input_file.content.as_bytes();
        let line_starts = compute_ecma_line_starts(&input_file.content);
        let lines: Vec<&str> = LINE_DELIMITER.split(&input_file.content).collect();

        for (line_index, line) in lines.iter().enumerate() {
            let mut line = line.as_bytes();
            if !line.is_empty() && line[line.len() - 1] == b'\r' {
                line = &line[..line.len() - 1];
            }

            let this_line_start = line_starts[line_index];
            // On the last line of the file, fake the next line start number so that we handle errors on the last character of the file correctly
            let next_line_start = if line_index == lines.len() - 1 { content.len() as i32 } else { line_starts[line_index + 1] };
            // Emit this line from the original file
            output_lines.push_str(new_line());
            output_lines.push_str("    ");
            output_lines.push_str(&tsrs_core::utf8::from_utf8_lossy(line));
            for err_diagnostic in &file_errors {
                // Does any error start or continue on to this line? Emit squiggles
                let err_start = err_diagnostic.pos;
                let end = err_start + err_diagnostic.len();
                if end >= this_line_start && (err_start < next_line_start || line_index == lines.len() - 1) {
                    // How many characters from the start of this line the error starts at (could be positive or negative)
                    let relative_offset = err_start - this_line_start;
                    // How many characters of the error are on this line (might be longer than this line in reality)
                    let length = (end - err_start) - 0.max(this_line_start - err_start);
                    // Calculate the start of the squiggle
                    let squiggle_start = 0.max(relative_offset) as usize;
                    output_lines.push_str(new_line());
                    output_lines.push_str("    ");
                    output_lines.push_str(&replace_non_whitespace(&line[..squiggle_start.min(line.len())]));
                    // This was `new Array(count).join("~")`; which maps 0 to "", 1 to "", 2 to "~", 3 to "~~", etc.
                    let squiggle_end = squiggle_start.max((squiggle_start as i64 + length as i64).min(line.len() as i64).max(0) as usize);
                    let squiggle_slice = if squiggle_start <= line.len() { &line[squiggle_start..squiggle_end.min(line.len())] } else { &[][..] };
                    output_lines.push_str(&"~".repeat(rune_count(squiggle_slice)));
                    // If the error ended here, or we're at the end of the file, emit its message
                    if line_index == lines.len() - 1 || next_line_start > end {
                        output_error_text(&mut output_lines, &mut new_line, err_diagnostic);
                    }
                }
            }
        }

        result.push(std::mem::take(&mut output_lines));
    }

    result
}

// utf8.RuneCountInString: each byte of an invalid sequence counts as one rune.
fn rune_count(bytes: &[u8]) -> usize {
    bytes.utf8_chunks().map(|c| c.valid().chars().count() + c.invalid().len()).sum()
}

// nonWhitespace.ReplaceAllString(s, " "), with Go's ASCII-only `\S`: every rune that is not one of
// `[\t\n\f\r ]` becomes a single space.
fn replace_non_whitespace(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        for c in chunk.valid().chars() {
            out.push(if matches!(c, '\t' | '\n' | '\x0C' | '\r' | ' ') { c } else { ' ' });
        }
        for _ in chunk.invalid() {
            out.push(' ');
        }
    }
    out
}

fn format_location(file: &dw::FileLike, pos: i32, format_opts: &FormattingOptions, write_with_style_and_reset: dw::FormattedWriter) -> String {
    let mut output = String::new();
    dw::write_location(&mut output, file, pos, Some(format_opts), write_with_style_and_reset);
    output
}

// util.go

const TEST_PATH_PREFIXES: &[(&str, &str)] = &[
    ("/.ts/", ""),
    ("/.lib/", ""),
    ("/.src/", ""),
    ("bundled:///libs/", ""),
    ("file:///./ts/", "file:///"),
    ("file:///./lib/", "file:///"),
    ("file:///./src/", "file:///"),
];

const TEST_PATH_PREFIXES_TRAILING_SEPARATOR: &[(&str, &str)] = &[
    ("/.ts/", "/"),
    ("/.lib/", "/"),
    ("/.src/", "/"),
    ("bundled:///libs/", "/"),
    ("file:///./ts/", "file:///"),
    ("file:///./lib/", "file:///"),
    ("file:///./src/", "file:///"),
];

// strings.NewReplacer semantics: scan left to right, at each position the first pattern (in argument
// order) that matches is replaced; matches do not overlap.
fn replace_all(text: &str, pairs: &[(&str, &str)]) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut last = 0;
    while i < bytes.len() {
        if let Some((old, new)) = pairs.iter().find(|(old, _)| bytes[i..].starts_with(old.as_bytes())) {
            out.push_str(&text[last..i]);
            out.push_str(new);
            i += old.len();
            last = i;
        } else {
            i += 1;
        }
    }
    out.push_str(&text[last..]);
    out
}

pub fn remove_test_path_prefixes(text: &str, retain_trailing_directory_separator: bool) -> String {
    if retain_trailing_directory_separator {
        return replace_all(text, TEST_PATH_PREFIXES_TRAILING_SEPARATOR);
    }
    replace_all(text, TEST_PATH_PREFIXES)
}

pub(crate) fn is_default_library_file(file_path: &str) -> bool {
    let file_name = tspath::get_base_file_name(file_path);
    file_name.starts_with("lib.") && file_name.ends_with(tspath::EXTENSION_DTS)
}
