//! Measurement hooks for the speculative-region census (notes/mem-overload-rollback.md). Compiled to nothing
//! without the `alloc-profile` feature; with it, they do work only while `TSRS_CENSUS=1` records.
//!
//! - `region_enter` / `region_exit`: allocations made while at least one region is entered on this thread are tagged
//!   as region allocations; at exit the census reports how many of them are still reachable, conservatively
//!   (`census::run`) and with the weak tables below treated as weak.
//! - `weak(addr)`: the block containing `addr` is a cache whose keys are object identities (type / symbol ids or
//!   pointers): the second mark does not scan it.
//! - `ephemeron(keys, start, len)`: a cache entry inside a weak block whose value words are `start .. start + len`;
//!   the second mark scans them once every key address is reachable (an entry whose key involves a dead object can
//!   never be looked up again, since ids are never reused).
//! - `note(kind, addr)` / `noted(kind)`: objects the weak-table walk must find later (checker-created symbols, so
//!   link stores keyed by symbol id can name the symbol of a slot; instantiation tables inside types and links).

#[inline(always)]
pub fn census_on() -> bool {
    #[cfg(feature = "alloc-profile")]
    {
        crate::alloc_profile::census::recording()
    }
    #[cfg(not(feature = "alloc-profile"))]
    {
        false
    }
}

#[inline(always)]
pub fn region_enter() {
    #[cfg(feature = "alloc-profile")]
    crate::alloc_profile::census::region_enter();
}

#[inline(always)]
pub fn region_exit() {
    #[cfg(feature = "alloc-profile")]
    crate::alloc_profile::census::region_exit();
}

/// Which region kind the census measures (`TSRS_CENSUS_REGION`: `applicable` (default), `overload`, `none`).
#[inline(always)]
pub fn region_kind() -> u8 {
    #[cfg(feature = "alloc-profile")]
    {
        crate::alloc_profile::census::region_kind()
    }
    #[cfg(not(feature = "alloc-profile"))]
    {
        0
    }
}

#[inline(always)]
pub fn weak(addr: usize) {
    #[cfg(feature = "alloc-profile")]
    crate::alloc_profile::census::weak(addr);
    #[cfg(not(feature = "alloc-profile"))]
    let _ = addr;
}

#[inline(always)]
pub fn ephemeron(keys: &[usize], start: usize, len: usize) {
    #[cfg(feature = "alloc-profile")]
    crate::alloc_profile::census::ephemeron(keys, start, len);
    #[cfg(not(feature = "alloc-profile"))]
    let _ = (keys, start, len);
}

pub const NOTE_TRANSIENT_SYMBOL: u8 = 0;
pub const NOTE_REFERENCE_INSTANTIATIONS: u8 = 1;
pub const NOTE_ALIAS_INSTANTIATIONS: u8 = 2;
pub const NOTE_CONDITIONAL_INSTANTIATIONS: u8 = 3;
pub const NOTE_TYPE: u8 = 4;

#[inline(always)]
pub fn note(kind: u8, addr: usize) {
    #[cfg(feature = "alloc-profile")]
    crate::alloc_profile::census::note(kind, addr);
    #[cfg(not(feature = "alloc-profile"))]
    let _ = (kind, addr);
}

pub fn noted(kind: u8) -> Vec<usize> {
    #[cfg(feature = "alloc-profile")]
    {
        crate::alloc_profile::census::noted(kind)
    }
    #[cfg(not(feature = "alloc-profile"))]
    {
        let _ = kind;
        Vec::new()
    }
}
