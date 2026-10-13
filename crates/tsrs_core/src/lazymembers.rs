// Port of microsoft/TypeScript#64526 (lazy members of `{ [P in keyof T]: X }` mapped types), plus independent
// follow-up candidates. Class/interface members use the shared layouts from TypeScript#64765; this master switch
// retains the old eager path for oracle comparisons. See notes/lazy-members.md and notes/shared-member-layouts.md.
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

/// Candidate L5 (notes/mem-lazy.md): instantiating a conditional type applies `t.mapper` and `m` to the outer type
/// parameters the way the composite mapper would, instead of first allocating `combineTypeMappers(t.mapper, m)`,
/// which is only needed for that. `TSRS_LAZY_COND_MAPPER=0|1`.
pub fn lazy_cond_mapper() -> bool {
    static F: OnceLock<bool> = OnceLock::new();
    env_flag(&F, "TSRS_LAZY_COND_MAPPER", true)
}

/// Candidate L6 (notes/mem-lazy.md): union/intersection property caches are allocated on first store, and a
/// non-partial property found without the function-property augment is not copied into the augmented cache;
/// augmented lookups read it from the other cache instead. `TSRS_LAZY_PROP_CACHE=0|1`.
pub fn lazy_prop_cache() -> bool {
    static F: OnceLock<bool> = OnceLock::new();
    env_flag(&F, "TSRS_LAZY_PROP_CACHE", true)
}

/// Candidate B2 (notes/mem-round3.md): an inference context's fixing and non-fixing mappers are created on first
/// use instead of with the context. `TSRS_LAZY_INFERENCE_MAPPERS=0|1`.
pub fn lazy_inference_mappers() -> bool {
    static F: OnceLock<bool> = OnceLock::new();
    env_flag(&F, "TSRS_LAZY_INFERENCE_MAPPERS", true)
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
    member_table_declared_instantiated: "Shared-layout members instantiated",
    mapped_tables_created: "Lazy mapped tables",
    mapped_tables_resolved_in_full: "Lazy mapped tables resolved in full",
    mapped_members_created: "Lazy mapped members created",
    mapped_member_lookups: "Lazy mapped member lookups",
    mapped_index_info_queries: "Lazy mapped index info queries",
    mapped_signature_early_returns: "Mapped signature early returns",
    cond_mappers_avoided: "Conditional composite mappers avoided",
    prop_cache_copies_avoided: "Augmented property cache copies avoided",
    prop_cache_shared_hits: "Augmented lookups served by the other cache",
}
