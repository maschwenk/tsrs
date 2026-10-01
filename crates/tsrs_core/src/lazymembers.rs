// Port of microsoft/TypeScript#64475 (lazy member tables of instantiated class/interface references) and #64526
// (lazy members of `{ [P in keyof T]: X }` mapped types). On by default; `--noLazyMembers` or `TSRS_LAZY_MEMBERS=0`
// turns it off, which restores the reference (tsgo) behavior exactly. See notes/lazy-members.md.

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

/// Per-checker counts of the lazy paths (only incremented when the flag is on).
#[derive(Clone, Copy, Default, Debug)]
pub struct LazyMemberStats {
    pub member_tables_created: u64,
    pub member_tables_resolved_in_full: u64,
    pub member_table_declared_members: u64,
    pub member_table_declared_instantiated: u64,
    pub member_lookups: u64,
    pub member_signature_queries: u64,
    pub member_index_info_queries: u64,
    pub member_every_property_queries: u64,
    pub mapped_tables_created: u64,
    pub mapped_tables_resolved_in_full: u64,
    pub mapped_members_created: u64,
    pub mapped_member_lookups: u64,
    pub mapped_index_info_queries: u64,
    pub mapped_signature_early_returns: u64,
    pub some_property_skipped_constituents: u64,
}

impl LazyMemberStats {
    pub fn add(&mut self, o: &LazyMemberStats) {
        for (a, b) in self.fields_mut().into_iter().zip(o.rows()) {
            *a += b.1;
        }
    }

    fn fields_mut(&mut self) -> [&mut u64; 15] {
        [
            &mut self.member_tables_created,
            &mut self.member_tables_resolved_in_full,
            &mut self.member_table_declared_members,
            &mut self.member_table_declared_instantiated,
            &mut self.member_lookups,
            &mut self.member_signature_queries,
            &mut self.member_index_info_queries,
            &mut self.member_every_property_queries,
            &mut self.mapped_tables_created,
            &mut self.mapped_tables_resolved_in_full,
            &mut self.mapped_members_created,
            &mut self.mapped_member_lookups,
            &mut self.mapped_index_info_queries,
            &mut self.mapped_signature_early_returns,
            &mut self.some_property_skipped_constituents,
        ]
    }

    pub fn rows(&self) -> [(&'static str, u64); 15] {
        [
            ("Lazy member tables", self.member_tables_created),
            ("Lazy member tables resolved in full", self.member_tables_resolved_in_full),
            ("Lazy table declared members", self.member_table_declared_members),
            ("Lazy table members instantiated", self.member_table_declared_instantiated),
            ("Lazy member lookups", self.member_lookups),
            ("Lazy signature queries", self.member_signature_queries),
            ("Lazy index info queries", self.member_index_info_queries),
            ("Lazy every-property queries", self.member_every_property_queries),
            ("Lazy mapped tables", self.mapped_tables_created),
            ("Lazy mapped tables resolved in full", self.mapped_tables_resolved_in_full),
            ("Lazy mapped members created", self.mapped_members_created),
            ("Lazy mapped member lookups", self.mapped_member_lookups),
            ("Lazy mapped index info queries", self.mapped_index_info_queries),
            ("Mapped signature early returns", self.mapped_signature_early_returns),
            ("somePropertyReducesToNever skips", self.some_property_skipped_constituents),
        ]
    }
}
