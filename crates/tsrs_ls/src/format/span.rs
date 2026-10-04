use std::cell::{Cell, RefCell};
use std::rc::Rc;

use tsrs_ast::{self as ast, Diagnostic, Kind, Node, NodeFlags, NodeList, NodeVisitorHooks, SourceFile};
use tsrs_core::stringutil::{decode_rune, is_white_space_single_line};
use tsrs_core::{TextChange, TextRange, P};
use tsrs_scanner as scanner;

use super::*;
use crate::astnav;
use crate::lsutil::FormatCodeSettings;

// span.go:20
/* find node that fully contains given text range */
pub(crate) fn find_enclosing_node(r: TextRange, source_file: P<SourceFile>) -> P<Node> {
    fn find(n: P<Node>, r: TextRange, source_file: P<SourceFile>) -> P<Node> {
        let mut candidate: Option<P<Node>> = None;
        n.for_each_child(&mut |c| {
            if c.flags().intersects(NodeFlags::Reparsed) {
                return false;
            }
            if r.contained_by(with_token_start(c, source_file)) {
                candidate = Some(c);
                return true;
            }
            false
        });
        if let Some(candidate) = candidate {
            return find(candidate, r, source_file);
        }

        n
    }
    find(source_file.as_node(), r, source_file)
}

// span.go:51
/*
 * Start of the original range might fall inside the comment - scanner will not yield appropriate results
 * This function will look for token that is located before the start of target range
 * and return its end as start position for the scanner.
 */
pub(crate) fn get_scan_start_position(enclosing_node: P<Node>, original_range: TextRange, source_file: P<SourceFile>) -> i32 {
    let adjusted = with_token_start(enclosing_node, source_file);
    let start = adjusted.pos();
    if start == original_range.pos() && enclosing_node.end() == original_range.end() {
        return start;
    }

    // exclude JSDoc so the scan never starts inside a JSDoc comment
    let preceding_token = astnav::find_preceding_token_ex(source_file, original_range.pos(), None /*startNode*/, true /*excludeJSDoc*/);
    let Some(preceding_token) = preceding_token else {
        // no preceding token found - start from the beginning of enclosing node
        return enclosing_node.pos();
    };

    // preceding token ends after the start of original range (i.e when originalRange.pos falls in the middle of literal)
    // start from the beginning of enclosingNode to handle the entire 'originalRange'
    if preceding_token.end() >= original_range.pos() {
        return enclosing_node.pos();
    }

    preceding_token.end()
}

// span.go:89
/*
 * For cases like
 * if (a ||
 *     b ||$
 *     c) {...}
 * If we hit Enter at $ we want line '    b ||' to be indented.
 * Formatting will be applied to the last two lines.
 * Node that fully encloses these lines is binary expression 'a ||...'.
 * Initial indentation for this node will be 0.
 * Binary expressions don't introduce new indentation scopes, however it is possible
 * that some parent node on the same line does - like if statement in this case.
 * Note that we are considering parents only from the same line with initial node -
 * if parent is on the different line - its delta was already contributed
 * to the initial indentation.
 */
pub(crate) fn get_own_or_inherited_delta(n: P<Node>, options: &FormatCodeSettings, source_file: P<SourceFile>) -> i32 {
    let mut previous_line = -1;
    let mut child: Option<P<Node>> = None;
    let mut n = Some(n);
    while let Some(node) = n {
        let line = scanner::get_ecma_line_of_position(source_file.get(), with_token_start(node, source_file).pos());
        if previous_line != -1 && line != previous_line {
            break;
        }

        if should_indent_child_node(options, node, child, Some(source_file), false) {
            return options.indent_size; // !!! nil check???
        }

        previous_line = line;
        child = Some(node);
        n = node.parent();
    }
    0
}

// span.go:109
fn range_has_no_errors(_: TextRange) -> bool {
    false
}

pub(crate) type RangeContainsError = Box<dyn FnMut(TextRange) -> bool>;

// span.go:113
pub(crate) fn prepare_range_contains_error_function(errors: &[P<Diagnostic>], original_range: TextRange) -> RangeContainsError {
    if errors.is_empty() {
        return Box::new(range_has_no_errors);
    }

    // pick only errors that fall in range
    let mut sorted: Vec<P<Diagnostic>> = errors.iter().copied().filter(|d| original_range.overlaps(d.loc())).collect();
    if sorted.is_empty() {
        return Box::new(range_has_no_errors);
    }
    sorted.sort_by_key(|d| d.pos());

    let mut index = 0;
    Box::new(move |r: TextRange| -> bool {
        // in current implementation sequence of arguments [r1, r2...] is monotonically increasing.
        // 'index' tracks the index of the most recent error that was checked.
        loop {
            if index >= sorted.len() {
                // all errors in the range were already checked -> no error in specified range
                return false;
            }

            let err = sorted[index];

            if r.end() <= err.pos() {
                // specified range ends before the error referred by 'index' - no error in range
                return false;
            }

            if r.overlaps(err.loc()) {
                // specified range overlaps with error range
                return true;
            }

            index += 1;
        }
    })
}

// span.go:155
pub(crate) struct FormatSpanWorker {
    original_range: TextRange,
    enclosing_node: P<Node>,
    initial_indentation: i32,
    delta: i32,
    request_kind: FormatRequestKind,
    range_contains_error: RangeContainsError,
    source_file: P<SourceFile>,

    ctx: FormatContext,

    pub(crate) formatting_scanner: Option<FormattingScanner>,
    formatting_context: Option<FormattingContext>,

    edits: Vec<TextChange>,
    previous_range: TextRangeWithKind,
    previous_range_trivia_end: i32,
    previous_parent: Option<P<Node>>,
    previous_range_start_line: i32,

    child_context_node: Option<P<Node>>,
    last_indented_line: i32,
    indentation_on_last_indented_line: i32,

    visiting_node: Option<P<Node>>,
    visiting_indenter: Option<DynamicIndenterRef>,
    visiting_node_start_line: i32,
    visiting_undecorated_node_start_line: i32,

    current_rules: Vec<&'static RuleImpl>,
}

// span.go:188
pub(crate) fn new_format_span_worker(
    ctx: &FormatContext,
    original_range: TextRange,
    enclosing_node: P<Node>,
    initial_indentation: i32,
    delta: i32,
    request_kind: FormatRequestKind,
    range_contains_error: RangeContainsError,
    source_file: P<SourceFile>,
) -> FormatSpanWorker {
    FormatSpanWorker {
        ctx: ctx.clone(),
        original_range,
        enclosing_node,
        initial_indentation,
        delta,
        request_kind,
        range_contains_error,
        source_file,
        formatting_scanner: None,
        formatting_context: None,
        edits: Vec::new(),
        previous_range: TextRangeWithKind::default(),
        previous_range_trivia_end: 0,
        previous_parent: None,
        previous_range_start_line: 0,
        child_context_node: None,
        last_indented_line: 0,
        indentation_on_last_indented_line: 0,
        visiting_node: None,
        visiting_indenter: None,
        visiting_node_start_line: 0,
        visiting_undecorated_node_start_line: 0,
        current_rules: Vec::with_capacity(32), // increaseInsertionIndex should assert there are no more than 32 rules in a given bucket
    }
}

// span.go:211
fn get_non_decorator_token_pos_of_node(node: P<Node>, file: Option<P<SourceFile>>) -> i32 {
    let mut last_decorator: Option<P<Node>> = None;
    if ast::has_decorators(node) {
        last_decorator = tsrs_core::find_last(node.modifier_nodes(), |&n| ast::is_decorator(n));
    }
    let file = match file {
        Some(file) => file,
        None => ast::get_source_file_of_node(node).unwrap(),
    };
    let Some(last_decorator) = last_decorator else {
        return with_token_start(node, file).pos();
    };
    scanner::skip_trivia(file.text(), last_decorator.end())
}

// Go walks a node's children with `node.VisitEachChild(w.visitor)`, whose callbacks re-enter the worker. The Rust
// visitor's callbacks cannot borrow the worker, so the children are first collected in VisitEachChild order (single
// nodes through `visit`, node lists through the `VisitNodes` hook, exactly as Go's visitor dispatches them) and then
// processed in that order. Processing never changes the node's children, so the order and the set are the same.
enum VisitedChild {
    Node(P<Node>),
    Nodes(P<NodeList>),
}

fn collect_visited_children(node: P<Node>) -> Vec<VisitedChild> {
    let items: Rc<RefCell<Vec<VisitedChild>>> = Rc::new(RefCell::new(Vec::new()));
    let visit_items = Rc::clone(&items);
    let nodes_items = Rc::clone(&items);
    let mut visitor = ast::new_node_visitor(
        Some(Rc::new(move |_: &mut ast::NodeVisitor, child: P<Node>| {
            visit_items.borrow_mut().push(VisitedChild::Node(child));
            Some(child)
        })),
        None,
        NodeVisitorHooks {
            visit_nodes: Some(Rc::new(move |nodes: Option<P<NodeList>>, _: &mut ast::NodeVisitor| {
                if let Some(list) = nodes {
                    nodes_items.borrow_mut().push(VisitedChild::Nodes(list));
                }
                nodes
            })),
            ..Default::default()
        },
    );
    node.visit_each_child(&mut visitor);
    items.take()
}

impl FormatSpanWorker {
    fn fs(&mut self) -> &mut FormattingScanner {
        self.formatting_scanner.as_mut().unwrap()
    }

    fn fs_ref(&self) -> &FormattingScanner {
        self.formatting_scanner.as_ref().unwrap()
    }

    fn fc(&self) -> &FormattingContext {
        self.formatting_context.as_ref().unwrap()
    }

    // span.go:225
    pub(crate) fn execute(&mut self, s: FormattingScanner) -> Vec<TextChange> {
        self.formatting_scanner = Some(s);
        self.indentation_on_last_indented_line = -1;
        self.last_indented_line = -1;
        let opt = Rc::new(get_format_code_settings_from_context(&self.ctx));
        self.formatting_context = Some(new_formatting_context(self.source_file, self.request_kind, Rc::clone(&opt)));
        // formatting context is used by rules provider

        self.fs().advance();

        if self.fs_ref().is_on_token() {
            let start_line =
                scanner::get_ecma_line_of_position(self.source_file.get(), with_token_start(self.enclosing_node, self.source_file).pos());
            let mut undecorated_start_line = start_line;
            if ast::has_decorators(self.enclosing_node) {
                undecorated_start_line = scanner::get_ecma_line_of_position(
                    self.source_file.get(),
                    get_non_decorator_token_pos_of_node(self.enclosing_node, Some(self.source_file)),
                );
            }

            self.process_node(
                self.enclosing_node,
                Some(self.enclosing_node),
                start_line,
                undecorated_start_line,
                self.initial_indentation,
                self.delta,
            );
        }

        // Leading trivia items get attached to and processed with the token that proceeds them. If the
        // range ends in the middle of some leading trivia, the token that proceeds them won't be in the
        // range and thus won't get processed. So we process those remaining trivia items here.
        let remaining_trivia = self.fs_ref().get_current_leading_trivia();
        if !remaining_trivia.is_empty() {
            let mut indentation = self.initial_indentation;
            if node_will_indent_child(&self.fc().options, self.enclosing_node, None, Some(self.source_file), false) {
                indentation += opt.indent_size; // !!! TODO: nil check???
            }

            self.indent_trivia_items(&remaining_trivia, indentation, true, &mut |w: &mut FormatSpanWorker, item: TextRangeWithKind| {
                let (start_line, start_char) = scanner::get_ecma_line_and_byte_offset_of_position(w.source_file.get(), item.loc.pos());
                w.process_range(item, start_line, start_char, Some(w.enclosing_node), Some(w.enclosing_node), None);
                w.insert_indentation(item.loc.pos(), indentation, false);
            });

            if opt.trim_trailing_whitespace.is_true() {
                self.trim_trailing_whitespaces_for_remaining_range(&remaining_trivia);
            }
        }

        if self.previous_range != new_text_range_with_kind(0, 0, Kind::Unknown) && self.fs_ref().get_token_full_start() >= self.original_range.end() {
            // Formatting edits happen by looking at pairs of contiguous tokens (see `processPair`),
            // typically inserting or deleting whitespace between them. The recursive `processNode`
            // logic above bails out as soon as it encounters a token that is beyond the end of the
            // range we're supposed to format (or if we reach the end of the file). But this potentially
            // leaves out an edit that would occur *inside* the requested range but cannot be discovered
            // without looking at one token *beyond* the end of the range: consider the line `x = { }`
            // with a selection from the beginning of the line to the space inside the curly braces,
            // inclusive. We would expect a format-selection would delete the space (if rules apply),
            // but in order to do that, we need to process the pair ["{", "}"], but we stopped processing
            // just before getting there. This block handles this trailing edit.
            let mut token_info = TextRangeWithKind::default();
            if self.fs_ref().is_on_eof() {
                token_info = self.fs_ref().read_eof_token_range();
            } else if self.fs_ref().is_on_token() {
                let enclosing_node = self.enclosing_node;
                token_info = self.fs().read_token_info(enclosing_node).token;
            }

            if token_info.loc.pos() == self.previous_range_trivia_end {
                // We need to check that tokenInfo and previousRange are contiguous: the `originalRange`
                // may have ended in the middle of a token, which means we will have stopped formatting
                // on that token, leaving `previousRange` pointing to the token before it, but already
                // having moved the formatting scanner (where we just got `tokenInfo`) to the next token.
                // If this happens, our supposed pair [previousRange, tokenInfo] actually straddles the
                // token that intersects the end of the range we're supposed to format, so the pair will
                // produce bogus edits if we try to `processPair`. Recall that the point of this logic is
                // to perform a trailing edit at the end of the selection range: but there can be no valid
                // edit in the middle of a token where the range ended, so if we have a non-contiguous
                // pair here, we're already done and we can ignore it.
                let mut parent = astnav::find_preceding_token(self.source_file, token_info.loc.end());
                if let Some(p) = parent {
                    parent = p.parent();
                }
                if parent.is_none() {
                    parent = self.previous_parent;
                }
                let line = scanner::get_ecma_line_of_position(self.source_file.get(), token_info.loc.pos());
                self.process_pair(
                    token_info,
                    line,
                    parent,
                    self.previous_range,
                    self.previous_range_start_line,
                    self.previous_parent,
                    parent,
                    None,
                );
            }
        }

        std::mem::take(&mut self.edits)
    }

    // span.go:334
    #[allow(clippy::too_many_arguments)]
    fn process_child_node(
        &mut self,
        node: P<Node>,
        _indenter: Option<DynamicIndenterRef>,
        _node_start_line: i32,
        _undecorated_node_start_line: i32,
        child: P<Node>,
        mut inherited_indentation: i32,
        parent: P<Node>,
        parent_dynamic_indentation: DynamicIndenterRef,
        parent_start_line: i32,
        undecorated_parent_start_line: i32,
        is_list_item: bool,
        is_first_list_item: bool,
    ) -> i32 {
        assert!(!ast::node_is_synthesized(child));

        if ast::node_is_missing(child) || child.flags().intersects(NodeFlags::Reparsed) {
            return inherited_indentation;
        }
        let child_start_pos = scanner::get_token_pos_of_node(child, self.source_file, false);
        let child_start_line = scanner::get_ecma_line_of_position(self.source_file.get(), child_start_pos);

        let mut undecorated_child_start_line = child_start_line;
        if ast::has_decorators(child) {
            undecorated_child_start_line =
                scanner::get_ecma_line_of_position(self.source_file.get(), get_non_decorator_token_pos_of_node(child, Some(self.source_file)));
        }

        let is_error_member_list_element = child.flags().intersects(NodeFlags::ThisNodeHasError) && is_member_list_element(parent, child);
        // if child is a list item - try to get its indentation, only if parent is within the original range.
        let mut child_indentation_amount = -1;

        if !is_error_member_list_element && is_list_item && parent.loc().contained_by(self.original_range) {
            child_indentation_amount =
                self.try_compute_indentation_for_list_item(child_start_pos, child.end(), parent_start_line, self.original_range, inherited_indentation);
            if child_indentation_amount != -1 {
                inherited_indentation = child_indentation_amount;
            }
        }

        // child node is outside the target range - do not dive inside
        if !self.original_range.overlaps(child.loc()) {
            if child.end() < self.original_range.pos() {
                self.fs().skip_to_end_of(&child.loc());
            }
            return inherited_indentation;
        }

        if child.loc().len() == 0 {
            return inherited_indentation;
        }

        while self.fs_ref().is_on_token() && self.fs_ref().get_token_full_start() < self.original_range.end() {
            // proceed any parent tokens that are located prior to child.getStart()
            let token_info = self.fs().read_token_info(node);
            if token_info.token.loc.end() > self.original_range.end() {
                return inherited_indentation;
            }
            if token_info.token.loc.end() > child_start_pos {
                if token_info.token.loc.pos() > child_start_pos {
                    self.fs().skip_to_start_of(&child.loc());
                }
                // stop when formatting scanner advances past the beginning of the child
                break;
            }

            self.consume_token_and_advance_scanner(token_info, node, Rc::clone(&parent_dynamic_indentation), node, false);
        }

        if !self.fs_ref().is_on_token() || self.fs_ref().get_token_full_start() >= self.original_range.end() {
            return inherited_indentation;
        }

        if ast::is_token_kind(child.kind()) {
            // if child node is a token, it does not impact indentation, proceed it using parent indentation scope rules
            let token_info = self.fs().read_token_info(child);
            // JSX text shouldn't affect indenting
            if child.kind() != Kind::JsxText {
                assert!(token_info.token.loc.end() == child.loc().end(), "Token end is child end");
                self.consume_token_and_advance_scanner(token_info, node, parent_dynamic_indentation, child, false);
                return inherited_indentation;
            }
        }

        let mut effective_parent_start_line = undecorated_parent_start_line;
        if child.kind() == Kind::Decorator {
            effective_parent_start_line = child_start_line;
        }
        let child_indentation;
        let mut delta = 0;
        if is_error_member_list_element {
            child_indentation = self.get_current_indentation_at_position(child_start_pos);
        } else {
            (child_indentation, delta) = self.compute_indentation(
                child,
                child_start_line,
                child_indentation_amount,
                node,
                &parent_dynamic_indentation,
                effective_parent_start_line,
            );
        }

        self.process_node(child, self.child_context_node, child_start_line, undecorated_child_start_line, child_indentation, delta);

        self.child_context_node = Some(node);

        if is_first_list_item && parent.kind() == Kind::ArrayLiteralExpression && inherited_indentation == -1 {
            inherited_indentation = child_indentation;
        }

        inherited_indentation
    }

    // span.go:439
    #[allow(clippy::too_many_arguments)]
    fn process_child_nodes(
        &mut self,
        node: P<Node>,
        indenter: Option<DynamicIndenterRef>,
        node_start_line: i32,
        undecorated_node_start_line: i32,
        nodes: P<NodeList>,
        parent: P<Node>,
        parent_start_line: i32,
        parent_dynamic_indentation: DynamicIndenterRef,
    ) {
        assert!(!ast::position_is_synthesized(nodes.pos()));
        assert!(!ast::position_is_synthesized(nodes.end()));

        let list_start_token = get_open_token_for_list(parent, nodes);

        let mut list_dynamic_indentation = Rc::clone(&parent_dynamic_indentation);
        let mut start_line = parent_start_line;

        // node range is outside the target range - do not dive inside
        if !self.original_range.overlaps(nodes.loc.get()) {
            if nodes.end() < self.original_range.pos() && (nodes.nodes().is_empty() || !nodes.nodes()[0].flags().intersects(NodeFlags::Reparsed)) {
                self.fs().skip_to_end_of(&nodes.loc.get());
            }
            return;
        }

        if list_start_token != Kind::Unknown {
            // introduce a new indentation scope for lists (including list start and end tokens)
            while self.fs_ref().is_on_token() && self.fs_ref().get_token_full_start() < self.original_range.end() {
                let token_info = self.fs().read_token_info(parent);
                if token_info.token.loc.end() > nodes.pos() {
                    // stop when formatting scanner moves past the beginning of node list
                    break;
                } else if token_info.token.kind == list_start_token {
                    // consume list start token
                    start_line = scanner::get_ecma_line_of_position(self.source_file.get(), token_info.token.loc.pos());

                    let token_pos = token_info.token.loc.pos();
                    self.consume_token_and_advance_scanner(token_info, parent, Rc::clone(&parent_dynamic_indentation), parent, false);

                    let indentation_on_list_start_token = if self.indentation_on_last_indented_line != -1 {
                        // scanner just processed list start token so consider last indentation as list indentation
                        // function foo(): { // last indentation was 0, list item will be indented based on this value
                        //   foo: number;
                        // }: {};
                        self.indentation_on_last_indented_line
                    } else {
                        self.get_current_indentation_at_position(token_pos)
                    };

                    list_dynamic_indentation =
                        self.get_dynamic_indentation(parent, parent_start_line, indentation_on_list_start_token, self.fc().options.indent_size);
                } else {
                    // consume any tokens that precede the list as child elements of 'node' using its indentation scope
                    self.consume_token_and_advance_scanner(token_info, parent, Rc::clone(&parent_dynamic_indentation), parent, false);
                }
            }
        }

        let mut inherited_indentation = -1;
        for i in 0..nodes.nodes().len() {
            let child = nodes.nodes()[i];
            inherited_indentation = self.process_child_node(
                node,
                indenter.clone(),
                node_start_line,
                undecorated_node_start_line,
                child,
                inherited_indentation,
                node,
                Rc::clone(&list_dynamic_indentation),
                start_line,
                start_line,
                true,
                i == 0,
            );
        }

        let list_end_token = get_close_token_for_open_token(list_start_token);
        if list_end_token != Kind::Unknown && self.fs_ref().is_on_token() && self.fs_ref().get_token_full_start() < self.original_range.end() {
            let mut token_info = self.fs().read_token_info(parent);
            if token_info.token.kind == Kind::CommaToken {
                // consume the comma
                self.consume_token_and_advance_scanner(token_info, parent, Rc::clone(&list_dynamic_indentation), parent, false);
                if self.fs_ref().is_on_token() {
                    token_info = self.fs().read_token_info(parent);
                } else {
                    return;
                }
            }

            // consume the list end token only if it is still belong to the parent
            // there might be the case when current token matches end token but does not considered as one
            // function (x: function) <--
            // without this check close paren will be interpreted as list end token for function expression which is wrong
            if token_info.token.kind == list_end_token && token_info.token.loc.contained_by(parent.loc()) {
                // consume list end token
                self.consume_token_and_advance_scanner(token_info, parent, list_dynamic_indentation, parent, true /*isListEndToken*/);
            }
        }
    }

    // span.go:528
    fn execute_process_node_visitor(&mut self, node: P<Node>, indenter: DynamicIndenterRef, node_start_line: i32, undecorated_node_start_line: i32) {
        let old_node = self.visiting_node;
        let old_indenter = self.visiting_indenter.take();
        let old_start = self.visiting_node_start_line;
        let old_undecorated_start = self.visiting_undecorated_node_start_line;
        self.visiting_node = Some(node);
        self.visiting_indenter = Some(indenter);
        self.visiting_node_start_line = node_start_line;
        self.visiting_undecorated_node_start_line = undecorated_node_start_line;
        for child in collect_visited_children(node) {
            let visiting_node = self.visiting_node.unwrap();
            let visiting_indenter = self.visiting_indenter.clone().unwrap();
            match child {
                VisitedChild::Node(child) => {
                    self.process_child_node(
                        visiting_node,
                        Some(Rc::clone(&visiting_indenter)),
                        self.visiting_node_start_line,
                        self.visiting_undecorated_node_start_line,
                        child,
                        -1,
                        visiting_node,
                        visiting_indenter,
                        self.visiting_node_start_line,
                        self.visiting_undecorated_node_start_line,
                        false,
                        false,
                    );
                }
                VisitedChild::Nodes(nodes) => {
                    self.process_child_nodes(
                        visiting_node,
                        Some(Rc::clone(&visiting_indenter)),
                        self.visiting_node_start_line,
                        self.visiting_undecorated_node_start_line,
                        nodes,
                        visiting_node,
                        self.visiting_node_start_line,
                        visiting_indenter,
                    );
                }
            }
        }
        self.visiting_node = old_node;
        self.visiting_indenter = old_indenter;
        self.visiting_node_start_line = old_start;
        self.visiting_undecorated_node_start_line = old_undecorated_start;
    }

    // span.go:544
    fn get_current_indentation_at_position(&self, pos: i32) -> i32 {
        let start_line_position = get_line_start_position_for_position(pos, self.source_file);
        find_first_non_whitespace_column(start_line_position, pos, self.source_file, &self.fc().options)
    }

    // span.go:549
    fn compute_indentation(
        &self,
        node: P<Node>,
        start_line: i32,
        inherited_indentation: i32,
        parent: P<Node>,
        parent_dynamic_indentation: &DynamicIndenterRef,
        effective_parent_start_line: i32,
    ) -> (i32, i32) {
        let mut delta = 0;
        if should_indent_child_node(&self.fc().options, node, None, None, false) {
            delta = self.fc().options.indent_size;
        }

        if effective_parent_start_line == start_line {
            // if node is located on the same line with the parent
            // - inherit indentation from the parent
            // - push children if either parent of node itself has non-zero delta
            let mut indentation = self.indentation_on_last_indented_line;
            if start_line != self.last_indented_line {
                indentation = parent_dynamic_indentation.get_indentation();
            }
            delta = self.fc().options.indent_size.min(parent_dynamic_indentation.get_delta(Some(node)) + delta);
            return (indentation, delta);
        } else if inherited_indentation == -1 {
            if node.kind() == Kind::OpenParenToken && start_line == self.last_indented_line {
                // the is used for chaining methods formatting
                // - we need to get the indentation on last line and the delta of parent
                return (self.indentation_on_last_indented_line, parent_dynamic_indentation.get_delta(Some(node)));
            } else if child_starts_on_the_same_line_with_else_in_if_statement(parent, node, start_line, self.source_file)
                || child_is_unindented_branch_of_conditional_expression(parent, node, start_line, self.source_file)
                || argument_starts_on_same_line_as_previous_argument(parent, node, start_line, self.source_file)
            {
                return (parent_dynamic_indentation.get_indentation(), delta);
            } else {
                let i = parent_dynamic_indentation.get_indentation();
                if i == -1 {
                    return (parent_dynamic_indentation.get_indentation(), delta);
                }
                return (i + parent_dynamic_indentation.get_delta(Some(node)), delta);
            }
        }

        (inherited_indentation, delta)
    }

    // span.go:593
    /* Tries to compute the indentation for a list element.
    * If list element is not in range then
    * function will pick its actual indentation
    * so it can be pushed downstream as inherited indentation.
    * If list element is in the range - its indentation will be equal
    * to inherited indentation from its predecessors.
     */
    fn try_compute_indentation_for_list_item(
        &self,
        start_pos: i32,
        end_pos: i32,
        parent_start_line: i32,
        r: TextRange,
        inherited_indentation: i32,
    ) -> i32 {
        let r2 = TextRange::new(start_pos, end_pos);
        if r.overlaps(r2) || r2.contained_by(r) {
            /* Not to miss zero-range nodes e.g. JsxText */
            if inherited_indentation != -1 {
                return inherited_indentation;
            }
        } else {
            let start_line = scanner::get_ecma_line_of_position(self.source_file.get(), start_pos);
            let column = self.get_current_indentation_at_position(start_pos);
            if start_line != parent_start_line || start_pos == column {
                // Use the base indent size if it is greater than
                // the indentation of the inherited predecessor.
                let base_indent_size = self.fc().options.base_indent_size;
                if base_indent_size > column {
                    return base_indent_size;
                }
                return column;
            }
        }
        -1
    }

    // span.go:615
    fn process_node(
        &mut self,
        node: P<Node>,
        context_node: Option<P<Node>>,
        node_start_line: i32,
        undecorated_node_start_line: i32,
        indentation: i32,
        delta: i32,
    ) {
        if !self.original_range.overlaps(with_token_start(node, self.source_file)) {
            return;
        }

        let node_dynamic_indentation = self.get_dynamic_indentation(node, node_start_line, indentation, delta);

        // a useful observations when tracking context node
        //        /
        //      [a]
        //   /   |   \
        //  [b] [c] [d]
        // node 'a' is a context node for nodes 'b', 'c', 'd'
        // except for the leftmost leaf token in [b] - in this case context node ('e') is located somewhere above 'a'
        // this rule can be applied recursively to child nodes of 'a'.
        //
        // context node is set to parent node value after processing every child node
        // context node is set to parent of the token after processing every token

        self.child_context_node = context_node;

        // if there are any tokens that logically belong to node and interleave child nodes
        // such tokens will be consumed in processChildNode for the child that follows them
        self.execute_process_node_visitor(node, Rc::clone(&node_dynamic_indentation), node_start_line, undecorated_node_start_line);

        // proceed any tokens in the node that are located after child nodes
        while self.fs_ref().is_on_token() && self.fs_ref().get_token_full_start() < self.original_range.end() {
            let token_info = self.fs().read_token_info(node);
            if token_info.token.loc.end() > node.end().min(self.original_range.end()) {
                break;
            }
            self.consume_token_and_advance_scanner(token_info, node, Rc::clone(&node_dynamic_indentation), node, false);
        }
    }

    // span.go:650
    #[allow(clippy::too_many_arguments)]
    fn process_pair(
        &mut self,
        current_item: TextRangeWithKind,
        current_start_line: i32,
        current_parent: Option<P<Node>>,
        previous_item: TextRangeWithKind,
        previous_start_line: i32,
        previous_parent: Option<P<Node>>,
        context_node: Option<P<Node>>,
        dynamic_indentation: Option<DynamicIndenterRef>,
    ) -> LineAction {
        self.formatting_context.as_mut().unwrap().update_context(previous_item, previous_parent, current_item, current_parent, context_node);

        let mut current_rules = std::mem::take(&mut self.current_rules);
        current_rules.clear();
        self.current_rules = get_rules(self.fc(), current_rules);

        let mut trim_trailing_whitespaces = !self.fc().options.trim_trailing_whitespace.is_false();
        let mut line_action = LineAction::None;

        if !self.current_rules.is_empty() {
            // Apply rules in reverse order so that higher priority rules (which are first in the array)
            // win in a conflict with lower priority rules.
            let rules = self.current_rules.clone();
            for &rule in rules.iter().rev() {
                line_action = self.apply_rule_edits(rule, previous_item, previous_start_line, current_item, current_start_line);
                if let Some(dynamic_indentation) = &dynamic_indentation {
                    match line_action {
                        LineAction::LineRemoved => {
                            // Handle the case where the next line is moved to be the end of this line.
                            // In this case we don't indent the next line in the next pass.
                            if scanner::get_token_pos_of_node(current_parent.unwrap(), self.source_file, false) == current_item.loc.pos() {
                                dynamic_indentation.recompute_indentation(false /*lineAddedByFormatting*/, context_node.unwrap());
                            }
                        }
                        LineAction::LineAdded => {
                            // Handle the case where token2 is moved to the new line.
                            // In this case we indent token2 in the next pass but we set
                            // sameLineIndent flag to notify the indenter that the indentation is within the line.
                            if scanner::get_token_pos_of_node(current_parent.unwrap(), self.source_file, false) == current_item.loc.pos() {
                                dynamic_indentation.recompute_indentation(true /*lineAddedByFormatting*/, context_node.unwrap());
                            }
                        }
                        LineAction::None => {}
                    }
                }

                // We need to trim trailing whitespace between the tokens if they were on different lines, and no rule was applied to put them on the same line
                trim_trailing_whitespaces = trim_trailing_whitespaces
                    && (rule.action() & RuleAction::DeleteSpace == RuleAction::None)
                    && rule.flags() != RuleFlags::CanDeleteNewLines;
            }
        } else {
            trim_trailing_whitespaces = trim_trailing_whitespaces && current_item.kind != Kind::EndOfFile;
        }

        if current_start_line != previous_start_line && trim_trailing_whitespaces {
            // We need to trim trailing whitespace between the tokens if they were on different lines, and no rule was applied to put them on the same line
            self.trim_trailing_whitespaces_for_lines(previous_start_line, current_start_line, previous_item);
        }

        line_action
    }

    // span.go:700
    fn apply_rule_edits(
        &mut self,
        rule: &'static RuleImpl,
        previous_range: TextRangeWithKind,
        previous_start_line: i32,
        current_range: TextRangeWithKind,
        current_start_line: i32,
    ) -> LineAction {
        let on_later_line = current_start_line != previous_start_line;
        match rule.action() {
            RuleAction::StopProcessingSpaceActions => {
                // no action required
                return LineAction::None;
            }
            RuleAction::DeleteSpace => {
                if previous_range.loc.end() != current_range.loc.pos() {
                    // delete characters starting from t1.end up to t2.pos exclusive
                    self.record_delete(previous_range.loc.end(), current_range.loc.pos() - previous_range.loc.end());
                    if on_later_line {
                        return LineAction::LineRemoved;
                    }
                    return LineAction::None;
                }
            }
            RuleAction::DeleteToken => {
                self.record_delete(previous_range.loc.pos(), previous_range.loc.len());
            }
            RuleAction::InsertNewLine => {
                // exit early if we on different lines and rule cannot change number of newlines
                // if line1 and line2 are on subsequent lines then no edits are required - ok to exit
                // if line1 and line2 are separated with more than one newline - ok to exit since we cannot delete extra new lines
                if rule.flags() != RuleFlags::CanDeleteNewLines && previous_start_line != current_start_line {
                    return LineAction::None;
                }

                // edit should not be applied if we have one line feed between elements
                let line_delta = current_start_line - previous_start_line;
                if line_delta != 1 {
                    let new_line = get_new_line_or_default_from_context(&self.ctx);
                    self.record_replace(previous_range.loc.end(), current_range.loc.pos() - previous_range.loc.end(), &new_line);
                    if on_later_line {
                        return LineAction::None;
                    }
                    return LineAction::LineAdded;
                }
            }
            RuleAction::InsertSpace => {
                // exit early if we on different lines and rule cannot change number of newlines
                if rule.flags() != RuleFlags::CanDeleteNewLines && previous_start_line != current_start_line {
                    return LineAction::None;
                }

                let pos_delta = current_range.loc.pos() - previous_range.loc.end();
                if pos_delta != 1 || !self.source_file.text()[previous_range.loc.end() as usize..].starts_with(' ') {
                    self.record_replace(previous_range.loc.end(), pos_delta, " ");
                    if on_later_line {
                        return LineAction::LineRemoved;
                    }
                    return LineAction::None;
                }
            }
            RuleAction::InsertTrailingSemicolon => {
                self.record_insert(previous_range.loc.end(), ";");
            }
            _ => {}
        }
        LineAction::None
    }
}

// span.go:754
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineAction {
    None,
    LineAdded,
    LineRemoved,
}

impl FormatSpanWorker {
    // span.go:762
    fn process_range(
        &mut self,
        r: TextRangeWithKind,
        range_start_line: i32,
        _range_start_character: i32,
        parent: Option<P<Node>>,
        context_node: Option<P<Node>>,
        dynamic_indentation: Option<DynamicIndenterRef>,
    ) -> LineAction {
        let range_has_error = (self.range_contains_error)(r.loc);
        let mut line_action = LineAction::None;
        if !range_has_error {
            if self.previous_range == new_text_range_with_kind(0, 0, Kind::Unknown) {
                // trim whitespaces starting from the beginning of the span up to the current line
                let original_start_line = scanner::get_ecma_line_of_position(self.source_file.get(), self.original_range.pos());
                self.trim_trailing_whitespaces_for_lines(original_start_line, range_start_line, new_text_range_with_kind(0, 0, Kind::Unknown));
            } else {
                line_action = self.process_pair(
                    r,
                    range_start_line,
                    parent,
                    self.previous_range,
                    self.previous_range_start_line,
                    self.previous_parent,
                    context_node,
                    dynamic_indentation,
                );
            }
        }

        self.previous_range = r;
        self.previous_range_trivia_end = r.loc.end();
        self.previous_parent = parent;
        self.previous_range_start_line = range_start_line;

        line_action
    }

    // span.go:783
    fn process_trivia(
        &mut self,
        trivia: &[TextRangeWithKind],
        parent: Option<P<Node>>,
        context_node: Option<P<Node>>,
        dynamic_indentation: Option<DynamicIndenterRef>,
    ) {
        for &trivia_item in trivia {
            if is_comment(trivia_item.kind) && trivia_item.loc.contained_by(self.original_range) {
                let (trivia_item_start_line, trivia_item_start_character) =
                    scanner::get_ecma_line_and_byte_offset_of_position(self.source_file.get(), trivia_item.loc.pos());
                self.process_range(trivia_item, trivia_item_start_line, trivia_item_start_character, parent, context_node, dynamic_indentation.clone());
            }
        }
    }

    // span.go:796
    /*
    * Trimming will be done for lines after the previous range.
    * Exclude comments as they had been previously processed.
     */
    fn trim_trailing_whitespaces_for_remaining_range(&mut self, trivias: &[TextRangeWithKind]) {
        let mut start_pos = self.original_range.pos();
        if self.previous_range != new_text_range_with_kind(0, 0, Kind::Unknown) {
            start_pos = self.previous_range.loc.end();
        }

        for trivia in trivias {
            if is_comment(trivia.kind) {
                if start_pos < trivia.loc.pos() {
                    self.trim_trailing_witespaces_for_positions(start_pos, trivia.loc.pos() - 1, self.previous_range);
                }

                start_pos = trivia.loc.end() + 1;
            }
        }

        if start_pos < self.original_range.end() {
            self.trim_trailing_witespaces_for_positions(start_pos, self.original_range.end(), self.previous_range);
        }
    }

    // span.go:817
    fn trim_trailing_witespaces_for_positions(&mut self, start_pos: i32, end_pos: i32, previous_range: TextRangeWithKind) {
        let start_line = scanner::get_ecma_line_of_position(self.source_file.get(), start_pos);
        let end_line = scanner::get_ecma_line_of_position(self.source_file.get(), end_pos);

        self.trim_trailing_whitespaces_for_lines(start_line, end_line + 1, previous_range);
    }

    // span.go:824
    fn trim_trailing_whitespaces_for_lines(&mut self, line1: i32, line2: i32, r: TextRangeWithKind) {
        let line_starts = scanner::get_ecma_line_starts(self.source_file.get());
        for line in line1..line2 {
            let line_start_position = line_starts[line as usize] as i32;
            let line_end_position = scanner::get_ecma_end_line_position(self.source_file, line);

            // do not trim whitespaces in comments or template expression
            if r != new_text_range_with_kind(0, 0, Kind::Unknown)
                && (is_comment(r.kind) || is_string_or_regular_expression_or_template_literal(r.kind))
                && r.loc.pos() <= line_end_position
                && r.loc.end() > line_end_position
            {
                continue;
            }

            let whitespace_start = self.get_trailing_whitespace_start_position(line_start_position, line_end_position);
            if whitespace_start != -1 {
                if whitespace_start != line_start_position {
                    let (r, _) = decode_rune(&self.source_file.text().as_bytes()[(whitespace_start - 1) as usize..]);
                    assert!(!is_white_space_single_line(r));
                }
                self.record_delete(whitespace_start, line_end_position + 1 - whitespace_start);
            }
        }
    }

    // span.go:850
    /*
    * @param start The position of the first character in range
    * @param end The position of the last character in range
     */
    fn get_trailing_whitespace_start_position(&self, start: i32, end: i32) -> i32 {
        let mut pos = end;
        let text = self.source_file.text().as_bytes();
        while pos >= start {
            let (ch, size) = decode_rune(&text[pos as usize..]);
            if size == 0 {
                pos -= 1; // multibyte character, rewind more
                continue;
            }
            if !is_white_space_single_line(ch) {
                break;
            }
            pos -= 1;
        }
        if pos != end {
            return pos + 1;
        }
        -1
    }
}

// span.go:870
pub(crate) fn is_string_or_regular_expression_or_template_literal(kind: Kind) -> bool {
    kind == Kind::StringLiteral || kind == Kind::RegularExpressionLiteral || ast::is_template_literal_kind(kind)
}

// span.go:874
fn is_comment(kind: Kind) -> bool {
    kind == Kind::SingleLineCommentTrivia || kind == Kind::MultiLineCommentTrivia
}

impl FormatSpanWorker {
    // span.go:878
    fn insert_indentation(&mut self, pos: i32, indentation: i32, line_added: bool) {
        let indentation_string = get_indentation_string(indentation, &self.fc().options);
        if line_added {
            // new line is added before the token by the formatting rules
            // insert indentation string at the very beginning of the token
            self.record_replace(pos, 0, &indentation_string);
        } else {
            let (token_start_line, token_start_character) = scanner::get_ecma_line_and_byte_offset_of_position(self.source_file.get(), pos);
            let start_line_position = scanner::get_ecma_line_starts(self.source_file.get())[token_start_line as usize] as i32;
            if indentation != self.character_to_column(start_line_position, token_start_character)
                || self.indentation_is_different(&indentation_string, start_line_position)
            {
                self.record_replace(start_line_position, token_start_character, &indentation_string);
            }
        }
    }

    // span.go:893
    fn character_to_column(&self, start_line_position: i32, character_in_line: i32) -> i32 {
        let mut column = 0;
        let text = self.source_file.text().as_bytes();
        let tab_size = self.fc().options.tab_size;
        for i in 0..character_in_line {
            if text[(start_line_position + i) as usize] == b'\t' {
                if tab_size > 0 {
                    column += tab_size - (column % tab_size);
                }
            } else {
                column += 1;
            }
        }
        column
    }

    // span.go:907
    fn indentation_is_different(&self, indentation_string: &str, start_line_position: i32) -> bool {
        let text = self.source_file.text().as_bytes();
        let end = start_line_position as usize + indentation_string.len();
        if end > text.len() {
            return true;
        }
        indentation_string.as_bytes() != &text[start_line_position as usize..end]
    }

    // span.go:916
    fn indent_trivia_items(
        &mut self,
        trivia: &[TextRangeWithKind],
        comment_indentation: i32,
        mut indent_next_token_or_trivia: bool,
        indent_single_line: &mut dyn FnMut(&mut FormatSpanWorker, TextRangeWithKind),
    ) -> bool {
        for &trivia_item in trivia {
            let trivia_in_range = trivia_item.loc.contained_by(self.original_range);
            match trivia_item.kind {
                Kind::MultiLineCommentTrivia => {
                    if trivia_in_range {
                        self.indent_multiline_comment(trivia_item.loc, comment_indentation, !indent_next_token_or_trivia, true);
                    }
                    indent_next_token_or_trivia = false;
                }
                Kind::SingleLineCommentTrivia => {
                    if indent_next_token_or_trivia && trivia_in_range {
                        indent_single_line(self, trivia_item);
                    }
                    indent_next_token_or_trivia = false;
                }
                Kind::NewLineTrivia => {
                    indent_next_token_or_trivia = true;
                }
                _ => {}
            }
        }
        indent_next_token_or_trivia
    }

    // span.go:937
    fn indent_multiline_comment(&mut self, comment_range: TextRange, indentation: i32, first_line_is_indented: bool, indent_final_line: bool) {
        // split comment in lines
        let mut start_line = scanner::get_ecma_line_of_position(self.source_file.get(), comment_range.pos());
        let end_line = scanner::get_ecma_line_of_position(self.source_file.get(), comment_range.end());

        if start_line == end_line {
            if !first_line_is_indented {
                // treat as single line comment
                self.insert_indentation(comment_range.pos(), indentation, false);
            }
            return;
        }

        let text = self.source_file.text();
        let mut parts: Vec<TextRange> =
            Vec::with_capacity(text[comment_range.pos() as usize..comment_range.end() as usize].matches('\n').count());
        let mut start_pos = comment_range.pos();
        for line in start_line..end_line {
            let end_of_line = scanner::get_ecma_end_line_position(self.source_file, line);
            parts.push(TextRange::new(start_pos, end_of_line));
            start_pos = scanner::get_ecma_line_starts(self.source_file.get())[(line + 1) as usize] as i32;
        }

        if indent_final_line {
            parts.push(TextRange::new(start_pos, comment_range.end()));
        }

        if parts.is_empty() {
            return;
        }

        let start_line_pos = scanner::get_ecma_line_starts(self.source_file.get())[start_line as usize] as i32;

        let (non_whitespace_in_first_part_character, non_whitespace_in_first_part_column) =
            find_first_non_whitespace_character_and_column(start_line_pos, parts[0].pos(), self.source_file, &self.fc().options);

        let mut start_index = 0;

        if first_line_is_indented {
            start_index = 1;
            start_line += 1;
        }

        // shift all parts on the delta size
        let delta = indentation - non_whitespace_in_first_part_column;
        for i in start_index..parts.len() {
            let start_line_pos = scanner::get_ecma_line_starts(self.source_file.get())[start_line as usize] as i32;
            let mut non_whitespace_character = non_whitespace_in_first_part_character;
            let mut non_whitespace_column = non_whitespace_in_first_part_column;
            if i != 0 {
                (non_whitespace_character, non_whitespace_column) =
                    find_first_non_whitespace_character_and_column(parts[i].pos(), parts[i].end(), self.source_file, &self.fc().options);
            }
            let new_indentation = non_whitespace_column + delta;
            if new_indentation > 0 {
                let indentation_string = get_indentation_string(new_indentation, &self.fc().options);
                self.record_replace(start_line_pos, non_whitespace_character, &indentation_string);
            } else {
                self.record_delete(start_line_pos, non_whitespace_character);
            }

            start_line += 1;
        }
    }
}

// span.go:998
pub(crate) fn get_indentation_string(indentation: i32, options: &FormatCodeSettings) -> String {
    // go's `strings.Repeat` already has static, global caching for repeated tabs and spaces, so there's no need to cache here like in strada
    if !options.convert_tabs_to_spaces.is_true() {
        if options.tab_size == 0 {
            return String::new();
        }
        let tabs = (indentation as f64 / options.tab_size as f64).floor() as i32;
        let spaces = indentation - (tabs * options.tab_size);
        let mut res = "\t".repeat(tabs.max(0) as usize);
        if spaces > 0 {
            res += &" ".repeat(spaces as usize);
        }

        res
    } else {
        " ".repeat(indentation.max(0) as usize)
    }
}

// span.go:1017
fn create_text_change_from_start_length(start: i32, length: i32, new_text: &str) -> TextChange {
    TextChange { new_text: new_text.to_string(), text_range: TextRange::new(start, start + length) }
}

impl FormatSpanWorker {
    // span.go:1024
    fn record_delete(&mut self, start: i32, length: i32) {
        if length != 0 {
            self.edits.push(create_text_change_from_start_length(start, length, ""));
        }
    }

    // span.go:1030
    fn record_replace(&mut self, start: i32, length: i32, new_text: &str) {
        if length != 0 || !new_text.is_empty() {
            self.edits.push(create_text_change_from_start_length(start, length, new_text));
        }
    }

    // span.go:1036
    fn record_insert(&mut self, start: i32, text: &str) {
        if !text.is_empty() {
            self.edits.push(create_text_change_from_start_length(start, 0, text));
        }
    }

    // span.go:1042
    fn consume_token_and_advance_scanner(
        &mut self,
        current_token_info: TokenInfo,
        parent: P<Node>,
        dynamic_indenation: DynamicIndenterRef,
        container: P<Node>,
        is_list_end_token: bool,
    ) {
        // assert(currentTokenInfo.token.Loc.ContainedBy(parent.Loc)) // !!!
        let last_trivia_was_new_line = self.fs_ref().last_trailing_trivia_was_new_line();
        let mut indent_token = false;

        if !current_token_info.leading_trivia.is_empty() {
            self.process_trivia(&current_token_info.leading_trivia, Some(parent), self.child_context_node, Some(Rc::clone(&dynamic_indenation)));
        }

        let mut line_action = LineAction::None;
        let is_token_in_range = current_token_info.token.loc.contained_by(self.original_range);

        let (token_start_line, token_start_char) =
            scanner::get_ecma_line_and_byte_offset_of_position(self.source_file.get(), current_token_info.token.loc.pos());

        if is_token_in_range {
            let range_has_error = (self.range_contains_error)(current_token_info.token.loc);
            // save previousRange since processRange will overwrite this value with current one
            let save_previous_range = self.previous_range;
            line_action = self.process_range(
                current_token_info.token,
                token_start_line,
                token_start_char,
                Some(parent),
                self.child_context_node,
                Some(Rc::clone(&dynamic_indenation)),
            );
            // do not indent comments\token if token range overlaps with some error
            if !range_has_error {
                if line_action == LineAction::None {
                    // indent token only if end line of previous range does not match start line of the token
                    if save_previous_range != new_text_range_with_kind(0, 0, Kind::Unknown) {
                        let prev_end_line = scanner::get_ecma_line_of_position(self.source_file.get(), save_previous_range.loc.end());
                        indent_token = last_trivia_was_new_line && token_start_line != prev_end_line;
                    } else {
                        // When there's no previous range (first token), TS sets prevEndLine to undefined.
                        // tokenStart.line !== undefined is always true in JS, so indentToken = lastTriviaWasNewLine.
                        indent_token = last_trivia_was_new_line;
                    }
                } else {
                    indent_token = line_action == LineAction::LineAdded;
                }
            }
        }

        if !current_token_info.trailing_trivia.is_empty() {
            self.previous_range_trivia_end = current_token_info.trailing_trivia.last().unwrap().loc.end();
            // If any trailing comment trivia extends past the original range, it won't be
            // processed by processTrivia (which skips comments not contained by originalRange).
            // Cap previousRangeTriviaEnd before such comments so the trailing edit contiguity
            // check in execute() won't pair across unprocessed comment content.
            for trivia in &current_token_info.trailing_trivia {
                if is_comment(trivia.kind) && !trivia.loc.contained_by(self.original_range) {
                    self.previous_range_trivia_end = trivia.loc.pos();
                    break;
                }
            }
            self.process_trivia(&current_token_info.trailing_trivia, Some(parent), self.child_context_node, Some(Rc::clone(&dynamic_indenation)));
        }

        if indent_token {
            let mut token_indentation = -1;
            if is_token_in_range && !(self.range_contains_error)(current_token_info.token.loc) {
                token_indentation =
                    dynamic_indenation.get_indentation_for_token(token_start_line, current_token_info.token.kind, container, is_list_end_token);
            }
            let mut indent_next_token_or_trivia = true;
            if !current_token_info.leading_trivia.is_empty() {
                let comment_indentation = dynamic_indenation.get_indentation_for_comment(current_token_info.token.kind, token_indentation, container);
                indent_next_token_or_trivia = self.indent_trivia_items(
                    &current_token_info.leading_trivia,
                    comment_indentation,
                    indent_next_token_or_trivia,
                    &mut |w: &mut FormatSpanWorker, item: TextRangeWithKind| {
                        w.insert_indentation(item.loc.pos(), comment_indentation, false);
                    },
                );
            }

            // indent token only if is it is in target range and does not overlap with any error ranges
            if token_indentation != -1 && indent_next_token_or_trivia {
                self.insert_indentation(current_token_info.token.loc.pos(), token_indentation, line_action == LineAction::LineAdded);

                self.last_indented_line = token_start_line;
                self.indentation_on_last_indented_line = token_indentation;
            }
        }

        self.fs().advance();

        self.child_context_node = Some(parent);
    }
}

// span.go:1121
pub(crate) struct DynamicIndenter {
    node: P<Node>,
    node_start_line: i32,
    indentation: Cell<i32>,
    delta: Cell<i32>,

    options: Rc<FormatCodeSettings>,
    source_file: P<SourceFile>,
}

// Go passes `*dynamicIndenter` around and mutates it in place (recomputeIndentation).
pub(crate) type DynamicIndenterRef = Rc<DynamicIndenter>;

impl DynamicIndenter {
    // span.go:1131
    fn get_indentation_for_comment(&self, kind: Kind, token_indentation: i32, container: P<Node>) -> i32 {
        match kind {
            // preceding comment to the token that closes the indentation scope inherits the indentation from the scope
            // ..  {
            //     // comment
            // }
            Kind::CloseBraceToken | Kind::CloseBracketToken | Kind::CloseParenToken => {
                return self.indentation.get() + self.get_delta(Some(container));
            }
            _ => {}
        }
        if token_indentation != -1 {
            return token_indentation;
        }
        self.indentation.get()
    }

    // span.go:1159
    // if list end token is LessThanToken '>' then its delta should be explicitly suppressed
    // so that LessThanToken as a binary operator can still be indented.
    // foo.then
    //
    //	<
    //	    number,
    //	    string,
    //	>();
    //
    // vs
    // var a = xValue
    //
    //	> yValue;
    fn get_indentation_for_token(&self, line: i32, kind: Kind, container: P<Node>, suppress_delta: bool) -> i32 {
        if !suppress_delta && self.should_add_delta(line, kind, container) {
            return self.indentation.get() + self.get_delta(Some(container));
        }
        self.indentation.get()
    }

    // span.go:1166
    fn get_indentation(&self) -> i32 {
        self.indentation.get()
    }

    // span.go:1170
    fn get_delta(&self, child: Option<P<Node>>) -> i32 {
        // Delta value should be zero when the node explicitly prevents indentation of the child node
        if node_will_indent_child(&self.options, self.node, child, Some(self.source_file), true) {
            return self.delta.get();
        }
        0
    }

    // span.go:1178
    fn recompute_indentation(&self, line_added: bool, parent: P<Node>) {
        if should_indent_child_node(&self.options, parent, Some(self.node), Some(self.source_file), false) {
            if line_added {
                self.indentation.set(self.indentation.get() + self.options.indent_size); // !!! no nil check???
            } else {
                self.indentation.set(self.indentation.get() - self.options.indent_size); // !!! no nil check???
            }
            if should_indent_child_node(&self.options, self.node, None, None, false) {
                self.delta.set(self.options.indent_size);
            } else {
                self.delta.set(0);
            }
        }
    }

    // span.go:1193
    fn should_add_delta(&self, line: i32, kind: Kind, container: P<Node>) -> bool {
        match kind {
            // open and close brace, 'else' and 'while' (in do statement) tokens has indentation of the parent
            Kind::OpenBraceToken
            | Kind::CloseBraceToken
            | Kind::CloseParenToken
            | Kind::ElseKeyword
            | Kind::WhileKeyword
            | Kind::AtToken => return false,
            Kind::SlashToken | Kind::GreaterThanToken => {
                if matches!(container.kind(), Kind::JsxOpeningElement | Kind::JsxClosingElement | Kind::JsxSelfClosingElement) {
                    return false;
                }
            }
            Kind::OpenBracketToken | Kind::CloseBracketToken => {
                if container.kind() != Kind::MappedType {
                    return false;
                }
            }
            _ => {}
        }
        // if token line equals to the line of containing node (this is a first token in the node) - use node indentation
        self.node_start_line != line &&
            // if this token is the first token following the list of decorators, we do not need to indent
            !(ast::has_decorators(self.node) && kind == get_first_non_decorator_token_of_node(self.node))
    }
}

// span.go:1214
fn get_first_non_decorator_token_of_node(node: P<Node>) -> Kind {
    if ast::can_have_modifiers(node) {
        let modifier_nodes = node.modifier_nodes();
        let first_decorator = tsrs_core::find_index(modifier_nodes, |&n| ast::is_decorator(n));
        let modifier = modifier_nodes[first_decorator as usize..].iter().copied().find(|&n| ast::is_modifier(n));
        if let Some(modifier) = modifier {
            return modifier.kind();
        }
    }

    match node.kind() {
        Kind::ClassDeclaration => return Kind::ClassKeyword,
        Kind::InterfaceDeclaration => return Kind::InterfaceKeyword,
        Kind::FunctionDeclaration => return Kind::FunctionKeyword,
        Kind::EnumDeclaration => return Kind::EnumDeclaration,
        Kind::GetAccessor => return Kind::GetKeyword,
        Kind::SetAccessor => return Kind::SetKeyword,
        Kind::MethodDeclaration | Kind::PropertyDeclaration | Kind::Parameter => {
            if node.kind() == Kind::MethodDeclaration && node.as_method_declaration().asterisk_token().is_some() {
                return Kind::AsteriskToken;
            }
            let name = ast::get_name_of_declaration(node);
            if let Some(name) = name {
                return name.kind();
            }
        }
        _ => {}
    }

    Kind::Unknown
}

impl FormatSpanWorker {
    // span.go:1251
    fn get_dynamic_indentation(&self, node: P<Node>, node_start_line: i32, indentation: i32, delta: i32) -> DynamicIndenterRef {
        Rc::new(DynamicIndenter {
            node,
            node_start_line,
            indentation: Cell::new(indentation),
            delta: Cell::new(delta),
            options: Rc::clone(&self.fc().options),
            source_file: self.source_file,
        })
    }
}
