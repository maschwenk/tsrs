use std::hash::BuildHasher;
use std::sync::atomic::AtomicU32;

use hashbrown::HashTable;
use rustc_hash::FxBuildHasher;
use tsrs_core::{OwnedCell, OwnedPSliceCell, OwnedTaggedStrCell, PKey, P};

use crate::ast::{Node, SourceFile};
use crate::checkflags::CheckFlags;
use crate::modifierflags::ModifierFlags;
use crate::symbolflags::SymbolFlags;
use crate::*;

// Symbol
//
// Go's `Symbol` holds `Members`, `Exports` and `ExportSymbol` inline. Few symbols have any of them (on the private monorepo 5%
// of 15.3M: binder symbols of classes, interfaces, modules and exported locals; almost no transient symbols), so
// they live in a tail allocated on the first write of a non-nil value (`members()` / `set_members()` & co.).
// Reads of an absent tail return nil, like the unset Go field. The tail shares a field with `parent`
// (`parent_or_tables`): the field holds the parent until a tail exists, then the tail (which holds the parent).
// `ValueDeclaration` is the first declaration in 82% of the symbols and nil in 17.5% (on the private monorepo:
// 10.88M / 2.31M of 13.2M; another node in 23K), so a bit says "the first declaration" and only another node is
// kept in the tail (`value_declaration()` / `set_value_declaration()`; a declarations write that replaces the
// first declaration moves it to the tail first). Both bits (`TAG_TABLES`, `TAG_VALUE_FIRST`) are the tag bits of
// the name word (`OwnedTaggedStrCell`: pointer, length and tags in one word). `declarations` is an
// `OwnedPSliceCell`: 40 bytes, 32 with compressed pointers (handle-sized parent, 8-byte declarations).

#[derive(Default)]
pub struct Symbol {
    pub flags: OwnedCell<SymbolFlags>,
    pub check_flags: OwnedCell<CheckFlags>, // Non-zero only in transient symbols created by Checker
    pub name: OwnedTaggedStrCell,
    declarations: OwnedPSliceCell<P<Node>>, // Go slice: shared by copies, replaced (not mutated) on append
    pub(crate) id: AtomicU32,               // Go uint64; ids above u32::MAX panic in get_symbol_id
    parent_or_tables: OwnedCell<PKey>,      // `P::key` of the parent or (with `TAG_TABLES`) of the tail; 0 = none
}

#[derive(Default)]
struct SymbolTables {
    parent: OwnedCell<Option<P<Symbol>>>,
    members: OwnedCell<Option<P<SymbolTable>>>,
    exports: OwnedCell<Option<P<SymbolTable>>>,
    export_symbol: OwnedCell<Option<P<Symbol>>>,
    value_declaration: OwnedCell<Option<P<Node>>>, // when it is not the first declaration
}

const _: () = assert!(std::mem::size_of::<Symbol>() == if tsrs_core::COMPRESSED_PTRS { 32 } else { 40 });

/// `parent_or_tables` holds the `SymbolTables` tail.
const TAG_TABLES: u8 = 1;
/// The value declaration is the first declaration.
const TAG_VALUE_FIRST: u8 = 2;

/// Census builds: the name word keeps its length and tag bits above the address (`crate::census_layouts`).
pub(crate) fn census_layout() {
    let off = std::mem::offset_of!(Symbol, name);
    tsrs_core::census_layout(std::any::type_name::<Symbol>(), &[tsrs_core::CensusField::Tagged { off }]);
}

impl Symbol {
    /// Allocates a fresh symbol (Go `&ast.Symbol{Flags: flags, Name: name}`).
    pub fn new(flags: SymbolFlags, name: &'static str) -> P<Symbol> {
        P::new(Symbol { flags: OwnedCell::new(flags), name: OwnedTaggedStrCell::new(name), ..Default::default() })
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
    /// Go `symbol.Declarations = slices.Clone(declarations)`-like: stores a copy.
    #[inline]
    pub fn set_declarations(&self, declarations: &[P<Node>]) {
        self.set_declarations_static(tsrs_core::alloc_slice(declarations))
    }
    /// Go `symbol.Declarations = declarations` (shares the slice).
    pub fn set_declarations_static(&self, declarations: &'static [P<Node>]) {
        let tags = self.name.tags();
        if tags & TAG_VALUE_FIRST != 0 {
            let value_declaration = self.declarations.get()[0];
            if declarations.first() != Some(&value_declaration) {
                self.name.set_tags(tags & !TAG_VALUE_FIRST);
                self.tables_for_write().value_declaration.set(Some(value_declaration));
            }
        }
        self.declarations.set(declarations)
    }
    /// Go `append(symbol.Declarations, declarations...)`.
    pub fn append_declarations(&self, declarations: &[P<Node>]) {
        if declarations.is_empty() {
            return;
        }
        let mut result = Vec::with_capacity(self.declarations.get().len() + declarations.len());
        result.extend_from_slice(self.declarations.get());
        result.extend_from_slice(declarations);
        self.set_declarations_static(tsrs_core::alloc_vec(result))
    }
    #[inline]
    #[expect(clippy::disallowed_methods, reason = "indexing with a bounds check: +0.7% instructions, one checker (notes/mem-small.md)")]
    pub fn value_declaration(&self) -> Option<P<Node>> {
        if self.name.tags() & TAG_VALUE_FIRST != 0 {
            let declarations = self.declarations.get();
            debug_assert!(!declarations.is_empty());
            // SAFETY: the bit is set only while the declarations are non-empty (`set_value_declaration`,
            // `set_declarations_static`).
            return Some(unsafe { *declarations.get_unchecked(0) });
        }
        self.tables().and_then(|t| t.value_declaration.get())
    }
    pub fn set_value_declaration(&self, value_declaration: Option<P<Node>>) {
        let tags = self.name.tags();
        if value_declaration.is_some() && self.declarations.get().first().copied() == value_declaration {
            self.name.set_tags(tags | TAG_VALUE_FIRST);
            if let Some(tables) = self.tables() {
                tables.value_declaration.set(None);
            }
            return;
        }
        self.name.set_tags(tags & !TAG_VALUE_FIRST);
        if value_declaration.is_some() || self.tables().is_some() {
            self.tables_for_write().value_declaration.set(value_declaration);
        }
    }
    #[inline]
    fn tables(&self) -> Option<P<SymbolTables>> {
        // SAFETY: with the tag, the field holds the key of the tail made by `tables_for_write` (never freed).
        (self.name.tags() & TAG_TABLES != 0).then(|| unsafe { P::from_key(self.parent_or_tables.get()) })
    }
    #[inline]
    fn tables_for_write(&self) -> P<SymbolTables> {
        match self.tables() {
            Some(tables) => tables,
            None => {
                let tables = P::new(SymbolTables { parent: OwnedCell::new(self.parent()), ..Default::default() });
                self.parent_or_tables.set(tables.key());
                self.name.set_tags(self.name.tags() | TAG_TABLES);
                tables
            }
        }
    }
    #[inline]
    pub fn members(&self) -> Option<P<SymbolTable>> {
        self.tables().and_then(|t| t.members.get())
    }
    #[inline]
    pub fn set_members(&self, members: Option<P<SymbolTable>>) {
        if members.is_some() || self.tables().is_some() {
            self.tables_for_write().members.set(members);
        }
    }
    #[inline]
    pub fn exports(&self) -> Option<P<SymbolTable>> {
        self.tables().and_then(|t| t.exports.get())
    }
    #[inline]
    pub fn set_exports(&self, exports: Option<P<SymbolTable>>) {
        if exports.is_some() || self.tables().is_some() {
            self.tables_for_write().exports.set(exports);
        }
    }
    #[inline]
    pub fn parent(&self) -> Option<P<Symbol>> {
        match self.tables() {
            Some(tables) => tables.parent.get(),
            // SAFETY: without the tag, the field is 0 or the key of the parent (a live symbol).
            None => unsafe { P::from_key_opt(self.parent_or_tables.get()) },
        }
    }
    #[inline]
    pub fn set_parent(&self, parent: Option<P<Symbol>>) {
        match self.tables() {
            Some(tables) => tables.parent.set(parent),
            None => self.parent_or_tables.set(P::key_opt(parent)),
        }
    }
    #[inline]
    pub fn export_symbol(&self) -> Option<P<Symbol>> {
        self.tables().and_then(|t| t.export_symbol.get())
    }
    #[inline]
    pub fn set_export_symbol(&self, export_symbol: Option<P<Symbol>>) {
        if export_symbol.is_some() || self.tables().is_some() {
            self.tables_for_write().export_symbol.set(export_symbol);
        }
    }

    pub fn is_external_module(&self) -> bool {
        self.flags.get().intersects(SymbolFlags::Module) && is_ambient_module_symbol_name(self.name.get())
    }

    pub fn is_static(&self) -> bool {
        let Some(value_declaration) = self.value_declaration() else {
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
    if symbol.declarations().is_empty() {
        // A class's implicit prototype has no declaration of its own.
        assert!(symbol.flags.get().intersects(SymbolFlags::Prototype), "File-bound symbol has no declarations");
        let parent = symbol.parent();
        assert!(parent.is_some_and(|p| p.flags.get().intersects(SymbolFlags::Class)), "Prototype has no declaring class");
        symbol = parent.unwrap();
        assert!(!symbol.flags.get().intersects(SymbolFlags::Transient), "Prototype parent is not file-bound");
        assert!(!symbol.declarations().is_empty(), "Prototype parent has no declarations");
    }
    let first = symbol.declarations()[0];
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
// has more than `SYMBOL_TABLE_LINEAR_MAX` entries (most tables are tiny: on the private monorepo 82% of the 3.9M tables hold at
// most 8 entries); smaller tables are searched linearly, which compares only the entries' stored lengths and hashes
// until one matches (16 rather than 8 entries: -0.03 GiB on the private monorepo, the same instructions). Same observable behavior as the insertion-ordered map it
// replaces (`IndexMap`): `set` of an existing name keeps the entry's position and stored key, `delete` shifts the
// later entries down.
//
// An entry does not store its key: the key is almost always the symbol's own name (on the private monorepo all but 1 of 16.7M
// inserts; symbol names never change), so an entry is one word: the symbol plus a fingerprint of the key (its
// length up to 63 and 12 bits of its hash), which rejects non-matching entries without reading the symbol (see
// `SymbolMapEntry`). A key with other text than its symbol's name is kept in `SymbolMapExtra::odd_keys`. Keys
// are compared by text, so returning the symbol's name where the caller stored an equal string is unobservable.
//
// Frozen tables (not in Go; notes/mem-flat-symbol-tables.md): when the binder finishes a file it replaces the small
// tables it made with frozen ones (`freeze_symbol_tables`): a one-word header (`FrozenTable`, bit 63 set, where a
// mutable table's first word is its entry buffer's address) whose entries are a run in one exactly sized arena
// array per file. Nothing writes to a binder table once its file is bound (the checker clones a table before it
// merges into it); a write to a frozen table panics.

/// The ownership contract of `tsrs_core::FrozenCell`: only the owning thread writes, and never while other threads
/// can read (a binder table is written only while its file is bound). Not a `FrozenCell`, whose layout is free in
/// checked builds: the first word must be the entry buffer's address in every build (`repr(C)` down to
/// `EntryVec::ptr`), so that a frozen table (`FrozenTable`) can be told from it.
#[repr(C)]
pub struct SymbolTable(std::cell::UnsafeCell<SymbolMap>);

impl Default for SymbolTable {
    fn default() -> SymbolTable {
        SymbolTable::from_map(SymbolMap::default())
    }
}

// SAFETY: the map owns plain words and boxes of them (`EntryVec`, `ExtraSlot`, both Send).
unsafe impl Send for SymbolTable {}
// SAFETY: `FrozenCell`'s contract (above): shared access only reads, and the owning thread writes only while no other
// thread can read the table; a frozen table is never written.
unsafe impl Sync for SymbolTable {}

const _: () = assert!(std::mem::size_of::<SymbolTable>() == 24);

const SYMBOL_TABLE_LINEAR_MAX: usize = 16;

#[derive(Default, Clone)]
#[repr(C)]
struct SymbolMap {
    entries: EntryVec,
    extra: ExtraSlot,
}

const _: () = assert!(std::mem::size_of::<SymbolMap>() == 24);

/// `Vec<SymbolMapEntry>` with a `u32` length and capacity (16 bytes instead of 24; 3.5M symbol tables on the private monorepo).
/// Grows like `Vec` (`push` doubles from 4; `reserve_exact` adds exactly). `ptr` is first (`SymbolTable`).
#[repr(C)]
struct EntryVec {
    ptr: std::ptr::NonNull<SymbolMapEntry>,
    len: u32,
    cap: u32,
}

impl Default for EntryVec {
    fn default() -> Self {
        EntryVec { ptr: std::ptr::NonNull::dangling(), len: 0, cap: 0 }
    }
}

impl EntryVec {
    fn with_capacity(n: usize) -> EntryVec {
        let mut v = EntryVec::default();
        v.reserve_exact(n);
        v
    }

    #[inline]
    fn capacity(&self) -> usize {
        self.cap as usize
    }

    fn layout(cap: usize) -> std::alloc::Layout {
        std::alloc::Layout::array::<SymbolMapEntry>(cap).expect("symbol table too large")
    }

    /// Makes room for at least `additional` more entries, allocating exactly that much when it grows.
    fn reserve_exact(&mut self, additional: usize) {
        let needed = self.len as usize + additional;
        if needed > self.cap as usize {
            self.set_capacity(needed);
        }
    }

    fn set_capacity(&mut self, cap: usize) {
        let cap32 = u32::try_from(cap).expect("symbol table with more than u32::MAX entries");
        let ptr = if self.cap == 0 {
            // SAFETY: `cap` > 0 entries of a non-zero-sized type.
            unsafe { std::alloc::alloc(Self::layout(cap)) }
        } else {
            // SAFETY: `ptr` was allocated with the layout of `self.cap` entries.
            unsafe { std::alloc::realloc(self.ptr.as_ptr().cast(), Self::layout(self.cap as usize), Self::layout(cap).size()) }
        };
        self.ptr = std::ptr::NonNull::new(ptr.cast()).unwrap_or_else(|| std::alloc::handle_alloc_error(Self::layout(cap)));
        self.cap = cap32;
    }

    #[inline]
    fn push(&mut self, e: SymbolMapEntry) {
        if self.len == self.cap {
            self.set_capacity((self.cap as usize * 2).max(4));
        }
        // SAFETY: `len` < `cap`.
        unsafe { self.ptr.as_ptr().add(self.len as usize).write(e) };
        self.len += 1;
    }

    fn remove(&mut self, i: usize) {
        let len = self.len as usize;
        assert!(i < len, "removal index out of bounds");
        // SAFETY: `i` < `len`; the entries are `Copy`.
        unsafe { std::ptr::copy(self.ptr.as_ptr().add(i + 1), self.ptr.as_ptr().add(i), len - i - 1) };
        self.len -= 1;
    }
}

impl std::ops::Deref for EntryVec {
    type Target = [SymbolMapEntry];
    #[inline]
    fn deref(&self) -> &[SymbolMapEntry] {
        // SAFETY: the first `len` entries are initialized (dangling and empty when `cap` is 0).
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len as usize) }
    }
}

impl std::ops::DerefMut for EntryVec {
    #[inline]
    fn deref_mut(&mut self) -> &mut [SymbolMapEntry] {
        // SAFETY: as in `deref`, and `&mut self` is unique.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len as usize) }
    }
}

impl Clone for EntryVec {
    /// Like `Vec::clone`: capacity equal to the length.
    fn clone(&self) -> EntryVec {
        let mut v = EntryVec::with_capacity(self.len as usize);
        for &e in self.iter() {
            v.push(e);
        }
        v
    }
}

impl Drop for EntryVec {
    fn drop(&mut self) {
        if self.cap != 0 {
            // SAFETY: allocated with the layout of `cap` entries.
            unsafe { std::alloc::dealloc(self.ptr.as_ptr().cast(), Self::layout(self.cap as usize)) };
        }
    }
}

// SAFETY: an `EntryVec` owns its buffer like a `Vec` (entries are plain words).
unsafe impl Send for EntryVec {}
// SAFETY: as for the `Vec` it stands for: through `&EntryVec` the plain-word entries are only read.
unsafe impl Sync for EntryVec {}

/// One word: the symbol in the low 45 bits (`P::pack`), then the odd-key flag (the key is in `odd_keys`), the key length capped at 63 (6 bits)
/// and the top 12 bits of `hash_name(key)`. 8 bytes instead of 16 (pointer + 32-bit hash + length): symbol table
/// entries are 465 MB of capacity on the private monorepo.
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
    const ADDR_BITS: u32 = tsrs_core::PACK_BITS;
    const ADDR_MASK: u64 = (1 << Self::ADDR_BITS) - 1;
    const ODD_BIT: u64 = 1 << Self::ADDR_BITS;
    const LEN_SHIFT: u32 = Self::ADDR_BITS + 1;
    const LEN_MASK: u64 = (1 << 6) - 1;
    const HASH_SHIFT: u32 = Self::LEN_SHIFT + 6;
    const HASH_BITS: u32 = 64 - Self::HASH_SHIFT;

    #[inline]
    fn addr_bits(symbol: P<Symbol>) -> u64 {
        symbol.pack()
    }

    #[inline]
    fn new(symbol: P<Symbol>, print: KeyPrint) -> SymbolMapEntry {
        SymbolMapEntry(Self::addr_bits(symbol) | print.len << Self::LEN_SHIFT | print.hash << Self::HASH_SHIFT)
    }

    #[inline]
    fn symbol(self) -> P<Symbol> {
        // SAFETY: the low bits were stored from a live `P<Symbol>` (arena symbols are never freed or moved).
        unsafe { P::unpack(self.0) }
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

/// `SymbolMap::extra`: one word holding either a boxed `SymbolMapExtra` (an even address) or, in a table without
/// one (a linear table whose keys are all their symbols' names: nearly every table), a 64-bit Bloom filter of its
/// keys' hashes (two bits per key) whose bit 0 is set as the tag. A lookup the filter rejects returns without
/// reading the entries, which are a second cache line: over half of all lookups are misses in small tables
/// (a property lookup tries each type on the apparent-type chain, down to `Object`'s members), and the entries of
/// a table looked up once in a while are rarely in cache. Deleting a key leaves its bits set (a superset is still
/// a valid filter).
struct ExtraSlot(*mut SymbolMapExtra);

impl ExtraSlot {
    const EMPTY_FILTER: usize = 1;

    #[inline]
    fn boxed(extra: SymbolMapExtra) -> ExtraSlot {
        ExtraSlot(Box::into_raw(Box::new(extra)))
    }

    #[inline]
    fn is_filter(&self) -> bool {
        self.0.addr() & 1 != 0
    }

    #[inline]
    fn get(&self) -> Option<&SymbolMapExtra> {
        // SAFETY: an even word is the pointer `boxed` created, owned by this slot.
        (!self.is_filter()).then(|| unsafe { &*self.0 })
    }

    #[inline]
    fn get_mut(&mut self) -> Option<&mut SymbolMapExtra> {
        // SAFETY: as in `get`, and `&mut self` is unique.
        (!self.is_filter()).then(|| unsafe { &mut *self.0 })
    }

    /// The boxed extra, created (dropping the filter) when the slot holds a filter.
    fn get_or_insert(&mut self) -> &mut SymbolMapExtra {
        if self.is_filter() {
            *self = ExtraSlot::boxed(SymbolMapExtra::default());
        }
        self.get_mut().unwrap()
    }

    #[inline]
    fn filter_bits(hash: u32) -> usize {
        (1usize << (hash & 63)) | (1usize << ((hash >> 6) & 63))
    }

    /// False if no key with this hash is in a table whose slot holds a filter.
    #[inline]
    fn may_contain(&self, hash: u32) -> bool {
        let bits = Self::filter_bits(hash);
        !self.is_filter() || self.0.addr() & bits == bits
    }

    #[inline]
    fn add_to_filter(&mut self, hash: u32) {
        if self.is_filter() {
            self.0 = std::ptr::without_provenance_mut(self.0.addr() | Self::filter_bits(hash));
        }
    }
}

const _: () = assert!(usize::BITS == 64);

impl Default for ExtraSlot {
    #[inline]
    fn default() -> ExtraSlot {
        ExtraSlot(std::ptr::without_provenance_mut(Self::EMPTY_FILTER))
    }
}

impl Clone for ExtraSlot {
    fn clone(&self) -> ExtraSlot {
        match self.get() {
            Some(extra) => ExtraSlot::boxed(extra.clone()),
            None => ExtraSlot(self.0),
        }
    }
}

impl Drop for ExtraSlot {
    fn drop(&mut self) {
        if !self.is_filter() {
            // SAFETY: an even word is the pointer `boxed` created, owned by this slot.
            drop(unsafe { Box::from_raw(self.0) });
        }
    }
}

// SAFETY: an `ExtraSlot` owns its `SymbolMapExtra` like a `Box` (or holds plain bits).
unsafe impl Send for ExtraSlot {}
// SAFETY: as for the `Box` it stands for: `SymbolMapExtra` has no interior mutability, and through `&ExtraSlot` it
// is only read.
unsafe impl Sync for ExtraSlot {}

#[inline]
fn hash_name(name: &str) -> u32 {
    let h = FxBuildHasher.hash_one(name);
    (h ^ (h >> 32)) as u32
}

/// `a == b`, decided without reading the bytes when both are the same string: over 40% of lookup hits use the very
/// string the symbol was named with (an instantiated property looked up by its declaration's name), and an insert
/// nearly always stores a symbol under its own name.
#[inline]
fn same_text(a: &str, b: &str) -> bool {
    a.len() == b.len() && (std::ptr::eq(a.as_ptr(), b.as_ptr()) || a.as_bytes() == b.as_bytes())
}

/// The index's hash of an entry: its 32-bit hash spread over 64 bits (hashbrown takes its tag from the top bits).
#[inline]
fn index_hash(hash: u32) -> u64 {
    (hash as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}


impl SymbolMap {
    fn with_capacity(n: usize) -> SymbolMap {
        let extra = if n > SYMBOL_TABLE_LINEAR_MAX {
            ExtraSlot::boxed(SymbolMapExtra { index: Some(HashTable::with_capacity(n)), odd_keys: Vec::new() })
        } else {
            ExtraSlot::default()
        };
        SymbolMap { entries: EntryVec::with_capacity(n), extra }
    }

    #[inline]
    fn index(&self) -> Option<&HashTable<u32>> {
        self.extra.get().and_then(|e| e.index.as_ref())
    }

    /// The stored key of entry `i`.
    #[inline]
    fn key(&self, i: usize) -> &'static str {
        let e = self.entries[i];
        if !e.is_odd() {
            return e.symbol().name();
        }
        let odd_keys = &self.extra.get().unwrap().odd_keys;
        odd_keys.iter().find(|&&(j, _)| j as usize == i).unwrap().1
    }

    /// Whether entry `i`'s key is `name` (whose fingerprint is `print`).
    #[inline]
    fn entry_matches(&self, i: usize, name: &str, print: KeyPrint) -> bool {
        self.entries[i].print() == print && same_text(self.key(i), name)
    }

    /// The hash of entry `i`'s key (rehashing the index, removing from it).
    #[inline]
    fn entry_hash(&self, i: usize) -> u32 {
        hash_name(self.key(i))
    }

    /// The position of `name`. Over half of all lookups end at the filter (a miss in a small table), so that test is
    /// all this function does before the search, which is out of line: with the search inlined, every call saved
    /// the search's registers first.
    #[inline]
    fn position(&self, name: &str) -> Option<usize> {
        let hash = hash_name(name);
        if !self.extra.may_contain(hash) {
            return None;
        }
        self.search(name, hash)
    }

    /// `position` past the filter (`hash` = `hash_name(name)`).
    #[inline(never)]
    fn search(&self, name: &str, hash: u32) -> Option<usize> {
        let print = KeyPrint::of(name, hash);
        match self.index() {
            None => (0..self.entries.len()).find(|&i| self.entry_matches(i, name, print)),
            Some(index) => index.find(index_hash(hash), |&i| self.entry_matches(i as usize, name, print)).map(|&i| i as usize),
        }
    }

    fn add_odd_key(&mut self, i: usize, key: &'static str) {
        self.entries[i].0 |= SymbolMapEntry::ODD_BIT;
        self.extra.get_or_insert().odd_keys.push((i as u32, key));
    }

    fn insert(&mut self, name: &'static str, symbol: P<Symbol>) {
        let hash = hash_name(name);
        if let Some(i) = if self.extra.may_contain(hash) { self.search(name, hash) } else { None } {
            // Go keeps the stored key; it is no longer the new symbol's name when that differs.
            self.entries[i].set_symbol(symbol);
            if !self.entries[i].is_odd() && !same_text(symbol.name(), name) {
                self.add_odd_key(i, name);
            }
            return;
        }
        let i = self.entries.len();
        if i == self.entries.capacity() && i >= 8 {
            // Grow by half instead of doubling: most large tables stop growing soon after (member tables,
            // property caches), and the slack of a doubled Vec is the larger part of their memory.
            self.entries.reserve_exact(i / 2);
        } else if i == self.entries.capacity() && i < 2 {
            // Capacity 1, then 2, then doubling from 4 (`push`): over half of the binder's tables hold one or two
            // symbols (locals of small functions, members of small object literals).
            self.entries.reserve_exact(1);
        }
        self.entries.push(SymbolMapEntry::new(symbol, KeyPrint::of(name, hash)));
        self.extra.add_to_filter(hash);
        if !same_text(symbol.name(), name) {
            self.add_odd_key(i, name);
        }
        let len = self.entries.len();
        let has_index = self.index().is_some();
        if has_index || len > SYMBOL_TABLE_LINEAR_MAX {
            let mut index = self.extra.get_mut().and_then(|e| e.index.take()).unwrap_or_else(|| HashTable::with_capacity(len));
            let start = if has_index { i } else { 0 };
            for j in start..len {
                index.insert_unique(index_hash(self.entry_hash(j)), j as u32, |&m| index_hash(self.entry_hash(m as usize)));
            }
            self.extra.get_or_insert().index = Some(index);
        }
    }

    fn shift_remove(&mut self, name: &str) {
        let Some(i) = self.position(name) else {
            return;
        };
        let hash = self.entry_hash(i);
        if let Some(extra) = self.extra.get_mut() {
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

/// A frozen table: one word, bit 63 set (a mutable table's first word is a user-space address, below 2^48), its
/// length in bits 58..63, the filter of its keys (`ExtraSlot`'s, narrower) and its run of entries in the file's
/// frozen-entries array (`P::to_bits` of the first, 0 for an empty table): with compressed pointers the run / 8 in
/// bits 26..58 and the filter in bits 0..26; with plain pointers the run's address in the low 48 bits (a tagged
/// pointer to the census, `census_layout_frozen`) and the filter in bits 48..58. A table of more than
/// `WIDE_FILTER_MIN` entries, whose keys would fill so narrow a filter, also keeps its mutable form's 64-bit filter
/// in the word before its run (`ExtraSlot`). `P<SymbolTable>` handles of frozen tables point at one of these.
#[derive(Clone, Copy)]
#[repr(transparent)]
struct FrozenTable(u64);

/// One entry of a file's frozen-entries array (its own type so the allocation profile shows the array as a row).
#[derive(Clone, Copy)]
#[repr(transparent)]
struct FrozenEntry(SymbolMapEntry);

impl FrozenTable {
    const TAG: u64 = 1 << 63;
    const LEN_SHIFT: u32 = 58;
    const FILTER_SHIFT: u32 = if tsrs_core::COMPRESSED_PTRS { 0 } else { 48 };
    const FILTER_BITS: u64 = if tsrs_core::COMPRESSED_PTRS { 26 } else { 10 };
    const RUN_SHIFT: u32 = 26;
    const WIDE_FILTER_MIN: usize = 4;

    fn new(run: Option<P<FrozenEntry>>, len: usize, filter: u64) -> FrozenTable {
        assert!(len <= SYMBOL_TABLE_LINEAR_MAX);
        let bits = P::to_bits_opt(run) as u64;
        let run = if tsrs_core::COMPRESSED_PTRS { bits >> 3 << Self::RUN_SHIFT } else { bits };
        FrozenTable(Self::TAG | (len as u64) << Self::LEN_SHIFT | run | filter)
    }

    #[inline]
    fn entries(self) -> &'static [SymbolMapEntry] {
        let len = ((self.0 >> Self::LEN_SHIFT) & 31) as usize;
        if len == 0 {
            return &[];
        }
        let bits = if tsrs_core::COMPRESSED_PTRS { ((self.0 >> Self::RUN_SHIFT) as u32 as u64) << 3 } else { self.0 & ((1 << 48) - 1) };
        // SAFETY: `new` stored `to_bits` of the first of this table's `len` entries, in an arena array that is never
        // freed; `FrozenEntry` is a transparent `SymbolMapEntry`.
        unsafe { std::slice::from_raw_parts(std::ptr::from_ref(P::<FrozenEntry>::from_bits(bits as usize).get()).cast::<SymbolMapEntry>(), len) }
    }

    /// Two of the filter's bits, from the 12 hash bits an entry keeps (`KeyPrint::hash`, so freezing rehashes
    /// nothing), by multiply-shift over the filter's width.
    #[inline]
    fn filter_bits(print_hash: u64) -> u64 {
        let bit = |h: u64| 1 << ((h & 63) * Self::FILTER_BITS >> 6);
        (bit(print_hash) | bit(print_hash >> 6)) << Self::FILTER_SHIFT
    }

    #[inline]
    fn may_contain(self, hash: u32) -> bool {
        let bits = Self::filter_bits(KeyPrint::hash_bits(hash));
        self.0 & bits == bits
    }

    /// The 64-bit filter (`ExtraSlot`'s word) of a table of more than `WIDE_FILTER_MIN` entries.
    #[inline]
    fn wide_filter(entries: &'static [SymbolMapEntry]) -> usize {
        // SAFETY: the word before the run of such a table is its 64-bit filter (`freeze_symbol_tables`).
        unsafe { entries.as_ptr().sub(1).read().0 as usize }
    }

    #[inline]
    fn search(self, name: &str, hash: u32) -> Option<SymbolMapEntry> {
        let entries = self.entries();
        if entries.len() > Self::WIDE_FILTER_MIN {
            let bits = ExtraSlot::filter_bits(hash);
            if Self::wide_filter(entries) & bits != bits {
                return None;
            }
        }
        let print = KeyPrint::of(name, hash);
        entries.iter().copied().find(|e| e.print() == print && same_text(e.symbol().name(), name))
    }

    /// Out of line, like `SymbolMap::search`: the callers inline only the test for the frozen form, so the mutable
    /// path stays as small as it was.
    #[inline(never)]
    fn find(self, name: &str) -> Option<SymbolMapEntry> {
        let hash = hash_name(name);
        if !self.may_contain(hash) {
            return None;
        }
        self.search(name, hash)
    }
}

/// Census builds (plain pointers): a frozen table's word is the run's address with flag bits above it.
pub(crate) fn census_layout_frozen() {
    tsrs_core::census_layout(std::any::type_name::<FrozenTable>(), &[tsrs_core::CensusField::Tagged { off: 0 }]);
}

/// The frozen replacement of each of the binder's `tables` of a bound file (`None` for a table that stays mutable:
/// one with a hash index or odd keys, i.e. more than `SYMBOL_TABLE_LINEAR_MAX` entries or a key that is not its
/// symbol's name). The entries of all of them go to one exactly sized arena array, in each table's order (odd-key
/// bits clear, so a frozen entry's key is its symbol's name). `tables` must not repeat a table. The caller points
/// the tables' owners at the replacements and then gives the old tables back (`release_symbol_table`).
pub fn freeze_symbol_tables(tables: &[P<SymbolTable>]) -> Vec<Option<P<SymbolTable>>> {
    debug_assert!({
        let mut keys: Vec<PKey> = tables.iter().map(|t| t.key()).collect();
        keys.sort_unstable();
        keys.windows(2).all(|w| w[0] != w[1])
    });
    let freezable = |t: P<SymbolTable>| t.frozen().is_none() && t.map().extra.get().is_none();
    let mut words: Vec<FrozenEntry> = Vec::new();
    let mut runs: Vec<Option<(usize, usize)>> = Vec::with_capacity(tables.len());
    for &t in tables {
        if !freezable(t) {
            runs.push(None);
            continue;
        }
        let entries = &t.map().entries;
        if entries.len() > FrozenTable::WIDE_FILTER_MIN {
            // The filter of a table without an `ExtraSlot` box (`freezable`), bit 0 (its tag) included.
            words.push(FrozenEntry(SymbolMapEntry(t.map().extra.0.addr() as u64)));
        }
        runs.push(Some((words.len(), entries.len())));
        words.extend(entries.iter().map(|&e| FrozenEntry(e)));
    }
    let array = tsrs_core::alloc_slice(&words);
    tables
        .iter()
        .zip(runs)
        .map(|(t, run)| {
            let (start, len) = run?;
            let filter = t.map().entries.iter().fold(0, |f, e| f | FrozenTable::filter_bits(e.print().hash));
            // SAFETY: an element of the fresh arena array (8-byte entries, 8-aligned).
            let run = (len != 0).then(|| unsafe { P::from_arena(&array[start]) });
            // SAFETY: a `SymbolTable` handle may point at a `FrozenTable`: every method reads only the first word
            // until it has seen that the table is not frozen.
            Some(unsafe { P::new(FrozenTable::new(run, len, filter)).cast::<SymbolTable>() })
        })
        .collect()
}

/// Gives a table that `freeze_symbol_tables` replaced back to the arena (its entry buffer to the heap).
///
/// # Safety
/// Nothing may point to the table any more (its owners point at the replacement).
pub unsafe fn release_symbol_table(table: P<SymbolTable>) {
    // An empty map: dropping the table again (a region's drop list) frees nothing.
    *table.map_mut() = SymbolMap::default();
    // SAFETY: the caller's contract.
    unsafe { tsrs_core::free!(table) };
}

impl SymbolTable {
    fn from_map(map: SymbolMap) -> SymbolTable {
        SymbolTable(std::cell::UnsafeCell::new(map))
    }

    /// The table's frozen form, if it is one.
    #[inline]
    fn frozen(&self) -> Option<FrozenTable> {
        // SAFETY: the first word is `EntryVec::ptr` (an address below 2^48) or a `FrozenTable` (bit 63 set), and is
        // written only by the owning thread while no other thread reads (the contract above).
        let word = unsafe { self.0.get().cast::<u64>().read() };
        (word & FrozenTable::TAG != 0).then_some(FrozenTable(word))
    }

    /// The map of a mutable table.
    #[inline]
    fn map(&self) -> &SymbolMap {
        // SAFETY: the contract above; the callers checked that the table is not frozen.
        unsafe { &*self.0.get() }
    }

    #[inline]
    #[expect(clippy::mut_from_ref, reason = "`FrozenCell::borrow_mut`: only the owning thread writes, while nobody reads")]
    fn map_mut(&self) -> &mut SymbolMap {
        assert!(self.frozen().is_none(), "write to a frozen binder symbol table");
        // SAFETY: the contract above.
        unsafe { &mut *self.0.get() }
    }

    /// Go `make(ast.SymbolTable)`.
    pub fn new() -> P<SymbolTable> {
        P::new(SymbolTable::default())
    }

    /// `new` in a block that a replaced binder table gave back (`release_symbol_table`), if the thread has one.
    pub fn new_recycled() -> P<SymbolTable> {
        P::new_recycled(SymbolTable::default())
    }

    /// Go `make(ast.SymbolTable, n)`.
    pub fn with_capacity(n: usize) -> P<SymbolTable> {
        P::new(SymbolTable::from_map(SymbolMap::with_capacity(n)))
    }

    /// Go `maps.Clone(table)` for a non-nil table.
    pub fn clone_table(&self) -> P<SymbolTable> {
        let map = match self.frozen() {
            None => self.map().clone(),
            Some(f) => {
                let entries = f.entries();
                let mut map = SymbolMap::with_capacity(entries.len());
                for &e in entries {
                    map.entries.push(e);
                }
                if entries.len() > FrozenTable::WIDE_FILTER_MIN {
                    map.extra = ExtraSlot(std::ptr::without_provenance_mut(FrozenTable::wide_filter(entries)));
                } else {
                    for e in entries {
                        map.extra.add_to_filter(hash_name(e.symbol().name()));
                    }
                }
                map
            }
        };
        P::new(SymbolTable::from_map(map))
    }

    /// Go `table[name]`. On a `P<SymbolTable>` receiver `table.get(name)` resolves to `P::get`, so use
    /// `table.lookup(name)` (same thing) there.
    #[inline]
    pub fn get(&self, name: &str) -> Option<P<Symbol>> {
        self.lookup(name)
    }

    #[inline]
    pub fn lookup(&self, name: &str) -> Option<P<Symbol>> {
        if let Some(f) = self.frozen() {
            return f.find(name).map(SymbolMapEntry::symbol);
        }
        let m = self.map();
        m.position(name).map(|i| m.entries[i].symbol())
    }

    /// `lookup` that also returns the stored key.
    #[inline]
    pub fn lookup_entry(&self, name: &str) -> Option<(&'static str, P<Symbol>)> {
        if let Some(f) = self.frozen() {
            return f.find(name).map(|e| (e.symbol().name(), e.symbol()));
        }
        let m = self.map();
        m.position(name).map(|i| (m.key(i), m.entries[i].symbol()))
    }

    #[inline]
    pub fn set(&self, name: &'static str, symbol: P<Symbol>) {
        self.map_mut().insert(name, symbol);
    }

    pub fn delete(&self, name: &str) {
        self.map_mut().shift_remove(name);
    }

    #[inline]
    pub fn has(&self, name: &str) -> bool {
        if let Some(f) = self.frozen() {
            return f.find(name).is_some();
        }
        self.map().position(name).is_some()
    }

    #[inline]
    pub fn len(&self) -> usize {
        if let Some(f) = self.frozen() {
            return f.entries().len();
        }
        self.map().entries.len()
    }

    /// Heap census (notes/mem-checker-heap.md): entries, entry capacity, and the heap bytes of the entry buffer
    /// plus the boxed index of a large table (a frozen table's entries are in the arena).
    pub fn heap_usage(&self) -> (usize, usize, usize) {
        if let Some(f) = self.frozen() {
            let len = f.entries().len();
            return (len, len, 0);
        }
        let m = self.map();
        let mut bytes = m.entries.capacity() * std::mem::size_of::<SymbolMapEntry>();
        if let Some(extra) = m.extra.get() {
            bytes += std::mem::size_of::<SymbolMapExtra>() + extra.odd_keys.capacity() * 24;
            if let Some(index) = &extra.index {
                let cap = index.capacity();
                let buckets = if cap < 8 { (cap + 1).next_power_of_two() } else { cap / 7 * 8 };
                bytes += (buckets * 4).div_ceil(16) * 16 + buckets + 16;
            }
        }
        (m.entries.len(), m.entries.capacity(), bytes)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        if let Some(f) = self.frozen() {
            return f.entries().is_empty();
        }
        self.map().entries.is_empty()
    }

    /// Iterates over a snapshot, so `f` may mutate the table.
    pub fn for_each(&self, mut f: impl FnMut(&'static str, P<Symbol>)) {
        for (name, symbol) in self.entries() {
            f(name, symbol);
        }
    }

    pub fn entries(&self) -> Vec<(&'static str, P<Symbol>)> {
        if let Some(f) = self.frozen() {
            return f.entries().iter().map(|e| (e.symbol().name(), e.symbol())).collect();
        }
        self.map().pairs()
    }

    pub fn keys(&self) -> Vec<&'static str> {
        if let Some(f) = self.frozen() {
            return f.entries().iter().map(|e| e.symbol().name()).collect();
        }
        let m = self.map();
        (0..m.entries.len()).map(|i| m.key(i)).collect()
    }

    pub fn values(&self) -> Vec<P<Symbol>> {
        if let Some(f) = self.frozen() {
            return f.entries().iter().map(|e| e.symbol()).collect();
        }
        self.map().entries.iter().map(|e| e.symbol()).collect()
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
    if let Some(value_declaration) = symbol.value_declaration() {
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
