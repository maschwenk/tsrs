// Port of Go's fourslash/baselineutil.go: baseline commands, file names and the text formatting of location
// baselines.

use std::fmt::Write as _;
use std::sync::{Arc, LazyLock};

use regex::Regex;
use rustc_hash::FxHashMap;
use tsrs_core::collections::{group_by, MultiMap, OrderedMap};
use tsrs_core::stringutil;
use tsrs_ls::lsconv::{self, LSPLineMap, Script};
use tsrs_ls::spanmap::SpanMap;
use tsrs_lsproto as lsproto;

use crate::fourslash::{new_test_converters, FourslashTest, Marker, MarkerOrRange, TestConverters};
use crate::testing::T;
use crate::testutil::baseline;

// baselineutil.go:46
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BaselineCommand(pub &'static str);

// baselineutil.go:23
pub(crate) const AUTO_IMPORTS_CMD: BaselineCommand = BaselineCommand("Auto Imports");
pub(crate) const CALL_HIERARCHY_CMD: BaselineCommand = BaselineCommand("Call Hierarchy");
pub(crate) const CLOSING_TAG_CMD: BaselineCommand = BaselineCommand("Closing Tag");
pub(crate) const DOCUMENT_HIGHLIGHTS_CMD: BaselineCommand = BaselineCommand("documentHighlights");
pub(crate) const FIND_ALL_REFERENCES_CMD: BaselineCommand = BaselineCommand("findAllReferences");
pub(crate) const VS_FIND_ALL_REFERENCES_CMD: BaselineCommand = BaselineCommand("vsFindAllReferences");
pub(crate) const GO_TO_DEFINITION_CMD: BaselineCommand = BaselineCommand("goToDefinition");
pub(crate) const GO_TO_IMPLEMENTATION_CMD: BaselineCommand = BaselineCommand("goToImplementation");
pub(crate) const GO_TO_SOURCE_DEFINITION_CMD: BaselineCommand = BaselineCommand("goToSourceDefinition");
pub(crate) const GO_TO_TYPE_DEFINITION_CMD: BaselineCommand = BaselineCommand("goToType");
pub(crate) const INLAY_HINTS_CMD: BaselineCommand = BaselineCommand("Inlay Hints");
pub(crate) const NON_SUGGESTION_DIAGNOSTICS_CMD: BaselineCommand = BaselineCommand("Syntax and Semantic Diagnostics");
pub(crate) const QUICK_INFO_CMD: BaselineCommand = BaselineCommand("QuickInfo");
pub(crate) const VS_QUICK_INFO_CMD: BaselineCommand = BaselineCommand("VSQuickInfo");
pub(crate) const LINKED_EDITING_CMD: BaselineCommand = BaselineCommand("linkedEditing");
pub(crate) const RENAME_CMD: BaselineCommand = BaselineCommand("findRenameLocations");
pub(crate) const SIGNATURE_HELP_CMD: BaselineCommand = BaselineCommand("SignatureHelp");
pub(crate) const SMART_SELECTION_CMD: BaselineCommand = BaselineCommand("Smart Selection");
pub(crate) const CODE_LENSES_CMD: BaselineCommand = BaselineCommand("Code Lenses");
pub(crate) const DOCUMENT_SYMBOLS_CMD: BaselineCommand = BaselineCommand("Document Symbols");

impl FourslashTest {
    // baselineutil.go:48
    pub(crate) fn add_result_to_baseline(&mut self, _t: &T, command: BaselineCommand, actual: &str) {
        let b: &mut String = if self.test_data.is_state_baselining_enabled() {
            // Single baseline for all commands
            &mut self.state_baseline.as_mut().unwrap().baseline
        } else {
            self.baselines.entry(command).or_default()
        };
        if !b.is_empty() {
            b.push_str("\n\n\n\n");
        }
        b.push_str("// === ");
        b.push_str(command.0);
        b.push_str(" ===\n");
        b.push_str(actual);
    }

    // baselineutil.go:68
    pub(crate) fn write_to_baseline(&mut self, command: BaselineCommand, content: &str) {
        self.baselines.entry(command).or_default().push_str(content);
    }

    // baselineutil.go:96
    pub(crate) fn get_baseline_options(&self, command: BaselineCommand, _test_path: &str) -> baseline::Options {
        baseline::Options { subfolder: format!("fourslash/{}", normalize_command_name(command.0)), ..Default::default() }
    }
}

// baselineutil.go:77
pub(crate) fn get_baseline_file_name(t: &T, command: BaselineCommand) -> String {
    format!("{}.{}", crate::fourslash::get_base_file_name_from_test(t), get_baseline_extension(command))
}

// baselineutil.go:81
pub(crate) fn get_baseline_extension(command: BaselineCommand) -> &'static str {
    match command {
        QUICK_INFO_CMD | VS_QUICK_INFO_CMD | SIGNATURE_HELP_CMD | SMART_SELECTION_CMD | INLAY_HINTS_CMD | NON_SUGGESTION_DIAGNOSTICS_CMD
        | DOCUMENT_SYMBOLS_CMD | CLOSING_TAG_CMD | VS_FIND_ALL_REFERENCES_CMD => "baseline",
        CALL_HIERARCHY_CMD => "callHierarchy.txt",
        AUTO_IMPORTS_CMD => "baseline.md",
        LINKED_EDITING_CMD => "linkedEditing.txt",
        _ => "baseline.jsonc",
    }
}

// baselineutil.go:102
pub(crate) fn drop_trailing_empty_lines(ss: &[String]) -> &[String] {
    let last = ss.iter().rposition(|s| !s.is_empty()).map(|i| i as i32).unwrap_or(-1);
    &ss[..(last + 1) as usize]
}

// baselineutil.go:106
pub(crate) fn normalize_command_name(command: &str) -> String {
    let words: Vec<&str> = command.split_whitespace().collect();
    let command = words.join("");
    stringutil::lower_first_char(&command)
}

// baselineutil.go:112
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct DocumentSpan {
    pub(crate) uri: lsproto::DocumentUri,
    pub(crate) text_span: lsproto::Range,
    pub(crate) context_span: Option<lsproto::Range>,
}

// baselineutil.go:118
#[derive(Default)]
pub(crate) struct BaselineFourslashLocationsOptions {
    // markerInfo
    pub(crate) marker: Option<MarkerOrRange>, // location
    pub(crate) marker_name: String,          // name of the marker to be printed in baseline

    pub(crate) end_marker: String,

    pub(crate) start_marker_prefix: Option<Box<dyn Fn(&DocumentSpan) -> Option<String>>>,
    pub(crate) end_marker_suffix: Option<Box<dyn Fn(&DocumentSpan) -> Option<String>>>,
    pub(crate) get_location_data: Option<Box<dyn Fn(&DocumentSpan) -> String>>,

    pub(crate) additional_span: Option<DocumentSpan>,
    pub(crate) preserve_result_order: bool,
    pub(crate) ordered_files: Vec<lsproto::DocumentUri>,
}

// baselineutil.go:134
pub(crate) fn location_to_span(loc: &lsproto::Location) -> DocumentSpan {
    DocumentSpan { uri: loc.uri.clone(), text_span: loc.range, context_span: None }
}

impl FourslashTest {
    // baselineutil.go:141
    pub(crate) fn get_baseline_for_locations_with_file_contents(&self, locations: &[lsproto::Location], options: BaselineFourslashLocationsOptions) -> String {
        self.get_baseline_for_spans_with_file_contents(&locations.iter().map(location_to_span).collect::<Vec<_>>(), options)
    }

    // baselineutil.go:148
    pub(crate) fn get_baseline_for_spans_with_file_contents(&self, spans: &[DocumentSpan], mut options: BaselineFourslashLocationsOptions) -> String {
        let spans_by_file = group_by(spans, |span: &DocumentSpan| span.uri.clone());
        if options.preserve_result_order {
            options.ordered_files = unique_files_in_span_order(spans);
        }
        self.get_baseline_for_grouped_spans_with_file_contents(&spans_by_file, &options)
    }

    // baselineutil.go:159
    pub(crate) fn get_baseline_for_grouped_spans_with_file_contents(
        &self,
        grouped_ranges: &MultiMap<lsproto::DocumentUri, DocumentSpan>,
        options: &BaselineFourslashLocationsOptions,
    ) -> String {
        // We must always print the file containing the marker,
        // but don't want to print it twice at the end if it already
        // found in a file with ranges.
        let mut found_marker = false;
        let mut found_additional_location = false;
        let mut span_to_context_id: FxHashMap<DocumentSpan, i32> = FxHashMap::default();

        let mut baseline_entries: Vec<String> = Vec::new();
        let mut add_file_entry = |path: &str, found_marker: &mut bool, found_additional_location: &mut bool, baseline_entries: &mut Vec<String>| {
            let file_name = lsconv::file_name_to_document_uri(path);
            let ranges = grouped_ranges.get(&file_name);
            if ranges.is_empty() {
                return;
            }

            let Some(content) = self.text_of_file(path) else {
                return;
            };

            if let Some(marker) = &options.marker {
                if marker.file_name() == path {
                    *found_marker = true;
                }
            }

            if let Some(additional_span) = &options.additional_span {
                if additional_span.uri == file_name {
                    *found_additional_location = true;
                }
            }

            baseline_entries.push(self.get_baseline_content_for_file(path, &content, ranges, &mut span_to_context_id, options));
        };
        if options.preserve_result_order {
            for uri in &options.ordered_files {
                add_file_entry(&uri.file_name(), &mut found_marker, &mut found_additional_location, &mut baseline_entries);
            }
        } else {
            for path in get_accessible_file_paths(&*self.vfs, "/") {
                add_file_entry(&path, &mut found_marker, &mut found_additional_location, &mut baseline_entries);
            }
            for path in get_accessible_file_paths(&*self.vfs, &tsrs_vfs::bundled::lib_path()) {
                add_file_entry(&path, &mut found_marker, &mut found_additional_location, &mut baseline_entries);
            }
        }
        drop(add_file_entry);

        // In Strada, there is a bug where we only ever add additional spans to baselines if we haven't
        // already added the file to the baseline.
        if let Some(additional_span) = &options.additional_span {
            if !found_additional_location {
                let file_name = additional_span.uri.file_name();
                if let Some(content) = self.text_of_file(&file_name) {
                    baseline_entries.push(self.get_baseline_content_for_file(
                        &file_name,
                        &content,
                        std::slice::from_ref(additional_span),
                        &mut span_to_context_id,
                        options,
                    ));
                    if let Some(marker) = &options.marker {
                        if marker.file_name() == file_name {
                            found_marker = true;
                        }
                    }
                }
            }
        }

        if !found_marker {
            if let Some(marker) = &options.marker {
                // If we didn't find the marker in any file, we need to add it.
                let marker_file_name = marker.file_name();
                if let Some(content) = self.text_of_file(&marker_file_name) {
                    baseline_entries.push(self.get_baseline_content_for_file(&marker_file_name, &content, &[], &mut span_to_context_id, options));
                }
            }
        }

        // !!! skipDocumentContainingOnlyMarker

        baseline_entries.join("\n\n")
    }

    // baselineutil.go:267
    pub(crate) fn text_of_file(&self, file_name: &str) -> Option<String> {
        if self.open_files.contains_key(file_name) {
            return Some(self.get_script_info(file_name).content);
        }
        self.vfs.read_file(file_name)
    }
}

// baselineutil.go:231
pub(crate) fn get_accessible_file_paths(file_system: &dyn tsrs_vfs::FS, root: &str) -> Vec<String> {
    if !file_system.directory_exists(root) {
        return Vec::new();
    }
    let mut files = Vec::new();
    let err = tsrs_vfs::walk_dir(file_system, root, &mut |path, entry, err| {
        if let Some(err) = err {
            return Err(tsrs_vfs::WalkDirError::Err(err.clone()));
        }
        if entry.is_some_and(|e| e.type_().is_regular()) {
            files.push(path.to_string());
        }
        Ok(())
    });
    if let Err(err) = err {
        panic!("walkdir error during fourslash baseline: {err:?}");
    }
    files
}

// baselineutil.go:251
pub(crate) fn unique_files_in_span_order(spans: &[DocumentSpan]) -> Vec<lsproto::DocumentUri> {
    if spans.is_empty() {
        return Vec::new();
    }
    let mut seen: FxHashMap<lsproto::DocumentUri, ()> = FxHashMap::default();
    let mut result = Vec::with_capacity(spans.len());
    for span in spans {
        if seen.contains_key(&span.uri) {
            continue;
        }
        seen.insert(span.uri.clone(), ());
        result.push(span.uri.clone());
    }
    result
}

// baselineutil.go:274
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum DetailKind {
    Marker,       // /*MARKER*/
    ContextStart, // <|
    TextStart,    // [|
    TextEnd,      // |]
    ContextEnd,   // |>
}

impl DetailKind {
    // baselineutil.go:284
    pub(crate) fn is_end(self) -> bool {
        self == DetailKind::ContextEnd || self == DetailKind::TextEnd
    }

    // baselineutil.go:288
    pub(crate) fn is_start(self) -> bool {
        self == DetailKind::ContextStart || self == DetailKind::TextStart
    }
}

// baselineutil.go:292 (`span` is Go's *documentSpan: index of the span in spansInFile, compared by identity)
#[derive(Clone, Debug)]
pub(crate) struct BaselineDetail {
    pub(crate) pos: lsproto::Position,
    pub(crate) position_marker: String,
    pub(crate) span: Option<usize>,
    pub(crate) kind: DetailKind,
}

impl BaselineDetail {
    // baselineutil.go:299
    pub(crate) fn get_range(&self, spans: &[DocumentSpan]) -> lsproto::Range {
        match self.kind {
            DetailKind::ContextStart | DetailKind::ContextEnd => spans[self.span.unwrap()].context_span.unwrap(),
            DetailKind::TextStart | DetailKind::TextEnd => spans[self.span.unwrap()].text_span,
            DetailKind::Marker => lsproto::Range { start: self.pos, end: self.pos },
        }
    }
}

impl FourslashTest {
    // baselineutil.go:319
    pub(crate) fn get_baseline_content_for_file(
        &self,
        file_name: &str,
        content: &str,
        spans_in_file: &[DocumentSpan],
        span_to_context_id: &mut FxHashMap<DocumentSpan, i32>,
        options: &BaselineFourslashLocationsOptions,
    ) -> String {
        let mut details: Vec<BaselineDetail> = Vec::new();
        // Go keys these by *baselineDetail; details are only reordered after the maps are filled, so the
        // creation index identifies a detail.
        let mut detail_prefixes: FxHashMap<usize, String> = FxHashMap::default();
        let mut detail_suffixes: FxHashMap<usize, String> = FxHashMap::default();
        let can_determine_context_id_inline = true;

        if let Some(marker) = &options.marker {
            if marker.file_name() == file_name {
                details.push(BaselineDetail { pos: marker.ls_pos(), position_marker: options.marker_name.clone(), span: None, kind: DetailKind::Marker });
            }
        }

        for (si, span) in spans_in_file.iter().enumerate() {
            let context_span_index = details.len();

            // Add context span markers if present
            if let Some(context_span) = span.context_span {
                details.push(BaselineDetail { pos: context_span.start, position_marker: "<|".to_string(), span: Some(si), kind: DetailKind::ContextStart });

                // Check if context span starts after text span
                if lsproto::compare_positions(context_span.start, span.text_span.start) > 0 {
                    // can_determine_context_id_inline = false (unused, see below)
                }
            }

            let text_span_index = details.len();
            let mut start_marker = "[|".to_string();
            if let Some(get_location_data) = &options.get_location_data {
                start_marker += &get_location_data(span);
            }
            details.push(BaselineDetail { pos: span.text_span.start, position_marker: start_marker, span: Some(si), kind: DetailKind::TextStart });
            details.push(BaselineDetail {
                pos: span.text_span.end,
                position_marker: if options.end_marker.is_empty() { "|]".to_string() } else { options.end_marker.clone() },
                span: Some(si),
                kind: DetailKind::TextEnd,
            });

            if let Some(context_span) = span.context_span {
                details.push(BaselineDetail { pos: context_span.end, position_marker: "|>".to_string(), span: Some(si), kind: DetailKind::ContextEnd });
            }

            if let Some(start_marker_prefix) = &options.start_marker_prefix {
                if let Some(start_prefix) = start_marker_prefix(span) {
                    // Special case: if this span starts at the same position as the provided marker,
                    // we want the span's prefix to appear before the marker name.
                    // i.e. We want `/*START PREFIX*/A: /*RENAME*/[|ARENAME|]`,
                    // not `/*RENAME*//*START PREFIX*/A: [|ARENAME|]`
                    let at_marker = options.marker.as_ref().is_some_and(|m| file_name == m.file_name() && span.text_span.start == m.ls_pos());
                    if at_marker {
                        assert!(!detail_prefixes.contains_key(&0), "Expected only single prefix at marker location");
                        detail_prefixes.insert(0, start_prefix);
                    } else if span.context_span.is_some_and(|c| c.start == span.text_span.start) {
                        detail_prefixes.insert(context_span_index, start_prefix);
                    } else {
                        detail_prefixes.insert(text_span_index, start_prefix);
                    }
                }
            }

            if let Some(end_marker_suffix) = &options.end_marker_suffix {
                if let Some(end_suffix) = end_marker_suffix(span) {
                    // Same as above for suffixes:
                    let at_marker = options.marker.as_ref().is_some_and(|m| file_name == m.file_name() && span.text_span.end == m.ls_pos());
                    if at_marker {
                        detail_suffixes.insert(0, end_suffix);
                    } else if span.context_span.is_some_and(|c| c.end == span.text_span.end) {
                        detail_suffixes.insert(text_span_index + 2, end_suffix);
                    } else {
                        detail_suffixes.insert(text_span_index + 1, end_suffix);
                    }
                }
            }
        }

        // Our preferred way to write markers is
        // /*MARKER*/[| some text |]
        // [| some /*MARKER*/ text |]
        // [| some text |]/*MARKER*/
        let mut order: Vec<usize> = (0..details.len()).collect();
        order.sort_by(|&i1, &i2| {
            let (d1, d2) = (&details[i1], &details[i2]);
            let c = lsproto::compare_positions(d1.pos, d2.pos);
            if c != 0 || d1.kind == DetailKind::Marker && d2.kind == DetailKind::Marker {
                return c.cmp(&0);
            }

            // /*MARKER*/[| some text |]
            if d1.kind == DetailKind::Marker && d2.kind.is_start() {
                return std::cmp::Ordering::Less;
            }
            if d2.kind == DetailKind::Marker && d1.kind.is_start() {
                return std::cmp::Ordering::Greater;
            }

            // [| some text |]/*MARKER*/
            if d1.kind == DetailKind::Marker && d2.kind.is_end() {
                return std::cmp::Ordering::Greater;
            }
            if d2.kind == DetailKind::Marker && d1.kind.is_end() {
                return std::cmp::Ordering::Less;
            }

            // [||] or <||>
            if d1.span == d2.span {
                return d1.kind.cmp(&d2.kind);
            }

            // ...|><|...
            if d1.kind.is_start() && d2.kind.is_end() {
                return std::cmp::Ordering::Greater;
            }
            if d1.kind.is_end() && d2.kind.is_start() {
                return std::cmp::Ordering::Less;
            }

            // <| ... [| ... |]|>
            if d1.kind.is_end() && d2.kind.is_end() {
                let c = lsproto::compare_positions(d2.get_range(spans_in_file).start, d1.get_range(spans_in_file).start);
                if c != 0 {
                    return c.cmp(&0);
                }
                return d1.kind.cmp(&d2.kind);
            }

            // <|[| ... |] ... |>
            if d1.kind.is_start() && d2.kind.is_start() {
                // (Go compares d2's range end with itself.)
                let c = lsproto::compare_positions(d2.get_range(spans_in_file).end, d2.get_range(spans_in_file).end);
                if c != 0 {
                    return c.cmp(&0);
                }
                return d1.kind.cmp(&d2.kind);
            }

            std::cmp::Ordering::Equal
        });
        // !!! if canDetermineContextIdInline

        let mut text_with_context = new_text_with_context(file_name, content);
        for (index, &di) in order.iter().enumerate() {
            let detail = &details[di];
            text_with_context.add(Some(detail));
            text_with_context.pos = detail.pos;
            // Prefix
            if let Some(prefix) = detail_prefixes.get(&di) {
                if !prefix.is_empty() {
                    text_with_context.new_content.push_str(prefix);
                }
            }
            text_with_context.new_content.push_str(&detail.position_marker);
            if let Some(span_index) = detail.span {
                let span = &spans_in_file[span_index];
                match detail.kind {
                    DetailKind::TextStart => {
                        let mut text = String::new();
                        if let Some(context_id) = span_to_context_id.get(span) {
                            let mut is_after_context_start = false;
                            let mut text_start_index = index as i32 - 1;
                            while text_start_index >= 0 {
                                let text_start_detail = &details[order[text_start_index as usize]];
                                if text_start_detail.kind == DetailKind::ContextStart && text_start_detail.span == detail.span {
                                    is_after_context_start = true;
                                    break;
                                }
                                // Marker is ok to skip over
                                if text_start_detail.span.is_some() {
                                    break;
                                }
                                text_start_index -= 1;
                            }
                            // Skip contextId on span thats surrounded by context span immediately
                            if !is_after_context_start {
                                if text.is_empty() {
                                    text = format!("contextId: {context_id}");
                                } else {
                                    text = format!("contextId: {context_id}") + ", " + &text;
                                }
                            }
                        }
                        if !text.is_empty() {
                            text_with_context.new_content.push_str("{ ");
                            text_with_context.new_content.push_str(&text);
                            text_with_context.new_content.push_str(" |}");
                        }
                    }
                    DetailKind::ContextStart => {
                        if can_determine_context_id_inline {
                            let n = span_to_context_id.len() as i32;
                            span_to_context_id.insert(span.clone(), n);
                        }
                    }
                    _ => {}
                }
            }
            if let Some(suffix) = detail_suffixes.get(&di) {
                text_with_context.new_content.push_str(suffix);
            }
        }
        text_with_context.add(None);
        if !text_with_context.new_content.is_empty() {
            text_with_context.readable_contents.push('\n');
            let nc = text_with_context.new_content.clone();
            text_with_context.readable_jsonc_baseline(&nc);
        }
        text_with_context.readable_contents
    }
}

// baselineutil.go:527
static LINE_SPLITTER: LazyLock<Regex> = LazyLock::new(|| Regex::new("\r?\n").unwrap());

// baselineutil.go:529
pub(crate) struct TextWithContext {
    n_lines_context: i32, // number of context lines to write to baseline

    readable_contents: String, // builds what will be returned to be written to baseline

    new_content: String, // helper; the part of the original file content to write between details
    pos: lsproto::Position,
    is_lib_file: bool,
    file_name: String,
    content: String, // content of the original file
    line_starts: Arc<LSPLineMap>,
    converters: TestConverters,

    // posLineInfo
    pos_info: Option<lsproto::Position>,
    line_info: i32,
}

// baselineutil.go:547-564: textWithContext implements lsconv.Script.
impl Script for TextWithContext {
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

// The converters need an owned Script; this is the file text TextWithContext converts against.
#[derive(Clone)]
struct ContentScript {
    file_name: String,
    content: Arc<str>,
}

impl Script for ContentScript {
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

// baselineutil.go:566
pub(crate) fn new_text_with_context(file_name: &str, content: &str) -> TextWithContext {
    let line_starts = lsconv::compute_lsp_line_starts(content);
    let ls = line_starts.clone();
    let mut t = TextWithContext {
        n_lines_context: 4,

        readable_contents: String::new(),

        is_lib_file: is_lib_file(file_name),
        new_content: String::new(),
        pos: lsproto::Position { line: 0, character: 0 },
        file_name: file_name.to_string(),
        content: content.to_string(),
        line_starts,
        converters: new_test_converters(lsconv::new_converters(lsproto::PositionEncodingKind::UTF8, move |_| Some(ls.clone()))),
        pos_info: None,
        line_info: 0,
    };
    t.readable_contents.push_str("// === ");
    t.readable_contents.push_str(file_name);
    t.readable_contents.push_str(" ===");
    t
}

// fourslash.go:5685
pub(crate) fn is_lib_file(file_name: &str) -> bool {
    let base_name = tsrs_core::tspath::get_base_file_name(file_name);
    if base_name.starts_with("lib.") && base_name.ends_with(".d.ts") {
        return true;
    }
    false
}

impl TextWithContext {
    fn script(&self) -> ContentScript {
        ContentScript { file_name: self.file_name.clone(), content: Arc::from(self.content.as_str()) }
    }

    fn position_index(&self, p: lsproto::Position) -> usize {
        self.converters.line_and_character_to_position(self.script(), p) as usize
    }

    // baselineutil.go:589
    pub(crate) fn add(&mut self, detail: Option<&BaselineDetail>) {
        if self.new_content.is_empty() && detail.is_none() {
            panic!("Unsupported");
        }
        if detail.is_none() || detail.is_some_and(|d| d.kind != DetailKind::TextEnd && d.kind != DetailKind::ContextEnd) {
            // Calculate pos to location number of lines
            let mut pos_line_index = self.line_info;
            if self.pos_info != Some(self.pos) {
                pos_line_index = self.line_starts.compute_index_of_line_start(self.converters.line_and_character_to_position(self.script(), self.pos)) as i32;
            }

            let mut location_line_index = self.line_starts.line_starts.len() as i32 - 1;
            if let Some(detail) = detail {
                location_line_index = self.line_starts.compute_index_of_line_start(self.converters.line_and_character_to_position(self.script(), detail.pos)) as i32;
                self.pos_info = Some(detail.pos);
                self.line_info = location_line_index;
            }

            let mut n_lines = 0;
            if !self.new_content.is_empty() {
                n_lines += self.n_lines_context + 1;
            }
            if detail.is_some() {
                n_lines += self.n_lines_context + 1;
            }
            // first nLinesContext and last nLinesContext
            if location_line_index - pos_line_index > n_lines {
                if !self.new_content.is_empty() {
                    let skipped_string =
                        if self.is_lib_file { "--- (line: --) skipped ---\n".to_string() } else { format!("--- (line: {}) skipped ---", pos_line_index + self.n_lines_context + 1) };

                    self.readable_contents.push('\n');
                    let start = self.position_index(self.pos);
                    let end = self.line_starts.line_starts[(pos_line_index + self.n_lines_context) as usize] as usize;
                    let text = self.new_content.clone() + &self.slice_of_content(Some(start as u32), Some(end as u32)) + &skipped_string;
                    self.readable_jsonc_baseline(&text);

                    if detail.is_some() {
                        self.readable_contents.push('\n');
                    }
                    self.new_content.clear();
                }
                if let Some(detail) = detail {
                    if self.is_lib_file {
                        self.new_content.push_str("--- (line: --) skipped ---\n");
                    } else {
                        let _ = writeln!(self.new_content, "--- (line: {}) skipped ---", location_line_index - self.n_lines_context + 1);
                    }
                    let start = self.line_starts.line_starts[(location_line_index - self.n_lines_context + 1) as usize];
                    let end = self.position_index(detail.pos) as u32;
                    let s = self.slice_of_content(Some(start), Some(end));
                    self.new_content.push_str(&s);
                }
                return;
            }
        }
        let s = match detail {
            None => self.slice_of_content(Some(self.position_index(self.pos) as u32), None),
            Some(detail) => self.slice_of_content(Some(self.position_index(self.pos) as u32), Some(self.position_index(detail.pos) as u32)),
        };
        self.new_content.push_str(&s);
    }

    // baselineutil.go:656
    pub(crate) fn readable_jsonc_baseline(&mut self, text: &str) {
        for (i, line) in LINE_SPLITTER.split(text).enumerate() {
            if i > 0 {
                self.readable_contents.push('\n');
            }
            self.readable_contents.push_str("// ");
            self.readable_contents.push_str(line);
        }
    }

    // baselineutil.go:763
    pub(crate) fn slice_of_content(&self, start: Option<u32>, end: Option<u32>) -> String {
        let start = start.map_or(0, |s| s as usize);
        let end = match end {
            Some(e) if (e as usize) <= self.content.len() => e as usize,
            _ => self.content.len(),
        };

        if start > end {
            return String::new();
        }

        self.content[start..end].to_string()
    }
}

// baselineutil.go:666
#[derive(Clone, Debug)]
pub(crate) struct MarkerAndItem<I> {
    pub(crate) marker: Arc<Marker>,
    pub(crate) item: I,
}

impl FourslashTest {
    // baselineutil.go:671
    pub(crate) fn annotate_content_with_tooltips<I: Clone + PartialEq + Default>(
        &self,
        t: &T,
        markers_and_items: &[MarkerAndItem<I>],
        op_name: &str,
        get_range: impl Fn(&I) -> Option<lsproto::Range>,
        get_tooltip_lines: impl Fn(&I, &I) -> Vec<String>,
    ) -> String {
        let bar_with_gutter = format!("| {}", "-".repeat(70));

        // sort by file, then *backwards* by position in the file
        // so we can insert multiple times on a line without counting.
        let mut sorted = markers_and_items.to_vec();
        sorted.sort_by(|a, b| {
            let c = a.marker.file_name().cmp(&b.marker.file_name());
            if c != std::cmp::Ordering::Equal {
                return c;
            }
            b.marker.position.cmp(&a.marker.position)
        });

        let mut files_to_lines: OrderedMap<String, Vec<String>> = OrderedMap::default();
        let mut previous = I::default();
        for item_and_marker in &sorted {
            let marker = &item_and_marker.marker;
            let item = &item_and_marker.item;

            let text_range = match get_range(item) {
                Some(r) => r,
                None => {
                    let start = marker.ls_position;
                    let mut end = start;
                    end.character += 1;
                    lsproto::Range { start, end }
                }
            };

            if text_range.start.line != text_range.end.line {
                t.fatal(&format!("Expected text range to be on a single line, got {text_range:?}"));
            }
            let underline = " ".repeat(text_range.start.character as usize) + &"^".repeat((text_range.end.character - text_range.start.character) as usize);

            let file_name = marker.file_name();
            let mut lines = match files_to_lines.get(&file_name) {
                Some(l) => l.clone(),
                None => LINE_SPLITTER.split(&self.get_script_info(&file_name).content).map(|s| s.to_string()).collect(),
            };

            let mut tooltip_lines: Vec<String> = Vec::new();
            if *item != I::default() {
                tooltip_lines = get_tooltip_lines(item, &previous);
            }
            if tooltip_lines.is_empty() {
                tooltip_lines = vec![format!("No {} at /*{}*/.", op_name, marker.name.clone().unwrap_or_default())];
            }
            let tooltip_lines: Vec<String> = tooltip_lines.iter().map(|line| format!("| {line}")).collect();

            let mut lines_to_insert = Vec::with_capacity(tooltip_lines.len() + 3);
            lines_to_insert.push(underline);
            lines_to_insert.push(bar_with_gutter.clone());
            lines_to_insert.extend(tooltip_lines);
            lines_to_insert.push(bar_with_gutter.clone());

            let at = (text_range.start.line + 1) as usize;
            lines.splice(at..at, lines_to_insert);
            files_to_lines.insert(file_name, lines);

            previous = item.clone();
        }

        let mut builder = String::new();
        let mut seen_first = false;
        for (file_name, lines) in &files_to_lines {
            let _ = writeln!(builder, "=== {file_name} ===");
            for line in lines {
                builder.push_str("// ");
                builder.push_str(line);
                builder.push('\n');
            }

            if seen_first {
                builder.push_str("\n\n");
            } else {
                seen_first = true;
            }
        }

        builder
    }
}

// baselineutil.go:797
pub(crate) fn code_fence(lang: &str, code: &str) -> String {
    format!("```{lang}\n{code}\n```")
}

// baselineutil.go:801
pub(crate) fn symbol_information_to_data(symbol: &lsproto::SymbolInformation) -> String {
    format!("{{| name: {}, kind: {} |}}", symbol.name, symbol.kind.string())
}
