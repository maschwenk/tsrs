// The part of Go's testutil/tsbaseline/error_baseline.go (and the diagnosticwriter functions it calls) that
// VerifyBaselineNonSuggestionDiagnostics uses: GetErrorBaseline with pretty=false over fourslash diagnostics.
// (tsrs_testrunner, a binary crate, has its own copy over its own diagnostic snapshot type.)

use std::sync::LazyLock;

use regex::Regex;
use tsrs_core::tspath::{self, ComparePathsOptions};

use crate::fourslash::FourslashDiagnostic;
use crate::testing::T;

// harnessutil.TestFile
#[derive(Clone, Debug, Default)]
pub struct TestFile {
    pub unit_name: String,
    pub content: String,
}

// error_baseline.go:24
const HARNESS_NEW_LINE: &str = "\r\n";

// error_baseline.go:30
static DIAGNOSTICS_LOCATION_PREFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?im)^(lib.*\.d\.ts)\([0-9]+,[0-9]+\)").unwrap());
static DIAGNOSTICS_LOCATION_PATTERN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(lib.*\.d\.ts):[0-9]+:[0-9]+").unwrap());
static LINE_DELIMITER: LazyLock<Regex> = LazyLock::new(|| Regex::new("\r?\n").unwrap());

// error_baseline.go:51 (pretty=false: diagnosticwriter.WriteFormatDiagnostics)
fn minimal_diagnostics_to_string(diagnostics: &[&FourslashDiagnostic]) -> String {
    let mut output = String::new();
    for diagnostic in diagnostics {
        write_format_diagnostic(&mut output, diagnostic);
    }
    output
}

// error_baseline.go:61 (pretty=false)
pub(crate) fn get_error_baseline(
    t: &T,
    input_files: &[TestFile],
    diagnostics: &[FourslashDiagnostic],
    compare_diagnostics: impl Fn(&FourslashDiagnostic, &FourslashDiagnostic) -> i32,
) -> String {
    let output_lines = iterate_error_baseline(t, input_files, diagnostics, compare_diagnostics);
    output_lines.concat()
}

// error_baseline.go:78 (pretty=false)
fn iterate_error_baseline(
    t: &T,
    input_files: &[TestFile],
    input_diagnostics: &[FourslashDiagnostic],
    compare_diagnostics: impl Fn(&FourslashDiagnostic, &FourslashDiagnostic) -> i32,
) -> Vec<String> {
    let mut diagnostics: Vec<&FourslashDiagnostic> = input_diagnostics.iter().collect();
    diagnostics.sort_by(|a, b| compare_diagnostics(a, b).cmp(&0));

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

    let output_error_text = |output_lines: &mut String, new_line: &mut dyn FnMut() -> &'static str, diag: &FourslashDiagnostic| {
        let message = flatten_diagnostic_message(diag);

        let mut err_lines = Vec::new();
        for line in remove_test_path_prefixes(&message).split('\n') {
            let line = line.strip_suffix('\r').unwrap_or(line);
            if line.is_empty() {
                continue;
            }
            err_lines.push(format!("!!! {} TS{}: {}", diag.category.name(), diag.code, line));
        }

        for info in &diag.related_diagnostics {
            let mut location = format!(" {}", format_location(info));
            location = remove_test_path_prefixes(&location);
            if !location.is_empty() && is_default_library_file(&info.file.file_name) {
                location = DIAGNOSTICS_LOCATION_PATTERN.replace_all(&location, "$1:--:--").into_owned();
            }
            err_lines.push(format!("!!! related TS{}{}: {}", info.code, location, flatten_diagnostic_message(info)));
        }

        for e in err_lines {
            output_lines.push_str(new_line());
            output_lines.push_str(&e);
        }
    };

    let top_diagnostics = minimal_diagnostics_to_string(&diagnostics);
    let top_diagnostics = remove_test_path_prefixes(&top_diagnostics);
    let top_diagnostics = DIAGNOSTICS_LOCATION_PREFIX.replace_all(&top_diagnostics, "$1(--,--)");

    result.push(format!("{top_diagnostics}{HARNESS_NEW_LINE}{HARNESS_NEW_LINE}"));

    // Report global errors (fourslash diagnostics always have a file)
    result.push(std::mem::take(&mut output_lines));

    // 'merge' the lines of each input file with any errors associated with it
    for input_file in input_files {
        // Filter down to the errors in the file
        let unit = remove_test_path_prefixes(&input_file.unit_name);
        let file_errors: Vec<&FourslashDiagnostic> = diagnostics
            .iter()
            .copied()
            .filter(|e| tspath::compare_paths(&remove_test_path_prefixes(&e.file.file_name), &unit, &ComparePathsOptions::default()) == 0)
            .collect();

        // Header
        output_lines.push_str(&format!("{}==== {} ({} errors) ====", new_line(), unit, file_errors.len()));

        // Make sure we emit something for every error
        let mut marked_error_count = 0;
        // For each line, emit the line followed by any error squiggles matching this line
        let content = input_file.content.as_bytes();
        let line_starts = tsrs_core::compute_ecma_line_starts(&input_file.content);
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
                let err_start = err_diagnostic.loc.pos();
                let end = err_start + err_diagnostic.loc.len();
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
                        marked_error_count += 1;
                    }
                }
            }
        }

        // Verify we didn't miss any errors in this file
        crate::go::assert::check(t, marked_error_count == file_errors.len(), &format!("count of errors in {}", input_file.unit_name));
        result.push(std::mem::take(&mut output_lines));
    }

    result
}

// utf8.RuneCountInString: each byte of an invalid sequence counts as one rune.
fn rune_count(bytes: &[u8]) -> usize {
    bytes.utf8_chunks().map(|c| c.valid().chars().count() + c.invalid().len()).sum()
}

// nonWhitespace.ReplaceAllString(s, " "), with Go's ASCII-only `\S`.
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

// error_baseline.go:272 (with a writer that drops the styles: diagnosticwriter.WriteLocation)
fn format_location(diag: &FourslashDiagnostic) -> String {
    let (first_line, first_char) = get_ecma_line_and_utf16_character_of_position(diag, diag.loc.pos());
    let relative_file_name = tspath::convert_to_relative_path(&diag.file.file_name, &ComparePathsOptions::default());
    format!("{}:{}:{}", relative_file_name, first_line + 1, first_char + 1)
}

// diagnosticwriter.GetECMALineAndUTF16CharacterOfPosition
fn get_ecma_line_and_utf16_character_of_position(diag: &FourslashDiagnostic, pos: i32) -> (usize, i32) {
    let line_map = &diag.file.ecma_line_map;
    let line = tsrs_scanner::compute_line_of_position(line_map, pos).max(0) as usize;
    let start = line_map[line].max(0) as usize;
    let text = &diag.file.content;
    let end = (pos.max(0) as usize).min(text.len()).max(start);
    let character = text.get(start..end).map_or(0, |s| s.chars().map(|c| c.len_utf16() as i32).sum());
    (line, character)
}

// diagnosticwriter.FlattenDiagnosticMessage (fourslash diagnostics have no message chain)
fn flatten_diagnostic_message(diag: &FourslashDiagnostic) -> String {
    diag.message.clone()
}

// diagnosticwriter.WriteFormatDiagnostic
fn write_format_diagnostic(output: &mut String, diagnostic: &FourslashDiagnostic) {
    let (line, character) = get_ecma_line_and_utf16_character_of_position(diagnostic, diagnostic.loc.pos());
    let relative_file_name = tspath::convert_to_relative_path(&diagnostic.file.file_name, &ComparePathsOptions::default());
    output.push_str(&format!("{}({},{}): ", relative_file_name, line + 1, character + 1));

    output.push_str(&format!("{} TS{}: ", diagnostic.category.name(), diagnostic.code));
    output.push_str(&flatten_diagnostic_message(diagnostic));
    output.push_str(HARNESS_NEW_LINE);
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

// strings.NewReplacer semantics: scan left to right, at each position the first pattern (in argument
// order) that matches is replaced; matches do not overlap.
fn remove_test_path_prefixes(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut last = 0;
    while i < bytes.len() {
        if let Some((old, new)) = TEST_PATH_PREFIXES.iter().find(|(old, _)| bytes[i..].starts_with(old.as_bytes())) {
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

fn is_default_library_file(file_path: &str) -> bool {
    let file_name = tspath::get_base_file_name(file_path);
    file_name.starts_with("lib.") && file_name.ends_with(tspath::EXTENSION_DTS)
}
