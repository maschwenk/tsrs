//! Relating a derived generic instance to a reference to one of its generic bases by variances
//! (`TSRS_DERIVED_VARIANCE`, off by default; docs/DEBUGGING.md, notes/perf-checker-algorithms.md "Row 2").
//!
//! `ZodObject<Shape>` against `ZodType<any, any, any>`: the target is a reference to `ZodType`, the source a reference
//! to `ZodObject`, whose base-type chain contains `R = ZodType<any, any, $ZodObjectInternals<Shape>>` (with the source
//! as its `this` argument, as `resolveObjectTypeMembers` builds it). TypeScript relates two references to the same
//! generic by variances, but never across a derivation, so it compares the source member by member. With the switch
//! on, `R` is first related to the target by variances; if that holds, every target member the source inherits
//! unchanged from `R` (the same member symbol) counts as related, and only the others are compared structurally,
//! followed by signatures and index signatures. It answers True (or Maybe, when a member comparison meets a pair
//! already being compared) or falls back to the normal comparison. So it trusts TypeScript's variance digest across a
//! derivation, and its failure mode is a missed error, never a spurious one. Six guards close the disagreements found
//! so far, each with a regression test (`testdata/regressions/derived-variance-*`, notes/fuzz-derived-variance.md):
//! 1. the `this` type must be measured covariant, bivariant or independent (the digest covers type parameters only);
//! 2. no decisions while a variance computation is running (its comparisons have marker arguments);
//! 3. an `any` argument in the base reference falls back when its parameter reaches the check type of a conditional
//!    type in the generic's members (`any` takes both branches where the measuring markers kept it deferred);
//! 4. an inherited member counts as related only if its declaration is monotone: no type parameter or `this` under
//!    keyof, a conditional, mapped, indexed access, template literal or intersection type, or in a rest parameter
//!    (directly or through a type alias), where the markers' answer and the real arguments' answer differ; other members are compared
//!    structurally, unless every such slot is `any` / `unknown` in the target (a property type, a method's return or
//!    parameter type). This subsumes guard 3;
//! 5. `in` / `out` annotations are verified with markers before the variances they imply are trusted (TypeScript
//!    does not measure them, and under skipLibCheck never reports a wrong one in a declaration file);
//! 6. no decisions under the strict subtype relation when an argument contains `void` (a trailing `void` parameter
//!    is optional for its arity check).
//!
//! `TSRS_DERIVED_VARIANCE=shadow` computes both answers, reports each disagreement on stderr and continues; the CLI
//! then exits with status 7 if there was any. `=on` uses the variance answer. Both are forced off under Go-compatible
//! history (`--checkerAssignment go`). `TSRS_DERIVED_VARIANCE_RELIABLE=params` restricts it to bases whose type
//! parameters' variances carry neither Unmeasurable nor Unreliable (`=1`: also the `this` variance);
//! Only targets with at least 16 properties are tried (`TSRS_DERIVED_VARIANCE_MIN_MEMBERS=<n>`): below that the
//! member-by-member comparison is cheaper than measuring and checking variances.
//! `TSRS_DERIVED_VARIANCE_BASES=A,B` limits it to the generic bases named A and B (`=-A,B`: all but those);
//! `TSRS_DERIVED_VARIANCE_LOG=<file>` appends one line per decision, per measured `this` variance and per member
//! compared structurally; `TSRS_DERIVED_VARIANCE_NO_GUARD=4,5,6` switches guards 4-6 off (tests and measurements).

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

/// `TSRS_DERIVED_VARIANCE_NO_GUARD=4,5`: switch guards off, to measure them and to test that their cases fail.
fn guard_enabled(n: u8) -> bool {
    static OFF: OnceLock<Vec<u8>> = OnceLock::new();
    !OFF.get_or_init(|| std::env::var("TSRS_DERIVED_VARIANCE_NO_GUARD").map(|v| v.split(',').filter_map(|x| x.trim().parse().ok()).collect()).unwrap_or_default()).contains(&n)
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
    /// True, or Maybe when a member comparison met a pair already being compared (as the relater combines them).
    pub(crate) result: Ternary,
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
                (Some(decision.result), None)
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
        // Guard 5: TypeScript takes `in` / `out` annotations as the variances without measuring them, and reports a
        // wrong one only where it checks the declaration (never under skipLibCheck for a .d.ts). Across a derivation
        // a wrong annotation would hide an error, so the annotations must hold for the check markers.
        if guard_enabled(5) && !self.annotations_hold(c, generic) {
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
        // Guard 6: under the strict subtype relation (union reduction) a trailing parameter whose type contains `void`
        // is optional for the arity check (`getMinArgumentCount`), so `(x: void) => void` is not a strict subtype of
        // `(x: unknown) => void`; the markers never are `void`, so the digest cannot see it. A tuple argument spreads into
        // a rest parameter (`m(...a: T)` with `T = [void]`), so tuple and array elements count too.
        if guard_enabled(6) && self.rel() == c.strict_subtype_relation && base_arguments.iter().chain(target_arguments.iter()).any(|&a| reaches_void(c, a, 0)) {
            return None;
        }
        let mut result = self.type_arguments_related_to(c, base_arguments, target_arguments, &variances, false, intersection_state);
        if result == Ternary::False {
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
            // Guard 4: the variance digest speaks for a member only where its type is monotone in the type
            // parameters and `this`; a member that puts them under keyof, a conditional, a mapped, indexed access,
            // template literal or intersection type (directly or through a type alias) is compared structurally.
            if bp == Some(sp) && !(guard_enabled(4) && self.member_sensitive(c, sp) && !top_target_slots(c, sp, tp)) {
                covered.push((sp, tp));
                continue;
            }
            structural += 1;
            if log_path().is_some() {
                let why = if bp == Some(sp) { "sensitive" } else { "declared" };
                log_line(&format!("M {} {} {}\n", generic.symbol().map(|s| s.name()).unwrap_or_default(), tp.name(), why));
            }
            let related = self.property_related_to(c, source, target, sp, tp, |c, s| c.get_non_missing_type_of_symbol(s), false, intersection_state, self.rel() == c.comparable_relation);
            if related == Ternary::False {
                return None;
            }
            result &= related;
        }
        for kind in [SignatureKind::Call, SignatureKind::Construct] {
            let related = self.signatures_related_to(c, source, target, kind, false, intersection_state);
            if related == Ternary::False {
                return None;
            }
            result &= related;
        }
        let related = self.index_signatures_related_to(c, source, target, false, false, intersection_state);
        if related == Ternary::False {
            return None;
        }
        result &= related;
        Some(DerivedDecision { base_symbol: generic.symbol()?, reliable, this_reliable, members_total: properties.len(), members_structural: structural, covered, elapsed_ns: 0, result })
    }

    /// Guard 5: every `in` / `out` annotation of `generic`'s type parameters holds when checked the way
    /// `checkTypeParameterDeferred` checks it (with markers of its own), silently. Cached.
    fn annotations_hold(&self, c: &mut Checker, generic: P<Type>) -> bool {
        if let Some(&ok) = c.derived_annotations_ok.get(&generic) {
            return ok;
        }
        // No decisions for this generic while its annotations are being checked.
        c.derived_annotations_ok.insert(generic, false);
        let Some(symbol) = generic.symbol() else { return true };
        let mut ok = true;
        for &tp in generic.as_interface_type().type_parameters() {
            let modifiers = c.get_type_parameter_modifiers(tp) & (ModifierFlags::In | ModifierFlags::Out);
            if modifiers != ModifierFlags::In && modifiers != ModifierFlags::Out {
                continue;
            }
            let out = modifiers == ModifierFlags::Out;
            let (sup, sub) = match c.derived_annotation_markers {
                Some(m) => m,
                None => {
                    let sup = c.new_type_parameter(None);
                    let sub = c.new_type_parameter(None);
                    sub.as_type_parameter().constraint.set(Some(sup));
                    c.derived_annotation_markers = Some((sup, sub));
                    (sup, sub)
                }
            };
            let source = c.create_marker_type(symbol, tp, if out { sub } else { sup });
            let target = c.create_marker_type(symbol, tp, if out { sup } else { sub });
            if !c.is_type_assignable_to(source, target) {
                ok = false;
                break;
            }
        }
        c.derived_annotations_ok.insert(generic, ok);
        ok
    }

    /// Guard 4: whether a member's declarations use a type parameter (other than a mapped type's key or an `infer`
    /// variable) or `this` under a type operator whose result is not monotone in assignability: keyof / unique,
    /// conditional types, mapped types, indexed access, template literal types, intersections, `infer`, type queries,
    /// or an intrinsic string mapping; type aliases are followed into their bodies. References to classes and
    /// interfaces are not followed: two instantiations of one are related by its own variances on both routes.
    /// A declaration whose type is inferred (no annotation) counts as sensitive. Cached per declaration.
    fn member_sensitive(&self, c: &mut Checker, prop: P<Symbol>) -> bool {
        let declarations = prop.declarations();
        if declarations.is_empty() {
            return true;
        }
        for &d in declarations {
            if let Some(&v) = c.derived_sensitive_decls.get(&d) {
                if v {
                    return true;
                }
                continue;
            }
            let mut aliases = Vec::new();
            let v = sensitive_declaration(c, d, &mut aliases);
            c.derived_sensitive_decls.insert(d, v);
            if v {
                return true;
            }
        }
        false
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

/// Guard 6: whether `void` is a constituent of `t`, or of an element of a tuple or array constituent of `t`.
fn reaches_void(c: &mut Checker, t: P<Type>, depth: u32) -> bool {
    some_type(c, t, |c, t| {
        if t.flags().intersects(TypeFlags::Void) {
            return true;
        }
        if depth < 4 && (is_tuple_type(t) || c.is_array_or_tuple_type(t)) {
            let arguments = c.get_type_arguments(t);
            return arguments.iter().any(|&a| reaches_void(c, a, depth + 1));
        }
        false
    })
}

/// Guard 4's exception: a sensitive member still counts as related when every sensitive slot of its declaration is
/// `any` or `unknown` in the target, and the slot is one where any source type relates to that: a property's type,
/// a method's return type, or a method parameter (compared bivariantly, so the source -> target direction suffices).
/// The method must have one declaration, no type parameters and no `this` parameter; its other slots are monotone,
/// so the variance digest covers them. (Zod's `parse(): core.output<this>` against `ZodType<any, any, any>`.)
fn top_target_slots(c: &mut Checker, sp: P<Symbol>, tp: P<Symbol>) -> bool {
    let [d] = sp.declarations() else { return false };
    let d = *d;
    let top = |t: P<Type>| t.flags().intersects(TypeFlags::AnyOrUnknown);
    match d.kind() {
        Kind::PropertySignature | Kind::PropertyDeclaration => {
            let t = c.get_type_of_symbol(tp);
            top(t)
        }
        Kind::MethodSignature | Kind::MethodDeclaration => {
            if !d.type_parameters().is_empty() {
                return false;
            }
            let target_type = c.get_type_of_symbol(tp);
            let &[sig] = c.get_signatures_of_type(target_type, SignatureKind::Call) else { return false };
            let params = d.parameters();
            if sig.this_parameter().is_some() || !sig.type_parameters().is_empty() || sig.parameters().len() != params.len() {
                return false;
            }
            let mut aliases = Vec::new();
            for (i, &p) in params.iter().enumerate() {
                match p.type_node() {
                    Some(t) => {
                        if sensitive_type(c, t, is_rest(p), &[], &mut aliases) {
                            let pt = c.get_type_of_symbol(sig.parameters()[i]);
                            if !top(pt) {
                                return false;
                            }
                        }
                    }
                    None => {
                        if p.initializer().is_some() {
                            return false;
                        }
                    }
                }
            }
            match d.type_node() {
                None => d.body().is_none(),
                Some(r) => {
                    if !sensitive_type(c, r, false, &[], &mut aliases) {
                        return true;
                    }
                    if r.kind() == Kind::TypePredicate {
                        return false;
                    }
                    let rt = c.get_return_type_of_signature(sig);
                    top(rt)
                }
            }
        }
        _ => false,
    }
}

fn is_rest(p: P<Node>) -> bool {
    p.kind() == Kind::Parameter && p.as_parameter_declaration().dot_dot_dot_token().is_some()
}

/// Guard 4 walker for one member declaration (see `member_sensitive`).
fn sensitive_declaration(c: &mut Checker, d: P<Node>, aliases: &mut Vec<P<Symbol>>) -> bool {
    match d.kind() {
        Kind::PropertySignature | Kind::PropertyDeclaration => match d.type_node() {
            Some(t) => sensitive_type(c, t, false, &[], aliases),
            None => d.kind() == Kind::PropertyDeclaration,
        },
        Kind::MethodSignature | Kind::MethodDeclaration | Kind::GetAccessor | Kind::SetAccessor => {
            if d.type_node().is_none() && d.body().is_some() && d.kind() != Kind::SetAccessor {
                return true;
            }
            for &tp in d.type_parameters() {
                if sensitive_type(c, tp, false, &[], aliases) {
                    return true;
                }
            }
            for &p in d.parameters() {
                match p.type_node() {
                    Some(t) => {
                        if sensitive_type(c, t, is_rest(p), &[], aliases) {
                            return true;
                        }
                    }
                    None => {
                        if p.initializer().is_some() {
                            return true;
                        }
                    }
                }
            }
            d.type_node().is_some_and(|t| sensitive_type(c, t, false, &[], aliases))
        }
        // Parameter properties, enum members, JS declarations and the rest: not analysed.
        _ => true,
    }
}

/// Whether `node` (a type node or a part of one) references a variable under a non-monotone operator; `under` says
/// whether an enclosing node already is one. Variables: `this`, class / interface / signature type parameters, and
/// the type parameters in `alias_vars` (those of the type alias being walked whose arguments mention a variable).
fn sensitive_type(c: &mut Checker, node: P<Node>, under: bool, alias_vars: &[P<Symbol>], aliases: &mut Vec<P<Symbol>>) -> bool {
    let mut under_here = under;
    match node.kind() {
        Kind::ThisType => return under,
        // `typeof this.x` depends on `this` through an expression, not a `this` type node.
        Kind::TypeQuery if tsrs_ast::is_this_identifier(tsrs_ast::get_first_identifier(node.as_type_query_node().expr_name)) => return true,
        Kind::ConditionalType | Kind::IndexedAccessType | Kind::MappedType | Kind::TemplateLiteralType | Kind::IntersectionType | Kind::InferType | Kind::TypeQuery => {
            under_here = true;
        }
        Kind::TypeOperator => {
            if node.as_type_operator_node().operator != Kind::ReadonlyKeyword {
                under_here = true;
            }
        }
        // A rest parameter is compared element by element (`getTypeAtPosition`): `...a: T` with `T = never[]`
        // against `T = any` compares `never` with `any`, where the markers compare the array types.
        Kind::Parameter if is_rest(node) => under_here = true,
        Kind::TypeReference => {
            let mut symbol = c.get_symbol_from_type_reference(node);
            if symbol.flags().intersects(SymbolFlags::Alias) {
                symbol = c.resolve_alias(symbol);
            }
            if symbol.flags().intersects(SymbolFlags::TypeParameter) {
                if !under {
                    return false;
                }
                let decl = symbol.declarations().first().copied();
                let owner = decl.and_then(|d| d.parent());
                return match owner.map(|o| o.kind()) {
                    // A mapped type's key and an `infer` variable are bound inside the operator.
                    Some(Kind::MappedType) | Some(Kind::InferType) => false,
                    Some(Kind::TypeAliasDeclaration) | Some(Kind::JSTypeAliasDeclaration) => alias_vars.contains(&symbol),
                    _ => true,
                };
            }
            if symbol.flags().intersects(SymbolFlags::TypeAlias) && !node.type_arguments().is_empty() {
                let Some(decl) = symbol.declarations().iter().copied().find(|d| matches!(d.kind(), Kind::TypeAliasDeclaration | Kind::JSTypeAliasDeclaration)) else { return true };
                let params = decl.type_parameters();
                let mut vars = Vec::new();
                for (i, &arg) in node.type_arguments().iter().enumerate() {
                    if sensitive_type(c, arg, under, alias_vars, aliases) {
                        return true;
                    }
                    // Does the argument mention a variable at all?
                    if sensitive_type(c, arg, true, alias_vars, aliases) {
                        if let Some(p) = params.get(i).and_then(|p| c.get_symbol_of_declaration(*p)) {
                            vars.push(p);
                        }
                    }
                }
                if vars.is_empty() {
                    return false;
                }
                if aliases.contains(&symbol) {
                    return true;
                }
                let Some(body) = decl.type_node() else { return true };
                if body.kind() == Kind::IntrinsicKeyword {
                    return true;
                }
                aliases.push(symbol);
                let v = sensitive_type(c, body, under, &vars, aliases);
                aliases.pop();
                return v;
            }
        }
        _ => {}
    }
    let mut found = false;
    node.for_each_child(&mut |child| {
        if !found && sensitive_type(c, child, under_here, alias_vars, aliases) {
            found = true;
        }
        found
    });
    found
}
