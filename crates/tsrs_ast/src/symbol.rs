use std::cell::Cell;
use std::hash::BuildHasher;
use std::sync::atomic::AtomicU32;

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
// 72 bytes per symbol instead of 88. Reads of an absent tail return nil, like the unset Go field. `name` is a
// `PackedStr` (pointer and length in one word), `declarations` a 4-byte-aligned (pointer, `u32` length) pair packed
// with the two flag words and the `u32` id: 56 bytes.

#[derive(Default)]
pub struct Symbol {
    pub flags: OwnedCell<SymbolFlags>,
    pub check_flags: OwnedCell<CheckFlags>, // Non-zero only in transient symbols created by Checker
    pub name: OwnedStrCell,
    pub declarations: OwnedSliceCell<P<Node>>, // Go slice: shared by copies, replaced (not mutated) on append
    pub value_declaration: OwnedCell<Option<P<Node>>>,
    pub(crate) id: AtomicU32, // Go uint64; ids above u32::MAX panic in get_symbol_id
    pub parent: OwnedCell<Option<P<Symbol>>>,
    tables: OwnedCell<Option<P<SymbolTables>>>,
}

#[derive(Default)]
struct SymbolTables {
    members: OwnedCell<Option<P<SymbolTable>>>,
    exports: OwnedCell<Option<P<SymbolTable>>>,
    export_symbol: OwnedCell<Option<P<Symbol>>>,
}

const _: () = assert!(std::mem::size_of::<Symbol>() == 56);

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
// Representation: the entries in insertion order in a `Vec`, plus a hash index of entry positions once a table
// has more than `SYMBOL_TABLE_LINEAR_MAX` entries (most tables are tiny: on Project 82% of the 3.9M tables hold at
// most 8 entries); smaller tables are searched linearly, which compares only the entries' stored lengths and hashes
// until one matches (16 rather than 8 entries: -0.03 GiB on Project, the same instructions). Same observable behavior as the insertion-ordered map it
// replaces (`IndexMap`): `set` of an existing name keeps the entry's position and stored key, `delete` shifts the
// later entries down.
//
// An entry does not store its key: the key is almost always the symbol's own name (on Project all but 1 of 16.7M
// inserts; symbol names never change), so an entry is one word: the symbol plus a fingerprint of the key (its
// length up to 63 and 12 bits of its hash), which rejects non-matching entries without reading the symbol (see
// `SymbolMapEntry`). A key with other text than its symbol's name is kept in `SymbolMapExtra::odd_keys`. Keys
// are compared by text, so returning the symbol's name where the caller stored an equal string is unobservable.

#[derive(Default)]
pub struct SymbolTable(FrozenCell<SymbolMap>);

const SYMBOL_TABLE_LINEAR_MAX: usize = 16;

#[derive(Default, Clone)]
struct SymbolMap {
    entries: Vec<SymbolMapEntry>,
    extra: Option<Box<SymbolMapExtra>>,
}

/// One word: the symbol's address / 8 in the low 45 bits (symbols are 8-aligned and user-space addresses are below
/// 2^48; checked on store), then the odd-key flag (the key is in `odd_keys`), the key length capped at 63 (6 bits)
/// and the top 12 bits of `hash_name(key)`. 8 bytes instead of 16 (pointer + 32-bit hash + length): symbol table
/// entries are 465 MB of capacity on Project. The symbol's provenance is exposed on store and recovered with
/// `with_exposed_provenance`.
#[derive(Clone, Copy)]
struct SymbolMapEntry(u64);

const _: () = assert!(std::mem::size_of::<SymbolMapEntry>() == 8);

/// The key fingerprint stored in an entry (above the address bits): length capped at 63, 12 hash bits.
#[derive(Clone, Copy, PartialEq, Eq)]
struct KeyPrint {
    len: u64,
    hash: u64,
}

impl KeyPrint {
    #[inline]
    fn len_bits(name: &str) -> u64 {
        name.len().min(SymbolMapEntry::LEN_MASK as usize) as u64
    }

    #[inline]
    fn hash_bits(hash: u32) -> u64 {
        (hash >> (32 - SymbolMapEntry::HASH_BITS)) as u64
    }

    #[inline]
    fn of(name: &str, hash: u32) -> KeyPrint {
        KeyPrint { len: Self::len_bits(name), hash: Self::hash_bits(hash) }
    }
}

impl SymbolMapEntry {
    const ADDR_BITS: u32 = 45;
    const ADDR_MASK: u64 = (1 << Self::ADDR_BITS) - 1;
    const ODD_BIT: u64 = 1 << Self::ADDR_BITS;
    const LEN_SHIFT: u32 = Self::ADDR_BITS + 1;
    const LEN_MASK: u64 = (1 << 6) - 1;
    const HASH_SHIFT: u32 = Self::LEN_SHIFT + 6;
    const HASH_BITS: u32 = 64 - Self::HASH_SHIFT;

    #[inline]
    fn addr_bits(symbol: P<Symbol>) -> u64 {
        let addr = (symbol.get() as *const Symbol).expose_provenance() as u64;
        assert!(addr & 7 == 0 && addr >> (Self::ADDR_BITS + 3) == 0, "symbol address {addr:#x} does not fit a symbol table entry");
        addr >> 3
    }

    #[inline]
    fn new(symbol: P<Symbol>, print: KeyPrint) -> SymbolMapEntry {
        SymbolMapEntry(Self::addr_bits(symbol) | print.len << Self::LEN_SHIFT | print.hash << Self::HASH_SHIFT)
    }

    #[inline]
    fn symbol(self) -> P<Symbol> {
        let addr = ((self.0 & Self::ADDR_MASK) << 3) as usize;
        // SAFETY: the address was stored from a live `P<Symbol>` (arena symbols are never freed or moved), whose
        // provenance `addr_bits` exposed.
        P::from_static(unsafe { &*std::ptr::with_exposed_provenance::<Symbol>(addr) })
    }

    #[inline]
    fn set_symbol(&mut self, symbol: P<Symbol>) {
        self.0 = self.0 & !Self::ADDR_MASK | Self::addr_bits(symbol);
    }

    #[inline]
    fn is_odd(self) -> bool {
        self.0 & Self::ODD_BIT != 0
    }

    #[inline]
    fn len_bits(self) -> u64 {
        (self.0 >> Self::LEN_SHIFT) & Self::LEN_MASK
    }

    #[inline]
    fn print(self) -> KeyPrint {
        KeyPrint { len: self.len_bits(), hash: self.0 >> Self::HASH_SHIFT }
    }
}

#[derive(Default, Clone)]
struct SymbolMapExtra {
    index: Option<HashTable<u32>>, // positions in `entries`; present once len > SYMBOL_TABLE_LINEAR_MAX
    odd_keys: Vec<(u32, &'static str)>, // (position, key) of the entries whose key is not their symbol's name
}

#[inline]
fn hash_name(name: &str) -> u32 {
    let h = FxBuildHasher.hash_one(name);
    (h ^ (h >> 32)) as u32
}

/// The index's hash of an entry: its 32-bit hash spread over 64 bits (hashbrown takes its tag from the top bits).
#[inline]
fn index_hash(hash: u32) -> u64 {
    (hash as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}


impl SymbolMap {
    fn with_capacity(n: usize) -> SymbolMap {
        let extra = (n > SYMBOL_TABLE_LINEAR_MAX)
            .then(|| Box::new(SymbolMapExtra { index: Some(HashTable::with_capacity(n)), odd_keys: Vec::new() }));
        SymbolMap { entries: Vec::with_capacity(n), extra }
    }

    #[inline]
    fn index(&self) -> Option<&HashTable<u32>> {
        self.extra.as_ref().and_then(|e| e.index.as_ref())
    }

    /// The stored key of entry `i`.
    #[inline]
    fn key(&self, i: usize) -> &'static str {
        let e = self.entries[i];
        if !e.is_odd() {
            return e.symbol().name();
        }
        let odd_keys = &self.extra.as_ref().unwrap().odd_keys;
        odd_keys.iter().find(|&&(j, _)| j as usize == i).unwrap().1
    }

    /// Whether entry `i`'s key is `name` (whose fingerprint is `print`).
    #[inline]
    fn entry_matches(&self, i: usize, name: &str, print: KeyPrint) -> bool {
        self.entries[i].print() == print && self.key(i) == name
    }

    /// The hash of entry `i`'s key (rehashing the index, removing from it).
    #[inline]
    fn entry_hash(&self, i: usize) -> u32 {
        hash_name(self.key(i))
    }

    #[inline]
    fn position(&self, name: &str) -> Option<usize> {
        match self.index() {
            None => {
                // Hash the name only once an entry of the same (capped) length turns up.
                let len = KeyPrint::len_bits(name);
                let mut hash = None;
                (0..self.entries.len()).find(|&i| {
                    self.entries[i].len_bits() == len
                        && self.entry_matches(i, name, KeyPrint { len, hash: KeyPrint::hash_bits(*hash.get_or_insert_with(|| hash_name(name))) })
                })
            }
            Some(index) => {
                let hash = hash_name(name);
                let print = KeyPrint::of(name, hash);
                index.find(index_hash(hash), |&i| self.entry_matches(i as usize, name, print)).map(|&i| i as usize)
            }
        }
    }

    fn add_odd_key(&mut self, i: usize, key: &'static str) {
        self.entries[i].0 |= SymbolMapEntry::ODD_BIT;
        self.extra.get_or_insert_with(Default::default).odd_keys.push((i as u32, key));
    }

    fn insert(&mut self, name: &'static str, symbol: P<Symbol>) {
        if let Some(i) = self.position(name) {
            // Go keeps the stored key; it is no longer the new symbol's name when that differs.
            self.entries[i].set_symbol(symbol);
            if !self.entries[i].is_odd() && symbol.name() != name {
                self.add_odd_key(i, name);
            }
            return;
        }
        let i = self.entries.len();
        if i == self.entries.capacity() && i >= 8 {
            // Grow by half instead of doubling: most large tables stop growing soon after (member tables,
            // property caches), and the slack of a doubled Vec is the larger part of their memory.
            self.entries.reserve_exact(i / 2);
        }
        self.entries.push(SymbolMapEntry::new(symbol, KeyPrint::of(name, hash_name(name))));
        if symbol.name() != name {
            self.add_odd_key(i, name);
        }
        let len = self.entries.len();
        let has_index = self.index().is_some();
        if has_index || len > SYMBOL_TABLE_LINEAR_MAX {
            let mut index = self.extra.as_mut().and_then(|e| e.index.take()).unwrap_or_else(|| HashTable::with_capacity(len));
            let start = if has_index { i } else { 0 };
            for j in start..len {
                index.insert_unique(index_hash(self.entry_hash(j)), j as u32, |&m| index_hash(self.entry_hash(m as usize)));
            }
            self.extra.get_or_insert_with(Default::default).index = Some(index);
        }
    }

    fn shift_remove(&mut self, name: &str) {
        let Some(i) = self.position(name) else {
            return;
        };
        let hash = self.entry_hash(i);
        if let Some(extra) = &mut self.extra {
            if let Some(index) = &mut extra.index {
                if let Ok(entry) = index.find_entry(index_hash(hash), |&j| j as usize == i) {
                    entry.remove();
                }
                for j in index.iter_mut() {
                    if *j as usize > i {
                        *j -= 1;
                    }
                }
            }
            extra.odd_keys.retain(|&(j, _)| j as usize != i);
            for (j, _) in extra.odd_keys.iter_mut() {
                if *j as usize > i {
                    *j -= 1;
                }
            }
        }
        self.entries.remove(i);
    }

    fn pairs(&self) -> Vec<(&'static str, P<Symbol>)> {
        (0..self.entries.len()).map(|i| (self.key(i), self.entries[i].symbol())).collect()
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
        m.position(name).map(|i| m.entries[i].symbol())
    }

    /// `lookup` that also returns the stored key.
    #[inline]
    pub fn lookup_entry(&self, name: &str) -> Option<(&'static str, P<Symbol>)> {
        let m = self.0.borrow();
        m.position(name).map(|i| (m.key(i), m.entries[i].symbol()))
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
        self.0.borrow().pairs()
    }

    pub fn keys(&self) -> Vec<&'static str> {
        let m = self.0.borrow();
        (0..m.entries.len()).map(|i| m.key(i)).collect()
    }

    pub fn values(&self) -> Vec<P<Symbol>> {
        self.0.borrow().entries.iter().map(|e| e.symbol()).collect()
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
