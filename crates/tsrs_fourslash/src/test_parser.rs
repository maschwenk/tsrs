use std::sync::Arc;

use tsrs_core::collections::OrderedMap;
use tsrs_core::stringutil;
use tsrs_core::tspath;
use tsrs_core::TextRange;
use tsrs_ls::lsconv::{self, Script};
use tsrs_ls::spanmap::SpanMap;
use tsrs_lsproto as lsproto;

use crate::fourslash::new_test_converters;
use crate::go::Any;
use crate::testing::T;
use crate::testrunner;

// Inserted in source files by surrounding desired text
// in a range with `[|` and `|]`. For example,
//
// [|text in range|]
//
// is a range with `text in range` "selected".
// test_parser.go:26
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RangeMarker {
    pub(crate) file_name: String,
    pub range: TextRange,
    pub ls_range: lsproto::Range,
    pub marker: Option<Arc<Marker>>,
}

impl RangeMarker {
    // test_parser.go:33
    pub fn ls_pos(&self) -> lsproto::Position {
        self.ls_range.start
    }

    // test_parser.go:37
    pub fn file_name(&self) -> String {
        self.file_name.clone()
    }

    // test_parser.go:41
    pub fn get_name(&self) -> Option<String> {
        match &self.marker {
            None => None,
            Some(m) => m.name.clone(),
        }
    }

    // test_parser.go:48
    pub fn ls_location(&self) -> lsproto::Location {
        lsproto::Location { uri: lsconv::file_name_to_document_uri(&self.file_name), range: self.ls_range }
    }
}

// test_parser.go:55
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Marker {
    pub(crate) file_name: String,
    pub position: i32,
    pub ls_position: lsproto::Position,
    pub name: Option<String>, // `nil` for anonymous markers such as `{| "foo": "bar" |}`
    // Go's map is nil for markers without data; object markers always have at least one key.
    pub data: OrderedMap<String, Any>,
}

impl Marker {
    // test_parser.go:63
    pub fn ls_pos(&self) -> lsproto::Position {
        self.ls_position
    }

    // test_parser.go:67
    pub fn file_name(&self) -> String {
        self.file_name.clone()
    }

    // test_parser.go:71
    pub fn get_name(&self) -> Option<String> {
        self.name.clone()
    }

    // test_parser.go:75
    pub fn maker_with_symlink(&self, file_name: &str) -> Arc<Marker> {
        Arc::new(Marker {
            file_name: file_name.to_string(),
            position: self.position,
            ls_position: self.ls_position,
            name: self.name.clone(),
            data: self.data.clone(),
        })
    }
}

// Go interface `MarkerOrRange` (implemented by *Marker and *RangeMarker).
// test_parser.go:85
#[derive(Clone, Debug, PartialEq)]
pub enum MarkerOrRange {
    Marker(Arc<Marker>),
    RangeMarker(Arc<RangeMarker>),
}

impl MarkerOrRange {
    pub fn file_name(&self) -> String {
        match self {
            MarkerOrRange::Marker(m) => m.file_name(),
            MarkerOrRange::RangeMarker(r) => r.file_name(),
        }
    }

    pub fn ls_pos(&self) -> lsproto::Position {
        match self {
            MarkerOrRange::Marker(m) => m.ls_pos(),
            MarkerOrRange::RangeMarker(r) => r.ls_pos(),
        }
    }

    pub fn get_name(&self) -> Option<String> {
        match self {
            MarkerOrRange::Marker(m) => m.get_name(),
            MarkerOrRange::RangeMarker(r) => r.get_name(),
        }
    }
}

// test_parser.go:91
#[derive(Clone, Debug, Default)]
pub struct TestData {
    pub files: Vec<Arc<TestFileInfo>>,
    pub marker_positions: OrderedMap<String, Arc<Marker>>,
    pub markers: Vec<Arc<Marker>>,
    pub symlinks: OrderedMap<String, String>,
    pub global_options: OrderedMap<String, String>,
    pub ranges: Vec<Arc<RangeMarker>>,
}

impl TestData {
    // test_parser.go:100
    pub(crate) fn is_state_baselining_enabled(&self) -> bool {
        is_state_baselining_enabled(&self.global_options)
    }
}

// Markers are mutated after the file is scanned (LSPosition) while range markers point at them, so they are
// collected by value and linked by index; the Arcs are made once the positions are known.
// test_parser.go:104
struct TestFileWithMarkers {
    file: Arc<TestFileInfo>,
    markers: Vec<Arc<Marker>>,
    ranges: Vec<Arc<RangeMarker>>,
}

// test_parser.go:110
pub(crate) fn is_state_baselining_enabled(global_options: &OrderedMap<String, String>) -> bool {
    global_options.get("statebaseline").map(|s| s.as_str()) == Some("true")
}

// test_parser.go:114
pub fn parse_test_data(t: &T, contents: &str, file_name: &str) -> TestData {
    // List of all the subfiles we've parsed out
    let mut files: Vec<Arc<TestFileInfo>> = Vec::new();

    let mut marker_positions: OrderedMap<String, Arc<Marker>> = OrderedMap::default();
    let mut markers: Vec<Arc<Marker>> = Vec::new();
    let mut ranges: Vec<Arc<RangeMarker>> = Vec::new();

    let parsed = testrunner::parse_test_files_and_symlinks_with_options(
        contents,
        file_name,
        parse_file_content,
        testrunner::ParseTestFilesOptions { allow_implicit_first_file: true },
    );
    let parsed = match parsed {
        Ok(p) => p,
        Err(e) => t.fatal(&format!("Error parsing fourslash data: {}", e.error())),
    };
    let files_with_marker = parsed.units;
    let symlinks = parsed.symlinks;
    let global_options = parsed.global_options;

    let mut has_ts_config = false;
    for file in files_with_marker {
        files.push(file.file.clone());
        has_ts_config = has_ts_config || is_config_file(&file.file.file_name);

        markers.extend(file.markers.iter().cloned());
        ranges.extend(file.ranges.iter().cloned());
        for marker in &file.markers {
            let Some(name) = &marker.name else {
                if !marker.data.is_empty() {
                    // The marker is an anonymous object marker, which does not need a name. Markers are only set into markerPositions if they have a name
                    continue;
                }
                t.fatal(&format!("Marker at position {} is unnamed", marker.position));
            };
            if let Some(existing) = marker_positions.get(name) {
                t.fatal(&format!(r#"Duplicate marker name: "{}" at {} and {}"#, name, marker.position, existing.position));
            }
            marker_positions.insert(name.clone(), marker.clone());
        }
    }

    if has_ts_config && has_unsupported_global_options_with_config(&global_options) && !is_state_baselining_enabled(&global_options) {
        t.fatal("It is not allowed to use global options along with config files.");
    }

    TestData { files, marker_positions, markers, symlinks, global_options, ranges }
}

// test_parser.go:171
fn has_unsupported_global_options_with_config(global_options: &OrderedMap<String, String>) -> bool {
    for option in global_options.keys() {
        match option.to_lowercase().as_str() {
            "symlink" | "link" | "usecasesensitivefilenames" => continue,
            _ => return true,
        }
    }
    false
}

// test_parser.go:183
fn is_config_file(file_name: &str) -> bool {
    let file_name = file_name.to_lowercase();
    file_name.ends_with("tsconfig.json") || file_name.ends_with("jsconfig.json")
}

// test_parser.go:188
#[derive(Clone, Copy)]
struct LocationInformation {
    position: i32,
    source_position: i32,
    source_line: i32,
    source_column: i32,
}

// test_parser.go:195
#[derive(Clone, Copy)]
struct RangeLocationInformation {
    location_information: LocationInformation,
    marker: Option<usize>, // index into the file's markers
}

// test_parser.go:200
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TestFileInfo {
    pub(crate) file_name: String,
    // The contents of the file (with markers, etc stripped out)
    pub content: String,
    pub(crate) emit: bool,
    pub(crate) open: bool,
}

impl TestFileInfo {
    // test_parser.go:209
    pub fn file_name(&self) -> String {
        self.file_name.clone()
    }
}

// test_parser.go:208-225: TestFileInfo implements lsconv.Script.
impl Script for TestFileInfo {
    fn file_name(&self) -> &str {
        &self.file_name
    }
    fn original_file_name(&self) -> &str {
        &self.file_name
    }
    fn text(&self) -> &str {
        &self.content
    }
    fn span_map(&self) -> Option<&SpanMap> {
        None
    }
    fn original_text(&self) -> &str {
        &self.content
    }
}

// test_parser.go:229
const EMIT_THIS_FILE_OPTION: &str = "emitthisfile";
const NO_OPEN_FILE_OPTION: &str = "noopen";

// test_parser.go:234
#[derive(Clone, Copy, PartialEq, Eq)]
enum ParserState {
    None,
    InSlashStarMarker,
    InObjectMarker,
}

// utf8.RuneError
const RUNE_ERROR: char = '\u{FFFD}';

// utf8.DecodeRuneInString on valid UTF-8: (RuneError, 0) for the empty string.
fn decode_rune_in_string(s: &str) -> (char, usize) {
    match s.chars().next() {
        Some(c) => (c, c.len_utf8()),
        None => (RUNE_ERROR, 0),
    }
}

// test_parser.go:242
fn parse_file_content(file_name: &str, content: &str, file_options: &OrderedMap<String, String>) -> Result<TestFileWithMarkers, FourslashError> {
    let file_name = tspath::get_normalized_absolute_path(file_name, "/");
    let content = chomp_leading_space(content);
    let content = content.as_str();
    let bytes = content.as_bytes();

    // The file content (minus metacharacters) so far
    let mut output = String::new();

    let mut markers: Vec<PendingMarker> = Vec::new();

    // A stack of the open range markers that are still unclosed
    let mut open_ranges: Vec<RangeLocationInformation> = Vec::new();
    // A list of closed ranges we've collected so far
    let mut range_markers: Vec<(TextRange, Option<usize>)> = Vec::new();

    // The total number of metacharacters removed from the file (so far)
    let mut difference: i32 = 0;

    // One-based current position data
    let mut line: i32 = 1;
    let mut column: i32 = 1;

    // The current marker (or maybe multi-line comment?) we're parsing, possibly
    let mut open_marker: Option<LocationInformation> = None;

    // The latest position of the start of an unflushed plain text area
    let mut last_normal_char_position: usize = 0;

    let flush = |output: &mut String, last_normal_char_position: usize, last_safe_char_index: Option<usize>| {
        if let Some(last_safe_char_index) = last_safe_char_index {
            output.push_str(&content[last_normal_char_position..last_safe_char_index]);
        } else {
            output.push_str(&content[last_normal_char_position..]);
        }
    };

    let mut state = ParserState::None;
    let (mut previous_character, mut i) = decode_rune_in_string(content);
    let mut size: usize;
    let mut current_character: char;
    while i < content.len() {
        (current_character, size) = decode_rune_in_string(&content[i..]);
        let ii = i as i32;
        match state {
            ParserState::None => {
                if previous_character == '[' && current_character == '|' {
                    // found a range start
                    open_ranges.push(RangeLocationInformation {
                        location_information: LocationInformation {
                            position: (ii - 1) - difference,
                            source_position: ii - 1,
                            source_line: line,
                            source_column: column,
                        },
                        marker: None,
                    });
                    // copy all text up to marker position
                    flush(&mut output, last_normal_char_position, Some(i - 1));
                    last_normal_char_position = i + 1;
                    difference += 2;
                } else if previous_character == '|' && current_character == ']' {
                    // found a range end
                    let Some(range_start) = open_ranges.pop() else {
                        return Err(report_error(&file_name, line, column, "Found range end with no matching start."));
                    };

                    range_markers.push((TextRange::new(range_start.location_information.position, (ii - 1) - difference), range_start.marker));

                    // copy all text up to range marker position
                    flush(&mut output, last_normal_char_position, Some(i - 1));
                    last_normal_char_position = i + 1;
                    difference += 2;
                } else if previous_character == '/' && current_character == '*' && (i + 1 >= bytes.len() || bytes[i + 1] != b'/') {
                    // found a possible marker start
                    state = ParserState::InSlashStarMarker;
                    open_marker = Some(LocationInformation {
                        position: (ii - 1) - difference,
                        source_position: ii - 1,
                        source_line: line,
                        source_column: column - 1,
                    });
                } else if previous_character == '{' && current_character == '|' {
                    // found an object marker start
                    state = ParserState::InObjectMarker;
                    open_marker = Some(LocationInformation {
                        position: (ii - 1) - difference,
                        source_position: ii - 1,
                        source_line: line,
                        source_column: column,
                    });
                    flush(&mut output, last_normal_char_position, Some(i - 1));
                }
            }
            ParserState::InObjectMarker => {
                // Object markers are only ever terminated by |} and have no content restrictions
                if previous_character == '|' && current_character == '}' {
                    let om = open_marker.unwrap();
                    let object_marker_data = content[om.source_position as usize + 2..i - 1].trim();
                    let marker = get_object_marker(&file_name, &om, object_marker_data)?;

                    if let Some(last) = open_ranges.last_mut() {
                        last.marker = Some(markers.len());
                    }
                    markers.push(marker);

                    // Set the current start to point to the end of the current marker to ignore its text
                    last_normal_char_position = i + 1;
                    difference += ii + 1 - om.source_position;

                    // Reset the state
                    open_marker = None;
                    state = ParserState::None;
                }
            }
            ParserState::InSlashStarMarker => {
                if previous_character == '*' && current_character == '/' {
                    let om = open_marker.unwrap();
                    // Record the marker
                    // start + 2 to ignore the */, -1 on the end to ignore the * (/ is next)
                    let marker_name_text = content[om.source_position as usize + 2..i - 1].trim().to_string();
                    let marker = PendingMarker { position: om.position, name: Some(marker_name_text), data: OrderedMap::default() };
                    if let Some(last) = open_ranges.last_mut() {
                        last.marker = Some(markers.len());
                    }
                    markers.push(marker);

                    // Set the current start to point to the end of the current marker to ignore its text
                    flush(&mut output, last_normal_char_position, Some(om.source_position as usize));
                    last_normal_char_position = i + 1;
                    difference += ii + 1 - om.source_position;

                    // Reset the state
                    open_marker = None;
                    state = ParserState::None;
                } else if !(stringutil::is_digit(current_character)
                    || stringutil::is_ascii_letter(current_character)
                    || current_character == '$'
                    || current_character == '_')
                {
                    // Invalid marker character
                    if current_character == '*' && i < bytes.len() - 1 && bytes[i + 1] == b'/' {
                        // The marker is about to be closed, ignore the 'invalid' char
                    } else {
                        // We've hit a non-valid marker character, so we were actually in a block comment
                        // Bail out the text we've gathered so far back into the output
                        flush(&mut output, last_normal_char_position, Some(i));
                        last_normal_char_position = i;
                        open_marker = None;
                        state = ParserState::None;
                    }
                }
            }
        }
        if current_character == '\n' && previous_character == '\r' {
            // Ignore trailing \n after \r
            i += size;
            continue;
        } else if current_character == '\n' || current_character == '\r' {
            line += 1;
            column = 1;
            i += size;
            continue;
        }
        column += 1;
        if i >= last_normal_char_position {
            previous_character = current_character;
        } else {
            previous_character = RUNE_ERROR; // reset to avoid accidentally reusing marker delimiters as part of other markers
        }
        i += size;
    }

    // Add the remaining text
    flush(&mut output, last_normal_char_position, None);

    if let Some(open_range) = open_ranges.first() {
        return Err(report_error(&file_name, open_range.location_information.source_line, open_range.location_information.source_column, "Unterminated range."));
    }

    if let Some(om) = open_marker {
        return Err(report_error(&file_name, om.source_line, om.source_column, "Unterminated marker."));
    }

    let output_string = output;
    // Set LS positions for markers
    let line_map = lsconv::compute_lsp_line_starts(&output_string);
    let converters = new_test_converters(lsconv::new_converters(lsproto::PositionEncodingKind::UTF8, move |_| Some(line_map.clone())));

    let emit = file_options.get(EMIT_THIS_FILE_OPTION).map(|s| s.as_str()) == Some("true");

    let test_file_info = TestFileInfo {
        file_name: file_name.clone(),
        content: output_string,
        emit,
        open: file_options.get(NO_OPEN_FILE_OPTION).map(|s| s.as_str()) != Some("true"),
    };

    // slices.SortStableFunc: by start, then the longer range first
    range_markers.sort_by(|a, b| {
        if a.0.pos() != b.0.pos() {
            return a.0.pos().cmp(&b.0.pos());
        }
        b.0.end().cmp(&a.0.end())
    });

    let markers: Vec<Arc<Marker>> = markers
        .into_iter()
        .map(|m| {
            Arc::new(Marker {
                file_name: file_name.clone(),
                position: m.position,
                ls_position: converters.position_to_line_and_character(&test_file_info, m.position),
                name: m.name,
                data: m.data,
            })
        })
        .collect();
    let ranges: Vec<Arc<RangeMarker>> = range_markers
        .into_iter()
        .map(|(range, marker)| {
            Arc::new(RangeMarker {
                file_name: file_name.clone(),
                range,
                ls_range: lsproto::Range {
                    start: converters.position_to_line_and_character(&test_file_info, range.pos()),
                    end: converters.position_to_line_and_character(&test_file_info, range.end()),
                },
                marker: marker.map(|i| markers[i].clone()),
            })
        })
        .collect();

    Ok(TestFileWithMarkers { file: Arc::new(test_file_info), markers, ranges })
}

// test_parser.go:466
fn get_object_marker(file_name: &str, location: &LocationInformation, text: &str) -> Result<PendingMarker, FourslashError> {
    // Attempt to parse the marker value as JSON
    let v = tsrs_core::json::unmarshal(&format!("{{ {text} }}"));

    let Ok(v) = v else {
        return Err(report_error(file_name, location.source_line, location.source_column, &format!("Unable to parse marker text {text}")));
    };
    let marker_value = match Any::from_json(&v) {
        Any::Map(m) if !m.is_empty() => m,
        _ => return Err(report_error(file_name, location.source_line, location.source_column, "Object markers can not be empty")),
    };

    let mut marker = PendingMarker { position: location.position, name: None, data: marker_value };

    // Object markers can be anonymous
    if let Some(Any::String(name)) = marker.data.get("name") {
        if !name.is_empty() {
            marker.name = Some(name.clone());
        }
    }

    Ok(marker)
}

// A *Marker under construction (its LSPosition is computed after the whole file is scanned).
struct PendingMarker {
    position: i32,
    name: Option<String>,
    data: OrderedMap<String, Any>,
}

// test_parser.go:495
fn report_error(file_name: &str, line: i32, col: i32, message: &str) -> FourslashError {
    FourslashError { err: format!("{file_name} ({line},{col}): {message}") }
}

// test_parser.go:499
fn chomp_leading_space(content: &str) -> String {
    let lines: Vec<&str> = content.split('\n').collect();
    for line in &lines {
        if !line.is_empty() && line.as_bytes()[0] != b' ' {
            return content.to_string();
        }
    }

    let mut result = Vec::with_capacity(lines.len());
    for line in &lines {
        if !line.is_empty() {
            result.push(&line[1..]);
        } else {
            result.push("");
        }
    }
    result.join("\n")
}

// test_parser.go:516
#[derive(Debug)]
pub struct FourslashError {
    err: String,
}

impl FourslashError {
    // test_parser.go:520
    pub fn error(&self) -> String {
        self.err.clone()
    }
}
