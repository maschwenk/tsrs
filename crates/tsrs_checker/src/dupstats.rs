// Cross-checker duplication census (feature `assignment-stats`, `TSRS_ASSIGNMENT_STATS=dup`; notes/mem-shared-base.md).
//
// Every object a checker created is given a checker-independent fingerprint: the data Go's `CompareTypes` orders
// types by (type flags, name symbol and alias type arguments, kind-specific data, constituent lists, mappers), with
// symbols identified by their first declaration (shared AST nodes, the same in every checker) and children by their
// own fingerprints. Two checkers that created objects with equal fingerprints created the same thing twice. Types
// created while the checker was initialized are identified by their creation index (initialization is the same
// code over the same program in every checker). Read-only over the checker; runs after checking.

use crate::*;
use tsrs_ast::Node;

/// File categories, ordered from most shared to most specific.
pub const CAT_NONE: u8 = 0;
pub const CAT_LIB: u8 = 1;
pub const CAT_NODE_MODULES: u8 = 2;
pub const CAT_WORKSPACE: u8 = 3;
pub const CAT_PROJECT: u8 = 4;

/// One created object: its fingerprint, kind (`KINDS`), estimated bytes, the category of the file that declares
/// it (its symbol or alias symbol; `CAT_NONE` without one), the most specific category among all declarations its
/// fingerprint involves (`full_cat`: an object whose `full_cat` is at most `CAT_NODE_MODULES` is built only from
/// lib and node_modules declarations), and whether the fingerprint is canonical (no inference / deferred mapper,
/// no cycle).
#[derive(Clone, Copy)]
pub struct DupRec {
    pub fp: u128,
    pub kind: u8,
    pub decl_cat: u8,
    pub full_cat: u8,
    pub canonical: bool,
    pub bytes: u32,
}

pub const KINDS: &[&str] = &[
    "type:intrinsic",
    "type:literal",
    "type:unique-symbol",
    "type:anonymous",
    "type:reference",
    "type:class/interface",
    "type:tuple-target",
    "type:instantiation-expr",
    "type:mapped",
    "type:reverse-mapped",
    "type:evolving-array",
    "type:union",
    "type:intersection",
    "type:type-parameter",
    "type:index",
    "type:indexed-access",
    "type:template-literal",
    "type:string-mapping",
    "type:substitution",
    "type:conditional",
    "symbol:instantiated",
    "symbol:mapped-member",
    "symbol:union/intersection-prop",
    "symbol:reverse-mapped",
    "symbol:other",
    "signature",
    "relation-entry",
    "init (type/symbol/signature)",
];

pub const KIND_SYMBOL_BASE: u8 = 20;
pub const KIND_SIGNATURE: u8 = 25;
pub const KIND_RELATION: u8 = 26;
pub const KIND_INIT: u8 = 27;

#[derive(Clone, Copy)]
struct Fp {
    h: u128,
    cat: u8,
    canonical: bool,
}

const IN_PROGRESS: Fp = Fp { h: 0, cat: 0, canonical: false };

struct Key {
    buf: Vec<u8>,
    cat: u8,
    canonical: bool,
}

impl Key {
    fn new(tag: &str) -> Key {
        let mut k = Key { buf: Vec::with_capacity(64), cat: CAT_NONE, canonical: true };
        k.bytes(tag.as_bytes());
        k
    }
    fn bytes(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(&(b.len() as u32).to_le_bytes());
        self.buf.extend_from_slice(b);
    }
    fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
    fn fp(&mut self, f: Fp) {
        self.buf.extend_from_slice(&f.h.to_le_bytes());
        self.cat = self.cat.max(f.cat);
        self.canonical &= f.canonical;
    }
    fn finish(self) -> Fp {
        Fp { h: xxhash_rust::xxh3::xxh3_128(&self.buf) | 1, cat: self.cat, canonical: self.canonical }
    }
}

fn addr<T>(p: P<T>) -> usize {
    (p.get() as *const T).addr()
}

pub struct Census<'a> {
    c: &'a Checker,
    cat_of_file: &'a dyn Fn(P<tsrs_ast::SourceFile>) -> u8,
    init_types: u32,
    types: FxHashMap<usize, Fp>,
    mappers: FxHashMap<usize, Fp>,
    symbols: FxHashMap<usize, Fp>,
    signatures: FxHashMap<usize, Fp>,
    node_cats: FxHashMap<usize, u8>,
}

const OBJECT_KIND_FLAGS: u32 = (1 << 19) - 1;

impl<'a> Census<'a> {
    pub fn new(c: &'a Checker, cat_of_file: &'a dyn Fn(P<tsrs_ast::SourceFile>) -> u8) -> Census<'a> {
        Census {
            c,
            cat_of_file,
            init_types: c.stats_init.0,
            types: FxHashMap::default(),
            mappers: FxHashMap::default(),
            symbols: FxHashMap::default(),
            signatures: FxHashMap::default(),
            node_cats: FxHashMap::default(),
        }
    }

    fn node_cat(&mut self, node: P<Node>) -> u8 {
        let a = addr(node);
        if let Some(&c) = self.node_cats.get(&a) {
            return c;
        }
        let cat = tsrs_ast::get_source_file_of_node(node).map_or(CAT_NONE, |f| (self.cat_of_file)(f));
        self.node_cats.insert(a, cat);
        cat
    }

    fn node(&mut self, k: &mut Key, node: Option<P<Node>>) {
        match node {
            Some(n) => {
                k.u64(addr(n) as u64);
                let cat = self.node_cat(n);
                k.cat = k.cat.max(cat);
            }
            None => k.u64(0),
        }
    }

    /// A symbol as a component of a type's identity (Go `compareSymbols`: first declaration, then name).
    fn symbol_ref(&mut self, k: &mut Key, s: Option<P<Symbol>>) {
        let Some(s) = s else {
            k.u64(0);
            return;
        };
        if let Some(&decl) = s.declarations().first() {
            k.u64(1);
            self.node(k, Some(decl));
            k.bytes(s.name().as_bytes());
        } else {
            k.u64(2);
            k.bytes(s.name().as_bytes());
            k.u64(s.flags().bits() as u64);
        }
    }

    fn type_list(&mut self, k: &mut Key, types: &[P<Type>]) {
        k.u64(types.len() as u64);
        for &t in types {
            let f = self.type_fp(t);
            k.fp(f);
        }
    }

    fn opt_type(&mut self, k: &mut Key, t: Option<P<Type>>) {
        match t {
            Some(t) => {
                let f = self.type_fp(t);
                k.fp(f);
            }
            None => k.u64(0),
        }
    }

    pub fn type_fp_pub(&mut self, t: P<Type>) -> (u128, u8, bool) {
        let f = self.type_fp(t);
        (f.h, f.cat, f.canonical)
    }

    fn type_fp(&mut self, t: P<Type>) -> Fp {
        let a = addr(t);
        if let Some(&f) = self.types.get(&a) {
            if f.h == 0 {
                // A cycle through the identity data (deferred type arguments and the like).
                return Fp { h: 0x5eed_c1c1e, cat: CAT_NONE, canonical: false };
            }
            return f;
        }
        if t.id.0 <= self.init_types {
            let mut k = Key::new("init-type");
            k.u64(t.id.0 as u64);
            let f = k.finish();
            self.types.insert(a, f);
            return f;
        }
        self.types.insert(a, IN_PROGRESS);
        let mut k = Key::new("type");
        k.u64(t.data_tag() as u64);
        k.u64(t.flags().bits() as u64);
        let object_flags = t.object_flags().bits();
        let mut identity_flags = object_flags & OBJECT_KIND_FLAGS;
        if t.flags().intersects(TypeFlags::Object) {
            identity_flags |= object_flags
                & (ObjectFlags::ContainsSpread
                    | ObjectFlags::ObjectRestType
                    | ObjectFlags::InstantiationExpressionType
                    | ObjectFlags::SingleSignatureType
                    | ObjectFlags::IsClassInstanceClone
                    | ObjectFlags::FromTypeNode)
                    .bits();
        } else if t.flags().intersects(TypeFlags::Union) {
            identity_flags |= object_flags & ObjectFlags::ContainsIntersections.bits();
        }
        k.u64(identity_flags as u64);
        if let Some(alias) = t.alias() {
            k.u64(7);
            self.symbol_ref(&mut k, alias.symbol());
            self.type_list(&mut k, alias.type_arguments());
        }
        match t.data() {
            TypeData::Intrinsic(d) => {
                k.bytes(d.intrinsic_name().as_bytes());
            }
            TypeData::Literal(d) => {
                match d.value() {
                    None => k.u64(0),
                    Some(LiteralValue::String(s)) => {
                        k.u64(1);
                        k.bytes(s.as_bytes())
                    }
                    Some(LiteralValue::Number(n)) => {
                        k.u64(2);
                        k.u64(n.0.to_bits())
                    }
                    Some(LiteralValue::Boolean(b)) => {
                        k.u64(3);
                        k.u64(b as u64)
                    }
                    Some(LiteralValue::BigInt(b)) => {
                        k.u64(4);
                        k.u64(b.negative as u64);
                        k.bytes(b.base10_value.as_bytes())
                    }
                }
                k.u64((d.regular_type() == Some(t)) as u64);
                self.symbol_ref(&mut k, t.symbol());
            }
            TypeData::UniqueESSymbol(d) => {
                k.bytes(d.name.get().as_bytes());
                self.symbol_ref(&mut k, t.symbol());
            }
            TypeData::Interface(_) => {
                self.symbol_ref(&mut k, t.symbol());
            }
            TypeData::Tuple(d) => {
                k.u64(d.readonly.get() as u64);
                k.u64(d.element_infos().len() as u64);
                for info in d.element_infos() {
                    k.u64(info.flags.bits() as u64);
                    self.node(&mut k, info.labeled_declaration);
                }
            }
            TypeData::TypeReference(d) => {
                self.opt_type(&mut k, d.target.get());
                if let Some(node) = d.node.get() {
                    k.u64(1);
                    self.node(&mut k, Some(node));
                    let m = self.mapper_fp(d.mapper.get());
                    k.fp(m);
                } else {
                    k.u64(2);
                    self.type_list(&mut k, d.resolved_type_arguments.get().unwrap_or(&[]));
                }
            }
            TypeData::Object(d) => {
                self.symbol_ref(&mut k, t.symbol());
                self.opt_type(&mut k, d.target.get());
                let m = self.mapper_fp(d.mapper.get());
                k.fp(m);
            }
            TypeData::InstantiationExpression(d) => {
                self.symbol_ref(&mut k, t.symbol());
                self.node(&mut k, d.node.get());
                self.opt_type(&mut k, d.target.get());
                let m = self.mapper_fp(d.mapper.get());
                k.fp(m);
            }
            TypeData::Mapped(d) => {
                self.symbol_ref(&mut k, t.symbol());
                self.node(&mut k, d.declaration.get());
                self.opt_type(&mut k, d.target.get());
                // Go `CompareTypes`: an instantiated mapped type's mapper starts with a fresh type parameter
                // mapping; compare the effective instantiation (the composite mapper's second half).
                let mut m = d.mapper.get();
                if let Some(mapper) = m {
                    if let TypeMapperData::Composite { m2, .. } = mapper.data() {
                        m = Some(m2);
                    }
                }
                let m = self.mapper_fp(m);
                k.fp(m);
            }
            TypeData::ReverseMapped(d) => {
                self.opt_type(&mut k, d.source.get());
                self.opt_type(&mut k, d.mapped_type.get());
                self.opt_type(&mut k, d.constraint_type.get());
            }
            TypeData::EvolvingArray(d) => {
                self.opt_type(&mut k, d.element_type.get());
                k.canonical = false;
            }
            TypeData::Union(d) => {
                if let Some(origin) = d.origin.get() {
                    k.u64(1);
                    let f = self.type_fp(origin);
                    k.fp(f);
                } else {
                    // Constituents are ordered by `CompareTypes`, whose last resort is the type id; sort the
                    // fingerprints so an id tie-break cannot make equal unions look different.
                    k.u64(2);
                    let mut fps: Vec<Fp> = d.types().iter().map(|&c| self.type_fp(c)).collect();
                    fps.sort_by_key(|f| f.h);
                    k.u64(fps.len() as u64);
                    for f in fps {
                        k.fp(f);
                    }
                }
            }
            TypeData::Intersection(d) => {
                self.type_list(&mut k, d.types());
            }
            TypeData::TypeParameter(d) => {
                self.symbol_ref(&mut k, t.symbol());
                k.u64(d.is_this_type.get() as u64);
                self.opt_type(&mut k, d.target.get());
                let m = self.mapper_fp(d.mapper.get());
                k.fp(m);
            }
            TypeData::Index(d) => {
                self.opt_type(&mut k, d.target.get());
                k.u64(d.index_flags.get().bits() as u64);
            }
            TypeData::IndexedAccess(d) => {
                self.opt_type(&mut k, d.object_type.get());
                self.opt_type(&mut k, d.index_type.get());
                k.u64(d.access_flags.get().bits() as u64);
            }
            TypeData::TemplateLiteral(d) => {
                k.u64(d.texts().len() as u64);
                for s in d.texts() {
                    k.bytes(s.as_bytes());
                }
                self.type_list(&mut k, d.types());
            }
            TypeData::StringMapping(d) => {
                self.symbol_ref(&mut k, t.symbol());
                self.opt_type(&mut k, d.target.get());
            }
            TypeData::Substitution(d) => {
                self.opt_type(&mut k, d.base_type.get());
                self.opt_type(&mut k, d.constraint.get());
            }
            TypeData::Conditional(d) => {
                self.node(&mut k, d.root.get().and_then(|r| r.node.get()));
                let m = self.mapper_fp(d.mapper.get());
                k.fp(m);
            }
        }
        let f = k.finish();
        self.types.insert(a, f);
        f
    }

    fn mapper_fp(&mut self, m: Option<P<TypeMapper>>) -> Fp {
        let Some(m) = m else {
            return Fp { h: 0x6e11, cat: CAT_NONE, canonical: true };
        };
        let a = addr(m);
        if let Some(&f) = self.mappers.get(&a) {
            if f.h == 0 {
                return Fp { h: 0x5eed_c1c1f, cat: CAT_NONE, canonical: false };
            }
            return f;
        }
        self.mappers.insert(a, IN_PROGRESS);
        let mut k = Key::new("mapper");
        match m.data() {
            TypeMapperData::Simple { source, target } => {
                k.u64(1);
                let s = self.type_fp(source);
                k.fp(s);
                let t = self.type_fp(target);
                k.fp(t);
            }
            TypeMapperData::Array { sources, targets } => {
                k.u64(2);
                self.type_list(&mut k, sources);
                self.type_list(&mut k, targets);
            }
            TypeMapperData::ArrayToSingle { sources, target } => {
                k.u64(3);
                self.type_list(&mut k, sources);
                let t = self.type_fp(target);
                k.fp(t);
            }
            TypeMapperData::Deferred { data } => {
                k.u64(4);
                self.type_list(&mut k, data.sources);
                k.u64((data as *const DeferredTypeMapper).addr() as u64);
                k.canonical = false;
            }
            TypeMapperData::Function { f } => {
                k.u64(5);
                k.u64(f as usize as u64);
            }
            TypeMapperData::Merged { m1, m2 } => {
                k.u64(6);
                let f1 = self.mapper_fp(Some(m1));
                k.fp(f1);
                let f2 = self.mapper_fp(Some(m2));
                k.fp(f2);
            }
            TypeMapperData::Composite { m1, m2 } => {
                k.u64(7);
                let f1 = self.mapper_fp(Some(m1));
                k.fp(f1);
                let f2 = self.mapper_fp(Some(m2));
                k.fp(f2);
            }
            TypeMapperData::Inference { n, fixing } => {
                k.u64(8);
                k.u64(addr(n) as u64);
                k.u64(fixing as u64);
                k.canonical = false;
            }
        }
        let f = k.finish();
        self.mappers.insert(a, f);
        f
    }

    /// A checker-created symbol as an object (what it was instantiated or synthesized from).
    fn symbol_fp(&mut self, s: P<Symbol>, index: usize) -> (Fp, u8) {
        let a = addr(s);
        if let Some(&f) = self.symbols.get(&a) {
            return (f, KIND_SYMBOL_BASE + 4);
        }
        if index < self.c.stats_init.1 {
            let mut k = Key::new("init-symbol");
            k.u64(index as u64);
            let f = k.finish();
            self.symbols.insert(a, f);
            return (f, KIND_INIT);
        }
        let check_flags = s.check_flags() & !(CheckFlags::IsDiscriminantComputed | CheckFlags::IsDiscriminant);
        let mut k = Key::new("symbol");
        k.u64(s.flags().bits() as u64);
        k.u64(check_flags.bits() as u64);
        k.bytes(s.name().as_bytes());
        let links = self.c.value_symbol_links.try_get(s);
        let kind;
        if check_flags.intersects(CheckFlags::Instantiated) {
            kind = KIND_SYMBOL_BASE;
            let target = links.and_then(|l| l.target());
            match target {
                Some(target) => {
                    let tf = self.symbol_identity(target);
                    k.fp(tf);
                }
                None => k.u64(0),
            }
            let m = self.mapper_fp(links.and_then(|l| l.mapper()));
            k.fp(m);
        } else if check_flags.intersects(CheckFlags::Mapped) {
            kind = KIND_SYMBOL_BASE + 1;
            self.opt_type(&mut k, links.and_then(|l| l.containing_type()));
            let key_type = self.c.mapped_symbol_links.try_get(s).and_then(|l| l.key_type.get());
            self.opt_type(&mut k, key_type);
        } else if check_flags.intersects(CheckFlags::Synthetic) {
            kind = KIND_SYMBOL_BASE + 2;
            self.opt_type(&mut k, links.and_then(|l| l.containing_type()));
        } else if check_flags.intersects(CheckFlags::ReverseMapped) {
            kind = KIND_SYMBOL_BASE + 3;
            if let Some(l) = self.c.reverse_mapped_symbol_links.try_get(s) {
                self.opt_type(&mut k, l.property_type.get());
                self.opt_type(&mut k, l.mapped_type.get());
                self.opt_type(&mut k, l.constraint_type.get());
            }
        } else {
            kind = KIND_SYMBOL_BASE + 4;
            self.symbol_ref(&mut k, Some(s));
            // Object literal members and the like: one per expression check, not a canonical key.
            k.canonical = s.declarations().is_empty() || !s.flags().intersects(SymbolFlags::Property);
        }
        let f = k.finish();
        self.symbols.insert(a, f);
        (f, kind)
    }

    /// A symbol as the target of an instantiation: binder symbols by declaration, checker symbols recursively.
    fn symbol_identity(&mut self, s: P<Symbol>) -> Fp {
        let mut k = Key::new("symbol-ref");
        self.symbol_ref(&mut k, Some(s));
        if s.check_flags().intersects(CheckFlags::Instantiated) {
            if let Some(l) = self.c.value_symbol_links.try_get(s) {
                let m = self.mapper_fp(l.mapper());
                k.fp(m);
            }
        }
        k.finish()
    }

    fn signature_fp(&mut self, sig: P<Signature>) -> Fp {
        let a = addr(sig);
        if let Some(&f) = self.signatures.get(&a) {
            if f.h == 0 {
                return Fp { h: 0x5eed_c1c20, cat: CAT_NONE, canonical: false };
            }
            return f;
        }
        self.signatures.insert(a, IN_PROGRESS);
        let mut k = Key::new("signature");
        k.u64(sig.flags().bits() as u64);
        self.node(&mut k, sig.declaration());
        if let Some(target) = sig.target() {
            k.u64(1);
            let tf = self.signature_fp(target);
            k.fp(tf);
            let m = self.mapper_fp(sig.mapper.get());
            k.fp(m);
        } else if let Some(composite) = sig.composite() {
            k.u64(2);
            k.u64(composite.is_union.get() as u64);
            for &s in composite.signatures.get() {
                let f = self.signature_fp(s);
                k.fp(f);
            }
        } else if sig.declaration().is_none() {
            // Synthetic signatures (union signatures, signatures of synthesized types): by parameter types.
            k.u64(3);
            self.type_list(&mut k, sig.type_parameters());
            for &p in sig.parameters() {
                k.bytes(p.name().as_bytes());
                let t = self.c.value_symbol_links.try_get(p).and_then(|l| l.resolved_type.get());
                self.opt_type(&mut k, t);
            }
            self.opt_type(&mut k, sig.resolved_return_type.get());
        }
        let f = k.finish();
        self.signatures.insert(a, f);
        f
    }

    fn type_bytes(t: P<Type>) -> usize {
        let mut bytes = t.stats_alloc_bytes();
        if let Some(st) = t.try_as_structured_type() {
            bytes += st.stats_resolved_bytes();
        }
        match t.data() {
            TypeData::Union(d) => bytes += 8 * d.types().len(),
            TypeData::Intersection(d) => bytes += 8 * d.types().len(),
            TypeData::TypeReference(d) => bytes += 8 * d.resolved_type_arguments.get().map_or(0, |a| a.len()),
            _ => {}
        }
        if let Some(alias) = t.alias() {
            bytes += 16 + 24 + 8 * alias.type_arguments().len();
        }
        bytes
    }

    fn mapper_bytes(m: Option<P<TypeMapper>>) -> usize {
        match m.map(|m| m.data()) {
            None => 0,
            Some(TypeMapperData::Array { sources, targets }) => 16 + 8 * (sources.len() + targets.len()),
            Some(_) => 16,
        }
    }

    /// Every type, symbol and signature the checker created after initialization, plus its relation cache entries.
    pub fn collect(&mut self) -> Vec<DupRec> {
        let c = self.c;
        let mut out = Vec::with_capacity(c.stats_created.0.len() + c.stats_created.1.len() + c.stats_signatures.len());
        for &t in &c.stats_created.0 {
            let f = self.type_fp(t);
            let decl_cat = match t.alias().and_then(|a| a.symbol()).or_else(|| t.symbol()).and_then(|s| s.declarations().first().copied()) {
                Some(d) => self.node_cat(d),
                None => CAT_NONE,
            };
            let kind = if t.id.0 <= self.init_types { KIND_INIT } else { t.data_tag() as u8 };
            let mapper = match t.data() {
                TypeData::Object(d) => d.mapper.get(),
                TypeData::Mapped(d) => d.mapper.get(),
                TypeData::InstantiationExpression(d) => d.mapper.get(),
                TypeData::TypeReference(d) => d.mapper.get(),
                TypeData::Conditional(d) => d.mapper.get(),
                TypeData::TypeParameter(d) => d.mapper.get(),
                _ => None,
            };
            let bytes = Self::type_bytes(t) + Self::mapper_bytes(mapper);
            out.push(DupRec { fp: f.h, kind, decl_cat, full_cat: f.cat, canonical: f.canonical, bytes: bytes as u32 });
        }
        for (index, &s) in c.stats_created.1.iter().enumerate() {
            let (f, kind) = self.symbol_fp(s, index);
            let decl_cat = match s.declarations().first() {
                Some(&d) => self.node_cat(d),
                None => CAT_NONE,
            };
            // Symbol (40 bytes) + value links record (24) when it has one + its tail when it has one.
            let mut bytes = 40usize;
            if let Some(l) = c.value_symbol_links.try_get(s) {
                bytes += 24 + l.stats_tail_bytes();
            }
            bytes += s.stats_tail_bytes();
            out.push(DupRec { fp: f.h, kind, decl_cat, full_cat: f.cat.max(decl_cat), canonical: f.canonical, bytes: bytes as u32 });
        }
        for (index, &sig) in c.stats_signatures.iter().enumerate() {
            let (f, kind) = if index < c.stats_init.2 {
                let mut k = Key::new("init-signature");
                k.u64(index as u64);
                (k.finish(), KIND_INIT)
            } else {
                (self.signature_fp(sig), KIND_SIGNATURE)
            };
            let decl_cat = match sig.declaration() {
                Some(d) => self.node_cat(d),
                None => CAT_NONE,
            };
            let bytes = 88 + 8 * (sig.parameters().len() + sig.type_parameters().len()) + Self::mapper_bytes(sig.mapper.get());
            out.push(DupRec { fp: f.h, kind, decl_cat, full_cat: f.cat.max(decl_cat), canonical: f.canonical, bytes: bytes as u32 });
        }
        self.collect_relations(&mut out);
        out
    }

    fn collect_relations(&mut self, out: &mut Vec<DupRec>) {
        let c = self.c;
        let by_id: FxHashMap<u32, P<Type>> = c.stats_created.0.iter().map(|&t| (t.id.0, t)).collect();
        let relations = [c.subtype_relation, c.strict_subtype_relation, c.assignable_relation, c.comparable_relation, c.identity_relation];
        for (ri, relation) in relations.iter().enumerate() {
            let pairs: Vec<u64> = relation.pairs.borrow().iter().copied().collect();
            for slot in pairs {
                let key = slot >> RELATION_RESULT_BITS;
                let s = (key & ((1 << 28) - 1)) as u32;
                let t = ((key >> 28) & ((1 << 28) - 1)) as u32;
                let state = key >> 56;
                let (Some(&st), Some(&tt)) = (by_id.get(&s), by_id.get(&t)) else { continue };
                let mut k = Key::new("relation");
                k.u64(ri as u64);
                k.u64(state);
                k.u64(slot & ((1 << RELATION_RESULT_BITS) - 1));
                let sf = self.type_fp(st);
                k.fp(sf);
                let tf = self.type_fp(tt);
                k.fp(tf);
                let f = k.finish();
                // An 8-byte slot in a hashbrown table at ~7/8 load, plus control byte.
                out.push(DupRec { fp: f.h, kind: KIND_RELATION, decl_cat: CAT_NONE, full_cat: f.cat, canonical: f.canonical, bytes: 10 });
            }
            let hashed = relation.hashed.borrow().len();
            if hashed > 0 {
                // `Hashed` keys (generic references, 0.3%) are not decoded; counted as unique.
                for i in 0..hashed {
                    let mut k = Key::new("relation-hashed");
                    k.u64(addr(c.assignable_relation) as u64);
                    k.u64(ri as u64);
                    k.u64(i as u64);
                    let f = k.finish();
                    out.push(DupRec { fp: f.h, kind: KIND_RELATION, decl_cat: CAT_NONE, full_cat: CAT_NONE, canonical: false, bytes: 20 });
                }
            }
        }
    }
}

/// One link store of one checker: name, bytes per record (value + slot), key kind, and per key (address or id,
/// category, shared): shared keys are binder symbols and AST nodes, the same objects in every checker.
pub struct LinkKeys {
    pub name: &'static str,
    pub record_bytes: usize,
    pub keys: Vec<(u64, u8, bool)>,
}

impl Census<'_> {
    fn node_keys(&mut self, keys: Vec<u64>) -> Vec<(u64, u8, bool)> {
        keys.into_iter()
            .map(|a| {
                // SAFETY: the key is the address of a live arena node (nodes are never freed during checking).
                let node: P<Node> = P::from_static(unsafe { &*std::ptr::with_exposed_provenance::<Node>(a as usize) });
                let cat = self.node_cat(node);
                (a, cat, !node.flags.get().intersects(tsrs_ast::NodeFlags::Synthesized))
            })
            .collect()
    }

    fn symbol_keys(&mut self, keys: Vec<u64>) -> Vec<(u64, u8, bool)> {
        keys.into_iter()
            .map(|a| {
                // SAFETY: the key is the address of a live arena symbol.
                let s: P<Symbol> = P::from_static(unsafe { &*std::ptr::with_exposed_provenance::<Symbol>(a as usize) });
                let cat = s.declarations().first().map_or(CAT_NONE, |&d| self.node_cat(d));
                (a, cat, !s.flags().intersects(SymbolFlags::Transient))
            })
            .collect()
    }

    fn id_keys(keys: Vec<u64>) -> Vec<(u64, u8, bool)> {
        // Ids are process-wide: an id in two checkers is the same binder object.
        keys.into_iter().map(|id| (id, CAT_NONE, true)).collect()
    }

    pub fn link_keys(&mut self) -> Vec<LinkKeys> {
        use std::mem::size_of;
        let c = self.c;
        let mut out = Vec::new();
        macro_rules! node_store {
            ($field:ident, $v:ty) => {{
                let keys = self.node_keys(c.$field.stats_keys());
                out.push(LinkKeys { name: stringify!($field), record_bytes: size_of::<$v>() + 13, keys });
            }};
        }
        macro_rules! symbol_store {
            ($field:ident, $v:ty) => {{
                let keys = self.symbol_keys(c.$field.stats_keys());
                out.push(LinkKeys { name: stringify!($field), record_bytes: size_of::<$v>() + 13, keys });
            }};
        }
        node_store!(node_links, NodeLinks);
        node_store!(signature_links, SignatureLinks);
        node_store!(type_node_links, TypeNodeLinks);
        node_store!(enum_member_links, EnumMemberLinks);
        node_store!(assertion_links, AssertionLinks);
        node_store!(array_literal_links, ArrayLiteralLinks);
        node_store!(switch_statement_links, SwitchStatementLinks);
        node_store!(jsx_element_links, JsxElementLinks);
        node_store!(computed_name_links, ComputedNameNodeLinks);
        symbol_store!(symbol_reference_links, SymbolReferenceLinks);
        symbol_store!(mapped_symbol_links, MappedSymbolLinks);
        symbol_store!(deferred_symbol_links, DeferredSymbolLinks);
        symbol_store!(alias_symbol_links, AliasSymbolLinks);
        symbol_store!(module_symbol_links, ModuleSymbolLinks);
        symbol_store!(late_bound_links, LateBoundLinks);
        symbol_store!(export_type_links, ExportTypeLinks);
        symbol_store!(members_and_exports_links, MembersAndExportsLinks);
        symbol_store!(type_alias_links, TypeAliasLinks);
        symbol_store!(declared_type_links, DeclaredTypeLinks);
        symbol_store!(spread_links, SpreadLinks);
        symbol_store!(variance_links, VarianceLinks);
        symbol_store!(reverse_mapped_symbol_links, ReverseMappedSymbolLinks);
        symbol_store!(marked_assignment_symbol_links, MarkedAssignmentSymbolLinks);
        symbol_store!(symbol_container_links, ContainingSymbolLinks);
        out.push(LinkKeys { name: "symbol_node_links (by node id)", record_bytes: size_of::<SymbolNodeLinks>() + 4, keys: Self::id_keys(c.symbol_node_links.stats_keys()) });
        // Value links of checker-created symbols are counted with the symbols; these are all records by symbol id.
        out.push(LinkKeys { name: "value_symbol_links (by symbol id)", record_bytes: size_of::<ValueSymbolLinks>() + 4, keys: Self::id_keys(c.value_symbol_links.stats_keys()) });
        let merged: Vec<u64> = c.merged_symbols.keys().map(|&s| addr(s) as u64).collect();
        let merged = self.symbol_keys(merged);
        out.push(LinkKeys { name: "merged_symbols (map entries)", record_bytes: 24, keys: merged });
        out
    }
}
