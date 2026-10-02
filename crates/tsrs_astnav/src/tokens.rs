use std::cell::{Cell, RefCell};
use std::rc::Rc;

use tsrs_ast::{self as ast, Kind, ModifierList, Node, NodeFlags, NodeList, NodeVisitor, NodeVisitorHooks, SourceFile};
use tsrs_core::{binary_search_unique_func, P};
use tsrs_scanner::{self as scanner, Scanner};

pub type VisitNodeFn = Rc<dyn Fn(Option<P<Node>>, &mut NodeVisitor) -> Option<P<Node>>>;
pub type VisitNodesFn = Rc<dyn Fn(Option<P<NodeList>>, &mut NodeVisitor) -> Option<P<NodeList>>>;

// tokens.go:12
fn should_rescan_less_than_less_than_token(_s: &Scanner, containing_node: P<Node>, token: Kind) -> bool {
    token == Kind::LessThanLessThanToken && ast::is_jsx_child(containing_node)
}

// tokens.go:16
fn scan_navigation_token(s: &mut Scanner, containing_node: P<Node>) -> Kind {
    let token = s.token();
    if should_rescan_less_than_less_than_token(s, containing_node, token) {
        return s.re_scan_jsx_token(true /*allowMultilineJsxText*/);
    }
    token
}

// tokens.go:24
pub fn get_touching_property_name(source_file: P<SourceFile>, position: i32) -> P<Node> {
    get_token_at_position_worker(source_file, position, false /*allowPositionInLeadingTrivia*/, Some(|node: P<Node>| {
        ast::is_property_name_literal(node) || ast::is_keyword_kind(node.kind()) || ast::is_private_identifier(node)
    }))
}

// tokens.go:30
pub fn get_touching_token(source_file: P<SourceFile>, position: i32) -> P<Node> {
    get_token_at_position_worker(source_file, position, false /*allowPositionInLeadingTrivia*/, None)
}

// tokens.go:34
pub fn get_token_at_position(source_file: P<SourceFile>, position: i32) -> P<Node> {
    get_token_at_position_worker(source_file, position, true /*allowPositionInLeadingTrivia*/, None)
}

struct TokenAtPositionState {
    next: Cell<Option<P<Node>>>,
    prev_subtree: Cell<Option<P<Node>>>,
    left: Cell<i32>,
    node_after_left: Cell<Option<P<Node>>>,
}

// tokens.go:38
pub(crate) fn get_token_at_position_worker(
    source_file: P<SourceFile>,
    position: i32,
    allow_position_in_leading_trivia: bool,
    include_preceding_token_at_end_position: Option<fn(P<Node>) -> bool>,
) -> P<Node> {
    // getTokenAtPosition returns a token at the given position in the source file.
    // The token can be a real node in the AST, or a synthesized token constructed
    // with information from the scanner. Synthesized tokens are only created when
    // needed, and they are stored in the source file's token cache such that multiple
    // calls to getTokenAtPosition with the same position will return the same object
    // in memory. If there is no token at the given position (possible when
    // `allowPositionInLeadingTrivia` is false), the lowest node that encloses the
    // position is returned.

    // `next` tracks the node whose children will be visited on the next iteration.
    // `prevSubtree` is a node whose end position is equal to the target position,
    // only if `includePrecedingTokenAtEndPosition` is provided. Once set, the next
    // iteration of the loop will test the rightmost token of `prevSubtree` to see
    // if it should be returned.
    // `left` tracks the lower boundary of the node/token that could be returned,
    // and is eventually the scanner's start position, if the scanner is used.
    // `nodeAfterLeft` tracks the first node we visit after visiting the node that advances `left`.
    // When scanning in between nodes for token, we should only scan up to the start of `nodeAfterLeft`.
    let st = Rc::new(TokenAtPositionState {
        next: Cell::new(None),
        prev_subtree: Cell::new(None),
        left: Cell::new(0),
        node_after_left: Cell::new(None),
    });
    let mut current = source_file.as_node();

    let get_included_preceding_token = move |subtree: P<Node>| -> Option<P<Node>> {
        let child = find_preceding_token_ex(source_file, position, Some(subtree), false /*excludeJSDoc*/);
        if let Some(child) = child {
            if child.end() == position && include_preceding_token_at_end_position.unwrap()(child) {
                return Some(child);
            }
        }
        None
    };

    let test_node = {
        let st = st.clone();
        Rc::new(move |node: P<Node>| -> i32 {
            if node.kind() != Kind::EndOfFile
                && node.end() == position
                && include_preceding_token_at_end_position.is_some()
                && !node.flags().intersects(NodeFlags::Reparsed)
            {
                if let Some(prev_subtree) = st.prev_subtree.get() {
                    if get_included_preceding_token(prev_subtree).is_some() {
                        return 0;
                    }
                }
                st.prev_subtree.set(Some(node));
            }

            // A node "contains" the position if position < end, except nodes at the file end
            // treat end as inclusive (there's nowhere else to look). This applies to the EOF
            // token itself, and to JSDoc nodes reaching EOF (e.g. unterminated JSDoc comments).
            if node.end() < position
                || node.end() == position
                    && node.kind() != Kind::EndOfFile
                    && (!ast::is_jsdoc_kind(node.kind()) || node.end() != source_file.end_of_file_token.end())
            {
                return -1;
            }
            let node_pos = get_position(node, source_file, allow_position_in_leading_trivia);
            if node_pos > position {
                return 1;
            }
            0
        })
    };

    // We zero in on the node that contains the target position by visiting each
    // child and JSDoc comment of the current node. Node children are walked in
    // order, while node lists are binary searched.
    let visit_node: VisitNodeFn = {
        let st = st.clone();
        let test_node = test_node.clone();
        Rc::new(move |node: Option<P<Node>>, _: &mut NodeVisitor| -> Option<P<Node>> {
            // We can't abort visiting children, so once a match is found, we set `next`
            // and do nothing on subsequent visits.
            let node = node?;
            if node.flags().intersects(NodeFlags::Reparsed) {
                return None;
            }
            if st.node_after_left.get().is_none() {
                st.node_after_left.set(Some(node));
            }
            if st.next.get().is_none() {
                let result = test_node(node);
                match result {
                    -1 => {
                        if !ast::is_jsdoc_kind(node.kind()) {
                            // We can't move the left boundary into or beyond JSDoc,
                            // because we may end up returning the token after this JSDoc,
                            // constructing it with the scanner, and we need to include
                            // all its leading trivia in its position.
                            st.left.set(node.end());
                        }
                        st.node_after_left.set(None);
                    }
                    0 => st.next.set(Some(node)),
                    _ => {}
                }
            }
            Some(node)
        })
    };

    let visit_node_list: VisitNodesFn = {
        let st = st.clone();
        let test_node = test_node.clone();
        Rc::new(move |node_list: Option<P<NodeList>>, _: &mut NodeVisitor| -> Option<P<NodeList>> {
            let Some(list) = node_list else {
                return node_list;
            };
            if list.nodes.is_empty() {
                return node_list;
            }
            if st.node_after_left.get().is_none() {
                for &node in list.nodes {
                    if !node.flags().intersects(NodeFlags::Reparsed) {
                        st.node_after_left.set(Some(node));
                        break;
                    }
                }
            }
            if st.next.get().is_none() {
                if list.end() == position && include_preceding_token_at_end_position.is_some() {
                    st.left.set(list.end());
                    st.node_after_left.set(None);
                    for &v in list.nodes.iter().rev() {
                        if !v.flags().intersects(NodeFlags::Reparsed) {
                            st.prev_subtree.set(Some(v));
                            break;
                        }
                    }
                } else if list.end() <= position {
                    st.left.set(list.end());
                    st.node_after_left.set(None);
                } else if list.pos() <= position {
                    let all_nodes: &'static [P<Node>] = list.nodes;
                    let (mut index, mut match_) = binary_search_unique_func(all_nodes, |middle, node| {
                        if node.flags().intersects(NodeFlags::Reparsed) {
                            return 0;
                        }
                        let cmp = test_node(*node);
                        if cmp < 0 {
                            st.left.set(node.end());
                            st.node_after_left.set(None);
                            for &n in &all_nodes[middle + 1..] {
                                if !n.flags().intersects(NodeFlags::Reparsed) {
                                    st.node_after_left.set(Some(n));
                                    break;
                                }
                            }
                        }
                        cmp
                    });
                    let mut nodes: Vec<P<Node>> = all_nodes.to_vec();
                    if match_ && nodes[index].flags().intersects(NodeFlags::Reparsed) {
                        // filter and search again
                        nodes.retain(|node| !node.flags().intersects(NodeFlags::Reparsed));
                        let nodes_ref = &nodes;
                        (index, match_) = binary_search_unique_func(nodes_ref, |middle, node| {
                            let cmp = test_node(*node);
                            if cmp < 0 {
                                st.left.set(node.end());
                                if middle + 1 < nodes_ref.len() {
                                    st.node_after_left.set(Some(nodes_ref[middle + 1]));
                                } else {
                                    st.node_after_left.set(None);
                                }
                            }
                            cmp
                        });
                    }
                    if match_ {
                        st.next.set(Some(nodes[index]));
                    }
                }
            }
            node_list
        })
    };

    loop {
        visit_each_child_and_jsdoc(current, source_file, Some(visit_node.clone()), Some(visit_node_list.clone()));
        // If prevSubtree was set on the last iteration, it ends at the target position.
        // Check if the rightmost token of prevSubtree should be returned based on the
        // `includePrecedingTokenAtEndPosition` callback.
        if let Some(prev_subtree) = st.prev_subtree.get() {
            if let Some(child) = get_included_preceding_token(prev_subtree) {
                // Optimization: includePrecedingTokenAtEndPosition only ever returns true
                // for real AST nodes, so we don't run the scanner here.
                return child;
            }
            st.prev_subtree.set(None);
        }

        // No node was found that contains the target position, so we've gone as deep as
        // we can in the AST. We've either found a token, or we need to run the scanner
        // to construct one that isn't stored in the AST.
        let Some(next) = st.next.get() else {
            if ast::is_token_kind(current.kind()) || should_skip_child(current) {
                return current;
            }
            let mut left = st.left.get();
            let mut scanner = scanner::get_scanner_for_source_file(source_file, left);
            let mut end = current.end();
            // We should only scan up to the start of the next node in the AST after the node ending at position `left`.
            // It is necessary to enforce this invariant in cases where `position` occurs in between two node/tokens,
            // such that we would not find a token in the loop below before we reach the next node.
            // We can fall into this case when `allowPositionInLeadingTrivia` is false and `position` is in a leading trivia,
            // or when `position` would be in the leading trivia of a node but this node is inside JSDoc:
            // ```
            // /**
            //  * @type {{
            //  */*$*/ identifier: boolean;
            //  * }}
            //  */
            // ```
            // The position of marker '$' falls in between the asterisk token and the identifier token, but is not
            // part of the leading trivia for `identifier`.
            if let Some(node_after_left) = st.node_after_left.get() {
                end = node_after_left.pos();
            }
            while left < end {
                let token = scan_navigation_token(&mut scanner, current);
                let token_full_start = scanner.token_full_start();
                let token_start = if allow_position_in_leading_trivia { token_full_start } else { scanner.token_start() };
                let token_end = scanner.token_end();
                let flags = scanner.token_flags();
                if token_end > end {
                    break;
                }
                if token_start <= position && (position < token_end) {
                    if token == Kind::Identifier || !ast::is_token_kind(token) {
                        if ast::is_jsdoc_kind(current.kind()) {
                            return current;
                        }
                        panic!("did not expect {:?} to have {:?} in its trivia", current.kind(), token);
                    }
                    return source_file.get_or_create_token(token, token_full_start, token_end, current, flags);
                }
                if let Some(include) = include_preceding_token_at_end_position {
                    if token_end == position {
                        let prev_token = source_file.get_or_create_token(token, token_full_start, token_end, current, flags);
                        if include(prev_token) {
                            return prev_token;
                        }
                    }
                }
                left = token_end;
                scanner.scan();
            }
            return current;
        };
        current = next;
        st.left.set(current.pos());
        st.node_after_left.set(None);
        st.next.set(None);
    }
}

// tokens.go:252
fn get_position(node: P<Node>, source_file: P<SourceFile>, allow_position_in_leading_trivia: bool) -> i32 {
    if allow_position_in_leading_trivia {
        return node.pos();
    }
    scanner::get_token_pos_of_node(node, source_file, true /*includeJSDoc*/)
}

// tokens.go:259
pub fn find_rightmost_node(node: P<Node>) -> P<Node> {
    let next: Rc<Cell<Option<P<Node>>>> = Rc::new(Cell::new(None));
    let mut current = node;
    let visit_node: VisitNodeFn = {
        let next = next.clone();
        Rc::new(move |node: Option<P<Node>>, _: &mut NodeVisitor| {
            if node.is_some() {
                next.set(node);
            }
            node
        })
    };
    let visit_nodes: VisitNodesFn = {
        let next = next.clone();
        Rc::new(move |node_list: Option<P<NodeList>>, _: &mut NodeVisitor| {
            if let Some(list) = node_list {
                if let Some(rightmost) = ast::find_last_visible_node(list.nodes) {
                    next.set(Some(rightmost));
                }
            }
            node_list
        })
    };
    let mut visitor = get_node_visitor(Some(visit_node), Some(visit_nodes));

    loop {
        current.visit_each_child(&mut visitor);
        let Some(n) = next.get() else {
            return current;
        };
        current = n;
        next.set(None);
    }
}

// tokens.go:290
pub fn visit_each_child_and_jsdoc(
    node: P<Node>,
    source_file: P<SourceFile>,
    visit_node: Option<VisitNodeFn>,
    visit_nodes: Option<VisitNodesFn>,
) {
    let mut visitor = get_node_visitor(visit_node, visit_nodes);
    for &jsdoc in node.jsdoc(Some(source_file.get())) {
        if let Some(hook) = visitor.hooks.visit_node.clone() {
            hook(Some(jsdoc), &mut visitor);
        } else {
            visitor.visit_node(Some(jsdoc));
        }
    }
    node.visit_each_child(&mut visitor);
}

// tokens.go:307
const COMPARISON_LESS_THAN: i32 = -1;
const COMPARISON_EQUAL_TO: i32 = 0;
const COMPARISON_GREATER_THAN: i32 = 1;

// tokens.go:317
// Finds the leftmost token satisfying `position < token.End()`.
// If the leftmost token satisfying `position < token.End()` is invalid, or if position
// is in the trivia of that leftmost token,
// we will find the rightmost valid token with `token.End() <= position`.
pub fn find_preceding_token(source_file: P<SourceFile>, position: i32) -> Option<P<Node>> {
    find_preceding_token_ex(source_file, position, None, false)
}

// tokens.go:321
pub fn find_preceding_token_ex(
    source_file: P<SourceFile>,
    position: i32,
    start_node: Option<P<Node>>,
    exclude_jsdoc: bool,
) -> Option<P<Node>> {
    fn find(source_file: P<SourceFile>, position: i32, exclude_jsdoc: bool, n: P<Node>) -> Option<P<Node>> {
        if ast::is_non_whitespace_token(n) && n.kind() != Kind::EndOfFile {
            return Some(n);
        }

        // `foundChild` is the leftmost node that contains the target position.
        // `prevChild` is the last visited child of the current node.
        let found_child: Rc<Cell<Option<P<Node>>>> = Rc::new(Cell::new(None));
        let prev_child: Rc<Cell<Option<P<Node>>>> = Rc::new(Cell::new(None));
        let visit_node: VisitNodeFn = {
            let found_child = found_child.clone();
            let prev_child = prev_child.clone();
            Rc::new(move |node: Option<P<Node>>, _: &mut NodeVisitor| {
                // skip synthesized nodes (that will exist now because of jsdoc handling)
                let Some(nd) = node else {
                    return node;
                };
                if nd.flags().intersects(NodeFlags::Reparsed) {
                    return node;
                }
                if found_child.get().is_some() {
                    // We cannot abort visiting children, so once the desired child is found, we do nothing.
                    return node;
                }
                if position < nd.end() && prev_child.get().is_none_or(|p| p.end() <= position) {
                    found_child.set(Some(nd));
                } else {
                    prev_child.set(Some(nd));
                }
                node
            })
        };
        let visit_nodes: VisitNodesFn = {
            let found_child = found_child.clone();
            let prev_child = prev_child.clone();
            Rc::new(move |node_list: Option<P<NodeList>>, _: &mut NodeVisitor| {
                if found_child.get().is_some() {
                    return node_list;
                }
                if let Some(list) = node_list {
                    if !list.nodes.is_empty() {
                        let nodes = list.nodes;
                        let (index, match_) = binary_search_unique_func(nodes, |middle, _| {
                            // synthetic jsdoc nodes should have jsdocNode.End() <= n.Pos()
                            if nodes[middle].flags().intersects(NodeFlags::Reparsed) {
                                return COMPARISON_LESS_THAN;
                            }
                            if position < nodes[middle].end() {
                                if middle == 0 || position >= nodes[middle - 1].end() {
                                    return COMPARISON_EQUAL_TO;
                                }
                                return COMPARISON_GREATER_THAN;
                            }
                            COMPARISON_LESS_THAN
                        });

                        if match_ {
                            found_child.set(Some(nodes[index]));
                        }

                        let valid_lookup_index: i64 = if match_ { index as i64 - 1 } else { nodes.len() as i64 - 1 };
                        let mut i = valid_lookup_index;
                        while i >= 0 {
                            let node = nodes[i as usize];
                            i -= 1;
                            if node.flags().intersects(NodeFlags::Reparsed) {
                                continue;
                            }
                            if prev_child.get().is_none() {
                                prev_child.set(Some(node));
                            }
                        }
                    }
                }
                node_list
            })
        };
        visit_each_child_and_jsdoc(n, source_file, Some(visit_node), Some(visit_nodes));

        if let Some(found_child) = found_child.get() {
            // Note that the span of a node's tokens is [getStartOfNode(node, ...), node.end).
            // Given that `position < child.end` and child has constituent tokens, we distinguish these cases:
            // 1) `position` precedes `child`'s tokens or `child` has no tokens (ie: in a comment or whitespace preceding `child`):
            // we need to find the last token in a previous child node or child tokens.
            // 2) `position` is within the same span: we recurse on `child`.
            let start = get_start_of_node(found_child, source_file, !exclude_jsdoc /*includeJSDoc*/);
            let look_in_previous_child = start >= position || // cursor in the leading trivia or preceding tokens
                !is_valid_preceding_node(found_child, source_file);
            if look_in_previous_child {
                if position >= found_child.pos() {
                    // Find jsdoc preceding the foundChild.
                    let mut js_doc: Option<P<Node>> = None;
                    let node_jsdoc = n.jsdoc(Some(source_file.get()));
                    for &n in node_jsdoc.iter().rev() {
                        if n.pos() >= found_child.pos() {
                            js_doc = Some(n);
                            break;
                        }
                    }
                    if let Some(js_doc) = js_doc {
                        if !exclude_jsdoc && position < js_doc.end() {
                            return find(source_file, position, exclude_jsdoc, js_doc);
                        } else {
                            return find_rightmost_valid_token(js_doc.end(), source_file, n, position, exclude_jsdoc);
                        }
                    }
                    return find_rightmost_valid_token(found_child.pos(), source_file, n, -1 /*position*/, exclude_jsdoc);
                } else {
                    // Answer is in tokens between two visited children.
                    return find_rightmost_valid_token(found_child.pos(), source_file, n, position, exclude_jsdoc);
                }
            } else {
                // position is in [foundChild.getStart(), foundChild.End): recur.
                return find(source_file, position, exclude_jsdoc, found_child);
            }
        }

        // We have two cases here: either the position is at the end of the file,
        // or the desired token is in the unvisited trailing tokens of the current node.
        if position >= n.end() {
            find_rightmost_valid_token(n.end(), source_file, n, -1 /*position*/, exclude_jsdoc)
        } else {
            find_rightmost_valid_token(n.end(), source_file, n, position, exclude_jsdoc)
        }
    }

    let node = match start_node {
        Some(start_node) => start_node,
        None => source_file.as_node(),
    };
    let result = find(source_file, position, exclude_jsdoc, node);
    if let Some(result) = result {
        if ast::is_whitespace_only_jsx_text(result) {
            panic!("Expected result to be a non-whitespace token.");
        }
    }
    result
}

// tokens.go:450
fn is_valid_preceding_node(node: P<Node>, source_file: P<SourceFile>) -> bool {
    if node.kind() == Kind::EndOfFile {
        return !node.jsdoc(Some(source_file.get())).is_empty();
    }
    let start = get_start_of_node(node, source_file, false /*includeJSDoc*/);
    let width = node.end() - start;
    !(ast::is_whitespace_only_jsx_text(node) || width == 0)
}

// tokens.go:459
pub fn get_start_of_node(node: P<Node>, file: P<SourceFile>, include_jsdoc: bool) -> i32 {
    scanner::get_token_pos_of_node(node, file, include_jsdoc)
}

// tokens.go:465
// Looks for rightmost valid token in the range [startPos, endPos).
// If position is >= 0, looks for rightmost valid token that precedes or touches that position.
fn find_rightmost_valid_token(
    end_pos: i32,
    source_file: P<SourceFile>,
    containing_node: P<Node>,
    mut position: i32,
    exclude_jsdoc: bool,
) -> Option<P<Node>> {
    if position == -1 {
        position = containing_node.end();
    }
    fn should_visit_node(node: P<Node>, end_pos: i32, position: i32, source_file: P<SourceFile>, exclude_jsdoc: bool) -> bool {
        // Node is synthetic or out of the desired range: don't visit it.
        !(node.flags().intersects(NodeFlags::Reparsed)
            || node.end() > end_pos
            || get_start_of_node(node, source_file, !exclude_jsdoc /*includeJSDoc*/) >= position)
    }
    fn find(
        n: Option<P<Node>>,
        mut end_pos: i32,
        source_file: P<SourceFile>,
        containing_node: P<Node>,
        position: i32,
        exclude_jsdoc: bool,
    ) -> Option<P<Node>> {
        let n = n?;
        if ast::is_non_whitespace_token(n) {
            return Some(n);
        }

        let rightmost_valid_node: Rc<Cell<Option<P<Node>>>> = Rc::new(Cell::new(None));
        let rightmost_visited_nodes: Rc<RefCell<Vec<P<Node>>>> = Rc::new(RefCell::new(Vec::with_capacity(1))); // Nodes after the last valid node.
        let has_children: Rc<Cell<bool>> = Rc::new(Cell::new(false));
        let visit_node: VisitNodeFn = {
            let rightmost_valid_node = rightmost_valid_node.clone();
            let rightmost_visited_nodes = rightmost_visited_nodes.clone();
            let has_children = has_children.clone();
            Rc::new(move |node: Option<P<Node>>, _: &mut NodeVisitor| {
                let Some(nd) = node else {
                    return node;
                };
                if nd.flags().intersects(NodeFlags::Reparsed) {
                    return node;
                }
                has_children.set(true);
                if !should_visit_node(nd, end_pos, position, source_file, exclude_jsdoc) {
                    return node;
                }
                rightmost_visited_nodes.borrow_mut().push(nd);
                if is_valid_preceding_node(nd, source_file) {
                    rightmost_valid_node.set(Some(nd));
                    rightmost_visited_nodes.borrow_mut().clear();
                }
                node
            })
        };
        let visit_nodes: VisitNodesFn = {
            let rightmost_valid_node = rightmost_valid_node.clone();
            let rightmost_visited_nodes = rightmost_visited_nodes.clone();
            let has_children = has_children.clone();
            Rc::new(move |node_list: Option<P<NodeList>>, _: &mut NodeVisitor| {
                if let Some(list) = node_list {
                    if !list.nodes.is_empty() {
                        has_children.set(true);
                        let nodes = list.nodes;
                        let (index, _) = binary_search_unique_func(nodes, |_, node| {
                            if node.end() > end_pos {
                                return COMPARISON_GREATER_THAN;
                            }
                            COMPARISON_LESS_THAN
                        });
                        let mut valid_index: i64 = -1;
                        let mut i = index as i64 - 1;
                        while i >= 0 {
                            let node = nodes[i as usize];
                            i -= 1;
                            if !should_visit_node(node, end_pos, position, source_file, exclude_jsdoc) {
                                continue;
                            }
                            if is_valid_preceding_node(node, source_file) {
                                valid_index = i + 1;
                                rightmost_valid_node.set(Some(node));
                                break;
                            }
                        }
                        let mut i = valid_index + 1;
                        while i < index as i64 {
                            let node = nodes[i as usize];
                            i += 1;
                            if !should_visit_node(node, end_pos, position, source_file, exclude_jsdoc) {
                                continue;
                            }
                            rightmost_visited_nodes.borrow_mut().push(node);
                        }
                    }
                }
                node_list
            })
        };
        visit_each_child_and_jsdoc(n, source_file, Some(visit_node), Some(visit_nodes));

        // Three cases:
        // 1. The answer is a token of `rightmostValidNode`.
        // 2. The answer is one of the unvisited tokens that occur after the rightmost valid node.
        // 3. The current node is a childless, token-less node. The answer is the current node.

        // Case 2: Look at unvisited trailing tokens that occur in between the rightmost visited nodes.
        if !should_skip_child(n) {
            // JSDoc nodes don't include trivia tokens as children.
            let mut start_pos = match rightmost_valid_node.get() {
                Some(r) => r.end(),
                None => n.pos(),
            };
            let mut scanner = scanner::get_scanner_for_source_file(source_file, start_pos);
            let mut tokens: Vec<P<Node>> = Vec::new();
            let visited: Vec<P<Node>> = rightmost_visited_nodes.borrow().clone();
            for visited_node in visited {
                // Trailing tokens that occur before this node.
                while start_pos < visited_node.pos().min(position) {
                    let token = scan_navigation_token(&mut scanner, n);
                    let token_start = scanner.token_start();
                    if token_start >= visited_node.pos().min(position) {
                        break;
                    }
                    let token_full_start = scanner.token_full_start();
                    let token_end = scanner.token_end();
                    start_pos = token_end;
                    let flags = scanner.token_flags();
                    tokens.push(source_file.get_or_create_token(token, token_full_start, token_end, n, flags));
                    scanner.scan();
                }
                start_pos = visited_node.end();
                scanner.reset_pos(start_pos);
                scanner.scan();
            }
            // Trailing tokens after last visited node.
            while start_pos < end_pos.min(position) {
                let token = scan_navigation_token(&mut scanner, n);
                let token_start = scanner.token_start();
                if token_start >= end_pos.min(position) {
                    break;
                }
                let token_full_start = scanner.token_full_start();
                let token_end = scanner.token_end();
                start_pos = token_end;
                let flags = scanner.token_flags();
                tokens.push(source_file.get_or_create_token(token, token_full_start, token_end, n, flags));
                scanner.scan();
            }

            // Find preceding valid token.
            for &token in tokens.iter().rev() {
                if !ast::is_whitespace_only_jsx_text(token) {
                    return Some(token);
                }
            }
        }

        // Case 3: childless node.
        if !has_children.get() {
            if n != containing_node {
                return Some(n);
            }
            return None;
        }
        // Case 1: recur on rightmostValidNode.
        if let Some(r) = rightmost_valid_node.get() {
            end_pos = r.end();
        }
        find(rightmost_valid_node.get(), end_pos, source_file, containing_node, position, exclude_jsdoc)
    }

    find(Some(containing_node), end_pos, source_file, containing_node, position, exclude_jsdoc)
}

// tokens.go:610
pub fn find_next_token(previous_token: P<Node>, parent: P<Node>, file: P<SourceFile>) -> Option<P<Node>> {
    fn find(n: P<Node>, previous_token: P<Node>, file: P<SourceFile>) -> Option<P<Node>> {
        if ast::is_token_kind(n.kind()) && n.pos() == previous_token.end() {
            // this is token that starts at the end of previous token - return it
            return Some(n);
        }
        // Node that contains `previousToken` or occurs immediately after it.
        let found_node: Rc<Cell<Option<P<Node>>>> = Rc::new(Cell::new(None));
        let visit_node: VisitNodeFn = {
            let found_node = found_node.clone();
            Rc::new(move |node: Option<P<Node>>, _: &mut NodeVisitor| {
                if let Some(nd) = node {
                    if !nd.flags().intersects(NodeFlags::Reparsed) && nd.pos() <= previous_token.end() && nd.end() > previous_token.end() {
                        found_node.set(Some(nd));
                    }
                }
                node
            })
        };
        let visit_nodes: VisitNodesFn = {
            let found_node = found_node.clone();
            Rc::new(move |node_list: Option<P<NodeList>>, _: &mut NodeVisitor| {
                if let Some(list) = node_list {
                    if !list.nodes.is_empty() && found_node.get().is_none() {
                        let nodes = list.nodes;
                        let (index, match_) = binary_search_unique_func(nodes, |_, node| {
                            if node.flags().intersects(NodeFlags::Reparsed) {
                                return COMPARISON_LESS_THAN;
                            }
                            if node.pos() > previous_token.end() {
                                return COMPARISON_GREATER_THAN;
                            }
                            if node.end() <= previous_token.pos() {
                                return COMPARISON_LESS_THAN;
                            }
                            COMPARISON_EQUAL_TO
                        });
                        if match_ {
                            found_node.set(Some(nodes[index]));
                        }
                    }
                }
                node_list
            })
        };
        visit_each_child_and_jsdoc(n, file, Some(visit_node), Some(visit_nodes));
        // Cases:
        // 1. no answer exists
        // 2. answer is an unvisited token
        // 3. answer is in the visited found node

        // Case 3: look for the next token inside the found node.
        if let Some(found_node) = found_node.get() {
            return find(found_node, previous_token, file);
        }
        let start_pos = previous_token.end();
        // Case 2: look for the next token directly.
        if start_pos >= n.pos() && start_pos < n.end() {
            let scanner = scanner::get_scanner_for_source_file(file, start_pos);
            let token = scanner.token();
            let token_full_start = scanner.token_full_start();
            let token_end = scanner.token_end();
            let flags = scanner.token_flags();
            // Use tokenFullStart (which includes leading trivia) to match TS's
            // findNextToken behavior where `n.pos === previousToken.end` is checked
            // (TS's pos includes trivia, same as Go's Pos()/tokenFullStart).
            if token_full_start == previous_token.end() {
                return Some(file.get_or_create_token(token, token_full_start, token_end, n, flags));
            }
            panic!("Expected to find next token at {}, got token {:?} at {}", previous_token.end(), token, token_full_start);
        }
        // Case 3: no answer.
        None
    }
    find(parent, previous_token, file)
}

// tokens.go:690
fn get_node_visitor(visit_node: Option<VisitNodeFn>, visit_nodes: Option<VisitNodesFn>) -> NodeVisitor {
    let wrapped_visit_node: Option<VisitNodeFn> = visit_node.map(|visit_node| -> VisitNodeFn {
        Rc::new(move |n: Option<P<Node>>, v: &mut NodeVisitor| {
            if ast::is_jsdoc_single_comment_node_comment(n) {
                return n;
            }
            visit_node(n, v)
        })
    });

    let wrapped_visit_nodes: Option<VisitNodesFn> = visit_nodes.map(|visit_nodes| -> VisitNodesFn {
        Rc::new(move |n: Option<P<NodeList>>, v: &mut NodeVisitor| {
            if ast::is_jsdoc_single_comment_node_list(n) {
                return n;
            }
            visit_nodes(n, v)
        })
    });

    let visit_modifiers = {
        let wrapped_visit_nodes = wrapped_visit_nodes.clone();
        Rc::new(move |modifiers: Option<P<ModifierList>>, visitor: &mut NodeVisitor| {
            if let Some(m) = modifiers {
                wrapped_visit_nodes.as_ref().unwrap()(Some(P::from_static(&m.get().list)), visitor);
            }
            modifiers
        })
    };

    ast::new_node_visitor(
        Some(Rc::new(|_: &mut NodeVisitor, n: P<Node>| Some(n))),
        None,
        NodeVisitorHooks {
            visit_node: wrapped_visit_node.clone(),
            visit_token: wrapped_visit_node,
            visit_nodes: wrapped_visit_nodes,
            visit_modifiers: Some(visit_modifiers),
            ..Default::default()
        },
    )
}

// tokens.go:727
fn should_skip_child(node: P<Node>) -> bool {
    node.kind() == Kind::JSDoc
        || node.kind() == Kind::JSDocText
        || node.kind() == Kind::JSDocTypeLiteral
        || node.kind() == Kind::JSDocSignature
        || ast::is_jsdoc_link_like(node)
        || ast::is_jsdoc_tag(node)
}

// tokens.go:738
// FindChildOfKind searches for a child node or token of the specified kind within a containing node.
// This function scans through both AST nodes and intervening tokens to find the first match.
pub fn find_child_of_kind(containing_node: P<Node>, kind: Kind, source_file: P<SourceFile>) -> Option<P<Node>> {
    let mut last_node_pos = containing_node.pos();
    let mut scan = scanner::get_scanner_for_source_file(source_file, last_node_pos);

    let mut found_child: Option<P<Node>> = None;
    let mut visit_node = |node: P<Node>| -> bool {
        if node.flags().intersects(NodeFlags::Reparsed) {
            return false;
        }
        // Look for child in preceding tokens.
        let mut start_pos = last_node_pos;
        while start_pos < node.pos() {
            let token_kind = scan.token();
            let token_end = scan.token_end();
            if token_kind == kind {
                let token_full_start = scan.token_full_start();
                let flags = scan.token_flags();
                found_child = Some(source_file.get_or_create_token(token_kind, token_full_start, token_end, containing_node, flags));
                return true;
            }
            start_pos = token_end;
            scan.scan();
        }

        if node.kind() == kind {
            found_child = Some(node);
            return true;
        }

        last_node_pos = node.end();
        scan.reset_pos(last_node_pos);
        false
    };

    ast::for_each_child_and_jsdoc(containing_node, source_file.get(), &mut visit_node);

    if found_child.is_some() {
        return found_child;
    }

    // Look for child in trailing tokens.
    let mut start_pos = last_node_pos;
    while start_pos < containing_node.end() {
        let token_kind = scan.token();
        let token_end = scan.token_end();
        if token_kind == kind {
            let token_full_start = scan.token_full_start();
            let flags = scan.token_flags();
            let token = source_file.get_or_create_token(token_kind, token_full_start, token_end, containing_node, flags);
            return Some(token);
        }
        start_pos = token_end;
        scan.scan();
    }
    None
}
