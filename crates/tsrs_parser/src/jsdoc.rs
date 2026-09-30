use bitflags::bitflags;
use tsrs_ast::{self as ast, Diagnostic, Kind, ModifierList, Node, NodeFlags, NodeList, SourceFile};
use tsrs_core::{alloc_slice, alloc_str, alloc_vec, stringutil, TextRange, P};
use tsrs_diagnostics::{self as diagnostics, Message};

use crate::parser_1::{new_parser, JSDocInfo, JsdocScannerInfo, Parser, ParsingContext};
use crate::parser_3::is_reserved_word;
use crate::utilities::{get_jsdoc_comment_ranges, is_jsdoc_like_text, token_is_identifier_or_keyword};

static INIT: std::sync::Once = std::sync::Once::new();

// Go registers this from the parser package's init(); Rust has no package initializers, so parser
// entry points call this (idempotent) before any lazy JSDoc access can happen.
pub(crate) fn init() {
    INIT.call_once(|| ast::set_parse_jsdoc_for_node(parse_jsdoc_for_node));
}

// parseJSDocForNode lazily parses JSDoc for a node in a TS file.
// Called on first access to Node.JSDoc() for non-JS source files.
fn parse_jsdoc_for_node(source_file: P<SourceFile>, node: P<Node>) -> Vec<P<Node>> {
    let mut p = new_parser();
    p.initialize_state(source_file.parse_options(), source_file.text(), source_file.script_kind());
    let ranges = get_jsdoc_comment_ranges(&mut p.factory, &[], node, source_file.text());
    if ranges.is_empty() {
        return Vec::new();
    }
    let mut jsdoc = Vec::with_capacity(ranges.len());
    let mut pos = node.pos();
    for comment in &ranges {
        if let Some(parsed) = p.parse_jsdoc_comment(node, comment.pos(), comment.end(), pos) {
            parsed.set_parent(Some(node));
            jsdoc.push(parsed);
            pos = parsed.end();
        }
    }
    jsdoc
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum JsdocState {
    BeginningOfLine,
    SawAsterisk,
    SavingComments,
    SavingBackticks,
}

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
    pub(crate) struct PropertyLikeParse: i32 {
        const Property = 1 << 0;
        const Parameter = 1 << 1;
        const CallbackParameter = 1 << 2;
    }
}

impl Parser {
    pub(crate) fn with_jsdoc(&mut self, node: P<Node>, info: JsdocScannerInfo) -> &'static [P<Node>] {
        if !info.intersects(JsdocScannerInfo::HasJSDoc) {
            return &[];
        }

        // For TS/TSX files, defer JSDoc parsing to first access, unless the comment
        // contains @see/@link (needed for unused-identifier checks).
        // @deprecated is detected via cheap text scan to set PossiblyContainsDeprecatedTag;
        // callers must confirm via JSDoc lookup.
        if !self.is_java_script() {
            node.set_flags(node.flags() | NodeFlags::HasJSDoc);
            if info.intersects(JsdocScannerInfo::HasDeprecated) {
                node.set_flags(node.flags() | NodeFlags::PossiblyContainsDeprecatedTag);
            }
            if !info.intersects(JsdocScannerInfo::HasSeeOrLink) {
                return &[];
            }
            // Fall through to eager parse for @see/@link
        }

        let ranges = get_jsdoc_comment_ranges(&mut self.factory, &[], node, self.source_text);

        // Should only be called once per node
        self.has_deprecated_tag = false;
        let mut jsdoc: Vec<P<Node>> = Vec::with_capacity(ranges.len());
        let mut pos = node.pos();
        for comment in &ranges {
            if let Some(parsed) = self.parse_jsdoc_comment(node, comment.pos(), comment.end(), pos) {
                parsed.set_parent(Some(node));
                jsdoc.push(parsed);
                pos = parsed.end();
            }
        }
        if !jsdoc.is_empty() {
            if !node.flags().intersects(NodeFlags::HasJSDoc) {
                node.set_flags(node.flags() | NodeFlags::HasJSDoc);
            }
            if self.has_deprecated_tag {
                self.has_deprecated_tag = false;
                node.set_flags(node.flags() | NodeFlags::PossiblyContainsDeprecatedTag);
            }
            let jsdoc = alloc_vec(jsdoc);
            if self.is_java_script() {
                self.reparse_tags(node, jsdoc);
            }
            self.jsdoc_infos.push(JSDocInfo { parent: node, js_docs: jsdoc });
            return jsdoc;
        }
        &[]
    }

    pub(crate) fn parse_jsdoc_type_expression(&mut self, may_omit_braces: bool) -> P<Node> {
        let pos = self.node_pos();
        let has_brace = if may_omit_braces {
            self.parse_optional(Kind::OpenBraceToken)
        } else {
            self.parse_expected(Kind::OpenBraceToken)
        };
        let save_context_flags = self.context_flags;
        self.set_context_flags(NodeFlags::JSDoc, true);
        let t = self.parse_jsdoc_type();
        self.context_flags = save_context_flags;
        if has_brace {
            self.parse_expected_jsdoc(Kind::CloseBraceToken);
        }

        let node = self.factory.new_jsdoc_type_expression(t);
        self.finish_node(node, pos)
    }

    pub(crate) fn parse_jsdoc_name_reference(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let has_brace = self.parse_optional(Kind::OpenBraceToken);
        let entity_name = self.parse_jsdoc_link_name();
        if has_brace {
            self.parse_expected_jsdoc(Kind::CloseBraceToken);
        }
        let full_start = self.scanner.token_full_start();
        self.scanner.reset_pos(full_start);
        self.next_token_jsdoc();
        let node = self.factory.new_jsdoc_name_reference(entity_name);
        self.finish_node(node, pos)
    }

    // Pass end=-1 to parse the text to the end
    pub(crate) fn parse_jsdoc_comment(&mut self, parent: P<Node>, start: i32, end: i32, full_start: i32) -> Option<P<Node>> {
        let end = if end == -1 { self.source_text.len() as i32 } else { end };
        // Check for /** (JSDoc opening part)
        if !is_jsdoc_like_text(&self.source_text[start as usize..]) {
            // TODO: This should be a panic, unless parseSingleJSDocComment is calling this (not ported yet)
            return None;
        }

        let save_source_text = self.source_text;
        let save_token = self.token;
        let save_context_flags = self.context_flags;
        let save_parsing_contexts = self.parsing_contexts;
        let save_scanner_state = self.scanner.mark();
        let save_diagnostics_length = self.diagnostics.len();
        let save_has_parse_error = self.has_parse_error;
        let save_has_await_identifier = self.statement_has_await_identifier;

        // initial indent is start+4 to account for leading `/** `
        // + 1 because \n is one character before the first character in the line and,
        // if there is no \n before start, -1 is one index before the first character in the string
        let last_newline = self.source_text[..start as usize].rfind('\n').map_or(-1, |i| i as i32);
        let initial_indent = start + 4 - (last_newline + 1);
        // -2 for trailing `*/`
        self.source_text = &self.source_text[..(end - 2) as usize];
        self.scanner.set_text(self.source_text);
        // +3 for leading `/**`
        self.scanner.reset_pos(start + 3);
        self.set_context_flags(NodeFlags::JSDoc, true);
        self.parsing_contexts |= 1 << ParsingContext::JSDocComment as i32;

        let comment = self.parse_jsdoc_comment_worker(start, end, full_start, initial_indent);
        // move jsdoc diagnostics to jsdocDiagnostics -- for JS files only
        let moved = self.diagnostics.split_off(save_diagnostics_length);
        if self.context_flags.intersects(NodeFlags::JavaScriptFile) {
            self.jsdoc_diagnostics.extend(moved);
        }

        self.source_text = save_source_text;
        self.scanner.set_text(self.source_text);
        self.parsing_contexts = save_parsing_contexts;
        self.context_flags = save_context_flags;
        self.scanner.rewind(save_scanner_state);
        self.token = save_token;
        self.has_parse_error = save_has_parse_error;
        self.statement_has_await_identifier = save_has_await_identifier;

        Some(comment)
    }

    /**
     * @param offset - the offset in the containing file
     * @param indent - the number of spaces to consider as the margin (applies to non-first lines only)
     */
    pub(crate) fn parse_jsdoc_comment_worker(&mut self, start: i32, end: i32, full_start: i32, indent: i32) -> P<Node> {
        let mut indent = indent;
        // Initially we can parse out a tag.  We also have seen a starting asterisk.
        // This is so that /** * @type */ doesn't parse.
        let mut tags: Vec<P<Node>> = Vec::new();
        let mut tags_pos: i32 = -1;
        let mut tags_end: i32 = -1;
        let mut state = JsdocState::SawAsterisk;
        let mut backtick_count = 0;
        let mut in_fenced_code_block = false;
        let mut comment_parts: Vec<P<Node>> = Vec::new();
        let mut comments: Vec<&'static str> = std::mem::take(&mut self.jsdoc_comments_space);
        let mut comments_pos: i32 = -1;
        let mut link_end = start;
        let mut margin: i32 = -1;
        macro_rules! push_comment {
            ($text:expr) => {{
                let text: &'static str = $text;
                if margin == -1 {
                    margin = indent;
                }
                comments.push(text);
                indent += text.len() as i32;
            }};
        }

        self.next_token_jsdoc();
        while self.parse_optional_jsdoc(Kind::WhitespaceTrivia) {}
        if self.parse_optional_jsdoc(Kind::NewLineTrivia) {
            state = JsdocState::BeginningOfLine;
            indent = 0;
        }
        loop {
            // Detect fenced code blocks by counting consecutive backtick tokens.
            // Three or more consecutive backticks toggle the fenced code block state.
            if self.token != Kind::BacktickToken && backtick_count > 0 {
                if backtick_count >= 3 {
                    in_fenced_code_block = !in_fenced_code_block;
                }
                backtick_count = 0;
            }
            let mut is_default = false;
            match self.token {
                Kind::AtToken => {
                    if in_fenced_code_block || !self.scanner.can_follow_jsdoc_at() {
                        if in_fenced_code_block {
                            state = JsdocState::SavingBackticks;
                        } else {
                            state = JsdocState::SavingComments;
                        }
                        push_comment!(self.scanner.token_text());
                    } else {
                        remove_trailing_whitespace(&mut comments);
                        if comments_pos == -1 {
                            comments_pos = self.node_pos();
                        }
                        let tag = self.parse_tag(&tags, indent);
                        if tags_pos == -1 {
                            tags_pos = tag.pos();
                        }
                        tags.push(tag);
                        tags_end = tag.end();
                        // NOTE: According to usejsdoc.org, a tag goes to end of line, except the last tag.
                        // Real-world comments may break this rule, so "BeginningOfLine" will not be a real line beginning
                        // for malformed examples like `/** @param {string} x @returns {number} the length */`
                        state = JsdocState::BeginningOfLine;
                        margin = -1;
                    }
                }
                Kind::NewLineTrivia => {
                    comments.push(self.scanner.token_text());
                    state = JsdocState::BeginningOfLine;
                    indent = 0;
                }
                Kind::AsteriskToken => {
                    let asterisk = self.scanner.token_text();
                    if state == JsdocState::SawAsterisk {
                        // If we've already seen an asterisk, then we can no longer parse a tag on this line
                        state = JsdocState::SavingComments;
                        push_comment!(asterisk);
                    } else {
                        if state != JsdocState::BeginningOfLine {
                            panic!("state must be BeginningOfLine");
                        }
                        // Ignore the first asterisk on a line
                        state = JsdocState::SawAsterisk;
                        indent += asterisk.len() as i32;
                    }
                }
                Kind::WhitespaceTrivia => {
                    if state == JsdocState::SavingComments || state == JsdocState::SavingBackticks {
                        panic!("whitespace shouldn't come from the scanner while saving top-level comment text");
                    }
                    // only collect whitespace if we're already saving comments or have just crossed the comment indent margin
                    let whitespace = self.scanner.token_text();
                    if margin > -1 && indent + whitespace.len() as i32 > margin {
                        let mut existing_indent = margin - indent;
                        if existing_indent < 0 {
                            existing_indent += whitespace.len() as i32;
                        }
                        if existing_indent < 0 {
                            existing_indent = 0;
                        }
                        comments.push(&whitespace[existing_indent as usize..]);
                    }
                    indent += whitespace.len() as i32;
                }
                Kind::EndOfFile => break,
                Kind::JSDocCommentTextToken => {
                    if state != JsdocState::SavingBackticks {
                        if in_fenced_code_block {
                            state = JsdocState::SavingBackticks;
                        } else {
                            state = JsdocState::SavingComments;
                        }
                    }
                    push_comment!(self.scanner.token_value());
                }
                Kind::BacktickToken => {
                    backtick_count += 1;
                    if state == JsdocState::SavingBackticks {
                        state = JsdocState::SavingComments;
                    } else {
                        state = JsdocState::SavingBackticks;
                    }
                    push_comment!(self.scanner.token_text());
                }
                Kind::OpenBraceToken => {
                    if in_fenced_code_block {
                        state = JsdocState::SavingBackticks;
                        push_comment!(self.scanner.token_text());
                    } else {
                        state = JsdocState::SavingComments;
                        let comment_end = self.scanner.token_full_start();
                        let link_start = self.scanner.token_end() - 1;
                        let link = self.parse_jsdoc_link(link_start);
                        if let Some(link) = link {
                            if link_end == start {
                                remove_leading_newlines(&mut comments);
                            }
                            let text = self.factory.new_jsdoc_text(alloc_slice(&comments));
                            let jsdoc_text = self.finish_node_with_end(text, link_end, comment_end);
                            comment_parts.push(jsdoc_text);
                            comment_parts.push(link);
                            comments.clear();
                            link_end = self.scanner.token_end();
                        } else {
                            is_default = true;
                        }
                    }
                }
                _ => is_default = true,
            }
            if is_default {
                // Anything else is doc comment text. We just save it. Because it
                // wasn't a tag, we can no longer parse a tag on this line until we hit the next
                // line break.
                if state != JsdocState::SavingBackticks {
                    if in_fenced_code_block {
                        state = JsdocState::SavingBackticks;
                    } else {
                        state = JsdocState::SavingComments;
                    }
                }
                push_comment!(self.scanner.token_text());
            }
            if state == JsdocState::SavingComments || state == JsdocState::SavingBackticks {
                self.next_jsdoc_comment_text_token(state == JsdocState::SavingBackticks);
            } else {
                self.next_token_jsdoc();
            }
        }

        if comments_pos == -1 {
            comments_pos = self.scanner.token_full_start();
        }

        if !comments.is_empty() {
            let last = comments.len() - 1;
            comments[last] = comments[last].trim_end();
            let text = self.factory.new_jsdoc_text(alloc_slice(&comments));
            let jsdoc_text = self.finish_node_with_end(text, link_end, comments_pos);
            comment_parts.push(jsdoc_text);
        }
        comments.clear();
        self.jsdoc_comments_space = comments; // Reuse this slice for further parses

        if !comment_parts.is_empty() && !tags.is_empty() && comments_pos == -1 {
            panic!("having parsed tags implies that the end of the comment span should be set");
        }

        let tags_node_list = if tags_pos != -1 {
            Some(self.new_node_list(TextRange::new(tags_pos, tags_end), &tags))
        } else {
            None
        };

        let comment_list = self.new_node_list(TextRange::new(start, comments_pos), &comment_parts);
        let jsdoc_comment = self.factory.new_jsdoc(comment_list, tags_node_list);
        self.finish_node_with_end(jsdoc_comment, full_start, end)
    }

    pub(crate) fn is_next_nonwhitespace_token_end_of_file(&mut self) -> bool {
        // We must use infinite lookahead, as there could be any number of newlines :(
        loop {
            self.next_token_jsdoc();
            if self.token == Kind::EndOfFile {
                return true;
            }
            if !(self.token == Kind::WhitespaceTrivia || self.token == Kind::NewLineTrivia) {
                return false;
            }
        }
    }

    pub(crate) fn skip_whitespace(&mut self) {
        if self.token == Kind::WhitespaceTrivia || self.token == Kind::NewLineTrivia {
            if self.look_ahead(Parser::is_next_nonwhitespace_token_end_of_file) {
                return;
                // Don't skip whitespace prior to EoF (or end of comment) - that shouldn't be included in any node's range
            }
        }
        while self.token == Kind::WhitespaceTrivia || self.token == Kind::NewLineTrivia {
            self.next_token_jsdoc();
        }
    }

    pub(crate) fn skip_whitespace_or_asterisk(&mut self) -> String {
        if self.token == Kind::WhitespaceTrivia || self.token == Kind::NewLineTrivia {
            if self.look_ahead(Parser::is_next_nonwhitespace_token_end_of_file) {
                return String::new();
                // Don't skip whitespace prior to EoF (or end of comment) - that shouldn't be included in any node's range
            }
        }

        let mut preceding_line_break = self.scanner.has_preceding_line_break();
        let mut seen_line_break = false;
        let mut indents: Vec<&'static str> = Vec::with_capacity(4);
        while (preceding_line_break && self.token == Kind::AsteriskToken)
            || self.token == Kind::WhitespaceTrivia
            || self.token == Kind::NewLineTrivia
        {
            indents.push(self.scanner.token_text());
            if self.token == Kind::NewLineTrivia {
                preceding_line_break = true;
                seen_line_break = true;
                indents.clear();
            } else if self.token == Kind::AsteriskToken {
                preceding_line_break = false;
            }
            self.next_token_jsdoc();
        }
        if seen_line_break {
            indents.concat()
        } else {
            String::new()
        }
    }

    pub(crate) fn parse_tag(&mut self, tags: &[P<Node>], margin: i32) -> P<Node> {
        if self.token != Kind::AtToken {
            panic!("should be called only at the start of a tag");
        }
        let start = self.scanner.token_start();
        self.next_token_jsdoc();

        let tag_name = self.parse_jsdoc_identifier_name(Some(&diagnostics::Identifier_expected));
        let indent_text = self.skip_whitespace_or_asterisk();

        let tag = match tag_name.text() {
            "implements" => self.parse_implements_tag(start, tag_name, margin, &indent_text),
            "augments" | "extends" => self.parse_augments_tag(start, tag_name, margin, &indent_text),
            "public" => self.parse_simple_tag(
                start,
                |p, tag_name, comments| p.factory.new_jsdoc_public_tag(tag_name, comments),
                tag_name,
                margin,
                &indent_text,
            ),
            "private" => self.parse_simple_tag(
                start,
                |p, tag_name, comments| p.factory.new_jsdoc_private_tag(tag_name, comments),
                tag_name,
                margin,
                &indent_text,
            ),
            "protected" => self.parse_simple_tag(
                start,
                |p, tag_name, comments| p.factory.new_jsdoc_protected_tag(tag_name, comments),
                tag_name,
                margin,
                &indent_text,
            ),
            "readonly" => self.parse_simple_tag(
                start,
                |p, tag_name, comments| p.factory.new_jsdoc_readonly_tag(tag_name, comments),
                tag_name,
                margin,
                &indent_text,
            ),
            "override" => self.parse_simple_tag(
                start,
                |p, tag_name, comments| p.factory.new_jsdoc_override_tag(tag_name, comments),
                tag_name,
                margin,
                &indent_text,
            ),
            "deprecated" => {
                self.has_deprecated_tag = true;
                self.parse_simple_tag(
                    start,
                    |p, tag_name, comments| p.factory.new_jsdoc_deprecated_tag(tag_name, comments),
                    tag_name,
                    margin,
                    &indent_text,
                )
            }
            "this" => self.parse_this_tag(start, tag_name, margin, &indent_text),
            "arg" | "argument" | "param" => {
                self.parse_parameter_or_property_tag(start, tag_name, PropertyLikeParse::Parameter, margin)
            }
            "return" | "returns" => self.parse_return_tag(tags, start, tag_name, margin, &indent_text),
            "template" => self.parse_template_tag(start, tag_name, margin, &indent_text),
            "type" => self.parse_type_tag(tags, start, tag_name, margin, &indent_text),
            "typedef" => self.parse_typedef_tag(start, tag_name, margin, &indent_text),
            "callback" => self.parse_callback_tag(start, tag_name, margin, &indent_text),
            "overload" => self.parse_overload_tag(start, tag_name, margin, &indent_text),
            "satisfies" => self.parse_satisfies_tag(start, tag_name, margin, &indent_text),
            "see" => self.parse_see_tag(start, tag_name, margin, &indent_text),
            "exception" | "throws" => self.parse_throws_tag(start, tag_name, margin, &indent_text),
            "import" => self.parse_import_tag(start, tag_name, margin, &indent_text),
            _ => self.parse_unknown_tag(start, tag_name, margin, &indent_text),
        };
        tag
    }

    pub(crate) fn parse_trailing_tag_comments(&mut self, pos: i32, end: i32, margin: i32, indent_text: &str) -> Option<P<NodeList>> {
        let mut margin = margin;
        // some tags, like typedef and callback, have already parsed their comments earlier
        if indent_text.is_empty() {
            margin += end - pos;
        }
        let mut initial_margin: &'static str = "";
        if margin < indent_text.len() as i32 {
            initial_margin = alloc_str(&indent_text[margin as usize..]);
        }
        self.parse_tag_comments(margin, Some(initial_margin))
    }

    pub(crate) fn parse_tag_comments(&mut self, indent: i32, initial_margin: Option<&'static str>) -> Option<P<NodeList>> {
        let mut indent = indent;
        let comments_pos = self.node_pos();
        let mut comments: Vec<&'static str> = std::mem::take(&mut self.jsdoc_tag_comments_space);
        let mut parts: Vec<P<Node>> = std::mem::take(&mut self.jsdoc_tag_comments_parts_space);
        let mut link_end: i32 = -1;
        let mut state = JsdocState::BeginningOfLine;
        let mut backtick_count = 0;
        let mut in_fenced_code_block = false;
        if indent < 0 {
            panic!("indent must be a natural number");
        }
        let mut margin: i32 = -1;
        macro_rules! push_comment {
            ($text:expr) => {{
                let text: &'static str = $text;
                if margin == -1 {
                    margin = indent;
                }
                comments.push(text);
                indent += text.len() as i32;
            }};
        }

        if let Some(initial_margin) = initial_margin {
            // jump straight to saving comments if there is some initial indentation
            if !initial_margin.is_empty() {
                push_comment!(initial_margin);
            }
            state = JsdocState::SawAsterisk;
        }
        let mut tok = self.token;
        loop {
            // Detect fenced code blocks by counting consecutive backtick tokens.
            // Three or more consecutive backticks toggle the fenced code block state.
            if tok != Kind::BacktickToken && backtick_count > 0 {
                if backtick_count >= 3 {
                    in_fenced_code_block = !in_fenced_code_block;
                }
                backtick_count = 0;
            }
            let mut is_default = false;
            match tok {
                Kind::NewLineTrivia => {
                    state = JsdocState::BeginningOfLine;
                    // don't use pushComment here because we want to keep the margin unchanged
                    comments.push(self.scanner.token_text());
                    indent = 0;
                }
                Kind::AtToken => {
                    if !in_fenced_code_block && self.scanner.can_follow_jsdoc_at() {
                        let pos = self.scanner.token_end() - 1;
                        self.scanner.reset_pos(pos);
                        break;
                    }
                    if in_fenced_code_block {
                        state = JsdocState::SavingBackticks;
                    } else {
                        state = JsdocState::SavingComments;
                    }
                    push_comment!(self.scanner.token_text());
                }
                Kind::EndOfFile => {
                    // Done
                    break;
                }
                Kind::WhitespaceTrivia => {
                    if state == JsdocState::SavingComments || state == JsdocState::SavingBackticks {
                        panic!("whitespace shouldn't come from the scanner while saving comment text");
                    }
                    let whitespace = self.scanner.token_text();
                    // if the whitespace crosses the margin, take only the whitespace that passes the margin
                    if margin > -1 && indent + whitespace.len() as i32 > margin {
                        comments.push(&whitespace[(margin - indent).max(0) as usize..]);
                        if in_fenced_code_block {
                            state = JsdocState::SavingBackticks;
                        } else {
                            state = JsdocState::SavingComments;
                        }
                    }
                    indent += whitespace.len() as i32;
                }
                Kind::OpenBraceToken => {
                    if in_fenced_code_block {
                        state = JsdocState::SavingBackticks;
                        push_comment!(self.scanner.token_text());
                    } else {
                        state = JsdocState::SavingComments;
                        let comment_end = self.scanner.token_full_start();
                        let link_start = self.scanner.token_end() - 1;
                        let link = self.parse_jsdoc_link(link_start);
                        if let Some(link) = link {
                            let comment_start = if link_end > -1 { link_end } else { comments_pos };
                            let text = self.factory.new_jsdoc_text(alloc_slice(&comments));
                            let text = self.finish_node_with_end(text, comment_start, comment_end);
                            parts.push(text);
                            parts.push(link);
                            comments.clear();
                            link_end = self.scanner.token_end();
                        } else {
                            push_comment!(self.scanner.token_text());
                        }
                    }
                }
                Kind::BacktickToken => {
                    backtick_count += 1;
                    if state == JsdocState::SavingBackticks {
                        state = JsdocState::SavingComments;
                    } else {
                        state = JsdocState::SavingBackticks;
                    }
                    push_comment!(self.scanner.token_text());
                }
                Kind::JSDocCommentTextToken => {
                    if state != JsdocState::SavingBackticks {
                        if in_fenced_code_block {
                            state = JsdocState::SavingBackticks;
                        } else {
                            state = JsdocState::SavingComments;
                        }
                        // leading identifiers start recording as well
                    }
                    push_comment!(self.scanner.token_value());
                }
                Kind::AsteriskToken => {
                    if state == JsdocState::BeginningOfLine {
                        // leading asterisks start recording on the *next* (non-whitespace) token
                        state = JsdocState::SawAsterisk;
                        indent += 1;
                    } else {
                        // record the * as a comment
                        is_default = true;
                    }
                }
                _ => is_default = true,
            }
            if is_default {
                if state != JsdocState::SavingBackticks {
                    if in_fenced_code_block {
                        state = JsdocState::SavingBackticks;
                    } else {
                        state = JsdocState::SavingComments;
                    }
                    // leading identifiers start recording as well
                }
                push_comment!(self.scanner.token_text());
            }
            if state == JsdocState::SavingComments || state == JsdocState::SavingBackticks {
                tok = self.next_jsdoc_comment_text_token(state == JsdocState::SavingBackticks);
            } else {
                tok = self.next_token_jsdoc();
            }
        }

        remove_leading_newlines(&mut comments);
        remove_trailing_whitespace(&mut comments);
        if !comments.is_empty() {
            let comment_start = if link_end > -1 { link_end } else { comments_pos };
            let text = self.factory.new_jsdoc_text(alloc_slice(&comments));
            let text = self.finish_node(text, comment_start);
            parts.push(text);
        }
        comments.clear();
        self.jsdoc_tag_comments_space = comments;

        let result = if !parts.is_empty() {
            let end = self.scanner.token_end();
            Some(self.new_node_list(TextRange::new(comments_pos, end), &parts))
        } else {
            None
        };
        parts.clear();
        self.jsdoc_tag_comments_parts_space = parts;
        result
    }

    pub(crate) fn parse_jsdoc_link(&mut self, start: i32) -> Option<P<Node>> {
        let state = self.mark();
        let Some(link_type) = self.parse_jsdoc_link_prefix() else {
            self.rewind(state);
            return None;
        };
        self.next_token_jsdoc();
        // start at token after link, then skip any whitespace
        self.skip_whitespace();
        let name = self.parse_jsdoc_link_name();
        let mut text: Vec<&'static str> = Vec::new();
        while self.token != Kind::CloseBraceToken && self.token != Kind::NewLineTrivia && self.token != Kind::EndOfFile {
            text.push(self.scanner.token_text());
            self.next_token_jsdoc(); // Couldn't this be nextTokenCommentJSDoc?
        }
        let text = alloc_vec(text);
        let create = match link_type {
            "link" => self.factory.new_jsdoc_link(name, text),
            "linkcode" => self.factory.new_jsdoc_link_code(name, text),
            _ => self.factory.new_jsdoc_link_plain(name, text),
        };
        let end = self.scanner.token_end();
        Some(self.finish_node_with_end(create, start, end))
    }

    pub(crate) fn parse_jsdoc_link_name(&mut self) -> Option<P<Node>> {
        if token_is_identifier_or_keyword(self.token) {
            let pos = self.node_pos();
            let mut name = self.parse_identifier_name();
            while self.parse_optional(Kind::DotToken) {
                let right = if self.token == Kind::PrivateIdentifier {
                    self.create_missing_identifier()
                } else {
                    self.parse_identifier_name()
                };
                let qualified = self.factory.new_qualified_name(name, right);
                name = self.finish_node(qualified, pos);
            }
            while self.token == Kind::PrivateIdentifier {
                self.scanner.re_scan_hash_token();
                self.report_scan_errors();
                self.next_token_jsdoc();
                let right = self.parse_identifier();
                let qualified = self.factory.new_qualified_name(name, right);
                name = self.finish_node(qualified, pos);
            }
            return Some(name);
        }
        None
    }

    pub(crate) fn parse_jsdoc_link_prefix(&mut self) -> Option<&'static str> {
        self.skip_whitespace_or_asterisk();
        if self.token == Kind::OpenBraceToken
            && self.next_token_jsdoc() == Kind::AtToken
            && token_is_identifier_or_keyword(self.next_token_jsdoc())
        {
            let kind = self.scanner.token_value();
            if is_jsdoc_link_tag(&kind) {
                return Some(kind);
            }
        }
        None
    }

    pub(crate) fn parse_unknown_tag(&mut self, start: i32, tag_name: P<Node>, indent: i32, indent_text: &str) -> P<Node> {
        let pos = self.node_pos();
        let comments = self.parse_trailing_tag_comments(start, pos, indent, indent_text);
        let node = self.factory.new_jsdoc_unknown_tag(tag_name, comments);
        self.finish_node(node, start)
    }

    pub(crate) fn try_parse_type_expression(&mut self) -> Option<P<Node>> {
        self.skip_whitespace_or_asterisk();
        if self.token == Kind::OpenBraceToken {
            Some(self.parse_jsdoc_type_expression(false /*mayOmitBraces*/))
        } else {
            None
        }
    }

    pub(crate) fn parse_bracket_name_in_property_and_param_tag(&mut self, target: PropertyLikeParse) -> (P<Node>, bool) {
        // Looking for something like '[foo]', 'foo', '[foo.bar]' or 'foo.bar'
        let is_bracketed = self.parse_optional_jsdoc(Kind::OpenBracketToken);
        if is_bracketed {
            self.skip_whitespace();
        }
        // a markdown-quoted name: `arg` is not legal jsdoc, but occurs in the wild
        let is_backquoted = self.parse_optional_jsdoc(Kind::BacktickToken);
        let name = self.parse_jsdoc_entity_name(if target == PropertyLikeParse::Parameter {
            None
        } else {
            Some(&diagnostics::Identifier_expected)
        });
        if is_backquoted {
            self.parse_expected_token_jsdoc(Kind::BacktickToken);
        }
        if is_bracketed {
            self.skip_whitespace();
            // May have an optional default, e.g. '[foo = 42]'
            if self.parse_optional_token(Kind::EqualsToken).is_some() {
                self.parse_expression();
            }

            self.parse_expected(Kind::CloseBracketToken);
        }

        (name, is_bracketed)
    }

    pub(crate) fn parse_parameter_or_property_tag(
        &mut self,
        start: i32,
        tag_name: P<Node>,
        target: PropertyLikeParse,
        indent: i32,
    ) -> P<Node> {
        let mut type_expression = self.try_parse_type_expression();
        let mut is_name_first = type_expression.is_none();
        self.skip_whitespace_or_asterisk();

        let (name, is_bracketed) = self.parse_bracket_name_in_property_and_param_tag(target);
        let indent_text = self.skip_whitespace_or_asterisk();

        if is_name_first && self.look_ahead(|p| p.parse_jsdoc_link_prefix().is_none()) {
            type_expression = self.try_parse_type_expression();
        }

        let pos = self.node_pos();
        let comment = self.parse_trailing_tag_comments(start, pos, indent, &indent_text);

        let nested_type_literal = self.parse_nested_type_literal(type_expression, name, target, indent);
        if nested_type_literal.is_some() {
            type_expression = nested_type_literal;
            is_name_first = true;
        }
        let kind = if target == PropertyLikeParse::Property { Kind::JSDocPropertyTag } else { Kind::JSDocParameterTag };
        let result = self.factory.new_jsdoc_parameter_or_property_tag(
            kind,
            tag_name,
            name,
            is_bracketed,
            type_expression,
            is_name_first,
            comment,
        );
        self.finish_node(result, start)
    }

    pub(crate) fn parse_nested_type_literal(
        &mut self,
        type_expression: Option<P<Node>>,
        name: P<Node>,
        target: PropertyLikeParse,
        indent: i32,
    ) -> Option<P<Node>> {
        if let Some(type_expression) = type_expression {
            if is_object_or_object_array_type_reference(type_expression.type_node().unwrap()) {
                let pos = self.node_pos();
                let mut children: Option<Vec<P<Node>>> = None;
                loop {
                    let state = self.mark();
                    let child = self.parse_child_parameter_or_property_tag(target, indent, Some(name));
                    let Some(child) = child else {
                        self.rewind(state);
                        break;
                    };
                    match child.kind {
                        Kind::JSDocParameterTag | Kind::JSDocPropertyTag => {
                            children.get_or_insert_with(Vec::new).push(child);
                        }
                        Kind::JSDocTemplateTag => {
                            self.parse_error_at_range(
                                child.tag_name().loc(),
                                &diagnostics::A_JSDoc_template_tag_may_not_follow_a_typedef_callback_or_overload_tag,
                                &[],
                            );
                        }
                        _ => {}
                    }
                }
                if let Some(children) = children {
                    let is_array_type = type_expression.type_node().unwrap().kind == Kind::ArrayType;
                    let literal = self.factory.new_jsdoc_type_literal(alloc_vec(children), is_array_type);
                    let literal = self.finish_node(literal, pos);
                    let node = self.factory.new_jsdoc_type_expression(literal);
                    return Some(self.finish_node(node, pos));
                }
            }
        }
        None
    }

    pub(crate) fn parse_return_tag(
        &mut self,
        previous_tags: &[P<Node>],
        start: i32,
        tag_name: P<Node>,
        indent: i32,
        indent_text: &str,
    ) -> P<Node> {
        if previous_tags.iter().any(|t| ast::is_jsdoc_return_tag(*t)) {
            let end = self.scanner.token_start();
            self.parse_error_at(tag_name.pos(), end, &diagnostics::X_0_tag_already_specified, &[&tag_name.text()]);
        }

        let type_expression = self.try_parse_type_expression();
        let pos = self.node_pos();
        let comments = self.parse_trailing_tag_comments(start, pos, indent, indent_text);
        let node = self.factory.new_jsdoc_return_tag(tag_name, type_expression, comments);
        self.finish_node(node, start)
    }

    // pass indent=-1 to skip parsing trailing comments (as when a type tag is nested in a typedef)
    pub(crate) fn parse_type_tag(
        &mut self,
        previous_tags: &[P<Node>],
        start: i32,
        tag_name: P<Node>,
        indent: i32,
        indent_text: &str,
    ) -> P<Node> {
        if previous_tags.iter().any(|t| ast::is_jsdoc_type_tag(*t)) {
            let end = self.scanner.token_start();
            self.parse_error_at(tag_name.pos(), end, &diagnostics::X_0_tag_already_specified, &[&tag_name.text()]);
        }

        let type_expression = self.parse_jsdoc_type_expression(true);
        let mut comments = None;
        if indent != -1 {
            let pos = self.node_pos();
            comments = self.parse_trailing_tag_comments(start, pos, indent, indent_text);
        }
        let node = self.factory.new_jsdoc_type_tag(tag_name, type_expression, comments);
        self.finish_node(node, start)
    }

    pub(crate) fn parse_see_tag(&mut self, start: i32, tag_name: P<Node>, indent: i32, indent_text: &str) -> P<Node> {
        let has_name_reference = self.is_identifier()
            && !self.source_text[self.scanner.token_end() as usize..].starts_with("://")
            || self.token == Kind::OpenBraceToken && self.look_ahead(Parser::next_token_is_identifier_or_keyword);
        let mut name_expression = None;
        if has_name_reference {
            name_expression = Some(self.parse_jsdoc_name_reference());
        }
        let pos = self.node_pos();
        let comments = self.parse_trailing_tag_comments(start, pos, indent, indent_text);
        let node = self.factory.new_jsdoc_see_tag(tag_name, name_expression, comments);
        self.finish_node(node, start)
    }

    pub(crate) fn parse_implements_tag(&mut self, start: i32, tag_name: P<Node>, margin: i32, indent_text: &str) -> P<Node> {
        let class_name = self.parse_expression_with_type_arguments_for_augments();
        let pos = self.node_pos();
        let comments = self.parse_trailing_tag_comments(start, pos, margin, indent_text);
        let node = self.factory.new_jsdoc_implements_tag(tag_name, class_name, comments);
        self.finish_node(node, start)
    }

    pub(crate) fn parse_augments_tag(&mut self, start: i32, tag_name: P<Node>, margin: i32, indent_text: &str) -> P<Node> {
        let class_name = self.parse_expression_with_type_arguments_for_augments();
        let pos = self.node_pos();
        let comments = self.parse_trailing_tag_comments(start, pos, margin, indent_text);
        let node = self.factory.new_jsdoc_augments_tag(tag_name, class_name, comments);
        self.finish_node(node, start)
    }

    pub(crate) fn parse_satisfies_tag(&mut self, start: i32, tag_name: P<Node>, margin: i32, indent_text: &str) -> P<Node> {
        let type_expression = self.parse_jsdoc_type_expression(false);
        let pos = self.node_pos();
        let comments = self.parse_trailing_tag_comments(start, pos, margin, indent_text);
        let node = self.factory.new_jsdoc_satisfies_tag(tag_name, type_expression, comments);
        self.finish_node(node, start)
    }

    pub(crate) fn parse_throws_tag(&mut self, start: i32, tag_name: P<Node>, margin: i32, indent_text: &str) -> P<Node> {
        let type_expression = self.try_parse_type_expression();
        let pos = self.node_pos();
        let comment = self.parse_trailing_tag_comments(start, pos, margin, indent_text);
        let node = self.factory.new_jsdoc_throws_tag(tag_name, type_expression, comment);
        self.finish_node(node, start)
    }

    pub(crate) fn parse_import_tag(&mut self, start: i32, tag_name: P<Node>, margin: i32, indent_text: &str) -> P<Node> {
        let after_import_tag_pos = self.scanner.token_full_start();

        let mut identifier = None;
        if self.is_identifier() {
            identifier = Some(self.parse_identifier());
        }

        let import_clause =
            self.try_parse_import_clause(identifier, after_import_tag_pos, Kind::TypeKeyword, true /*skipJSDocLeadingAsterisks*/);
        let module_specifier = self.parse_module_specifier();
        let attributes = self.try_parse_import_attributes();

        let pos = self.node_pos();
        let comments = self.parse_trailing_tag_comments(start, pos, margin, indent_text);
        let node = self.factory.new_jsdoc_import_tag(tag_name, import_clause, module_specifier, attributes, comments);
        self.finish_node(node, start)
    }

    pub(crate) fn parse_expression_with_type_arguments_for_augments(&mut self) -> P<Node> {
        let used_brace = self.parse_optional(Kind::OpenBraceToken);
        let pos = self.node_pos();
        let expression = self.parse_property_access_entity_name_expression();
        self.scanner.set_skip_jsdoc_leading_asterisks(true);
        let type_arguments = self.parse_type_arguments();
        self.scanner.set_skip_jsdoc_leading_asterisks(false);
        let node = self.factory.new_expression_with_type_arguments(expression, type_arguments);
        let node = self.finish_node(node, pos);
        if used_brace {
            self.skip_whitespace();
            self.parse_expected(Kind::CloseBraceToken);
        }
        node
    }

    pub(crate) fn parse_property_access_entity_name_expression(&mut self) -> P<Node> {
        let pos = self.node_pos();
        let mut node = self.parse_jsdoc_identifier_name(Some(&diagnostics::Identifier_expected));
        while self.parse_optional(Kind::DotToken) {
            let name = self.parse_jsdoc_identifier_name(Some(&diagnostics::Identifier_expected));
            let access = self.factory.new_property_access_expression(node, None, name, NodeFlags::None);
            node = self.finish_node(access, pos);
        }
        node
    }

    pub(crate) fn parse_simple_tag(
        &mut self,
        start: i32,
        create_tag: impl FnOnce(&mut Parser, P<Node>, Option<P<NodeList>>) -> P<Node>,
        tag_name: P<Node>,
        margin: i32,
        indent_text: &str,
    ) -> P<Node> {
        let pos = self.node_pos();
        let comments = self.parse_trailing_tag_comments(start, pos, margin, indent_text);
        let node = create_tag(self, tag_name, comments);
        self.finish_node(node, start)
    }

    pub(crate) fn parse_this_tag(&mut self, start: i32, tag_name: P<Node>, margin: i32, indent_text: &str) -> P<Node> {
        let type_expression = self.parse_jsdoc_type_expression(true);
        self.skip_whitespace();
        let pos = self.node_pos();
        let comments = self.parse_trailing_tag_comments(start, pos, margin, indent_text);
        let result = self.factory.new_jsdoc_this_tag(tag_name, type_expression, comments);
        self.finish_node(result, start)
    }

    pub(crate) fn parse_jsdoc_type_name_with_namespace(&mut self, nested: bool) -> Option<P<Node>> {
        let start = self.scanner.token_start();
        if !token_is_identifier_or_keyword(self.token) {
            return None;
        }
        let type_name_or_namespace_name = self.parse_jsdoc_identifier_name(None);
        if self.parse_optional_jsdoc(Kind::DotToken) {
            let body = self.parse_jsdoc_type_name_with_namespace(true /*nested*/);
            let jsdoc_namespace_node = self.factory.new_module_declaration(
                None,                  /*modifiers*/
                Kind::NamespaceKeyword, /*keyword*/
                type_name_or_namespace_name,
                None, /*attributes*/
                body,
            );
            if nested {
                jsdoc_namespace_node.set_flags(jsdoc_namespace_node.flags() | NodeFlags::NestedNamespace);
            }
            return Some(self.finish_node(jsdoc_namespace_node, start));
        }
        if nested {
            type_name_or_namespace_name.set_flags(type_name_or_namespace_name.flags() | NodeFlags::IdentifierIsInJSDocNamespace);
        }
        Some(type_name_or_namespace_name)
    }

    pub(crate) fn parse_typedef_tag(&mut self, start: i32, tag_name: P<Node>, indent: i32, indent_text: &str) -> P<Node> {
        let mut type_expression = self.try_parse_type_expression();
        self.skip_whitespace_or_asterisk();
        let mut full_name = self.parse_jsdoc_type_name_with_namespace(false /*nested*/);
        if full_name.is_none() {
            full_name = Some(self.parse_jsdoc_identifier_name(Some(&diagnostics::Identifier_expected)));
        }
        self.skip_whitespace();
        let mut comment = self.parse_tag_comments(indent, None);

        let mut end: i32 = -1;
        let mut has_children = false;
        if type_expression.is_none() || is_object_or_object_array_type_reference(type_expression.unwrap().type_node().unwrap()) {
            let mut child_type_tag: Option<P<Node>> = None;
            let mut jsdoc_property_tags: Vec<P<Node>> = Vec::new();
            loop {
                let state = self.mark();
                let child = self.parse_child_property_tag(indent);
                let Some(child) = child else {
                    self.rewind(state);
                    break;
                };
                has_children = true;
                match child.kind {
                    Kind::JSDocTemplateTag => {
                        self.parse_error_at_range(
                            child.tag_name().loc(),
                            &diagnostics::A_JSDoc_template_tag_may_not_follow_a_typedef_callback_or_overload_tag,
                            &[],
                        );
                    }
                    Kind::JSDocTypeTag => {
                        if child_type_tag.is_none() {
                            child_type_tag = Some(child);
                        } else {
                            let last_error = self.parse_error_at_current_token(
                                &diagnostics::A_JSDoc_typedef_comment_may_not_contain_multiple_type_tags,
                                &[],
                            );
                            if let Some(last_error) = last_error {
                                let related = ast::new_diagnostic(
                                    None,
                                    TextRange::new(0, 0),
                                    &diagnostics::The_tag_was_first_specified_here,
                                    &[],
                                );
                                last_error.add_related_info(related);
                            }
                        }
                    }
                    _ => jsdoc_property_tags.push(child),
                }
            }
            if has_children {
                let is_array_type = type_expression.is_some_and(|t| t.type_node().unwrap().kind == Kind::ArrayType);
                let first_pos = jsdoc_property_tags.first().map(|t| t.pos());
                let jsdoc_type_literal = self.factory.new_jsdoc_type_literal(alloc_vec(jsdoc_property_tags), is_array_type);
                let child_type_expression = child_type_tag.map(|t| t.as_jsdoc_type_tag().type_expression);
                if let Some(child_type_expression) =
                    child_type_expression.filter(|t| !is_object_or_object_array_type_reference(t.type_node().unwrap()))
                {
                    type_expression = Some(child_type_expression);
                } else {
                    // !!! This differs from Strada but prevents a crash
                    let pos = first_pos.unwrap_or(start);
                    type_expression = Some(self.finish_node(jsdoc_type_literal, pos));
                }
                end = type_expression.unwrap().end();
            }
        }

        // Only include the characters between the name end and the next token if a comment was actually parsed out - otherwise it's just whitespace
        if end == -1 {
            if has_children && type_expression.is_some() {
                end = type_expression.unwrap().end();
            } else if comment.is_some() {
                end = self.node_pos();
            } else if let Some(full_name) = full_name {
                end = full_name.end();
            } else if let Some(type_expression) = type_expression {
                end = type_expression.end();
            } else {
                end = tag_name.end();
            }
        }

        if comment.is_none() {
            comment = self.parse_trailing_tag_comments(start, end, indent, indent_text);
        }

        let typedef_tag = self.factory.new_jsdoc_typedef_tag(tag_name, type_expression, full_name, comment);
        let typedef_tag = self.finish_node_with_end(typedef_tag, start, end);
        if let Some(type_expression) = type_expression {
            type_expression.set_parent(Some(typedef_tag)); // forcibly overwrite parent potentially set by inner type expression parse
        }
        typedef_tag
    }

    pub(crate) fn parse_callback_tag_parameters(&mut self, indent: i32) -> P<NodeList> {
        let mut parameters: Vec<P<Node>> = Vec::new();
        let pos = self.node_pos();
        loop {
            let state = self.mark();
            let child = self.parse_child_parameter_or_property_tag(PropertyLikeParse::CallbackParameter, indent, None);
            let Some(child) = child else {
                self.rewind(state);
                break;
            };
            if child.kind == Kind::JSDocTemplateTag {
                self.parse_error_at_range(
                    child.tag_name().loc(),
                    &diagnostics::A_JSDoc_template_tag_may_not_follow_a_typedef_callback_or_overload_tag,
                    &[],
                );
            } else {
                parameters.push(child);
            }
        }
        let end = self.node_pos();
        self.new_node_list(TextRange::new(pos, end), &parameters)
    }

    pub(crate) fn parse_jsdoc_signature(&mut self, start: i32, indent: i32) -> P<Node> {
        let parameters = self.parse_callback_tag_parameters(indent);
        let mut return_tag = None;
        let state = self.mark();
        if self.parse_optional_jsdoc(Kind::AtToken) {
            let tag = self.parse_tag(&[], indent);
            if tag.kind == Kind::JSDocReturnTag {
                return_tag = Some(tag);
            }
        }
        if return_tag.is_none() {
            self.rewind(state);
        }
        let node = self.factory.new_jsdoc_signature(None, parameters, return_tag);
        self.finish_node(node, start)
    }

    pub(crate) fn parse_callback_tag(&mut self, start: i32, tag_name: P<Node>, indent: i32, indent_text: &str) -> P<Node> {
        let mut full_name = self.parse_jsdoc_type_name_with_namespace(false /*nested*/);
        if full_name.is_none() {
            full_name = Some(self.parse_jsdoc_identifier_name(Some(&diagnostics::Identifier_expected)));
        }
        self.skip_whitespace();
        let mut comment = self.parse_tag_comments(indent, None);
        let pos = self.node_pos();
        let type_expression = self.parse_jsdoc_signature(pos, indent);
        if comment.is_none() {
            let pos = self.node_pos();
            comment = self.parse_trailing_tag_comments(start, pos, indent, indent_text);
        }
        let end = if comment.is_some() { self.node_pos() } else { type_expression.end() };
        let node = self.factory.new_jsdoc_callback_tag(tag_name, type_expression, full_name, comment);
        self.finish_node_with_end(node, start, end)
    }

    pub(crate) fn parse_overload_tag(&mut self, start: i32, tag_name: P<Node>, indent: i32, indent_text: &str) -> P<Node> {
        self.skip_whitespace();
        let mut comment = self.parse_tag_comments(indent, None);
        let type_expression = self.parse_jsdoc_signature(start, indent);
        if comment.is_none() {
            let pos = self.node_pos();
            comment = self.parse_trailing_tag_comments(start, pos, indent, indent_text);
        }
        let end = if comment.is_some() { self.node_pos() } else { type_expression.end() };
        let node = self.factory.new_jsdoc_overload_tag(tag_name, type_expression, comment);
        self.finish_node_with_end(node, start, end)
    }

    pub(crate) fn parse_child_property_tag(&mut self, indent: i32) -> Option<P<Node>> {
        self.parse_child_parameter_or_property_tag(PropertyLikeParse::Property, indent, None)
    }

    pub(crate) fn parse_child_parameter_or_property_tag(
        &mut self,
        target: PropertyLikeParse,
        indent: i32,
        name: Option<P<Node>>,
    ) -> Option<P<Node>> {
        let mut can_parse_tag = true;
        let mut seen_asterisk = false;
        loop {
            match self.next_token_jsdoc() {
                Kind::AtToken => {
                    if can_parse_tag && self.scanner.can_follow_jsdoc_at() {
                        let child = self.try_parse_child_tag(target, indent);
                        if let (Some(child), Some(name)) = (child, name) {
                            if (child.kind == Kind::JSDocParameterTag || child.kind == Kind::JSDocPropertyTag)
                                && (ast::is_identifier(child.name().unwrap())
                                    || !texts_equal(name, child.name().unwrap().as_qualified_name().left))
                            {
                                return None;
                            }
                        }
                        return child;
                    }
                    seen_asterisk = false;
                }
                Kind::NewLineTrivia => {
                    can_parse_tag = true;
                    seen_asterisk = false;
                }
                Kind::AsteriskToken => {
                    if seen_asterisk {
                        can_parse_tag = false;
                    }
                    seen_asterisk = true;
                }
                Kind::Identifier => {
                    can_parse_tag = false;
                }
                Kind::EndOfFile => {
                    return None;
                }
                _ => {}
            }
        }
    }

    pub(crate) fn try_parse_child_tag(&mut self, target: PropertyLikeParse, indent: i32) -> Option<P<Node>> {
        if self.token != Kind::AtToken {
            panic!("should only be called when at @");
        }
        let start = self.scanner.token_full_start();
        self.next_token_jsdoc();

        let tag_name = self.parse_jsdoc_identifier_name(Some(&diagnostics::Identifier_expected));
        let indent_text = self.skip_whitespace_or_asterisk();
        let mut t = PropertyLikeParse::empty();
        match tag_name.text() {
            "type" => {
                if target == PropertyLikeParse::Property {
                    return Some(self.parse_type_tag(&[], start, tag_name, -1, ""));
                }
            }
            "prop" | "property" => {
                t = PropertyLikeParse::Property;
            }
            "arg" | "argument" | "param" => {
                t = PropertyLikeParse::Parameter | PropertyLikeParse::CallbackParameter;
            }
            "template" => {
                return Some(self.parse_template_tag(start, tag_name, indent, &indent_text));
            }
            "this" => {
                return Some(self.parse_this_tag(start, tag_name, indent, &indent_text));
            }
            _ => return None,
        }
        if !target.intersects(t) {
            return None;
        }
        Some(self.parse_parameter_or_property_tag(start, tag_name, target, indent))
    }

    pub(crate) fn parse_template_tag_type_parameter(&mut self) -> Option<P<Node>> {
        let type_parameter_pos = self.node_pos();
        let is_bracketed = self.parse_optional_jsdoc(Kind::OpenBracketToken);
        if is_bracketed {
            self.skip_whitespace();
        }

        let modifiers = self.parse_modifiers_ex(false, true /*permitConstAsModifier*/, false);
        let name = self.parse_jsdoc_identifier_name(Some(
            &diagnostics::Unexpected_token_A_type_parameter_name_was_expected_without_curly_braces,
        ));
        let mut default_type = None;
        if is_bracketed {
            self.skip_whitespace();
            self.parse_expected(Kind::EqualsToken);
            let save_context_flags = self.context_flags;
            self.set_context_flags(NodeFlags::JSDoc, true);
            default_type = Some(self.parse_jsdoc_type());
            self.context_flags = save_context_flags;
            self.parse_expected(Kind::CloseBracketToken);
        }

        if ast::node_is_missing(Some(name)) {
            return None;
        }
        let node = self.factory.new_type_parameter_declaration(modifiers, name, None /*constraint*/, None /*expression*/, default_type);
        Some(self.finish_node(node, type_parameter_pos))
    }

    pub(crate) fn parse_template_tag_type_parameters(&mut self) -> P<NodeList> {
        let mut nodes: Vec<P<Node>> = Vec::new();
        loop {
            // do-while loop
            self.skip_whitespace();
            let node = self.parse_template_tag_type_parameter();
            if let Some(node) = node {
                nodes.push(node);
            }
            self.skip_whitespace_or_asterisk();
            if !self.parse_optional_jsdoc(Kind::CommaToken) {
                break;
            }
        }
        // Go builds a zero-valued ast.TypeParameterList{} literal, whose Loc is (0, 0).
        self.new_node_list(TextRange::new(0, 0), &nodes)
    }

    pub(crate) fn parse_template_tag(&mut self, start: i32, tag_name: P<Node>, indent: i32, indent_text: &str) -> P<Node> {
        // The template tag looks like one of the following:
        //   @template T,U,V
        //   @template {Constraint} T
        //
        // According to the [closure docs](https://github.com/google/closure-compiler/wiki/Generic-Types#multiple-bounded-template-types):
        //   > Multiple bounded generics cannot be declared on the same line. For the sake of clarity, if multiple templates share the same
        //   > type bound they must be declared on separate lines.
        //
        // TODO: Determine whether we should enforce this in the checker.
        // TODO: Consider moving the `constraint` to the first type parameter as we could then remove `getEffectiveConstraintOfTypeParameter`.
        // TODO: Consider only parsing a single type parameter if there is a constraint.
        let mut constraint = None;
        if self.token == Kind::OpenBraceToken {
            constraint = Some(self.parse_jsdoc_type_expression(false));
        }
        let type_parameters = self.parse_template_tag_type_parameters();
        let pos = self.node_pos();
        let comments = self.parse_trailing_tag_comments(start, pos, indent, indent_text);
        let result = self.factory.new_jsdoc_template_tag(tag_name, constraint, type_parameters, comments);
        self.finish_node(result, start)
    }

    pub(crate) fn parse_optional_jsdoc(&mut self, t: Kind) -> bool {
        if self.token == t {
            self.next_token_jsdoc();
            return true;
        }
        false
    }

    pub(crate) fn parse_jsdoc_entity_name(&mut self, diagnostic_message: Option<&'static Message>) -> P<Node> {
        let mut entity = self.parse_jsdoc_identifier_name(diagnostic_message);
        if self.parse_optional(Kind::OpenBracketToken) {
            self.parse_expected(Kind::CloseBracketToken);
            // Note that y[] is accepted as an entity name, but the postfix brackets are not saved for checking.
            // Technically usejsdoc.org requires them for specifying a property of a type equivalent to Array<{ x: ...}>
            // but it's not worth it to enforce that restriction.
        }
        while self.parse_optional(Kind::DotToken) {
            let name = self.parse_jsdoc_identifier_name(Some(&diagnostics::Identifier_expected));
            if self.parse_optional(Kind::OpenBracketToken) {
                self.parse_expected(Kind::CloseBracketToken);
            }
            let pos = entity.pos();
            let qualified = self.factory.new_qualified_name(entity, name);
            entity = self.finish_node(qualified, pos);
        }
        entity
    }

    pub(crate) fn parse_jsdoc_identifier_name(&mut self, diagnostic_message: Option<&'static Message>) -> P<Node> {
        if !token_is_identifier_or_keyword(self.token) {
            if let Some(diagnostic_message) = diagnostic_message {
                self.parse_error_at_current_token(diagnostic_message, &[]);
            } else if is_reserved_word(self.token) {
                let text = self.scanner.token_text();
                self.parse_error_at_current_token(&diagnostics::Identifier_expected_0_is_a_reserved_word_that_cannot_be_used_here, &[&text]);
            }
            let identifier = self.new_identifier("");
            let pos = self.node_pos();
            return self.finish_node(identifier, pos);
        }
        let pos = self.scanner.token_start();
        let end = self.scanner.token_end();
        let text = self.scanner.token_value();
        self.next_token_jsdoc();
        let identifier = self.new_identifier(text);
        self.finish_node_with_end(identifier, pos, end)
    }
}

fn remove_leading_newlines(comments: &mut Vec<&'static str>) {
    let mut i = 0;
    while i < comments.len() && comments[i].trim_start_matches(['\r', '\n']).is_empty() {
        i += 1;
    }
    comments.drain(..i);
}

fn trim_end(s: &str) -> &str {
    s.trim_end_matches(|c: char| stringutil::is_white_space_like(c))
}

fn remove_trailing_whitespace(comments: &mut Vec<&'static str>) {
    let mut end = comments.len();
    for i in (0..comments.len()).rev() {
        let trimmed = trim_end(comments[i]);
        if trimmed.is_empty() {
            end = i;
        } else {
            comments[i] = trimmed;
            break;
        }
    }
    comments.truncate(end);
}

fn is_jsdoc_link_tag(kind: &str) -> bool {
    kind == "link" || kind == "linkcode" || kind == "linkplain"
}

fn is_object_or_object_array_type_reference(node: P<Node>) -> bool {
    match node.kind {
        Kind::ObjectKeyword => true,
        Kind::ArrayType => is_object_or_object_array_type_reference(node.as_array_type_node().element_type),
        _ => {
            if ast::is_type_reference_node(node) {
                let ref_ = node.as_type_reference_node();
                return ast::is_identifier(ref_.type_name) && ref_.type_name.text() == "Object" && ref_.type_arguments.is_none();
            }
            false
        }
    }
}

fn texts_equal(a: P<Node>, b: P<Node>) -> bool {
    let mut a = a;
    let mut b = b;
    while !ast::is_identifier(a) || !ast::is_identifier(b) {
        if !ast::is_identifier(a) && !ast::is_identifier(b) && a.as_qualified_name().right.text() == b.as_qualified_name().right.text() {
            a = a.as_qualified_name().left;
            b = b.as_qualified_name().left;
        } else {
            return false;
        }
    }
    a.text() == b.text()
}
