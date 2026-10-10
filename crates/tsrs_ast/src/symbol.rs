use std::hash::BuildHasher;
use std::sync::atomic::AtomicU32;

use hashbrown::HashTable;
use rustc_hash::FxBuildHasher;
use tsrs_core::{FrozenCell, OwnedCell, OwnedPSliceCell, OwnedTaggedStrCell, PKey, P};

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
    // tsrs-only: a member list of one of the symbol's declarations that is parsed and bound on first use
    // (`lazylist`); `members` and `exports` force it first. Written by the binder of the file only.
    lazy: OwnedCell<Option<P<crate::lazylist::LazyNodeList>>>,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<Symbol>() == if tsrs_core::COMPRESSED_PTRS { 32 } else { 40 });
#[cfg(target_pointer_width = "32")]
const _: () = assert!(std::mem::size_of::<Symbol>() == 32);

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
        self.set_declarations_static(tsrs_core::alloc_slice_concat(self.declarations.get(), declarations))
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
        let t = self.tables()?;
        if let Some(lazy) = t.lazy.get() {
            lazy.ensure();
        }
        t.members.get()
    }
    #[inline]
    pub fn set_members(&self, members: Option<P<SymbolTable>>) {
        if members.is_some() || self.tables().is_some() {
            self.tables_for_write().members.set(members);
        }
    }
    #[inline]
    pub fn exports(&self) -> Option<P<SymbolTable>> {
        let t = self.tables()?;
        if let Some(lazy) = t.lazy.get() {
            if lazy.fills_exports() {
                lazy.ensure();
            }
        }
        t.exports.get()
    }

    /// The pending lazy member list of one of this symbol's declarations (`lazylist`), if any (forced or not).
    #[inline]
    pub fn lazy_list(&self) -> Option<P<crate::lazylist::LazyNodeList>> {
        self.tables().and_then(|t| t.lazy.get())
    }

    /// Binder: `list` (a member list of a declaration of this symbol) is bound on first use.
    pub fn set_lazy_list(&self, list: Option<P<crate::lazylist::LazyNodeList>>) {
        if list.is_some() || self.tables().is_some() {
            self.tables_for_write().lazy.set(list);
        }
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

#[derive(Default)]
pub struct SymbolTable(FrozenCell<SymbolMap>);

const SYMBOL_TABLE_LINEAR_MAX: usize = 16;

#[derive(Default, Clone)]
struct SymbolMap {
    entries: EntryVec,
    extra: ExtraSlot,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<SymbolMap>() == 24);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(std::mem::size_of::<SymbolMap>() == 16);

/// `Vec<SymbolMapEntry>` with a `u32` length and capacity (16 bytes instead of 24; 3.5M symbol tables on the private monorepo).
/// Grows like `Vec` (`push` doubles from 4; `reserve_exact` adds exactly).
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
        (1usize << (hash & (usize::BITS - 1))) | (1usize << ((hash >> 6) & (usize::BITS - 1)))
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

#[cfg(target_pointer_width = "64")]
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
        self.position_hashed(name, hash_name(name))
    }

    /// `position` of a name hashed beforehand (`hash` = `hash_name(name)`).
    #[inline]
    fn position_hashed(&self, name: &str, hash: u32) -> Option<usize> {
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

/// A name with its hash, for looking one name up in several tables (a property lookup's own members, then
/// `Function`'s and `Object`'s) and testing it against a `NameFilter` without hashing it again for each.
#[derive(Clone, Copy)]
pub struct HashedName<'a> {
    pub name: &'a str,
    hash: u32,
}

impl<'a> HashedName<'a> {
    #[inline]
    pub fn new(name: &'a str) -> HashedName<'a> {
        HashedName { name, hash: hash_name(name) }
    }
}

/// A 256-bit Bloom filter of the keys of a few tables taken together (two bits per key, other hash bits than a
/// table's own filter), for a lookup that tries the same tables for every name: a name it rejects is a key of none
/// of them. It holds the keys the tables had when they were added; the owner drops it when they may change.
#[derive(Clone, Copy, Default)]
pub struct NameFilter([u64; 4]);

impl NameFilter {
    #[inline]
    fn bit_positions(hash: u32) -> [u32; 2] {
        [(hash >> 12) & 255, (hash >> 20) & 255]
    }

    pub fn add_keys(&mut self, table: &SymbolTable) {
        let m = table.0.borrow();
        for i in 0..m.entries.len() {
            for b in Self::bit_positions(m.entry_hash(i)) {
                self.0[(b >> 6) as usize] |= 1 << (b & 63);
            }
        }
    }

    #[inline]
    pub fn may_contain(&self, key: HashedName<'_>) -> bool {
        Self::bit_positions(key.hash).iter().all(|&b| self.0[(b >> 6) as usize] & 1 << (b & 63) != 0)
    }
}

impl SymbolTable {
    /// `lookup` of a name hashed beforehand.
    #[inline]
    pub fn lookup_hashed(&self, key: HashedName<'_>) -> Option<P<Symbol>> {
        let m = self.0.borrow();
        m.position_hashed(key.name, key.hash).map(|i| m.entries[i].symbol())
    }

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

    /// Heap census (notes/mem-checker-heap.md): entries, entry capacity, and the heap bytes of the entry buffer
    /// plus the boxed index of a large table.
    pub fn heap_usage(&self) -> (usize, usize, usize) {
        let m = self.0.borrow();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_filter_keeps_every_key() {
        // Linear and indexed tables, keys up to and past 16 bytes, and an odd key (not its symbol's name).
        let small = SymbolTable::new();
        let large = SymbolTable::new();
        let mut names = Vec::new();
        for i in 0..200 {
            let name: &'static str = Box::leak(format!("member{i}{}", "x".repeat(i % 23)).into_boxed_str());
            names.push(name);
            let table = if i < 10 { &small } else { &large };
            table.set(name, Symbol::new(SymbolFlags::Property, name));
        }
        small.set("odd", Symbol::new(SymbolFlags::Property, "other"));
        names.push("odd");
        let mut filter = NameFilter::default();
        filter.add_keys(&small);
        filter.add_keys(&large);
        for name in names {
            assert!(filter.may_contain(HashedName::new(name)), "{name}");
        }
        assert!(!NameFilter::default().may_contain(HashedName::new("member0")));
    }
}
