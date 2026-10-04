//! Throwaway probe for the persisted front-end design (branch design/persisted-frontend-probe, not for main).
//!
//! `TSRS_PFE_PROBE=1`: every parsed file gets its own region (as in the language server), every region chunk is its
//! own page-aligned mapping, and once all files are bound (just before the checkers are created) every region page
//! is `mprotect`ed to `PROT_NONE`. A SIGSEGV/SIGBUS handler records the first read and the first write of each page
//! and opens the page (read-only after a read, read-write after a write). At exit the per-file counts tell which
//! pages of each file's parse + bind output the rest of the run touched: what a lazily mapped cache would page in,
//! and what it would dirty.

use crate::arena::Region;
use std::sync::atomic::{AtomicPtr, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

pub const PAGE: usize = 16384;

pub fn enabled() -> bool {
    static E: OnceLock<bool> = OnceLock::new();
    *E.get_or_init(|| std::env::var("TSRS_PFE_PROBE").is_ok_and(|v| !v.is_empty() && v != "0"))
}

pub struct FileRec {
    pub file_addr: usize,
    pub region: Region,
    pub parse_ns: u64,
    pub bind_ns: AtomicU64,
    /// Filled by `protect`: (chunk start, mapped length) of every chunk at protection time.
    pub chunks: Mutex<Vec<(usize, usize)>>,
    pub used_at_protect: AtomicUsize,
    pub cap_at_protect: AtomicUsize,
    pub used_ranges: Mutex<Vec<(usize, usize)>>,
}

static FILES: Mutex<Vec<&'static FileRec>> = Mutex::new(Vec::new());
static BY_ADDR: OnceLock<Mutex<rustc_hash::FxHashMap<usize, &'static FileRec>>> = OnceLock::new();

fn by_addr() -> &'static Mutex<rustc_hash::FxHashMap<usize, &'static FileRec>> {
    BY_ADDR.get_or_init(Default::default)
}

pub fn thread_cpu_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: writes one timespec.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

pub fn register_file(file_addr: usize, region: Region, parse_ns: u64) {
    let rec: &'static FileRec = Box::leak(Box::new(FileRec {
        file_addr,
        region,
        parse_ns,
        bind_ns: AtomicU64::new(0),
        chunks: Mutex::new(Vec::new()),
        used_at_protect: AtomicUsize::new(0),
        cap_at_protect: AtomicUsize::new(0),
        used_ranges: Mutex::new(Vec::new()),
    }));
    FILES.lock().unwrap().push(rec);
    by_addr().lock().unwrap().insert(file_addr, rec);
}

pub fn record_bind(file_addr: usize, ns: u64) {
    if let Some(rec) = by_addr().lock().unwrap().get(&file_addr) {
        rec.bind_ns.fetch_add(ns, Ordering::Relaxed);
    }
}

pub fn files() -> Vec<&'static FileRec> {
    FILES.lock().unwrap().clone()
}

struct Table {
    starts: Vec<usize>,
    ends: Vec<usize>,
    offs: Vec<usize>,
    states: Box<[AtomicU8]>,
}

static TABLE: AtomicPtr<Table> = AtomicPtr::new(std::ptr::null_mut());
static FAULTS: AtomicU64 = AtomicU64::new(0);
static mut OLD_SEGV: std::mem::MaybeUninit<libc::sigaction> = std::mem::MaybeUninit::uninit();
static mut OLD_BUS: std::mem::MaybeUninit<libc::sigaction> = std::mem::MaybeUninit::uninit();
static PROTECT_NS: AtomicU64 = AtomicU64::new(0);
const PCS: usize = 1 << 20;
static READ_PCS: [AtomicU64; PCS] = [const { AtomicU64::new(0) }; PCS];
static WRITE_PCS: [AtomicU64; PCS] = [const { AtomicU64::new(0) }; PCS];
static N_READ: AtomicUsize = AtomicUsize::new(0);
static N_WRITE: AtomicUsize = AtomicUsize::new(0);

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
unsafe fn fault_pc(ctx: *mut libc::c_void) -> u64 {
    let uc = ctx as *const libc::ucontext_t;
    (*(*uc).uc_mcontext).__ss.__pc
}
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
unsafe fn fault_pc(_ctx: *mut libc::c_void) -> u64 {
    0
}

extern "C" {
    fn _dyld_get_image_vmaddr_slide(image_index: u32) -> isize;
}

/// First-touch program counters (minus the ASLR slide, for `atos -o tsrs`), most frequent first.
pub fn top_pcs(write: bool, n: usize) -> Vec<(u64, usize)> {
    let (arr, cnt) = if write { (&WRITE_PCS, &N_WRITE) } else { (&READ_PCS, &N_READ) };
    let len = cnt.load(Ordering::Relaxed).min(PCS);
    // SAFETY: plain dyld query.
    let slide = unsafe { _dyld_get_image_vmaddr_slide(0) } as u64;
    let mut m: rustc_hash::FxHashMap<u64, usize> = Default::default();
    for a in &arr[..len] {
        *m.entry(a.load(Ordering::Relaxed).wrapping_sub(slide)).or_default() += 1;
    }
    let mut v: Vec<_> = m.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    v.truncate(n);
    v
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
unsafe fn fault_is_write(ctx: *mut libc::c_void) -> bool {
    let uc = ctx as *const libc::ucontext_t;
    let mc = (*uc).uc_mcontext;
    let esr = (*mc).__es.__esr;
    let ec = esr >> 26;
    (ec == 0x24 || ec == 0x25) && esr & (1 << 6) != 0
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
unsafe fn fault_is_write(_ctx: *mut libc::c_void) -> bool {
    false
}

extern "C" fn handler(sig: libc::c_int, info: *mut libc::siginfo_t, ctx: *mut libc::c_void) {
    // SAFETY: the kernel passes a valid siginfo and ucontext; the table is never freed once published.
    unsafe {
        let addr = (*info).si_addr as usize;
        let t = TABLE.load(Ordering::Acquire);
        if !t.is_null() {
            let t = &*t;
            let i = t.starts.partition_point(|&s| s <= addr);
            if i > 0 && addr < t.ends[i - 1] {
                let start = t.starts[i - 1];
                let page = (addr - start) / PAGE;
                let state = &t.states[t.offs[i - 1] + page];
                let want = if fault_is_write(ctx) { 2 } else { 1 };
                let old = state.fetch_max(want, Ordering::AcqRel);
                if old < want {
                    FAULTS.fetch_add(1, Ordering::Relaxed);
                    let (arr, cnt) = if want == 2 { (&WRITE_PCS, &N_WRITE) } else { (&READ_PCS, &N_READ) };
                    let k = cnt.fetch_add(1, Ordering::Relaxed);
                    if k < PCS {
                        arr[k].store(fault_pc(ctx), Ordering::Relaxed);
                    }
                }
                let prot = if old.max(want) == 2 { libc::PROT_READ | libc::PROT_WRITE } else { libc::PROT_READ };
                libc::mprotect((start + page * PAGE) as *mut libc::c_void, PAGE, prot);
                return;
            }
        }
        // Not ours: restore the previous action and return, so the fault repeats under it.
        let old = if sig == libc::SIGSEGV { (*std::ptr::addr_of!(OLD_SEGV)).as_ptr() } else { (*std::ptr::addr_of!(OLD_BUS)).as_ptr() };
        libc::sigaction(sig, old, std::ptr::null_mut());
    }
}

/// Protects every page of every registered region. Call once, after binding, before checking.
pub fn protect() {
    let t0 = std::time::Instant::now();
    let files = files();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for rec in &files {
        let mut chunks = Vec::new();
        for (start, size) in rec.region.chunk_ranges() {
            assert_eq!(start % PAGE, 0, "probe chunks must be page aligned");
            let len = size.next_multiple_of(PAGE);
            chunks.push((start, len));
            ranges.push((start, len));
        }
        rec.used_at_protect.store(rec.region.used_bytes(), Ordering::Relaxed);
        rec.cap_at_protect.store(rec.region.allocated_bytes(), Ordering::Relaxed);
        *rec.used_ranges.lock().unwrap() = rec.region.used_ranges();
        *rec.chunks.lock().unwrap() = chunks;
    }
    ranges.sort_unstable();
    let mut starts = Vec::with_capacity(ranges.len());
    let mut ends = Vec::with_capacity(ranges.len());
    let mut offs = Vec::with_capacity(ranges.len());
    let mut total_pages = 0;
    for &(start, len) in &ranges {
        if let Some(&e) = ends.last() {
            assert!(start >= e, "overlapping probe chunks");
        }
        starts.push(start);
        ends.push(start + len);
        offs.push(total_pages);
        total_pages += len / PAGE;
    }
    let states: Box<[AtomicU8]> = (0..total_pages).map(|_| AtomicU8::new(0)).collect();
    let table = Box::leak(Box::new(Table { starts, ends, offs, states }));
    TABLE.store(table, Ordering::Release);
    // SAFETY: installs a handler that only touches the leaked table; mprotect on whole mappings we own.
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = handler as *const () as usize;
        sa.sa_flags = libc::SA_SIGINFO | libc::SA_NODEFER;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGSEGV, &sa, (*std::ptr::addr_of_mut!(OLD_SEGV)).as_mut_ptr());
        libc::sigaction(libc::SIGBUS, &sa, (*std::ptr::addr_of_mut!(OLD_BUS)).as_mut_ptr());
        for &(start, len) in &ranges {
            let r = libc::mprotect(start as *mut libc::c_void, len, libc::PROT_NONE);
            assert_eq!(r, 0, "mprotect failed");
        }
    }
    PROTECT_NS.store(t0.elapsed().as_nanos() as u64, Ordering::Relaxed);
}

pub struct PageStats {
    pub pages: usize,
    pub touched: usize,
    pub written: usize,
}

/// Page states of one file's chunks (as of now).
pub fn page_stats(rec: &FileRec) -> PageStats {
    let t = TABLE.load(Ordering::Acquire);
    let mut s = PageStats { pages: 0, touched: 0, written: 0 };
    if t.is_null() {
        return s;
    }
    // SAFETY: published and never freed.
    let t = unsafe { &*t };
    for &(ustart, ulen) in rec.used_ranges.lock().unwrap().iter() {
        if ulen == 0 {
            continue;
        }
        let i = t.starts.partition_point(|&x| x <= ustart) - 1;
        let start = t.starts[i];
        let first = (ustart - start) / PAGE;
        let last = (ustart + ulen - 1 - start) / PAGE;
        for p in first..=last {
            let st = t.states[t.offs[i] + p].load(Ordering::Relaxed);
            s.pages += 1;
            if st >= 1 {
                s.touched += 1;
            }
            if st == 2 {
                s.written += 1;
            }
        }
    }
    s
}

pub fn faults() -> u64 {
    FAULTS.load(Ordering::Relaxed)
}

pub fn protect_seconds() -> f64 {
    PROTECT_NS.load(Ordering::Relaxed) as f64 * 1e-9
}

/// An `mmap`ed, page-aligned mapping for one region chunk (probe mode only).
pub(crate) fn map_chunk(size: usize) -> *mut u8 {
    let len = size.next_multiple_of(PAGE);
    // SAFETY: anonymous private mapping.
    let p = unsafe {
        libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_PRIVATE | libc::MAP_ANON, -1, 0)
    };
    assert!(p != libc::MAP_FAILED, "probe mmap failed");
    p.cast()
}
