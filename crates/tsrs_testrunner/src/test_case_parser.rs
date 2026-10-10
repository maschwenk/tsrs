use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;
use rustc_hash::FxHashMap;
use tsrs_core::tspath;

use crate::compiler_runner::SRC_FOLDER;
use crate::harnessutil;

// Go's regexp `\s` is ASCII-only `[\t\n\f\r ]` and `\w` is `[0-9A-Za-z_]`; spell them out so the Rust
// (Unicode-aware) regex engine matches exactly the same inputs.
static LINE_DELIMITER: LazyLock<Regex> = LazyLock::new(|| Regex::new("\r?\n").unwrap());

// Regex for parsing options in the format "@Alpha: Value of any sort"
pub(crate) static OPTION_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^/{2}[\t\n\x0C\r ]*@([0-9A-Za-z_]+)[\t\n\x0C\r ]*:[\t\n\x0C\r ]*([^\r\n]*)").unwrap());

// Regex for parsing @link option
static LINK_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^/{2}[\t\n\x0C\r ]*@link[\t\n\x0C\r ]*:[\t\n\x0C\r ]*([^\r\n]*)[\t\n\x0C\r ]*->[\t\n\x0C\r ]*([^\r\n]*)").unwrap()
});

// File-specific directives used by fourslash tests
const FOURSLASH_DIRECTIVES: &[&str] = &["emitthisfile", "noopen"];

// This maps a compiler setting to its value as written in the test file. For example, if a test file contains:
//
//	// @target: esnext, es2015
//
// Then the map will map "target" to "esnext, es2015"
pub type RawCompilerSettings = BTreeMap<String, String>;

// All the necessary information to turn a multi file test into useful units for later compilation
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestUnit {
    pub content: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestCaseContent {
    pub test_unit_data: Vec<TestUnit>,
    pub ts_config_file_unit_data: Option<TestUnit>,
    pub symlinks: BTreeMap<String, String>,
    pub current_directory: String,
    pub global_options: FxHashMap<String, String>,
}

// Given a test file containing // @FileName directives,
// return an array of named units of code to be added to an existing compiler instance.
//
// The tsconfig.json unit (if any) is split off here; parsing it into a ParsedCommandLine happens in
// options.rs (parse_test_ts_config).
pub fn make_units_from_test(code: &str, file_name: &str) -> TestCaseContent {
    let (mut test_units, symlinks, mut current_directory, global_options) =
        parse_test_files_and_symlinks(code, file_name, |filename, content, _| TestUnit { content: content.to_string(), name: filename.to_string() });

    if current_directory.is_empty() {
        current_directory = SRC_FOLDER.to_string();
    }

    // check if project has tsconfig.json in the list of files
    let mut ts_config_file_unit_data = None;
    if let Some(i) = test_units.iter().position(|data| !harnessutil::get_config_name_from_file_name(&data.name).is_empty()) {
        ts_config_file_unit_data = Some(test_units.remove(i));
    }

    TestCaseContent { test_unit_data: test_units, ts_config_file_unit_data, symlinks, current_directory, global_options }
}

#[derive(Default, Clone, Copy)]
pub struct ParseTestFilesOptions {
    // If true, allows test content to appear before the first @Filename directive.
    // In this case, an implicit first file is created using the fileName parameter.
    // This matches the behavior of the TypeScript fourslash test harness.
    pub allow_implicit_first_file: bool,
}

// Given a test file containing // @FileName and // @symlink directives,
// return an array of named units of code to be added to an existing compiler instance,
// along with a map of symlinks and the current directory.
pub fn parse_test_files_and_symlinks<T>(
    code: &str,
    file_name: &str,
    parse_file: impl FnMut(&str, &str, &FxHashMap<String, String>) -> T,
) -> (Vec<T>, BTreeMap<String, String>, String, FxHashMap<String, String>) {
    parse_test_files_and_symlinks_with_options(code, file_name, parse_file, ParseTestFilesOptions::default())
}

pub fn parse_test_files_and_symlinks_with_options<T>(
    code: &str,
    file_name: &str,
    mut parse_file: impl FnMut(&str, &str, &FxHashMap<String, String>) -> T,
    options: ParseTestFilesOptions,
) -> (Vec<T>, BTreeMap<String, String>, String, FxHashMap<String, String>) {
    // List of all the subfiles we've parsed out
    let mut test_units: Vec<T> = Vec::new();

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
    let mut current_file_options: FxHashMap<String, String> = FxHashMap::default();
    let mut symlinks = BTreeMap::new();
    let mut global_options: FxHashMap<String, String> = FxHashMap::default();

    for line in LINE_DELIMITER.split(code) {
        if parse_symlink_from_test(line, &mut symlinks) {
            continue;
        }
        if let Some(test_meta_data) = OPTION_REGEX.captures(line) {
            // Comment line, check for global/file @options and record them
            let meta_data_name = test_meta_data[1].to_lowercase();
            let meta_data_value = test_meta_data[2].trim().to_string();
            if meta_data_name == "currentdirectory" {
                current_directory.clone_from(&meta_data_value);
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
                    test_units.push(parse_file(&current_file_name, &current_file_content, &current_file_options));
                }

                // Reset local data
                current_file_content.clear();
                seen_content_line = false;
                current_file_name = meta_data_value;
                current_file_options = FxHashMap::default();
            } else {
                // First metadata marker in the file
                let has_content_before_first_filename =
                    !current_file_content.is_empty() && skip_trivia(&current_file_content, 0) != current_file_content.len();
                if has_content_before_first_filename && !options.allow_implicit_first_file {
                    panic!("Non-comment test content appears before the first '// @Filename' directive");
                }

                // If we have content before the first @Filename and AllowImplicitFirstFile is true,
                // we need to save it as an implicit first file before starting the new file
                if has_content_before_first_filename && options.allow_implicit_first_file && !current_file_name.is_empty() {
                    has_seen_file = true;
                    test_units.push(parse_file(&current_file_name, &current_file_content, &current_file_options));
                }

                // Reset for the new file
                current_file_content.clear();
                seen_content_line = false;
                current_file_name = test_meta_data[2].trim().to_string();
                current_file_options = FxHashMap::default();
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

    // EOF, push whatever remains
    test_units.push(parse_file(&current_file_name, &current_file_content, &current_file_options));

    (test_units, symlinks, current_directory, global_options)
}

pub fn extract_compiler_settings(content: &str) -> RawCompilerSettings {
    let mut opts = RawCompilerSettings::new();
    for m in OPTION_REGEX.captures_iter(content) {
        let value = m[2].trim();
        opts.insert(m[1].to_lowercase(), value.strip_suffix(';').unwrap_or(value).to_string());
    }
    opts
}

fn parse_symlink_from_test(line: &str, symlinks: &mut BTreeMap<String, String>) -> bool {
    let Some(link_meta_data) = LINK_REGEX.captures(line) else {
        return false;
    };
    symlinks.insert(link_meta_data[2].trim().to_string(), link_meta_data[1].trim().to_string());
    true
}

// scanner.SkipTrivia, restricted to what the "content before first @Filename" check needs: whitespace,
// line breaks, single-line and multi-line comments, and a leading shebang.
fn skip_trivia(text: &str, mut pos: usize) -> usize {
    let bytes = text.as_bytes();
    if pos == 0 && bytes.starts_with(b"#!") {
        while pos < bytes.len() && bytes[pos] != b'\n' && bytes[pos] != b'\r' {
            pos += 1;
        }
    }
    loop {
        if pos >= bytes.len() {
            return pos;
        }
        let rest = &text[pos..];
        let ch = rest.chars().next().unwrap();
        match ch {
            '\r' | '\n' | '\t' | '\u{000B}' | '\u{000C}' | ' ' => pos += 1,
            '/' if rest.starts_with("//") => {
                pos += 2;
                while pos < bytes.len() {
                    let c = text[pos..].chars().next().unwrap();
                    if matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}') {
                        break;
                    }
                    pos += c.len_utf8();
                }
            }
            '/' if rest.starts_with("/*") => {
                pos += 2;
                while pos < bytes.len() {
                    if text[pos..].starts_with("*/") {
                        pos += 2;
                        break;
                    }
                    pos += text[pos..].chars().next().unwrap().len_utf8();
                }
            }
            c if c > '\u{7F}' && (c.is_whitespace() || c == '\u{FEFF}') => pos += c.len_utf8(),
            _ => return pos,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_make_units_from_test() {
        let code = "// @strict: true
// @noEmit: true
// @filename: firstFile.ts
function foo() { return \"a\"; }
// normal comment
// @filename: secondFile.ts
// some other comment
function bar() { return \"b\"; }";
        let result = make_units_from_test(code, "simpleTest.ts");
        assert_eq!(
            result.test_unit_data,
            vec![
                TestUnit { content: "function foo() { return \"a\"; }\n// normal comment".to_string(), name: "firstFile.ts".to_string() },
                TestUnit { content: "// some other comment\nfunction bar() { return \"b\"; }".to_string(), name: "secondFile.ts".to_string() },
            ]
        );
        assert_eq!(result.ts_config_file_unit_data, None);
        assert!(result.symlinks.is_empty());
    }

    #[test]
    fn test_extract_settings() {
        let s = extract_compiler_settings("// @Target: ES5, ES2015;\r\n//@strict:true\n// @filename: a.ts\nfoo");
        assert_eq!(s.get("target").map(String::as_str), Some("ES5, ES2015"));
        assert_eq!(s.get("strict").map(String::as_str), Some("true"));
        assert_eq!(s.get("filename").map(String::as_str), Some("a.ts"));
    }

    #[test]
    fn test_symlinks() {
        let code = "// @filename: /a/b.ts\nexport {}\n// @link: /a -> /c\n// @filename: /x.ts\n// @symlink: /y.ts, /z.ts\nlet x";
        let result = make_units_from_test(code, "t.ts");
        assert_eq!(result.symlinks.get("/c").map(String::as_str), Some("/a"));
        assert_eq!(result.symlinks.get("/y.ts").map(String::as_str), Some("/x.ts"));
        assert_eq!(result.symlinks.get("/z.ts").map(String::as_str), Some("/x.ts"));
    }
}
