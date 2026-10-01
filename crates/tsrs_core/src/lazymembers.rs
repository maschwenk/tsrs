// Port of microsoft/TypeScript#64475 (lazy member tables of instantiated class/interface references) and #64526
// (lazy members of `{ [P in keyof T]: X }` mapped types). On by default; `--noLazyMembers` or `TSRS_LAZY_MEMBERS=0`
// turns it off, which restores the reference (tsgo) behavior exactly. See notes/lazy-members.md.
//
// Follow-up candidates (notes/mem-lazy.md) each have their own switch below. A candidate only takes effect when the
// master switch is on too, so `TSRS_LAZY_MEMBERS=0` stays reference-identical whatever the candidate switches say.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

// 0: not given on the command line, 1: `--lazyMembers`, 2: `--noLazyMembers`.
static CLI: AtomicU8 = AtomicU8::new(0);

pub fn set_from_cli(on: bool) {
    CLI.store(if on { 1 } else { 2 }, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    static ENV: OnceLock<bool> = OnceLock::new();
    match CLI.load(Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => *ENV.get_or_init(|| std::env::var("TSRS_LAZY_MEMBERS").map_or(true, |v| v != "0")),
    }
}

fn env_flag(cell: &'static OnceLock<bool>, name: &str, default: bool) -> bool {
    enabled() && *cell.get_or_init(|| std::env::var(name).map_or(default, |v| v != "0"))
}

/// Candidate L1 (notes/mem-lazy.md): tuple references get lazy member tables like class/interface references.
/// `TSRS_LAZY_TUPLES=0|1`.
pub fn lazy_tuples() -> bool {
    static F: OnceLock<bool> = OnceLock::new();
    env_flag(&F, "TSRS_LAZY_TUPLES", true)
}

/// Candidate L5 (notes/mem-lazy.md): instantiating a conditional type computes its type arguments through
/// `mapTypeWithCompositeMapper(t.mapper, m)` instead of first allocating `combineTypeMappers(t.mapper, m)`, which is
/// only needed for that. `TSRS_LAZY_COND_MAPPER=0|1`.
pub fn lazy_cond_mapper() -> bool {
    static F: OnceLock<bool> = OnceLock::new();
    env_flag(&F, "TSRS_LAZY_COND_MAPPER", false)
}

/// Candidate L3 (notes/mem-lazy.md): shape-only queries (`isWeakType`, `isEmptyObjectType`,
/// `everyPropertyOfStructuredType`) on an unresolved instantiation of an anonymous type ask its target instead of
/// resolving it. `TSRS_LAZY_ANON_SHAPES=0|1`.
pub fn lazy_anon_shapes() -> bool {
    static F: OnceLock<bool> = OnceLock::new();
    env_flag(&F, "TSRS_LAZY_ANON_SHAPES", false)
}

/// Candidate L2 (notes/mem-lazy.md): `somePropertyReducesToNever` counts the property names of every constituent
/// with a lazy member table (and of unresolved anonymous instantiations, by their target) without creating the
/// members, instead of looking up the names of one skipped constituent. `TSRS_LAZY_REDUCE_NAMES=0|1`.
pub fn lazy_reduce_names() -> bool {
    static F: OnceLock<bool> = OnceLock::new();
    env_flag(&F, "TSRS_LAZY_REDUCE_NAMES", false)
}

/// Candidate L6 (notes/mem-lazy.md): union/intersection property caches are allocated on first store, and a
/// non-partial property found without the function-property augment is not copied into the augmented cache;
/// augmented lookups read it from the other cache instead. `TSRS_LAZY_PROP_CACHE=0|1`.
pub fn lazy_prop_cache() -> bool {
    static F: OnceLock<bool> = OnceLock::new();
    env_flag(&F, "TSRS_LAZY_PROP_CACHE", false)
}

/// Candidate L4 (notes/mem-lazy.md): `everyPropertyOfStructuredType` on a mapped type with a #64526 lazy table
/// creates its members one at a time, in resolution order, and stops when the predicate does.
/// `TSRS_LAZY_MAPPED_EVERY=0|1`.
pub fn lazy_mapped_every() -> bool {
    static F: OnceLock<bool> = OnceLock::new();
    env_flag(&F, "TSRS_LAZY_MAPPED_EVERY", false)
}

/// Candidate L8 (notes/mem-lazy.md): `prepareLazyMembers` does not resolve the properties of base types that
/// cannot have a lazy table (the #64475 timing mirror of `resolveObjectTypeMembers`). `TSRS_LAZY_BASE_PROPS=0|1`.
pub fn lazy_base_props() -> bool {
    static F: OnceLock<bool> = OnceLock::new();
    env_flag(&F, "TSRS_LAZY_BASE_PROPS", false)
}

/// Candidate L9 (notes/mem-lazy.md): `getUnmatchedProperties` without discriminant matching only needs to know
/// whether the source has each property, so it asks without instantiating lazy members. `TSRS_LAZY_HAS_PROP=0|1`.
pub fn lazy_has_prop() -> bool {
    static F: OnceLock<bool> = OnceLock::new();
    env_flag(&F, "TSRS_LAZY_HAS_PROP", false)
}

/// Candidate L10 (notes/mem-lazy.md): `getUnmatchedProperties` walks the target's properties through a lazy member
/// table (names and flags) instead of resolving the target, and looks up the real target property only to return
/// it or to compare discriminant types. `TSRS_LAZY_UNMATCHED=0|1`.
pub fn lazy_unmatched() -> bool {
    static F: OnceLock<bool> = OnceLock::new();
    env_flag(&F, "TSRS_LAZY_UNMATCHED", false)
}

/// Candidate L11 (notes/mem-lazy.md): "is this an empty object type" (`isEmptyObjectType`, the empty-object test in
/// `removeSubtypes`) answers from a lazy member table instead of resolving it. `TSRS_LAZY_EMPTY=0|1`.
pub fn lazy_empty() -> bool {
    static F: OnceLock<bool> = OnceLock::new();
    env_flag(&F, "TSRS_LAZY_EMPTY", false)
}

macro_rules! lazy_member_stats {
    ($($field:ident: $label:literal,)*) => {
        /// Per-checker counts of the lazy paths (only incremented when the flag is on).
        #[derive(Clone, Copy, Default, Debug)]
        pub struct LazyMemberStats {
            $(pub $field: u64,)*
        }

        impl LazyMemberStats {
            pub fn add(&mut self, o: &LazyMemberStats) {
                $(self.$field += o.$field;)*
            }

            pub fn rows(&self) -> Vec<(&'static str, u64)> {
                vec![$(($label, self.$field),)*]
            }
        }
    };
}

lazy_member_stats! {
    member_tables_created: "Lazy member tables",
    member_tables_resolved_in_full: "Lazy member tables resolved in full",
    member_table_declared_members: "Lazy table declared members",
    member_table_declared_instantiated: "Lazy table members instantiated",
    member_lookups: "Lazy member lookups",
    member_signature_queries: "Lazy signature queries",
    member_index_info_queries: "Lazy index info queries",
    member_every_property_queries: "Lazy every-property queries",
    mapped_tables_created: "Lazy mapped tables",
    mapped_tables_resolved_in_full: "Lazy mapped tables resolved in full",
    mapped_members_created: "Lazy mapped members created",
    mapped_member_lookups: "Lazy mapped member lookups",
    mapped_index_info_queries: "Lazy mapped index info queries",
    mapped_signature_early_returns: "Mapped signature early returns",
    some_property_skipped_constituents: "somePropertyReducesToNever skips",
    tuple_tables_created: "Lazy tuple tables",
    tuple_tables_resolved_in_full: "Lazy tuple tables resolved in full",
    cond_mappers_avoided: "Conditional composite mappers avoided",
    anon_shape_every_property: "Anonymous shape every-property queries",
    anon_shape_weak: "Anonymous shape isWeakType queries",
    anon_shape_empty: "Anonymous shape isEmptyObjectType queries",
    reduce_names_lazy_constituents: "somePropertyReducesToNever lazy name walks",
    prop_cache_copies_avoided: "Augmented property cache copies avoided",
    prop_cache_shared_hits: "Augmented lookups served by the other cache",
    mapped_every_property_queries: "Lazy mapped every-property queries",
    mapped_every_property_fallbacks: "Lazy mapped every-property fallbacks",
    base_props_skipped: "Base property resolutions skipped in prepare",
    has_prop_queries: "Existence-only property queries",
    has_prop_uninstantiated: "Existence answers from an uninstantiated member",
    unmatched_lazy_walks: "getUnmatchedProperties lazy target walks",
    empty_lazy_queries: "Empty-object queries answered by lazy tables",
}
