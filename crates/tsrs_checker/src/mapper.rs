use crate::*;
use tsrs_core::{SlicePair, StaticSlicePtr};

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

/// The array mappers keep their slice lengths next to the tag (`array_sources_targets()` /
/// `array_to_single_sources()` read them back), so the enum is 24 bytes; lists longer than `u16::MAX` use
/// `ArrayLong`.
pub enum TypeMapperData {
    Simple { source: P<Type>, target: P<Type> },
    Array { sources: StaticSlicePtr<P<Type>>, targets: StaticSlicePtr<P<Type>>, sources_len: u16, targets_len: u16 },
    ArrayLong { sources_targets: &'static SlicePair<P<Type>, P<Type>> },
    ArrayToSingle { sources: StaticSlicePtr<P<Type>>, sources_len: u32, target: P<Type> },
    Deferred { data: &'static DeferredTypeMapper },
    Function { f: fn(&mut Checker, P<Type>) -> P<Type> },
    Merged { m1: P<TypeMapper>, m2: P<TypeMapper> },
    Composite { m1: P<TypeMapper>, m2: P<TypeMapper> },
    Inference { n: P<InferenceContext>, fixing: bool },
}

// Rare; kept out of line so `TypeMapperData` stays 24 bytes (checked below).
pub struct DeferredTypeMapper {
    pub sources: &'static [P<Type>],
    pub targets: Vec<Box<dyn Fn(&mut Checker) -> P<Type>>>,
}

const _: () = assert!(std::mem::size_of::<TypeMapper>() == 24);

impl TypeMapperData {
    /// The sources and targets of an array mapper (Go `ArrayMapper`).
    #[inline]
    pub fn array_sources_targets(&self) -> Option<(&'static [P<Type>], &'static [P<Type>])> {
        match *self {
            // SAFETY: the lengths were stored with the pointers by `new_array_type_mapper`.
            TypeMapperData::Array { sources, targets, sources_len, targets_len } => {
                Some(unsafe { (sources.slice(sources_len as usize), targets.slice(targets_len as usize)) })
            }
            TypeMapperData::ArrayLong { sources_targets } => Some((sources_targets.first(), sources_targets.second())),
            _ => None,
        }
    }

    /// The sources of an array-to-single mapper (Go `ArrayToSingleTypeMapper`).
    #[inline]
    fn array_to_single_sources(&self) -> Option<&'static [P<Type>]> {
        match *self {
            // SAFETY: the length was stored with the pointer by `new_array_to_single_type_mapper`.
            TypeMapperData::ArrayToSingle { sources, sources_len, .. } => Some(unsafe { sources.slice(sources_len as usize) }),
            _ => None,
        }
    }
}

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
            TypeMapperData::Array { .. } | TypeMapperData::ArrayLong { .. } => {
                let (sources, targets) = self.data.array_sources_targets().unwrap();
                for (i, s) in sources.iter().enumerate() {
                    if t == *s {
                        return targets[i];
                    }
                }
                t
            }
            TypeMapperData::ArrayToSingle { target, .. } => {
                if self.data.array_to_single_sources().unwrap().contains(&t) {
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
            TypeMapperData::Array { .. } | TypeMapperData::ArrayLong { .. } => TypeMapperKind::Array,
            TypeMapperData::Merged { .. } => TypeMapperKind::Merged,
            _ => TypeMapperKind::Unknown,
        }
    }

    pub fn maps_this_only(&self) -> bool {
        match &self.data {
            TypeMapperData::Simple { source, .. } => is_this_type_parameter(*source),
            TypeMapperData::Array { .. } | TypeMapperData::ArrayLong { .. } => {
                let sources = self.data.array_sources_targets().unwrap().0;
                sources.len() == 1 && is_this_type_parameter(sources[0])
            }
            TypeMapperData::ArrayToSingle { .. } => {
                let sources = self.data.array_to_single_sources().unwrap();
                sources.len() == 1 && is_this_type_parameter(sources[0])
            }
            TypeMapperData::Deferred { data: DeferredTypeMapper { sources, .. } } => sources.len() == 1 && is_this_type_parameter(sources[0]),
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

    #[cfg_attr(feature = "site-counts", track_caller)]
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
    #[cfg_attr(feature = "site-counts", track_caller)]
    pub(crate) fn new_backreference_mapper(&mut self, context: P<InferenceContext>, index: i32) -> P<TypeMapper> {
        let forward_inferences = &context.inferences.get()[index as usize..];
        let type_parameters: Vec<P<Type>> = forward_inferences.iter().map(|i| i.type_parameter.get().unwrap()).collect();
        new_array_to_single_type_mapper(alloc_slice(&type_parameters), self.unknown_type)
    }

    #[cfg_attr(feature = "site-counts", track_caller)]
    pub(crate) fn new_inference_type_mapper(&mut self, n: P<InferenceContext>, fixing: bool) -> P<TypeMapper> {
        tsrs_core::sitecount::hit("mapper", "inference");
        P::new(TypeMapper { data: TypeMapperData::Inference { n, fixing } })
    }
}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn new_type_mapper(sources: &'static [P<Type>], targets: &'static [P<Type>]) -> P<TypeMapper> {
    if sources.len() == 1 {
        return new_simple_type_mapper(sources[0], targets[0]);
    }
    new_array_type_mapper(sources, targets)
}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn merge_type_mappers(m1: Option<P<TypeMapper>>, m2: P<TypeMapper>) -> P<TypeMapper> {
    if let Some(m1) = m1 {
        return new_merged_type_mapper(m1, m2);
    }
    m2
}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn prepend_type_mapping(source: P<Type>, target: P<Type>, mapper: Option<P<TypeMapper>>) -> P<TypeMapper> {
    let Some(mapper) = mapper else {
        return new_simple_type_mapper(get_non_distributed_type_parameter(source).unwrap(), target);
    };
    new_merged_type_mapper(new_simple_type_mapper(get_non_distributed_type_parameter(source).unwrap(), target), mapper)
}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn append_type_mapping(mapper: Option<P<TypeMapper>>, source: P<Type>, target: P<Type>) -> P<TypeMapper> {
    let Some(mapper) = mapper else {
        return new_simple_type_mapper(get_non_distributed_type_parameter(source).unwrap(), target);
    };
    new_merged_type_mapper(mapper, new_simple_type_mapper(get_non_distributed_type_parameter(source).unwrap(), target))
}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn new_simple_type_mapper(source: P<Type>, target: P<Type>) -> P<TypeMapper> {
    tsrs_core::sitecount::hit("mapper", "simple");
    P::new(TypeMapper { data: TypeMapperData::Simple { source, target } })
}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn new_array_type_mapper(sources: &'static [P<Type>], targets: &'static [P<Type>]) -> P<TypeMapper> {
    tsrs_core::sitecount::hit("mapper", "array");
    let data = match (u16::try_from(sources.len()), u16::try_from(targets.len())) {
        (Ok(sources_len), Ok(targets_len)) => TypeMapperData::Array {
            sources: StaticSlicePtr::new(sources),
            targets: StaticSlicePtr::new(targets),
            sources_len,
            targets_len,
        },
        _ => TypeMapperData::ArrayLong { sources_targets: alloc(SlicePair::new(sources, targets)) },
    };
    P::new(TypeMapper { data })
}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn new_array_to_single_type_mapper(sources: &'static [P<Type>], target: P<Type>) -> P<TypeMapper> {
    tsrs_core::sitecount::hit("mapper", "array_to_single");
    let sources_len = u32::try_from(sources.len()).expect("type mapper source list longer than u32::MAX");
    P::new(TypeMapper { data: TypeMapperData::ArrayToSingle { sources: StaticSlicePtr::new(sources), sources_len, target } })
}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn new_deferred_type_mapper(sources: &'static [P<Type>], targets: Vec<Box<dyn Fn(&mut Checker) -> P<Type>>>) -> P<TypeMapper> {
    tsrs_core::sitecount::hit("mapper", "deferred");
    P::new(TypeMapper { data: TypeMapperData::Deferred { data: alloc(DeferredTypeMapper { sources, targets }) } })
}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn new_function_type_mapper(f: fn(&mut Checker, P<Type>) -> P<Type>) -> P<TypeMapper> {
    tsrs_core::sitecount::hit("mapper", "function");
    P::new(TypeMapper { data: TypeMapperData::Function { f } })
}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn new_merged_type_mapper(m1: P<TypeMapper>, m2: P<TypeMapper>) -> P<TypeMapper> {
    tsrs_core::sitecount::hit("mapper", "merged");
    P::new(TypeMapper { data: TypeMapperData::Merged { m1, m2 } })
}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn new_composite_type_mapper(m1: P<TypeMapper>, m2: P<TypeMapper>) -> P<TypeMapper> {
    tsrs_core::sitecount::hit("mapper", "composite");
    P::new(TypeMapper { data: TypeMapperData::Composite { m1, m2 } })
}
