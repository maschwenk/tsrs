// Go `getSourceFileSymbolIndex` (tsc/internal/api/session.go): the file-owned (non-transient) binder
// symbols reachable from a source file's nodes, locals, global exports and pattern ambient modules,
// plus their parents, export symbols, members and exports. Used to resolve file-owned symbol
// references (`SymbolOwnerKindFile`) by id.
//
// Go memoizes the full map on the source file; the port has no per-file data slot, so it searches the
// same set and stops at the first match (no allocation beyond the visited set).

use rustc_hash::FxHashSet;
use tsrs_ast::{Node, SourceFile, Symbol, SymbolFlags};
use tsrs_core::P;

struct Search {
    file: P<SourceFile>,
    id: u64,
    seen: FxHashSet<u64>,
    found: Option<P<Symbol>>,
}

impl Search {
    // Returns true once found (stop the walk).
    fn add(&mut self, symbol: Option<P<Symbol>>) -> bool {
        let Some(symbol) = symbol else { return false };
        if self.found.is_some() {
            return true;
        }
        if symbol.flags().intersects(SymbolFlags::Transient) {
            return false;
        }
        let id = tsrs_ast::get_symbol_id(symbol).0;
        if !self.seen.insert(id) {
            return false;
        }
        if tsrs_ast::get_source_file_of_symbol(symbol) != Some(self.file) {
            // Go asserts this; a foreign symbol is never part of this file's index.
            return false;
        }
        if id == self.id {
            self.found = Some(symbol);
            return true;
        }
        if self.add(symbol.parent()) || self.add(symbol.export_symbol()) {
            return true;
        }
        for table in [symbol.members(), symbol.exports()].into_iter().flatten() {
            for child in table.values() {
                if self.add(Some(child)) {
                    return true;
                }
            }
        }
        false
    }

    fn visit(&mut self, node: P<Node>) -> bool {
        if self.add(node.symbol()) || self.add(node.local_symbol()) {
            return true;
        }
        if let Some(locals) = node.locals() {
            for symbol in locals.values() {
                if self.add(Some(symbol)) {
                    return true;
                }
            }
        }
        self.visit_children(node)
    }

    fn visit_children(&mut self, node: P<Node>) -> bool {
        let mut stop = false;
        node.for_each_child(&mut |child: P<Node>| {
            stop = self.visit(child);
            stop
        });
        if stop {
            return true;
        }
        for jsdoc in node.jsdoc(Some(self.file.get())) {
            if self.visit(*jsdoc) {
                return true;
            }
        }
        false
    }
}

/// The file-owned symbol with `id` in `file`, if any.
pub(crate) fn source_file_symbol(file: P<SourceFile>, id: u64) -> Option<P<Symbol>> {
    let mut search = Search { file, id, seen: FxHashSet::default(), found: None };
    let root = file.as_node();
    if search.visit(root) {
        return search.found;
    }
    if let Some(globals) = file.global_exports() {
        for symbol in globals.values() {
            if search.add(Some(symbol)) {
                return search.found;
            }
        }
    }
    for module in file.pattern_ambient_modules() {
        if search.add(Some(module.symbol)) {
            return search.found;
        }
    }
    search.found
}
