// Port of ls/format.go. Renamed: module `format` is Go package `internal/format`.

use tsrs_ast::{self as ast, CommentRange, Kind, Node, SourceFile};
use tsrs_core::context::Context;
use tsrs_core::{TextChange, TextRange, P};
use tsrs_lsproto as lsproto;
use tsrs_scanner as scanner;

use crate::astnav;
use crate::format;
use crate::languageservice::LanguageService;
use crate::lsutil::{self, FormatCodeSettings};
use crate::spanmap::{self, Feature};
use crate::utilities::{get_leading_comment_ranges_of_node, is_in_comment};

impl LanguageService {
    // format.go:19 (Go returns a nil slice when a change does not map exactly; callers take its address, so the
    // response always carries a list.)
    fn to_ls_proto_text_edits(&self, file: P<SourceFile>, changes: &[TextChange]) -> Vec<lsproto::TextEdit> {
        let mut result = Vec::with_capacity(changes.len());
        for c in changes {
            let (lsp_range, fidelity) = self.converters.to_lsp_range(&file, TextRange::new(c.pos(), c.end()));
            if !fidelity.is_exact() {
                return Vec::new();
            }
            result.push(lsproto::TextEdit { new_text: c.new_text.clone(), range: lsp_range });
        }
        result
    }

    // format.go:34
    pub fn provide_format_document(
        &self,
        ctx: &Context,
        document_uri: &lsproto::DocumentUri,
        options: lsproto::FormattingOptions,
    ) -> Result<lsproto::DocumentFormattingResponse, lsproto::Error> {
        if self.user_preferences().enable_formatting.is_false() {
            return Ok(lsproto::TextEditsOrNull::default());
        }
        let (_, file) = self.get_program_and_file(document_uri);
        let format_opts = lsutil::from_ls_format_options(&self.format_options(), options);
        let edits = if file.content_mapper().is_empty() {
            self.to_ls_proto_text_edits(file, &self.get_formatting_edits_for_document(ctx, file, &format_opts))
        } else {
            self.get_formatting_edits_for_mapped_range(ctx, file, &format_opts, TextRange::new(0, file.original_text().len() as i32))
        };
        Ok(lsproto::TextEditsOrNull { text_edits: Some(edits) })
    }

    // getFormattingEditsForMappedRange formats each formatting-enabled verbatim intersection with originalRange.
    // Duplicate formatting projections are unsupported. If mappings overlap anyway, each original-text position
    // is formatted only once, preferring the earliest and then longest applicable mapping.
    // format.go:56
    fn get_formatting_edits_for_mapped_range(
        &self,
        ctx: &Context,
        file: P<SourceFile>,
        options: &FormatCodeSettings,
        original_range: TextRange,
    ) -> Vec<lsproto::TextEdit> {
        let mut projections = vec![file];
        projections.extend_from_slice(file.supplemental_source_files());
        let mut candidates: Vec<MappedFormattingRange> = Vec::new();
        for &projection in &projections {
            let Some(span_map) = crate::lsconv::Script::span_map(&projection) else {
                continue;
            };
            for segment in span_map.segments() {
                if segment.kind != spanmap::Kind::Verbatim || !segment.features.intersects(Feature::Formatting) {
                    continue;
                }
                let original_start = original_range.pos().max(segment.original_start);
                let original_end = original_range.end().min(segment.original_end);
                if original_start >= original_end {
                    continue;
                }
                candidates.push(MappedFormattingRange {
                    projection,
                    segment,
                    original_range: TextRange::new(original_start, original_end),
                });
            }
        }

        let mut edits: Vec<lsproto::TextEdit> = Vec::new();
        for candidate in non_overlapping_formatting_ranges(&candidates) {
            let virtual_range = TextRange::new(
                candidate.segment.virtual_start + candidate.original_range.pos() - candidate.segment.original_start,
                candidate.segment.virtual_start + candidate.original_range.end() - candidate.segment.original_start,
            );
            for change in self.get_formatting_edits_for_range(ctx, candidate.projection, options, virtual_range) {
                if change.pos() < virtual_range.pos() || change.end() > virtual_range.end() {
                    continue;
                }
                let (lsp_range, fidelity) = self.converters.to_lsp_range_for_feature(
                    &candidate.projection,
                    TextRange::new(change.pos(), change.end()),
                    Feature::Formatting,
                );
                if !fidelity.is_exact() {
                    continue;
                }
                edits.push(lsproto::TextEdit { range: lsp_range, new_text: change.new_text });
            }
        }
        // Go `slices.SortStableFunc` (CompareRanges, then the text).
        edits.sort_by(|a, b| lsproto::compare_ranges(a.range, b.range).cmp(&0).then_with(|| a.new_text.cmp(&b.new_text)));
        edits
    }
}

// format.go:107
#[derive(Clone, Copy)]
struct MappedFormattingRange {
    projection: P<SourceFile>,
    segment: spanmap::Segment,
    original_range: TextRange,
}

// nonOverlappingFormattingRanges chooses at most one formatting projection for each original-text position.
// Candidates are ordered by original start and then descending end, so a longer mapping wins when several
// mappings start together:
//
//	candidates:  [---------- A ----------)
//	             [---- B ----)
//	result:      [---------- A ----------)
//
// Since starts are ordered, a candidate can only overlap the end of the last accepted range. Its start is
// trimmed to that end, preserving any uncovered suffix:
//
//	candidates:  [------- A -------)
//	                    [------- B ----------)
//	result:      [------- A -------)[-- B' --)
//
// Fully covered candidates have no suffix and are discarded. The segment itself is retained so callers can
// translate a trimmed original range to the corresponding offset in its virtual projection.
// format.go:130
fn non_overlapping_formatting_ranges(candidates: &[MappedFormattingRange]) -> Vec<MappedFormattingRange> {
    let mut candidates = candidates.to_vec();
    // Go `slices.SortStableFunc`.
    candidates.sort_by(|a, b| {
        a.original_range.pos().cmp(&b.original_range.pos()).then_with(|| b.original_range.end().cmp(&a.original_range.end()))
    });

    let mut result: Vec<MappedFormattingRange> = Vec::with_capacity(candidates.len());
    for mut candidate in candidates {
        if let Some(last) = result.last() {
            candidate.original_range = candidate.original_range.with_pos(candidate.original_range.pos().max(last.original_range.end()));
        }
        if candidate.original_range.len() > 0 {
            result.push(candidate);
        }
    }
    result
}

impl LanguageService {
    // format.go:157
    pub fn provide_format_document_range(
        &self,
        ctx: &Context,
        document_uri: &lsproto::DocumentUri,
        options: lsproto::FormattingOptions,
        r: lsproto::Range,
    ) -> Result<lsproto::DocumentRangeFormattingResponse, lsproto::Error> {
        if self.user_preferences().enable_formatting.is_false() {
            return Ok(lsproto::TextEditsOrNull::default());
        }
        let (_, file) = self.get_program_and_file(document_uri);
        let format_opts = lsutil::from_ls_format_options(&self.format_options(), options);
        if !file.content_mapper().is_empty() {
            let original = self.converters.from_lsp_range_to_original(&file, r);
            let edits = self.get_formatting_edits_for_mapped_range(ctx, file, &format_opts, original);
            return Ok(lsproto::TextEditsOrNull { text_edits: Some(edits) });
        }
        let ranges = self.converters.from_lsp_range_for_source_file(file, r, Feature::Formatting);
        if ranges.len() != 1 || !ranges[0].fidelity.is_exact() {
            return Ok(lsproto::TextEditsOrNull::default());
        }
        let file = ranges[0].script;
        let edits = self.to_ls_proto_text_edits(file, &self.get_formatting_edits_for_range(ctx, file, &format_opts, ranges[0].span));
        Ok(lsproto::TextEditsOrNull { text_edits: Some(edits) })
    }

    // format.go:184
    pub fn provide_format_document_on_type(
        &self,
        ctx: &Context,
        document_uri: &lsproto::DocumentUri,
        options: lsproto::FormattingOptions,
        position: lsproto::Position,
        character: &str,
    ) -> Result<lsproto::DocumentOnTypeFormattingResponse, lsproto::Error> {
        if self.user_preferences().enable_formatting.is_false() {
            return Ok(lsproto::TextEditsOrNull::default());
        }
        let (_, file) = self.get_program_and_file(document_uri);
        let format_opts = lsutil::from_ls_format_options(&self.format_options(), options);
        let positions = self.converters.from_lsp_position_for_source_file(file, position, Feature::Formatting);
        if positions.len() != 1 || !positions[0].fidelity.is_exact() {
            return Ok(lsproto::TextEditsOrNull::default());
        }
        let file = positions[0].script;
        let edits = self.to_ls_proto_text_edits(file, &self.get_formatting_edits_after_keystroke(ctx, file, &format_opts, positions[0].position, character));
        Ok(lsproto::TextEditsOrNull { text_edits: Some(edits) })
    }

    // format.go:210 (Go's ctx carries the settings: format::FormatContext)
    fn get_formatting_edits_for_range(&self, _ctx: &Context, file: P<SourceFile>, options: &FormatCodeSettings, r: TextRange) -> Vec<TextChange> {
        let ctx = format::with_format_code_settings(&format::FormatContext::default(), options.clone(), &options.new_line_character);
        format::format_selection(&ctx, file, r.pos(), r.end())
    }

    // format.go:220
    fn get_formatting_edits_for_document(&self, _ctx: &Context, file: P<SourceFile>, options: &FormatCodeSettings) -> Vec<TextChange> {
        let ctx = format::with_format_code_settings(&format::FormatContext::default(), options.clone(), &options.new_line_character);
        format::format_document(&ctx, file)
    }

    // format.go:229
    fn get_formatting_edits_after_keystroke(&self, _ctx: &Context, file: P<SourceFile>, options: &FormatCodeSettings, position: i32, key: &str) -> Vec<TextChange> {
        let ctx = format::with_format_code_settings(&format::FormatContext::default(), options.clone(), &options.new_line_character);

        let token_at_position = astnav::get_token_at_position(file, position);
        if is_in_comment(file, position, token_at_position).is_none() {
            return match key {
                "{" => format::format_on_opening_curly(&ctx, file, position),
                "}" => format::format_on_closing_curly(&ctx, file, position),
                ";" => format::format_on_semicolon(&ctx, file, position),
                "\n" => format::format_on_enter(&ctx, file, position),
                _ => Vec::new(),
            };
        }
        Vec::new()
    }
}

// Unlike the TS implementation, this function *will not* compute default values for
// `precedingToken` and `tokenAtPosition`.
// It is the caller's responsibility to call `astnav.GetTokenAtPosition` to compute a default `tokenAtPosition`,
// or `astnav.FindPrecedingToken` to compute a default `precedingToken`.
// format.go:257
pub(crate) fn get_range_of_enclosing_comment(
    file: P<SourceFile>,
    position: i32,
    preceding_token: Option<P<Node>>,
    mut token_at_position: P<Node>,
) -> Option<CommentRange> {
    let jsdoc = ast::find_ancestor(token_at_position, |n| n.is_jsdoc());
    if let Some(jsdoc) = jsdoc {
        token_at_position = jsdoc.parent().unwrap();
    }
    let token_start = astnav::get_start_of_node(token_at_position, file, false /*includeJSDoc*/);
    if token_start <= position && position < token_at_position.end() {
        return None;
    }

    // Between two consecutive tokens, all comments are either trailing on the former
    // or leading on the latter (and none are in both lists).
    let trailing_ranges_of_previous_token = preceding_token.map(|t| scanner::get_trailing_comment_ranges(file.text(), t.end()));
    let leading_ranges_of_next_token = get_leading_comment_ranges_of_node(token_at_position, file);
    let comment_ranges = trailing_ranges_of_previous_token.into_iter().flatten().chain(leading_ranges_of_next_token.into_iter().flatten());
    for comment_range in comment_ranges {
        // The end marker of a single-line comment does not include the newline character.
        // In the following case where the cursor is at `^`, we are inside a comment:
        //
        //    // asdf   ^\n
        //
        // But for closed multi-line comments, we don't want to be inside the comment in the following case:
        //
        //    /* asdf */^
        //
        // Internally, we represent the end of the comment prior to the newline and at the '/', respectively.
        //
        // However, unterminated multi-line comments lack a `/`, end at the end of the file, and *do* contain their end.
        //
        if comment_range.text_range.contains_exclusive(position)
            || position == comment_range.end() && (comment_range.kind == Kind::SingleLineCommentTrivia || position == file.text().len() as i32)
        {
            return Some(comment_range);
        }
    }
    None
}
