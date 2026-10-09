//! spike/r1-read-path (research, never merged): a checker map that a fork of the shared type layer would read over the
//! frozen seed's map (notes/design-shared-type-layer.md sections 3.2 and 5.2). The frozen map is probed first, and
//! only for a key whose handles all lie in the window (`KeyAnd`, the AND the key builder keeps); every other lookup and
//! every insert use the checker's own map. A key with no handle (a literal's value) always probes the frozen map, so
//! for it the hook is a null test. No frozen map exists on this branch (`base` is always `None`).

use tsrs_core::shwindow::KeyAnd;

pub struct TwoLevel<M: 'static> {
    own: M,
    base: Option<&'static M>,
}

impl<M: Default> Default for TwoLevel<M> {
    fn default() -> Self {
        TwoLevel { own: M::default(), base: None }
    }
}

impl<M> std::ops::Deref for TwoLevel<M> {
    type Target = M;
    #[inline]
    fn deref(&self) -> &M {
        &self.own
    }
}

impl<M> std::ops::DerefMut for TwoLevel<M> {
    #[inline]
    fn deref_mut(&mut self) -> &mut M {
        &mut self.own
    }
}

impl<M> TwoLevel<M> {
    /// `get_base` for a map keyed by one object (the lazy member and mapped tables): probed only when it is shared.
    #[inline]
    pub fn get_base_of<T: ?Sized, R>(&self, key: tsrs_core::P<T>, probe: impl FnOnce(&'static M) -> Option<R>) -> Option<R> {
        if !key.is_shared() {
            return None;
        }
        match self.base {
            None => None,
            Some(base) => probe_base(base, probe),
        }
    }

    /// The frozen map's answer for a key whose handles AND to `and` (probed only when they all lie in the window).
    #[inline]
    pub fn get_base<R>(&self, and: KeyAnd, probe: impl FnOnce(&'static M) -> Option<R>) -> Option<R> {
        if !and.all_shared() {
            return None;
        }
        match self.base {
            None => None,
            Some(base) => probe_base(base, probe),
        }
    }
}

#[cold]
#[inline(never)]
fn probe_base<M, R>(base: &'static M, probe: impl FnOnce(&'static M) -> Option<R>) -> Option<R> {
    probe(base)
}

// Mechanism 4 of R1: the instantiation tables under shared targets (design section 3.4). A lookup under a shared
// target asks the frozen table only when every handle of the key is shared, then the fork's side table
// (`Checker::shared_instantiations`); an insert under a shared target goes to the side table. The hot paths test the
// target (or, for a type alias, its table) once per lookup and insert; these cold paths never run here.
impl crate::Checker {
    #[cold]
    #[inline(never)]
    pub(crate) fn shared_instantiation_get(
        &self,
        target: tsrs_core::PKey,
        key: crate::CacheHashKey,
        and: KeyAnd,
        frozen: impl FnOnce() -> Option<tsrs_core::P<crate::Type>>,
    ) -> Option<tsrs_core::P<crate::Type>> {
        if and.all_shared() {
            if let Some(t) = frozen() {
                return Some(t);
            }
        }
        self.shared_instantiations.get(&(target, key)).copied()
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn shared_instantiation_set(&mut self, target: tsrs_core::PKey, key: crate::CacheHashKey, t: tsrs_core::P<crate::Type>) {
        self.shared_instantiations.insert((target, key), t);
    }

    /// `create_type_reference`'s lookup under a shared interface target.
    #[cold]
    #[inline(never)]
    pub(crate) fn shared_reference_get(&self, target: tsrs_core::P<crate::Type>, type_arguments: &[tsrs_core::P<crate::Type>]) -> Option<tsrs_core::P<crate::Type>> {
        let mut and = KeyAnd::NONE;
        for &a in type_arguments {
            and.add(a);
        }
        let key = crate::get_type_list_key(type_arguments);
        self.shared_instantiation_get(target.key(), key, and, || target.as_interface_type().instantiations.get(type_arguments))
    }

    /// `create_type_reference`'s insert under a shared interface target.
    #[cold]
    #[inline(never)]
    pub(crate) fn shared_reference_add(&mut self, target: tsrs_core::P<crate::Type>, reference: tsrs_core::P<crate::Type>) {
        let key = crate::get_type_list_key(reference.as_type_reference().resolved_type_arguments.get().unwrap_or(&[]));
        self.shared_instantiation_set(target.key(), key, reference);
    }

    /// `get_object_type_instantiation`'s probe of the frozen map under a shared target.
    #[cold]
    #[inline(never)]
    pub(crate) fn shared_object_instantiation_get(&self, target: tsrs_core::P<crate::Type>, key: crate::CacheHashKey, and: KeyAnd) -> Option<tsrs_core::P<crate::Type>> {
        self.object_type_instantiations.get_base(and, |b| b.get(&target).and_then(|m| m.get(&key)))
    }
}
