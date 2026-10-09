//! Opt-in allocation profile (`--features alloc-profile` on `tsrs_cli` / `tsrs_testrunner`, which forwards to
//! `tsrs_core/alloc-profile`; compiled out otherwise).
//!
//! Every leak-arena allocation (`P::new`, `alloc`, `alloc_slice`, `alloc_vec`, `alloc_str`) is recorded per
//! (call site, element type) through `#[track_caller]`; a counting global allocator (over mimalloc, the
//! binaries' production allocator) tracks the Rust heap (which includes the arena chunks). `dump()` prints top-N tables to stderr; `TSRS_ALLOC_PROFILE_TOP`
//! sets N (default 60). `TSRS_CENSUS=1` additionally records every live block and marks what is reachable at the
//! end of the run (`census`).

use rustc_hash::FxHashMap;
use mimalloc::MiMalloc as System;
use std::alloc::{GlobalAlloc, Layout};
use std::panic::Location;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

pub mod census;
mod os;

struct Counting;

static HEAP_CURRENT: AtomicUsize = AtomicUsize::new(0);
static HEAP_PEAK: AtomicUsize = AtomicUsize::new(0);
static HEAP_ALLOCS: AtomicUsize = AtomicUsize::new(0);

#[inline]
fn grow(n: usize) {
    let now = HEAP_CURRENT.fetch_add(n, Ordering::Relaxed) + n;
    HEAP_PEAK.fetch_max(now, Ordering::Relaxed);
}

/// The lazy declaration-file census's per-thread net heap bytes (arena growth excluded).
#[inline]
fn thread_heap(n: i64) {
    if !heap_sample::IN_ARENA.try_with(|c| c.get()).unwrap_or(true) {
        crate::lazydts_census::heap_delta(n);
    }
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            grow(layout.size());
            thread_heap(layout.size() as i64);
            HEAP_ALLOCS.fetch_add(1, Ordering::Relaxed);
            heap_sample::on_alloc(p, layout.size());
            census::on_alloc(p, layout.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(layout);
        if !p.is_null() {
            grow(layout.size());
            thread_heap(layout.size() as i64);
            HEAP_ALLOCS.fetch_add(1, Ordering::Relaxed);
            heap_sample::on_alloc(p, layout.size());
            census::on_alloc(p, layout.size());
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        heap_sample::on_free(p);
        census::on_free(p);
        if census::zero_on_free() {
            std::ptr::write_bytes(p, 0, layout.size());
        }
        System.dealloc(p, layout);
        HEAP_CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
        thread_heap(-(layout.size() as i64));
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        heap_sample::on_free(p);
        census::on_free(p);
        let q = if census::zero_on_free() {
            // Move by hand so the old block can be cleared before it is freed (see `census::zero_on_free`).
            let q = System.alloc(Layout::from_size_align_unchecked(new_size, layout.align()));
            if !q.is_null() {
                std::ptr::copy_nonoverlapping(p, q, layout.size().min(new_size));
                std::ptr::write_bytes(p, 0, layout.size());
                System.dealloc(p, layout);
            }
            q
        } else {
            System.realloc(p, layout, new_size)
        };
        if !q.is_null() {
            thread_heap(new_size as i64 - layout.size() as i64);
            if new_size >= layout.size() {
                grow(new_size - layout.size());
            } else {
                HEAP_CURRENT.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
            heap_sample::on_alloc(q, new_size);
            census::on_alloc(q, new_size);
        }
        q
    }
}

/// Sampling heap profiler (`TSRS_HEAP_PROFILE=1`): roughly every `RATE` allocated bytes the current
/// allocation's raw stack is recorded; live sampled bytes are aggregated per stack and resolved with `atos`.
/// `TSRS_HEAP_PROFILE=count` samples every `COUNT_RATE`-th allocation instead (allocation churn by call site,
/// whatever the size; arena chunk allocations included).
mod heap_sample {
    use rustc_hash::FxHashMap;
    use std::cell::Cell;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
    use std::sync::Mutex;

    pub(super) const RATE: isize = 256 * 1024;
    pub(super) const COUNT_RATE: isize = 1024;
    const DEPTH: usize = 20;
    pub(super) type Stack = [usize; DEPTH];

    extern "C" {
        fn backtrace(buf: *mut *mut c_void, size: i32) -> i32;
    }

    /// One sampled stack of one thread group (see `group`).
    pub(super) struct Site {
        pub(super) stack: Stack,
        pub(super) group: u8,
        pub(super) live: i64,
        pub(super) total: u64,
    }

    pub(super) struct State {
        pub(super) live: FxHashMap<usize, (u32, usize)>,
        pub(super) stacks: Vec<Site>,
        pub(super) index: FxHashMap<(Stack, u8), u32>,
        // Live bytes per stack when the heap last grew past `peak_heap` + 8 MB: "live at the heap peak".
        pub(super) peak: Vec<i64>,
        pub(super) peak_heap: usize,
    }
    pub(super) static STATE: Mutex<Option<State>> = Mutex::new(None);
    static ENABLED: AtomicU8 = AtomicU8::new(0); // 0 unknown, 1 off, 2 on (bytes), 3 on (counts)
    static SAMPLE_RATE: AtomicUsize = AtomicUsize::new(RATE as usize);

    thread_local! {
        static BUSY: Cell<bool> = const { Cell::new(false) };
        static COUNTDOWN: Cell<isize> = const { Cell::new(RATE) };
        pub(super) static IN_ARENA: Cell<bool> = const { Cell::new(false) };
        static GROUP: Cell<u8> = const { Cell::new(GROUP_UNKNOWN) };
    }

    /// Thread groups: the main thread (the CLI's `tsrs` thread), checker thread `checker-<n>` as `n + 1`, every other thread.
    pub(super) const GROUP_MAIN: u8 = 0;
    pub(super) const GROUP_OTHER: u8 = 254;
    const GROUP_UNKNOWN: u8 = 255;

    fn group() -> u8 {
        GROUP.try_with(|g| {
            if g.get() == GROUP_UNKNOWN {
                let current = std::thread::current();
                let group = match current.name() {
                    Some("main" | "tsrs") => GROUP_MAIN,
                    Some(name) => match name.strip_prefix("checker-").and_then(|n| n.parse::<u8>().ok()) {
                        Some(n) if n < GROUP_OTHER - 1 => n + 1,
                        _ => GROUP_OTHER,
                    },
                    None => GROUP_OTHER,
                };
                g.set(group);
            }
            g.get()
        })
        .unwrap_or(GROUP_OTHER)
    }

    pub(super) fn group_name(group: u8) -> String {
        match group {
            GROUP_MAIN => "main".to_string(),
            GROUP_OTHER => "other".to_string(),
            n => format!("checker-{}", n - 1),
        }
    }

    fn mode() -> u8 {
        match ENABLED.load(Ordering::Relaxed) {
            0 => {
                let v = std::env::var_os("TSRS_HEAP_PROFILE");
                let m = match v.as_ref().and_then(|v| v.to_str()) {
                    Some("1") => 2,
                    Some("count") => 3,
                    _ => 1,
                };
                // TSRS_HEAP_PROFILE_RATE: bytes between samples (default 256 KiB).
                // Relaxed: a profiling knob; see `on_alloc`.
                if let Some(rate) = std::env::var("TSRS_HEAP_PROFILE_RATE").ok().and_then(|s| s.parse::<usize>().ok()) {
                    SAMPLE_RATE.store(rate.max(1), Ordering::Relaxed);
                }
                ENABLED.store(m, Ordering::Relaxed);
                m
            }
            m => m,
        }
    }

    pub(super) fn counting() -> bool {
        ENABLED.load(Ordering::Relaxed) == 3
    }

    fn guarded(f: impl FnOnce()) {
        let _ = BUSY.try_with(|busy| {
            if busy.get() {
                return;
            }
            busy.set(true);
            f();
            busy.set(false);
        });
    }

    pub(super) fn on_alloc(p: *mut u8, size: usize) {
        guarded(|| {
            let mode = mode();
            if mode == 1 {
                return;
            }
            // Relaxed: a profiling knob written once before sampling starts; a stale read only shifts one sample.
            let byte_rate = SAMPLE_RATE.load(Ordering::Relaxed) as isize;
            let (cost, rate) = if mode == 3 { (1, COUNT_RATE) } else { (size as isize, byte_rate) };
            let left = COUNTDOWN.with(|c| {
                let left = c.get().min(rate) - cost;
                c.set(if left <= 0 { rate } else { left });
                left
            });
            if left > 0 {
                return;
            }
            let weight = if mode == 3 { COUNT_RATE as usize } else { size.max(byte_rate as usize) };
            let mut raw = [std::ptr::null_mut::<c_void>(); DEPTH + 3];
            // SAFETY: `raw` has room for DEPTH + 3 frames.
            let n = unsafe { backtrace(raw.as_mut_ptr(), raw.len() as i32) } as usize;
            let mut stack: Stack = [0; DEPTH];
            if IN_ARENA.with(|c| c.get()) {
                stack[0] = 1; // arena chunk
            } else {
                for (i, f) in raw.iter().take(n).skip(3).enumerate() {
                    stack[i] = *f as usize;
                }
            }
            let group = group();
            let mut guard = STATE.lock().unwrap();
            let state = guard.get_or_insert_with(|| State {
                live: FxHashMap::default(),
                stacks: Vec::new(),
                index: FxHashMap::default(),
                peak: Vec::new(),
                peak_heap: 0,
            });
            let next = state.stacks.len() as u32;
            let id = *state.index.entry((stack, group)).or_insert(next);
            if id == next {
                state.stacks.push(Site { stack, group, live: 0, total: 0 });
            }
            state.stacks[id as usize].live += weight as i64;
            state.stacks[id as usize].total += weight as u64;
            if mode == 2 {
                state.live.insert(p as usize, (id, weight));
                let now = super::HEAP_CURRENT.load(Ordering::Relaxed);
                if now > state.peak_heap + (8 << 20) {
                    state.peak_heap = now;
                    state.peak = state.stacks.iter().map(|s| s.live).collect();
                }
            }
        });
    }

    pub(super) fn on_free(p: *mut u8) {
        guarded(|| {
            if ENABLED.load(Ordering::Relaxed) != 2 {
                return;
            }
            let mut guard = STATE.lock().unwrap();
            if let Some(state) = guard.as_mut() {
                if let Some((id, weight)) = state.live.remove(&(p as usize)) {
                    state.stacks[id as usize].live -= weight as i64;
                }
            }
        });
    }

    fn library(name: &str) -> bool {
        ["hashbrown::", "indexmap::", "alloc::", "core::", "std::", "<alloc::", "<core::", "<std::", "<hashbrown::"]
            .iter()
            .any(|p| name.starts_with(p))
    }

    /// A resolved frame without its library parts (a frame is several names when calls were inlined); None if
    /// nothing else is left.
    fn own_frame(name: &str) -> Option<String> {
        let parts: Vec<&str> = name.split(" / ").filter(|p| !library(p)).collect();
        (!parts.is_empty()).then(|| parts.join(" / "))
    }

    pub(super) fn dump(top: usize) {
        let counting = counting();
        ENABLED.store(1, Ordering::Relaxed);
        let Some(state) = STATE.lock().unwrap().take() else {
            return;
        };
        // Sites merged over thread groups, as before; the per-group rows go to TSRS_HEAP_PROFILE_TSV.
        let mut merged: FxHashMap<Stack, usize> = FxHashMap::default();
        let mut sites: Vec<(Stack, i64, u64, i64)> = Vec::new(); // stack, live, total, live at peak
        for (i, site) in state.stacks.iter().enumerate() {
            let next = sites.len();
            let j = *merged.entry(site.stack).or_insert(next);
            if j == next {
                sites.push((site.stack, 0, 0, 0));
            }
            sites[j].1 += site.live;
            sites[j].2 += site.total;
            sites[j].3 += state.peak.get(i).copied().unwrap_or(0);
        }
        let all_ips: Vec<usize> = state.stacks.iter().flat_map(|s| s.stack.iter().copied().filter(|&ip| ip > 1)).collect();
        let mut names: FxHashMap<usize, String> = FxHashMap::default();
        let want_tsv = std::env::var_os("TSRS_HEAP_PROFILE_TSV");
        if want_tsv.is_some() {
            super::os::resolve_into(&all_ips, &mut names, super::census::function_name);
        }
        let frames_of = |stack: &Stack, names: &FxHashMap<usize, String>, limit: usize| -> Vec<String> {
            stack
                .iter()
                .filter(|&&ip| ip > 1)
                .filter_map(|ip| own_frame(names.get(ip).map(|s| s.as_str()).unwrap_or("?")))
                .take(limit)
                .collect()
        };
        if let Some(path) = want_tsv {
            use std::io::Write;
            let mut out = String::from("group\tlive_bytes\tpeak_bytes\ttotal_bytes\tframes\n");
            for (i, site) in state.stacks.iter().enumerate() {
                let frames = if site.stack[0] == 1 { vec!["<arena chunks>".to_string()] } else { frames_of(&site.stack, &names, 12) };
                out.push_str(&format!(
                    "{}\t{}\t{}\t{}\t{}\n",
                    group_name(site.group),
                    site.live,
                    state.peak.get(i).copied().unwrap_or(0),
                    site.total,
                    frames.join(" <- ")
                ));
            }
            if let Ok(mut f) = std::fs::File::create(&path) {
                let _ = f.write_all(out.as_bytes());
            }
            // Per thread group: sampled live bytes outside arena chunks.
            let mut groups: Vec<(u8, i64, i64)> = Vec::new();
            for (i, site) in state.stacks.iter().enumerate() {
                if site.stack[0] == 1 {
                    continue;
                }
                let at = match groups.iter().position(|g| g.0 == site.group) {
                    Some(at) => at,
                    None => {
                        groups.push((site.group, 0, 0));
                        groups.len() - 1
                    }
                };
                groups[at].1 += site.live;
                groups[at].2 += state.peak.get(i).copied().unwrap_or(0);
            }
            groups.sort_by_key(|g| g.0);
            eprintln!("\n-- heap outside arena chunks by thread (sampled): live MB / at heap peak MB --");
            for (g, live, peak) in groups {
                eprintln!("{:>12} {:>10.1} {:>10.1}", group_name(g), live as f64 / 1048576.0, peak as f64 / 1048576.0);
            }
        }
        let mut order: Vec<usize> = (0..sites.len()).collect();
        let print = |title: &str, order: &[usize], key: &dyn Fn(usize) -> i64, names: &mut FxHashMap<usize, String>| {
            let ips: Vec<usize> =
                order.iter().take(top).flat_map(|&i| sites[i].0.iter().copied().filter(|&ip| ip > 1)).collect();
            super::os::resolve_into(&ips, names, super::census::function_name);
            eprintln!("\n-- heap {title} (sampled, top {top}) --");
            for &i in order.iter().take(top) {
                let stack = &sites[i].0;
                if counting {
                    eprintln!("{:>10.3} M allocations", key(i) as f64 / 1e6);
                } else {
                    eprintln!("{:>10.1} MB", key(i) as f64 / (1024.0 * 1024.0));
                }
                if stack[0] == 1 {
                    eprintln!("             <arena chunks>");
                    continue;
                }
                for name in frames_of(stack, names, 6) {
                    eprintln!("             {name}");
                }
            }
        };
        if counting {
            let total: u64 = sites.iter().map(|s| s.2).sum();
            eprintln!("\nheap allocations (sampled every {COUNT_RATE}th): {:.1} M", total as f64 / 1e6);
            order.sort_by_key(|&i| -(sites[i].2 as i64));
            print("allocations by count", &order, &|i| sites[i].2 as i64, &mut names);
            return;
        }
        let (mut live, mut total) = (0i64, 0u64);
        for (stack, l, t, _) in &sites {
            if stack[0] != 1 {
                live += l;
                total += t;
            }
        }
        eprintln!(
            "\nheap outside arena chunks (sampled): live {:.1} MB, allocated {:.1} MB",
            live as f64 / 1048576.0,
            total as f64 / 1048576.0
        );
        order.sort_by_key(|&i| -sites[i].1);
        print("live", &order, &|i| sites[i].1, &mut names);
        let peak_live: i64 = sites.iter().filter(|s| s.0[0] != 1).map(|s| s.3).sum();
        eprintln!(
            "\nheap at its peak (snapshot at {:.1} MB counted): sampled live outside arena chunks {:.1} MB",
            state.peak_heap as f64 / 1048576.0,
            peak_live as f64 / 1048576.0
        );
        order.sort_by_key(|&i| -sites[i].3);
        print("live at the heap peak", &order, &|i| sites[i].3, &mut names);
        order.sort_by_key(|&i| -(sites[i].2 as i64));
        print("allocated (cumulative)", &order, &|i| sites[i].2 as i64, &mut names);
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

#[derive(Default, Clone, Copy)]
struct Entry {
    count: u64,
    bytes: u64,
}

type Key = (usize, usize);
struct ThreadData {
    index: FxHashMap<Key, u32>,
    sites: Vec<(&'static Location<'static>, &'static str, Entry)>,
    /// Census only: every arena block this thread allocated (address, size, index into `sites`), and the raw stack
    /// of every `census::ARENA_SAMPLE`-th one (address, stack id).
    blocks: Vec<(u64, u32, u32, u32)>, // address, size, site, census::next_seq()
    samples: Vec<(u64, u32)>,
    /// Census only: blocks the arena was asked to free or rewind (address, size); see `census::run`.
    would_free: Vec<(u64, u32, u32)>, // address, size, census::next_seq() when freed
    countdown: u32,
}
type Shared = Arc<Mutex<ThreadData>>;

static THREADS: Mutex<Vec<Shared>> = Mutex::new(Vec::new());
static ARENAS: Mutex<Vec<usize>> = Mutex::new(Vec::new());
/// The name of the thread that made each arena of `ARENAS` (same order).
static ARENA_THREADS: Mutex<Vec<String>> = Mutex::new(Vec::new());

thread_local! {
    // Census bookkeeping: made with tracking suspended, like the rest of it.
    static LOCAL: Shared = census::with_guard(|| {
        let data = Arc::new(Mutex::new(ThreadData {
            index: FxHashMap::default(),
            sites: Vec::new(),
            blocks: Vec::new(),
            samples: Vec::new(),
            would_free: Vec::new(),
            countdown: 0,
        }));
        THREADS.lock().unwrap().push(data.clone());
        data
    });
}

/// Marks heap allocations made while the arena grows (chunk allocations) so the heap sampler can tell them apart.
pub(crate) struct ArenaScope;
impl ArenaScope {
    #[inline]
    pub(crate) fn enter() -> ArenaScope {
        heap_sample::IN_ARENA.with(|c| c.set(true));
        ArenaScope
    }
}
impl Drop for ArenaScope {
    #[inline]
    fn drop(&mut self) {
        heap_sample::IN_ARENA.with(|c| c.set(false));
    }
}

pub(crate) fn register_arena(arena: &'static crate::arena::Arena) {
    let name = census::with_guard(|| std::thread::current().name().unwrap_or("?").to_string());
    let mut arenas = ARENAS.lock().unwrap();
    arenas.push(arena as *const crate::arena::Arena as usize);
    census::with_guard(|| ARENA_THREADS.lock().unwrap().push(name));
}

/// Free lists per arena (`free_stats`): bytes on the lists at exit and at their peak, the sum of the per-class peaks,
/// bytes handed out again and bytes the recycling sites bumped because the list was empty; then the bytes at exit by
/// size class, summed over the checker threads' arenas.
fn dump_free_lists() {
    let arenas = ARENAS.lock().unwrap();
    let names = ARENA_THREADS.lock().unwrap();
    let mut rows: Vec<(String, u64, [u64; 5])> = Vec::new();
    let mut by_class: Vec<[u64; 3]> = vec![[0; 3]; crate::arena::MAX_FREE_SIZE / 8 + 1];
    for (i, &a) in arenas.iter().enumerate() {
        // SAFETY: arenas are leaked; their statistics are read after every thread is done.
        let a = unsafe { &*(a as *const crate::arena::Arena) };
        let s = &a.free_stats;
        let mut v = [s.bytes.get(), s.peak_bytes.get(), 0, 0, 0];
        for c in 1..s.blocks.len() {
            let size = c as u64 * 8;
            v[2] += s.peak_blocks[c].get() * size;
            v[3] += s.reissued[c].get() * size;
            v[4] += s.misses[c].get() * size;
        }
        if v.iter().all(|&x| x == 0) {
            continue;
        }
        let name = names.get(i).cloned().unwrap_or_default();
        if name.starts_with("checker") || name == "tsrs" {
            for (c, row) in by_class.iter_mut().enumerate().skip(1) {
                row[0] += s.blocks[c].get() * c as u64 * 8;
                row[1] += s.peak_blocks[c].get() * c as u64 * 8;
                row[2] += s.misses[c].get() * c as u64 * 8;
            }
        }
        let used: u64 = a.used_ranges().iter().map(|&(_, len)| len as u64).sum();
        rows.push((name, used, v));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let kb = |b: u64| format!("{:.1}", b as f64 / 1024.0);
    eprintln!("\n-- arena free lists: thread, arena used MB, on lists at exit KB, peak on lists KB, sum of class peaks KB, reissued MB, bumped by recycling sites MB --");
    let mut tot = [0u64; 6];
    for (name, used, v) in &rows {
        eprintln!("{name:<14} {:>9} {:>9} {:>9} {:>9} {:>10} {:>10}", mb(*used), kb(v[0]), kb(v[1]), kb(v[2]), mb(v[3]), mb(v[4]));
        if name.starts_with("checker") || name == "tsrs" {
            tot[0] += used;
            for k in 0..5 {
                tot[k + 1] += v[k];
            }
        }
    }
    eprintln!("{:<14} {:>9} {:>9} {:>9} {:>9} {:>10} {:>10}", "checkers", mb(tot[0]), kb(tot[1]), kb(tot[2]), kb(tot[3]), mb(tot[4]), mb(tot[5]));
    let mut classes: Vec<(usize, [u64; 3])> = by_class.into_iter().enumerate().filter(|r| r.1[1] > 0).collect();
    classes.sort_by(|a, b| b.1[0].cmp(&a.1[0]));
    eprintln!("-- checker free lists by size class: size, on lists at exit KB, sum of per-arena class peaks KB, bumped MB --");
    for (c, r) in classes.iter().take(16) {
        eprintln!("{:>5} B {:>9} {:>9} {:>9}", c * 8, kb(r[0]), kb(r[1]), mb(r[2]));
    }
}

/// Census only: the number of arena blocks this thread has recorded (an arena checkpoint keeps it).
pub(crate) fn census_block_count() -> usize {
    if !census::recording() {
        return 0;
    }
    LOCAL.try_with(|local| local.lock().unwrap().blocks.len()).unwrap_or(0)
}

/// Census only: a block the arena would have freed (`P::free`); never reused, checked at exit.
pub(crate) fn census_would_free(addr: usize, size: usize) {
    let _ = LOCAL.try_with(|local| {
        let mut data = local.lock().unwrap();
        census::with_guard(|| data.would_free.push((addr as u64, size as u32, census::next_seq())));
    });
}

/// Census only: a freed region's used chunk range; every arena block recorded inside it is would-free as of now.
pub(crate) fn census_would_free_range(start: usize, len: usize) {
    census::with_guard(|| census::REGION_FREES.lock().unwrap().push((start as u64, len as u64, census::next_seq())));
}

/// Census only: every block this thread recorded since `census_block_count()` returned `since` would be discarded
/// by an arena rewind.
pub(crate) fn census_would_free_since(since: usize) {
    let _ = LOCAL.try_with(|local| {
        let mut data = local.lock().unwrap();
        let data = &mut *data;
        census::with_guard(|| {
            let seq = census::next_seq();
            for &(addr, size, _, _) in data.blocks.get(since..).unwrap_or(&[]) {
                data.would_free.push((addr, size, seq));
            }
        });
    });
}

#[inline(never)]
pub(crate) fn record(site: &'static Location<'static>, ty: &'static str, bytes: usize, addr: usize) {
    let _ = LOCAL.try_with(|local| {
        let mut data = local.lock().unwrap();
        let key = (site as *const Location as usize, ty.as_ptr() as usize ^ ty.len());
        let next = data.sites.len() as u32;
        let idx = *data.index.entry(key).or_insert(next);
        if idx == next {
            data.sites.push((site, ty, Entry::default()));
        }
        let e = &mut data.sites[idx as usize].2;
        e.count += 1;
        e.bytes += bytes as u64;
        if bytes != 0 && census::recording() {
            let size = u32::try_from(bytes).expect("arena block >= 4 GiB");
            let sample = data.countdown == 0;
            data.countdown = if sample { census::arena_sample_rate() - 1 } else { data.countdown - 1 };
            census::with_guard(|| {
                data.blocks.push((addr as u64, size, idx, census::next_seq()));
                if sample {
                    data.samples.push((addr as u64, census::stack_id()));
                }
            });
        }
    });
}

fn mb(b: u64) -> String {
    format!("{:.1}", b as f64 / (1024.0 * 1024.0))
}

pub(crate) fn short_type(ty: &str) -> String {
    // Strip module paths: `tsrs_checker::types::Type` -> `Type`.
    let mut out = String::new();
    let mut seg = String::new();
    for ch in ty.chars() {
        if ch.is_alphanumeric() || ch == '_' || ch == ':' {
            seg.push(ch);
        } else {
            out.push_str(seg.rsplit("::").next().unwrap_or(""));
            seg.clear();
            out.push(ch);
        }
    }
    out.push_str(seg.rsplit("::").next().unwrap_or(""));
    out
}

pub fn dump() {
    let top: usize = std::env::var("TSRS_ALLOC_PROFILE_TOP").ok().and_then(|s| s.parse().ok()).unwrap_or(60);
    let mut sites: FxHashMap<Key, (&'static Location<'static>, &'static str, Entry)> = FxHashMap::default();
    for t in THREADS.lock().unwrap().iter() {
        let data = t.lock().unwrap();
        for (k, &i) in data.index.iter() {
            let v = &data.sites[i as usize];
            let e = &mut sites.entry(*k).or_insert((v.0, v.1, Entry::default())).2;
            e.count += v.2.count;
            e.bytes += v.2.bytes;
        }
    }
    let arena_chunks: u64 = ARENAS
        .lock()
        .unwrap()
        .iter()
        .map(|&a| {
            // SAFETY: arenas are leaked; allocated_bytes only reads the chunk list header.
            let a = unsafe { &*(a as *const crate::arena::Arena) };
            a.capacity() as u64
        })
        .sum();
    let requested: u64 = sites.values().map(|v| v.2.bytes).sum();
    let heap_cur = HEAP_CURRENT.load(Ordering::Relaxed) as u64;
    let heap_peak = HEAP_PEAK.load(Ordering::Relaxed) as u64;
    eprintln!("== alloc profile ==");
    eprintln!("arena chunks:        {} MB ({} arenas)", mb(arena_chunks), ARENAS.lock().unwrap().len());
    eprintln!("arena requested:     {} MB", mb(requested));
    eprintln!("heap current:        {} MB (includes arena chunks)", mb(heap_cur));
    eprintln!("heap non-arena now:  {} MB", mb(heap_cur.saturating_sub(arena_chunks)));
    eprintln!("heap peak:           {} MB", mb(heap_peak));
    eprintln!("heap allocs:         {}", HEAP_ALLOCS.load(Ordering::Relaxed));
    // Per arena: chunk capacity against the bump-used part, and the unused part of the current chunk (the tail a
    // transparent huge page keeps resident up to 2 MiB; notes/mem-64.md). Histogram by used size.
    {
        let mut rows: Vec<(u64, u64, u64)> = ARENAS
            .lock()
            .unwrap()
            .iter()
            .map(|&a| {
                // SAFETY: arenas are leaked; the chunk list and the finger are read after every thread is done.
                let a = unsafe { &*(a as *const crate::arena::Arena) };
                let used: usize = a.used_ranges().iter().map(|&(_, len)| len).sum();
                (a.capacity() as u64, used as u64, a.current_chunk_unused() as u64)
            })
            .collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r.1));
        eprintln!("\n-- arenas (capacity / used / unused tail of the current chunk, MB) --");
        for (cap, used, tail) in rows.iter().take(top.min(12)) {
            eprintln!("{:>10} {:>10} {:>10}", mb(*cap), mb(*used), mb(*tail));
        }
        let bucket = |used: u64| match used >> 20 {
            0 => "< 1 MB",
            1 => "1-2 MB",
            2..=3 => "2-4 MB",
            4..=7 => "4-8 MB",
            8..=15 => "8-16 MB",
            16..=63 => "16-64 MB",
            _ => ">= 64 MB",
        };
        let mut hist: Vec<(&str, u64, u64, u64)> = Vec::new();
        for (cap, used, tail) in &rows {
            let b = bucket(*used);
            match hist.iter_mut().find(|h| h.0 == b) {
                Some(h) => {
                    h.1 += 1;
                    h.2 += *cap;
                    h.3 += *tail;
                }
                None => hist.push((b, 1, *cap, *tail)),
            }
        }
        eprintln!("{:>10} {:>7} {:>12} {:>12}", "used", "arenas", "capacity MB", "tails MB");
        for (b, n, cap, tail) in hist {
            eprintln!("{:>10} {:>7} {:>12} {:>12}", b, n, mb(cap), mb(tail));
        }
    }

    let mut by_type: FxHashMap<String, Entry> = FxHashMap::default();
    for v in sites.values() {
        let e = by_type.entry(short_type(v.1)).or_default();
        e.count += v.2.count;
        e.bytes += v.2.bytes;
    }
    let mut types: Vec<_> = by_type.into_iter().collect();
    types.sort_by(|a, b| b.1.bytes.cmp(&a.1.bytes));
    eprintln!("\n-- arena by type (top {top}) --");
    eprintln!("{:>10} {:>6} {:>12} {:>8}  type", "MB", "%", "count", "B/each");
    for (ty, e) in types.iter().take(top) {
        eprintln!(
            "{:>10} {:>6.2} {:>12} {:>8}  {}",
            mb(e.bytes),
            e.bytes as f64 * 100.0 / requested.max(1) as f64,
            e.count,
            e.bytes / e.count.max(1),
            ty
        );
    }

    dump_free_lists();

    heap_sample::dump(std::env::var("TSRS_HEAP_PROFILE_TOP").ok().and_then(|s| s.parse().ok()).unwrap_or(25));

    let mut list: Vec<_> = sites.into_values().collect();
    list.sort_by(|a, b| b.2.bytes.cmp(&a.2.bytes));
    eprintln!("\n-- arena by call site (top {top}) --");
    eprintln!("{:>10} {:>6} {:>12} {:>8}  site / type", "MB", "%", "count", "B/each");
    for (loc, ty, e) in list.iter().take(top) {
        let file = loc.file();
        let file = file.find("crates/").map(|i| &file[i + 7..]).unwrap_or(file);
        eprintln!(
            "{:>10} {:>6.2} {:>12} {:>8}  {}:{}  {}",
            mb(e.bytes),
            e.bytes as f64 * 100.0 / requested.max(1) as f64,
            e.count,
            e.bytes / e.count.max(1),
            file,
            loc.line(),
            short_type(ty)
        );
    }
}
