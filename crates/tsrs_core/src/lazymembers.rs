// Port of microsoft/TypeScript#64475 (lazy member tables of instantiated class/interface references) and #64526
// (lazy members of `{ [P in keyof T]: X }` mapped types), with the follow-ups of notes/mem-lazy.md (lazy tuple
// tables, conditional-type instantiation without the composite mapper, union/intersection property caches without
// the eager copy, `getUnmatchedProperties` over lazy targets, empty-object tests on lazy tables) and the inference
// context mappers created on first use (notes/mem-round3.md). One switch for all of it: on by default;
// `--noLazyMembers` or `TSRS_LAZY_MEMBERS=0` turns it off, which restores the reference (tsgo) behavior exactly. See
// notes/lazy-members.md.

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
    prop_cache_copies_avoided: "Augmented property cache copies avoided",
    prop_cache_shared_hits: "Augmented lookups served by the other cache",
    has_prop_queries: "Existence-only property queries",
    has_prop_uninstantiated: "Existence answers from an uninstantiated member",
    unmatched_lazy_walks: "Lazy property-order lists requested",
    empty_lazy_queries: "Empty-object queries answered by lazy tables",
}
