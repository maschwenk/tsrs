//! Heap census of one checker (`TSRS_HEAP_CENSUS=1` with `--extendedDiagnostics`; notes/mem-checker-heap.md): every
//! hash table and vector the checker owns directly, with its entry count, capacity, bytes per slot, load factor and
//! allocated bytes, printed per checker on stderr after the statistics. Containers owned by arena objects (type
//! instantiation maps, symbol tables, lazy member tables' contents) are not reachable from here; the alloc profile's
//! heap sampler attributes them by call site (`TSRS_HEAP_PROFILE=1`, `TSRS_HEAP_PROFILE_TSV`). Reporting only:
//! nothing runs unless the variable is set.

use crate::*;

/// One container (or a group of containers of one kind): entries, capacity in entries, bytes per slot, heap bytes.
#[derive(Default, Clone, Copy)]
pub struct HeapStat {
    pub containers: u64,
    pub len: u64,
    pub cap: u64,
    pub slot: u64,
    pub bytes: u64,
}

impl HeapStat {
    pub fn add(&mut self, other: HeapStat) {
        self.containers += other.containers;
        self.len += other.len;
        self.cap += other.cap;
        self.slot = self.slot.max(other.slot);
        self.bytes += other.bytes;
    }

    /// Heap bytes of a hashbrown table with `cap` items of `slot` bytes (data buckets, control bytes, group padding).
    pub fn table(len: usize, cap: usize, slot: usize) -> HeapStat {
        if cap == 0 {
            return HeapStat { containers: 1, slot: slot as u64, ..HeapStat::default() };
        }
        let buckets = if cap < 8 { (cap + 1).next_power_of_two() } else { cap / 7 * 8 };
        let data = (buckets * slot).div_ceil(16) * 16;
        HeapStat { containers: 1, len: len as u64, cap: cap as u64, slot: slot as u64, bytes: (data + buckets + 16) as u64 }
    }

    pub fn vec<T>(v: &Vec<T>) -> HeapStat {
        let slot = std::mem::size_of::<T>();
        HeapStat { containers: 1, len: v.len() as u64, cap: v.capacity() as u64, slot: slot as u64, bytes: (v.capacity() * slot) as u64 }
    }

    pub fn boxed_slice<T>(values: &[T]) -> HeapStat {
        let slot = std::mem::size_of::<T>();
        HeapStat { containers: 1, len: values.len() as u64, cap: values.len() as u64, slot: slot as u64, bytes: std::mem::size_of_val(values) as u64 }
    }
}

pub trait HeapSize {
    fn heap_stat(&self) -> HeapStat;
}

#[expect(clippy::disallowed_types, reason = "the type behind FxHashMap, measured, not used")]
impl<K, V, S> HeapSize for std::collections::HashMap<K, V, S> {
    fn heap_stat(&self) -> HeapStat {
        HeapStat::table(self.len(), self.capacity(), std::mem::size_of::<(K, V)>())
    }
}

#[expect(clippy::disallowed_types, reason = "the type behind FxHashSet, measured, not used")]
impl<T, S> HeapSize for std::collections::HashSet<T, S> {
    fn heap_stat(&self) -> HeapStat {
        HeapStat::table(self.len(), self.capacity(), std::mem::size_of::<T>())
    }
}

impl<T> HeapSize for hashbrown::HashTable<T> {
    fn heap_stat(&self) -> HeapStat {
        HeapStat::table(self.len(), self.capacity(), std::mem::size_of::<T>())
    }
}

impl<T> HeapSize for Vec<T> {
    fn heap_stat(&self) -> HeapStat {
        HeapStat::vec(self)
    }
}

impl<T: std::hash::Hash + Eq> HeapSize for Set<T> {
    fn heap_stat(&self) -> HeapStat {
        self.m.heap_stat()
    }
}

/// The rows of one checker's census.
#[derive(Default)]
pub struct HeapCensus {
    pub rows: Vec<(String, HeapStat)>,
}

impl HeapCensus {
    pub fn row(&mut self, name: &str, stat: HeapStat) {
        self.rows.push((name.to_string(), stat));
    }

    fn total(&self) -> u64 {
        self.rows.iter().map(|r| r.1.bytes).sum()
    }

    /// Rows of at least `min_bytes`, largest first, as a Markdown table.
    pub fn report(&self, title: &str, min_bytes: u64) -> String {
        use std::fmt::Write;
        let mut rows: Vec<&(String, HeapStat)> = self.rows.iter().collect();
        rows.sort_by(|a, b| b.1.bytes.cmp(&a.1.bytes).then_with(|| a.0.cmp(&b.0)));
        let mut out = String::new();
        let _ = writeln!(out, "heap census {title}: {:.1} MB in {} rows", self.total() as f64 / 1048576.0, self.rows.len());
        let _ = writeln!(out, "| owner | containers | entries | capacity | B/slot | load | MB |");
        let _ = writeln!(out, "| --- | --- | --- | --- | --- | --- | --- |");
        for (name, s) in rows.iter().filter(|r| r.1.bytes >= min_bytes) {
            let load = if s.cap > 0 { s.len as f64 / s.cap as f64 } else { 0.0 };
            let _ = writeln!(
                out,
                "| {name} | {} | {} | {} | {} | {:.2} | {:.1} |",
                s.containers,
                s.len,
                s.cap,
                s.slot,
                load,
                s.bytes as f64 / 1048576.0
            );
        }
        out
    }
}

/// Whether `TSRS_HEAP_CENSUS` is set (read once).
pub fn heap_census_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("TSRS_HEAP_CENSUS").is_ok_and(|v| !v.is_empty() && v != "0"))
}

/// `TSRS_HEAP_CENSUS_MIN` (bytes, default 1 MiB): the smallest row printed.
pub fn heap_census_min_bytes() -> u64 {
    std::env::var("TSRS_HEAP_CENSUS_MIN").ok().and_then(|s| s.parse().ok()).unwrap_or(1 << 20)
}

macro_rules! rows {
    ($census:expr_2021, $c:expr_2021, $($field:ident),* $(,)?) => {
        $( $census.row(stringify!($field), $c.$field.heap_stat()); )*
    };
}

/// A symbol table as a census row: entries, entry capacity, heap bytes.
#[cfg(feature = "assignment-stats")]
fn symbol_table_stat(t: P<SymbolTable>) -> HeapStat {
    let (len, cap, bytes) = t.heap_usage();
    HeapStat { containers: 1, len: len as u64, cap: cap as u64, slot: 8, bytes: bytes as u64 }
}

impl Checker {
    /// `--features assignment-stats` only (it records every type the checker creates): the containers owned by this
    /// checker's types: resolved member tables (with a histogram of their sizes and the unused capacity), union and
    /// intersection property caches, reference instantiation tables, conditional-root instantiation maps.
    #[cfg(feature = "assignment-stats")]
    fn heap_census_objects(&self, h: &mut HeapCensus) {
        let mut members = HeapStat::default();
        let mut by_len = [HeapStat::default(); 7]; // 0-1, 2, 3-4, 5-8, 9-16, 17-64, more
        let mut prop_caches = HeapStat::default();
        let mut references = HeapStat::default();
        let mut roots = HeapStat::default();
        let mut seen_roots: FxHashSet<P<ConditionalRoot>> = FxHashSet::default();
        let mut seen_tables: FxHashSet<P<SymbolTable>> = FxHashSet::default();
        for &t in &self.stats_created.0 {
            if let Some(st) = t.try_as_structured_type() {
                if let Some(m) = st.members().filter(|&m| seen_tables.insert(m)) {
                    let s = symbol_table_stat(m);
                    members.add(s);
                    let bucket = match s.len {
                        0..=1 => 0,
                        2 => 1,
                        3..=4 => 2,
                        5..=8 => 3,
                        9..=16 => 4,
                        17..=64 => 5,
                        _ => 6,
                    };
                    by_len[bucket].add(s);
                }
            }
            if let Some(u) = t.try_as_union_or_intersection_type() {
                for skip in [false, true] {
                    if let Some(m) = u.property_cache(skip).filter(|&m| seen_tables.insert(m)) {
                        prop_caches.add(symbol_table_stat(m));
                    }
                }
            }
            if let Some(i) = t.try_as_interface_type() {
                if let Some(s) = i.instantiations.heap_stat() {
                    references.add(s);
                }
            }
            if t.flags().intersects(TypeFlags::Conditional) {
                if let Some(root) = t.as_conditional_type().root.get().filter(|&r| seen_roots.insert(r)) {
                    if let Some(s) = root.instantiations.heap_stat() {
                        roots.add(s);
                    }
                }
            }
        }
        h.row("objects: resolved member tables of created types", members);
        for (i, name) in ["0-1", "2", "3-4", "5-8", "9-16", "17-64", ">64"].iter().enumerate() {
            h.row(&format!("objects: resolved member tables with {name} entries"), by_len[i]);
        }
        h.row("objects: union/intersection property caches", prop_caches);
        h.row("objects: reference instantiation tables", references);
        h.row("objects: conditional-root instantiation maps", roots);
    }

    pub fn heap_census_enabled() -> bool {
        heap_census_enabled()
    }

    pub fn heap_census_min_bytes() -> u64 {
        heap_census_min_bytes()
    }

    #[expect(clippy::iter_over_hash_type, reason = "sums sizes: the order does not matter")]
    pub fn heap_census(&self) -> HeapCensus {
        let mut h = HeapCensus::default();
        let slot = std::mem::size_of::<InferenceContext>() as u64;
        h.row("inference_contexts (owned records)", HeapStat {
            containers: 1,
            len: self.inference_contexts.len() as u64,
            cap: self.inference_contexts.capacity() as u64,
            slot,
            bytes: self.inference_contexts.capacity() as u64 * slot,
        });
        let slot = std::mem::size_of::<InferenceInfo>() as u64;
        h.row("inference_infos (owned records)", HeapStat {
            containers: 1,
            len: self.inference_infos.len() as u64,
            cap: self.inference_infos.capacity() as u64,
            slot,
            bytes: self.inference_infos.capacity() as u64 * slot,
        });
        let slot = std::mem::size_of::<InferenceState>() as u64;
        h.row("inference_states (owned records)", HeapStat {
            containers: 1,
            len: self.inference_states.len() as u64,
            cap: self.inference_states.capacity() as u64,
            slot,
            bytes: self.inference_states.capacity() as u64 * slot,
        });
        let slot = std::mem::size_of::<Signature>() as u64;
        h.row("signatures (owned records)", HeapStat {
            containers: 1,
            len: self.signatures.len() as u64,
            cap: self.signatures.capacity() as u64,
            slot,
            bytes: self.signatures.capacity() as u64 * slot,
        });
        let slot = std::mem::size_of::<CompositeSignature>() as u64;
        h.row("composite_signatures (owned records)", HeapStat {
            containers: 1,
            len: self.composite_signatures.len() as u64,
            cap: self.composite_signatures.capacity() as u64,
            slot,
            bytes: self.composite_signatures.capacity() as u64 * slot,
        });
        let slot = std::mem::size_of::<TypeAlias>() as u64;
        h.row("type_aliases (owned records)", HeapStat {
            containers: 1,
            len: self.type_aliases.len() as u64,
            cap: self.type_aliases.capacity() as u64,
            slot,
            bytes: self.type_aliases.capacity() as u64 * slot,
        });
        let slot = std::mem::size_of::<TypePredicate>() as u64;
        h.row("type_predicates (owned records)", HeapStat {
            containers: 1,
            len: self.type_predicates.len() as u64,
            cap: self.type_predicates.capacity() as u64,
            slot,
            bytes: self.type_predicates.capacity() as u64 * slot,
        });
        let slot = std::mem::size_of::<IndexInfo>() as u64;
        h.row("index_infos (owned records)", HeapStat {
            containers: 1,
            len: self.index_infos.len() as u64,
            cap: self.index_infos.capacity() as u64,
            slot,
            bytes: self.index_infos.capacity() as u64 * slot,
        });
        rows!(
            h, self,
            file_index_map, instantiation_stack, string_literal_types, number_literal_types,
            bigint_literal_types, enum_literal_types, enum_nan_literal_types, indexed_access_types,
            template_literal_types, string_mapping_types, unique_es_symbol_types, this_expando_kinds,
            this_expando_locations, subtype_reduction_cache, cached_types, cached_signatures, undefined_properties,
            undefined_properties_by_prop, narrowed_types, assignment_reduced_types, discriminated_contextual_types,
            instantiation_expression_types, substitution_types, reverse_mapped_cache, reverse_homomorphic_mapped_cache,
            iteration_types_cache, marker_types, resolving_explicit_type_of_symbol, unresolved_symbols, error_types,
            module_symbols, symbol_table_alias_cache, class_expression_name_tables, tuple_types, union_types,
            union_of_union_types, intersection_types, properties_types, merged_symbols, pattern_for_type,
            object_types_without_abstract_construct_signatures, structured_type_base_constraints, context_free_types,
            cached_arguments_referenced, module_import_attributes_types, flow_loop_cache, flow_node_reachable,
            flow_node_post_super, enum_relation, skip_direct_inference_nodes, active_mappers, free_type_lists, ambient_modules, reported_unreachable_nodes,
            non_existent_properties, exports_by_target_index, scratch_keyed_chain_cache, type_resolutions,
            contextual_infos, inference_context_infos, shared_flows, antecedent_types, free_inference_states,
        );
        // Values that own heap memory themselves.
        let mut arrays = HeapStat::default();
        for values in self.subtype_reduction_cache.values() {
            arrays.add(HeapStat::boxed_slice(values));
        }
        h.row("subtype_reduction_cache (owned arrays)", arrays);
        let mut arrays = HeapStat::default();
        for values in self.symbol_table_alias_cache.values() {
            arrays.add(HeapStat::boxed_slice(values));
        }
        h.row("symbol_table_alias_cache (owned arrays)", arrays);
        let mut arrays = HeapStat::default();
        for values in self.global_builtin_iterator_types_cache.iter().chain(self.global_builtin_async_iterator_types_cache.iter()) {
            let (len, cap, bytes) = values.heap_usage();
            arrays.add(HeapStat { containers: 1, len: len as u64, cap: cap as u64, slot: std::mem::size_of::<P<Type>>() as u64, bytes: bytes as u64 });
        }
        h.row("global builtin iterator caches (owned arrays)", arrays);
        let mut keys = HeapStat::default();
        for k in self.undefined_properties.keys().chain(self.unresolved_symbols.keys()) {
            keys.add(HeapStat { containers: 1, len: k.len() as u64, cap: k.capacity() as u64, slot: 1, bytes: k.capacity() as u64 });
        }
        h.row("string keys (undefined_properties, unresolved_symbols)", keys);
        let mut inner = HeapStat::default();
        for m in self.object_type_instantiations.values() {
            inner.add(m.heap_stat());
        }
        h.row("object_type_instantiations", self.object_type_instantiations.heap_stat());
        h.row("object_type_instantiations (inner maps)", inner);
        let mut inner = HeapStat::default();
        for m in self.active_type_mappers_caches.iter().chain(self.free_type_mapper_caches.iter()) {
            inner.add(m.heap_stat());
        }
        h.row("active/free type mapper caches (inner maps)", inner);
        if let Some(m) = &self.flow_type_cache {
            h.row("flow_type_cache", m.heap_stat());
        }
        // Lazy member tables: the map, the `Rc` boxes, and what each owns on the heap.
        h.row("lazy_member_tables", self.lazy_member_tables.heap_stat());
        h.row("lazy_mapped_tables", self.lazy_mapped_tables.heap_stat());
        h.rows.extend(crate::checker_09::lazy_member_tables_heap(self));
        // Relations.
        for (name, r) in [
            ("relation subtype", self.subtype_relation),
            ("relation strict_subtype", self.strict_subtype_relation),
            ("relation assignable", self.assignable_relation),
            ("relation comparable", self.comparable_relation),
            ("relation identity", self.identity_relation),
        ] {
            h.row(&format!("{name} (pairs)"), r.pairs.borrow().heap_stat());
            h.row(&format!("{name} (hashed)"), r.hashed.borrow().heap_stat());
        }
        // Link stores.
        macro_rules! links {
            ($($field:ident),* $(,)?) => {
                $( for (part, stat) in self.$field.heap_parts() { h.row(&format!("{} ({part})", stringify!($field)), stat); } )*
            };
        }
        links!(
            node_links, signature_links, symbol_node_links, type_node_links, enum_member_links, assertion_links,
            array_literal_links, switch_statement_links, jsx_element_links, computed_name_links,
            symbol_reference_links, value_symbol_links, mapped_symbol_links, deferred_symbol_links, alias_symbol_links,
            module_symbol_links, late_bound_links, export_type_links, members_and_exports_links, type_alias_links,
            declared_type_links, spread_links, variance_links, reverse_mapped_symbol_links,
            marked_assignment_symbol_links, symbol_container_links, source_file_links,
        );
        let mut inner = HeapStat::default();
        for index in self.exports_by_target_index.values() {
            inner.add(index.heap_stat());
        }
        h.row("exports_by_target_index (inner maps and lists)", inner);
        #[cfg(feature = "assignment-stats")]
        self.heap_census_objects(&mut h);
        h.row("flow_memo", self.flow_memo.heap_stat());
        h.row("infer_memo", self.infer_memo.heap_stat());
        h.row("module_export_index", self.module_export_index.heap_stat());
        h
    }
}
