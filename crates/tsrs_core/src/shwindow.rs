//! spike/r1-read-path (research, never merged): the read path of an immutable shared type layer, priced with the
//! layer never built (notes/design-shared-type-layer.md sections 5.2 and 5.3, experiment R1).
//!
//! The top `WINDOW` bytes of the arena reservation are kept for the layer's frozen region, so "is this object shared"
//! is one comparison of its handle (or of its address minus a constant) against a constant, with no load: the
//! reservation sits at a constant address (`reserve::BASE_ADDR`). Nothing is ever allocated in the window on this
//! branch (`reserve` stops below it), so every object is private, every test here is false and every `#[cold]` path is
//! dead. What the branch measures is the cost of the tests and of the code behind them.

use crate::ptr::{PKey, P};

/// The window: the top 4 GiB of the 32 GiB reservation.
#[cfg(compressed_ptrs)]
pub const WINDOW: usize = 4 << 30;
/// The window's offset in the reservation; chunks are carved only below it.
#[cfg(compressed_ptrs)]
pub const WINDOW_OFF: usize = crate::reserve::RESERVE - WINDOW;
/// Handles at or above this lie in the window (0xE000_0000). Their top three bits are set, so the AND of handles that
/// all lie in the window lies in it too, and the AND of any set that holds one private handle does not (`KeyAnd`).
#[cfg(compressed_ptrs)]
pub const WINDOW_UNIT: u32 = (WINDOW_OFF >> crate::reserve::UNIT_SHIFT) as u32;

/// Whether the object with this key (`P::key`: the handle) lies in the window. Plain pointers have no window.
#[inline]
pub fn shared_key(k: PKey) -> bool {
    #[cfg(compressed_ptrs)]
    return k >= WINDOW_UNIT;
    #[cfg(not(compressed_ptrs))]
    {
        let _ = k;
        false
    }
}

/// Whether the byte at `addr` lies in the window.
#[inline]
pub fn shared_addr(addr: usize) -> bool {
    #[cfg(compressed_ptrs)]
    return addr.wrapping_sub(crate::reserve::BASE_ADDR + WINDOW_OFF) < WINDOW;
    #[cfg(not(compressed_ptrs))]
    {
        let _ = addr;
        false
    }
}

/// Whether `r` (a field of an arena object) lies in the window.
#[inline]
pub fn shared_ref<T: ?Sized>(r: &T) -> bool {
    shared_addr(std::ptr::from_ref(r).cast::<u8>().addr())
}

impl<T: ?Sized> P<T> {
    /// Whether the object is in the shared layer's window (never, on this branch).
    #[inline]
    pub fn is_shared(self) -> bool {
        shared_key(self.key())
    }
}

/// The AND of the handles a key was built from (`keyBuilder`): a two-level map probes the frozen map only when it lies
/// in the window, which is when every handle does. A key with no handle (a literal's value) starts and stays at
/// `KeyAnd::NONE`, which lies in the window.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KeyAnd(pub PKey);

impl KeyAnd {
    pub const NONE: KeyAnd = KeyAnd(PKey::MAX);

    #[inline]
    pub fn add<T: ?Sized>(&mut self, p: P<T>) {
        self.0 &= p.key();
    }

    #[inline]
    pub fn and(self, other: KeyAnd) -> KeyAnd {
        KeyAnd(self.0 & other.0)
    }

    /// Whether every handle of the key lies in the window.
    #[inline]
    pub fn all_shared(self) -> bool {
        shared_key(self.0)
    }
}

impl Default for KeyAnd {
    fn default() -> Self {
        KeyAnd::NONE
    }
}
