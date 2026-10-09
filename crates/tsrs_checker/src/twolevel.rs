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
