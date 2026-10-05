//! Relating a derived generic instance to a reference to one of its generic bases by variances
//! (`TSRS_DERIVED_VARIANCE`, off by default; docs/DEBUGGING.md, notes/perf-checker-algorithms.md "Row 2").
//!
//! `ZodObject<Shape>` against `ZodType<any, any, any>`: the target is a reference to `ZodType`, the source a reference
//! to `ZodObject`, whose base-type chain contains `R = ZodType<any, any, $ZodObjectInternals<Shape>>` (with the source
//! as its `this` argument, as `resolveObjectTypeMembers` builds it). TypeScript relates two references to the same
//! generic by variances, but never across a derivation, so it compares the source member by member. With the switch
//! on, `R` is first related to the target by variances; if that is True, every target member the source inherits
//! unchanged from `R` (the same member symbol) counts as related, and only the others are compared structurally,
//! followed by signatures and index signatures. It only ever answers True; anything else runs the normal comparison.
//! So it trusts TypeScript's variance digest across a derivation, and its failure mode is a missed error (a relation
//! answered True that the structural comparison answers False), never a spurious one. Three guards close the
//! disagreements found so far, each with a regression test (`testdata/regressions/derived-variance-*`):
//! - the `this` type must be measured covariant, bivariant or independent (the digest covers type parameters only);
//! - no decisions while a variance computation is running (its comparisons have marker arguments);
//! - an `any` argument in the base reference falls back when its parameter reaches the check type of a conditional
//!   type in the generic's members (`any` takes both branches where the measuring markers kept it deferred).
//!
//! `TSRS_DERIVED_VARIANCE=shadow` computes both answers, reports each disagreement on stderr and continues; the CLI
//! then exits with status 7 if there was any. `=on` uses the variance answer. Both are forced off under Go-compatible
//! history (`--checkerAssignment go`). `TSRS_DERIVED_VARIANCE_RELIABLE=params` restricts it to bases whose type
//! parameters' variances carry neither Unmeasurable nor Unreliable (`=1`: also the `this` variance);
//! Only targets with at least 16 properties are tried (`TSRS_DERIVED_VARIANCE_MIN_MEMBERS=<n>`): below that the
//! member-by-member comparison is cheaper than measuring and checking variances.
//! `TSRS_DERIVED_VARIANCE_BASES=A,B` limits it to the generic bases named A and B (`=-A,B`: all but those);
//! `TSRS_DERIVED_VARIANCE_LOG=<file>` appends one line per decision and per measured `this` variance.

use crate::*;
use std::io::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DerivedVarianceMode {
    Off,
    Shadow,
    On,
}

fn env_mode() -> DerivedVarianceMode {
    static MODE: OnceLock<DerivedVarianceMode> = OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("TSRS_DERIVED_VARIANCE").as_deref() {
        Ok("shadow") => DerivedVarianceMode::Shadow,
        Ok("on") => DerivedVarianceMode::On,
        _ => DerivedVarianceMode::Off,
    })
}

/// The mode for a checker created now: the environment's, but always off under Go-compatible history.
pub(crate) fn derived_variance_mode() -> DerivedVarianceMode {
    if tsrs_core::compat::go_compatible_history() {
        return DerivedVarianceMode::Off;
    }
    env_mode()
}

static DECISIONS: AtomicU64 = AtomicU64::new(0);
static DISAGREEMENTS: AtomicU64 = AtomicU64::new(0);

impl Checker {
    /// Shadow mode: prints the totals to stderr and returns the number of disagreements (0 in the other modes).
    pub fn derived_variance_finish() -> u64 {
        if env_mode() != DerivedVarianceMode::Shadow || tsrs_core::compat::go_compatible_history() {
            return 0;
        }
        // Relaxed: counters read once at the end, after every checker thread has been joined.
        let decisions = DECISIONS.load(Ordering::Relaxed);
        let disagreements = DISAGREEMENTS.load(Ordering::Relaxed);
        eprintln!("tsrs: TSRS_DERIVED_VARIANCE=shadow: {decisions} decisions, {disagreements} disagreements");
        disagreements
    }
}

fn no_any_arguments() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| std::env::var("TSRS_DERIVED_VARIANCE_NOANY").is_ok_and(|v| v == "1"))
}

/// 0: any variances; 1: the type parameters' variances reliable (`=params`); 2: also the `this` variance (`=1`).
fn reliable_level() -> u8 {
    static V: OnceLock<u8> = OnceLock::new();
    *V.get_or_init(|| match std::env::var("TSRS_DERIVED_VARIANCE_RELIABLE").as_deref() {
        Ok("params") => 1,
        Ok("1") => 2,
        _ => 0,
    })
}

/// `TSRS_DERIVED_VARIANCE_BASES=Name,Name` / `=-Name,Name`: only / all but these generic bases (by symbol name).
fn base_filter() -> Option<&'static (bool, Vec<String>)> {
    static FILTER: OnceLock<Option<(bool, Vec<String>)>> = OnceLock::new();
    FILTER
        .get_or_init(|| {
            let v = std::env::var("TSRS_DERIVED_VARIANCE_BASES").ok().filter(|v| !v.is_empty())?;
            let (deny, list) = match v.strip_prefix('-') {
                Some(rest) => (true, rest),
                None => (false, v.as_str()),
            };
            Some((deny, list.split(',').map(str::to_string).collect()))
        })
        .as_ref()
}

/// `TSRS_DERIVED_VARIANCE_MIN_MEMBERS=<n>` (default 16): only targets with at least n properties. Below that the
/// member-by-member comparison is cheaper than the variance route (notes/perf-derived-variance.md).
fn min_members() -> usize {
    static N: OnceLock<usize> = OnceLock::new();
    *N.get_or_init(|| std::env::var("TSRS_DERIVED_VARIANCE_MIN_MEMBERS").ok().and_then(|v| v.parse().ok()).unwrap_or(16))
}

fn log_path() -> Option<&'static str> {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(|| std::env::var("TSRS_DERIVED_VARIANCE_LOG").ok().filter(|v| !v.is_empty())).as_deref()
}

fn log_line(line: &str) {
    if let Some(path) = log_path() {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

/// What the experiment found for one (source, target) pair before the relater runs.
pub(crate) struct DerivedDecision {
    pub(crate) base_symbol: P<Symbol>,
    pub(crate) reliable: bool,
    pub(crate) this_reliable: bool,
    pub(crate) members_total: usize,
    pub(crate) members_structural: usize,
    pub(crate) covered: Vec<(P<Symbol>, P<Symbol>)>,
    pub(crate) elapsed_ns: u64,
}

impl Relater {
    /// Shadow / on mode entry, called from `structuredTypeRelatedTo` before the relater's own comparison. Returns
    /// Some(result) when the caller should return it (on mode, experiment answered True), None to run the relater.
    pub(crate) fn derived_variance_pre(&self, c: &mut Checker, source: P<Type>, target: P<Type>, report_errors: bool, intersection_state: IntersectionState) -> (Option<Ternary>, Option<DerivedDecision>) {
        if report_errors || self.rel() == c.identity_relation {
            return (None, None);
        }
        let t0 = Instant::now();
        let Some(mut decision) = self.derived_variance_decide(c, source, target, intersection_state) else { return (None, None) };
        decision.elapsed_ns = t0.elapsed().as_nanos() as u64;
        match derived_variance_mode() {
            DerivedVarianceMode::On => {
                if log_path().is_some() {
                    log_line(&format!("D {} {} {} {} {} on\n", c.symbol_to_string_exported(decision.base_symbol), decision.reliable as u8 + 2 * decision.this_reliable as u8, decision.members_total, decision.members_structural, decision.elapsed_ns));
                }
                (Some(Ternary::True), None)
            }
            _ => (None, Some(decision)),
        }
    }

    /// Shadow mode: the relater answered `result` in `base_ns`; log the decision and any disagreement.
    pub(crate) fn derived_variance_post(&self, c: &mut Checker, source: P<Type>, target: P<Type>, decision: &DerivedDecision, result: Ternary, base_ns: u64, outermost: bool) {
        // Relaxed: statistics; read once at the end.
        DECISIONS.fetch_add(1, Ordering::Relaxed);
        if log_path().is_none() && result != Ternary::False {
            return;
        }
        let base = c.symbol_to_string_exported(decision.base_symbol);
        log_line(&format!(
            "D {} {} {} {} {} {} {} {}\n",
            base,
            decision.reliable as u8 + 2 * decision.this_reliable as u8,
            decision.members_total,
            decision.members_structural,
            decision.elapsed_ns,
            base_ns,
            if result == Ternary::False { "false" } else { "true" },
            outermost as u8
        ));
        if result != Ternary::False {
            return;
        }
        // The experiment said True, the relater False: find the covered members that do not relate on their own.
        let mut culprits: Vec<String> = Vec::new();
        for &(sp, tp) in &decision.covered {
            let related = self.property_related_to(c, source, target, sp, tp, |c, s| c.get_non_missing_type_of_symbol(s), false, IntersectionState::None, self.rel() == c.comparable_relation);
            if related == Ternary::False {
                let st = c.get_type_of_symbol(sp);
                let tt = c.get_type_of_symbol(tp);
                culprits.push(format!("{}: {} vs {}", tp.name(), c.type_to_string_exported(st), c.type_to_string_exported(tt)));
            }
        }
        let s = c.type_to_string_exported(source);
        let t = c.type_to_string_exported(target);
        let line = format!("X {} | {} | {} | {}\n", base, &s[..s.len().min(300)], &t[..t.len().min(300)], culprits.join(" ; "));
        // Relaxed: see DECISIONS.
        DISAGREEMENTS.fetch_add(1, Ordering::Relaxed);
        eprint!("tsrs: TSRS_DERIVED_VARIANCE=shadow: the variance shortcut would relate what the structural comparison does not (base, source, target, members): {}", &line[2..]);
        log_line(&line);
    }

    fn derived_variance_decide(&self, c: &mut Checker, source: P<Type>, target: P<Type>, intersection_state: IntersectionState) -> Option<DerivedDecision> {
        // A variance computation measures variances through these structural comparisons (with marker type
        // arguments); answering them from variances would be circular.
        if !c.variance_stack.is_empty() {
            return None;
        }
        // Target: a non-deferred reference to a generic class or interface (not an array or tuple).
        if !is_non_deferred_type_reference(target) || is_tuple_type(target) || c.is_array_type(target) {
            return None;
        }
        let generic = target.target()?;
        if !generic.object_flags().intersects(ObjectFlags::ClassOrInterface) || generic.as_interface_type().type_parameters().is_empty() {
            return None;
        }
        if let Some((deny, names)) = base_filter() {
            let listed = generic.symbol().is_some_and(|s| names.iter().any(|n| n == s.name()));
            if listed == *deny {
                return None;
            }
        }
        // Source: a reference to a different class or interface.
        if !source.flags().intersects(TypeFlags::Object) || !is_non_deferred_type_reference(source) || source.target() == Some(generic) {
            return None;
        }
        if c.is_marker_type(source) || c.is_marker_type(target) {
            return None;
        }
        if c.get_properties_of_type(target).len() < min_members() {
            return None;
        }
        let base = self.derived_base_reference(c, source, generic, 0)?;
        let variances = c.get_variances(generic);
        if variances.is_empty() {
            return None;
        }
        // The variance digest covers the declared type parameters only; the base reference carries the source as its
        // `this` argument where the target has its own, so `this` must not be used contravariantly or invariantly.
        let this_variance = self.this_type_variance(c, generic)?;
        let this_ok = matches!(this_variance & VarianceFlags::VarianceMask, VarianceFlags::Covariant | VarianceFlags::Bivariant | VarianceFlags::Independent);
        if !this_ok {
            return None;
        }
        // "Reliable" as TypeScript uses it for two references to the same generic: the type parameters' variances;
        // `this_reliable` adds the variance of `this`, which TypeScript does not measure.
        let unreliable = |v: &VarianceFlags| v.intersects(VarianceFlags::Unmeasurable | VarianceFlags::Unreliable);
        let reliable = !variances.iter().any(unreliable);
        let this_reliable = !unreliable(&this_variance);
        if reliable_level() >= 1 && !reliable || reliable_level() >= 2 && !this_reliable {
            return None;
        }
        let base_arguments = c.get_type_arguments(base);
        let target_arguments = c.get_type_arguments(target);
        // An `any` argument resolves a conditional type on its parameter to both branches, where the markers that
        // measured the variances kept it deferred (`asElement(): T extends Node ? ElementHandle<T> : null` relates
        // `JSHandle<any>` to `JSHandle<unknown>` by variances, structurally it does not). TypeScript accepts that for
        // two references to the same generic; across a derivation it would hide an error, so such arguments fall back.
        let conditional_params = if base_arguments.iter().any(|a| a.flags().intersects(TypeFlags::Any)) { self.conditional_params(c, generic, 0) } else { Vec::new() };
        for (i, (&s, &t)) in base_arguments.iter().zip(target_arguments.iter()).enumerate() {
            if s.flags().intersects(TypeFlags::Any) && !t.flags().intersects(TypeFlags::Any) {
                let independent = variances.get(i).is_some_and(|v| *v & VarianceFlags::VarianceMask == VarianceFlags::Independent);
                if no_any_arguments() && !independent || conditional_params.get(i).copied().unwrap_or(false) {
                    return None;
                }
            }
        }
        if self.type_arguments_related_to(c, base_arguments, target_arguments, &variances, false, intersection_state) != Ternary::True {
            return None;
        }
        // Members: inherited unchanged from the base reference = the same member symbol in both.
        if c.get_unmatched_property(source, target, false, false).is_some() {
            return None;
        }
        let properties = c.get_properties_of_type(target);
        let mut covered = Vec::new();
        let mut structural = 0;
        for &tp in properties {
            if tp.flags().intersects(SymbolFlags::Prototype) {
                continue;
            }
            let Some(sp) = c.get_property_of_type(source, tp.name()) else { continue };
            if sp == tp {
                continue;
            }
            let bp = c.get_property_of_type(base, tp.name());
            if bp == Some(sp) {
                covered.push((sp, tp));
                continue;
            }
            structural += 1;
            let related = self.property_related_to(c, source, target, sp, tp, |c, s| c.get_non_missing_type_of_symbol(s), false, intersection_state, self.rel() == c.comparable_relation);
            if related != Ternary::True {
                return None;
            }
        }
        if self.signatures_related_to(c, source, target, SignatureKind::Call, false, intersection_state) != Ternary::True
            || self.signatures_related_to(c, source, target, SignatureKind::Construct, false, intersection_state) != Ternary::True
            || self.index_signatures_related_to(c, source, target, false, false, intersection_state) != Ternary::True
        {
            return None;
        }
        Some(DerivedDecision { base_symbol: generic.symbol()?, reliable, this_reliable, members_total: properties.len(), members_structural: structural, covered, elapsed_ns: 0 })
    }

    /// Which of `generic`'s type parameters reach the check type of a conditional type in its declarations: directly,
    /// through a type argument of a type alias whose body is a conditional type, or through a type argument of a
    /// base type at a position that is itself such a parameter. Cached; a cycle reads as "no".
    fn conditional_params(&self, c: &mut Checker, generic: P<Type>, depth: u32) -> Vec<bool> {
        if let Some(v) = c.derived_conditional_params.get(&generic) {
            return v.clone();
        }
        let type_parameters = generic.as_interface_type().type_parameters();
        let mut result = vec![false; type_parameters.len()];
        c.derived_conditional_params.insert(generic, result.clone());
        if depth > 8 || type_parameters.is_empty() {
            return result;
        }
        let Some(symbol) = generic.symbol() else { return result };
        let mut stack: Vec<P<Node>> = symbol.declarations().to_vec();
        while let Some(node) = stack.pop() {
            match node.kind() {
                Kind::ConditionalType => {
                    let check = node.as_conditional_type_node().check_type;
                    for (i, &tp) in type_parameters.iter().enumerate() {
                        if !result[i] && c.type_parameter_contains_reference(tp, check) {
                            result[i] = true;
                        }
                    }
                }
                Kind::TypeReference | Kind::ExpressionWithTypeArguments if !node.type_arguments().is_empty() => {
                    let referenced = if node.kind() == Kind::TypeReference {
                        c.get_symbol_from_type_reference(node)
                    } else {
                        // A heritage clause (`extends JSHandle<T>`): its type is a reference to the base generic.
                        let t = c.get_type_from_type_node(node);
                        t.target().filter(|_| t.object_flags().intersects(ObjectFlags::Reference)).and_then(|g| g.symbol()).unwrap_or(c.unknown_symbol)
                    };
                    let referenced = if referenced.flags().intersects(SymbolFlags::Alias) { c.resolve_alias(referenced) } else { referenced };
                    let arguments = node.type_arguments();
                    if referenced.flags().intersects(SymbolFlags::TypeAlias) {
                        let body = referenced.declarations().first().and_then(|d| d.type_node()).map(ast::skip_type_parentheses);
                        if body.is_some_and(|b| b.kind() == Kind::ConditionalType) {
                            for &arg in arguments {
                                for (i, &tp) in type_parameters.iter().enumerate() {
                                    if !result[i] && c.type_parameter_contains_reference(tp, arg) {
                                        result[i] = true;
                                    }
                                }
                            }
                        }
                    } else if referenced.flags().intersects(SymbolFlags::Class | SymbolFlags::Interface) {
                        let declared = c.get_declared_type_of_symbol(referenced);
                        if declared != generic && declared.object_flags().intersects(ObjectFlags::ClassOrInterface) {
                            let inner = self.conditional_params(c, declared, depth + 1);
                            for (j, &arg) in arguments.iter().enumerate() {
                                if inner.get(j).copied().unwrap_or(false) {
                                    for (i, &tp) in type_parameters.iter().enumerate() {
                                        if !result[i] && c.type_parameter_contains_reference(tp, arg) {
                                            result[i] = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            node.for_each_child(&mut |child| {
                stack.push(child);
                false
            });
        }
        c.derived_conditional_params.insert(generic, result.clone());
        result
    }

    /// The variance of `generic`'s `this` type, measured like `getVariancesWorker` measures a type parameter: compare
    /// references whose `this` argument is the sub and the super marker. Cached; None while it is being computed.
    fn this_type_variance(&self, c: &mut Checker, generic: P<Type>) -> Option<VarianceFlags> {
        if let Some(&v) = c.derived_this_variances.get(&generic) {
            return v;
        }
        let Some(_this_type) = generic.as_interface_type().this_type.get() else {
            c.derived_this_variances.insert(generic, Some(VarianceFlags::Independent));
            return Some(VarianceFlags::Independent);
        };
        c.derived_this_variances.insert(generic, None);
        let save_reliability_flags = c.reliability_flags;
        c.reliability_flags = RelationComparisonResult::None;
        let type_parameters = generic.as_interface_type().type_parameters();
        let make = |c: &mut Checker, marker: P<Type>| {
            let mut args = type_parameters.to_vec();
            args.push(marker);
            let r = c.create_type_reference(generic, &args);
            c.marker_types.add(r);
            r
        };
        let (marker_sub, marker_super, marker_other) = (c.marker_sub_type, c.marker_super_type, c.marker_other_type);
        let with_sub = make(c, marker_sub);
        let with_super = make(c, marker_super);
        let mut v = (if c.is_type_assignable_to(with_sub, with_super) { VarianceFlags::Covariant } else { VarianceFlags::Invariant })
            | (if c.is_type_assignable_to(with_super, with_sub) { VarianceFlags::Contravariant } else { VarianceFlags::Invariant });
        if v == VarianceFlags::Bivariant {
            let with_other = make(c, marker_other);
            if c.is_type_assignable_to(with_other, with_super) {
                v = VarianceFlags::Independent;
            }
        }
        if c.reliability_flags.intersects(RelationComparisonResult::ReportsUnmeasurable) {
            v |= VarianceFlags::Unmeasurable;
        }
        if c.reliability_flags.intersects(RelationComparisonResult::ReportsUnreliable) {
            v |= VarianceFlags::Unreliable;
        }
        c.reliability_flags = save_reliability_flags;
        c.derived_this_variances.insert(generic, Some(v));
        if log_path().is_some() {
            let variances = c.get_variances(generic);
            let name = generic.symbol().map(|s| c.symbol_to_string_exported(s)).unwrap_or_default();
            log_line(&format!("V {name} params {variances:?} this {v:?}\n"));
        }
        Some(v)
    }

    /// The reference to `generic` in the base-type chain of the reference `t`, instantiated the way
    /// `resolveObjectTypeMembers` instantiates base types (with `t`'s `this` argument).
    fn derived_base_reference(&self, c: &mut Checker, t: P<Type>, generic: P<Type>, depth: u32) -> Option<P<Type>> {
        if depth > 16 || !t.object_flags().intersects(ObjectFlags::Reference) {
            return None;
        }
        let source = t.target()?;
        if !source.object_flags().intersects(ObjectFlags::ClassOrInterface) {
            return None;
        }
        let (type_parameters, type_arguments) = c.get_reference_member_type_arguments(t, source);
        let this_argument = type_arguments.last().copied();
        let base_types = c.get_base_types(source);
        if base_types.is_empty() {
            return None;
        }
        let mapper = if type_parameters == type_arguments.as_slice() { None } else { Some(new_type_mapper(alloc_slice(type_parameters), alloc_slice(&type_arguments))) };
        for &base_type in base_types {
            let mut instantiated = base_type;
            if this_argument.is_some() {
                let inst = c.instantiate_type(base_type, mapper);
                instantiated = c.get_type_with_this_argument(inst, this_argument, false /*needsApparentType*/);
            }
            if instantiated.object_flags().intersects(ObjectFlags::Reference) && instantiated.target() == Some(generic) {
                return Some(instantiated);
            }
            if let Some(found) = self.derived_base_reference(c, instantiated, generic, depth + 1) {
                return Some(found);
            }
        }
        None
    }
}
