// Not in Go: builds a file's `FlowNameIndex` while the binder makes its flow nodes (tsrs_ast flownames.rs,
// notes/perf-flow-skip-unnarrowed.md).
//
// A reference path is an identifier, `this` or `super` followed by property names (`a.b["c"]`; parentheses, `!`,
// `satisfies`, assignments and comma expressions are looked through, as `isMatchingReference` does). Per graph,
// conservatively (a key too many, or a position too early, only makes a walk run):
// - every prefix of every reference path that `narrowType` can reach in a condition, a switch (its expression and case
//   expressions) or the expression of a `for...in` statement, and for `"k" in x` and `x.hasOwnProperty("k")` also
//   `x.k` (`collect_paths`); closed over aliases: when such a path names a const variable in scope (declared in the
//   graph or an enclosing one) whose initializer the checker may inline as a condition (`const ok = typeof x ===
//   "string"`) or take as a discriminant's object (`const k = x.kind`, `const { kind } = x`), that initializer's paths
//   too, transitively; variables of global scripts that other files can see are exported for the checker to close
//   over;
// - every assignment target and declared name, exactly (`assigned`);
// - per assertion-candidate call (an expression statement calling a dotted name), the same paths of the callee's
//   object and of the arguments, closed over aliases, with the call (`calls`);
// - the earliest position of a node that mentions each key, and the spans of loops and immediately invoked functions,
//   where a walk can reach nodes that start later than where it starts.
// An element access whose argument is not a string literal ends a path with `FLOW_WILDCARD`. Array mutations are not
// recorded: they narrow only auto-typed references, which are never skipped.

use rustc_hash::FxHashSet;
use tsrs_ast as ast;
use tsrs_ast::flownames::{
    flow_key_is_name, flow_name_key, flow_path_key, register_flow_name_index, FlowCallMention, FlowGlobalAlias, FlowGraphNames, FlowNameIndex, FLOW_GRAPH_MAX,
    FLOW_WILDCARD,
};
use tsrs_ast::{Kind, Node, SourceFile};
use tsrs_core::P;

struct GraphBuild {
    nodes: u32,
    container: Option<P<Node>>,
    /// The graph whose container encloses this one's (0: none).
    parent: u32,
    /// Ranges in `FlowNames::done_keys` (mentioned, assigned) and `FlowNames::done_calls`, set when the graph ends.
    mentioned: (u32, u32),
    assigned: (u32, u32),
    calls: (u32, u32),
}

const MENTIONED: u32 = 0;
const ASSIGNED: u32 = 1;

pub(crate) struct FlowNames {
    enabled: bool,
    current: u32,
    graphs: Vec<GraphBuild>,
    /// (MENTIONED or ASSIGNED, key, position) for every key recorded in the graphs not ended yet; each graph's are
    /// contiguous (the graphs it contains end before it records more).
    entries: Vec<(u32, u32, u32)>,
    /// (key, call) for every path of an assertion-candidate call in those graphs, each call's together.
    call_entries: Vec<(u32, P<Node>)>,
    /// The graphs not ended yet, with where their entries and call entries start.
    open: Vec<(u32, u32, u32)>,
    /// Per ended graph, its keys (sorted, the earliest position of each) and its calls' keys (each call's sorted).
    done_keys: Vec<(u32, u32)>,
    done_calls: Vec<(u32, P<Node>)>,
    /// (graph, start, end) of every loop and immediately invoked function.
    spans: Vec<(u32, u32, u32)>,
    /// (name, initializer, visible to other files, graph) of every variable whose initializer may alias paths; the
    /// initializer is walked only if the name is looked up while closing over aliases.
    aliases: Vec<(u32, P<Node>, bool, u32)>,
    last_switch: Option<P<Node>>,
    last_condition: Option<P<Node>>,
    stack: Vec<P<Node>>,
    components: Vec<Option<u32>>,
    scratch: Vec<u32>,
}

fn pos_of(node: P<Node>) -> u32 {
    node.pos().max(0) as u32
}

impl FlowNames {
    pub(crate) fn new(enabled: bool) -> FlowNames {
        FlowNames {
            enabled,
            current: 0,
            graphs: Vec::new(),
            entries: Vec::new(),
            call_entries: Vec::new(),
            open: Vec::new(),
            done_keys: Vec::new(),
            done_calls: Vec::new(),
            spans: Vec::new(),
            aliases: Vec::new(),
            last_switch: None,
            last_condition: None,
            stack: Vec::new(),
            components: Vec::new(),
            scratch: Vec::new(),
        }
    }

    /// The graph of the flow nodes made now (0: none).
    #[inline]
    pub(crate) fn graph(&self) -> u32 {
        self.current
    }

    #[inline]
    pub(crate) fn count_node(&mut self) {
        if self.current != 0 {
            let g = &mut self.graphs[self.current as usize - 1];
            g.nodes = g.nodes.saturating_add(1);
        }
    }

    /// A new graph starts (its `Start` node is made next; `container` is what that node continues to). Returns the
    /// graph to restore with `exit_graph`.
    pub(crate) fn enter_graph(&mut self, container: Option<P<Node>>) -> u32 {
        let saved = self.current;
        if !self.enabled {
            return saved;
        }
        if self.graphs.len() as u32 >= FLOW_GRAPH_MAX {
            self.current = 0;
            return saved;
        }
        self.graphs.push(GraphBuild { nodes: 0, container, parent: saved, mentioned: (0, 0), assigned: (0, 0), calls: (0, 0) });
        self.current = self.graphs.len() as u32;
        self.open.push((self.current, self.entries.len() as u32, self.call_entries.len() as u32));
        saved
    }

    pub(crate) fn exit_graph(&mut self, saved: u32) {
        if self.current != 0 && self.open.last().is_some_and(|o| o.0 == self.current) {
            let (graph, entries_start, calls_start) = self.open.pop().unwrap();
            self.end_graph(graph, entries_start as usize, calls_start as usize);
        }
        self.current = saved;
    }

    /// Sorts the ended graph's keys (keeping the earliest position of each) and each of its calls' keys.
    fn end_graph(&mut self, graph: u32, entries_start: usize, calls_start: usize) {
        let entries = &mut self.entries[entries_start..];
        entries.sort_unstable();
        let base = self.done_keys.len();
        let mut split = None;
        for &(kind, key, pos) in entries.iter() {
            if kind == ASSIGNED && split.is_none() {
                split = Some(self.done_keys.len());
            }
            let section = if kind == ASSIGNED { split.unwrap() } else { base };
            if self.done_keys.len() > section && self.done_keys.last().unwrap().0 == key {
                continue;
            }
            self.done_keys.push((key, pos));
        }
        let split = split.unwrap_or(self.done_keys.len());
        let ranges = [(base as u32, (split - base) as u32), (split as u32, (self.done_keys.len() - split) as u32)];
        self.entries.truncate(entries_start);
        let calls_begin = self.done_calls.len() as u32;
        let mut j = calls_start;
        while j < self.call_entries.len() {
            let call = self.call_entries[j].1;
            let group = self.done_calls.len();
            while j < self.call_entries.len() && self.call_entries[j].1 == call {
                self.done_calls.push(self.call_entries[j]);
                j += 1;
            }
            self.done_calls[group..].sort_unstable_by_key(|c| c.0);
            let mut w = group;
            for r in group..self.done_calls.len() {
                if r == group || self.done_calls[r].0 != self.done_calls[w - 1].0 {
                    self.done_calls[w] = self.done_calls[r];
                    w += 1;
                }
            }
            self.done_calls.truncate(w);
        }
        self.call_entries.truncate(calls_start);
        let g = &mut self.graphs[graph as usize - 1];
        g.mentioned = ranges[0];
        g.assigned = ranges[1];
        g.calls = (calls_begin, self.done_calls.len() as u32 - calls_begin);
    }

    /// A loop statement, or the call of an immediately invoked function (whose arguments are bound before its body).
    pub(crate) fn record_span(&mut self, node: P<Node>) {
        if self.current != 0 {
            self.spans.push((self.current, pos_of(node), node.end().max(0) as u32));
        }
    }

    /// A condition (once: its true and false flow nodes are made one after the other).
    pub(crate) fn record_condition(&mut self, node: P<Node>) {
        if self.current == 0 || self.last_condition == Some(node) {
            return;
        }
        self.last_condition = Some(node);
        let start = self.scratch.len();
        collect_paths(node, &mut self.stack, &mut self.components, &mut self.scratch);
        self.flush(start, MENTIONED, pos_of(node));
    }

    /// An assertion-candidate call: its paths are kept with the call, which narrows only if its effects signature
    /// asserts (getTypeAtFlowCall).
    pub(crate) fn record_call(&mut self, node: P<Node>) {
        if self.current == 0 {
            return;
        }
        let start = self.scratch.len();
        collect_paths(node, &mut self.stack, &mut self.components, &mut self.scratch);
        for &key in &self.scratch[start..] {
            self.call_entries.push((key, node));
        }
        self.scratch.truncate(start);
    }

    /// An assignment's target.
    pub(crate) fn record_assignment(&mut self, node: P<Node>) {
        if self.current == 0 {
            return;
        }
        let start = self.scratch.len();
        match node.kind() {
            Kind::VariableDeclaration | Kind::BindingElement => {
                if let Some(name) = node.name() {
                    collect_names(name, &mut self.stack, &mut self.scratch);
                }
                self.flush(start, ASSIGNED, pos_of(node));
                // `for (const k in obj)` narrows `obj` to non-null (getTypeAtFlowAssignment).
                if let Some(statement) = node.parent().and_then(|p| p.parent()) {
                    if statement.kind() == Kind::ForInStatement {
                        if let Some(expression) = statement.expression() {
                            collect_paths(expression, &mut self.stack, &mut self.components, &mut self.scratch);
                            self.flush(start, MENTIONED, pos_of(node));
                        }
                    }
                }
            }
            _ => {
                target_path(node, &mut self.scratch);
                self.flush(start, ASSIGNED, pos_of(node));
            }
        }
    }

    /// A switch statement's expression and case expressions (once per statement).
    pub(crate) fn record_switch(&mut self, switch_statement: P<Node>) {
        if self.current == 0 || self.last_switch == Some(switch_statement) {
            return;
        }
        self.last_switch = Some(switch_statement);
        let start = self.scratch.len();
        if let Some(expression) = switch_statement.expression() {
            collect_paths(expression, &mut self.stack, &mut self.components, &mut self.scratch);
        }
        for &clause in switch_statement.as_switch_statement().case_block().as_case_block().clauses().nodes() {
            if clause.kind() == Kind::CaseClause {
                if let Some(expression) = clause.expression() {
                    collect_paths(expression, &mut self.stack, &mut self.components, &mut self.scratch);
                }
            }
        }
        self.flush(start, MENTIONED, pos_of(switch_statement));
    }

    fn flush(&mut self, start: usize, kind: u32, pos: u32) {
        for &key in &self.scratch[start..] {
            self.entries.push((kind, key, pos));
        }
        self.scratch.truncate(start);
    }

    /// A variable declaration: when the checker may inline its initializer as a condition (`narrowType`: a const
    /// variable without a type annotation) or take it as a discriminant's object
    /// (`getCandidateDiscriminantPropertyAccess`: such a variable initialized with an access expression, or a name bound
    /// directly in its binding pattern, initialized with an identifier or access expression), the initializer's paths
    /// are what a condition on the variable can narrow.
    pub(crate) fn record_alias(&mut self, declaration: P<Node>, global: bool) {
        if !self.enabled || declaration.type_node().is_some() {
            return;
        }
        let (Some(initializer), Some(name)) = (declaration.initializer(), declaration.name()) else {
            return;
        };
        if !ast::get_combined_node_flags(declaration).intersects(tsrs_ast::NodeFlags::Constant) {
            return;
        }
        let inner = ast::skip_parentheses(initializer);
        if name.kind() == Kind::Identifier {
            if may_alias(inner) {
                self.aliases.push((flow_name_key(name.text()), initializer, global, self.current));
            }
        } else if matches!(name.kind(), Kind::ObjectBindingPattern | Kind::ArrayBindingPattern)
            && matches!(inner.kind(), Kind::Identifier | Kind::PropertyAccessExpression | Kind::ElementAccessExpression)
        {
            for &element in name.elements() {
                if let Some(bound) = (element.kind() == Kind::BindingElement).then(|| element.name()).flatten().filter(|n| n.kind() == Kind::Identifier) {
                    self.aliases.push((flow_name_key(bound.text()), initializer, global, self.current));
                }
            }
        }
    }

    /// Closes the recorded paths over the file's aliases and registers the index for the file's text.
    pub(crate) fn finish(&mut self, file: P<SourceFile>) {
        if !self.enabled || self.graphs.is_empty() {
            return;
        }
        let mut alias_map = AliasTable::new(
            std::mem::take(&mut self.aliases),
            self.graphs.iter().map(|g| g.parent).collect(),
            std::mem::take(&mut self.stack),
            std::mem::take(&mut self.components),
        );
        let graph_count = self.graphs.len() + 1;
        let mut closure = Closure { added: Vec::new() };
        let mut keys: Vec<(u32, u32)> = Vec::new();

        // Spans: per graph, the outermost ones.
        self.spans.sort_unstable();
        let mut spans: Vec<(u32, u32)> = Vec::new();
        let mut span_ranges: Vec<(u32, u32)> = vec![(0, 0); graph_count];
        for &(graph, start, end) in &self.spans {
            let range = &mut span_ranges[graph as usize];
            if range.1 == 0 {
                range.0 = spans.len() as u32;
            } else if let Some(last) = spans.last_mut() {
                if start < last.1 {
                    last.1 = last.1.max(end);
                    continue;
                }
            }
            spans.push((start, end));
            range.1 = spans.len() as u32 - range.0;
        }
        let narrow = |n: u32| u16::try_from(n).ok().filter(|&n| n != u16::MAX);

        let mut names: Vec<u32> = Vec::with_capacity(self.done_keys.len());
        let mut positions: Vec<u32> = Vec::with_capacity(self.done_keys.len());
        let mut calls: Vec<FlowCallMention> = Vec::with_capacity(self.done_calls.len());
        let mut graphs: Vec<FlowGraphNames> = Vec::with_capacity(graph_count);
        graphs.push(FlowGraphNames { start: 0, calls_start: 0, container: None, len: u16::MAX, assigned: 0, calls: 0, spans: 0, nodes: 0 });
        for (g, build) in self.graphs.iter().enumerate() {
            let graph = g as u32 + 1;
            let start = names.len() as u32;
            keys.clear();
            keys.extend_from_slice(&self.done_keys[build.mentioned.0 as usize..(build.mentioned.0 + build.mentioned.1) as usize]);
            closure.close(&mut keys, &mut alias_map, graph);
            for &(key, pos) in &keys {
                names.push(key);
                positions.push(pos);
            }
            let len = names.len() as u32 - start;
            for &(key, pos) in &self.done_keys[build.assigned.0 as usize..(build.assigned.0 + build.assigned.1) as usize] {
                names.push(key);
                positions.push(pos);
            }
            let assigned = build.assigned.1;
            let calls_start = calls.len() as u32;
            let mut j = build.calls.0 as usize;
            let calls_end = (build.calls.0 + build.calls.1) as usize;
            while j < calls_end {
                let call = self.done_calls[j].1;
                keys.clear();
                while j < calls_end && self.done_calls[j].1 == call {
                    keys.push((self.done_calls[j].0, 0));
                    j += 1;
                }
                closure.close(&mut keys, &mut alias_map, graph);
                calls.extend(keys.iter().map(|&(key, _)| FlowCallMention { key, call }));
            }
            calls[calls_start as usize..].sort_unstable_by_key(|c| (c.key, pos_of(c.call), c.call.to_bits()));
            let calls_len = calls.len() as u32 - calls_start;
            let (spans_start, spans_len) = span_ranges[graph as usize];
            for &(span_start, span_end) in &spans[spans_start as usize..(spans_start + spans_len) as usize] {
                names.push(span_start);
                names.push(span_end);
                positions.push(0);
                positions.push(0);
            }
            let counts = (narrow(len), narrow(assigned), narrow(calls_len), narrow(spans_len));
            let (len, assigned, calls_count, spans_count) = match counts {
                (Some(len), Some(assigned), Some(calls), Some(spans)) => (len, assigned, calls, spans),
                _ => (u16::MAX, 0, 0, 0),
            };
            graphs.push(FlowGraphNames {
                start,
                calls_start,
                container: build.container,
                len,
                assigned,
                calls: calls_count,
                spans: spans_count,
                nodes: u16::try_from(build.nodes).unwrap_or(u16::MAX),
            });
        }

        let mut global_aliases = Vec::new();
        if !ast::is_external_or_common_js_module(file) {
            let mut seen: FxHashSet<u32> = FxHashSet::default();
            for a in 0..alias_map.entries.len() {
                let (name, _, global, graph) = alias_map.entries[a];
                if !global || !seen.insert(name) {
                    continue;
                }
                keys.clear();
                keys.push((name, 0));
                closure.close(&mut keys, &mut alias_map, graph);
                keys.retain(|k| k.0 != name);
                let start = names.len() as u32;
                for &(key, _) in &keys {
                    names.push(key);
                    positions.push(0);
                }
                global_aliases.push(FlowGlobalAlias { name, start, len: names.len() as u32 - start });
            }
        }
        let index = P::new(FlowNameIndex {
            graphs: tsrs_core::alloc_vec(graphs),
            names: tsrs_core::alloc_vec(names),
            positions: tsrs_core::alloc_vec(positions),
            calls: tsrs_core::alloc_vec(calls),
            global_aliases: tsrs_core::alloc_vec(global_aliases),
        });
        register_flow_name_index(file.text_index.get(), index);
        self.entries = Vec::new();
        self.call_entries = Vec::new();
        self.done_keys = Vec::new();
        self.done_calls = Vec::new();
        self.spans = Vec::new();
    }
}

/// The file's alias declarations, sorted by name and graph, with the paths each may narrow once looked up.
struct AliasTable {
    entries: Vec<(u32, P<Node>, bool, u32)>,
    /// The parent of each graph (index graph - 1).
    parents: Vec<u32>,
    /// Per entry, the range in `closed` of what it may narrow: its initializer's paths and, transitively, those of the
    /// variables in scope where it is declared that those name.
    ranges: Vec<Option<(u32, u32)>>,
    closed: Vec<u32>,
    names: FxHashSet<u32>,
    /// A filter over `names`: bit `(key >> 1) % 4096`.
    bits: Box<[u64; 64]>,
    stack: Vec<P<Node>>,
    components: Vec<Option<u32>>,
    visited: Vec<u32>,
    found: Vec<u32>,
}

impl AliasTable {
    fn new(mut entries: Vec<(u32, P<Node>, bool, u32)>, parents: Vec<u32>, stack: Vec<P<Node>>, components: Vec<Option<u32>>) -> AliasTable {
        // Sorted by name and graph; stable, so each one's declarations stay in source order.
        entries.sort_by_key(|a| (a.0, a.3));
        let mut bits = Box::new([0u64; 64]);
        for e in &entries {
            let b = (e.0 >> 1) as usize & 4095;
            bits[b >> 6] |= 1 << (b & 63);
        }
        AliasTable {
            ranges: vec![None; entries.len()],
            names: entries.iter().map(|e| e.0).collect(),
            bits,
            entries,
            parents,
            closed: Vec::new(),
            stack,
            components,
            visited: Vec::new(),
            found: Vec::new(),
        }
    }

    #[inline]
    fn has(&self, key: u32) -> bool {
        let b = (key >> 1) as usize & 4095;
        flow_key_is_name(key) && self.bits[b >> 6] & (1 << (b & 63)) != 0 && self.names.contains(&key)
    }

    /// Appends to `found` each entry named `name` in scope in `graph`: declared in it or in an enclosing graph (or where
    /// no graph was recorded).
    fn for_each_in_scope(&mut self, name: u32, graph: u32) {
        let mut g = graph;
        loop {
            let mut i = self.entries.partition_point(|e| (e.0, e.3) < (name, g));
            while i < self.entries.len() && self.entries[i].0 == name && self.entries[i].3 == g {
                self.found.push(i as u32);
                i += 1;
            }
            if g == 0 {
                break;
            }
            g = self.parents[g as usize - 1];
        }
    }

    /// What entry `i` may narrow (`closed[start..start + len]`).
    fn closure(&mut self, i: usize) -> (u32, u32) {
        if let Some(range) = self.ranges[i] {
            return range;
        }
        let graph = self.entries[i].3;
        let start = self.closed.len();
        self.visited.clear();
        self.visited.push(i as u32);
        let mut next = 0;
        while next < self.visited.len() {
            let e = self.visited[next] as usize;
            next += 1;
            let first = self.closed.len();
            collect_paths(self.entries[e].1, &mut self.stack, &mut self.components, &mut self.closed);
            for k in first..self.closed.len() {
                let key = self.closed[k];
                if self.has(key) {
                    self.found.clear();
                    self.for_each_in_scope(key, graph);
                    for f in 0..self.found.len() {
                        let j = self.found[f];
                        if !self.visited.contains(&j) {
                            self.visited.push(j);
                        }
                    }
                }
            }
        }
        self.closed[start..].sort_unstable();
        let mut w = start;
        for r in start..self.closed.len() {
            if r == start || self.closed[r] != self.closed[w - 1] {
                self.closed[w] = self.closed[r];
                w += 1;
            }
        }
        self.closed.truncate(w);
        let range = (start as u32, (w - start) as u32);
        self.ranges[i] = Some(range);
        range
    }
}

/// Scratch for closing a key set over aliases.
struct Closure {
    added: Vec<(u32, u32)>,
}

impl Closure {
    /// Adds to `keys` (sorted by key, one entry per key with its earliest position) what the variables its names refer
    /// to in `graph` may narrow, at the position of the name; keeps it sorted that way.
    fn close(&mut self, keys: &mut Vec<(u32, u32)>, table: &mut AliasTable, graph: u32) {
        if table.entries.is_empty() {
            return;
        }
        self.added.clear();
        for k in 0..keys.len() {
            let (key, pos) = keys[k];
            if !table.has(key) {
                continue;
            }
            table.found.clear();
            table.for_each_in_scope(key, graph);
            let found = std::mem::take(&mut table.found);
            for &j in &found {
                let (start, len) = table.closure(j as usize);
                self.added.extend(table.closed[start as usize..(start + len) as usize].iter().map(|&t| (t, pos)));
            }
            table.found = found;
        }
        if self.added.is_empty() {
            return;
        }
        keys.append(&mut self.added);
        keys.sort_unstable();
        keys.dedup_by_key(|k| k.0);
    }
}

/// Whether `narrowType` (inlining a const variable's initializer) or `getCandidateDiscriminantPropertyAccess` (taking it
/// as a discriminant's object) can see anything through an initializer of this kind (parentheses skipped). False only
/// for kinds both pass over.
fn may_alias(initializer: P<Node>) -> bool {
    !matches!(
        initializer.kind(),
        Kind::AwaitExpression
            | Kind::NewExpression
            | Kind::StringLiteral
            | Kind::NumericLiteral
            | Kind::BigIntLiteral
            | Kind::NoSubstitutionTemplateLiteral
            | Kind::TemplateExpression
            | Kind::RegularExpressionLiteral
            | Kind::TrueKeyword
            | Kind::FalseKeyword
            | Kind::NullKeyword
            | Kind::ObjectLiteralExpression
            | Kind::ArrayLiteralExpression
            | Kind::FunctionExpression
            | Kind::ArrowFunction
            | Kind::ClassExpression
            | Kind::ConditionalExpression
            | Kind::AsExpression
            | Kind::TypeAssertionExpression
            | Kind::TaggedTemplateExpression
            | Kind::YieldExpression
            | Kind::VoidExpression
            | Kind::DeleteExpression
            | Kind::PostfixUnaryExpression
            | Kind::TypeOfExpression
            | Kind::JsxElement
            | Kind::JsxSelfClosingElement
            | Kind::JsxFragment
    )
}

fn is_body_or_type(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::FunctionExpression
            | Kind::ArrowFunction
            | Kind::ClassExpression
            | Kind::FunctionDeclaration
            | Kind::ClassDeclaration
            | Kind::MethodDeclaration
            | Kind::GetAccessor
            | Kind::SetAccessor
            | Kind::Constructor
    ) || kind != Kind::ExpressionWithTypeArguments && ast::is_type_node_kind(kind)
}

/// The key of every identifier in a binding name.
fn collect_names(root: P<Node>, stack: &mut Vec<P<Node>>, out: &mut Vec<u32>) {
    let base = stack.len();
    stack.push(root);
    while stack.len() > base {
        let node = stack.pop().unwrap();
        match node.kind() {
            Kind::Identifier => out.push(flow_name_key(node.text())),
            kind if is_body_or_type(kind) => {}
            _ => {
                node.for_each_child(&mut |child| {
                    stack.push(child);
                    false
                });
            }
        }
    }
}

/// A name of a path component: a property name, or an element access's string literal argument; None for any other
/// element access argument (the checker may resolve it to a name: `getAccessedPropertyName`).
fn component_name(access: P<Node>) -> Option<u32> {
    if access.kind() == Kind::PropertyAccessExpression {
        return Some(flow_name_key(access.name().unwrap().text()));
    }
    let argument = access.as_element_access_expression().argument_expression();
    matches!(argument.kind(), Kind::StringLiteral | Kind::NoSubstitutionTemplateLiteral).then(|| flow_name_key(argument.text()))
}

/// Pushes the keys of every prefix of every reference path that `narrowType` can reach in `root`: it looks through
/// parentheses, `!`, `satisfies`, unary and binary operators and `typeof`, matches reference paths (and, for
/// `"k" in x` and `x.hasOwnProperty("k")`, `x.k`), and in a call the object of the callee and the arguments
/// (`hasMatchingArgument`, `getTypePredicateArgument`, `narrowTypeByAssertion`). Nothing else in an expression (literals,
/// object and array literals, `new`, `await`, conditional expressions, `as`, functions, classes, JSX) is matched against
/// a reference or narrowed through.
fn collect_paths(root: P<Node>, stack: &mut Vec<P<Node>>, components: &mut Vec<Option<u32>>, out: &mut Vec<u32>) {
    let base = stack.len();
    stack.push(root);
    while stack.len() > base {
        let node = stack.pop().unwrap();
        match node.kind() {
            Kind::Identifier | Kind::ThisKeyword | Kind::SuperKeyword | Kind::PropertyAccessExpression | Kind::ElementAccessExpression => {
                // The longest path starting here; what is not part of it is traversed on its own.
                components.clear();
                let mut current = node;
                let root_key = loop {
                    match current.kind() {
                        Kind::PropertyAccessExpression | Kind::ElementAccessExpression => {
                            components.push(component_name(current));
                            if current.kind() == Kind::ElementAccessExpression {
                                stack.push(current.as_element_access_expression().argument_expression());
                            }
                            current = current.expression().unwrap();
                        }
                        Kind::ParenthesizedExpression | Kind::NonNullExpression | Kind::SatisfiesExpression => current = current.expression().unwrap(),
                        Kind::BinaryExpression => {
                            // `isMatchingReference` matches `(x = y).z` like `x.z` and `(y, x).z` like `x.z`.
                            let binary = current.as_binary_expression();
                            let operator = binary.operator_token().kind();
                            if operator == Kind::CommaToken {
                                stack.push(binary.left());
                                current = binary.right();
                            } else if ast::is_assignment_operator(operator) {
                                stack.push(binary.right());
                                current = binary.left();
                            } else {
                                stack.push(current);
                                break None;
                            }
                        }
                        Kind::Identifier => break Some(flow_name_key(current.text())),
                        Kind::ThisKeyword => break Some(flow_name_key("this")),
                        Kind::SuperKeyword => break Some(flow_name_key("super")),
                        _ => {
                            stack.push(current);
                            break None;
                        }
                    }
                };
                let Some(mut key) = root_key else {
                    continue;
                };
                out.push(key);
                for &component in components.iter().rev() {
                    match component {
                        Some(name) => {
                            key = flow_path_key(key, name);
                            out.push(key);
                        }
                        None => {
                            out.push(flow_path_key(key, FLOW_WILDCARD));
                            break;
                        }
                    }
                }
            }
            Kind::ParenthesizedExpression | Kind::NonNullExpression | Kind::SatisfiesExpression | Kind::TypeOfExpression => {
                stack.push(node.expression().unwrap());
            }
            Kind::PrefixUnaryExpression => stack.push(node.as_prefix_unary_expression().operand()),
            Kind::BinaryExpression => {
                let binary = node.as_binary_expression();
                if binary.operator_token().kind() == Kind::InKeyword {
                    let left = ast::skip_parentheses(binary.left());
                    let name = matches!(left.kind(), Kind::StringLiteral | Kind::NoSubstitutionTemplateLiteral).then(|| flow_name_key(left.text()));
                    if let Some(object) = exact_path_key(binary.right()) {
                        out.push(flow_path_key(object, name.unwrap_or(FLOW_WILDCARD)));
                    }
                }
                stack.push(binary.left());
                stack.push(binary.right());
            }
            Kind::CallExpression => {
                let callee = node.expression().unwrap();
                let arguments = node.arguments();
                if matches!(callee.kind(), Kind::PropertyAccessExpression | Kind::ElementAccessExpression) {
                    if callee.kind() == Kind::PropertyAccessExpression && callee.name().unwrap().text() == "hasOwnProperty" && arguments.len() == 1 {
                        let argument = arguments[0];
                        if matches!(argument.kind(), Kind::StringLiteral | Kind::NoSubstitutionTemplateLiteral) {
                            if let Some(object) = exact_path_key(callee.expression().unwrap()) {
                                out.push(flow_path_key(object, flow_name_key(argument.text())));
                            }
                        }
                    }
                    stack.push(callee.expression().unwrap());
                }
                stack.extend(arguments.iter().copied());
            }
            _ => {}
        }
    }
}

enum PathKey {
    /// Not a reference path: no reference matches it.
    None,
    Exact(u32),
    /// The path up to an element access whose name the checker may resolve, with `FLOW_WILDCARD`.
    Wildcard(u32),
}

/// The key of the reference path `node` is matched as (`getReferenceCandidate` and `isMatchingReference` look through
/// parentheses, `!`, `satisfies`, assignments and comma expressions).
fn path_key(node: P<Node>) -> PathKey {
    // Longer paths than this are never skipped (flowskip.rs MAX_COMPONENTS); one of them as a target is kept as a
    // wildcard below its root.
    const MAX: usize = 16;
    let mut components = [None; MAX];
    let mut n = 0;
    let mut current = node;
    let mut key = loop {
        match current.kind() {
            Kind::PropertyAccessExpression | Kind::ElementAccessExpression => {
                if n == MAX {
                    return root_key(current).map_or(PathKey::None, |root| PathKey::Wildcard(flow_path_key(root, FLOW_WILDCARD)));
                }
                components[n] = component_name(current);
                n += 1;
                current = current.expression().unwrap();
            }
            Kind::ParenthesizedExpression | Kind::NonNullExpression | Kind::SatisfiesExpression => current = current.expression().unwrap(),
            Kind::BinaryExpression if current.as_binary_expression().operator_token().kind() == Kind::CommaToken => {
                current = current.as_binary_expression().right();
            }
            Kind::BinaryExpression if ast::is_assignment_operator(current.as_binary_expression().operator_token().kind()) => {
                current = current.as_binary_expression().left();
            }
            Kind::Identifier => break flow_name_key(current.text()),
            Kind::ThisKeyword => break flow_name_key("this"),
            Kind::SuperKeyword => break flow_name_key("super"),
            _ => return PathKey::None,
        }
    };
    for &component in components[..n].iter().rev() {
        match component {
            Some(name) => key = flow_path_key(key, name),
            None => return PathKey::Wildcard(flow_path_key(key, FLOW_WILDCARD)),
        }
    }
    PathKey::Exact(key)
}

/// The key of the root of the reference path `node`, if it is one.
fn root_key(node: P<Node>) -> Option<u32> {
    let mut current = node;
    loop {
        match current.kind() {
            Kind::PropertyAccessExpression | Kind::ElementAccessExpression | Kind::ParenthesizedExpression | Kind::NonNullExpression | Kind::SatisfiesExpression => {
                current = current.expression().unwrap();
            }
            Kind::BinaryExpression if current.as_binary_expression().operator_token().kind() == Kind::CommaToken => {
                current = current.as_binary_expression().right();
            }
            Kind::BinaryExpression if ast::is_assignment_operator(current.as_binary_expression().operator_token().kind()) => {
                current = current.as_binary_expression().left();
            }
            Kind::Identifier => return Some(flow_name_key(current.text())),
            Kind::ThisKeyword => return Some(flow_name_key("this")),
            Kind::SuperKeyword => return Some(flow_name_key("super")),
            _ => return None,
        }
    }
}

/// The key of a reference path without wildcards (one ending in a wildcard is already covered by its prefixes).
fn exact_path_key(node: P<Node>) -> Option<u32> {
    match path_key(node) {
        PathKey::Exact(key) => Some(key),
        _ => None,
    }
}

/// The key of an assignment target's path, or of its prefix with `FLOW_WILDCARD`.
fn target_path(node: P<Node>, out: &mut Vec<u32>) {
    match path_key(node) {
        PathKey::Exact(key) | PathKey::Wildcard(key) => out.push(key),
        PathKey::None => {}
    }
}
