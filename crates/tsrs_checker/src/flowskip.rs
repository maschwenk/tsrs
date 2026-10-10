// Not in Go: skipping flow walks that can only end at the declared type (notes/perf-flow-skip-unnarrowed.md).
//
// The binder records, per flow graph, the reference paths its conditions and switches mention (with every prefix and
// what their aliases may narrow), its assignment targets, the paths its assertion-candidate calls mention, and the
// earliest position of each (tsrs_ast and tsrs_binder flownames.rs). A walk visits nodes of the reference's graph that
// the binder made before the reference's, and nodes of the graphs its `Start` nodes continue to. When none of those
// that a walk can reach mentions the reference's path (or a prefix with a wildcard below it), no condition, switch or
// `for...in` narrows it; an assignment to it gives the declared type unless that is a union or literal type; and an
// assertion-candidate call that mentions it whose effects signature is known to be none neither narrows it nor ends
// the path. So every path ends at the initial type, at an unreachable node (the declared type), or at a never-returning
// call (the unreachable never type, which `getFlowTypeOfReference` maps to the declared type), and with initial and
// declared type the same the walk returns the declared type. The walk's other effects (resolving the names and effects
// signatures it passes, `flowLoopCache` entries for the same answer) happen anyway when those nodes are checked or
// walked for another reference; `flowInvocationCount` is still counted.

use rustc_hash::{FxHashMap, FxHashSet};
use tsrs_ast as ast;
use tsrs_ast::flownames::{flow_graph_of, flow_reach_limit, flow_name_index, flow_name_key, flow_path_key, FlowCallMention, FlowSkipMode, FLOW_WILDCARD};
use tsrs_ast::{FlowNode, Kind, Node};
use tsrs_core::P;

use crate::*;

/// Walks through graphs with at least this many flow nodes in total are not skipped: a walk's recursion depth is at
/// most twice the flow nodes it can reach, so a skipped walk could never reach the 2000-frame limit that reports
/// TS2563 (`getTypeAtFlowNode`).
const NODE_LIMIT: u32 = 500;

/// References with longer property paths are not skipped.
const MAX_COMPONENTS: usize = 15;

pub struct FlowSkip {
    pub mode: FlowSkipMode,
    /// Path key -> the global aliases (flownames.rs) that may narrow it, built on first use from every file.
    global_aliases: Option<FxHashMap<u32, Box<[u32]>>>,
}

impl FlowSkip {
    pub fn new() -> FlowSkip {
        FlowSkip { mode: tsrs_ast::flownames::flow_skip_mode(), global_aliases: None }
    }
}

impl Checker {
    /// Whether the walk for `reference` from `flow_node` (`getFlowTypeOfReference`) can only return the declared type.
    /// `start` is the position the walk starts from (the reference's, when `flow_node` is its own), or `u32::MAX`.
    pub(crate) fn flow_walk_skippable(&mut self, reference: P<Node>, declared_type: P<Type>, initial_type: P<Type>, flow_container: Option<P<Node>>, flow_node: P<FlowNode>, start: u32) -> bool {
        if declared_type != initial_type || declared_type == self.auto_type || declared_type == self.auto_array_type {
            return false;
        }
        // The union of the declared type with the unreachable never type (a loop whose entry follows a never-returning
        // call) is the declared type, except for the any and unknown types that unions do not keep.
        if declared_type.flags().intersects(TypeFlags::AnyOrUnknown) && declared_type != self.any_type && declared_type != self.error_type && declared_type != self.unknown_type {
            return false;
        }
        let Some((index, mut graph)) = flow_graph_of(flow_node) else {
            return false;
        };
        // The keys of the reference's path and of each of its prefixes (flownames.rs), root first.
        let mut components = [0u32; MAX_COMPONENTS];
        let mut n = 0;
        let mut current = reference;
        let root = loop {
            match current.kind() {
                Kind::PropertyAccessExpression | Kind::ElementAccessExpression => {
                    if n == MAX_COMPONENTS {
                        return false;
                    }
                    let name = if current.kind() == Kind::PropertyAccessExpression {
                        current.name().unwrap().text()
                    } else {
                        let argument = current.as_element_access_expression().argument_expression();
                        if !matches!(argument.kind(), Kind::StringLiteral | Kind::NoSubstitutionTemplateLiteral) {
                            return false;
                        }
                        argument.text()
                    };
                    components[n] = flow_name_key(name);
                    n += 1;
                    current = current.expression().unwrap();
                }
                Kind::ParenthesizedExpression | Kind::NonNullExpression | Kind::SatisfiesExpression => current = current.expression().unwrap(),
                Kind::Identifier => break flow_name_key(current.text()),
                Kind::ThisKeyword => break flow_name_key("this"),
                Kind::SuperKeyword => break flow_name_key("super"),
                _ => return false,
            }
        };
        let mut prefixes = [0u32; MAX_COMPONENTS + 1];
        prefixes[0] = root;
        for i in 0..n {
            prefixes[i + 1] = flow_path_key(prefixes[i], components[n - 1 - i]);
        }
        let path = &prefixes[..=n];
        // An assignment to the reference itself gives the declared type unless that is a union (assignment narrowing)
        // or a literal type (its base type in compound-like assignments): getTypeAtFlowAssignment.
        let assignments_narrow = declared_type.flags().intersects(
            TypeFlags::Union
                | TypeFlags::EnumLike
                | TypeFlags::StringLiteral
                | TypeFlags::TemplateLiteral
                | TypeFlags::StringMapping
                | TypeFlags::NumberLiteral
                | TypeFlags::BigIntLiteral
                | TypeFlags::BooleanLiteral,
        );
        let aliases = global_aliases_of(&mut self.flow_memo.skip_state, self.files, root);
        let mut nodes = 0u32;
        let mut pos = start;
        loop {
            if graph.len == u16::MAX {
                return false;
            }
            let limit = flow_reach_limit(index.graph_spans(graph), pos);
            if mentions(index.graph_names(graph), path, limit) {
                return false;
            }
            if assignments_narrow && mentions(index.graph_assigned(graph), path, limit) {
                return false;
            }
            if !calls_cannot_narrow(&self.signature_links, self.unknown_signature, index.graph_calls(graph), path, limit) {
                return false;
            }
            if let Some(aliases) = aliases {
                // A global alias (another file's) that may narrow the reference, named in a condition or an assertion.
                let (names, positions) = index.graph_names(graph);
                let calls = index.graph_calls(graph);
                if aliases.iter().any(|&a| {
                    names.binary_search(&a).is_ok_and(|i| positions[i] < limit) || !calls_cannot_narrow(&self.signature_links, self.unknown_signature, calls, &[a], limit)
                }) {
                    return false;
                }
            }
            nodes = nodes.saturating_add(u32::from(graph.nodes));
            if nodes >= NODE_LIMIT {
                return false;
            }
            // getTypeAtFlowNode, FlowFlags::Start: whether the walk continues with the containing function's flow.
            let Some(container) = graph.container else {
                return true;
            };
            if Some(container) == flow_container
                || ast::is_property_access_expression(reference)
                || ast::is_element_access_expression(reference)
                || reference.kind() == Kind::ThisKeyword && !ast::is_arrow_function(container)
            {
                return true;
            }
            let Some((next_index, next_graph)) = container.flow_node().and_then(flow_graph_of) else {
                return false;
            };
            if !std::ptr::eq(next_index, index) {
                return false;
            }
            graph = next_graph;
            pos = if container.pos() >= 0 { container.pos() as u32 } else { u32::MAX };
        }
    }

    /// `TSRS_FLOW_SKIP=shadow`: a walk the index skips returned something other than the declared type.
    #[cold]
    pub(crate) fn flow_skip_shadow_failure(&mut self, reference: P<Node>, declared_type: P<Type>, result_type: P<Type>) {
        let file = ast::get_source_file_of_node(reference);
        let name = file.map_or(String::new(), |f| f.file_name().to_string());
        let (line, character) = file.map_or((0, 0), |f| {
            let (line, character) = tsrs_scanner::get_ecma_line_and_utf16_character_of_position(&*f, tsrs_scanner::skip_trivia(f.text(), reference.pos().max(0)));
            (line, character)
        });
        let text = if reference.pos() >= 0 && file.is_some() { tsrs_scanner::get_text_of_node(reference) } else { String::new() };
        let declared = self.type_to_string(declared_type, None);
        let result = self.type_to_string(result_type, None);
        panic!("flow skip shadow: {name}({}:{}) `{text}`: declared {declared}, walk {result}", line + 1, character + 1);
    }
}

/// The global aliases that may narrow the name `key` (flownames.rs), from every file of the program.
fn global_aliases_of<'a>(skip: &'a mut FlowSkip, files: &[P<ast::SourceFile>], key: u32) -> Option<&'a [u32]> {
    let map = skip.global_aliases.get_or_insert_with(|| build_global_aliases(files));
    if map.is_empty() {
        return None;
    }
    map.get(&key).map(|v| &**v)
}

/// Whether no assertion-candidate call that mentions the reference path, and that a walk may reach (below `limit`),
/// can narrow it: each has its effects signature computed already (by a walk or a reachability check that passed it),
/// and it is none (getTypeAtFlowCall). Peeks only: computing it here could resolve types a walk would not have.
fn calls_cannot_narrow(links: &crate::links::KeyedLinkStore<Node, SignatureLinks>, unknown_signature: P<Signature>, calls: &[FlowCallMention], path: &[u32], limit: u32) -> bool {
    if calls.is_empty() {
        return true;
    }
    let (&full, prefixes) = path.split_last().unwrap();
    let ok = |key: u32| {
        let first = calls.partition_point(|c| c.key < key);
        calls[first..]
            .iter()
            .take_while(|c| c.key == key && (c.call.pos().max(0) as u32) < limit)
            .all(|c| links.try_get(c.call).and_then(|l| l.effects_signature.get()) == Some(unknown_signature))
    };
    ok(full) && prefixes.iter().all(|&p| ok(flow_path_key(p, FLOW_WILDCARD)))
}

/// Whether `keys` (sorted, with the earliest position of each) has the reference path (the last of `path`), or a
/// wildcard below one of its proper prefixes, at a position a walk may reach (below `limit`).
#[inline]
fn mentions((keys, positions): (&[u32], &[u32]), path: &[u32], limit: u32) -> bool {
    if keys.is_empty() {
        return false;
    }
    let (&full, prefixes) = path.split_last().unwrap();
    let hit = |key: u32| keys.binary_search(&key).is_ok_and(|i| positions[i] < limit);
    hit(full) || prefixes.iter().any(|&p| hit(flow_path_key(p, FLOW_WILDCARD)))
}

/// Path key -> the global aliases whose initializers may narrow a path with that prefix: each alias's paths (closed over
/// its own file's aliases by the binder) closed over the other global aliases they name.
fn build_global_aliases(files: &[P<ast::SourceFile>]) -> FxHashMap<u32, Box<[u32]>> {
    let mut direct: Vec<(u32, &'static [u32])> = Vec::new();
    for &file in files {
        // Only scripts export global aliases. A module's index may be gone: a checked leaf file's region, which holds
        // it, is freed while other checkers run (tsrs_compiler fileregions.rs), and leaves are modules.
        if ast::is_external_or_common_js_module(file) {
            continue;
        }
        if let Some(index) = flow_name_index(file.text_index.get()) {
            for a in index.global_aliases {
                direct.push((a.name, &index.names[a.start as usize..(a.start + a.len) as usize]));
            }
        }
    }
    let mut result: FxHashMap<u32, Box<[u32]>> = FxHashMap::default();
    if direct.is_empty() {
        return result;
    }
    direct.sort_by_key(|d| d.0);
    let mut by_alias: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    for &(name, targets) in &direct {
        by_alias.entry(name).or_default().extend_from_slice(targets);
    }
    let mut reverse: FxHashMap<u32, Vec<u32>> = FxHashMap::default();
    let mut seen: FxHashSet<u32> = FxHashSet::default();
    let mut work: Vec<u32> = Vec::new();
    let mut aliases: Vec<u32> = direct.iter().map(|d| d.0).collect();
    aliases.dedup();
    for alias in aliases {
        seen.clear();
        work.clear();
        work.push(alias);
        while let Some(name) = work.pop() {
            if let Some(targets) = by_alias.get(&name) {
                for &t in targets {
                    if seen.insert(t) {
                        work.push(t);
                    }
                }
            }
        }
        let mut targets: Vec<u32> = seen.iter().copied().collect();
        targets.sort_unstable();
        for t in targets {
            reverse.entry(t).or_default().push(alias);
        }
    }
    let mut keys: Vec<u32> = reverse.keys().copied().collect();
    keys.sort_unstable();
    for k in keys {
        let v = reverse.remove(&k).unwrap();
        result.insert(k, v.into_boxed_slice());
    }
    result
}
