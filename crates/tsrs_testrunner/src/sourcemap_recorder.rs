// Go testutil/harnessutil/sourcemap_recorder.go: the span recorder behind the `.sourcemap.txt` baselines, and
// `CompilationResult.GetSourceMapRecord` (harnessutil.go:915).

use std::fmt::Write as _;
use tsrs_core::json::{self, Value};
use tsrs_core::{compute_ecma_line_starts, stringutil, TextPos};
use tsrs_sourcemap::{self as sourcemap, Mapping, RawSourceMap};

use crate::harnessutil::TestFile;

// Go builds the record in a strings.Builder, which holds bytes; positions below are byte offsets, as in Go.
#[derive(Default)]
pub(crate) struct writerAggregator {
    buf: Vec<u8>,
}

impl writerAggregator {
    fn write_string(&mut self, s: &str) {
        self.buf.extend_from_slice(s.as_bytes());
    }

    fn write_bytes(&mut self, s: &[u8]) {
        self.buf.extend_from_slice(s);
    }

    fn write_line(&mut self, s: &str) {
        self.write_string(s);
        self.write_string("\r\n");
    }

    pub(crate) fn string(self) -> String {
        String::from_utf8_lossy(&self.buf).into_owned()
    }
}

#[derive(Clone)]
struct sourceMapSpanWithDecodeErrors {
    source_map_span: Mapping,
    decode_errors: Vec<String>,
}

struct decodedMapping {
    source_map_span: Mapping,
    error: Option<String>,
}

struct sourceMapDecoder {
    source_map_mappings: String,
    mappings: sourcemap::MappingsDecoder,
}

fn new_source_map_decoder(source_map: &RawSourceMap) -> sourceMapDecoder {
    sourceMapDecoder { source_map_mappings: source_map.mappings.clone(), mappings: sourcemap::decode_mappings(&source_map.mappings) }
}

impl sourceMapDecoder {
    fn decode_next_encoded_source_map_span(&mut self) -> decodedMapping {
        match self.mappings.next() {
            None => {
                let mut mapping = decodedMapping { error: self.mappings.error().map(str::to_string), source_map_span: self.mappings.state() };
                if mapping.error.is_none() {
                    mapping.error = Some("No encoded entry found".to_string());
                }
                mapping
            }
            Some(value) => decodedMapping { source_map_span: value, error: None },
        }
    }

    fn has_completed_decoding(&self) -> bool {
        self.mappings.pos() == self.source_map_mappings.len()
    }

    fn get_remaining_decode_string(&self) -> &str {
        &self.source_map_mappings[self.mappings.pos()..]
    }
}

pub(crate) struct sourceMapSpanWriter<'a> {
    source_map_recorder: &'a mut writerAggregator,
    source_map_sources: Vec<String>,
    source_map_names: Vec<String>,
    js_file: TestFile,
    js_line_map: Vec<TextPos>,
    ts_code: String,
    ts_line_map: Vec<TextPos>,
    spans_on_single_line: Vec<sourceMapSpanWithDecodeErrors>,
    prev_written_source_pos: TextPos,
    next_js_line_to_write: i32,
    span_marker_continues: bool,
    source_map_decoder: sourceMapDecoder,
}

pub(crate) fn new_source_map_span_writer<'a>(source_map_recorder: &'a mut writerAggregator, source_map: &RawSourceMap, js_file: TestFile) -> sourceMapSpanWriter<'a> {
    let js_line_map = compute_ecma_line_starts(&js_file.content);
    let line_info = sourcemap::create_ecma_line_info(js_file.content.clone(), js_line_map.clone());
    source_map_recorder.write_line("===================================================================");
    source_map_recorder.write_line(&format!("JsFile: {}", source_map.file));
    source_map_recorder.write_line(&format!("mapUrl: {}", sourcemap::try_get_source_mapping_url(Some(&line_info))));
    source_map_recorder.write_line(&format!("sourceRoot: {}", source_map.source_root));
    source_map_recorder.write_line(&format!("sources: {}", source_map.sources.join(",")));
    if let Some(sources_content) = &source_map.sources_content {
        if !sources_content.is_empty() {
            let value = Value::Array(
                sources_content
                    .iter()
                    .map(|s| match s {
                        Some(s) => Value::String(s.clone()),
                        None => Value::Null,
                    })
                    .collect(),
            );
            let content = match json::marshal(&value) {
                Ok(content) => content,
                Err(err) => panic!("{}", err),
            };
            source_map_recorder.write_line(&format!("sourcesContent: {}", content));
        }
    }
    source_map_recorder.write_line("===================================================================");
    sourceMapSpanWriter {
        source_map_recorder,
        source_map_sources: source_map.sources.clone(),
        source_map_names: source_map.names.clone(),
        js_file,
        js_line_map,
        ts_code: String::new(),
        ts_line_map: Vec::new(),
        spans_on_single_line: Vec::new(),
        prev_written_source_pos: 0,
        next_js_line_to_write: 0,
        span_marker_continues: false,
        source_map_decoder: new_source_map_decoder(source_map),
    }
}

impl sourceMapSpanWriter<'_> {
    fn get_source_map_span_string(&self, map_entry: &Mapping, get_absent_name_index: bool) -> String {
        let mut map_string = format!("Emitted({}, {})", map_entry.generated_line + 1, map_entry.generated_character + 1);
        if map_entry.is_source_mapping() {
            let _ = write!(map_string, " Source({}, {}) + SourceIndex({})", map_entry.source_line + 1, map_entry.source_character + 1, map_entry.source_index);
            if map_entry.name_index >= 0 && (map_entry.name_index as usize) < self.source_map_names.len() {
                let _ = write!(map_string, " name ({})", self.source_map_names[map_entry.name_index as usize]);
            } else if map_entry.name_index != sourcemap::MISSING_NAME || get_absent_name_index {
                let _ = write!(map_string, " nameIndex ({})", map_entry.name_index);
            }
        }
        map_string
    }

    pub(crate) fn record_source_map_span(&mut self, source_map_span: Mapping) {
        // verify the decoded span is same as the new span
        let decode_result = self.source_map_decoder.decode_next_encoded_source_map_span();
        let mut decode_errors = Vec::new();
        if decode_result.error.is_some() || !decode_result.source_map_span.equals(&source_map_span) {
            if let Some(error) = &decode_result.error {
                decode_errors = vec![format!("!!^^ !!^^ There was decoding error in the sourcemap at this location: {}", error)];
            } else {
                decode_errors = vec!["!!^^ !!^^ The decoded span from sourcemap's mapping entry does not match what was encoded for this span:".to_string()];
            }
            decode_errors.push(format!(
                "!!^^ !!^^ Decoded span from sourcemap's mappings entry: {} Span encoded by the emitter:{}",
                self.get_source_map_span_string(&decode_result.source_map_span, true /*getAbsentNameIndex*/),
                self.get_source_map_span_string(&source_map_span, true /*getAbsentNameIndex*/),
            ));
        }

        if !self.spans_on_single_line.is_empty() && self.spans_on_single_line[0].source_map_span.generated_line != source_map_span.generated_line {
            // On different line from the one that we have been recording till now,
            self.write_recorded_spans();
            self.spans_on_single_line = Vec::new();
        }
        self.spans_on_single_line.push(sourceMapSpanWithDecodeErrors { source_map_span, decode_errors });
    }

    pub(crate) fn record_new_source_file_span(&mut self, source_map_span: Mapping, new_source_file_code: &str) {
        let mut continues_line = false;
        if !self.spans_on_single_line.is_empty()
            && u32::try_from(source_map_span.generated_line).ok() == Some(self.spans_on_single_line[0].source_map_span.generated_character)
        {
            // !!! char == line seems like a bug in Strada?
            self.write_recorded_spans();
            self.spans_on_single_line = Vec::new();
            self.next_js_line_to_write -= 1; // walk back one line to reprint the line
            continues_line = true;
        }

        self.record_source_map_span(source_map_span);

        if self.spans_on_single_line.len() != 1 {
            panic!("expected a single span");
        }

        self.source_map_recorder.write_line("-------------------------------------------------------------------");
        if continues_line {
            let line = format!("emittedFile:{} ({}, {})", self.js_file.unit_name, source_map_span.generated_line + 1, source_map_span.generated_character + 1);
            self.source_map_recorder.write_line(&line);
        } else {
            let line = format!("emittedFile:{}", self.js_file.unit_name);
            self.source_map_recorder.write_line(&line);
        }
        let line = format!("sourceFile:{}", self.source_map_sources[self.spans_on_single_line[0].source_map_span.source_index as usize]);
        self.source_map_recorder.write_line(&line);
        self.source_map_recorder.write_line("-------------------------------------------------------------------");

        self.ts_line_map = compute_ecma_line_starts(new_source_file_code);
        self.ts_code = new_source_file_code.to_string();
        self.prev_written_source_pos = 0;
    }

    pub(crate) fn close(&mut self) {
        // Write the lines pending on the single line
        self.write_recorded_spans();

        if !self.source_map_decoder.has_completed_decoding() {
            self.source_map_recorder.write_line("!!!! **** There are more source map entries in the sourceMap's mapping than what was encoded");
            let line = format!("!!!! **** Remaining decoded string: {}", self.source_map_decoder.get_remaining_decode_string());
            self.source_map_recorder.write_line(&line);
        }

        // write remaining js lines
        self.write_js_file_lines(self.js_line_map.len() as i32);
    }

    fn get_text_of_line<'c>(line: i32, line_map: &[TextPos], code: &'c str) -> &'c [u8] {
        let start_pos = line_map[line as usize];
        let end_pos: TextPos = if ((line + 1) as usize) < line_map.len() { line_map[line as usize + 1] } else { code.len() as TextPos };
        let text = &code.as_bytes()[start_pos as usize..end_pos as usize];
        if line == 0 {
            return remove_byte_order_mark(text);
        }
        // return line == 0 ? Utils.removeByteOrderMark(text) : text;
        text
    }

    fn write_js_file_lines(&mut self, end_js_line: i32) {
        while self.next_js_line_to_write < end_js_line {
            let text = Self::get_text_of_line(self.next_js_line_to_write, &self.js_line_map, &self.js_file.content);
            self.source_map_recorder.write_string(">>>");
            self.source_map_recorder.write_bytes(text);
            self.next_js_line_to_write += 1;
        }
    }

    fn write_recorded_spans(&mut self) {
        let mut recorded_span_writer = recordedSpanWriter { marker_ids: Vec::new(), prev_emitted_col: 0, w: self };
        recorded_span_writer.write_recorded_spans();
    }
}

fn remove_byte_order_mark(text: &[u8]) -> &[u8] {
    match std::str::from_utf8(text) {
        Ok(s) => stringutil::remove_byte_order_mark(s).as_bytes(),
        Err(_) => text.strip_prefix("\u{FEFF}".as_bytes()).unwrap_or(text),
    }
}

struct recordedSpanWriter<'w, 'a> {
    marker_ids: Vec<String>,
    prev_emitted_col: u32,
    w: &'w mut sourceMapSpanWriter<'a>,
}

impl recordedSpanWriter<'_, '_> {
    fn get_marker_id(&self, marker_index: usize) -> String {
        let mut marker_id;
        if self.w.span_marker_continues {
            if marker_index != 0 {
                panic!("expected markerIndex to be 0");
            }
            marker_id = "1->".to_string();
        } else {
            marker_id = (marker_index + 1).to_string();
            if marker_id.len() < 2 {
                marker_id.push(' ');
            }
            marker_id.push('>');
        }
        marker_id
    }

    fn iterate_spans(&mut self, f: fn(&mut Self, &sourceMapSpanWithDecodeErrors, usize)) {
        self.prev_emitted_col = 0;
        for i in 0..self.w.spans_on_single_line.len() {
            let span = self.w.spans_on_single_line[i].clone();
            f(self, &span, i);
            self.prev_emitted_col = self.w.spans_on_single_line[i].source_map_span.generated_character;
        }
    }

    fn write_source_map_indent(&mut self, indent_length: u32, indent_prefix: &str) {
        self.w.source_map_recorder.write_string(indent_prefix);
        for _ in 0..indent_length {
            self.w.source_map_recorder.write_string(" ");
        }
    }

    fn write_source_map_marker(&mut self, current_span: &sourceMapSpanWithDecodeErrors, index: usize) {
        self.write_source_map_marker_ex(Some(current_span), index, current_span.source_map_span.generated_character, false /*endContinues*/);
    }

    fn write_source_map_marker_ex(&mut self, _current_span: Option<&sourceMapSpanWithDecodeErrors>, index: usize, end_column: u32, end_continues: bool) {
        let marker_id = self.get_marker_id(index);
        self.marker_ids.push(marker_id.clone());
        self.write_source_map_indent(self.prev_emitted_col, &marker_id);
        for _ in self.prev_emitted_col..end_column {
            self.w.source_map_recorder.write_string("^");
        }
        if end_continues {
            self.w.source_map_recorder.write_string("->");
        }
        self.w.source_map_recorder.write_line("");
        self.w.span_marker_continues = end_continues;
    }

    fn write_source_map_source_text(&mut self, current_span: &sourceMapSpanWithDecodeErrors, index: usize) {
        // Convert UTF-16 character offset from the source map to a byte position.
        let source_pos = tsrs_scanner::compute_position_of_line_and_utf16_character(
            &self.w.ts_line_map,
            u32::try_from(current_span.source_map_span.source_line).expect("source-map line is nonnegative"),
            current_span.source_map_span.source_character,
            &self.w.ts_code,
            true, /*allowEdits*/
        );
        let mut source_text: &[u8] = &[];
        if self.w.prev_written_source_pos < source_pos {
            // Position that goes forward, get text
            source_text = &self.w.ts_code.as_bytes()[self.w.prev_written_source_pos as usize..source_pos as usize];
        }
        let source_text = source_text.to_vec();

        // If there are decode errors, write
        for decode_error in &current_span.decode_errors {
            let marker_id = self.marker_ids[index].clone();
            self.write_source_map_indent(self.prev_emitted_col, &marker_id);
            self.w.source_map_recorder.write_line(decode_error);
        }

        // Go's ComputeECMALineStarts and slicing work on the bytes of the string.
        let source_text_str = bytes_to_string(&source_text);
        let ts_code_line_map = compute_ecma_line_starts(&source_text_str);
        for i in 0..ts_code_line_map.len() {
            if i == 0 {
                let marker_id = self.marker_ids[index].clone();
                self.write_source_map_indent(self.prev_emitted_col, &marker_id);
            } else {
                self.write_source_map_indent(self.prev_emitted_col, "  >");
            }
            let text = sourceMapSpanWriter::get_text_of_line(i as i32, &ts_code_line_map, &source_text_str).to_vec();
            self.w.source_map_recorder.write_bytes(&text);
            if i == ts_code_line_map.len() - 1 {
                self.w.source_map_recorder.write_line("");
            }
        }

        self.w.prev_written_source_pos = source_pos;
    }

    fn write_span_details(&mut self, current_span: &sourceMapSpanWithDecodeErrors, index: usize) {
        let line = format!("{}{}", self.marker_ids[index], self.w.get_source_map_span_string(&current_span.source_map_span, false /*getAbsentNameIndex*/));
        self.w.source_map_recorder.write_line(&line);
    }

    fn write_recorded_spans(&mut self) {
        if !self.w.spans_on_single_line.is_empty() {
            let current_js_line = self.w.spans_on_single_line[0].source_map_span.generated_line;

            // Write js line
            self.w.write_js_file_lines(current_js_line + 1);

            // Emit markers
            self.iterate_spans(Self::write_source_map_marker);

            let js_file_text_len = u32::try_from(sourceMapSpanWriter::get_text_of_line(current_js_line + 1, &self.w.js_line_map, &self.w.js_file.content).len())
                .expect("source-map line exceeds u32"); // TODO: Strada is wrong here, we should be looking at `currentJsLine`, not `currentJsLine+1`
            if let Some(end_column) = js_file_text_len.checked_sub(1).filter(|&end_column| self.prev_emitted_col < end_column) {
                // There is remaining text on this line that will be part of next source span so write marker that continues
                let n = self.w.spans_on_single_line.len();
                self.write_source_map_marker_ex(None /*currentSpan*/, n, end_column, true /*endContinues*/);
            }

            // Emit Source text
            self.iterate_spans(Self::write_source_map_source_text);

            // Emit column number etc
            self.iterate_spans(Self::write_span_details);

            self.w.source_map_recorder.write_line("---");
        }
    }
}

// The source text slice is cut at a byte offset computed from UTF-16 positions, which is always a character
// boundary for well-formed input; fall back to a lossy copy otherwise (Go would keep the raw bytes).
fn bytes_to_string(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

// harnessutil.go:931 (`CompilationResult.GetSourceMapRecord`)
pub(crate) struct SourceMapRecordInput<'a> {
    pub(crate) generated_file: &'a str,
    pub(crate) input_source_file_names: &'a [String],
    pub(crate) source_map: &'a RawSourceMap,
}

// `get_output` is `c.DTS.GetOrZero` / `c.JS.GetOrZero` (Go picks by `IsDeclarationFileName`, done here), and
// `get_source_file` is `c.Program.GetSourceFile(name)` returning the file's identity and `OriginalText()`.
pub(crate) fn get_source_map_record(
    source_maps: &[SourceMapRecordInput],
    get_js: impl Fn(&str) -> Option<TestFile>,
    get_dts: impl Fn(&str) -> Option<TestFile>,
    get_source_file: impl Fn(&str) -> Option<(usize, String)>,
) -> String {
    if source_maps.is_empty() {
        return String::new();
    }

    let mut source_map_recorder = writerAggregator::default();
    for source_map_data in source_maps {
        let mut prev_source_file: Option<usize> = None;
        let current_file = if tsrs_core::tspath::is_declaration_file_name(source_map_data.generated_file) {
            get_dts(source_map_data.generated_file)
        } else {
            get_js(source_map_data.generated_file)
        }
        .unwrap_or(TestFile { unit_name: String::new(), content: String::new() });

        let mut source_map_span_writer = new_source_map_span_writer(&mut source_map_recorder, source_map_data.source_map, current_file);
        let mut mapper = sourcemap::decode_mappings(&source_map_data.source_map.mappings);
        for decoded_source_mapping in mapper.values() {
            if !decoded_source_mapping.is_source_mapping() {
                source_map_span_writer.record_source_map_span(decoded_source_mapping);
                continue;
            }
            let current_source_file = get_source_file(&source_map_data.input_source_file_names[decoded_source_mapping.source_index as usize]);
            let current_key = current_source_file.as_ref().map(|(k, _)| *k);
            if current_key != prev_source_file {
                if let Some((_, text)) = &current_source_file {
                    source_map_span_writer.record_new_source_file_span(decoded_source_mapping, text);
                }
                prev_source_file = current_key;
            } else {
                source_map_span_writer.record_source_map_span(decoded_source_mapping);
            }
        }
        source_map_span_writer.close();
    }
    source_map_recorder.string()
}
