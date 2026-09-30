use crate::*;
use tsrs_ast::*;
use tsrs_core::*;
use tsrs_ast as ast;

// Non-function declarations of inference.go (InferenceKey, InferenceState) are in inference_types.rs.

// Go `min` over InferencePriority values (signed: Circularity is -1).
fn min_priority(a: InferencePriority, b: InferencePriority) -> InferencePriority {
    if b.bits() < a.bits() { b } else { a }
}

impl Checker {
    // inference.go:32
    pub(crate) fn get_inference_state(&mut self) -> P<InferenceState> {
        let n = match self.freeinference_state {
            Some(n) => n,
            None => P::new(InferenceState::default()),
        };
        self.freeinference_state = n.next.get();
        n
    }

    // inference.go:41
    pub(crate) fn put_inference_state(&mut self, n: P<InferenceState>) {
        n.visited.clear();
        n.inferences.borrow_mut().clear();
        n.original_source.set(None);
        n.original_target.set(None);
        n.priority.set(InferencePriority::None);
        n.inference_priority.set(InferencePriority::None);
        n.contravariant.set(false);
        n.bivariant.set(false);
        n.expanding_flags.set(ExpandingFlags::None);
        n.propagation_type.set(None);
        n.source_stack.borrow_mut().clear();
        n.target_stack.borrow_mut().clear();
        n.next.set(self.freeinference_state);
        self.freeinference_state = Some(n);
    }

    // inference.go:53
    pub(crate) fn infer_types(&mut self, inferences: &[P<InferenceInfo>], original_source: P<Type>, original_target: P<Type>, priority: InferencePriority, contravariant: bool) {
        let n = self.get_inference_state();
        n.inferences.borrow_mut().extend_from_slice(inferences);
        n.original_source.set(Some(original_source));
        n.original_target.set(Some(original_target));
        n.priority.set(priority);
        n.inference_priority.set(InferencePriority::MaxValue);
        n.contravariant.set(contravariant);
        self.infer_from_types(n, original_source, original_target);
        self.put_inference_state(n);
    }

    // inference.go:65
    pub(crate) fn infer_from_types(&mut self, n: P<InferenceState>, source: P<Type>, target: P<Type>) {
        let mut source = source;
        let mut target = target;
        if !self.could_contain_type_variables(target) || self.is_no_infer_type(target) {
            return;
        }
        if source == self.wildcard_type || source == self.blocked_string_type {
            // We are inferring from an 'any' type. We want to infer this type for every type parameter
            // referenced in the target type, so we record it as the propagation type and infer from the
            // target to itself. Then, as we find candidates we substitute the propagation type.
            let save_propagation_type = n.propagation_type.get();
            n.propagation_type.set(Some(source));
            self.infer_from_types(n, target, target);
            n.propagation_type.set(save_propagation_type);
            return;
        }
        if let (Some(source_alias), Some(target_alias)) = (source.alias(), target.alias()) {
            if source_alias.symbol() == target_alias.symbol() {
                if !source_alias.type_arguments().is_empty() || !target_alias.type_arguments().is_empty() {
                    // Source and target are types originating in the same generic type alias declaration.
                    // Simply infer from source type arguments to target type arguments, with defaults applied.
                    let alias_symbol = source_alias.symbol().unwrap();
                    let params = self.type_alias_links.get(alias_symbol).type_parameters.get();
                    let min_params = self.get_min_type_argument_count(params);
                    let node_is_in_js_file = ast::is_in_js_file(alias_symbol.value_declaration.get());
                    let source_types = self.fill_missing_type_arguments(source_alias.type_arguments(), params, min_params, node_is_in_js_file);
                    let target_types = self.fill_missing_type_arguments(target_alias.type_arguments(), params, min_params, node_is_in_js_file);
                    let variances = self.get_alias_variances(alias_symbol);
                    self.infer_from_type_arguments(n, &source_types, &target_types, &variances);
                }
                // And if there weren't any type arguments, there's no reason to run inference as the types must be the same.
                return;
            }
        }
        if source == target && source.flags().intersects(TypeFlags::UnionOrIntersection) {
            // When source and target are the same union or intersection type, just relate each constituent
            // type to itself.
            for &t in source.types() {
                self.infer_from_types(n, t, t);
            }
            return;
        }
        if target.flags().intersects(TypeFlags::Union) {
            let source_types: Vec<P<Type>> = if source.flags().intersects(TypeFlags::Union) { source.types().to_vec() } else { vec![source] };
            // First, infer between identically matching source and target constituents and remove the
            // matching types.
            let (temp_sources, temp_targets) = self.infer_from_matching_types(n, &source_types, target.types(), |c, s, t| c.is_type_or_base_identical_to(s, t), false /*sort*/);
            // Next, infer between closely matching source and target constituents and remove
            // the matching types. Types closely match when they are instantiations of the same
            // object type or instantiations of the same type alias.
            let (sources, targets) = self.infer_from_matching_types(n, &temp_sources, &temp_targets, |c, s, t| c.is_type_closely_matched_by(s, t), true /*sort*/);
            if targets.is_empty() {
                return;
            }
            target = self.get_union_type(&targets);
            if sources.is_empty() {
                // All source constituents have been matched and there is nothing further to infer from.
                // However, simply making no inferences is undesirable because it could ultimately mean
                // inferring a type parameter constraint. Instead, make a lower priority inference from
                // the full source to whatever remains in the target. For example, when inferring from
                // string to 'string | T', make a lower priority inference of string for T.
                self.infer_with_priority(n, source, target, InferencePriority::NakedTypeVariable);
                return;
            }
            source = self.get_union_type(&sources);
        } else if target.flags().intersects(TypeFlags::Intersection) && !target.types().iter().all(|&t| self.is_non_generic_object_type(t)) {
            // We reduce intersection types unless they're simple combinations of object types. For example,
            // when inferring from 'string[] & { extra: any }' to 'string[] & T' we want to remove string[] and
            // infer { extra: any } for T. But when inferring to 'string[] & Iterable<T>' we want to keep the
            // string[] on the source side and infer string for T.
            if !source.flags().intersects(TypeFlags::Union) {
                let source_types: Vec<P<Type>> = if source.flags().intersects(TypeFlags::Intersection) { source.types().to_vec() } else { vec![source] };
                // Infer between identically matching source and target constituents and remove the matching types.
                let (sources, targets) = self.infer_from_matching_types(n, &source_types, target.types(), |c, s, t| c.is_type_identical_to(s, t), false /*sort*/);
                if sources.is_empty() || targets.is_empty() {
                    return;
                }
                source = self.get_intersection_type(&sources);
                target = self.get_intersection_type(&targets);
            }
        }
        if target.flags().intersects(TypeFlags::IndexedAccess | TypeFlags::Substitution) {
            if self.is_no_infer_type(target) {
                return;
            }
            target = self.get_actual_type_variable(target).unwrap();
        }
        if target.flags().intersects(TypeFlags::TypeVariable) {
            // Skip inference if the source is "blocked", which is used by the language service to
            // prevent inference on nodes currently being edited.
            if self.is_from_inference_blocked_source(source) {
                return;
            }
            let inference = get_inference_info_for_type(n, target);
            if let Some(inference) = inference {
                // If target is a type parameter, make an inference, unless the source type contains
                // a "non-inferrable" type. Types with this flag set are markers used to prevent inference.
                //
                // For example:
                //     - anyFunctionType is a wildcard type that's used to avoid contextually typing functions;
                //       it's internal, so should not be exposed to the user by adding it as a candidate.
                //     - autoType (and autoArrayType) is a special "any" used in control flow; like anyFunctionType,
                //       it's internal and should not be observable.
                //     - silentNeverType is returned by getInferredType when instantiating a generic function for
                //       inference (and a type variable has no mapping).
                //
                // This flag is infectious; if we produce Box<never> (where never is silentNeverType), Box<never> is
                // also non-inferrable.
                //
                // As a special case, also ignore nonInferrableAnyType, which is a special form of the any type
                // used as a stand-in for binding elements when they are being inferred.
                if source.object_flags().intersects(ObjectFlags::NonInferrableType) || source == self.non_inferrable_any_type {
                    return;
                }
                if !inference.is_fixed.get() {
                    let candidate = n.propagation_type.get().unwrap_or(source);
                    if candidate == self.blocked_string_type {
                        return;
                    }
                    if n.priority.get().bits() < inference.priority.get().bits() {
                        inference.candidates.borrow_mut().clear();
                        inference.contra_candidates.borrow_mut().clear();
                        inference.top_level.set(true);
                        inference.priority.set(n.priority.get());
                    }
                    if n.priority.get() == inference.priority.get() {
                        // We make contravariant inferences only if we are in a pure contravariant position,
                        // i.e. only if we have not descended into a bivariant position.
                        if n.contravariant.get() && !n.bivariant.get() {
                            if !inference.contra_candidates.borrow().contains(&candidate) {
                                inference.contra_candidates.borrow_mut().push(candidate);
                                clear_cached_inferences(&n.inferences.borrow());
                            }
                        } else if !inference.candidates.borrow().contains(&candidate) {
                            inference.candidates.borrow_mut().push(candidate);
                            clear_cached_inferences(&n.inferences.borrow());
                        }
                    }
                    if !n.priority.get().intersects(InferencePriority::ReturnType)
                        && target.flags().intersects(TypeFlags::TypeParameter)
                        && inference.top_level.get()
                        && !self.is_type_parameter_at_top_level(n.original_target.get().unwrap(), target, 0)
                    {
                        inference.top_level.set(false);
                        clear_cached_inferences(&n.inferences.borrow());
                    }
                }
                n.inference_priority.set(min_priority(n.inference_priority.get(), n.priority.get()));
                return;
            }
            // Infer to the simplified version of an indexed access, if possible, to (hopefully) expose more bare type parameters to the inference engine
            let simplified = self.get_simplified_type(target, false /*writing*/);
            if simplified != target {
                self.infer_from_types(n, source, simplified);
            } else if target.flags().intersects(TypeFlags::IndexedAccess) {
                let index_type = self.get_simplified_type(target.as_indexed_access_type().index_type().unwrap(), false /*writing*/);
                // Generally simplifications of instantiable indexes are avoided to keep relationship checking correct, however if our target is an access, we can consider
                // that key of that access to be "instantiated", since we're looking to find the infernce goal in any way we can.
                if index_type.flags().intersects(TypeFlags::Instantiable) {
                    let object_type = self.get_simplified_type(target.as_indexed_access_type().object_type().unwrap(), false /*writing*/);
                    let simplified = self.distribute_index_over_object_type(object_type, index_type, false /*writing*/);
                    if let Some(simplified) = simplified {
                        if simplified != target {
                            self.infer_from_types(n, source, simplified);
                        }
                    }
                }
            }
        }
        if source.object_flags().intersects(ObjectFlags::Reference)
            && target.object_flags().intersects(ObjectFlags::Reference)
            && (source.as_type_reference().target.get() == target.as_type_reference().target.get() || self.is_array_type(source) && self.is_array_type(target))
            && !(source.as_type_reference().node.get().is_some() && target.as_type_reference().node.get().is_some())
        {
            // If source and target are references to the same generic type, infer from type arguments
            let source_type_arguments = self.get_type_arguments(source);
            let target_type_arguments = self.get_type_arguments(target);
            let variances = self.get_variances(source.as_type_reference().target.get().unwrap());
            self.infer_from_type_arguments(n, &source_type_arguments, &target_type_arguments, &variances);
        } else if source.flags().intersects(TypeFlags::Index) && target.flags().intersects(TypeFlags::Index) {
            self.infer_from_contravariant_types(n, source.as_index_type().target().unwrap(), target.as_index_type().target().unwrap());
        } else if (is_literal_type(source) || source.flags().intersects(TypeFlags::String)) && target.flags().intersects(TypeFlags::Index) {
            let empty = self.create_empty_object_type_from_string_literal(source);
            self.infer_from_contravariant_types_with_priority(n, empty, target.as_index_type().target().unwrap(), InferencePriority::LiteralKeyof);
        } else if source.flags().intersects(TypeFlags::IndexedAccess) && target.flags().intersects(TypeFlags::IndexedAccess) {
            self.infer_from_types(n, source.as_indexed_access_type().object_type().unwrap(), target.as_indexed_access_type().object_type().unwrap());
            self.infer_from_types(n, source.as_indexed_access_type().index_type().unwrap(), target.as_indexed_access_type().index_type().unwrap());
        } else if source.flags().intersects(TypeFlags::StringMapping) && target.flags().intersects(TypeFlags::StringMapping) {
            if source.symbol() == target.symbol() {
                self.infer_from_types(n, source.as_string_mapping_type().target().unwrap(), target.as_string_mapping_type().target().unwrap());
            }
        } else if source.flags().intersects(TypeFlags::Substitution) {
            self.infer_from_types(n, source.as_substitution_type().base_type().unwrap(), target);
            // Make substitute inference at a lower priority
            let substitution_intersection = self.get_substitution_intersection(source);
            self.infer_with_priority(n, substitution_intersection, target, InferencePriority::SubstituteSource);
        } else if target.flags().intersects(TypeFlags::Conditional) {
            self.invoke_once(n, source, target, |c, n, s, t| c.infer_to_conditional_type(n, s, t));
        } else if target.flags().intersects(TypeFlags::UnionOrIntersection) {
            self.infer_to_multiple_types(n, source, target.types(), target.flags());
        } else if source.flags().intersects(TypeFlags::Union) {
            // Source is a union or intersection type, infer from each constituent type
            for &source_type in source.types() {
                self.infer_from_types(n, source_type, target);
            }
        } else if target.flags().intersects(TypeFlags::TemplateLiteral) {
            self.infer_to_template_literal_type(n, source, target.as_template_literal_type());
        } else {
            source = self.get_reduced_type(source);
            if self.is_generic_mapped_type(source) && self.is_generic_mapped_type(target) {
                self.invoke_once(n, source, target, |c, n, s, t| c.infer_from_generic_mapped_types(n, s, t));
            }
            if !(n.priority.get().intersects(InferencePriority::NoConstraints) && source.flags().intersects(TypeFlags::Intersection | TypeFlags::Instantiable)) {
                let apparent_source = self.get_apparent_type(source);
                // getApparentType can return _any_ type, since an indexed access or conditional may simplify to any other type.
                // If that occurs and it doesn't simplify to an object or intersection, we'll need to restart `inferFromTypes`
                // with the simplified source.
                if apparent_source != source && !apparent_source.flags().intersects(TypeFlags::Object | TypeFlags::Intersection) {
                    self.infer_from_types(n, apparent_source, target);
                    return;
                }
                source = apparent_source;
            }
            if source.flags().intersects(TypeFlags::Object | TypeFlags::Intersection) {
                self.invoke_once(n, source, target, |c, n, s, t| c.infer_from_object_types(n, s, t));
            }
        }
    }

    // inference.go:284
    pub(crate) fn infer_from_type_arguments(&mut self, n: P<InferenceState>, source_types: &[P<Type>], target_types: &[P<Type>], variances: &[VarianceFlags]) {
        for i in 0..source_types.len().min(target_types.len()) {
            if i < variances.len() && (variances[i] & VarianceFlags::VarianceMask) == VarianceFlags::Contravariant {
                self.infer_from_contravariant_types(n, source_types[i], target_types[i]);
            } else {
                self.infer_from_types(n, source_types[i], target_types[i]);
            }
        }
    }

    // inference.go:294
    pub(crate) fn infer_with_priority(&mut self, n: P<InferenceState>, source: P<Type>, target: P<Type>, new_priority: InferencePriority) {
        let save_priority = n.priority.get();
        n.priority.set(n.priority.get() | new_priority);
        self.infer_from_types(n, source, target);
        n.priority.set(save_priority);
    }

    // inference.go:301
    pub(crate) fn infer_from_contravariant_types_with_priority(&mut self, n: P<InferenceState>, source: P<Type>, target: P<Type>, new_priority: InferencePriority) {
        let save_priority = n.priority.get();
        n.priority.set(n.priority.get() | new_priority);
        self.infer_from_contravariant_types(n, source, target);
        n.priority.set(save_priority);
    }

    // inference.go:308
    pub(crate) fn infer_from_contravariant_types(&mut self, n: P<InferenceState>, source: P<Type>, target: P<Type>) {
        n.contravariant.set(!n.contravariant.get());
        self.infer_from_types(n, source, target);
        n.contravariant.set(!n.contravariant.get());
    }

    // inference.go:314
    pub(crate) fn infer_from_contravariant_types_if_strict_function_types(&mut self, n: P<InferenceState>, source: P<Type>, target: P<Type>) {
        if self.strict_function_types || n.priority.get().intersects(InferencePriority::AlwaysStrict) {
            self.infer_from_contravariant_types(n, source, target);
        } else {
            self.infer_from_types(n, source, target);
        }
    }

    // Ensure an inference action is performed only once for the given source and target types.
    // This includes two things:
    // Avoiding inferring between the same pair of source and target types,
    // and avoiding circularly inferring between source and target types.
    // For an example of the last, consider if we are inferring between source type
    // `type Deep<T> = { next: Deep<Deep<T>> }` and target type `type Loop<U> = { next: Loop<U> }`.
    // We would then infer between the types of the `next` property: `Deep<Deep<T>>` = `{ next: Deep<Deep<Deep<T>>> }` and `Loop<U>` = `{ next: Loop<U> }`.
    // We will then infer again between the types of the `next` property:
    // `Deep<Deep<Deep<T>>>` and `Loop<U>`, and so on, such that we would be forever inferring
    // between instantiations of the same types `Deep` and `Loop`.
    // In particular, we would be inferring from increasingly deep instantiations of `Deep` to `Loop`,
    // such that we would go on inferring forever, even though we would never infer
    // between the same pair of types.
    // inference.go:335
    pub(crate) fn invoke_once(&mut self, n: P<InferenceState>, source: P<Type>, target: P<Type>, mut action: impl FnMut(&mut Checker, P<InferenceState>, P<Type>, P<Type>)) {
        let key = InferenceKey { s: source.id, t: target.id };
        if let Some(status) = n.visited.get(&key) {
            n.inference_priority.set(min_priority(n.inference_priority.get(), status));
            return;
        }
        if n.visited.is_nil() {
            n.visited.make();
        }
        n.visited.set(key, InferencePriority::Circularity);
        let save_inference_priority = n.inference_priority.get();
        n.inference_priority.set(InferencePriority::MaxValue);
        // We stop inferring and report a circularity if we encounter duplicate recursion identities on both
        // the source side and the target side.
        let save_expanding_flags = n.expanding_flags.get();
        n.source_stack.borrow_mut().push(source);
        n.target_stack.borrow_mut().push(target);
        // The stack borrows are shared and isDeeplyNestedType never re-enters inference on this state.
        if self.is_deeply_nested_type(source, &n.source_stack.borrow(), 2) {
            n.expanding_flags.set(n.expanding_flags.get() | ExpandingFlags::Source);
        }
        if self.is_deeply_nested_type(target, &n.target_stack.borrow(), 2) {
            n.expanding_flags.set(n.expanding_flags.get() | ExpandingFlags::Target);
        }
        if n.expanding_flags.get() != ExpandingFlags::Both {
            action(self, n, source, target);
        } else {
            n.inference_priority.set(InferencePriority::Circularity);
        }
        n.target_stack.borrow_mut().pop();
        n.source_stack.borrow_mut().pop();
        n.expanding_flags.set(save_expanding_flags);
        n.visited.set(key, n.inference_priority.get());
        n.inference_priority.set(min_priority(n.inference_priority.get(), save_inference_priority));
    }

    // inference.go:370
    pub(crate) fn infer_from_matching_types(&mut self, n: P<InferenceState>, sources: &[P<Type>], targets: &[P<Type>], mut matches: impl FnMut(&mut Checker, P<Type>, P<Type>) -> bool, sort: bool) -> (Vec<P<Type>>, Vec<P<Type>>) {
        let mut matched_sources: Vec<P<Type>> = Vec::new();
        let mut matched_targets: Vec<P<Type>> = Vec::new();
        for &t in targets {
            for &s in sources {
                if matches(self, s, t) {
                    if !sort {
                        self.infer_from_types(n, s, t);
                    }
                    if !matched_sources.contains(&s) {
                        matched_sources.push(s);
                    }
                    if !matched_targets.contains(&t) {
                        matched_targets.push(t);
                    }
                }
            }
        }
        if sort {
            // Sort target types by decreasing depth of generic instantiations. Intuitively, a successful
            // inference from a type argument with deeper nesting is of higher quality because we've stripped
            // away more layers of type instantiations that otherwise might skew the results. For example,
            // when inferring from string[] | string[][] to T[] | T[][], the inference of string we make from
            // relating string[][] to T[][] is of higher quality than the inference of string[] we make relating
            // string[][] to T[].
            matched_targets.sort_by(|&t1, &t2| compare_types_and_depth(self, t1, t2).cmp(&0));
            for &t in &matched_targets {
                for &s in &matched_sources {
                    if matches(self, s, t) {
                        self.infer_from_types(n, s, t);
                    }
                }
            }
        }
        let mut sources = sources.to_vec();
        let mut targets = targets.to_vec();
        if !matched_sources.is_empty() {
            sources.retain(|t| !matched_sources.contains(t));
        }
        if !matched_targets.is_empty() {
            targets.retain(|t| !matched_targets.contains(t));
        }
        (sources, targets)
    }
}

// Compare two types first by depth and then by the regular type ordering.
// inference.go:410
pub(crate) fn compare_types_and_depth(c: &mut Checker, t1: P<Type>, t2: P<Type>) -> i32 {
    let d1 = get_type_depth(c, t1, 3);
    let d2 = get_type_depth(c, t2, 3);
    if d1 != d2 {
        return d2 - d1; // Largest depth sorts first
    }
    compare_types(c, Some(t1), Some(t2))
}

// Return the depth of the given type up to the given maximum depth. For generic aliased types
// and type references, the depth is one plus the largest type argument depth. For union and
// intersection types, the depth is the largest constituent type depth. For all other types,
// the depth is zero. The maximum depth limits infinite recursion of circular types.
// inference.go:423
pub(crate) fn get_type_depth(c: &mut Checker, t: P<Type>, max_depth: i32) -> i32 {
    if max_depth != 0 {
        if let Some(alias) = t.alias() {
            if !alias.type_arguments().is_empty() {
                return get_type_list_depth(c, alias.type_arguments(), max_depth - 1) + 1;
            }
        }
        if t.object_flags().intersects(ObjectFlags::Reference) {
            let type_arguments = c.get_type_arguments(t);
            if !type_arguments.is_empty() {
                return get_type_list_depth(c, &type_arguments, max_depth - 1) + 1;
            }
        }
        if t.flags().intersects(TypeFlags::UnionOrIntersection) {
            return get_type_list_depth(c, t.types(), max_depth);
        }
    }
    0
}

// inference.go:440
pub(crate) fn get_type_list_depth(c: &mut Checker, types: &[P<Type>], max_depth: i32) -> i32 {
    let mut depth = 0;
    for &t in types {
        depth = depth.max(get_type_depth(c, t, max_depth));
    }
    depth
}

impl Checker {
    // inference.go:448
    pub(crate) fn infer_to_multiple_types(&mut self, n: P<InferenceState>, source: P<Type>, targets: &[P<Type>], target_flags: TypeFlags) {
        let mut type_variable_count = 0;
        if target_flags.intersects(TypeFlags::Union) {
            let mut naked_type_variable: Option<P<Type>> = None;
            let sources: Vec<P<Type>> = if source.flags().intersects(TypeFlags::Union) { source.types().to_vec() } else { vec![source] };
            let mut matched = vec![false; sources.len()];
            let mut inference_circularity = false;
            // First infer to types that are not naked type variables. For each source type we
            // track whether inferences were made from that particular type to some target with
            // equal priority (i.e. of equal quality) to what we would infer for a naked type
            // parameter.
            for &t in targets {
                if get_inference_info_for_type(n, t).is_some() {
                    naked_type_variable = Some(t);
                    type_variable_count += 1;
                } else {
                    for i in 0..sources.len() {
                        let save_inference_priority = n.inference_priority.get();
                        n.inference_priority.set(InferencePriority::MaxValue);
                        self.infer_from_types(n, sources[i], t);
                        if n.inference_priority.get() == n.priority.get() {
                            matched[i] = true;
                        }
                        inference_circularity = inference_circularity || n.inference_priority.get() == InferencePriority::Circularity;
                        n.inference_priority.set(min_priority(n.inference_priority.get(), save_inference_priority));
                    }
                }
            }
            if type_variable_count == 0 {
                // If every target is an intersection of types containing a single naked type variable,
                // make a lower priority inference to that type variable. This handles inferring from
                // 'A | B' to 'T & (X | Y)' where we want to infer 'A | B' for T.
                let intersection_type_variable = get_single_type_variable_from_intersection_types(n, targets);
                if let Some(intersection_type_variable) = intersection_type_variable {
                    self.infer_with_priority(n, source, intersection_type_variable, InferencePriority::NakedTypeVariable);
                }
                return;
            }
            // If the target has a single naked type variable and no inference circularities were
            // encountered above (meaning we explored the types fully), create a union of the source
            // types from which no inferences have been made so far and infer from that union to the
            // naked type variable.
            if type_variable_count == 1 && !inference_circularity {
                let mut unmatched: Vec<P<Type>> = Vec::new();
                for (i, &s) in sources.iter().enumerate() {
                    if !matched[i] {
                        unmatched.push(s);
                    }
                }
                if !unmatched.is_empty() {
                    let union = self.get_union_type(&unmatched);
                    self.infer_from_types(n, union, naked_type_variable.unwrap());
                    return;
                }
            }
        } else {
            // We infer from types that are not naked type variables first so that inferences we
            // make from nested naked type variables and given slightly higher priority by virtue
            // of being first in the candidates array.
            for &t in targets {
                if get_inference_info_for_type(n, t).is_some() {
                    type_variable_count += 1;
                } else {
                    self.infer_from_types(n, source, t);
                }
            }
        }
        // Inferences directly to naked type variables are given lower priority as they are
        // less specific. For example, when inferring from Promise<string> to T | Promise<T>,
        // we want to infer string for T, not Promise<string> | string. For intersection types
        // we only infer to single naked type variables.
        if target_flags.intersects(TypeFlags::Intersection) && type_variable_count == 1 || !target_flags.intersects(TypeFlags::Intersection) && type_variable_count > 0 {
            for &t in targets {
                if get_inference_info_for_type(n, t).is_some() {
                    self.infer_with_priority(n, source, t, InferencePriority::NakedTypeVariable);
                }
            }
        }
    }
}

// inference.go:532
pub(crate) fn get_single_type_variable_from_intersection_types(n: P<InferenceState>, types: &[P<Type>]) -> Option<P<Type>> {
    let mut type_variable: Option<P<Type>> = None;
    for &t in types {
        if !t.flags().intersects(TypeFlags::Intersection) {
            return None;
        }
        let v = t.types().iter().copied().find(|&t| get_inference_info_for_type(n, t).is_some());
        if v.is_none() || type_variable.is_some() && v != type_variable {
            return None;
        }
        type_variable = v;
    }
    type_variable
}

impl Checker {
    // inference.go:547
    pub(crate) fn infer_to_multiple_types_with_priority(&mut self, n: P<InferenceState>, source: P<Type>, targets: &[P<Type>], target_flags: TypeFlags, new_priority: InferencePriority) {
        let save_priority = n.priority.get();
        n.priority.set(n.priority.get() | new_priority);
        self.infer_to_multiple_types(n, source, targets, target_flags);
        n.priority.set(save_priority);
    }

    // inference.go:554
    pub(crate) fn infer_to_conditional_type(&mut self, n: P<InferenceState>, source: P<Type>, target: P<Type>) {
        if source.flags().intersects(TypeFlags::Conditional) {
            let source_check_type = get_non_distributed_type_parameter(source.as_conditional_type().check_type().unwrap()).unwrap();
            self.infer_from_types(n, source_check_type, target.as_conditional_type().check_type().unwrap());
            let source_extends_type = get_non_distributed_type_parameter(source.as_conditional_type().extends_type().unwrap()).unwrap();
            self.infer_from_types(n, source_extends_type, target.as_conditional_type().extends_type().unwrap());
            let source_true_type = self.get_true_type_from_conditional_type(source);
            let source_true_type = get_non_distributed_type_parameter(source_true_type).unwrap();
            let target_true_type = self.get_true_type_from_conditional_type(target);
            self.infer_from_types(n, source_true_type, target_true_type);
            let source_false_type = self.get_false_type_from_conditional_type(source);
            let source_false_type = get_non_distributed_type_parameter(source_false_type).unwrap();
            let target_false_type = self.get_false_type_from_conditional_type(target);
            self.infer_from_types(n, source_false_type, target_false_type);
        } else {
            let target_types = [self.get_true_type_from_conditional_type(target), self.get_false_type_from_conditional_type(target)];
            let priority = if n.contravariant.get() { InferencePriority::ContravariantConditional } else { InferencePriority::None };
            self.infer_to_multiple_types_with_priority(n, source, &target_types, target.flags(), priority);
        }
    }

    // inference.go:566
    pub(crate) fn infer_to_template_literal_type(&mut self, n: P<InferenceState>, source: P<Type>, target: &'static TemplateLiteralType) {
        let comparer = self.compare_types_assignable_comparer();
        let matches = self.infer_types_from_template_literal_type(source, target, comparer);
        let types = target.types();
        // When the target template literal contains only placeholders (meaning that inference is intended to extract
        // single characters and remainder strings) and inference fails to produce matches, we want to infer 'never' for
        // each placeholder such that instantiation with the inferred value(s) produces 'never', a type for which an
        // assignment check will fail. If we make no inferences, we'll likely end up with the constraint 'string' which,
        // upon instantiation, would collapse all the placeholders to just 'string', and an assignment check might
        // succeed. That would be a pointless and confusing outcome.
        if !matches.is_empty() || target.texts().iter().all(|s| s.is_empty()) {
            'outer: for (i, &target) in types.iter().enumerate() {
                let source = if !matches.is_empty() { matches[i] } else { self.never_type };
                // If we are inferring from a string literal type to a type variable whose constraint includes one of the
                // allowed template literal placeholder types, infer from a literal type corresponding to the constraint.
                if source.flags().intersects(TypeFlags::StringLiteral) && target.flags().intersects(TypeFlags::TypeVariable) {
                    if let Some(inference_context) = get_inference_info_for_type(n, target) {
                        let constraint = self.get_base_constraint_of_type(inference_context.type_parameter.get().unwrap());
                        if let Some(constraint) = constraint {
                            if !is_type_any(Some(constraint)) {
                                let mut all_type_flags = TypeFlags::None;
                                for t in constraint.distributed() {
                                    all_type_flags |= t.flags();
                                }
                                // If the constraint contains `string`, we don't need to look for a more preferred type
                                if !all_type_flags.intersects(TypeFlags::String) {
                                    let str = get_string_literal_value(source);
                                    // If the type contains `number` or a number literal and the string isn't a valid number, exclude numbers
                                    if all_type_flags.intersects(TypeFlags::NumberLike) && !is_valid_number_string(&str, true /*roundTripOnly*/) {
                                        all_type_flags &= !TypeFlags::NumberLike;
                                    }
                                    // If the type contains `bigint` or a bigint literal and the string isn't a valid bigint, exclude bigints
                                    if all_type_flags.intersects(TypeFlags::BigIntLike) && !is_valid_big_int_string(&str, true /*roundTripOnly*/) {
                                        all_type_flags &= !TypeFlags::BigIntLike;
                                    }
                                    let mut matching_type = self.never_type;
                                    for t in constraint.distributed() {
                                        matching_type = infer_to_template_literal_type_choose(self, matching_type, t, all_type_flags, source, &str);
                                    }
                                    if !matching_type.flags().intersects(TypeFlags::Never) {
                                        self.infer_from_types(n, matching_type, target);
                                        continue 'outer;
                                    }
                                }
                            }
                        }
                    }
                }
                self.infer_from_types(n, source, target);
            }
        }
    }
}

// The `choose` closure of inferToTemplateLiteralType.
fn infer_to_template_literal_type_choose(c: &mut Checker, left: P<Type>, right: P<Type>, all_type_flags: TypeFlags, source: P<Type>, str: &str) -> P<Type> {
    if !right.flags().intersects(all_type_flags) {
        left
    } else if left.flags().intersects(TypeFlags::String) {
        left
    } else if right.flags().intersects(TypeFlags::String) {
        source
    } else if left.flags().intersects(TypeFlags::TemplateLiteral) {
        left
    } else if right.flags().intersects(TypeFlags::TemplateLiteral) && {
        let comparer = c.compare_types_assignable_comparer();
        c.is_type_matched_by_template_literal_type(source, right.as_template_literal_type(), comparer)
    } {
        source
    } else if left.flags().intersects(TypeFlags::StringMapping) {
        left
    } else if right.flags().intersects(TypeFlags::StringMapping) && str == apply_string_mapping(right.symbol().unwrap(), str) {
        source
    } else if left.flags().intersects(TypeFlags::StringLiteral) {
        left
    } else if right.flags().intersects(TypeFlags::StringLiteral) && get_string_literal_value(right) == str {
        right
    } else if left.flags().intersects(TypeFlags::Number) {
        left
    } else if right.flags().intersects(TypeFlags::Number) {
        c.get_number_literal_type(jsnum::from_string(str))
    } else if left.flags().intersects(TypeFlags::Enum) {
        left
    } else if right.flags().intersects(TypeFlags::Enum) {
        c.get_number_literal_type(jsnum::from_string(str))
    } else if left.flags().intersects(TypeFlags::NumberLiteral) {
        left
    } else if right.flags().intersects(TypeFlags::NumberLiteral) && get_number_literal_value(right) == jsnum::from_string(str) {
        right
    } else if left.flags().intersects(TypeFlags::BigInt) {
        left
    } else if right.flags().intersects(TypeFlags::BigInt) {
        c.parse_big_int_literal_type(str)
    } else if left.flags().intersects(TypeFlags::BigIntLiteral) {
        left
    } else if right.flags().intersects(TypeFlags::BigIntLiteral) && pseudo_big_int_to_string(get_big_int_literal_value(right)) == str {
        right
    } else if left.flags().intersects(TypeFlags::Boolean) {
        left
    } else if right.flags().intersects(TypeFlags::Boolean) {
        match str {
            "true" => c.true_type,
            "false" => c.false_type,
            _ => c.boolean_type,
        }
    } else if left.flags().intersects(TypeFlags::BooleanLiteral) {
        left
    } else if right.flags().intersects(TypeFlags::BooleanLiteral) && (if get_boolean_literal_value(right) { "true" } else { "false" }) == str {
        right
    } else if left.flags().intersects(TypeFlags::Undefined) {
        left
    } else if right.flags().intersects(TypeFlags::Undefined) && right.as_intrinsic_type().intrinsic_name() == str {
        right
    } else if left.flags().intersects(TypeFlags::Null) {
        left
    } else if right.flags().intersects(TypeFlags::Null) && right.as_intrinsic_type().intrinsic_name() == str {
        right
    } else {
        left
    }
}

impl Checker {
    // inference.go:687
    pub(crate) fn infer_from_generic_mapped_types(&mut self, n: P<InferenceState>, source: P<Type>, target: P<Type>) {
        // The source and target types are generic types { [P in S]: X } and { [P in T]: Y }, so we infer
        // from S to T and from X to Y.
        let source_constraint = self.get_constraint_type_from_mapped_type(source);
        let target_constraint = self.get_constraint_type_from_mapped_type(target);
        self.infer_from_types(n, source_constraint, target_constraint);
        let source_template = self.get_template_type_from_mapped_type(source);
        let target_template = self.get_template_type_from_mapped_type(target);
        self.infer_from_types(n, source_template, target_template);
        let source_name_type = self.get_name_type_from_mapped_type(source);
        let target_name_type = self.get_name_type_from_mapped_type(target);
        if let (Some(source_name_type), Some(target_name_type)) = (source_name_type, target_name_type) {
            self.infer_from_types(n, source_name_type, target_name_type);
        }
    }

    // inference.go:699
    pub(crate) fn infer_from_object_types(&mut self, n: P<InferenceState>, source: P<Type>, target: P<Type>) {
        if source.object_flags().intersects(ObjectFlags::Reference)
            && target.object_flags().intersects(ObjectFlags::Reference)
            && (source.target() == target.target() || self.is_array_type(source) && self.is_array_type(target))
        {
            // If source and target are references to the same generic type, infer from type arguments
            let source_type_arguments = self.get_type_arguments(source);
            let target_type_arguments = self.get_type_arguments(target);
            let variances = self.get_variances(source.target().unwrap());
            self.infer_from_type_arguments(n, &source_type_arguments, &target_type_arguments, &variances);
            return;
        }
        if self.is_generic_mapped_type(source) && self.is_generic_mapped_type(target) {
            self.infer_from_generic_mapped_types(n, source, target);
        }
        if target.object_flags().intersects(ObjectFlags::Mapped) && target.as_mapped_type().declaration.get().unwrap().as_mapped_type_node().name_type().is_none() {
            let constraint_type = self.get_constraint_type_from_mapped_type(target);
            if self.infer_to_mapped_type(n, source, target, constraint_type) {
                return;
            }
        }
        // Infer from the members of source and target only if the two types are possibly related
        if self.types_definitely_unrelated(source, target) {
            return;
        }
        if self.is_array_or_tuple_type(source) {
            if is_tuple_type(target) {
                let source_arity = self.get_type_reference_arity(source);
                let target_arity = self.get_type_reference_arity(target);
                let element_types = self.get_type_arguments(target);
                let element_infos = target.target_tuple_type().element_infos();
                // When source and target are tuple types with the same structure (fixed, variadic, and rest are matched
                // to the same kind in each position), simply infer between the element types.
                if is_tuple_type(source) && self.is_tuple_type_structure_matching(source, target) {
                    for i in 0..target_arity as usize {
                        let s = self.get_type_arguments(source)[i];
                        self.infer_from_types(n, s, element_types[i]);
                    }
                    return;
                }
                let mut start_length: i32 = 0;
                let mut end_length: i32 = 0;
                if is_tuple_type(source) {
                    start_length = source.target_tuple_type().fixed_length().min(target.target_tuple_type().fixed_length());
                    if target.target_tuple_type().combined_flags.get().intersects(ElementFlags::Variable) {
                        end_length = get_end_element_count(source.target_tuple_type(), ElementFlags::Fixed).min(get_end_element_count(target.target_tuple_type(), ElementFlags::Fixed));
                    }
                }
                // Infer between starting fixed elements.
                for i in 0..start_length as usize {
                    let s = self.get_type_arguments(source)[i];
                    self.infer_from_types(n, s, element_types[i]);
                }
                if !is_tuple_type(source) || source_arity - start_length - end_length == 1 && source.target_tuple_type().element_infos()[start_length as usize].flags.intersects(ElementFlags::Rest) {
                    // Single rest element remains in source, infer from that to every element in target
                    let rest_type = self.get_type_arguments(source)[start_length as usize];
                    for i in start_length..target_arity - end_length {
                        let mut t = rest_type;
                        if element_infos[i as usize].flags.intersects(ElementFlags::Variadic) {
                            t = self.create_array_type(t);
                        }
                        self.infer_from_types(n, t, element_types[i as usize]);
                    }
                } else {
                    let middle_length = target_arity - start_length - end_length;
                    let sl = start_length as usize;
                    if middle_length == 2 {
                        if (element_infos[sl].flags & element_infos[sl + 1].flags).intersects(ElementFlags::Variadic) {
                            // Middle of target is [...T, ...U] and source is tuple type
                            let target_info = get_inference_info_for_type(n, element_types[sl]);
                            if let Some(target_info) = target_info {
                                if target_info.implied_arity.get() >= 0 {
                                    // Infer slices from source based on implied arity of T.
                                    let implied_arity = target_info.implied_arity.get();
                                    let slice = self.slice_tuple_type(source, start_length, end_length + source_arity - implied_arity);
                                    self.infer_from_types(n, slice, element_types[sl]);
                                    let slice = self.slice_tuple_type(source, start_length + implied_arity, end_length);
                                    self.infer_from_types(n, slice, element_types[sl + 1]);
                                }
                            }
                        } else if element_infos[sl].flags.intersects(ElementFlags::Variadic) && element_infos[sl + 1].flags.intersects(ElementFlags::Rest) {
                            // Middle of target is [...T, ...rest] and source is tuple type
                            // if T is constrained by a fixed-size tuple we might be able to use its arity to infer T
                            if let Some(info) = get_inference_info_for_type(n, element_types[sl]) {
                                let constraint = self.get_base_constraint_of_type(info.type_parameter.get().unwrap());
                                if let Some(constraint) = constraint {
                                    if is_tuple_type(constraint) && !constraint.target_tuple_type().combined_flags.get().intersects(ElementFlags::Variable) {
                                        let implied_arity = constraint.target_tuple_type().fixed_length();
                                        let slice = self.slice_tuple_type(source, start_length, source_arity - (start_length + implied_arity));
                                        self.infer_from_types(n, slice, element_types[sl]);
                                        if let Some(rest_type) = self.get_element_type_of_slice_of_tuple_type(source, start_length + implied_arity, end_length, false, false) {
                                            self.infer_from_types(n, rest_type, element_types[sl + 1]);
                                        }
                                    }
                                }
                            }
                        } else if element_infos[sl].flags.intersects(ElementFlags::Rest) && element_infos[sl + 1].flags.intersects(ElementFlags::Variadic) {
                            // Middle of target is [...rest, ...T] and source is tuple type
                            // if T is constrained by a fixed-size tuple we might be able to use its arity to infer T
                            if let Some(info) = get_inference_info_for_type(n, element_types[sl + 1]) {
                                let constraint = self.get_base_constraint_of_type(info.type_parameter.get().unwrap());
                                if let Some(constraint) = constraint {
                                    if is_tuple_type(constraint) && !constraint.target_tuple_type().combined_flags.get().intersects(ElementFlags::Variable) {
                                        let implied_arity = constraint.target_tuple_type().fixed_length();
                                        let end_index = source_arity - get_end_element_count(target.target_tuple_type(), ElementFlags::Fixed);
                                        let start_index = end_index - implied_arity;
                                        if start_index >= start_length {
                                            let source_type_arguments = self.get_type_arguments(source);
                                            let trailing_slice = self.create_tuple_type_ex(
                                                &source_type_arguments[start_index as usize..end_index as usize],
                                                &source.target_tuple_type().element_infos()[start_index as usize..end_index as usize],
                                                false, /*readonly*/
                                            );
                                            if let Some(rest_type) = self.get_element_type_of_slice_of_tuple_type(source, start_length, end_length + implied_arity, false, false) {
                                                self.infer_from_types(n, rest_type, element_types[sl]);
                                            }
                                            self.infer_from_types(n, trailing_slice, element_types[sl + 1]);
                                        }
                                    }
                                }
                            }
                        }
                    } else if middle_length == 1 && element_infos[sl].flags.intersects(ElementFlags::Variadic) {
                        // Middle of target is exactly one variadic element. Infer the slice between the fixed parts in the source.
                        // If target ends in optional element(s), make a lower priority a speculative inference.
                        let priority = if element_infos[(target_arity - 1) as usize].flags.intersects(ElementFlags::Optional) { InferencePriority::SpeculativeTuple } else { InferencePriority::None };
                        let source_slice = self.slice_tuple_type(source, start_length, end_length);
                        self.infer_with_priority(n, source_slice, element_types[sl], priority);
                    } else if middle_length == 1 && element_infos[sl].flags.intersects(ElementFlags::Rest) {
                        // Middle of target is exactly one rest element. If middle of source is not empty, infer union of middle element types.
                        let rest_type = self.get_element_type_of_slice_of_tuple_type(source, start_length, end_length, false, false);
                        if let Some(rest_type) = rest_type {
                            self.infer_from_types(n, rest_type, element_types[sl]);
                        }
                    }
                }
                // Infer between ending fixed elements
                for i in 0..end_length {
                    let s = self.get_type_arguments(source)[(source_arity - i - 1) as usize];
                    self.infer_from_types(n, s, element_types[(target_arity - i - 1) as usize]);
                }
                return;
            }
            if self.is_array_type(target) {
                self.infer_from_index_types(n, source, target);
                return;
            }
        }
        self.infer_from_properties(n, source, target);
        self.infer_from_signatures(n, source, target, SignatureKind::Call);
        self.infer_from_signatures(n, source, target, SignatureKind::Construct);
        self.infer_from_index_types(n, source, target);
    }

    // inference.go:828
    pub(crate) fn infer_from_properties(&mut self, n: P<InferenceState>, source: P<Type>, target: P<Type>) {
        let properties = self.get_properties_of_object_type(target);
        for target_prop in properties {
            let source_prop = self.get_property_of_type(source, target_prop.name.get());
            if let Some(source_prop) = source_prop {
                let declarations = source_prop.declarations.borrow().clone();
                if !declarations.iter().any(|&d| self.is_skip_direct_inference_node(d)) {
                    let source_prop_type = self.get_type_of_symbol(source_prop);
                    let s = self.remove_missing_type(source_prop_type, source_prop.flags.get().intersects(SymbolFlags::Optional));
                    let target_prop_type = self.get_type_of_symbol(target_prop);
                    let t = self.remove_missing_type(target_prop_type, target_prop.flags.get().intersects(SymbolFlags::Optional));
                    self.infer_from_types(n, s, t);
                }
            }
        }
    }

    // inference.go:838
    pub(crate) fn infer_from_signatures(&mut self, n: P<InferenceState>, source: P<Type>, target: P<Type>, kind: SignatureKind) {
        let source_signatures = self.get_signatures_of_type(source, kind);
        let source_len = source_signatures.len() as i32;
        if source_len > 0 {
            // We match source and target signatures from the bottom up, and if the source has fewer signatures
            // than the target, we infer from the first source signature to the excess target signatures.
            let target_signatures = self.get_signatures_of_type(target, kind);
            let target_len = target_signatures.len() as i32;
            for i in 0..target_len {
                let source_index = (source_len - target_len + i).max(0);
                let base_signature = self.get_base_signature(source_signatures[source_index as usize]);
                let erased_signature = self.get_erased_signature(target_signatures[i as usize]);
                self.infer_from_signature(n, base_signature, erased_signature);
            }
        }
    }

    // inference.go:853
    pub(crate) fn infer_from_signature(&mut self, n: P<InferenceState>, source: P<Signature>, target: P<Signature>) {
        if !source.flags().intersects(SignatureFlags::IsNonInferrable) {
            let save_bivariant = n.bivariant.get();
            let kind = match target.declaration() {
                Some(declaration) => declaration.kind,
                None => Kind::Unknown,
            };
            // Once we descend into a bivariant signature we remain bivariant for all nested inferences
            n.bivariant.set(n.bivariant.get() || kind == Kind::MethodDeclaration || kind == Kind::MethodSignature || kind == Kind::Constructor);
            self.apply_to_parameter_types(source, target, |c, s, t| c.infer_from_contravariant_types_if_strict_function_types(n, s, t));
            n.bivariant.set(save_bivariant);
        }
        self.apply_to_return_types(source, target, |c, s, t| c.infer_from_types(n, s, t));
    }

    // inference.go:868
    pub(crate) fn apply_to_parameter_types(&mut self, source: P<Signature>, target: P<Signature>, mut callback: impl FnMut(&mut Checker, P<Type>, P<Type>)) {
        let source_count = self.get_parameter_count(source);
        let target_count = self.get_parameter_count(target);
        let source_rest_type = self.get_effective_rest_type(source);
        let target_rest_type = self.get_effective_rest_type(target);
        let mut target_non_rest_count = target_count;
        if target_rest_type.is_some() {
            target_non_rest_count -= 1;
        }
        let mut param_count = target_non_rest_count;
        if source_rest_type.is_none() {
            param_count = source_count.min(target_non_rest_count);
        }
        let source_this_type = self.get_this_type_of_signature(source);
        if let Some(source_this_type) = source_this_type {
            let target_this_type = self.get_this_type_of_signature(target);
            if let Some(target_this_type) = target_this_type {
                callback(self, source_this_type, target_this_type);
            }
        }
        for i in 0..param_count {
            let s = self.get_type_at_position(source, i);
            let t = self.get_type_at_position(target, i);
            callback(self, s, t);
        }
        if let Some(target_rest_type) = target_rest_type {
            let readonly = self.is_const_type_variable(Some(target_rest_type), 0) && !some_type(target_rest_type, |t| self.is_mutable_array_like_type(t));
            let rest_type = self.get_rest_type_at_position(source, param_count, readonly);
            callback(self, rest_type, target_rest_type);
        }
    }

    // inference.go:896
    pub(crate) fn apply_to_return_types(&mut self, source: P<Signature>, target: P<Signature>, mut callback: impl FnMut(&mut Checker, P<Type>, P<Type>)) {
        let target_type_predicate = self.get_type_predicate_of_signature(target);
        if let Some(target_type_predicate) = target_type_predicate {
            let source_type_predicate = self.get_type_predicate_of_signature(source);
            if let Some(source_type_predicate) = source_type_predicate {
                if self.type_predicate_kinds_match(source_type_predicate, target_type_predicate) {
                    if let (Some(s), Some(t)) = (source_type_predicate.t.get(), target_type_predicate.t.get()) {
                        callback(self, s, t);
                        return;
                    }
                }
            }
        }
        let target_return_type = self.get_return_type_of_signature(target);
        if self.could_contain_type_variables(target_return_type) {
            let source_return_type = self.get_return_type_of_signature(source);
            callback(self, source_return_type, target_return_type);
        }
    }

    // inference.go:911
    pub(crate) fn infer_from_index_types(&mut self, n: P<InferenceState>, source: P<Type>, target: P<Type>) {
        // Inferences across mapped type index signatures are pretty much the same a inferences to homomorphic variables
        let mut priority = InferencePriority::None;
        if (source.object_flags() & target.object_flags()).intersects(ObjectFlags::Mapped) {
            priority = InferencePriority::HomomorphicMappedType;
        }
        let index_infos = self.get_index_infos_of_type(target);
        if self.is_object_type_with_inferable_index(source) {
            for &target_info in &index_infos {
                let mut prop_types: Vec<P<Type>> = Vec::new();
                for prop in self.get_properties_of_type(source) {
                    let literal_type = self.get_literal_type_from_property(prop, TypeFlags::StringOrNumberLiteralOrUnique, false);
                    if self.is_applicable_index_type(literal_type, target_info.key_type()) {
                        let mut prop_type = self.get_type_of_symbol(prop);
                        if prop.flags.get().intersects(SymbolFlags::Optional) {
                            prop_type = self.remove_missing_or_undefined_type(prop_type);
                        }
                        prop_types.push(prop_type);
                    }
                }
                for info in self.get_index_infos_of_type(source) {
                    if self.is_applicable_index_type(info.key_type(), target_info.key_type()) {
                        prop_types.push(info.value_type());
                    }
                }
                if !prop_types.is_empty() {
                    let union = self.get_union_type(&prop_types);
                    self.infer_with_priority(n, union, target_info.value_type(), priority);
                }
            }
        }
        for &target_info in &index_infos {
            let source_info = self.get_applicable_index_info(source, target_info.key_type());
            if let Some(source_info) = source_info {
                self.infer_with_priority(n, source_info.value_type(), target_info.value_type(), priority);
            }
        }
    }

    // inference.go:948
    pub(crate) fn infer_to_mapped_type(&mut self, n: P<InferenceState>, source: P<Type>, target: P<Type>, constraint_type: P<Type>) -> bool {
        if constraint_type.flags().intersects(TypeFlags::Union) || constraint_type.flags().intersects(TypeFlags::Intersection) {
            let mut result = false;
            for &t in constraint_type.types() {
                let r = self.infer_to_mapped_type(n, source, target, t);
                result = r || result;
            }
            return result;
        }
        if constraint_type.flags().intersects(TypeFlags::Index) {
            // We're inferring from some source type S to a homomorphic mapped type { [P in keyof T]: X },
            // where T is a type variable. Use inferTypeForHomomorphicMappedType to infer a suitable source
            // type and then make a secondary inference from that type to T. We make a secondary inference
            // such that direct inferences to T get priority over inferences to Partial<T>, for example.
            let inference = get_inference_info_for_type(n, constraint_type.as_index_type().target().unwrap());
            if let Some(inference) = inference {
                if !inference.is_fixed.get() && !self.is_from_inference_blocked_source(source) {
                    let inferred_type = self.infer_type_for_homomorphic_mapped_type(source, target, constraint_type);
                    if let Some(inferred_type) = inferred_type {
                        // We assign a lower priority to inferences made from types containing non-inferrable
                        // types because we may only have a partial result (i.e. we may have failed to make
                        // reverse inferences for some properties).
                        let priority = if source.object_flags().intersects(ObjectFlags::NonInferrableType) {
                            InferencePriority::PartialHomomorphicMappedType
                        } else {
                            InferencePriority::HomomorphicMappedType
                        };
                        self.infer_with_priority(n, inferred_type, inference.type_parameter.get().unwrap(), priority);
                    }
                }
            }
            return true;
        }
        if constraint_type.flags().intersects(TypeFlags::TypeParameter) {
            // We're inferring from some source type S to a mapped type { [P in K]: X }, where K is a type
            // parameter. First infer from 'keyof S' to K.
            let index_flags = if self.pattern_for_type.contains_key(&source) { IndexFlags::NoIndexSignatures } else { IndexFlags::None };
            let index_type = self.get_index_type_ex(source, index_flags);
            self.infer_with_priority(n, index_type, constraint_type, InferencePriority::MappedTypeConstraint);
            // If K is constrained to a type C, also infer to C. Thus, for a mapped type { [P in K]: X },
            // where K extends keyof T, we make the same inferences as for a homomorphic mapped type
            // { [P in keyof T]: X }. This enables us to make meaningful inferences when the target is a
            // Pick<T, K>.
            let extended_constraint = self.get_constraint_of_type(constraint_type);
            if let Some(extended_constraint) = extended_constraint {
                if self.infer_to_mapped_type(n, source, target, extended_constraint) {
                    return true;
                }
            }
            // If no inferences can be made to K's constraint, infer from a union of the property types
            // in the source to the template type X.
            let mut prop_types: Vec<P<Type>> = Vec::new();
            for prop in self.get_properties_of_type(source) {
                prop_types.push(self.get_type_of_symbol(prop));
            }
            let index_types: Vec<P<Type>> = self
                .get_index_infos_of_type(source)
                .iter()
                .map(|&info| if info != self.enum_number_index_info { info.value_type() } else { self.never_type })
                .collect();
            prop_types.extend(index_types);
            let union = self.get_union_type(&prop_types);
            let template_type = self.get_template_type_from_mapped_type(target);
            self.infer_from_types(n, union, template_type);
            return true;
        }
        false
    }

    // Infer a suitable input type for a homomorphic mapped type { [P in keyof T]: X }. We construct
    // an object type with the same set of properties as the source type, where the type of each
    // property is computed by inferring from the source property type to X for the type
    // variable T[P] (i.e. we treat the type T[P] as the type variable we're inferring for).
    // inference.go:1004
    pub(crate) fn infer_type_for_homomorphic_mapped_type(&mut self, source: P<Type>, target: P<Type>, constraint: P<Type>) -> Option<P<Type>> {
        let key = ReverseMappedTypeKey { source_id: source.id, target_id: target.id, constraint_id: constraint.id };
        if let Some(&cached) = self.reverse_homomorphic_mapped_cache.get(&key) {
            return Some(cached);
        }
        let t = self.create_reverse_mapped_type(source, target, constraint);
        // Go also stores a nil result, which reads back as a miss; not storing it is equivalent.
        if let Some(t) = t {
            self.reverse_homomorphic_mapped_cache.insert(key, t);
        }
        t
    }

    // inference.go:1014
    pub(crate) fn create_reverse_mapped_type(&mut self, source: P<Type>, target: P<Type>, constraint: P<Type>) -> Option<P<Type>> {
        // We consider a source type reverse mappable if it has a string index signature or if
        // it has one or more properties and is of a partially inferable type.
        let string_type = self.string_type;
        if !(self.get_index_info_of_type(source, string_type).is_some() || !self.get_properties_of_type(source).is_empty() && self.is_partially_inferable_type(source)) {
            return None;
        }
        // For arrays and tuples we infer new arrays and tuples where the reverse mapping has been
        // applied to the element type(s).
        if self.is_array_type(source) {
            let source_element_type = self.get_type_arguments(source)[0];
            let element_type = self.infer_reverse_mapped_type(source_element_type, target, constraint)?;
            let readonly = self.is_readonly_array_type(source);
            return Some(self.create_array_type_ex(element_type, readonly));
        }
        if is_tuple_type(source) {
            let mut element_types: Vec<Option<P<Type>>> = Vec::new();
            for t in self.get_element_types(source) {
                element_types.push(self.infer_reverse_mapped_type(t, target, constraint));
            }
            if !element_types.iter().all(|t| t.is_some()) {
                return None;
            }
            let element_types: Vec<P<Type>> = element_types.into_iter().map(|t| t.unwrap()).collect();
            let mut element_infos: Vec<TupleElementInfo> = source.target_tuple_type().element_infos().to_vec();
            if get_mapped_type_modifiers(target).intersects(MappedTypeModifiers::IncludeOptional) {
                element_infos = element_infos
                    .into_iter()
                    .map(|info| {
                        if info.flags.intersects(ElementFlags::Optional) {
                            return TupleElementInfo { flags: ElementFlags::Required, labeled_declaration: info.labeled_declaration };
                        }
                        info
                    })
                    .collect();
            }
            let readonly = source.target_tuple_type().readonly.get();
            return Some(self.create_tuple_type_ex(&element_types, &element_infos, readonly));
        }
        // For all other object types we infer a new object type where the reverse mapping has been
        // applied to the type of each property.
        let reversed = self.new_object_type(ObjectFlags::ReverseMapped | ObjectFlags::Anonymous, None /*symbol*/);
        reversed.as_reverse_mapped_type().source.set(Some(source));
        reversed.as_reverse_mapped_type().mapped_type.set(Some(target));
        reversed.as_reverse_mapped_type().constraint_type.set(Some(constraint));
        Some(reversed)
    }

    // We consider a type to be partially inferable if it isn't marked non-inferable or if it is
    // an object literal type with at least one property of an inferable type. For example, an object
    // literal { a: 123, b: x => true } is marked non-inferable because it contains a context sensitive
    // arrow function, but is considered partially inferable because property 'a' has an inferable type.
    // inference.go:1060
    pub(crate) fn is_partially_inferable_type(&mut self, t: P<Type>) -> bool {
        !t.object_flags().intersects(ObjectFlags::NonInferrableType)
            || is_object_literal_type(t)
                && self.get_properties_of_type(t).into_iter().any(|prop| {
                    let prop_type = self.get_type_of_symbol(prop);
                    self.is_partially_inferable_type(prop_type)
                })
            || is_tuple_type(t) && self.get_element_types(t).into_iter().any(|t| self.is_partially_inferable_type(t))
    }

    // inference.go:1066
    pub(crate) fn infer_reverse_mapped_type(&mut self, source: P<Type>, target: P<Type>, constraint: P<Type>) -> Option<P<Type>> {
        let key = ReverseMappedTypeKey { source_id: source.id, target_id: target.id, constraint_id: constraint.id };
        if let Some(&cached) = self.reverse_mapped_cache.get(&key) {
            return Some(cached.unwrap_or(self.unknown_type));
        }
        self.reverse_mapped_source_stack.push(source);
        self.reverse_mapped_target_stack.push(target);
        let save_expanding_flags = self.reverse_expanding_flags;
        // The stacks are moved out for the duration of the (non-reentrant) isDeeplyNestedType calls.
        let source_stack = std::mem::take(&mut self.reverse_mapped_source_stack);
        let source_deeply_nested = self.is_deeply_nested_type(source, &source_stack, 2);
        self.reverse_mapped_source_stack = source_stack;
        if source_deeply_nested {
            self.reverse_expanding_flags |= ExpandingFlags::Source;
        }
        let target_stack = std::mem::take(&mut self.reverse_mapped_target_stack);
        let target_deeply_nested = self.is_deeply_nested_type(target, &target_stack, 2);
        self.reverse_mapped_target_stack = target_stack;
        if target_deeply_nested {
            self.reverse_expanding_flags |= ExpandingFlags::Target;
        }
        let mut t: Option<P<Type>> = None;
        if self.reverse_expanding_flags != ExpandingFlags::Both {
            t = Some(self.infer_reverse_mapped_type_worker(source, target, constraint));
        }
        self.reverse_mapped_source_stack.pop();
        self.reverse_mapped_target_stack.pop();
        self.reverse_expanding_flags = save_expanding_flags;
        self.reverse_mapped_cache.insert(key, t);
        t
    }

    // inference.go:1091
    pub(crate) fn infer_reverse_mapped_type_worker(&mut self, source: P<Type>, target: P<Type>, constraint: P<Type>) -> P<Type> {
        let mapped_type_parameter = self.get_type_parameter_from_mapped_type(target);
        let type_parameter = self.get_indexed_access_type(constraint.as_index_type().target().unwrap(), mapped_type_parameter);
        let template_type = self.get_template_type_from_mapped_type(target);
        let inference = new_inference_info(type_parameter);
        self.infer_types(&[inference], source, template_type, InferencePriority::None, false);
        let t = self.get_type_from_inference(inference).unwrap_or(self.unknown_type);
        self.get_widened_type(t)
    }

    // inference.go:1099
    pub(crate) fn resolve_reverse_mapped_type_members(&mut self, t: P<Type>) {
        let r = t.as_reverse_mapped_type();
        let r_source = r.source.get().unwrap();
        let r_mapped_type = r.mapped_type.get().unwrap();
        let r_constraint_type = r.constraint_type.get().unwrap();
        let string_type = self.string_type;
        let index_info = self.get_index_info_of_type(r_source, string_type);
        let modifiers = get_mapped_type_modifiers(r_mapped_type);
        let readonly_mask = !modifiers.intersects(MappedTypeModifiers::IncludeReadonly);
        let optional_mask = if modifiers.intersects(MappedTypeModifiers::IncludeOptional) { SymbolFlags::None } else { SymbolFlags::Optional };
        let mut index_infos: Vec<P<IndexInfo>> = Vec::new();
        if let Some(index_info) = index_info {
            let value_type = self.infer_reverse_mapped_type(index_info.value_type(), r_mapped_type, r_constraint_type).unwrap_or(self.unknown_type);
            index_infos = vec![self.new_index_info(string_type, value_type, readonly_mask && index_info.is_readonly(), None, &[])];
        }
        let members = SymbolTable::new();
        let limited_constraint = self.get_limited_constraint(t);
        for prop in self.get_properties_of_type(r_source) {
            // In case of a reverse mapped type with an intersection constraint, if we were able to
            // extract the filtering type literals we skip those properties that are not assignable to them,
            // because the extra properties wouldn't get through the application of the mapped type anyway
            if let Some(limited_constraint) = limited_constraint {
                let property_name_type = self.get_literal_type_from_property(prop, TypeFlags::StringOrNumberLiteralOrUnique, false);
                if !self.is_type_assignable_to(property_name_type, limited_constraint) {
                    continue;
                }
            }
            let check_flags = CheckFlags::ReverseMapped | if readonly_mask && self.is_readonly_symbol(prop) { CheckFlags::Readonly } else { CheckFlags::None };
            let inferred_prop = self.new_symbol_ex(SymbolFlags::Property | (prop.flags.get() & optional_mask), prop.name.get(), check_flags);
            *inferred_prop.declarations.borrow_mut() = prop.declarations.borrow().clone();
            let name_type = self.value_symbol_links.get(prop).name_type.get();
            self.value_symbol_links.get(inferred_prop).name_type.set(name_type);
            let links = self.reverse_mapped_symbol_links.get(inferred_prop);
            links.property_type.set(Some(self.get_type_of_symbol(prop)));
            let constraint_target = r_constraint_type.as_index_type().target().unwrap();
            if constraint_target.flags().intersects(TypeFlags::IndexedAccess)
                && constraint_target.as_indexed_access_type().object_type().unwrap().flags().intersects(TypeFlags::TypeParameter)
                && constraint_target.as_indexed_access_type().index_type().unwrap().flags().intersects(TypeFlags::TypeParameter)
            {
                // A reverse mapping of `{[K in keyof T[K_1]]: T[K_1]}` is the same as that of `{[K in keyof T]: T}`, since all we care about is
                // inferring to the "type parameter" (or indexed access) shared by the constraint and template. So, to reduce the number of
                // type identities produced, we simplify such indexed access occurrences
                let new_type_param = constraint_target.as_indexed_access_type().object_type().unwrap();
                let new_mapped_type = self.replace_indexed_access(r_mapped_type, constraint_target, new_type_param);
                links.mapped_type.set(Some(new_mapped_type));
                links.constraint_type.set(Some(self.get_index_type(new_type_param)));
            } else {
                links.mapped_type.set(Some(r_mapped_type));
                links.constraint_type.set(Some(r_constraint_type));
            }
            members.set(prop.name.get(), inferred_prop);
        }
        self.set_structured_type_members(t, Some(members), &[], &[], &index_infos);
    }

    // inference.go:1145
    pub(crate) fn get_type_of_reverse_mapped_symbol(&mut self, symbol: P<Symbol>) -> P<Type> {
        let links = self.value_symbol_links.get(symbol);
        if links.resolved_type.get().is_none() {
            let reverse_links = self.reverse_mapped_symbol_links.get(symbol);
            let t = self
                .infer_reverse_mapped_type(reverse_links.property_type.get().unwrap(), reverse_links.mapped_type.get().unwrap(), reverse_links.constraint_type.get().unwrap())
                .unwrap_or(self.unknown_type);
            links.resolved_type.set(Some(t));
        }
        links.resolved_type.get().unwrap()
    }

    // If the original mapped type had an intersection constraint we extract its components,
    // and we make an attempt to do so even if the intersection has been reduced to a union.
    // This entire process allows us to possibly retrieve the filtering type literals.
    // e.g. { [K in keyof U & ("a" | "b") ] } -> "a" | "b"
    // inference.go:1158
    pub(crate) fn get_limited_constraint(&mut self, t: P<Type>) -> Option<P<Type>> {
        let constraint = self.get_constraint_type_from_mapped_type(t.as_reverse_mapped_type().mapped_type.get().unwrap());
        if !(constraint.flags().intersects(TypeFlags::Union) || constraint.flags().intersects(TypeFlags::Intersection)) {
            return None;
        }
        let mut origin = Some(constraint);
        if constraint.flags().intersects(TypeFlags::Union) {
            origin = constraint.as_union_type().origin.get();
        }
        let origin = match origin {
            Some(origin) if origin.flags().intersects(TypeFlags::Intersection) => origin,
            _ => return None,
        };
        let constraint_type = t.as_reverse_mapped_type().constraint_type.get();
        let filtered: Vec<P<Type>> = origin.types().iter().copied().filter(|&t| Some(t) != constraint_type).collect();
        let limited_constraint = self.get_intersection_type(&filtered);
        if limited_constraint != self.never_type {
            return Some(limited_constraint);
        }
        None
    }

    // inference.go:1178
    pub(crate) fn replace_indexed_access(&mut self, instantiable: P<Type>, t: P<Type>, replacement: P<Type>) -> P<Type> {
        // map type.indexType to 0
        // map type.objectType to `[TReplacement]`
        // thus making the indexed access `[TReplacement][0]` or `TReplacement`
        let sources = alloc_vec(vec![t.as_indexed_access_type().index_type().unwrap(), t.as_indexed_access_type().object_type().unwrap()]);
        let zero = self.get_number_literal_type(jsnum::Number(0.0));
        let tuple = self.create_tuple_type(&[replacement]);
        let targets = alloc_vec(vec![zero, tuple]);
        self.instantiate_type(instantiable, Some(new_type_mapper(sources, targets)))
    }

    // inference.go:1185
    pub(crate) fn types_definitely_unrelated(&mut self, source: P<Type>, target: P<Type>) -> bool {
        // Two tuple types with incompatible arities are definitely unrelated.
        // Two object types that each have a property that is unmatched in the other are definitely unrelated.
        if is_tuple_type(source) && is_tuple_type(target) {
            return tuple_types_definitely_unrelated(source, target);
        }
        self.get_unmatched_property(source, target, false /*requireOptionalProperties*/, true /*matchDiscriminantProperties*/).is_some()
            && self.get_unmatched_property(target, source, false /*requireOptionalProperties*/, false /*matchDiscriminantProperties*/).is_some()
    }
}

// inference.go:1195
pub(crate) fn tuple_types_definitely_unrelated(source: P<Type>, target: P<Type>) -> bool {
    let s = source.target_tuple_type();
    let t = target.target_tuple_type();
    !t.combined_flags.get().intersects(ElementFlags::Variadic) && t.min_length.get() > s.min_length.get()
        || !t.combined_flags.get().intersects(ElementFlags::Variable) && (s.combined_flags.get().intersects(ElementFlags::Variable) || t.fixed_length.get() < s.fixed_length.get())
}

impl Checker {
    // inference.go:1202
    pub(crate) fn is_tuple_type_structure_matching(&mut self, t1: P<Type>, t2: P<Type>) -> bool {
        if self.get_type_reference_arity(t1) != self.get_type_reference_arity(t2) {
            return false;
        }
        for (i, e) in t1.target_tuple_type().element_infos().iter().enumerate() {
            if (e.flags & ElementFlags::Variable) != (t2.target_tuple_type().element_infos()[i].flags & ElementFlags::Variable) {
                return false;
            }
        }
        true
    }

    // inference.go:1214
    pub(crate) fn is_type_or_base_identical_to(&mut self, s: P<Type>, t: P<Type>) -> bool {
        if t == self.missing_type {
            return s == t;
        }
        self.is_type_identical_to(s, t)
            || t.flags().intersects(TypeFlags::String) && s.flags().intersects(TypeFlags::StringLiteral)
            || t.flags().intersects(TypeFlags::Number) && s.flags().intersects(TypeFlags::NumberLiteral)
    }

    // inference.go:1223
    pub(crate) fn is_type_closely_matched_by(&mut self, s: P<Type>, t: P<Type>) -> bool {
        s.flags().intersects(TypeFlags::Object) && t.flags().intersects(TypeFlags::Object) && s.symbol().is_some() && s.symbol() == t.symbol()
            || s.alias().is_some() && t.alias().is_some() && !s.alias().type_arguments().is_empty() && s.alias().symbol() == t.alias().symbol()
    }

    // Create an object with properties named in the string literal type. Every property has type `any`.
    // inference.go:1229
    pub(crate) fn create_empty_object_type_from_string_literal(&mut self, t: P<Type>) -> P<Type> {
        let members = SymbolTable::new();
        for t in t.distributed() {
            if !t.flags().intersects(TypeFlags::StringLiteral) {
                continue;
            }
            let name = get_string_literal_value(t);
            let literal_prop = self.new_symbol(SymbolFlags::Property, &name);
            self.value_symbol_links.get(literal_prop).resolved_type.set(Some(self.any_type));
            if let Some(symbol) = t.symbol() {
                *literal_prop.declarations.borrow_mut() = symbol.declarations.borrow().clone();
                literal_prop.value_declaration.set(symbol.value_declaration.get());
            }
            members.set(literal_prop.name.get(), literal_prop);
        }
        let mut index_infos: Vec<P<IndexInfo>> = Vec::new();
        if t.flags().intersects(TypeFlags::String) {
            let (string_type, empty_object_type) = (self.string_type, self.empty_object_type);
            index_infos = vec![self.new_index_info(string_type, empty_object_type, false /*isReadonly*/, None, &[])];
        }
        self.new_anonymous_type(None, Some(members), &[], &[], &index_infos)
    }

    // inference.go:1251
    pub(crate) fn new_inference_context(&mut self, type_parameters: &[P<Type>], signature: Option<P<Signature>>, flags: InferenceFlags, compare_types: Option<TypeComparer>) -> P<InferenceContext> {
        let compare_types = match compare_types {
            Some(compare_types) => compare_types,
            None => self.compare_types_assignable_comparer(),
        };
        let inferences: Vec<P<InferenceInfo>> = type_parameters.iter().map(|&tp| new_inference_info(tp)).collect();
        self.new_inference_context_worker(&inferences, signature, flags, compare_types)
    }

    // inference.go:1258
    // n and the result are Option (Go checks n for nil; callers pass getInferenceContext results).
    pub(crate) fn clone_inference_context(&mut self, n: Option<P<InferenceContext>>, extra_flags: InferenceFlags) -> Option<P<InferenceContext>> {
        let n = n?;
        let inferences: Vec<P<InferenceInfo>> = n.inferences.get().iter().map(|&info| clone_inference_info(info)).collect();
        Some(self.new_inference_context_worker(&inferences, n.signature.get(), n.flags.get() | extra_flags, n.compare_types.get().unwrap()))
    }

    // inference.go:1265
    pub(crate) fn clone_inferred_part_of_context(&mut self, n: P<InferenceContext>) -> Option<P<InferenceContext>> {
        let inferences: Vec<P<InferenceInfo>> = n.inferences.get().iter().copied().filter(|&info| has_inference_candidates(info)).collect();
        if inferences.is_empty() {
            return None;
        }
        let inferences: Vec<P<InferenceInfo>> = inferences.iter().map(|&info| clone_inference_info(info)).collect();
        Some(self.new_inference_context_worker(&inferences, n.signature.get(), n.flags.get(), n.compare_types.get().unwrap()))
    }

    // inference.go:1273
    pub(crate) fn new_inference_context_worker(&mut self, inferences: &[P<InferenceInfo>], signature: Option<P<Signature>>, flags: InferenceFlags, compare_types: TypeComparer) -> P<InferenceContext> {
        let n = P::new(InferenceContext {
            inferences: Cell::new(alloc_slice(inferences)),
            signature: Cell::new(signature),
            flags: Cell::new(flags),
            compare_types: Cell::new(Some(compare_types)),
            ..Default::default()
        });
        n.mapper.set(Some(self.new_inference_type_mapper(n, true /*fixing*/)));
        n.non_fixing_mapper.set(Some(self.new_inference_type_mapper(n, false /*fixing*/)));
        n
    }

    // inference.go:1285
    pub(crate) fn add_intra_expression_inference_site(&mut self, n: P<InferenceContext>, node: P<Node>, t: P<Type>) {
        n.intra_expression_inference_sites.borrow_mut().push(IntraExpressionInferenceSite { node, t });
    }

    // We collect intra-expression inference sites within object and array literals to handle cases where
    // inferred types flow between context sensitive element expressions. For example:
    //
    //	declare function foo<T>(arg: [(n: number) => T, (x: T) => void]): void;
    //	foo([_a => 0, n => n.toFixed()]);
    //
    // Above, both arrow functions in the tuple argument are context sensitive, thus both are omitted from the
    // pass that collects inferences from the non-context sensitive parts of the arguments. In the subsequent
    // pass where nothing is omitted, we need to commit to an inference for T in order to contextually type the
    // parameter in the second arrow function, but we want to first infer from the return type of the first
    // arrow function. This happens automatically when the arrow functions are discrete arguments (because we
    // infer from each argument before processing the next), but when the arrow functions are elements of an
    // object or array literal, we need to perform intra-expression inferences early.
    // inference.go:1302
    pub(crate) fn infer_from_intra_expression_sites(&mut self, n: P<InferenceContext>) {
        let sites = n.intra_expression_inference_sites.borrow().clone();
        for site in sites {
            let contextual_type = if ast::is_method_declaration(site.node) {
                self.get_contextual_type_for_object_literal_method(site.node, ContextFlags::NoConstraints)
            } else {
                self.get_contextual_type(site.node, ContextFlags::NoConstraints)
            };
            if let Some(contextual_type) = contextual_type {
                self.infer_types(n.inferences.get(), site.t, contextual_type, InferencePriority::None, false);
            }
        }
        n.intra_expression_inference_sites.borrow_mut().clear();
    }

    // inference.go:1317
    pub(crate) fn get_inferred_type(&mut self, n: P<InferenceContext>, index: i32) -> P<Type> {
        let inference = n.inferences.get()[index as usize];
        if inference.inferred_type.get().is_none() {
            if inference.type_parameter.get() == Some(self.error_type) {
                return inference.type_parameter.get().unwrap();
            }
            let type_parameter = inference.type_parameter.get().unwrap();
            let mut inferred_type: Option<P<Type>> = None;
            let mut fallback_type: Option<P<Type>> = None;
            if let Some(signature) = n.signature.get() {
                let mut inferred_covariant_type: Option<P<Type>> = None;
                if !inference.candidates.borrow().is_empty() {
                    inferred_covariant_type = Some(self.get_covariant_inference(inference, signature));
                }
                let mut inferred_contravariant_type: Option<P<Type>> = None;
                if !inference.contra_candidates.borrow().is_empty() {
                    inferred_contravariant_type = self.get_contravariant_inference(inference);
                }
                if inferred_covariant_type.is_some() || inferred_contravariant_type.is_some() {
                    // If we have both co- and contra-variant inferences, we prefer the co-variant inference if it is not 'never',
                    // all co-variant inferences are assignable to it (i.e. it isn't one of a conflicting set of candidates), it is
                    // assignable to some contra-variant inference, and no other type parameter is constrained to this type parameter
                    // and has inferences that would conflict. Otherwise, we prefer the contra-variant inference.
                    // Similarly ignore co-variant `any` inference when both are available as almost everything is assignable to it
                    // and it would spoil the overall inference.
                    let prefer_covariant_type = match inferred_covariant_type {
                        None => false,
                        Some(covariant) => {
                            inferred_contravariant_type.is_none()
                                || !covariant.flags().intersects(TypeFlags::Never | TypeFlags::Any)
                                    && {
                                        let contra_candidates = inference.contra_candidates.borrow().clone();
                                        contra_candidates.iter().any(|&t| self.is_type_assignable_to(covariant, t))
                                    }
                                    && n.inferences.get().iter().all(|&other| {
                                        other != inference && self.get_constraint_of_type_parameter(other.type_parameter.get().unwrap()) != inference.type_parameter.get() || {
                                            let other_candidates = other.candidates.borrow().clone();
                                            other_candidates.iter().all(|&t| self.is_type_assignable_to(t, covariant))
                                        }
                                    })
                        }
                    };
                    if prefer_covariant_type {
                        inferred_type = inferred_covariant_type;
                        fallback_type = inferred_contravariant_type;
                    } else {
                        inferred_type = inferred_contravariant_type;
                        fallback_type = inferred_covariant_type;
                    }
                } else if n.flags.get().intersects(InferenceFlags::NoDefault) {
                    // We use silentNeverType as the wildcard that signals no inferences.
                    inferred_type = Some(self.silent_never_type);
                } else {
                    // Infer either the default or the empty object type when no inferences were
                    // made. It is important to remember that in this case, inference still
                    // succeeds, meaning there is no error for not having inference candidates. An
                    // inference error only occurs when there are *conflicting* candidates, i.e.
                    // candidates with no common supertype.
                    let default_type = self.get_default_from_type_parameter(type_parameter);
                    if let Some(default_type) = default_type {
                        // Instantiate the default type. Any forward reference to a type
                        // parameter should be instantiated to the empty object type.
                        let backreference_mapper = self.new_backreference_mapper(n, index);
                        let mapper = merge_type_mappers(Some(backreference_mapper), n.non_fixing_mapper.get().unwrap());
                        inferred_type = Some(self.instantiate_type(default_type, Some(mapper)));
                    }
                }
            } else {
                inferred_type = self.get_type_from_inference(inference);
            }
            inference.inferred_type.set(inferred_type);
            if inference.inferred_type.get().is_none() {
                inference.inferred_type.set(Some(if n.flags.get().intersects(InferenceFlags::AnyDefault) { self.any_type } else { self.unknown_type }));
            }
            let constraint = self.get_constraint_of_type_parameter(type_parameter);
            if let Some(constraint) = constraint {
                let instantiated_constraint = self.instantiate_type(constraint, n.non_fixing_mapper.get());
                let compare_types = n.compare_types.get().unwrap();
                if let Some(inferred) = inferred_type {
                    if !n.flags.get().intersects(InferenceFlags::NoConstraintChecks) {
                        let constraint_with_this = self.get_type_with_this_argument(instantiated_constraint, Some(inferred), false);
                        if compare_types(self, inferred, constraint_with_this, false) == Ternary::False {
                            let mut filtered_by_constraint: Option<P<Type>> = None;
                            if inference.priority.get() == InferencePriority::ReturnType {
                                // If we have a pure return type inference, we may succeed by removing constituents of the inferred type
                                // that aren't assignable to the constraint type (pure return type inferences are speculation anyway).
                                filtered_by_constraint = self.map_type(inferred, |c, t| {
                                    Some(if compare_types(c, t, constraint_with_this, false) != Ternary::False { t } else { c.never_type })
                                });
                            }
                            inferred_type = match filtered_by_constraint {
                                Some(filtered) if !filtered.flags().intersects(TypeFlags::Never) => Some(filtered),
                                _ => None,
                            };
                        }
                    }
                }
                if inferred_type.is_none() {
                    // If the fallback type satisfies the constraint, we pick it. Otherwise, we pick the constraint.
                    inferred_type = Some(match fallback_type {
                        Some(fallback) if {
                            let fallback_constraint = self.get_type_with_this_argument(instantiated_constraint, Some(fallback), false);
                            compare_types(self, fallback, fallback_constraint, false) != Ternary::False
                        } =>
                        {
                            fallback
                        }
                        _ => instantiated_constraint,
                    });
                }
                inference.inferred_type.set(inferred_type);
            }
            self.clear_active_mapper_caches();
        }
        inference.inferred_type.get().unwrap()
    }

    // inference.go:1406
    pub(crate) fn get_inferred_types(&mut self, n: P<InferenceContext>) -> Vec<P<Type>> {
        let len = n.inferences.get().len();
        let mut result = Vec::with_capacity(len);
        for i in 0..len {
            result.push(self.get_inferred_type(n, i as i32));
        }
        result
    }

    // inference.go:1414
    // n and the result are Option (Go returns nil for a nil context; callers pass nil-able contexts).
    pub(crate) fn get_mapper_from_context(&mut self, n: Option<P<InferenceContext>>) -> Option<P<TypeMapper>> {
        let n = n?;
        n.mapper.get()
    }

    // Return a type mapper that combines the context's return mapper with a mapper that erases any additional type parameters
    // to their inferences at the time of creation.
    // inference.go:1423
    pub(crate) fn create_outer_return_mapper(&mut self, context: P<InferenceContext>) -> P<TypeMapper> {
        if context.outer_return_mapper.get().is_none() {
            let mut mapper = self.clone_inference_context(Some(context), InferenceFlags::None).unwrap().mapper.get().unwrap();
            if let Some(return_mapper) = context.return_mapper.get() {
                mapper = new_merged_type_mapper(return_mapper, mapper);
            }
            context.outer_return_mapper.set(Some(mapper));
        }
        context.outer_return_mapper.get().unwrap()
    }

    // inference.go:1434
    pub(crate) fn get_covariant_inference(&mut self, inference: P<InferenceInfo>, signature: P<Signature>) -> P<Type> {
        // Extract all object and array literal types and replace them with a single widened and normalized type.
        let inference_candidates = inference.candidates.borrow().clone();
        let candidates = self.union_object_and_array_literal_candidates(&inference_candidates);
        // We widen inferred literal types if
        // all inferences were made to top-level occurrences of the type parameter, and
        // the type parameter has no constraint or its constraint includes no primitive or literal types, and
        // the type parameter was fixed during inference or does not occur at top-level in the return type.
        let type_parameter = inference.type_parameter.get().unwrap();
        let primitive_constraint = self.has_primitive_constraint(type_parameter) || self.is_const_type_variable(Some(type_parameter), 0);
        let widen_literal_types = !primitive_constraint && inference.top_level.get() && (inference.is_fixed.get() || !self.is_type_parameter_at_top_level_in_return_type(signature, type_parameter));
        let base_candidates: Vec<P<Type>> = if primitive_constraint {
            candidates.iter().map(|&t| self.get_regular_type_of_literal_type(t)).collect()
        } else if widen_literal_types {
            candidates.iter().map(|&t| self.get_widened_literal_type(t)).collect()
        } else {
            candidates
        };
        // If all inferences were made from a position that implies a combined result, infer a union type.
        // Otherwise, infer a common supertype.
        let unwidened_type = if inference.priority.get().intersects(InferencePriority::PriorityImpliesCombination) {
            self.get_union_type_ex(&base_candidates, UnionReduction::Subtype, None, None)
        } else {
            self.get_common_supertype(&base_candidates).unwrap()
        };
        self.get_widened_type(unwidened_type)
    }

    // inference.go:1463
    pub(crate) fn get_contravariant_inference(&mut self, inference: P<InferenceInfo>) -> Option<P<Type>> {
        let contra_candidates = inference.contra_candidates.borrow().clone();
        if inference.priority.get().intersects(InferencePriority::PriorityImpliesCombination) {
            return Some(self.get_intersection_type(&contra_candidates));
        }
        self.get_common_subtype(&contra_candidates)
    }

    // inference.go:1470
    pub(crate) fn union_object_and_array_literal_candidates(&mut self, candidates: &[P<Type>]) -> Vec<P<Type>> {
        if candidates.len() > 1 {
            let object_literals: Vec<P<Type>> = candidates.iter().copied().filter(|&t| is_object_or_array_literal_type(t)).collect();
            if !object_literals.is_empty() {
                let literals_type = self.get_union_type_ex(&object_literals, UnionReduction::Subtype, None, None);
                let mut non_literal_types: Vec<P<Type>> = candidates.iter().copied().filter(|&t| !is_object_or_array_literal_type(t)).collect();
                non_literal_types.push(literals_type);
                return non_literal_types;
            }
        }
        candidates.to_vec()
    }

    // inference.go:1482
    pub(crate) fn has_primitive_constraint(&mut self, t: P<Type>) -> bool {
        let constraint = self.get_constraint_of_type_parameter(t);
        if let Some(mut constraint) = constraint {
            if constraint.flags().intersects(TypeFlags::Conditional) {
                constraint = self.get_default_constraint_of_conditional_type(constraint);
            }
            return self.maybe_type_of_kind(constraint, TypeFlags::Primitive | TypeFlags::Index | TypeFlags::TemplateLiteral | TypeFlags::StringMapping);
        }
        false
    }

    // inference.go:1493
    pub(crate) fn is_type_parameter_at_top_level(&mut self, t: P<Type>, tp: P<Type>, depth: i32) -> bool {
        t == tp
            || t.flags().intersects(TypeFlags::UnionOrIntersection) && t.types().iter().any(|&t| self.is_type_parameter_at_top_level(t, tp, depth))
            || depth < 3 && t.flags().intersects(TypeFlags::Conditional) && {
                let true_type = self.get_true_type_from_conditional_type(t);
                self.is_type_parameter_at_top_level(true_type, tp, depth + 1) || {
                    let false_type = self.get_false_type_from_conditional_type(t);
                    self.is_type_parameter_at_top_level(false_type, tp, depth + 1)
                }
            }
    }

    // inference.go:1501
    pub(crate) fn is_type_parameter_at_top_level_in_return_type(&mut self, signature: P<Signature>, type_parameter: P<Type>) -> bool {
        let type_predicate = self.get_type_predicate_of_signature(signature);
        if let Some(type_predicate) = type_predicate {
            return match type_predicate.t.get() {
                Some(t) => self.is_type_parameter_at_top_level(t, type_parameter, 0),
                None => false,
            };
        }
        let return_type = self.get_return_type_of_signature(signature);
        self.is_type_parameter_at_top_level(return_type, type_parameter, 0)
    }

    // inference.go:1509
    pub(crate) fn get_type_from_inference(&mut self, inference: P<InferenceInfo>) -> Option<P<Type>> {
        if !inference.candidates.borrow().is_empty() {
            let candidates = inference.candidates.borrow().clone();
            return Some(self.get_union_type_ex(&candidates, UnionReduction::Subtype, None, None));
        }
        if !inference.contra_candidates.borrow().is_empty() {
            let contra_candidates = inference.contra_candidates.borrow().clone();
            return Some(self.get_intersection_type(&contra_candidates));
        }
        None
    }
}

// inference.go:1519
pub(crate) fn get_inference_info_for_type(n: P<InferenceState>, t: P<Type>) -> Option<P<InferenceInfo>> {
    if t.flags().intersects(TypeFlags::TypeVariable) {
        let t = get_non_distributed_type_parameter(t);
        for &inference in n.inferences.borrow().iter() {
            if t == inference.type_parameter.get() {
                return Some(inference);
            }
        }
    }
    None
}

impl Checker {
    // inference.go:1531
    pub(crate) fn get_common_supertype(&mut self, types: &[P<Type>]) -> Option<P<Type>> {
        if types.len() == 1 {
            return Some(types[0]);
        }
        // Remove nullable types from each of the candidates.
        let mut primary_types: Vec<P<Type>> = types.to_vec();
        if self.strict_null_checks {
            primary_types = types.iter().map(|&t| self.filter_type(t, |_, u| !u.flags().intersects(TypeFlags::Nullable))).collect();
        }
        // core.Same(primaryTypes, types): SameMap returns the original slice iff no element changed.
        let same = primary_types.iter().zip(types.iter()).all(|(a, b)| a == b);
        // When the candidate types are all literal types with the same base type, return a union
        // of those literal types. Otherwise, return the leftmost type for which no type to the
        // right is a supertype.
        let supertype = if self.literal_types_with_same_base_type(&primary_types) {
            Some(self.get_union_type(&primary_types))
        } else {
            self.get_single_common_supertype(&primary_types)
        };
        // Add any nullable types that occurred in the candidates back to the result.
        if same {
            return supertype;
        }
        let nullable_flags = self.get_combined_type_flags(types) & TypeFlags::Nullable;
        Some(self.get_nullable_type(supertype.unwrap(), nullable_flags))
    }

    // inference.go:1558
    pub(crate) fn get_single_common_supertype(&mut self, types: &[P<Type>]) -> Option<P<Type>> {
        // First, find the leftmost type for which no type to the right is a strict supertype, and if that
        // type is a strict supertype of all other candidates, return it. Otherwise, return the leftmost type
        // for which no type to the right is a (regular) supertype.
        let candidate = self.find_leftmost_type(types, |c, s, t| c.is_type_strict_subtype_of(s, t));
        if types.iter().all(|&t| Some(t) == candidate || self.is_type_strict_subtype_of(t, candidate.unwrap())) {
            return candidate;
        }
        self.find_leftmost_type(types, |c, s, t| c.is_type_subtype_of(s, t))
    }

    // inference.go:1569
    pub(crate) fn find_leftmost_type(&mut self, types: &[P<Type>], mut f: impl FnMut(&mut Checker, P<Type>, P<Type>) -> bool) -> Option<P<Type>> {
        let mut candidate: Option<P<Type>> = None;
        for &t in types {
            if candidate.is_none() || f(self, candidate.unwrap(), t) {
                candidate = Some(t);
            }
        }
        candidate
    }

    // Return the leftmost type for which no type to the right is a subtype.
    // inference.go:1580
    pub(crate) fn get_common_subtype(&mut self, types: &[P<Type>]) -> Option<P<Type>> {
        let mut subtype: Option<P<Type>> = None;
        for &t in types {
            if subtype.is_none() || self.is_type_subtype_of(t, subtype.unwrap()) {
                subtype = Some(t);
            }
        }
        subtype
    }

    // inference.go:1590
    pub(crate) fn get_combined_type_flags(&mut self, types: &[P<Type>]) -> TypeFlags {
        let mut flags = TypeFlags::None;
        for &t in types {
            if t.flags().intersects(TypeFlags::Union) {
                flags |= self.get_combined_type_flags(t.types());
            } else {
                flags |= t.flags();
            }
        }
        flags
    }

    // inference.go:1602
    pub(crate) fn literal_types_with_same_base_type(&mut self, types: &[P<Type>]) -> bool {
        let mut common_base_type: Option<P<Type>> = None;
        for &t in types {
            if !t.flags().intersects(TypeFlags::Never) {
                let base_type = self.get_base_type_of_literal_type(t);
                if common_base_type.is_none() {
                    common_base_type = Some(base_type);
                }
                if base_type == t || Some(base_type) != common_base_type {
                    return false;
                }
            }
        }
        true
    }

    // inference.go:1618
    pub(crate) fn is_from_inference_blocked_source(&mut self, t: P<Type>) -> bool {
        match t.symbol() {
            Some(symbol) => {
                let declarations = symbol.declarations.borrow().clone();
                declarations.iter().any(|&d| self.is_skip_direct_inference_node(d))
            }
            None => false,
        }
    }

    // inference.go:1622
    pub(crate) fn is_skip_direct_inference_node(&mut self, node: P<Node>) -> bool {
        self.skip_direct_inference_nodes.has(&node)
    }
}

// inference.go:1626
pub(crate) fn new_inference_info(type_parameter: P<Type>) -> P<InferenceInfo> {
    P::new(InferenceInfo {
        type_parameter: Cell::new(Some(type_parameter)),
        priority: Cell::new(InferencePriority::MaxValue),
        top_level: Cell::new(true),
        implied_arity: Cell::new(-1),
        ..Default::default()
    })
}

// inference.go:1630
pub(crate) fn clone_inference_info(info: P<InferenceInfo>) -> P<InferenceInfo> {
    P::new(InferenceInfo {
        type_parameter: Cell::new(info.type_parameter.get()),
        candidates: RefCell::new(info.candidates.borrow().clone()),
        contra_candidates: RefCell::new(info.contra_candidates.borrow().clone()),
        inferred_type: Cell::new(info.inferred_type.get()),
        priority: Cell::new(info.priority.get()),
        top_level: Cell::new(info.top_level.get()),
        is_fixed: Cell::new(info.is_fixed.get()),
        implied_arity: Cell::new(info.implied_arity.get()),
    })
}

// inference.go:1643
pub(crate) fn clear_cached_inferences(inferences: &[P<InferenceInfo>]) {
    for inference in inferences {
        if !inference.is_fixed.get() {
            inference.inferred_type.set(None);
        }
    }
}

// inference.go:1651
pub(crate) fn has_inference_candidates(info: P<InferenceInfo>) -> bool {
    !info.candidates.borrow().is_empty() || !info.contra_candidates.borrow().is_empty()
}

// inference.go:1655
pub(crate) fn has_inference_candidates_or_default(info: P<InferenceInfo>) -> bool {
    has_inference_candidates(info) || has_type_parameter_default(info.type_parameter.get().unwrap())
}

// inference.go:1659
pub(crate) fn has_type_parameter_default(tp: P<Type>) -> bool {
    if let Some(symbol) = tp.symbol() {
        for &d in symbol.declarations.borrow().iter() {
            if ast::is_type_parameter_declaration(d) && d.as_type_parameter_declaration().default_type().is_some() {
                return true;
            }
        }
    }
    false
}

// inference.go:1670
pub(crate) fn has_overlapping_inferences(a: &[P<InferenceInfo>], b: &[P<InferenceInfo>]) -> bool {
    for i in 0..a.len() {
        if has_inference_candidates(a[i]) && has_inference_candidates(b[i]) {
            return true;
        }
    }
    false
}

impl Checker {
    // inference.go:1679
    pub(crate) fn merge_inferences(&mut self, target: &mut [P<InferenceInfo>], source: &[P<InferenceInfo>]) {
        for i in 0..target.len() {
            if !has_inference_candidates(target[i]) && has_inference_candidates(source[i]) {
                target[i] = source[i];
            }
        }
    }
}
