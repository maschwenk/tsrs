use crate::*;

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
///
/// Representation (16 bytes; there are tens of millions of mappers): two words. The low three bits of `first`
/// hold the kind (`TAG_*`); its other bits hold the first payload pointer, which is 8-aligned. The array kinds keep
/// their list lengths in the top 16 bits of the two slice pointers (when the pointers fit in 48 bits and the lists
/// in `u16`; otherwise the mapper is out of line). The rare kinds (functions, deferred mappers, long lists) point
/// to a `RareTypeMapper`. `data()` decodes a mapper into the `TypeMapperData` view that the code matches on.
pub struct TypeMapper {
    first: *const (),
    second: *const (),
}

const _: () = assert!(std::mem::size_of::<TypeMapper>() == 16);
// The payload pointers leave the tag bits free.
const _: () = assert!(std::mem::align_of::<Type>() >= 8 && std::mem::align_of::<TypeMapper>() >= 8);
const _: () = assert!(std::mem::align_of::<InferenceContext>() >= 8 && std::mem::align_of::<RareTypeMapper>() >= 8);

const TAG_MASK: usize = 0b111;
const TAG_SIMPLE: usize = 0;
const TAG_MERGED: usize = 1;
const TAG_COMPOSITE: usize = 2;
const TAG_INFERENCE: usize = 3;
const TAG_ARRAY: usize = 4;
const TAG_ARRAY_TO_SINGLE: usize = 5;
const TAG_RARE: usize = 6;
const LEN_SHIFT: u32 = 48;
const ADDR_MASK: usize = ((1 << LEN_SHIFT) - 1) & !TAG_MASK;

/// A decoded mapper (the Go mapper data structs).
#[derive(Clone, Copy)]
pub enum TypeMapperData {
    Simple { source: P<Type>, target: P<Type> },
    Array { sources: &'static [P<Type>], targets: &'static [P<Type>] },
    ArrayToSingle { sources: &'static [P<Type>], target: P<Type> },
    Deferred { data: &'static DeferredTypeMapper },
    Function { f: fn(&mut Checker, P<Type>) -> P<Type> },
    Merged { m1: P<TypeMapper>, m2: P<TypeMapper> },
    Composite { m1: P<TypeMapper>, m2: P<TypeMapper> },
    Inference { n: P<InferenceContext>, fixing: bool },
}

/// The mapper kinds that are rare enough to live out of line.
enum RareTypeMapper {
    Array { sources: &'static [P<Type>], targets: &'static [P<Type>] },
    ArrayToSingle { sources: &'static [P<Type>], target: P<Type> },
    Deferred(DeferredTypeMapper),
    Function(fn(&mut Checker, P<Type>) -> P<Type>),
}

pub struct DeferredTypeMapper {
    pub sources: &'static [P<Type>],
    pub targets: Vec<Box<dyn Fn(&mut Checker) -> P<Type>>>,
}

impl TypeMapperData {
    /// The sources and targets of an array mapper (Go `ArrayMapper`).
    #[inline]
    pub fn array_sources_targets(&self) -> Option<(&'static [P<Type>], &'static [P<Type>])> {
        match *self {
            TypeMapperData::Array { sources, targets } => Some((sources, targets)),
            _ => None,
        }
    }
}

/// A slice pointer with its length in the top 16 bits, or `None` when either does not fit.
#[inline]
fn pack_slice(s: &'static [P<Type>]) -> Option<*const ()> {
    let p = s.as_ptr() as *const ();
    (p.addr() & !ADDR_MASK == 0 && s.len() <= u16::MAX as usize).then(|| p.map_addr(|a| a | s.len() << LEN_SHIFT))
}

/// # Safety
/// `w` must come from `pack_slice` (tag bits may have been added).
#[inline]
unsafe fn unpack_slice(w: *const ()) -> &'static [P<Type>] {
    std::slice::from_raw_parts(w.map_addr(|a| a & ADDR_MASK) as *const P<Type>, w.addr() >> LEN_SHIFT)
}

#[inline]
fn tagged<T>(p: &'static T, tag: usize) -> *const () {
    (p as *const T as *const ()).map_addr(|a| a | tag)
}

/// # Safety
/// `w` must hold a `&'static T` (plus tag bits).
#[inline]
unsafe fn untagged<T>(w: *const ()) -> &'static T {
    &*(w.map_addr(|a| a & !TAG_MASK) as *const T)
}

impl TypeMapper {
    fn alloc_rare(rare: RareTypeMapper) -> P<TypeMapper> {
        P::new(TypeMapper { first: tagged(alloc(rare), TAG_RARE), second: std::ptr::null() })
    }

    /// The mapper's kind and payload.
    #[inline]
    pub fn data(&self) -> TypeMapperData {
        // SAFETY: the words were encoded for this tag by the constructors below.
        unsafe {
            match self.first.addr() & TAG_MASK {
                TAG_SIMPLE => TypeMapperData::Simple { source: P::from_static(untagged(self.first)), target: P::from_static(untagged(self.second)) },
                TAG_MERGED => TypeMapperData::Merged { m1: P::from_static(untagged(self.first)), m2: P::from_static(untagged(self.second)) },
                TAG_COMPOSITE => TypeMapperData::Composite { m1: P::from_static(untagged(self.first)), m2: P::from_static(untagged(self.second)) },
                TAG_INFERENCE => TypeMapperData::Inference { n: P::from_static(untagged(self.first)), fixing: self.second.addr() != 0 },
                TAG_ARRAY => TypeMapperData::Array { sources: unpack_slice(self.first), targets: unpack_slice(self.second) },
                TAG_ARRAY_TO_SINGLE => TypeMapperData::ArrayToSingle { sources: unpack_slice(self.first), target: P::from_static(untagged(self.second)) },
                _ => match untagged::<RareTypeMapper>(self.first) {
                    RareTypeMapper::Array { sources, targets } => TypeMapperData::Array { sources, targets },
                    RareTypeMapper::ArrayToSingle { sources, target } => TypeMapperData::ArrayToSingle { sources, target: *target },
                    RareTypeMapper::Deferred(data) => TypeMapperData::Deferred { data },
                    RareTypeMapper::Function(f) => TypeMapperData::Function { f: *f },
                },
            }
        }
    }
}

impl TypeMapper {
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
                let inferences = n.inferences.get();
                for (i, inference) in inferences.iter().enumerate() {
                    if Some(t) == inference.type_parameter.get() {
                        if fixing && !inference.is_fixed.get() {
                            // Before we commit to a particular inference (and thus lock out any further inferences),
                            // we infer from any intra-expression inference sites we have collected.
                            c.infer_from_intra_expression_sites(n);
                            clear_cached_inferences(n.inferences.get());
                            inference.is_fixed.set(true);
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
            TypeMapperData::Array { sources, .. } | TypeMapperData::ArrayToSingle { sources, .. } => sources.len() == 1 && is_this_type_parameter(sources[0]),
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

}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn new_inference_type_mapper(n: P<InferenceContext>, fixing: bool) -> P<TypeMapper> {
    tsrs_core::sitecount::hit("mapper", "inference");
    P::new(TypeMapper { first: tagged(n.get(), TAG_INFERENCE), second: std::ptr::without_provenance(fixing as usize) })
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
    P::new(TypeMapper { first: tagged(source.get(), TAG_SIMPLE), second: tagged(target.get(), 0) })
}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn new_array_type_mapper(sources: &'static [P<Type>], targets: &'static [P<Type>]) -> P<TypeMapper> {
    tsrs_core::sitecount::hit("mapper", "array");
    match (pack_slice(sources), pack_slice(targets)) {
        (Some(first), Some(second)) => P::new(TypeMapper { first: first.map_addr(|a| a | TAG_ARRAY), second }),
        _ => TypeMapper::alloc_rare(RareTypeMapper::Array { sources, targets }),
    }
}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn new_array_to_single_type_mapper(sources: &'static [P<Type>], target: P<Type>) -> P<TypeMapper> {
    tsrs_core::sitecount::hit("mapper", "array_to_single");
    match pack_slice(sources) {
        Some(first) => P::new(TypeMapper { first: first.map_addr(|a| a | TAG_ARRAY_TO_SINGLE), second: tagged(target.get(), 0) }),
        None => TypeMapper::alloc_rare(RareTypeMapper::ArrayToSingle { sources, target }),
    }
}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn new_deferred_type_mapper(sources: &'static [P<Type>], targets: Vec<Box<dyn Fn(&mut Checker) -> P<Type>>>) -> P<TypeMapper> {
    tsrs_core::sitecount::hit("mapper", "deferred");
    TypeMapper::alloc_rare(RareTypeMapper::Deferred(DeferredTypeMapper { sources, targets }))
}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn new_function_type_mapper(f: fn(&mut Checker, P<Type>) -> P<Type>) -> P<TypeMapper> {
    tsrs_core::sitecount::hit("mapper", "function");
    TypeMapper::alloc_rare(RareTypeMapper::Function(f))
}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn new_merged_type_mapper(m1: P<TypeMapper>, m2: P<TypeMapper>) -> P<TypeMapper> {
    tsrs_core::sitecount::hit("mapper", "merged");
    P::new(TypeMapper { first: tagged(m1.get(), TAG_MERGED), second: tagged(m2.get(), 0) })
}

#[cfg_attr(feature = "site-counts", track_caller)]
pub(crate) fn new_composite_type_mapper(m1: P<TypeMapper>, m2: P<TypeMapper>) -> P<TypeMapper> {
    tsrs_core::sitecount::hit("mapper", "composite");
    P::new(TypeMapper { first: tagged(m1.get(), TAG_COMPOSITE), second: tagged(m2.get(), 0) })
}
