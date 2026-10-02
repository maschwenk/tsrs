// Reachability census at the end of a run (alloc-profile build, `TSRS_CENSUS=1`; tsrs_core::alloc_profile::census).
// `TSRS_CENSUS_VERIFY=1` then checks the census against the program: every AST node, every binder symbol in a file's
// tables and every declaration of those symbols is reachable through the program, so none may be reported
// unreachable. It also walks the program precisely for the arena recycling gate (notes/mem-recycle.md): no node,
// parent, JSDoc node, flow node or flow list reachable through the program's fields may be a block the arena freed
// or rewound (the conservative mark can be fooled by stale words in padding; this walk cannot).

use rustc_hash::FxHashSet;
use tsrs_ast::{FlowNode, Kind, Node, Symbol};
use tsrs_compiler::Program;
use tsrs_core::alloc_profile::census;
use tsrs_core::P;

pub(crate) fn run(program: &'static Program, roots: &[usize]) {
    if !census::active() {
        return;
    }
    let mut all = vec![program as *const Program as usize];
    all.extend_from_slice(roots);
    census::run(&all);
    if std::env::var_os("TSRS_CENSUS_VERIFY").is_some_and(|v| v == "1") {
        verify(program);
    }
}

// Language server (`tsrs --lsp`, docs/LSP.md memory regions): at exit, the census marks from the server and its
// session (and the main thread's stack and the data segments). Freed regions (old file versions, checkers, programs)
// were recorded as would-free instead of released, so a strong reference into one is a region-lifetime bug. With
// `TSRS_CENSUS_VERIFY=1`, the programs of the session's current snapshot are also walked precisely.
pub(crate) fn run_lsp(server: &std::sync::Arc<tsrs_lsp::Server>) {
    if !census::active() {
        return;
    }
    // The census's own frames take the stack area the server's work used on this thread; their unset slots would
    // keep its words (pointers into freed regions).
    tsrs_core::census_scrub_stack();
    let session = server.session();
    let snapshot = session.snapshot();
    let programs: Vec<&'static Program> = snapshot.project_collection.projects().iter().filter_map(|p| p.program).collect();
    let roots = [std::sync::Arc::as_ptr(server) as usize, std::sync::Arc::as_ptr(session) as usize, std::sync::Arc::as_ptr(&snapshot) as usize];
    eprintln!("census (lsp): {} programs in the current snapshot", programs.len());
    census::run(&roots);
    if std::env::var_os("TSRS_CENSUS_VERIFY").is_some_and(|v| v == "1") {
        for program in programs {
            verify(program);
        }
    }
}

// Before the server starts: a process-wide lazily initialized static keeps the uninitialized payload bytes of its
// `None` fields, copied from the stack it was built on. The server first touches `EMPTY_COMPILER_OPTIONS` while
// updating the auto-import registry, whose stack then holds pointers into the update's scratch region, freed right
// after; initialized here, while no region exists, it cannot hold such a word (notes/lsp-memfix.md).
pub(crate) fn prepare_lsp() {
    if !census::active() {
        return;
    }
    std::sync::LazyLock::force(&tsrs_core::EMPTY_COMPILER_OPTIONS);
}

#[derive(Default)]
struct Tally {
    checked: u64,
    unreachable: u64,
    unrecorded: u64,
    examples: Vec<String>,
}

impl Tally {
    fn check(&mut self, addr: usize, what: &dyn Fn() -> String) {
        self.checked += 1;
        match census::is_reachable(addr) {
            Some(true) => {}
            Some(false) => {
                self.unreachable += 1;
                if self.examples.len() < 10 {
                    self.examples.push(what());
                }
            }
            None => self.unrecorded += 1,
        }
    }
}

#[derive(Default)]
struct FreedRefs {
    checked: u64,
    freed: u64,
    examples: Vec<String>,
}

impl FreedRefs {
    fn check(&mut self, addr: usize, what: &dyn Fn() -> String) {
        self.checked += 1;
        if census::is_would_free(addr) {
            self.freed += 1;
            if self.examples.len() < 10 {
                self.examples.push(what());
            }
        }
    }
}

fn flow_nodes_of(node: P<Node>) -> Vec<P<FlowNode>> {
    let mut out: Vec<P<FlowNode>> = Vec::new();
    out.extend(node.flow_node());
    if let Some(body) = node.body_data() {
        out.extend(body.end_flow_node());
    }
    match node.kind() {
        Kind::FunctionDeclaration => out.extend(node.as_function_declaration().return_flow_node()),
        Kind::FunctionExpression => out.extend(node.as_function_expression().return_flow_node.get()),
        Kind::Constructor => out.extend(node.as_constructor_declaration().return_flow_node()),
        Kind::ClassStaticBlockDeclaration => out.extend(node.as_class_static_block_declaration().return_flow_node()),
        Kind::CaseClause | Kind::DefaultClause => out.extend(node.as_case_or_default_clause().fallthrough_flow_node()),
        _ => {}
    }
    out
}

fn verify(program: &'static Program) {
    let (mut nodes, mut symbols) = (Tally::default(), Tally::default());
    let mut freed = FreedRefs::default();
    let mut seen_symbols: FxHashSet<P<Symbol>> = FxHashSet::default();
    let mut seen_flow: FxHashSet<P<FlowNode>> = FxHashSet::default();
    for &file in program.get_source_files() {
        let mut work: Vec<P<Node>> = vec![file.as_node()];
        let mut pending_symbols: Vec<P<Symbol>> = Vec::new();
        let mut flow_work: Vec<P<FlowNode>> = Vec::new();
        for list in [file.diagnostics(), file.jsdoc_diagnostics(), file.bind_diagnostics()] {
            for d in list {
                freed.check(d.addr(), &|| format!("diagnostic in {}", file.file_name()));
            }
        }
        while let Some(node) = work.pop() {
            nodes.check(node.addr(), &|| format!("{:?} in {}", node.kind(), file.file_name()));
            freed.check(node.addr(), &|| format!("{:?} at {} in {}", node.kind(), node.pos(), file.file_name()));
            if let Some(parent) = node.parent() {
                freed.check(parent.addr(), &|| format!("parent of {:?} at {} in {}", node.kind(), node.pos(), file.file_name()));
            }
            flow_work.extend(flow_nodes_of(node));
            pending_symbols.extend(node.symbol());
            pending_symbols.extend(node.local_symbol());
            if let Some(locals) = node.locals() {
                pending_symbols.extend(locals.values());
            }
            node.for_each_child(&mut |child| {
                work.push(child);
                false
            });
            work.extend(node.eager_jsdoc(Some(file.get())).iter().copied());
        }
        while let Some(flow) = flow_work.pop() {
            if !seen_flow.insert(flow) {
                continue;
            }
            freed.check(flow.addr(), &|| format!("flow node {:?} in {}", flow.flags.get(), file.file_name()));
            flow_work.extend(flow.antecedent.get());
            if let Some(n) = flow.node.get() {
                freed.check(n.addr(), &|| format!("node of flow node {:?} in {}", flow.flags.get(), file.file_name()));
            }
            let mut list = flow.antecedents.get();
            while let Some(l) = list {
                freed.check(l.addr(), &|| format!("antecedent list of {:?} in {}", flow.flags.get(), file.file_name()));
                flow_work.push(l.flow);
                list = l.next.get();
            }
        }
        while let Some(symbol) = pending_symbols.pop() {
            if !seen_symbols.insert(symbol) {
                continue;
            }
            symbols.check(symbol.addr(), &|| format!("symbol {} in {}", symbol.name(), file.file_name()));
            for table in [symbol.members(), symbol.exports()].into_iter().flatten() {
                pending_symbols.extend(table.values());
            }
            for &decl in symbol.declarations() {
                nodes.check(decl.addr(), &|| format!("declaration {:?} of {}", decl.kind(), symbol.name()));
                freed.check(decl.addr(), &|| format!("declaration {:?} of {}", decl.kind(), symbol.name()));
            }
        }
    }
    eprintln!(
        "census verify (recycling): {} program references checked (nodes, parents, JSDoc, diagnostics, flow nodes, flow lists, declarations), {} to freed or rewound blocks{}",
        freed.checked,
        freed.freed,
        if freed.examples.is_empty() { String::new() } else { format!(" (e.g. {})", freed.examples.join("; ")) }
    );
    for (what, t) in [("AST nodes", &nodes), ("binder symbols", &symbols)] {
        eprintln!(
            "census verify: {what}: {} checked, {} unreachable, {} not recorded{}",
            t.checked,
            t.unreachable,
            t.unrecorded,
            if t.examples.is_empty() { String::new() } else { format!(" (e.g. {})", t.examples.join("; ")) }
        );
    }
}
