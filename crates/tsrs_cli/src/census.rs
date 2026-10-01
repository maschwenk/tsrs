// Reachability census at the end of a run (alloc-profile build, `TSRS_CENSUS=1`; tsrs_core::alloc_profile::census).
// `TSRS_CENSUS_VERIFY=1` then checks the census against the program: every AST node, every binder symbol in a file's
// tables and every declaration of those symbols is reachable through the program, so none may be reported
// unreachable.

use rustc_hash::FxHashSet;
use tsrs_ast::{Node, Symbol};
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

fn verify(program: &'static Program) {
    let (mut nodes, mut symbols) = (Tally::default(), Tally::default());
    let mut seen_symbols: FxHashSet<P<Symbol>> = FxHashSet::default();
    for &file in program.get_source_files() {
        let mut work: Vec<P<Node>> = vec![file.as_node()];
        let mut pending_symbols: Vec<P<Symbol>> = Vec::new();
        while let Some(node) = work.pop() {
            nodes.check(node.addr(), &|| format!("{:?} in {}", node.kind(), file.file_name()));
            pending_symbols.extend(node.symbol());
            pending_symbols.extend(node.local_symbol());
            if let Some(locals) = node.locals() {
                pending_symbols.extend(locals.values());
            }
            node.for_each_child(&mut |child| {
                work.push(child);
                false
            });
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
            }
        }
    }
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
