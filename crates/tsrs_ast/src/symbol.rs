use std::cell::Cell;
use std::hash::BuildHasher;
use std::sync::atomic::AtomicU64;

use hashbrown::HashTable;
use rustc_hash::FxBuildHasher;
use tsrs_core::{FrozenCell, OwnedCell, OwnedSliceCell, OwnedStrCell, P};

use crate::ast::{Node, SourceFile};
use crate::checkflags::CheckFlags;
use crate::modifierflags::ModifierFlags;
use crate::symbolflags::SymbolFlags;
use crate::*;

// Symbol
//
// Go's `Symbol` holds `Members`, `Exports` and `ExportSymbol` inline. Few symbols have any of them (on Project 5%
// of 15.3M: binder symbols of classes, interfaces, modules and exported locals; almost no transient symbols), so
// they live in a tail allocated on the first write of a non-nil value (`members()` / `set_members()` & co.):
// 72 bytes per symbol instead of 88. Reads of an absent tail return nil, like the unset Go field. `name` and
// `declarations` are packed (pointer + `u32` length, 4-byte aligned) next to the two flag words: 64 bytes.

#[derive(Default)]
pub struct Symbol {
    pub flags: OwnedCell<SymbolFlags>,
    pub check_flags: OwnedCell<CheckFlags>, // Non-zero only in transient symbols created by Checker
    pub name: OwnedStrCell,
    pub declarations: OwnedSliceCell<P<Node>>, // Go slice: shared by copies, replaced (not mutated) on append
    pub value_declaration: OwnedCell<Option<P<Node>>>,
    pub(crate) id: AtomicU64,
    pub parent: OwnedCell<Option<P<Symbol>>>,
    tables: OwnedCell<Option<P<SymbolTables>>>,
}

#[derive(Default)]
struct SymbolTables {
    members: OwnedCell<Option<P<SymbolTable>>>,
    exports: OwnedCell<Option<P<SymbolTable>>>,
    export_symbol: OwnedCell<Option<P<Symbol>>>,
}

const _: () = assert!(std::mem::size_of::<Symbol>() == 64);

impl Symbol {
    /// Allocates a fresh symbol (Go `&ast.Symbol{Flags: flags, Name: name}`).
    pub fn new(flags: SymbolFlags, name: &'static str) -> P<Symbol> {
        P::new(Symbol { flags: OwnedCell::new(flags), name: OwnedStrCell::new(name), ..Default::default() })
    }

    #[inline]
    pub fn flags(&self) -> SymbolFlags {
        self.flags.get()
    }
    #[inline]
    pub fn set_flags(&self, flags: SymbolFlags) {
        self.flags.set(flags)
    }
    #[inline]
    pub fn check_flags(&self) -> CheckFlags {
        self.check_flags.get()
    }
    #[inline]
    pub fn name(&self) -> &'static str {
        self.name.get()
    }
    #[inline]
    pub fn declarations(&self) -> &'static [P<Node>] {
        self.declarations.get()
    }
    #[inline]
    pub fn set_declarations(&self, declarations: &[P<Node>]) {
        self.declarations.set(tsrs_core::alloc_slice(declarations))
    }
    /// Go `append(symbol.Declarations, declarations...)`.
    pub fn append_declarations(&self, declarations: &[P<Node>]) {
        if declarations.is_empty() {
            return;
        }
        let mut result = Vec::with_capacity(self.declarations.get().len() + declarations.len());
        result.extend_from_slice(self.declarations.get());
        result.extend_from_slice(declarations);
        self.declarations.set(tsrs_core::alloc_vec(result))
    }
    #[inline]
    pub fn value_declaration(&self) -> Option<P<Node>> {
        self.value_declaration.get()
    }
    #[inline]
    fn tables_for_write(&self) -> P<SymbolTables> {
        match self.tables.get() {
            Some(tables) => tables,
            None => {
                let tables = P::new(SymbolTables::default());
                self.tables.set(Some(tables));
                tables
            }
        }
    }
    #[inline]
    pub fn members(&self) -> Option<P<SymbolTable>> {
        self.tables.get().and_then(|t| t.members.get())
    }
    #[inline]
    pub fn set_members(&self, members: Option<P<SymbolTable>>) {
        if members.is_some() || self.tables.get().is_some() {
            self.tables_for_write().members.set(members);
        }
    }
    #[inline]
    pub fn exports(&self) -> Option<P<SymbolTable>> {
        self.tables.get().and_then(|t| t.exports.get())
    }
    #[inline]
    pub fn set_exports(&self, exports: Option<P<SymbolTable>>) {
        if exports.is_some() || self.tables.get().is_some() {
            self.tables_for_write().exports.set(exports);
        }
    }
    #[inline]
    pub fn parent(&self) -> Option<P<Symbol>> {
        self.parent.get()
    }
    #[inline]
    pub fn export_symbol(&self) -> Option<P<Symbol>> {
        self.tables.get().and_then(|t| t.export_symbol.get())
    }
    #[inline]
    pub fn set_export_symbol(&self, export_symbol: Option<P<Symbol>>) {
        if export_symbol.is_some() || self.tables.get().is_some() {
            self.tables_for_write().export_symbol.set(export_symbol);
        }
    }

    pub fn is_external_module(&self) -> bool {
        self.flags.get().intersects(SymbolFlags::Module) && is_ambient_module_symbol_name(self.name.get())
    }

    pub fn is_static(&self) -> bool {
        let Some(value_declaration) = self.value_declaration.get() else {
            return false;
        };
        let modifier_flags = value_declaration.modifier_flags();
        modifier_flags.intersects(ModifierFlags::Static)
    }

    // See comment on `declareModuleMember` in `binder.go`.
    pub fn combined_local_and_export_symbol_flags(&self) -> SymbolFlags {
        if let Some(export_symbol) = self.export_symbol() {
            return self.flags.get() | export_symbol.flags.get();
        }
        self.flags.get()
    }
}

// GetSourceFileOfSymbol returns the owning file of a published binder symbol, or
// nil for a non-file-owned symbol, even if it borrows declarations from a file.
// Ownership recovery walks only the first declaration's AST parents.
pub fn get_source_file_of_symbol(symbol: P<Symbol>) -> Option<P<SourceFile>> {
    let mut symbol = symbol;
    if symbol.flags.get().intersects(SymbolFlags::Transient) {
        return None;
    }
    if symbol.declarations.get().is_empty() {
        // A class's implicit prototype has no declaration of its own.
        assert!(symbol.flags.get().intersects(SymbolFlags::Prototype), "File-bound symbol has no declarations");
        let parent = symbol.parent.get();
        assert!(parent.is_some_and(|p| p.flags.get().intersects(SymbolFlags::Class)), "Prototype has no declaring class");
        symbol = parent.unwrap();
        assert!(!symbol.flags.get().intersects(SymbolFlags::Transient), "Prototype parent is not file-bound");
        assert!(!symbol.declarations.get().is_empty(), "Prototype parent has no declarations");
    }
    let first = symbol.declarations.get()[0];
    let file = get_source_file_of_node(first);
    assert!(file.is_some(), "File-bound declaration has no source file");
    file
}

// SymbolTable
//
// Go's `SymbolTable` is a map (a reference type). Here it is an arena object handled as `P<SymbolTable>`;
// a Go nil table is `None`. Iteration is in insertion order and returns snapshots.
//
// Representation: the entries in insertion order in a `Vec` (24 bytes each), plus a hash index of entry
// positions once a table has more than `SYMBOL_TABLE_LINEAR_MAX` entries (most tables are tiny: on Project 82%
// of the 3.9M tables hold at most 8 entries); smaller tables are searched linearly. Same observable behavior as
// the insertion-ordered map it replaces (`IndexMap`): `set` of an existing name keeps the entry's position and
// stored key, `delete` shifts the later entries down.

#[derive(Default)]
pub struct SymbolTable(FrozenCell<SymbolMap>);

const SYMBOL_TABLE_LINEAR_MAX: usize = 8;

#[derive(Default, Clone)]
struct SymbolMap {
    entries: Vec<(&'static str, P<Symbol>)>,
    index: Option<Box<HashTable<u32>>>, // positions in `entries`; present once len > SYMBOL_TABLE_LINEAR_MAX
}

#[inline]
fn hash_name(name: &str) -> u64 {
    FxBuildHasher.hash_one(name)
}

impl SymbolMap {
    fn with_capacity(n: usize) -> SymbolMap {
        let index = (n > SYMBOL_TABLE_LINEAR_MAX).then(|| Box::new(HashTable::with_capacity(n)));
        SymbolMap { entries: Vec::with_capacity(n), index }
    }

    #[inline]
    fn position(&self, name: &str) -> Option<usize> {
        match &self.index {
            None => self.entries.iter().position(|(k, _)| *k == name),
            Some(index) => {
                let entries = &self.entries;
                index.find(hash_name(name), |&i| entries[i as usize].0 == name).map(|&i| i as usize)
            }
        }
    }

    fn insert(&mut self, name: &'static str, symbol: P<Symbol>) {
        if let Some(i) = self.position(name) {
            self.entries[i].1 = symbol;
            return;
        }
        let i = self.entries.len();
        if i == self.entries.capacity() && i >= 8 {
            // Grow by half instead of doubling: most large tables stop growing soon after (member tables,
            // property caches), and the slack of a doubled Vec is the larger part of their memory.
            self.entries.reserve_exact(i / 2);
        }
        self.entries.push((name, symbol));
        let entries = &self.entries;
        match &mut self.index {
            Some(index) => {
                index.insert_unique(hash_name(name), i as u32, |&j| hash_name(entries[j as usize].0));
            }
            None if entries.len() > SYMBOL_TABLE_LINEAR_MAX => {
                let mut index = HashTable::with_capacity(entries.len());
                for (j, (k, _)) in entries.iter().enumerate() {
                    index.insert_unique(hash_name(k), j as u32, |&m| hash_name(entries[m as usize].0));
                }
                self.index = Some(Box::new(index));
            }
            None => {}
        }
    }

    fn shift_remove(&mut self, name: &str) {
        let Some(i) = self.position(name) else {
            return;
        };
        if let Some(index) = &mut self.index {
            let entries = &self.entries;
            if let Ok(entry) = index.find_entry(hash_name(name), |&j| entries[j as usize].0 == name) {
                entry.remove();
            }
            for j in index.iter_mut() {
                if *j as usize > i {
                    *j -= 1;
                }
            }
        }
        self.entries.remove(i);
    }
}

impl SymbolTable {
    /// Go `make(ast.SymbolTable)`.
    pub fn new() -> P<SymbolTable> {
        P::new(SymbolTable::default())
    }

    /// Go `make(ast.SymbolTable, n)`.
    pub fn with_capacity(n: usize) -> P<SymbolTable> {
        P::new(SymbolTable(FrozenCell::new(SymbolMap::with_capacity(n))))
    }

    /// Go `maps.Clone(table)` for a non-nil table.
    pub fn clone_table(&self) -> P<SymbolTable> {
        P::new(SymbolTable(FrozenCell::new(self.0.borrow().clone())))
    }

    /// Go `table[name]`. On a `P<SymbolTable>` receiver `table.get(name)` resolves to `P::get`, so use
    /// `table.lookup(name)` (same thing) there.
    #[inline]
    pub fn get(&self, name: &str) -> Option<P<Symbol>> {
        self.lookup(name)
    }

    #[inline]
    pub fn lookup(&self, name: &str) -> Option<P<Symbol>> {
        let m = self.0.borrow();
        m.position(name).map(|i| m.entries[i].1)
    }

    /// `lookup` that also returns the stored key.
    #[inline]
    pub fn lookup_entry(&self, name: &str) -> Option<(&'static str, P<Symbol>)> {
        let m = self.0.borrow();
        m.position(name).map(|i| m.entries[i])
    }

    #[inline]
    pub fn set(&self, name: &'static str, symbol: P<Symbol>) {
        self.0.borrow_mut().insert(name, symbol);
    }

    pub fn delete(&self, name: &str) {
        self.0.borrow_mut().shift_remove(name);
    }

    #[inline]
    pub fn has(&self, name: &str) -> bool {
        self.0.borrow().position(name).is_some()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.0.borrow().entries.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.borrow().entries.is_empty()
    }

    /// Iterates over a snapshot, so `f` may mutate the table.
    pub fn for_each(&self, mut f: impl FnMut(&'static str, P<Symbol>)) {
        for (name, symbol) in self.entries() {
            f(name, symbol);
        }
    }

    pub fn entries(&self) -> Vec<(&'static str, P<Symbol>)> {
        self.0.borrow().entries.clone()
    }

    pub fn keys(&self) -> Vec<&'static str> {
        self.0.borrow().entries.iter().map(|(k, _)| *k).collect()
    }

    pub fn values(&self) -> Vec<P<Symbol>> {
        self.0.borrow().entries.iter().map(|(_, v)| *v).collect()
    }
}

// Go uses the byte 0xFE, which is invalid UTF-8 and cannot live in a Rust `&str`. The port uses U+007F
// (DEL): a single byte, so `name.as_bytes()[0]` / `name.as_bytes()[1]` checks translate one to one, and it
// cannot occur in an IdentifierName. Translate Go `name[0] == '\xFE'` to
// `name.as_bytes().first() == Some(&InternalSymbolNamePrefixByte)`.
pub const InternalSymbolNamePrefix: &str = "\u{7f}";
pub const InternalSymbolNamePrefixByte: u8 = 0x7f;

pub const InternalSymbolNameCall: &str = "\u{7f}call"; // Call signatures
pub const InternalSymbolNameConstructor: &str = "\u{7f}constructor"; // Constructor implementations
pub const InternalSymbolNameNew: &str = "\u{7f}new"; // Constructor signatures
pub const InternalSymbolNameIndex: &str = "\u{7f}index"; // Index signatures
pub const InternalSymbolNameExportStar: &str = "\u{7f}export"; // Module export * declarations
pub const InternalSymbolNameGlobal: &str = "\u{7f}global"; // Global self-reference
pub const InternalSymbolNameMissing: &str = "\u{7f}missing"; // Indicates missing symbol
pub const InternalSymbolNameType: &str = "\u{7f}type"; // Anonymous type literal symbol
pub const InternalSymbolNameObject: &str = "\u{7f}object"; // Anonymous object literal declaration
pub const InternalSymbolNameJSXAttributes: &str = "\u{7f}jsxAttributes"; // Anonymous JSX attributes object literal declaration
pub const InternalSymbolNameClass: &str = "\u{7f}class"; // Unnamed class expression
pub const InternalSymbolNameFunction: &str = "\u{7f}function"; // Unnamed function expression
pub const InternalSymbolNameComputed: &str = "\u{7f}computed"; // Computed property name declaration with dynamic name
pub const InternalSymbolNameAssignmentDeclaration: &str = "\u{7f}assignment"; // Assignment declarations
pub const InternalSymbolNameInstantiationExpression: &str = "\u{7f}instantiationExpression"; // Instantiation expressions
pub const InternalSymbolNameImportAttributes: &str = "\u{7f}importAttributes";
pub const InternalSymbolNameExportEquals: &str = "export="; // Export assignment symbol
pub const InternalSymbolNameDefault: &str = "default"; // Default export symbol (technically not wholly internal, but included here for usability)
pub const InternalSymbolNameThis: &str = "this";
pub const InternalSymbolNameModuleExports: &str = "module.exports";

pub fn symbol_name(symbol: P<Symbol>) -> &'static str {
    if let Some(value_declaration) = symbol.value_declaration.get() {
        if is_private_identifier_class_element_declaration(value_declaration) {
            return value_declaration.name().unwrap().text();
        }
    }
    symbol.name.get()
}

// EscapeAllInternalSymbolNames replaces internal symbol name markers with "__".
pub fn escape_all_internal_symbol_names(name: &str) -> String {
    name.replace(InternalSymbolNamePrefix, "__")
}

pub fn escape_internal_symbol_name(name: &str) -> String {
    if let Some(rest) = name.strip_prefix(InternalSymbolNamePrefix) {
        return format!("__{rest}");
    }
    name.to_string()
}

// EscapeSymbolName converts a binder symbol name into its escaped "__String"
// form. Internal names (prefixed with the sentinel) become "__"-prefixed,
// and user names that already begin with "__" gain an extra leading underscore
// so they can be distinguished from internal names.
pub fn escape_symbol_name(name: &str) -> String {
    if let Some(rest) = name.strip_prefix(InternalSymbolNamePrefix) {
        return format!("__{rest}");
    }
    let b = name.as_bytes();
    if b.len() >= 2 && b[0] == b'_' && b[1] == b'_' {
        return format!("_{name}");
    }
    name.to_string()
}
