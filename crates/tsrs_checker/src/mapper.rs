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

/// Qualified mapper edge; resolving it borrows the checker that owns its record.
pub type TypeMapperKey = tsrs_core::arena_owner::ArenaKey<TypeMapper>;

/// A mapper owns its concrete variant and array payloads. Type edges still use legacy handles.
pub struct TypeMapper {
    data: OwnedMapper,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(std::mem::size_of::<TypeMapper>() == 24);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(std::mem::size_of::<TypeMapper>() == 20);

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
        m1: TypeMapperKey,
        m2: TypeMapperKey,
    },
    Composite {
        m1: TypeMapperKey,
        m2: TypeMapperKey,
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
        m1: TypeMapperKey,
        m2: TypeMapperKey,
    },
    Composite {
        m1: TypeMapperKey,
        m2: TypeMapperKey,
    },
    Inference {
        n: InferenceContextKey,
        fixing: bool,
    },
}

pub struct DeferredTypeMapper {
    pub sources: Box<[P<Type>]>,
    pub targets: Vec<std::sync::Arc<dyn Fn(&mut Checker) -> P<Type>>>,
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
    fn alloc(store: &mut tsrs_core::arena_owner::ArenaBuilder<Self>, data: OwnedMapper) -> TypeMapperKey {
        store.alloc(Self { data })
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
pub type MapperCell = Cell<Option<TypeMapperKey>>;

// Factory functions

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn allocate_simple_mapper(store: &mut tsrs_core::arena_owner::ArenaBuilder<TypeMapper>, source: P<Type>, target: P<Type>) -> TypeMapperKey {
    tsrs_core::sitecount::hit("mapper", "simple");
    TypeMapper::alloc(store, OwnedMapper::Simple { source, target })
}

impl Checker {
    pub(crate) fn apply_type_mapper(&mut self, mapper: TypeMapperKey, t: P<Type>) -> P<Type> {
        match self.type_mapper(mapper).data() {
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
                if let Some(i) = data.sources.iter().position(|s| t == *s) {
                    // The callback may grow mapper storage. Retain its owner before borrowing the checker mutably.
                    let target = std::sync::Arc::clone(&data.targets[i]);
                    return target(self);
                }
                t
            }
            TypeMapperData::Function { f } => f(self, t),
            TypeMapperData::Merged { m1, m2 } => {
                let t1 = self.apply_type_mapper(m1, t);
                self.apply_type_mapper(m2, t1)
            }
            TypeMapperData::Composite { m1, m2 } => {
                let t1 = self.apply_type_mapper(m1, t);
                if t1 != t {
                    return self.instantiate_type(t1, Some(m2));
                }
                self.apply_type_mapper(m2, t)
            }
            TypeMapperData::Inference { n, fixing } => {
                let inferences = self.inference_context(n).inferences.get();
                for (i, inference) in inferences.iter().enumerate() {
                    if Some(t) == self.inference_info(*inference).type_parameter.get() {
                        if fixing && !self.inference_info(*inference).is_fixed.get() {
                            // Before we commit to a particular inference (and thus lock out any further inferences),
                            // we infer from any intra-expression inference sites we have collected.
                            self.infer_from_intra_expression_sites(n);
                            clear_cached_inferences(self, &self.inference_context(n).inferences.get());
                            self.inference_info(*inference).is_fixed.set(true);
                        }
                        return self.get_inferred_type(n, i as i32);
                    }
                }
                t
            }
        }
    }

    /// Go free function `getMappedType(t, mapper)`; a method because mapping may need the checker.
    pub(crate) fn get_mapped_type(&mut self, t: P<Type>, mapper: TypeMapperKey) -> P<Type> {
        self.apply_type_mapper(mapper, get_non_distributed_type_parameter(t).unwrap())
    }

    #[cfg_attr(feature = "site-counts", track_caller)]
    pub(crate) fn combine_type_mappers(
        &mut self,
        m1: Option<TypeMapperKey>,
        m2: TypeMapperKey,
    ) -> TypeMapperKey {
        if let Some(m1) = m1 {
            return self.new_composite_type_mapper(m1, m2);
        }
        m2
    }

    pub(crate) fn map_type_with_composite_mapper(
        &mut self,
        t: P<Type>,
        m1: Option<TypeMapperKey>,
        m2: TypeMapperKey,
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
    ) -> TypeMapperKey {
        let forward_inferences = &self.inference_context(context).inferences.get()[index as usize..];
        let type_parameters: Vec<P<Type>> = forward_inferences
            .iter()
            .map(|i| self.inference_info(*i).type_parameter.get().unwrap())
            .collect();
        self.new_array_to_single_type_mapper(&type_parameters, self.unknown_type)
    }

    #[cfg_attr(feature = "site-counts", track_caller)]
    pub(crate) fn new_inference_type_mapper(&mut self, n: InferenceContextKey, fixing: bool) -> TypeMapperKey {
        tsrs_core::sitecount::hit("mapper", "inference");
        TypeMapper::alloc(&mut self.type_mappers, OwnedMapper::Inference { n, fixing })
    }

    #[cfg_attr(feature = "site-counts", track_caller)]
    pub(crate) fn new_type_mapper(&mut self, sources: &[P<Type>], targets: &[P<Type>]) -> TypeMapperKey {
        if sources.len() == 1 {
            return self.new_simple_type_mapper(sources[0], targets[0]);
        }
        self.new_array_type_mapper(sources, targets)
    }

    #[cfg_attr(feature = "site-counts", track_caller)]
    pub(crate) fn merge_type_mappers(&mut self, m1: Option<TypeMapperKey>, m2: TypeMapperKey) -> TypeMapperKey {
        if let Some(m1) = m1 {
            return self.new_merged_type_mapper(m1, m2);
        }
        m2
    }

    #[cfg_attr(feature = "site-counts", track_caller)]
    pub(crate) fn prepend_type_mapping(
        &mut self,
        source: P<Type>,
        target: P<Type>,
        mapper: Option<TypeMapperKey>,
    ) -> TypeMapperKey {
        let Some(mapper) = mapper else {
            return self.new_simple_type_mapper(get_non_distributed_type_parameter(source).unwrap(), target);
        };
        let mapping = self.new_simple_type_mapper(get_non_distributed_type_parameter(source).unwrap(), target);
        self.new_merged_type_mapper(mapping, mapper)
    }

    #[cfg_attr(feature = "site-counts", track_caller)]
    pub(crate) fn append_type_mapping(
        &mut self,
        mapper: Option<TypeMapperKey>,
        source: P<Type>,
        target: P<Type>,
    ) -> TypeMapperKey {
        let Some(mapper) = mapper else {
            return self.new_simple_type_mapper(get_non_distributed_type_parameter(source).unwrap(), target);
        };
        let mapping = self.new_simple_type_mapper(get_non_distributed_type_parameter(source).unwrap(), target);
        self.new_merged_type_mapper(mapper, mapping)
    }

    #[cfg_attr(feature = "site-counts", track_caller)]
    pub(crate) fn new_simple_type_mapper(&mut self, source: P<Type>, target: P<Type>) -> TypeMapperKey {
        allocate_simple_mapper(&mut self.type_mappers, source, target)
    }

    #[cfg_attr(feature = "site-counts", track_caller)]
    pub(crate) fn new_array_type_mapper(&mut self, sources: &[P<Type>], targets: &[P<Type>]) -> TypeMapperKey {
        tsrs_core::sitecount::hit("mapper", "array");
        TypeMapper::alloc(&mut self.type_mappers, OwnedMapper::Array(owned_type_payload(ArrayMapper {
            sources: sources.to_vec().into_boxed_slice(),
            targets: targets.to_vec().into_boxed_slice(),
        })))
    }

    #[cfg_attr(feature = "site-counts", track_caller)]
    pub(crate) fn new_array_to_single_type_mapper(
        &mut self,
        sources: &[P<Type>],
        target: P<Type>,
    ) -> TypeMapperKey {
        tsrs_core::sitecount::hit("mapper", "array_to_single");
        TypeMapper::alloc(&mut self.type_mappers, OwnedMapper::ArrayToSingle(owned_type_payload(
            ArrayToSingleMapper {
                sources: sources.to_vec().into_boxed_slice(),
                target,
            },
        )))
    }

    #[cfg_attr(feature = "site-counts", track_caller)]
    pub(crate) fn new_deferred_type_mapper(
        &mut self,
        sources: &[P<Type>],
        targets: Vec<Box<dyn Fn(&mut Checker) -> P<Type>>>,
    ) -> TypeMapperKey {
        tsrs_core::sitecount::hit("mapper", "deferred");
        TypeMapper::alloc(&mut self.type_mappers, OwnedMapper::Deferred(owned_type_payload(
            DeferredTypeMapper {
                sources: sources.to_vec().into_boxed_slice(),
                targets: targets.into_iter().map(std::sync::Arc::from).collect(),
            },
        )))
    }

    #[cfg_attr(feature = "site-counts", track_caller)]
    pub(crate) fn new_function_type_mapper(&mut self, f: fn(&mut Checker, P<Type>) -> P<Type>) -> TypeMapperKey {
        tsrs_core::sitecount::hit("mapper", "function");
        TypeMapper::alloc(&mut self.type_mappers, OwnedMapper::Function(f))
    }

    #[cfg_attr(feature = "site-counts", track_caller)]
    pub(crate) fn new_merged_type_mapper(&mut self, m1: TypeMapperKey, m2: TypeMapperKey) -> TypeMapperKey {
        tsrs_core::sitecount::hit("mapper", "merged");
        TypeMapper::alloc(&mut self.type_mappers, OwnedMapper::Merged { m1, m2 })
    }

    #[cfg_attr(feature = "site-counts", track_caller)]
    pub(crate) fn new_composite_type_mapper(&mut self, m1: TypeMapperKey, m2: TypeMapperKey) -> TypeMapperKey {
        tsrs_core::sitecount::hit("mapper", "composite");
        TypeMapper::alloc(&mut self.type_mappers, OwnedMapper::Composite { m1, m2 })
    }

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
        variants: (1 << 0) | (1 << 1) | (1 << 2) | (1 << 3),
    });
    fields.push(CensusField::Variant {
        ptr: second,
        tag,
        variants: 1 << 0,
    });
    tsrs_core::census_layout(std::any::type_name::<TypeMapper>(), &fields);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deferred_mapper_keys_own_inputs_and_release_callbacks_with_the_store() {
        let capture = std::rc::Rc::new(());
        let weak = std::rc::Rc::downgrade(&capture);
        let source = Type::alloc(TypeFlags::Any, ObjectFlags::None, TypeId(1), IntrinsicType::default());
        let sources = vec![source];
        let mut mappers = tsrs_core::arena_owner::ArenaBuilder::with_capacity(1);
        let mapper = TypeMapper::alloc(&mut mappers, OwnedMapper::Deferred(owned_type_payload(DeferredTypeMapper {
            sources: sources.to_vec().into_boxed_slice(),
            targets: vec![std::sync::Arc::new(move |_| {
                let _ = &capture;
                source
            })],
        })));
        drop(sources);
        for _ in 0..128 {
            allocate_simple_mapper(&mut mappers, source, source);
        }
        let TypeMapperData::Deferred { data } = mappers.get(mapper).unwrap().data() else {
            panic!("expected deferred mapper");
        };
        assert_eq!(&*data.sources, &[source]);
        let callback = std::sync::Arc::clone(&data.targets[0]);
        let mut foreign = tsrs_core::arena_owner::ArenaBuilder::new();
        let foreign_mapper = allocate_simple_mapper(&mut foreign, source, source);
        assert!(mappers.get(foreign_mapper).is_none());
        assert!(foreign.get(mapper).is_none());
        drop(mappers);
        assert!(weak.upgrade().is_some());
        drop(callback);
        assert!(weak.upgrade().is_none());
    }
}
