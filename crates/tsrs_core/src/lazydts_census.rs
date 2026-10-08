//! Lazy declaration-file member census (alloc-profile builds, `TSRS_LAZY_DTS_CENSUS=1`; notes/mem-lazy-dts-members.md).
//!
//! Measures the ceiling of parsing and binding the member lists of unchecked declaration files lazily:
//!
//! - the parser and the binder report the arena bytes (plus this thread's net heap bytes) each member of an
//!   interface, class, type literal or module block of a declaration file took (`note_parse_*`, `note_bind_member`);
//! - the compiler registers those lists, their owner symbols and their member symbols before the first checker is
//!   created (`install`), and from then on every read of a registered list's nodes (`mark_list`), of a registered
//!   owner's `members` / `exports` table (`mark_owner`) and every symbol-table hit or declarations read of a registered
//!   member symbol (`mark_member`) sets a bit, tagged with the phase it happened in (before checker creation, inside
//!   `new_checker`, later).
//!
//! In other builds every function here is an empty inline function and `enabled()` is `false`.

/// The address the census keys an arena object by.
#[inline]
pub fn addr_of<T>(r: &T) -> usize {
    std::ptr::from_ref(r).addr()
}

/// Per-thread records of the parser and binder (profile builds).
#[derive(Default)]
pub struct Records {
    pub lists: Vec<(usize, u64, u8)>,
    pub parse_members: Vec<(usize, u64)>,
    pub bind_members: Vec<(usize, u64)>,
    pub parse_files: Vec<(usize, u64)>,
    pub bind_files: Vec<(usize, u64)>,
}

/// The registered lists, owners and member symbols and their phase bits (profile builds).
pub struct Registry {
    pub lists: rustc_hash::FxHashMap<usize, u32>,
    pub owners: rustc_hash::FxHashMap<usize, u32>,
    pub members: rustc_hash::FxHashMap<usize, u32>,
    pub list_bits: Vec<std::sync::atomic::AtomicU8>,
    /// Per owner: phase bits of `members` reads (low nibble) and of `exports` reads (high nibble).
    pub owner_bits: Vec<std::sync::atomic::AtomicU8>,
    pub member_bits: Vec<std::sync::atomic::AtomicU8>,
}

#[cfg(feature = "alloc-profile")]
mod imp {
    use std::cell::{Cell, RefCell};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};

    static ENABLED: OnceLock<bool> = OnceLock::new();

    pub fn enabled() -> bool {
        *ENABLED.get_or_init(|| std::env::var_os("TSRS_LAZY_DTS_CENSUS").is_some_and(|v| v == "1"))
    }

    thread_local! {
        static HEAP_NET: Cell<i64> = const { Cell::new(0) };
        static IN_INIT: Cell<bool> = const { Cell::new(false) };
        static LOCAL: Arc<Mutex<Records>> = {
            let r = Arc::new(Mutex::new(Records::default()));
            ALL.lock().unwrap().push(r.clone());
            r
        };
        static PENDING: RefCell<Records> = RefCell::new(Records::default());
    }

    /// Heap bytes this thread allocated minus what it freed (the counting allocator, outside arena growth).
    #[inline]
    pub(crate) fn heap_delta(n: i64) {
        let _ = HEAP_NET.try_with(|c| c.set(c.get() + n));
    }

    /// Arena bytes in use on this thread's current target plus this thread's net heap bytes.
    pub fn bytes_now() -> u64 {
        let heap = HEAP_NET.with(|c| c.get());
        (crate::ptr::arena_used_bytes() as i64 + heap).max(0) as u64
    }

    use super::{Records, Registry};

    static ALL: Mutex<Vec<Arc<Mutex<Records>>>> = Mutex::new(Vec::new());

    fn push(f: impl FnOnce(&mut Records)) {
        PENDING.with(|p| {
            let mut p = p.borrow_mut();
            f(&mut p);
            if p.lists.len() + p.parse_members.len() + p.bind_members.len() > 4096 {
                flush_one(&mut p);
            }
        });
    }

    fn flush_one(p: &mut Records) {
        LOCAL.with(|l| {
            let mut l = l.lock().unwrap();
            l.lists.append(&mut p.lists);
            l.parse_members.append(&mut p.parse_members);
            l.bind_members.append(&mut p.bind_members);
            l.parse_files.append(&mut p.parse_files);
            l.bind_files.append(&mut p.bind_files);
        });
    }

    /// Flushes this thread's pending records (call at the end of each file's parse and bind).
    pub fn flush() {
        if !enabled() {
            return;
        }
        PENDING.with(|p| flush_one(&mut p.borrow_mut()));
    }

    pub fn note_parse_list(list: usize, bytes: u64, disq: u8) {
        push(|r| r.lists.push((list, bytes, disq)));
    }
    pub fn note_parse_member(member: usize, bytes: u64) {
        push(|r| r.parse_members.push((member, bytes)));
    }
    pub fn note_bind_member(member: usize, bytes: u64) {
        push(|r| r.bind_members.push((member, bytes)));
    }
    pub fn note_parse_file(file: usize, bytes: u64) {
        push(|r| r.parse_files.push((file, bytes)));
        flush();
    }
    pub fn note_bind_file(file: usize, bytes: u64) {
        push(|r| r.bind_files.push((file, bytes)));
        flush();
    }

    /// Every record of every thread so far (records of threads that have not flushed are missed: every file's
    /// parse and bind ends with a flush).
    pub fn take_records() -> Records {
        let mut out = Records::default();
        for r in ALL.lock().unwrap().iter() {
            let mut r = r.lock().unwrap();
            out.lists.append(&mut r.lists);
            out.parse_members.append(&mut r.parse_members);
            out.bind_members.append(&mut r.bind_members);
            out.parse_files.append(&mut r.parse_files);
            out.bind_files.append(&mut r.bind_files);
        }
        out
    }

    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    static MARKING: AtomicBool = AtomicBool::new(false);
    static CREATED: AtomicBool = AtomicBool::new(false);

    pub fn install(r: Registry) {
        let _ = REGISTRY.set(r);
        MARKING.store(true, Ordering::SeqCst);
    }

    pub fn registry() -> Option<&'static Registry> {
        REGISTRY.get()
    }

    /// Phase bits: 1 = after registration, before any checker exists; 2 = inside `new_checker`; 4 = later.
    #[inline]
    fn phase() -> u8 {
        if IN_INIT.with(|c| c.get()) {
            2
        // Relaxed: census bookkeeping; a late view only tags a read with an earlier phase.
        } else if CREATED.load(Ordering::Relaxed) {
            4
        } else {
            1
        }
    }

    pub fn set_in_checker_init(on: bool) {
        if !enabled() {
            return;
        }
        IN_INIT.with(|c| c.set(on));
        if on {
            // Relaxed: as in `phase`.
            CREATED.store(true, Ordering::Relaxed);
        }
    }

    #[inline]
    pub fn mark_list(addr: usize) {
        // Relaxed: set (SeqCst) before the checker threads start; the registry is a OnceLock.
        if !MARKING.load(Ordering::Relaxed) {
            return;
        }
        mark_list_slow(addr);
    }
    #[inline(never)]
    fn mark_list_slow(addr: usize) {
        let r = REGISTRY.get().unwrap();
        if let Some(&i) = r.lists.get(&addr) {
            // Relaxed: independent bits, read after the threads are joined.
            r.list_bits[i as usize].fetch_or(phase(), Ordering::Relaxed);
        }
    }

    #[inline]
    pub fn mark_owner(addr: usize, exports: bool) {
        // Relaxed: as in `mark_list`.
        if !MARKING.load(Ordering::Relaxed) {
            return;
        }
        mark_owner_slow(addr, exports);
    }
    #[inline(never)]
    fn mark_owner_slow(addr: usize, exports: bool) {
        let r = REGISTRY.get().unwrap();
        if let Some(&i) = r.owners.get(&addr) {
            let bit = if exports { phase() << 4 } else { phase() };
            // Relaxed: as in `mark_list_slow`.
            r.owner_bits[i as usize].fetch_or(bit, Ordering::Relaxed);
        }
    }

    #[inline]
    pub fn mark_member(addr: usize) {
        // Relaxed: as in `mark_list`.
        if !MARKING.load(Ordering::Relaxed) {
            return;
        }
        mark_member_slow(addr);
    }
    #[inline(never)]
    fn mark_member_slow(addr: usize) {
        let r = REGISTRY.get().unwrap();
        if let Some(&i) = r.members.get(&addr) {
            // Relaxed: as in `mark_list_slow`.
            r.member_bits[i as usize].fetch_or(phase(), Ordering::Relaxed);
        }
    }

    /// Stops marking (before the report walks the program).
    pub fn stop() {
        MARKING.store(false, Ordering::SeqCst);
    }
}

#[cfg(not(feature = "alloc-profile"))]
mod imp {
    use super::{Records, Registry};
    pub fn take_records() -> Records {
        Records::default()
    }
    pub fn install(_r: Registry) {}
    pub fn registry() -> Option<&'static Registry> {
        None
    }
    #[inline]
    pub const fn enabled() -> bool {
        false
    }
    #[inline]
    pub fn bytes_now() -> u64 {
        0
    }
    #[inline]
    pub fn flush() {}
    #[inline]
    pub fn note_parse_list(_list: usize, _bytes: u64, _disq: u8) {}
    #[inline]
    pub fn note_parse_member(_member: usize, _bytes: u64) {}
    #[inline]
    pub fn note_bind_member(_member: usize, _bytes: u64) {}
    #[inline]
    pub fn note_parse_file(_file: usize, _bytes: u64) {}
    #[inline]
    pub fn note_bind_file(_file: usize, _bytes: u64) {}
    #[inline]
    pub fn set_in_checker_init(_on: bool) {}
    #[inline]
    pub fn mark_list(_addr: usize) {}
    #[inline]
    pub fn mark_owner(_addr: usize, _exports: bool) {}
    #[inline]
    pub fn mark_member(_addr: usize) {}
    #[inline]
    pub fn stop() {}
}

pub use imp::*;
