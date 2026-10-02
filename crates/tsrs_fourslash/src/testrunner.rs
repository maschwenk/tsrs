// The part of Go's testrunner/test_case_parser.go that fourslash uses (ParseTestFilesAndSymlinksWithOptions with
// an error-returning parseFile). tsrs_testrunner is a binary crate with its own (non-erroring) copy of this
// function, so it is ported here again.

use std::sync::LazyLock;

use regex::Regex;
use tsrs_core::collections::OrderedMap;
use tsrs_core::tspath;

// Go's regexp `\s` is ASCII-only `[\t\n\f\r ]` and `\w` is `[0-9A-Za-z_]`; spelled out for the Rust engine.
// test_case_parser.go:18
static LINE_DELIMITER: LazyLock<Regex> = LazyLock::new(|| Regex::new("\r?\n").unwrap());

// test_case_parser.go:41
static OPTION_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^/{2}[\t\n\x0C\r ]*@([0-9A-Za-z_]+)[\t\n\x0C\r ]*:[\t\n\x0C\r ]*([^\r\n]*)").unwrap());

// test_case_parser.go:44
static LINK_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^/{2}[\t\n\x0C\r ]*@link[\t\n\x0C\r ]*:[\t\n\x0C\r ]*([^\r\n]*)[\t\n\x0C\r ]*->[\t\n\x0C\r ]*([^\r\n]*)").unwrap()
});

// test_case_parser.go:47
const FOURSLASH_DIRECTIVES: &[&str] = &["emitthisfile", "noopen"];

// test_case_parser.go:119
#[derive(Clone, Copy, Default)]
pub struct ParseTestFilesOptions {
    // If true, allows test content to appear before the first @Filename directive.
    // In this case, an implicit first file is created using the fileName parameter.
    // This matches the behavior of the TypeScript fourslash test harness.
    pub allow_implicit_first_file: bool,
}

pub struct ParsedTestFiles<U> {
    pub units: Vec<U>,
    pub symlinks: OrderedMap<String, String>,
    pub current_dir: String,
    pub global_options: OrderedMap<String, String>,
}

// Go returns all five results together, the error last; the partial results are unused when it is set.
// test_case_parser.go:138
pub fn parse_test_files_and_symlinks_with_options<U, E>(
    code: &str,
    file_name: &str,
    mut parse_file: impl FnMut(&str, &str, &OrderedMap<String, String>) -> Result<U, E>,
    options: ParseTestFilesOptions,
) -> Result<ParsedTestFiles<U>, E> {
    // List of all the subfiles we've parsed out
    let mut test_units: Vec<U> = Vec::new();

    // Stuff related to the subfile we're parsing
    let mut current_file_content = String::new();
    let mut current_file_name = String::new();
    let mut seen_content_line = false;
    let mut has_seen_file = false;
    if options.allow_implicit_first_file {
        // For fourslash tests, initialize currentFileName to the fileName parameter
        // so content before the first @Filename directive goes into an implicit first file
        current_file_name = file_name.to_string();
    }
    let mut current_directory = String::new();
    let mut parse_error: Option<E> = None;
    let mut current_file_options: OrderedMap<String, String> = OrderedMap::default();
    let mut symlinks: OrderedMap<String, String> = OrderedMap::default();
    let mut global_options: OrderedMap<String, String> = OrderedMap::default();

    for line in LINE_DELIMITER.split(code) {
        let ok = parse_symlink_from_test(line, &mut symlinks);
        if ok {
            continue;
        }
        if let Some(test_meta_data) = OPTION_REGEX.captures(line) {
            // Comment line, check for global/file @options and record them
            let meta_data_name = test_meta_data[1].to_lowercase();
            let meta_data_value = test_meta_data[2].trim().to_string();
            if meta_data_name == "currentdirectory" {
                current_directory = meta_data_value.clone();
            }
            if meta_data_name != "filename" {
                if meta_data_name == "symlink" && !current_file_name.is_empty() {
                    for link in meta_data_value.split(',') {
                        let link = link.trim();
                        if !link.is_empty() {
                            symlinks.insert(link.to_string(), current_file_name.clone());
                        }
                    }
                } else if FOURSLASH_DIRECTIVES.contains(&meta_data_name.as_str()) {
                    // File-specific option
                    current_file_options.insert(meta_data_name, meta_data_value);
                } else {
                    // Global option
                    // !!! (Go) a duplicate global option with a different value would break existing baselines
                    global_options.insert(meta_data_name, meta_data_value);
                }
                continue;
            }

            // New metadata statement after having collected some code to go with the previous metadata
            if !current_file_name.is_empty() {
                // Store result file - always save for regular tests, but skip empty implicit first file for fourslash
                let should_save_file = !options.allow_implicit_first_file || !current_file_content.is_empty() || has_seen_file;
                if should_save_file {
                    has_seen_file = true;
                    match parse_file(&current_file_name, &current_file_content, &current_file_options) {
                        Ok(u) => test_units.push(u),
                        Err(e) => {
                            parse_error = Some(e);
                            break;
                        }
                    }
                }

                // Reset local data
                current_file_content.clear();
                seen_content_line = false;
                current_file_name = meta_data_value;
                current_file_options = OrderedMap::default();
            } else {
                // First metadata marker in the file
                let has_content_before_first_filename = !current_file_content.is_empty()
                    && tsrs_scanner::skip_trivia(&current_file_content, 0) as usize != current_file_content.len();
                if has_content_before_first_filename && !options.allow_implicit_first_file {
                    panic!("Non-comment test content appears before the first '// @Filename' directive");
                }

                // If we have content before the first @Filename and AllowImplicitFirstFile is true,
                // we need to save it as an implicit first file before starting the new file
                if has_content_before_first_filename && options.allow_implicit_first_file && !current_file_name.is_empty() {
                    // Store the implicit first file
                    has_seen_file = true;
                    match parse_file(&current_file_name, &current_file_content, &current_file_options) {
                        Ok(u) => test_units.push(u),
                        Err(e) => {
                            parse_error = Some(e);
                            break;
                        }
                    }
                }

                // Reset for the new file
                current_file_content.clear();
                seen_content_line = false;
                current_file_name = test_meta_data[2].trim().to_string();
                current_file_options = OrderedMap::default();
            }
        } else {
            // Subfile content line
            // Append to the current subfile content, inserting a newline if needed
            // For fourslash tests, use seenContentLine to preserve leading blank lines
            // (matching TS fourslash's //// content markers). For compiler tests, use
            // Len() != 0 which drops leading blanks (matching TS's harness behavior).
            if options.allow_implicit_first_file {
                if seen_content_line {
                    current_file_content.push('\n');
                }
                seen_content_line = true;
            } else if !current_file_content.is_empty() {
                current_file_content.push('\n');
            }
            current_file_content.push_str(line);
        }
    }

    // normalize the fileName for the single file case
    if test_units.is_empty() && current_file_name.is_empty() {
        current_file_name = tspath::get_base_file_name(file_name);
    }

    // if there are no parse errors so far, parse the rest of the file
    if parse_error.is_none() {
        // EOF, push whatever remains
        match parse_file(&current_file_name, &current_file_content, &current_file_options) {
            Ok(u) => test_units.push(u),
            Err(e) => parse_error = Some(e),
        }
    }

    if let Some(e) = parse_error {
        return Err(e);
    }
    Ok(ParsedTestFiles { units: test_units, symlinks, current_dir: current_directory, global_options })
}

// test_case_parser.go:291
fn parse_symlink_from_test(line: &str, symlinks: &mut OrderedMap<String, String>) -> bool {
    let Some(link_meta_data) = LINK_REGEX.captures(line) else {
        return false;
    };

    symlinks.insert(link_meta_data[2].trim().to_string(), link_meta_data[1].trim().to_string());
    true
}
