//! The definitely-false test of `getConditionalType` without the relater, for the shape that dominates conditional
//! types over discriminated unions (`Extract<U, { kind: K }>` and friends, N constituents x M keys):
//! notes/perf-checker-algorithms.md, "A1".
//!
//! `isTypeAssignableTo(source, target)` with an object (or intersection) source and a non-generic, non-reference
//! object target fails in `propertiesRelatedTo` as soon as one property comparison fails. When the failing property
//! has non-structured types on both sides (a literal against another literal, a primitive, `never`, ...), that
//! comparison is `source == target || isSimpleTypeRelatedTo(source, target)` and records nothing. This module walks
//! the relater's path for that shape and calls the same checker functions in the same order (property lookups, type
//! resolution, normalization, apparent types, the relation cache probes), so every side effect that the relater would
//! have up to the failing property happens here too, in the same order. It gives up, before any side effect the
//! relater would not have, whenever the path leaves the shape (a structured property type, a weak target, variance
//! probing, a cached success, ...); the caller then runs the relater, which repeats the same (now cached) calls. When
//! it proves the relation false it records the relation cache entries the relater would have recorded, so later cache
//! probes and the complexity budget of later checks (`relationCount = (16M - size) / 8`, TS2859) see the same map.
//!
//! `TSRS_COND_PREFILTER=0` turns it off. `TSRS_VERIFY_COND_PREFILTER=<file>` runs the relater after every decision and
//! appends any disagreement (result, cache entries, instantiation / type / symbol counters) to the file.

use crate::*;
use std::io::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

pub(crate) fn cond_prefilter_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("TSRS_COND_PREFILTER").map_or(true, |v| v != "0"))
}

fn verify_path() -> Option<&'static str> {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(|| std::env::var("TSRS_VERIFY_COND_PREFILTER").ok().filter(|v| !v.is_empty())).as_deref()
}

/// The relation cache entries a proof would record, and the maybe keys of the emulated recursion.
#[derive(Default)]
struct Proof {
    entries: Vec<(RelationKey, RelationComparisonResult)>,
    maybe_keys: Vec<RelationKey>,
}

impl Checker {
    /// `isTypeAssignableTo(source, target)` as `getConditionalType` asks it (permissive instantiations of the check
    /// and extends types).
    pub(crate) fn is_conditional_extends_assignable(&mut self, source: P<Type>, target: P<Type>, distributive: bool) -> bool {
        if self.cond_prefilter && distributive && self.conditional_extends_disproved(source, target) {
            return false;
        }
        self.is_type_assignable_to(source, target)
    }

    /// True when `isTypeAssignableTo(source, target)` is false and the relation cache now holds what the relater would
    /// have recorded; false when undecided (nothing recorded; the calls made so far are a prefix of the relater's).
    fn conditional_extends_disproved(&mut self, source: P<Type>, target: P<Type>) -> bool {
        if !source.flags().intersects(TypeFlags::Object | TypeFlags::Intersection) || !is_prefilter_target(target) || source == target {
            return false;
        }
        let rel = self.assignable_relation;
        // isTypeRelatedTo. Fresh literal types are unit types, so neither side changes here.
        if self.is_simple_type_related_to(source, target, rel, None) {
            return false;
        }
        if source.flags().intersects(TypeFlags::Object) {
            let (id, _) = get_relation_key(self, source, target, IntersectionState::None, false, false);
            let related = rel.lookup(id);
            if related != RelationComparisonResult::None {
                // The relater answers from the cache without side effects.
                return !related.intersects(RelationComparisonResult::Succeeded);
            }
        }
        // checkTypeRelatedTo: the emulated recursion records at most a few entries; with this budget it cannot run out.
        if (16_000_000 - rel.size()) / 8 <= 256 {
            return false;
        }
        let mut proof = Proof::default();
        let saved_reliability = self.reliability_flags;
        let disproved = self.prefilter_is_related_to_false(source, target, IntersectionState::None, &mut proof);
        if !disproved {
            self.reliability_flags = saved_reliability;
            return false;
        }
        match verify_path() {
            None => {
                for &(id, result) in &proof.entries {
                    Relation::set(&rel, id, result);
                }
            }
            Some(path) => self.verify_prefilter_proof(path, source, target, &proof),
        }
        true
    }

    /// isRelatedToEx(source, target, RecursionFlags.Both / Source, reportErrors = false) proven False.
    fn prefilter_is_related_to_false(&mut self, original_source: P<Type>, original_target: P<Type>, intersection_state: IntersectionState, proof: &mut Proof) -> bool {
        if original_source == original_target || original_source.flags().intersects(TypeFlags::Object) && original_target.flags().intersects(TypeFlags::Primitive) {
            return false;
        }
        let source = self.get_normalized_type(original_source, false /*writing*/);
        let target = self.get_normalized_type(original_target, true /*writing*/);
        if source != original_source || target != original_target {
            return false;
        }
        let rel = self.assignable_relation;
        if self.is_simple_type_related_to(source, target, rel, None) {
            return false;
        }
        if !(source.flags().intersects(TypeFlags::StructuredOrInstantiable) || target.flags().intersects(TypeFlags::StructuredOrInstantiable)) {
            return false;
        }
        if !intersection_state.intersects(IntersectionState::Target) && is_object_literal_type(source) && source.object_flags().intersects(ObjectFlags::FreshLiteral) {
            // Excess property checks.
            return false;
        }
        // The common-property (weak type) check, evaluated in the relater's order.
        if !intersection_state.intersects(IntersectionState::Target)
            && source.flags().intersects(TypeFlags::Primitive | TypeFlags::Object | TypeFlags::Intersection)
            && source != self.global_object_type
            && target.flags().intersects(TypeFlags::Object | TypeFlags::Intersection)
            && self.is_weak_type(target)
        {
            return false;
        }
        // Neither side is a union, so the relater does not skip the cache: recursiveTypeRelatedTo.
        self.prefilter_recursive_false(source, target, intersection_state, proof)
    }

    fn prefilter_recursive_false(&mut self, source: P<Type>, target: P<Type>, intersection_state: IntersectionState, proof: &mut Proof) -> bool {
        let rel = self.assignable_relation;
        let (id, constrained) = get_relation_key(self, source, target, intersection_state, false, false);
        let entry = rel.lookup(id);
        if entry != RelationComparisonResult::None {
            if entry.intersects(RelationComparisonResult::Succeeded) {
                return false;
            }
            self.reliability_flags |= entry & (RelationComparisonResult::ReportsUnmeasurable | RelationComparisonResult::ReportsUnreliable);
            return true;
        }
        // The relater for this check starts with empty maybe stacks: only keys of this emulation can be on them.
        if proof.maybe_keys.contains(&id) {
            return false;
        }
        if constrained {
            let (broadest, _) = get_relation_key(self, source, target, intersection_state, false, true /*ignoreConstraints*/);
            if proof.maybe_keys.contains(&broadest) {
                return false;
            }
        }
        proof.maybe_keys.push(id);
        let save_reliability_flags = self.reliability_flags;
        self.reliability_flags = RelationComparisonResult::None;
        let disproved = self.prefilter_structured_false(source, target, intersection_state, proof);
        let propagating_variance_flags = self.reliability_flags;
        self.reliability_flags |= save_reliability_flags;
        proof.maybe_keys.pop();
        if !disproved {
            return false;
        }
        proof.entries.push((id, RelationComparisonResult::Failed | propagating_variance_flags));
        true
    }

    /// structuredTypeRelatedTo(source, target) proven False; `target` is a prefilter target (see below).
    fn prefilter_structured_false(&mut self, source: P<Type>, target: P<Type>, intersection_state: IntersectionState, proof: &mut Proof) -> bool {
        if source.flags().intersects(TypeFlags::Union) {
            return false;
        }
        if source.flags().intersects(TypeFlags::Intersection) {
            // unionOrIntersectionRelatedTo -> someTypeRelatedToType: no constituent may relate on its own.
            for &t in source.types() {
                if !self.prefilter_is_related_to_false(t, target, IntersectionState::Source, proof) {
                    return false;
                }
            }
        }
        // Alias variance probing.
        if source.flags().intersects(TypeFlags::Object | TypeFlags::Conditional)
            && source.alias().is_some_and(|a| !a.type_arguments().is_empty())
            && target.alias().is_some_and(|a| Some(a.symbol()) == source.alias().map(|s| s.symbol()))
        {
            return false;
        }
        if is_single_element_generic_tuple_type(source) || is_single_element_generic_tuple_type(target) {
            return false;
        }
        if source.flags().intersects(TypeFlags::TypeVariable | TypeFlags::Index | TypeFlags::Conditional | TypeFlags::TemplateLiteral | TypeFlags::StringMapping) {
            return false;
        }
        // The default arm: `target` is not a mapped type (so not a partial or generic mapped type).
        let apparent = self.get_apparent_type(source);
        if !apparent.flags().intersects(TypeFlags::Object | TypeFlags::Intersection) || self.is_array_type(target) {
            return false;
        }
        if !self.prefilter_properties_false(apparent, target) {
            return false;
        }
        // structuredTypeRelatedTo: an intersection source whose constituents have a combined constraint is compared
        // once more through that constraint.
        if source.flags().intersects(TypeFlags::Intersection) {
            let source_types = source.types().to_vec();
            if self.get_effective_constraint_of_intersection(&source_types, false /*targetIsUnion*/).is_some() {
                return false;
            }
        }
        let _ = intersection_state;
        true
    }

    /// propertiesRelatedTo(source, target) proven False under the assignable relation.
    fn prefilter_properties_false(&mut self, source: P<Type>, target: P<Type>) -> bool {
        if self.get_unmatched_property(source, target, false /*requireOptionalProperties*/, false /*matchDiscriminantProperties*/).is_some() {
            return true;
        }
        if is_object_literal_type(target) {
            return false;
        }
        let properties = self.get_properties_of_type(target);
        for &target_prop in properties {
            if target_prop.flags().intersects(SymbolFlags::Prototype) {
                continue;
            }
            let Some(source_prop) = self.get_property_of_type(source, target_prop.name()) else { continue };
            if source_prop == target_prop {
                continue;
            }
            // propertyRelatedTo: modifiers decide before types do.
            let modifiers = get_declaration_modifier_flags_from_symbol(source_prop) | get_declaration_modifier_flags_from_symbol(target_prop);
            if modifiers.intersects(ModifierFlags::Private | ModifierFlags::Protected) {
                return false;
            }
            // isPropertySymbolTypeRelated.
            let target_is_optional = self.strict_null_checks && target_prop.check_flags().intersects(CheckFlags::Partial);
            let non_missing_target = self.get_non_missing_type_of_symbol(target_prop);
            let effective_target = self.add_optionality_ex(non_missing_target, false /*isProperty*/, target_is_optional);
            if !effective_target.flags().intersects(TypeFlags::AnyOrUnknown) {
                let effective_source = self.get_non_missing_type_of_symbol(source_prop);
                match self.prefilter_simple_related(effective_source, effective_target) {
                    Some(true) => {}
                    Some(false) => return true,
                    None => return false,
                }
            }
            if source_prop.flags().intersects(SymbolFlags::Optional) && target_prop.flags().intersects(SymbolFlags::ClassMember) && !target_prop.flags().intersects(SymbolFlags::Optional) {
                return true;
            }
        }
        // Every property relates: the relater goes on to signatures and index infos.
        false
    }

    /// isRelatedToEx(source, target) for two types that are not structured or instantiable: no relation cache entry,
    /// no side effect. None for anything else.
    fn prefilter_simple_related(&mut self, source: P<Type>, target: P<Type>) -> Option<bool> {
        if source == target {
            return Some(true);
        }
        if (source.flags() | target.flags()).intersects(TypeFlags::StructuredOrInstantiable) {
            return None;
        }
        let s = if is_fresh_literal_type(source) { source.as_literal_type().regular_type.get().unwrap() } else { source };
        let t = if is_fresh_literal_type(target) { target.as_literal_type().regular_type.get().unwrap() } else { target };
        if s == t {
            return Some(true);
        }
        let rel = self.assignable_relation;
        Some(self.is_simple_type_related_to(s, t, rel, None))
    }

    #[cold]
    fn verify_prefilter_proof(&mut self, path: &str, source: P<Type>, target: P<Type>, proof: &Proof) {
        static DECISIONS: AtomicU64 = AtomicU64::new(0);
        // Relaxed: a statistics counter; the log lines carry no ordering guarantee.
        let n = DECISIONS.fetch_add(1, Ordering::Relaxed) + 1;
        let rel = self.assignable_relation;
        let before: Vec<RelationComparisonResult> = proof.entries.iter().map(|&(id, _)| rel.lookup(id)).collect();
        let (size, instantiations, types, symbols) = (rel.size(), self.total_instantiation_count, self.type_count, self.symbol_count);
        let related = self.is_type_assignable_to(source, target);
        let mut problems: Vec<String> = Vec::new();
        if related {
            problems.push("relater says assignable".to_string());
        }
        let new_entries = before.iter().filter(|&&b| b == RelationComparisonResult::None).count() as i32;
        if rel.size() - size != new_entries {
            problems.push(format!("relation size +{} (expected +{new_entries})", rel.size() - size));
        }
        for (i, &(id, expected)) in proof.entries.iter().enumerate() {
            let now = rel.lookup(id);
            let want = if before[i] == RelationComparisonResult::None { expected } else { before[i] };
            if now != want {
                problems.push(format!("entry {i}: {now:?}, expected {want:?}"));
            }
        }
        if self.total_instantiation_count != instantiations || self.type_count != types || self.symbol_count != symbols {
            problems.push(format!(
                "counters moved: instantiations +{}, types +{}, symbols +{}",
                self.total_instantiation_count - instantiations,
                self.type_count - types,
                self.symbol_count - symbols
            ));
        }
        if problems.is_empty() && n > 64 && n % 4096 != 1 {
            return;
        }
        let line = if problems.is_empty() {
            format!("ok {n} decisions\n")
        } else {
            let s = self.type_to_string_exported(source);
            let t = self.type_to_string_exported(target);
            format!("MISMATCH after {n} decisions: {} | {} | {}\n", problems.join("; "), &s[..s.len().min(400)], &t[..t.len().min(400)])
        };
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

/// A target the prefilter handles: a non-generic object type that is not a reference (no variance comparison), not
/// mapped, reverse mapped or a tuple/array.
fn is_prefilter_target(t: P<Type>) -> bool {
    t.flags().intersects(TypeFlags::Object)
        && !t.object_flags().intersects(ObjectFlags::Reference | ObjectFlags::Mapped | ObjectFlags::ReverseMapped | ObjectFlags::InstantiationExpressionType)
}
