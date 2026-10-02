// Census only (alloc-profile build, `TSRS_CENSUS=1`; notes/mem-overload-rollback.md): registers the checker's caches
// whose keys are object identities (type / symbol ids or pointers) as weak tables for the speculative-region census.
// An entry whose key involves an object nothing else reaches can never be looked up again (ids are never reused),
// so a rollback could drop it. Entries whose key components can be read off the entry are registered as ephemerons
// (kept while every key is reachable); the others are left fully weak, which over-approximates what could be freed
// (the census result is an upper bound). Caches keyed by values (literal types by text, undefined properties by
// name) and node-keyed links stay strong: such keys can be rebuilt at any time.

use crate::checker::Checker;
use crate::types::{ReferenceInstantiations, Type};
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use tsrs_ast::Symbol;
use tsrs_core::census_hooks as h;
use tsrs_core::P;

fn weak_map<K, V, S>(m: &std::collections::HashMap<K, V, S>) {
    if let Some((k, _)) = m.iter().next() {
        h::weak(k as *const K as usize);
    }
}

fn weak_set<T, S>(m: &std::collections::HashSet<T, S>) {
    if let Some(k) = m.iter().next() {
        h::weak(k as *const T as usize);
    }
}

/// Key components of a cached type that can be read off the type itself (`false`: not decodable).
fn type_keys(t: P<Type>, out: &mut Vec<usize>) -> bool {
    use crate::types::TypeFlags;
    let flags = t.flags();
    let ok = if flags.intersects(TypeFlags::UnionOrIntersection | TypeFlags::TemplateLiteral) {
        out.extend(t.types().iter().map(|t| t.addr()));
        true
    } else if flags.intersects(TypeFlags::IndexedAccess) {
        let ia = t.as_indexed_access_type();
        out.extend(ia.object_type().iter().chain(ia.index_type().iter()).map(|t| t.addr()));
        true
    } else if flags.intersects(TypeFlags::StringMapping) {
        out.extend(t.as_string_mapping_type().target().map(|t| t.addr()));
        true
    } else if flags.intersects(TypeFlags::Substitution) {
        let st = t.as_substitution_type();
        out.extend(st.base_type.get().iter().chain(st.constraint.get().iter()).map(|t| t.addr()));
        true
    } else {
        false
    };
    if let Some(alias) = t.alias() {
        out.extend(alias.type_arguments.get().iter().map(|t| t.addr()));
    }
    ok
}

/// The targets of a simple / array mapper (`false`: other kinds).
fn mapper_targets(m: Option<P<crate::mapper::TypeMapper>>, out: &mut Vec<usize>) -> bool {
    use crate::mapper::TypeMapperData;
    let Some(m) = m else { return false };
    match m.data() {
        TypeMapperData::Simple { target, .. } | TypeMapperData::ArrayToSingle { target, .. } => out.push(target.addr()),
        TypeMapperData::Array { targets, .. } => out.extend(targets.iter().map(|t| t.addr())),
        _ => return false,
    }
    true
}

/// Registers the entry whose value word is at `slot`: kept while `keys` are reachable. When the key could not be
/// recovered (`decoded == false`), the entry is fully weak (`fallback_weak`, an over-approximation) or strong.
fn entry(keys: &[usize], decoded: bool, _value: usize, slot: usize, len: usize, fallback_weak: bool) {
    if decoded {
        h::ephemeron(keys, slot, len);
    } else if !fallback_weak {
        h::ephemeron(&[], slot, len);
    }
}

fn census_size_of_pointee<K, V>(_: &crate::links::LinkStore<K, V>) -> usize {
    std::mem::size_of::<V>()
}

impl Checker {
    pub fn census_register_weak_tables(&self) {
        if !h::census_on() {
            return;
        }
        // Fully weak: keys are hashes of ids that the entry does not let us recover. `TSRS_CENSUS_WEAK=decoded`
        // keeps these strong (only the decoded tables below are weak), for the share the over-approximation adds.
        let all = std::env::var("TSRS_CENSUS_WEAK").as_deref() != Ok("decoded");
        if all {
            self.census_register_fully_weak_tables();
        }
        self.census_register_decoded_weak_tables(all);
        self.census_register_id_keyed_tables(all);
    }

    fn census_register_fully_weak_tables(&self) {
        weak_map(&self.subtype_reduction_cache);
        weak_map(&self.flow_loop_cache);
        weak_map(&self.error_types);
        weak_set(&self.marker_types.m);
    }

    /// Tables keyed by type ids (the census runs with one checker, whose type ids are dense).
    fn census_register_id_keyed_tables(&self, fallback_weak: bool) {
        let mut by_id: Vec<usize> = Vec::new();
        for a in h::noted(h::NOTE_TYPE) {
            // SAFETY: noted by `newType`; types are arena objects that are never freed.
            let t: &Type = unsafe { &*(a as *const Type) };
            let id = t.id.0 as usize;
            if id >= by_id.len() {
                by_id.resize(id + 1, 0);
            }
            // Several checkers: ids repeat, and an id no longer names one type (fully weak then).
            by_id[id] = if by_id[id] == 0 { a } else { usize::MAX };
        }
        let ty = |id: crate::types::TypeId| -> Option<usize> { by_id.get(id.0 as usize).copied().filter(|&a| a != 0 && a != usize::MAX) };
        macro_rules! by_ids {
            ($map:expr, $len:expr, |$k:ident| $ids:expr) => {{
                weak_map(&$map);
                for ($k, v) in &$map {
                    let ids: Vec<Option<usize>> = $ids;
                    let ok = ids.iter().all(|a| a.is_some());
                    let keys: Vec<usize> = ids.into_iter().flatten().collect();
                    entry(&keys, ok, v as *const _ as usize, v as *const _ as usize, $len, fallback_weak);
                }
            }};
        }
        by_ids!(self.cached_types, 8, |k| vec![ty(k.type_id)]);
        by_ids!(self.narrowed_types, 8, |k| vec![Some(k.t.addr()), Some(k.candidate.addr())]);
        by_ids!(self.assignment_reduced_types, 8, |k| vec![ty(k.id1), ty(k.id2)]);
        by_ids!(self.discriminated_contextual_types, 8, |k| vec![ty(k.type_id)]);
        by_ids!(self.instantiation_expression_types, 8, |k| vec![ty(k.type_id)]);
        by_ids!(self.reverse_homomorphic_mapped_cache, 8, |k| vec![ty(k.source_id), ty(k.target_id), ty(k.constraint_id)]);
        by_ids!(self.reverse_mapped_cache, std::mem::size_of::<Option<P<Type>>>(), |k| vec![ty(k.source_id), ty(k.target_id), ty(k.constraint_id)]);
        by_ids!(self.iteration_types_cache, std::mem::size_of::<crate::checker::IterationTypes>(), |k| vec![ty(k.type_id)]);
        by_ids!(self.properties_types, 8, |k| vec![ty(k.type_id)]);
        // The alias part of the key is a hash; the cached union's alias arguments stand for it.
        weak_map(&self.union_of_union_types);
        for (k, v) in &self.union_of_union_types {
            let ids = [ty(k.id1), ty(k.id2)];
            let ok = ids.iter().all(|a| a.is_some());
            let mut keys: Vec<usize> = ids.into_iter().flatten().collect();
            if let Some(alias) = v.alias() {
                keys.extend(alias.type_arguments.get().iter().map(|t| t.addr()));
            }
            entry(&keys, ok, v.addr(), v as *const P<Type> as usize, 8, fallback_weak);
        }
    }

    fn census_register_decoded_weak_tables(&self, fallback_weak: bool) {
        let mut keys: Vec<usize> = Vec::new();
        // Tables keyed by hashes of the ids of types that the cached type itself refers to: unions and intersections
        // (constituents), indexed accesses (object and index type), template literals, string mappings, substitutions.
        for m in [&self.union_types, &self.intersection_types, &self.indexed_access_types, &self.template_literal_types] {
            weak_map(m);
            for v in m.values() {
                keys.clear();
                let ok = type_keys(*v, &mut keys);
                entry(&keys, ok, v.addr(), v as *const P<Type> as usize, 8, fallback_weak);
            }
        }
        weak_map(&self.substitution_types);
        for v in self.substitution_types.values() {
            keys.clear();
            let ok = type_keys(*v, &mut keys);
            entry(&keys, ok, v.addr(), v as *const P<Type> as usize, 8, fallback_weak);
        }
        weak_map(&self.string_mapping_types);
        for (k, v) in &self.string_mapping_types {
            entry(&[k.s.addr(), k.t.addr()], true, v.addr(), v as *const P<Type> as usize, 8, fallback_weak);
        }
        // Signatures: by signature and a type list (the instantiation's mapper targets) or a constant.
        weak_map(&self.cached_signatures);
        for (k, v) in &self.cached_signatures {
            keys.clear();
            keys.push(k.sig.addr());
            use crate::checker::{SignatureKeyBase, SignatureKeyCanonical, SignatureKeyErased, SignatureKeyInner, SignatureKeyOuter};
            let special = [SignatureKeyErased, SignatureKeyCanonical, SignatureKeyBase, SignatureKeyInner, SignatureKeyOuter].contains(&k.key);
            let ok = special || mapper_targets(v.mapper.get(), &mut keys);
            entry(&keys, ok, v.addr(), v as *const P<crate::types::Signature> as usize, 8, fallback_weak);
        }
        // Anonymous / mapped type instantiations: by the generic type and the instantiation's mapper targets.
        weak_map(&self.object_type_instantiations);
        for (k, inner) in &self.object_type_instantiations {
            // The inner table itself lives while its generic type does.
            h::ephemeron(&[k.addr()], inner as *const _ as usize, std::mem::size_of_val(inner));
            weak_map(inner);
            for v in inner.values() {
                keys.clear();
                keys.push(k.addr());
                let ok = v.flags().intersects(crate::types::TypeFlags::Object) && mapper_targets(v.as_object_type().mapper.get(), &mut keys);
                entry(&keys, ok, v.addr(), v as *const P<Type> as usize, 8, fallback_weak);
            }
        }
        // Conditional type instantiations: by the instantiation's mapper targets, when the value is the deferred
        // conditional type itself.
        for addr in h::noted(h::NOTE_CONDITIONAL_INSTANTIATIONS) {
            // SAFETY: noted by `getTypeFromConditionalTypeNode` for exactly this type; arena objects are never freed.
            let m: &RefCell<FxHashMap<crate::checker::CacheHashKey, P<Type>>> = unsafe { &*(addr as *const _) };
            let m = m.borrow();
            weak_map(&m);
            for v in m.values() {
                keys.clear();
                let ok = v.flags().intersects(crate::types::TypeFlags::Conditional) && mapper_targets(v.as_conditional_type().mapper.get(), &mut keys);
                if let Some(alias) = v.alias() {
                    keys.extend(alias.type_arguments.get().iter().map(|t| t.addr()));
                }
                entry(&keys, ok, v.addr(), v as *const P<Type> as usize, 8, fallback_weak);
            }
        }
        // Tables keyed by a type.
        for m in [&self.structured_type_base_constraints, &self.object_types_without_abstract_construct_signatures] {
            weak_map(m);
            for (k, v) in m {
                h::ephemeron(&[k.addr()], v as *const P<Type> as usize, 8);
            }
        }
        weak_map(&self.pattern_for_type);
        for (k, v) in &self.pattern_for_type {
            h::ephemeron(&[k.addr()], v as *const _ as usize, 8);
        }
        // Type-keyed tables.
        weak_map(&self.lazy_member_tables);
        for (k, v) in &self.lazy_member_tables {
            h::ephemeron(&[k.addr()], v as *const _ as usize, 8);
        }
        weak_map(&self.lazy_mapped_tables);
        for (k, v) in &self.lazy_mapped_tables {
            h::ephemeron(&[k.addr()], v as *const _ as usize, 8);
        }
        // Instantiations of generic classes, interfaces and tuple targets: the key is the reference's type arguments.
        for addr in h::noted(h::NOTE_REFERENCE_INSTANTIATIONS) {
            let table = ReferenceInstantiations::census_table(addr);
            let table = table.borrow();
            for r in table.iter() {
                h::weak(r as *const P<Type> as usize);
                keys.clear();
                keys.extend(r.as_type_reference().resolved_type_arguments.get().unwrap_or(&[]).iter().map(|t| t.addr()));
                h::ephemeron(&keys, r as *const P<Type> as usize, 8);
            }
        }
        // Instantiations of generic type aliases: the key is the alias's type arguments.
        for addr in h::noted(h::NOTE_ALIAS_INSTANTIATIONS) {
            // SAFETY: noted by `getDeclaredTypeOfTypeAlias` for exactly this type.
            let m: &RefCell<FxHashMap<crate::checker::CacheHashKey, P<Type>>> = unsafe { &*(addr as *const _) };
            let m = m.borrow();
            weak_map(&m);
            for v in m.values() {
                keys.clear();
                let alias = v.alias();
                if let Some(alias) = alias {
                    keys.extend(alias.type_arguments.get().iter().map(|t| t.addr()));
                }
                entry(&keys, alias.is_some(), v.addr(), v as *const P<Type> as usize, 8, fallback_weak);
            }
        }
        // Links keyed by symbol: an entry lives while its symbol does.
        macro_rules! symbol_links {
            ($($f:ident),*) => {$(
                self.$f.census_entries(h::weak, |k, v| h::ephemeron(&[k], v, census_size_of_pointee(&self.$f)));
            )*};
        }
        symbol_links!(
            symbol_reference_links,
            mapped_symbol_links,
            deferred_symbol_links,
            alias_symbol_links,
            module_symbol_links,
            late_bound_links,
            export_type_links,
            members_and_exports_links,
            type_alias_links,
            declared_type_links,
            spread_links,
            variance_links,
            reverse_mapped_symbol_links,
            marked_assignment_symbol_links,
            symbol_container_links
        );
        // Value-symbol links (arena chunks in first-access order, found by symbol id): a slot of a checker-created
        // symbol lives while the symbol does; slots of binder symbols stay strong.
        let transient: FxHashMap<u64, usize> = h::noted(h::NOTE_TRANSIENT_SYMBOL)
            .into_iter()
            .filter_map(|a| {
                // SAFETY: noted by `newSymbol`; symbols are arena objects that are never freed.
                let s: &Symbol = unsafe { &*(a as *const Symbol) };
                let id = s.peek_id();
                (id != 0).then_some((id as u64, a))
            })
            .collect();
        let size = std::mem::size_of::<crate::types::ValueSymbolLinks>();
        self.value_symbol_links.census_slots(
            |chunk| {
                h::weak(chunk);
            },
            |id, slot| {
                match transient.get(&id) {
                    Some(&s) => h::ephemeron(&[s], slot, size),
                    None => h::ephemeron(&[], slot, size),
                }
            },
        );
    }
}
