use crate::*;

#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum TypeMapperKind {
    #[default]
    Unknown,
    Simple,
    Array,
    Merged,
}

/// A mapper owns its concrete variant and array payloads. Graph edges still use legacy handles.
pub struct TypeMapper {
    data: OwnedMapper,
}

#[repr(C, u8)]
enum OwnedMapper {
    Simple {
        source: P<Type>,
        target: P<Type>,
    },
    Array(Box<ArrayMapper>),
    ArrayToSingle(Box<ArrayToSingleMapper>),
    Deferred(Box<DeferredTypeMapper>),
    Function(fn(&mut Checker, P<Type>) -> P<Type>),
    Merged {
        m1: P<TypeMapper>,
        m2: P<TypeMapper>,
    },
    Composite {
        m1: P<TypeMapper>,
        m2: P<TypeMapper>,
    },
    Inference {
        n: InferenceContextKey,
        fixing: bool,
    },
}

struct ArrayMapper {
    sources: Box<[P<Type>]>,
    targets: Box<[P<Type>]>,
}
struct ArrayToSingleMapper {
    sources: Box<[P<Type>]>,
    target: P<Type>,
}

#[derive(Clone, Copy)]
pub enum TypeMapperData<'a> {
    Simple {
        source: P<Type>,
        target: P<Type>,
    },
    Array {
        sources: &'a [P<Type>],
        targets: &'a [P<Type>],
    },
    ArrayToSingle {
        sources: &'a [P<Type>],
        target: P<Type>,
    },
    Deferred {
        data: &'a DeferredTypeMapper,
    },
    Function {
        f: fn(&mut Checker, P<Type>) -> P<Type>,
    },
    Merged {
        m1: P<TypeMapper>,
        m2: P<TypeMapper>,
    },
    Composite {
        m1: P<TypeMapper>,
        m2: P<TypeMapper>,
    },
    Inference {
        n: InferenceContextKey,
        fixing: bool,
    },
}

pub struct DeferredTypeMapper {
    pub sources: Box<[P<Type>]>,
    pub targets: Vec<Box<dyn Fn(&mut Checker) -> P<Type>>>,
}

impl<'a> TypeMapperData<'a> {
    pub fn array_sources_targets(&self) -> Option<(&'a [P<Type>], &'a [P<Type>])> {
        match *self {
            Self::Array { sources, targets } => Some((sources, targets)),
            _ => None,
        }
    }
}

impl TypeMapper {
    fn alloc(data: OwnedMapper) -> P<Self> {
        P::new(Self { data })
    }

    pub fn data(&self) -> TypeMapperData<'_> {
        match &self.data {
            OwnedMapper::Simple { source, target } => TypeMapperData::Simple {
                source: *source,
                target: *target,
            },
            OwnedMapper::Array(data) => TypeMapperData::Array {
                sources: &data.sources,
                targets: &data.targets,
            },
            OwnedMapper::ArrayToSingle(data) => TypeMapperData::ArrayToSingle {
                sources: &data.sources,
                target: data.target,
            },
            OwnedMapper::Deferred(data) => TypeMapperData::Deferred { data },
            OwnedMapper::Function(f) => TypeMapperData::Function { f: *f },
            OwnedMapper::Merged { m1, m2 } => TypeMapperData::Merged { m1: *m1, m2: *m2 },
            OwnedMapper::Composite { m1, m2 } => TypeMapperData::Composite { m1: *m1, m2: *m2 },
            OwnedMapper::Inference { n, fixing } => TypeMapperData::Inference {
                n: *n,
                fixing: *fixing,
            },
        }
    }

    pub fn map(&self, c: &mut Checker, t: P<Type>) -> P<Type> {
        match self.data() {
            TypeMapperData::Simple { source, target } => {
                if t == source {
                    target
                } else {
                    t
                }
            }
            TypeMapperData::Array { sources, targets } => {
                for (i, s) in sources.iter().enumerate() {
                    if t == *s {
                        return targets[i];
                    }
                }
                t
            }
            TypeMapperData::ArrayToSingle { sources, target } => {
                if sources.contains(&t) {
                    target
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
                    return c.instantiate_type(t1, Some(m2));
                }
                m2.map(c, t)
            }
            TypeMapperData::Inference { n, fixing } => {
                let inferences = c.inference_context(n).inferences.get();
                for (i, inference) in inferences.iter().enumerate() {
                    if Some(t) == c.inference_info(*inference).type_parameter.get() {
                        if fixing && !c.inference_info(*inference).is_fixed.get() {
                            // Before we commit to a particular inference (and thus lock out any further inferences),
                            // we infer from any intra-expression inference sites we have collected.
                            c.infer_from_intra_expression_sites(n);
                            clear_cached_inferences(c, &c.inference_context(n).inferences.get());
                            c.inference_info(*inference).is_fixed.set(true);
                        }
                        return c.get_inferred_type(n, i as i32);
                    }
                }
                t
            }
        }
    }

    pub fn kind(&self) -> TypeMapperKind {
        match self.data() {
            TypeMapperData::Simple { .. } => TypeMapperKind::Simple,
            TypeMapperData::Array { .. } => TypeMapperKind::Array,
            TypeMapperData::Merged { .. } => TypeMapperKind::Merged,
            _ => TypeMapperKind::Unknown,
        }
    }

    pub fn maps_this_only(&self) -> bool {
        match self.data() {
            TypeMapperData::Simple { source, .. } => is_this_type_parameter(source),
            TypeMapperData::Array { sources, .. }
            | TypeMapperData::ArrayToSingle { sources, .. } => {
                sources.len() == 1 && is_this_type_parameter(sources[0])
            }
            TypeMapperData::Deferred {
                data: DeferredTypeMapper { sources, .. },
            } => sources.len() == 1 && is_this_type_parameter(sources[0]),
            _ => false,
        }
    }
}

/// A mapper edge stored in a checker-owned record.
pub type MapperCell = Cell<Option<P<TypeMapper>>>;

// Factory functions

impl Checker {
    /// Go free function `getMappedType(t, mapper)`; a method because mapping may need the checker.
    pub(crate) fn get_mapped_type(&mut self, t: P<Type>, mapper: P<TypeMapper>) -> P<Type> {
        mapper.map(self, get_non_distributed_type_parameter(t).unwrap())
    }

    #[cfg_attr(feature = "site-counts", track_caller)]
    pub(crate) fn combine_type_mappers(
        &mut self,
        m1: Option<P<TypeMapper>>,
        m2: P<TypeMapper>,
    ) -> P<TypeMapper> {
        if let Some(m1) = m1 {
            return new_composite_type_mapper(m1, m2);
        }
        m2
    }

    pub(crate) fn map_type_with_composite_mapper(
        &mut self,
        t: P<Type>,
        m1: Option<P<TypeMapper>>,
        m2: P<TypeMapper>,
    ) -> P<Type> {
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
    pub(crate) fn new_backreference_mapper(
        &mut self,
        context: InferenceContextKey,
        index: i32,
    ) -> P<TypeMapper> {
        let forward_inferences = &self.inference_context(context).inferences.get()[index as usize..];
        let type_parameters: Vec<P<Type>> = forward_inferences
            .iter()
            .map(|i| self.inference_info(*i).type_parameter.get().unwrap())
            .collect();
        new_array_to_single_type_mapper(&type_parameters, self.unknown_type)
    }
}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn new_inference_type_mapper(n: InferenceContextKey, fixing: bool) -> P<TypeMapper> {
    tsrs_core::sitecount::hit("mapper", "inference");
    TypeMapper::alloc(OwnedMapper::Inference { n, fixing })
}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn new_type_mapper(sources: &[P<Type>], targets: &[P<Type>]) -> P<TypeMapper> {
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
pub(crate) fn prepend_type_mapping(
    source: P<Type>,
    target: P<Type>,
    mapper: Option<P<TypeMapper>>,
) -> P<TypeMapper> {
    let Some(mapper) = mapper else {
        return new_simple_type_mapper(get_non_distributed_type_parameter(source).unwrap(), target);
    };
    new_merged_type_mapper(
        new_simple_type_mapper(get_non_distributed_type_parameter(source).unwrap(), target),
        mapper,
    )
}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn append_type_mapping(
    mapper: Option<P<TypeMapper>>,
    source: P<Type>,
    target: P<Type>,
) -> P<TypeMapper> {
    let Some(mapper) = mapper else {
        return new_simple_type_mapper(get_non_distributed_type_parameter(source).unwrap(), target);
    };
    new_merged_type_mapper(
        mapper,
        new_simple_type_mapper(get_non_distributed_type_parameter(source).unwrap(), target),
    )
}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn new_simple_type_mapper(source: P<Type>, target: P<Type>) -> P<TypeMapper> {
    tsrs_core::sitecount::hit("mapper", "simple");
    TypeMapper::alloc(OwnedMapper::Simple { source, target })
}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn new_array_type_mapper(sources: &[P<Type>], targets: &[P<Type>]) -> P<TypeMapper> {
    tsrs_core::sitecount::hit("mapper", "array");
    TypeMapper::alloc(OwnedMapper::Array(owned_type_payload(ArrayMapper {
        sources: sources.to_vec().into_boxed_slice(),
        targets: targets.to_vec().into_boxed_slice(),
    })))
}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn new_array_to_single_type_mapper(
    sources: &[P<Type>],
    target: P<Type>,
) -> P<TypeMapper> {
    tsrs_core::sitecount::hit("mapper", "array_to_single");
    TypeMapper::alloc(OwnedMapper::ArrayToSingle(owned_type_payload(
        ArrayToSingleMapper {
            sources: sources.to_vec().into_boxed_slice(),
            target,
        },
    )))
}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn new_deferred_type_mapper(
    sources: &[P<Type>],
    targets: Vec<Box<dyn Fn(&mut Checker) -> P<Type>>>,
) -> P<TypeMapper> {
    tsrs_core::sitecount::hit("mapper", "deferred");
    TypeMapper::alloc(OwnedMapper::Deferred(owned_type_payload(
        DeferredTypeMapper {
            sources: sources.to_vec().into_boxed_slice(),
            targets,
        },
    )))
}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn new_function_type_mapper(f: fn(&mut Checker, P<Type>) -> P<Type>) -> P<TypeMapper> {
    tsrs_core::sitecount::hit("mapper", "function");
    TypeMapper::alloc(OwnedMapper::Function(f))
}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn new_merged_type_mapper(m1: P<TypeMapper>, m2: P<TypeMapper>) -> P<TypeMapper> {
    tsrs_core::sitecount::hit("mapper", "merged");
    TypeMapper::alloc(OwnedMapper::Merged { m1, m2 })
}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn new_composite_type_mapper(m1: P<TypeMapper>, m2: P<TypeMapper>) -> P<TypeMapper> {
    tsrs_core::sitecount::hit("mapper", "composite");
    TypeMapper::alloc(OwnedMapper::Composite { m1, m2 })
}

/// Mapper keys and function addresses are scalars; only data-pointer variants contribute graph edges.
pub(crate) fn census_layouts() {
    use std::mem::{align_of, offset_of, size_of};
    use tsrs_core::CensusField;
    let tag = offset_of!(TypeMapper, data);
    let first = tag + align_of::<OwnedMapper>();
    let second = first + size_of::<usize>();
    let mut fields = CensusField::all_but(0, size_of::<TypeMapper>(), &[first, second]);
    fields.push(CensusField::Variant {
        ptr: first,
        tag,
        variants: (1 << 0) | (1 << 1) | (1 << 2) | (1 << 3) | (1 << 5) | (1 << 6),
    });
    fields.push(CensusField::Variant {
        ptr: second,
        tag,
        variants: (1 << 0) | (1 << 5) | (1 << 6),
    });
    tsrs_core::census_layout(std::any::type_name::<TypeMapper>(), &fields);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deferred_mapper_owns_inputs_and_releases_callbacks_with_region() {
        let region = tsrs_core::arena::Region::new(4096);
        let capture = std::rc::Rc::new(());
        let weak = std::rc::Rc::downgrade(&capture);
        {
            let _scope = region.enter();
            let source = Type::alloc(
                TypeFlags::Any,
                ObjectFlags::None,
                TypeId(1),
                IntrinsicType::default(),
            );
            let sources = vec![source];
            let mapper = new_deferred_type_mapper(
                &sources,
                vec![Box::new(move |_| {
                    let _ = &capture;
                    source
                })],
            );
            drop(sources);
            let TypeMapperData::Deferred { data } = mapper.data() else {
                panic!("expected deferred mapper");
            };
            assert_eq!(&*data.sources, &[source]);
        }
        assert!(weak.upgrade().is_some());
        drop(region);
        assert!(weak.upgrade().is_none());
    }
}
