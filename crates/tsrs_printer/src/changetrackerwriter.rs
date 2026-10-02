use std::cell::RefCell;
use std::rc::Rc;

use rustc_hash::FxHashMap;
use tsrs_ast::*;
use tsrs_core::stringutil;
use tsrs_core::*;
use tsrs_scanner as scanner;

use crate::textwriter::{textWriter, get_default_indent_size};
use crate::{EmitTextWriter, PrintHandlers};

// changetrackerwriter.go:19 (Go interface `*ast.Node | *ast.NodeList`, compared by pointer)
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum triviaPositionKey {
    Node(P<Node>),
    NodeList(P<NodeList>),
}

// The fields Go's `GetPrintHandlers` closures share with the writer through the `*ChangeTrackerWriter` pointer.
#[derive(Default)]
struct changeTrackerPositions {
    last_non_trivia_position: i32,
    pos: FxHashMap<triviaPositionKey, i32>,
    end: FxHashMap<triviaPositionKey, i32>,
}

// changetrackerwriter.go:12
pub struct ChangeTrackerWriter {
    text_writer: textWriter,
    positions: Rc<RefCell<changeTrackerPositions>>,
}

// changetrackerwriter.go:24
pub fn new_change_tracker_writer(newline: &str, indent_size: i32) -> ChangeTrackerWriter {
    // TODO: Callers passing -1 should pass actual indent options once indent-related formatting is ported.
    let indent_size = if indent_size < 0 { get_default_indent_size() } else { indent_size as usize };
    let mut ctw = ChangeTrackerWriter {
        text_writer: textWriter::new_with(newline, indent_size),
        positions: Rc::new(RefCell::new(changeTrackerPositions::default())),
    };
    ctw.text_writer.clear();
    ctw
}

impl ChangeTrackerWriter {
    // changetrackerwriter.go:39
    pub fn get_print_handlers(&self) -> PrintHandlers {
        let p1 = self.positions.clone();
        let p2 = self.positions.clone();
        let p3 = self.positions.clone();
        let p4 = self.positions.clone();
        let p5 = self.positions.clone();
        let p6 = self.positions.clone();
        PrintHandlers {
            on_before_emit_node: Some(Box::new(move |node_opt: Option<P<Node>>| {
                if let Some(node) = node_opt {
                    set_pos(&p1, triviaPositionKey::Node(node));
                }
            })),
            on_after_emit_node: Some(Box::new(move |node_opt: Option<P<Node>>| {
                if let Some(node) = node_opt {
                    set_end(&p2, triviaPositionKey::Node(node));
                }
            })),
            on_before_emit_node_list: Some(Box::new(move |nodes_opt: Option<P<NodeList>>| {
                if let Some(nodes) = nodes_opt {
                    set_pos(&p3, triviaPositionKey::NodeList(nodes));
                }
            })),
            on_after_emit_node_list: Some(Box::new(move |nodes_opt: Option<P<NodeList>>| {
                if let Some(nodes) = nodes_opt {
                    set_end(&p4, triviaPositionKey::NodeList(nodes));
                }
            })),
            on_before_emit_token: Some(Box::new(move |node_opt: Option<P<Node>>| {
                if let Some(node) = node_opt {
                    set_pos(&p5, triviaPositionKey::Node(node));
                }
            })),
            on_after_emit_token: Some(Box::new(move |node_opt: Option<P<Node>>| {
                if let Some(node) = node_opt {
                    set_end(&p6, triviaPositionKey::Node(node));
                }
            })),
            ..Default::default()
        }
    }

    // changetrackerwriter.go:82
    fn get_pos(&self, node: triviaPositionKey) -> i32 {
        self.positions.borrow().pos.get(&node).copied().unwrap_or(0)
    }

    // changetrackerwriter.go:86
    fn get_end(&self, node: triviaPositionKey) -> i32 {
        self.positions.borrow().end.get(&node).copied().unwrap_or(0)
    }

    // changetrackerwriter.go:90
    fn set_last_non_trivia_position(&mut self, s: &str, force: bool) {
        if force || scanner::skip_trivia(s, 0) as usize != s.len() {
            let mut last = self.text_writer.get_text_pos();
            // trim trailing whitespaces
            let mut pos = s.len();
            while pos > 0 {
                let (r, size) = stringutil::decode_last_rune(&s.as_bytes()[..pos]);
                if stringutil::is_white_space_like(r) {
                    pos -= size;
                } else {
                    break;
                }
            }
            last -= (s.len() - pos) as i32;
            self.positions.borrow_mut().last_non_trivia_position = last;
        }
    }

    // changetrackerwriter.go:107
    pub fn assign_positions_to_node(&self, node: P<Node>, factory: &NodeFactory) -> P<Node> {
        let positions = self.positions.clone();
        let p_visit = positions.clone();
        let p_node = positions.clone();
        let p_nodes = positions.clone();
        let p_token = positions.clone();
        let p_modifiers = positions.clone();
        let modifiers_factory = factory.clone();
        let mut visitor = new_node_visitor(
            Some(Rc::new(move |v: &mut NodeVisitor, n: P<Node>| assign_positions_to_node_worker(&p_visit, Some(n), v))),
            Some(factory.clone()),
            NodeVisitorHooks {
                visit_node: Some(Rc::new(move |n: Option<P<Node>>, v: &mut NodeVisitor| assign_positions_to_node_worker(&p_node, n, v))),
                visit_nodes: Some(Rc::new(move |nodes: Option<P<NodeList>>, v: &mut NodeVisitor| assign_positions_to_node_array(&p_nodes, nodes, v))),
                visit_token: Some(Rc::new(move |n: Option<P<Node>>, v: &mut NodeVisitor| assign_positions_to_node_worker(&p_token, n, v))),
                visit_modifiers: Some(Rc::new(move |modifiers: Option<P<ModifierList>>, v: &mut NodeVisitor| {
                    if let Some(modifiers) = modifiers {
                        let new_node_list = assign_positions_to_node_array(&p_modifiers, Some(P::from_static(&modifiers.get().list)), v).unwrap();
                        // Return a new ModifierList so that VisitEachChild/Update detects the
                        // change and creates a new node with reassigned child positions.
                        return Some(modifiers_factory.new_modifier_list_from_slice(new_node_list.nodes()));
                    }
                    modifiers
                })),
                ..Default::default()
            },
        );
        assign_positions_to_node_worker(&positions, Some(node), &mut visitor).unwrap()
    }
}

// changetrackerwriter.go:74
fn set_pos(positions: &Rc<RefCell<changeTrackerPositions>>, node: triviaPositionKey) {
    let mut p = positions.borrow_mut();
    let last = p.last_non_trivia_position;
    p.pos.insert(node, last);
}

// changetrackerwriter.go:78
fn set_end(positions: &Rc<RefCell<changeTrackerPositions>>, node: triviaPositionKey) {
    let mut p = positions.borrow_mut();
    let last = p.last_non_trivia_position;
    p.end.insert(node, last);
}

// changetrackerwriter.go:130
fn assign_positions_to_node_worker(positions: &Rc<RefCell<changeTrackerPositions>>, node: Option<P<Node>>, v: &mut NodeVisitor) -> Option<P<Node>> {
    let node = node?;
    let visited = node.visit_each_child(v);
    // Assigning positions must not mutate the caller's node: it may be printed again (a change in a
    // content-mapped file is formatted once per virtual projection of its insertion point), and a node
    // that has acquired positions is printed by reading text back out of the source file. VisitEachChild
    // returns a fresh node only when a child changed, so clone whenever it hands back the input.
    let mut new_node = visited;
    if visited == node {
        new_node = visited.clone_node(&v.factory);
    }
    // Go's callback returns true, which stops ForEachChild after the first child.
    new_node.for_each_child(&mut |child| {
        child.set_parent(Some(new_node));
        true
    });
    let p = positions.borrow();
    let key = triviaPositionKey::Node(node);
    new_node.set_loc(TextRange::new(p.pos.get(&key).copied().unwrap_or(0), p.end.get(&key).copied().unwrap_or(0)));
    Some(new_node)
}

// changetrackerwriter.go:154
fn assign_positions_to_node_array(positions: &Rc<RefCell<changeTrackerPositions>>, nodes: Option<P<NodeList>>, v: &mut NodeVisitor) -> Option<P<NodeList>> {
    let visited = v.visit_nodes(nodes)?;
    let Some(nodes) = nodes else {
        // Debug.assert(nodes);
        panic!("if nodes is nil, visited should not be nil");
    };
    // clone nodearray if necessary
    let mut node_array = visited;
    if visited == nodes {
        node_array = visited.clone_list(&v.factory);
    }

    let p = positions.borrow();
    let key = triviaPositionKey::NodeList(nodes);
    node_array.loc.set(TextRange::new(p.pos.get(&key).copied().unwrap_or(0), p.end.get(&key).copied().unwrap_or(0)));
    Some(node_array)
}

impl EmitTextWriter for ChangeTrackerWriter {
    // changetrackerwriter.go:176
    fn write(&mut self, text: &str) {
        self.text_writer.write(text);
        self.set_last_non_trivia_position(text, false);
    }

    // changetrackerwriter.go:181
    fn write_trailing_semicolon(&mut self, text: &str) {
        self.text_writer.write_trailing_semicolon(text);
        self.set_last_non_trivia_position(text, false);
    }
    // changetrackerwriter.go:185
    fn write_comment(&mut self, text: &str) {
        self.text_writer.write_comment(text)
    }
    // changetrackerwriter.go:186
    fn write_keyword(&mut self, text: &str) {
        self.text_writer.write_keyword(text);
        self.set_last_non_trivia_position(text, false);
    }

    // changetrackerwriter.go:191
    fn write_operator(&mut self, text: &str) {
        self.text_writer.write_operator(text);
        self.set_last_non_trivia_position(text, false);
    }

    // changetrackerwriter.go:196
    fn write_punctuation(&mut self, text: &str) {
        self.text_writer.write_punctuation(text);
        self.set_last_non_trivia_position(text, false);
    }

    // changetrackerwriter.go:201
    fn write_space(&mut self, text: &str) {
        self.text_writer.write_space(text);
        self.set_last_non_trivia_position(text, false);
    }

    // changetrackerwriter.go:206
    fn write_string_literal(&mut self, text: &str) {
        self.text_writer.write_string_literal(text);
        self.set_last_non_trivia_position(text, false);
    }

    // changetrackerwriter.go:211
    fn write_parameter(&mut self, text: &str) {
        self.text_writer.write_parameter(text);
        self.set_last_non_trivia_position(text, false);
    }

    // changetrackerwriter.go:216
    fn write_property(&mut self, text: &str) {
        self.text_writer.write_property(text);
        self.set_last_non_trivia_position(text, false);
    }

    // changetrackerwriter.go:221
    fn write_symbol(&mut self, text: &str, symbol: P<Symbol>) {
        self.text_writer.write_symbol(text, symbol);
        self.set_last_non_trivia_position(text, false);
    }
    // changetrackerwriter.go:225
    fn write_line(&mut self) {
        self.text_writer.write_line()
    }
    // changetrackerwriter.go:226
    fn write_line_force(&mut self, force: bool) {
        self.text_writer.write_line_force(force)
    }
    // changetrackerwriter.go:227
    fn increase_indent(&mut self) {
        self.text_writer.increase_indent()
    }
    // changetrackerwriter.go:228
    fn decrease_indent(&mut self) {
        self.text_writer.decrease_indent()
    }
    // changetrackerwriter.go:229
    fn clear(&mut self) {
        self.text_writer.clear();
        self.positions.borrow_mut().last_non_trivia_position = 0;
    }
    // changetrackerwriter.go:230
    fn string(&self) -> String {
        self.text_writer.string()
    }
    // changetrackerwriter.go:231
    fn raw_write(&mut self, s: &str) {
        self.text_writer.raw_write(s);
        self.set_last_non_trivia_position(s, false);
    }

    // changetrackerwriter.go:236
    fn write_literal(&mut self, s: &str) {
        self.text_writer.write_literal(s);
        self.set_last_non_trivia_position(s, true);
    }
    // changetrackerwriter.go:240
    fn get_text_pos(&self) -> i32 {
        self.text_writer.get_text_pos()
    }
    // changetrackerwriter.go:241
    fn get_line(&self) -> i32 {
        self.text_writer.get_line()
    }
    // changetrackerwriter.go:242
    fn get_column(&self) -> UTF16Offset {
        self.text_writer.get_column()
    }
    // changetrackerwriter.go:243
    fn get_indent(&self) -> i32 {
        self.text_writer.get_indent()
    }
    // changetrackerwriter.go:244
    fn is_at_start_of_line(&self) -> bool {
        self.text_writer.is_at_start_of_line()
    }

    // changetrackerwriter.go:246
    fn has_trailing_comment(&self) -> bool {
        self.text_writer.has_trailing_comment()
    }

    // changetrackerwriter.go:248
    fn has_trailing_whitespace(&self) -> bool {
        self.text_writer.has_trailing_whitespace()
    }
}
