//! Opt-in allocation profile (`--features tsrs_core/alloc-profile`, compiled out otherwise).
//!
//! Every leak-arena allocation (`P::new`, `alloc`, `alloc_slice`, `alloc_vec`, `alloc_str`) is recorded per
//! (call site, element type) through `#[track_caller]`; a counting global allocator tracks the Rust heap
//! (which includes the arena chunks). `dump()` prints top-N tables to stderr; `TSRS_ALLOC_PROFILE_TOP`
//! sets N (default 60).

use rustc_hash::FxHashMap;
use std::alloc::{GlobalAlloc, Layout, System};
use std::panic::Location;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

struct Counting;

static HEAP_CURRENT: AtomicUsize = AtomicUsize::new(0);
static HEAP_PEAK: AtomicUsize = AtomicUsize::new(0);
static HEAP_ALLOCS: AtomicUsize = AtomicUsize::new(0);

#[inline]
fn grow(n: usize) {
    let now = HEAP_CURRENT.fetch_add(n, Ordering::Relaxed) + n;
    HEAP_PEAK.fetch_max(now, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            grow(layout.size());
            HEAP_ALLOCS.fetch_add(1, Ordering::Relaxed);
            heap_sample::on_alloc(p, layout.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(layout);
        if !p.is_null() {
            grow(layout.size());
            HEAP_ALLOCS.fetch_add(1, Ordering::Relaxed);
            heap_sample::on_alloc(p, layout.size());
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        heap_sample::on_free(p);
        System.dealloc(p, layout);
        HEAP_CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        heap_sample::on_free(p);
        let q = System.realloc(p, layout, new_size);
        if !q.is_null() {
            if new_size >= layout.size() {
                grow(new_size - layout.size());
            } else {
                HEAP_CURRENT.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
            heap_sample::on_alloc(q, new_size);
        }
        q
    }
}

/// Sampling heap profiler (`TSRS_HEAP_PROFILE=1`): roughly every `RATE` allocated bytes the current
/// allocation's raw stack is recorded; live sampled bytes are aggregated per stack and resolved with `atos`.
mod heap_sample {
    use rustc_hash::FxHashMap;
    use std::cell::Cell;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::sync::Mutex;

    pub(super) const RATE: isize = 256 * 1024;
    const DEPTH: usize = 20;
    pub(super) type Stack = [usize; DEPTH];

    extern "C" {
        fn backtrace(buf: *mut *mut c_void, size: i32) -> i32;
    }

    pub(super) struct State {
        pub(super) live: FxHashMap<usize, (u32, usize)>,
        pub(super) stacks: Vec<(Stack, i64, u64)>, // stack, live bytes, total sampled bytes
        pub(super) index: FxHashMap<Stack, u32>,
    }
    pub(super) static STATE: Mutex<Option<State>> = Mutex::new(None);
    static ENABLED: AtomicU8 = AtomicU8::new(0); // 0 unknown, 1 off, 2 on

    thread_local! {
        static BUSY: Cell<bool> = const { Cell::new(false) };
        static COUNTDOWN: Cell<isize> = const { Cell::new(RATE) };
        pub(super) static IN_ARENA: Cell<bool> = const { Cell::new(false) };
    }

    fn enabled() -> bool {
        match ENABLED.load(Ordering::Relaxed) {
            1 => false,
            2 => true,
            _ => {
                let on = std::env::var_os("TSRS_HEAP_PROFILE").is_some_and(|v| v == "1");
                ENABLED.store(if on { 2 } else { 1 }, Ordering::Relaxed);
                on
            }
        }
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
            if !enabled() {
                return;
            }
            let left = COUNTDOWN.with(|c| {
                let left = c.get() - size as isize;
                c.set(if left <= 0 { RATE } else { left });
                left
            });
            if left > 0 {
                return;
            }
            let weight = size.max(RATE as usize);
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
            let mut guard = STATE.lock().unwrap();
            let state = guard.get_or_insert_with(|| State {
                live: FxHashMap::default(),
                stacks: Vec::new(),
                index: FxHashMap::default(),
            });
            let next = state.stacks.len() as u32;
            let id = *state.index.entry(stack).or_insert(next);
            if id == next {
                state.stacks.push((stack, 0, 0));
            }
            state.stacks[id as usize].1 += weight as i64;
            state.stacks[id as usize].2 += weight as u64;
            state.live.insert(p as usize, (id, weight));
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
                    state.stacks[id as usize].1 -= weight as i64;
                }
            }
        });
    }

    extern "C" {
        fn _dyld_get_image_header(index: u32) -> *const c_void;
    }

    pub(super) fn dump(top: usize) {
        ENABLED.store(1, Ordering::Relaxed);
        let Some(state) = STATE.lock().unwrap().take() else {
            return;
        };
        let state = &state;
        let mut order: Vec<usize> = (0..state.stacks.len()).collect();
        let print = |title: &str, order: &[usize], key: &dyn Fn(usize) -> i64| {
            let ips: Vec<usize> = order
                .iter()
                .take(top)
                .flat_map(|&i| state.stacks[i].0.iter().copied().filter(|&ip| ip > 1))
                .collect();
            let names = resolve(&ips);
            eprintln!("\n-- heap {title} (sampled, top {top}) --");
            for &i in order.iter().take(top) {
                let (stack, _, _) = &state.stacks[i];
                eprintln!("{:>10.1} MB", key(i) as f64 / (1024.0 * 1024.0));
                if stack[0] == 1 {
                    eprintln!("             <arena chunks>");
                    continue;
                }
                let mut shown = 0;
                for &ip in stack.iter().filter(|&&ip| ip != 0) {
                    let name = names.get(&ip).map(|s| s.as_str()).unwrap_or("?");
                    if ["hashbrown::", "indexmap::", "alloc::", "core::", "std::"].iter().any(|p| name.starts_with(p)) {
                        continue;
                    }
                    eprintln!("             {name}");
                    shown += 1;
                    if shown == 6 {
                        break;
                    }
                }
            }
        };
        order.sort_by_key(|&i| -state.stacks[i].1);
        print("live", &order, &|i| state.stacks[i].1);
        order.sort_by_key(|&i| -(state.stacks[i].2 as i64));
        print("allocated (cumulative)", &order, &|i| state.stacks[i].2 as i64);
    }

    fn resolve(ips: &[usize]) -> FxHashMap<usize, String> {
        let mut out = FxHashMap::default();
        let mut uniq: Vec<usize> = ips.to_vec();
        uniq.sort_unstable();
        uniq.dedup();
        let exe = std::env::current_exe().unwrap();
        // SAFETY: image 0 is the main executable.
        let load = unsafe { _dyld_get_image_header(0) } as usize;
        let mut cmd = std::process::Command::new("atos");
        cmd.arg("-o").arg(&exe).arg("-l").arg(format!("{load:#x}"));
        for ip in &uniq {
            cmd.arg(format!("{:#x}", ip - 1));
        }
        if let Ok(o) = cmd.output() {
            for (ip, line) in uniq.iter().zip(String::from_utf8_lossy(&o.stdout).lines()) {
                let line = line.replace(" (in tsrs)", "");
                // Shorten `func (in tsrs) (file.rs:12)` / generic noise.
                let short = if line.len() > 160 { format!("{}...", &line[..160]) } else { line };
                out.insert(*ip, short);
            }
        }
        out
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
    sites: FxHashMap<Key, (&'static Location<'static>, &'static str, Entry)>,
}
type Shared = Arc<Mutex<ThreadData>>;

static THREADS: Mutex<Vec<Shared>> = Mutex::new(Vec::new());
static ARENAS: Mutex<Vec<usize>> = Mutex::new(Vec::new());

thread_local! {
    static LOCAL: Shared = {
        let data = Arc::new(Mutex::new(ThreadData { sites: FxHashMap::default() }));
        THREADS.lock().unwrap().push(data.clone());
        data
    };
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

pub(crate) fn register_arena(arena: &'static bumpalo::Bump) {
    ARENAS.lock().unwrap().push(arena as *const bumpalo::Bump as usize);
}

#[inline(never)]
pub(crate) fn record(site: &'static Location<'static>, ty: &'static str, bytes: usize) {
    let _ = LOCAL.try_with(|local| {
        let mut data = local.lock().unwrap();
        let key = (site as *const Location as usize, ty.as_ptr() as usize ^ ty.len());
        let e = &mut data.sites.entry(key).or_insert((site, ty, Entry::default())).2;
        e.count += 1;
        e.bytes += bytes as u64;
    });
}

fn mb(b: u64) -> String {
    format!("{:.1}", b as f64 / (1024.0 * 1024.0))
}

fn short_type(ty: &str) -> String {
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
        for (k, v) in t.lock().unwrap().sites.iter() {
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
            let a = unsafe { &*(a as *const bumpalo::Bump) };
            a.allocated_bytes() as u64
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
