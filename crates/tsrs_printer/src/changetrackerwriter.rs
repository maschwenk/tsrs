use std::cell::RefCell;
use std::rc::Rc;

use rustc_hash::FxHashMap;
use tsrs_ast::*;
use tsrs_core::stringutil;
use tsrs_core::*;

use crate::{textWriter, EmitTextWriter, PrintHandlers};

// Go `triviaPositionKey` (`*ast.Node | *ast.NodeList`): keyed by address, like Go's interface-of-pointer map key.
type triviaPositionKey = usize;

fn node_key(node: P<Node>) -> triviaPositionKey {
    node.get() as *const Node as usize
}

fn list_key(list: P<NodeList>) -> triviaPositionKey {
    list.get() as *const NodeList as usize
}

// The part of the writer the print handlers write (Go's handlers close over the writer).
#[derive(Default)]
struct changeTrackerPositions {
    last_non_trivia_position: i32,
    pos: FxHashMap<triviaPositionKey, i32>,
    end: FxHashMap<triviaPositionKey, i32>,
}

pub struct ChangeTrackerWriter {
    text_writer: textWriter,
    positions: Rc<RefCell<changeTrackerPositions>>,
}

// changetrackerwriter.go:25
pub fn new_change_tracker_writer(newline: &str, indent_size: i32) -> ChangeTrackerWriter {
    // TODO: Callers passing -1 should pass actual indent options once indent-related formatting is ported.
    let mut indent_size = indent_size;
    if indent_size < 0 {
        indent_size = crate::get_default_indent_size() as i32;
    }
    let mut text_writer = textWriter::default();
    text_writer.set_new_line_and_indent_size(newline, indent_size as usize);
    let mut ctw = ChangeTrackerWriter { text_writer, positions: Rc::new(RefCell::new(changeTrackerPositions::default())) };
    ctw.text_writer.clear();
    ctw
}

impl ChangeTrackerWriter {
    // changetrackerwriter.go:40
    pub fn get_print_handlers(&self) -> PrintHandlers {
        let p1 = self.positions.clone();
        let p2 = self.positions.clone();
        let p3 = self.positions.clone();
        let p4 = self.positions.clone();
        let p5 = self.positions.clone();
        let p6 = self.positions.clone();
        PrintHandlers {
            on_before_emit_node: Some(Box::new(move |node_opt| {
                if let Some(n) = node_opt {
                    set_pos(&p1, node_key(n));
                }
            })),
            on_after_emit_node: Some(Box::new(move |node_opt| {
                if let Some(n) = node_opt {
                    set_end(&p2, node_key(n));
                }
            })),
            on_before_emit_node_list: Some(Box::new(move |nodes_opt| {
                if let Some(n) = nodes_opt {
                    set_pos(&p3, list_key(n));
                }
            })),
            on_after_emit_node_list: Some(Box::new(move |nodes_opt| {
                if let Some(n) = nodes_opt {
                    set_end(&p4, list_key(n));
                }
            })),
            on_before_emit_token: Some(Box::new(move |node_opt| {
                if let Some(n) = node_opt {
                    set_pos(&p5, node_key(n));
                }
            })),
            on_after_emit_token: Some(Box::new(move |node_opt| {
                if let Some(n) = node_opt {
                    set_end(&p6, node_key(n));
                }
            })),
            ..Default::default()
        }
    }

    // changetrackerwriter.go:83
    fn get_pos(&self, node: triviaPositionKey) -> i32 {
        self.positions.borrow().pos.get(&node).copied().unwrap_or(0)
    }

    // changetrackerwriter.go:87
    fn get_end(&self, node: triviaPositionKey) -> i32 {
        self.positions.borrow().end.get(&node).copied().unwrap_or(0)
    }

    // changetrackerwriter.go:91
    fn set_last_non_trivia_position(&mut self, s: &str, force: bool) {
        if force || tsrs_scanner::skip_trivia(s, 0) != s.len() as i32 {
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

    // changetrackerwriter.go:108
    pub fn assign_positions_to_node(&self, node: P<Node>, factory: &NodeFactory) -> P<Node> {
        let positions = self.positions.clone();
        let p_visit = positions.clone();
        let p_node = positions.clone();
        let p_nodes = positions.clone();
        let p_token = positions.clone();
        let p_mods = positions.clone();
        let mod_factory = factory.clone();
        let mut visitor = new_node_visitor(
            Some(Rc::new(move |v: &mut NodeVisitor, n: P<Node>| Some(assign_positions_to_node_worker(&p_visit, n, v)))),
            Some(factory.clone()),
            NodeVisitorHooks {
                visit_node: Some(Rc::new(move |n: Option<P<Node>>, v: &mut NodeVisitor| {
                    n.map(|n| assign_positions_to_node_worker(&p_node, n, v))
                })),
                visit_nodes: Some(Rc::new(move |n: Option<P<NodeList>>, v: &mut NodeVisitor| assign_positions_to_node_array(&p_nodes, n, v))),
                visit_token: Some(Rc::new(move |n: Option<P<Node>>, v: &mut NodeVisitor| {
                    n.map(|n| assign_positions_to_node_worker(&p_token, n, v))
                })),
                visit_modifiers: Some(Rc::new(move |modifiers: Option<P<ModifierList>>, v: &mut NodeVisitor| {
                    if let Some(modifiers) = modifiers {
                        let list = P::from_static(&modifiers.get().list);
                        let new_node_list = assign_positions_to_node_array(&p_mods, Some(list), v).unwrap();
                        // Return a new ModifierList so that VisitEachChild/Update detects the
                        // change and creates a new node with reassigned child positions.
                        return Some(mod_factory.new_modifier_list(new_node_list.nodes.to_vec()));
                    }
                    modifiers
                })),
                ..Default::default()
            },
        );
        assign_positions_to_node_worker(&positions, node, &mut visitor)
    }
}

fn set_pos(positions: &Rc<RefCell<changeTrackerPositions>>, node: triviaPositionKey) {
    let mut p = positions.borrow_mut();
    let last = p.last_non_trivia_position;
    p.pos.insert(node, last);
}

fn set_end(positions: &Rc<RefCell<changeTrackerPositions>>, node: triviaPositionKey) {
    let mut p = positions.borrow_mut();
    let last = p.last_non_trivia_position;
    p.end.insert(node, last);
}

// changetrackerwriter.go:134
fn assign_positions_to_node_worker(positions: &Rc<RefCell<changeTrackerPositions>>, node: P<Node>, v: &mut NodeVisitor) -> P<Node> {
    let visited = node.visit_each_child(v);
    // Assigning positions must not mutate the caller's node: it may be printed again (a change in a
    // content-mapped file is formatted once per virtual projection of its insertion point), and a node
    // that has acquired positions is printed by reading text back out of the source file. VisitEachChild
    // returns a fresh node only when a child changed, so clone whenever it hands back the input.
    let mut new_node = visited;
    if visited == node {
        new_node = visited.clone_node(&v.factory);
    }
    new_node.for_each_child(&mut |child| {
        child.set_parent(Some(new_node));
        false
    });
    let (pos, end) = {
        let p = positions.borrow();
        let key = node_key(node);
        (p.pos.get(&key).copied().unwrap_or(0), p.end.get(&key).copied().unwrap_or(0))
    };
    new_node.set_loc(TextRange::new(pos, end));
    new_node
}

// changetrackerwriter.go:159
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

    let (pos, end) = {
        let p = positions.borrow();
        let key = list_key(nodes);
        (p.pos.get(&key).copied().unwrap_or(0), p.end.get(&key).copied().unwrap_or(0))
    };
    node_array.loc.set(TextRange::new(pos, end));
    Some(node_array)
}

impl EmitTextWriter for ChangeTrackerWriter {
    // changetrackerwriter.go:180
    fn write(&mut self, text: &str) {
        self.text_writer.write(text);
        self.set_last_non_trivia_position(text, false);
    }
    fn write_trailing_semicolon(&mut self, text: &str) {
        self.text_writer.write_trailing_semicolon(text);
        self.set_last_non_trivia_position(text, false);
    }
    fn write_comment(&mut self, text: &str) {
        self.text_writer.write_comment(text)
    }
    fn write_keyword(&mut self, text: &str) {
        self.text_writer.write_keyword(text);
        self.set_last_non_trivia_position(text, false);
    }
    fn write_operator(&mut self, text: &str) {
        self.text_writer.write_operator(text);
        self.set_last_non_trivia_position(text, false);
    }
    fn write_punctuation(&mut self, text: &str) {
        self.text_writer.write_punctuation(text);
        self.set_last_non_trivia_position(text, false);
    }
    fn write_space(&mut self, text: &str) {
        self.text_writer.write_space(text);
        self.set_last_non_trivia_position(text, false);
    }
    fn write_string_literal(&mut self, text: &str) {
        self.text_writer.write_string_literal(text);
        self.set_last_non_trivia_position(text, false);
    }
    fn write_parameter(&mut self, text: &str) {
        self.text_writer.write_parameter(text);
        self.set_last_non_trivia_position(text, false);
    }
    fn write_property(&mut self, text: &str) {
        self.text_writer.write_property(text);
        self.set_last_non_trivia_position(text, false);
    }
    fn write_symbol(&mut self, text: &str, symbol: P<Symbol>) {
        self.text_writer.write_symbol(text, symbol);
        self.set_last_non_trivia_position(text, false);
    }
    fn write_line(&mut self) {
        self.text_writer.write_line()
    }
    fn write_line_force(&mut self, force: bool) {
        self.text_writer.write_line_force(force)
    }
    fn increase_indent(&mut self) {
        self.text_writer.increase_indent()
    }
    fn decrease_indent(&mut self) {
        self.text_writer.decrease_indent()
    }
    fn clear(&mut self) {
        self.text_writer.clear();
        self.positions.borrow_mut().last_non_trivia_position = 0;
    }
    fn string(&self) -> String {
        self.text_writer.string()
    }
    fn raw_write(&mut self, s: &str) {
        self.text_writer.raw_write(s);
        self.set_last_non_trivia_position(s, false);
    }
    fn write_literal(&mut self, s: &str) {
        self.text_writer.write_literal(s);
        self.set_last_non_trivia_position(s, true);
    }
    fn get_text_pos(&self) -> i32 {
        self.text_writer.get_text_pos()
    }
    fn get_line(&self) -> i32 {
        self.text_writer.get_line()
    }
    fn get_column(&self) -> UTF16Offset {
        self.text_writer.get_column()
    }
    fn get_indent(&self) -> i32 {
        self.text_writer.get_indent()
    }
    fn is_at_start_of_line(&self) -> bool {
        self.text_writer.is_at_start_of_line()
    }
    fn has_trailing_comment(&self) -> bool {
        self.text_writer.has_trailing_comment()
    }
    fn has_trailing_whitespace(&self) -> bool {
        self.text_writer.has_trailing_whitespace()
    }
}
