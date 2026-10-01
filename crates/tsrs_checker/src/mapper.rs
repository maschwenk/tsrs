use crate::*;
use tsrs_core::SlicePair;

// TypeMapperKind

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum TypeMapperKind {
    #[default]
    Unknown,
    Simple,
    Array,
    Merged,
}

// TypeMapper

/// Go's `TypeMapper` + `TypeMapperData` implementations. Mappers that Go implements with a stored checker or captured
/// closures receive the checker as a parameter of `map` instead.
pub struct TypeMapper {
    pub data: TypeMapperData,
}

pub enum TypeMapperData {
    Simple { source: P<Type>, target: P<Type> },
    Array { sources_targets: SlicePair<P<Type>, P<Type>> },
    ArrayToSingle { sources: &'static [P<Type>], target: P<Type> },
    Deferred { data: &'static DeferredTypeMapper },
    Function { f: fn(&mut Checker, P<Type>) -> P<Type> },
    Merged { m1: P<TypeMapper>, m2: P<TypeMapper> },
    Composite { m1: P<TypeMapper>, m2: P<TypeMapper> },
    Inference { n: P<InferenceContext>, fixing: bool },
}

// Rare; kept out of line so `TypeMapperData` stays 32 bytes (checked below).
pub struct DeferredTypeMapper {
    pub sources: &'static [P<Type>],
    pub targets: Vec<Box<dyn Fn(&mut Checker) -> P<Type>>>,
}

const _: () = assert!(std::mem::size_of::<TypeMapper>() == 32);

impl TypeMapper {
    pub fn map(&self, c: &mut Checker, t: P<Type>) -> P<Type> {
        match &self.data {
            TypeMapperData::Simple { source, target } => {
                if t == *source {
                    *target
                } else {
                    t
                }
            }
            TypeMapperData::Array { sources_targets } => {
                let (sources, targets) = (sources_targets.first(), sources_targets.second());
                for (i, s) in sources.iter().enumerate() {
                    if t == *s {
                        return targets[i];
                    }
                }
                t
            }
            TypeMapperData::ArrayToSingle { sources, target } => {
                if sources.contains(&t) {
                    *target
                } else {
                    t
                }
            }
            TypeMapperData::Deferred { data } => {
                for (i, s) in data.sources.iter().enumerate() {
                    if t == *s {
                        return data.targets[i](c);
                    }
                }
                t
            }
            TypeMapperData::Function { f } => f(c, t),
            TypeMapperData::Merged { m1, m2 } => {
                let t1 = m1.map(c, t);
                m2.map(c, t1)
            }
            TypeMapperData::Composite { m1, m2 } => {
                let t1 = m1.map(c, t);
                if t1 != t {
                    return c.instantiate_type(t1, Some(*m2));
                }
                m2.map(c, t)
            }
            TypeMapperData::Inference { n, fixing } => {
                let inferences = n.inferences.get();
                for (i, inference) in inferences.iter().enumerate() {
                    if Some(t) == inference.type_parameter.get() {
                        if *fixing && !inference.is_fixed.get() {
                            // Before we commit to a particular inference (and thus lock out any further inferences),
                            // we infer from any intra-expression inference sites we have collected.
                            c.infer_from_intra_expression_sites(*n);
                            clear_cached_inferences(n.inferences.get());
                            inference.is_fixed.set(true);
                        }
                        return c.get_inferred_type(*n, i as i32);
                    }
                }
                t
            }
        }
    }

    pub fn kind(&self) -> TypeMapperKind {
        match self.data {
            TypeMapperData::Simple { .. } => TypeMapperKind::Simple,
            TypeMapperData::Array { .. } => TypeMapperKind::Array,
            TypeMapperData::Merged { .. } => TypeMapperKind::Merged,
            _ => TypeMapperKind::Unknown,
        }
    }

    pub fn maps_this_only(&self) -> bool {
        match &self.data {
            TypeMapperData::Simple { source, .. } => is_this_type_parameter(*source),
            TypeMapperData::Array { sources_targets } => {
                let sources = sources_targets.first();
                sources.len() == 1 && is_this_type_parameter(sources[0])
            }
            TypeMapperData::ArrayToSingle { sources, .. } | TypeMapperData::Deferred { data: DeferredTypeMapper { sources, .. } } => {
                sources.len() == 1 && is_this_type_parameter(sources[0])
            }
            _ => false,
        }
    }
}

// Factory functions

impl Checker {
    /// Go free function `getMappedType(t, mapper)`; a method because mapping may need the checker.
    pub(crate) fn get_mapped_type(&mut self, t: P<Type>, mapper: P<TypeMapper>) -> P<Type> {
        mapper.map(self, get_non_distributed_type_parameter(t).unwrap())
    }

    pub(crate) fn combine_type_mappers(&mut self, m1: Option<P<TypeMapper>>, m2: P<TypeMapper>) -> P<TypeMapper> {
        if let Some(m1) = m1 {
            return new_composite_type_mapper(m1, m2);
        }
        m2
    }

    pub(crate) fn map_type_with_composite_mapper(&mut self, t: P<Type>, m1: Option<P<TypeMapper>>, m2: P<TypeMapper>) -> P<Type> {
        let Some(m1) = m1 else {
            return self.get_mapped_type(t, m2);
        };
        let t1 = self.get_mapped_type(t, m1);
        if t1 != t {
            return self.instantiate_type(t1, Some(m2));
        }
        self.get_mapped_type(t, m2)
    }

    /// Maps forward-references to later types parameters to the empty object type.
    /// This is used during inference when instantiating type parameter defaults.
    pub(crate) fn new_backreference_mapper(&mut self, context: P<InferenceContext>, index: i32) -> P<TypeMapper> {
        let forward_inferences = &context.inferences.get()[index as usize..];
        let type_parameters: Vec<P<Type>> = forward_inferences.iter().map(|i| i.type_parameter.get().unwrap()).collect();
        new_array_to_single_type_mapper(alloc_slice(&type_parameters), self.unknown_type)
    }

    pub(crate) fn new_inference_type_mapper(&mut self, n: P<InferenceContext>, fixing: bool) -> P<TypeMapper> {
        P::new(TypeMapper { data: TypeMapperData::Inference { n, fixing } })
    }
}

pub(crate) fn new_type_mapper(sources: &'static [P<Type>], targets: &'static [P<Type>]) -> P<TypeMapper> {
    if sources.len() == 1 {
        return new_simple_type_mapper(sources[0], targets[0]);
    }
    new_array_type_mapper(sources, targets)
}

pub(crate) fn merge_type_mappers(m1: Option<P<TypeMapper>>, m2: P<TypeMapper>) -> P<TypeMapper> {
    if let Some(m1) = m1 {
        return new_merged_type_mapper(m1, m2);
    }
    m2
}

pub(crate) fn prepend_type_mapping(source: P<Type>, target: P<Type>, mapper: Option<P<TypeMapper>>) -> P<TypeMapper> {
    let Some(mapper) = mapper else {
        return new_simple_type_mapper(get_non_distributed_type_parameter(source).unwrap(), target);
    };
    new_merged_type_mapper(new_simple_type_mapper(get_non_distributed_type_parameter(source).unwrap(), target), mapper)
}

pub(crate) fn append_type_mapping(mapper: Option<P<TypeMapper>>, source: P<Type>, target: P<Type>) -> P<TypeMapper> {
    let Some(mapper) = mapper else {
        return new_simple_type_mapper(get_non_distributed_type_parameter(source).unwrap(), target);
    };
    new_merged_type_mapper(mapper, new_simple_type_mapper(get_non_distributed_type_parameter(source).unwrap(), target))
}

pub(crate) fn new_simple_type_mapper(source: P<Type>, target: P<Type>) -> P<TypeMapper> {
    P::new(TypeMapper { data: TypeMapperData::Simple { source, target } })
}

pub(crate) fn new_array_type_mapper(sources: &'static [P<Type>], targets: &'static [P<Type>]) -> P<TypeMapper> {
    P::new(TypeMapper { data: TypeMapperData::Array { sources_targets: SlicePair::new(sources, targets) } })
}

pub(crate) fn new_array_to_single_type_mapper(sources: &'static [P<Type>], target: P<Type>) -> P<TypeMapper> {
    P::new(TypeMapper { data: TypeMapperData::ArrayToSingle { sources, target } })
}

pub(crate) fn new_deferred_type_mapper(sources: &'static [P<Type>], targets: Vec<Box<dyn Fn(&mut Checker) -> P<Type>>>) -> P<TypeMapper> {
    P::new(TypeMapper { data: TypeMapperData::Deferred { data: alloc(DeferredTypeMapper { sources, targets }) } })
}

pub(crate) fn new_function_type_mapper(f: fn(&mut Checker, P<Type>) -> P<Type>) -> P<TypeMapper> {
    P::new(TypeMapper { data: TypeMapperData::Function { f } })
}

pub(crate) fn new_merged_type_mapper(m1: P<TypeMapper>, m2: P<TypeMapper>) -> P<TypeMapper> {
    P::new(TypeMapper { data: TypeMapperData::Merged { m1, m2 } })
}

pub(crate) fn new_composite_type_mapper(m1: P<TypeMapper>, m2: P<TypeMapper>) -> P<TypeMapper> {
    P::new(TypeMapper { data: TypeMapperData::Composite { m1, m2 } })
}
