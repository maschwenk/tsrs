//! Reachability census (`TSRS_CENSUS=1` in the alloc-profile build): how much of the memory still held at the end of
//! a run is garbage, by arena type / call site and by heap allocating function.
//!
//! While recording, every arena block (address, size, call site + type) and every live Rust heap block (address,
//! size, raw stack) is kept; arena chunks themselves are not blocks. `run` freezes the tables and does a
//! conservative mark from the roots the caller passes plus the current thread's stack and the main image's
//! `__DATA*` segments (all statics, including `OnceLock` / `LazyLock` contents). Every 4-byte-aligned word of a
//! reachable block (except blocks of pointer-free arena types) is a candidate pointer, decoded two ways: the low 48
//! bits (plain pointers, low-bit tags, `PackedStr` and mapper slices with a length in the top 16 bits) and the low 45
//! bits times 8 (node parents, symbol table entries). Interior pointers count. A candidate that does not point
//! into a recorded block is ignored. Conservative: words that only look like pointers keep blocks alive, so the
//! unreachable numbers are lower bounds (freed memory is cleared while recording, see `zero_on_free`, so stale
//! words in reused memory do not add to that). Not scanned: other threads' stacks (idle pool threads), thread-locals
//! (none of ours holds program data) and registers other than what the stack holds.

use super::heap_sample::IN_ARENA;
use super::{short_type, THREADS};
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::panic::Location;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::Instant;

const DEPTH: usize = 16;
type Stack = [usize; DEPTH];

/// 0 unknown, 1 off, 2 recording, 3 frozen (after `run` started).
static MODE: AtomicU8 = AtomicU8::new(0);

const SHARDS: usize = 64;
type LiveMap = FxHashMap<usize, (usize, u32, u32)>;
static LIVE: [Mutex<Option<LiveMap>>; SHARDS] = [const { Mutex::new(None) }; SHARDS];

struct StackTable {
    index: FxHashMap<Stack, u32>,
    stacks: Vec<Stack>,
}
static STACKS: Mutex<Option<StackTable>> = Mutex::new(None);

thread_local! {
    static GUARD: Cell<bool> = const { Cell::new(false) };
    static CACHE: RefCell<Option<FxHashMap<Stack, u32>>> = const { RefCell::new(None) };
}

/// Runs `f` with heap tracking suspended on this thread: allocations made inside are not blocks (census bookkeeping).
pub(super) fn with_guard<R>(f: impl FnOnce() -> R) -> R {
    let prev = GUARD.try_with(|g| g.replace(true)).unwrap_or(true);
    let r = f();
    let _ = GUARD.try_with(|g| g.set(prev));
    r
}

fn unless_guarded(f: impl FnOnce()) {
    let _ = GUARD.try_with(|g| {
        if g.get() {
            return;
        }
        g.set(true);
        f();
        g.set(false);
    });
}

/// Call with the guard held (reading the environment allocates).
fn enabled() -> bool {
    match MODE.load(Ordering::Relaxed) {
        0 => {
            let on = std::env::var_os("TSRS_CENSUS").is_some_and(|v| v == "1");
            MODE.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
        m => m == 2,
    }
}

pub(crate) fn recording() -> bool {
    match MODE.load(Ordering::Relaxed) {
        0 => with_guard(enabled),
        m => m == 2,
    }
}

extern "C" {
    fn backtrace(buf: *mut *mut c_void, size: i32) -> i32;
}

fn capture() -> Stack {
    let mut raw = [std::ptr::null_mut::<c_void>(); DEPTH + 2];
    // SAFETY: `raw` has room for DEPTH + 2 frames.
    let n = unsafe { backtrace(raw.as_mut_ptr(), raw.len() as i32) } as usize;
    let mut stack: Stack = [0; DEPTH];
    for (i, f) in raw.iter().take(n).skip(2).enumerate() {
        stack[i] = *f as usize;
    }
    stack
}

fn intern_global(stack: &Stack) -> u32 {
    let mut guard = STACKS.lock().unwrap();
    let table = guard.get_or_insert_with(|| StackTable { index: FxHashMap::default(), stacks: Vec::new() });
    let next = table.stacks.len() as u32;
    let id = *table.index.entry(*stack).or_insert(next);
    if id == next {
        table.stacks.push(*stack);
    }
    id
}

fn intern(stack: &Stack) -> u32 {
    CACHE
        .try_with(|c| {
            let mut c = c.borrow_mut();
            let c = c.get_or_insert_with(FxHashMap::default);
            if let Some(&id) = c.get(stack) {
                return id;
            }
            let id = intern_global(stack);
            c.insert(*stack, id);
            id
        })
        .unwrap_or_else(|_| intern_global(stack))
}

#[inline]
fn shard(a: usize) -> usize {
    ((a >> 4) ^ (a >> 13)) % SHARDS
}

/// While recording, freed heap memory is cleared before it goes back to the allocator. Otherwise its stale contents
/// (pointers of dead objects, and the census's own address tables) would sit in the unused capacity of the blocks
/// that reuse the memory (`Vec` slack, empty hash table buckets) and keep arbitrary blocks "reachable".
#[inline]
pub(super) fn zero_on_free() -> bool {
    MODE.load(Ordering::Relaxed) == 2
}

#[inline]
pub(super) fn on_alloc(p: *mut u8, size: usize) {
    if MODE.load(Ordering::Relaxed) & 1 == 1 {
        return;
    }
    if IN_ARENA.try_with(|c| c.get()).unwrap_or(false) {
        return; // arena chunk: its blocks are recorded by `record`
    }
    unless_guarded(|| {
        if !enabled() {
            return;
        }
        let id = intern(&capture());
        let a = p as usize;
        LIVE[shard(a)].lock().unwrap().get_or_insert_with(FxHashMap::default).insert(a, (size, id, next_seq()));
    });
}

#[inline]
pub(super) fn on_free(p: *mut u8) {
    if MODE.load(Ordering::Relaxed) != 2 {
        return;
    }
    unless_guarded(|| {
        let a = p as usize;
        if let Some(m) = LIVE[shard(a)].lock().unwrap().as_mut() {
            m.remove(&a);
        }
    });
}

#[derive(Clone, Copy)]
struct Block {
    start: u64,
    size: u32,
    class: u32,
    /// Allocation order (`next_seq`).
    seq: u32,
}

static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A global event counter (/4, to fit `u32`): orders allocations and frees across threads for the would-free check.
pub(super) fn next_seq() -> u32 {
    (SEQ.fetch_add(1, Ordering::Relaxed) >> 2) as u32
}

enum Class {
    Arena { loc: &'static Location<'static>, ty: &'static str },
    Heap { stack: u32 },
}

const REGION_SHIFT: u32 = 30;
const PAGE_SHIFT: u32 = 12;
const PAGES: usize = 1 << (REGION_SHIFT - PAGE_SHIFT);
const MASK48: u64 = (1 << 48) - 1;
const MASK45: u64 = (1 << 45) - 1;

struct Table {
    blocks: Vec<Block>,
    /// Region (address >> 30) -> index into `pages`, or `u32::MAX`.
    regions: FxHashMap<u64, u32>,
    /// Per region, for each page (and one past the last): the first block starting at or after the page.
    pages: Vec<Box<[u32]>>,
    lo: u64,
    hi: u64,
    marks: Vec<u64>,
}

impl Table {
    fn build(mut blocks: Vec<Block>) -> Table {
        blocks.sort_unstable_by_key(|b| b.start);
        let lo = blocks.first().map_or(0, |b| b.start);
        let hi = blocks.iter().map(|b| b.start + b.size as u64).max().unwrap_or(0);
        let mut region_list: Vec<u64> = Vec::new();
        for b in &blocks {
            let (r0, r1) = (b.start >> REGION_SHIFT, (b.start + b.size as u64 - 1) >> REGION_SHIFT);
            for r in r0..=r1 {
                if region_list.last() != Some(&r) {
                    region_list.push(r);
                }
            }
        }
        region_list.sort_unstable();
        region_list.dedup();
        let mut regions = FxHashMap::default();
        let mut pages = Vec::with_capacity(region_list.len());
        for &r in &region_list {
            let base = r << REGION_SHIFT;
            let mut i = blocks.partition_point(|b| b.start < base);
            let mut lb = vec![0u32; PAGES + 1].into_boxed_slice();
            for (p, slot) in lb.iter_mut().enumerate() {
                let addr = base + ((p as u64) << PAGE_SHIFT);
                while i < blocks.len() && blocks[i].start < addr {
                    i += 1;
                }
                *slot = i as u32;
            }
            regions.insert(r, pages.len() as u32);
            pages.push(lb);
        }
        let marks = vec![0u64; blocks.len() / 64 + 1];
        Table { blocks, regions, pages, lo, hi, marks }
    }

    #[inline]
    fn lookup(&self, a: u64) -> Option<usize> {
        if a < self.lo || a >= self.hi {
            return None;
        }
        let slot = *self.regions.get(&(a >> REGION_SHIFT))?;
        let lb = &self.pages[slot as usize];
        let p = ((a >> PAGE_SHIFT) as usize) & (PAGES - 1);
        let (lo, hi) = ((lb[p] as usize).saturating_sub(1), lb[p + 1] as usize);
        let range = &self.blocks[lo..hi];
        let k = range.partition_point(|b| b.start <= a);
        if k == 0 {
            return None;
        }
        let i = lo + k - 1;
        let b = &self.blocks[i];
        (a < b.start + b.size as u64).then_some(i)
    }

    #[inline]
    fn marked(&self, i: usize) -> bool {
        self.marks[i / 64] & (1 << (i % 64)) != 0
    }

    #[inline]
    fn mark_word(&mut self, w: u64, work: &mut Vec<u32>) {
        let c1 = w & MASK48;
        if let Some(i) = self.lookup(c1) {
            self.mark(i, work);
        }
        let c2 = (w & MASK45) << 3;
        if c2 != c1 {
            if let Some(i) = self.lookup(c2) {
                self.mark(i, work);
            }
        }
    }

    #[inline]
    fn mark(&mut self, i: usize, work: &mut Vec<u32>) {
        let (word, bit) = (i / 64, 1u64 << (i % 64));
        if self.marks[word] & bit == 0 {
            self.marks[word] |= bit;
            work.push(i as u32);
        }
    }

    /// Scans `[from, from + len)` for candidate pointers at every 4-byte offset.
    fn scan(&mut self, from: usize, len: usize, work: &mut Vec<u32>) {
        if len < 8 {
            return;
        }
        let last = from + len - 8;
        let mut p = from;
        while p <= last {
            // SAFETY: `p .. p + 8` lies inside a live block or a mapped root range; the bytes are only inspected.
            let w = unsafe { std::ptr::read_volatile(p as *const [u8; 8]) };
            let w = u64::from_ne_bytes(w);
            if w != 0 {
                self.mark_word(w, work);
            }
            p += 4;
        }
    }
}

static RESULT: Mutex<Option<Table>> = Mutex::new(None);
/// Used ranges of freed regions (`census_would_free_range`).
pub(super) static REGION_FREES: Mutex<Vec<(u64, u64, u32)>> = Mutex::new(Vec::new());
static WOULD_FREE: Mutex<Vec<(u64, u32)>> = Mutex::new(Vec::new());

/// After `run`: whether `addr` lies in a block the arena freed or rewound (see `check_would_free`).
pub fn is_would_free(addr: usize) -> bool {
    with_guard(|| {
        let w = WOULD_FREE.lock().unwrap();
        let i = w.partition_point(|&(a, _)| a <= addr as u64);
        i > 0 && (addr as u64) < w[i - 1].0 + w[i - 1].1 as u64
    })
}

/// After `run`: whether `addr` points into a reachable block (`None`: not inside any recorded block).
pub fn is_reachable(addr: usize) -> Option<bool> {
    with_guard(|| {
        let guard = RESULT.lock().unwrap();
        let t = guard.as_ref()?;
        t.lookup(addr as u64).map(|i| t.marked(i))
    })
}

/// One arena allocation in this many records its raw stack (`TSRS_CENSUS_ARENA_SAMPLE`, default 16): arena call
/// sites are one level deep (`#[track_caller]`), so allocations through wrappers (`new_node`, `Type::alloc`, link
/// stores, ...) are attributed to the code that asked through the sampled stacks.
pub(super) fn arena_sample_rate() -> u32 {
    static RATE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *RATE.get_or_init(|| {
        with_guard(|| std::env::var("TSRS_CENSUS_ARENA_SAMPLE").ok().and_then(|s| s.parse().ok()).unwrap_or(16u32).max(1))
    })
}

/// The interned raw stack of the caller (call with the guard held).
pub(super) fn stack_id() -> u32 {
    intern(&capture())
}

/// Whether the census is on (`TSRS_CENSUS=1`) and has not run yet.
pub fn active() -> bool {
    recording()
}

/// Freezes the tables, marks everything reachable from `roots` (addresses; each marks the block it points into),
/// the current thread's stack and the main image's data segments, and prints the census to stderr.
#[inline(never)]
pub fn run(roots: &[usize]) {
    if !recording() {
        return;
    }
    MODE.store(3, Ordering::SeqCst);
    with_guard(|| run_frozen(roots));
}

#[repr(C)]
struct MachHeader64 {
    magic: u32,
    cputype: i32,
    cpusubtype: i32,
    filetype: u32,
    ncmds: u32,
    sizeofcmds: u32,
    flags: u32,
    reserved: u32,
}

#[repr(C)]
struct SegmentCommand64 {
    cmd: u32,
    cmdsize: u32,
    segname: [u8; 16],
    vmaddr: u64,
    vmsize: u64,
}

extern "C" {
    fn _dyld_get_image_header(index: u32) -> *const c_void;
    fn _dyld_get_image_vmaddr_slide(index: u32) -> isize;
    fn pthread_self() -> *mut c_void;
    fn pthread_get_stackaddr_np(thread: *mut c_void) -> *mut c_void;
}

/// The main executable's `__DATA`, `__DATA_CONST` and `__DATA_DIRTY` segments (statics).
fn data_segments() -> Vec<(usize, usize, String)> {
    const LC_SEGMENT_64: u32 = 0x19;
    let mut out = Vec::new();
    // SAFETY: image 0 is the main executable; its load commands follow the header and are mapped.
    unsafe {
        let header = _dyld_get_image_header(0) as *const MachHeader64;
        let slide = _dyld_get_image_vmaddr_slide(0);
        let mut p = (header as *const u8).add(std::mem::size_of::<MachHeader64>());
        for _ in 0..(*header).ncmds {
            let cmd = &*(p as *const SegmentCommand64);
            if cmd.cmd == LC_SEGMENT_64 && cmd.segname.starts_with(b"__DATA") {
                let name = String::from_utf8_lossy(&cmd.segname).trim_end_matches('\0').to_string();
                out.push(((cmd.vmaddr as isize + slide) as usize, cmd.vmsize as usize, name));
            }
            p = p.add(cmd.cmdsize as usize);
        }
    }
    out
}

#[inline(never)]
fn scan_stack(table: &mut Table, work: &mut Vec<u32>) -> usize {
    let marker = 0u64;
    let low = std::hint::black_box(&marker) as *const u64 as usize & !7;
    // SAFETY: plain libc queries about the current thread.
    let high = unsafe { pthread_get_stackaddr_np(pthread_self()) } as usize;
    table.scan(low, high - low, work);
    high - low
}

/// Arena element types without pointers: not scanned.
fn pointer_free(ty: &str) -> bool {
    matches!(ty, "str" | "tsrs_core::ptr::PackedStr" | "[u8]" | "[u16]" | "[u32]" | "[i32]" | "[f64]")
}

#[derive(Default, Clone, Copy)]
struct Agg {
    count: u64,
    bytes: u64,
    rcount: u64,
    rbytes: u64,
}

impl Agg {
    fn add(&mut self, o: &Agg) {
        self.count += o.count;
        self.bytes += o.bytes;
        self.rcount += o.rcount;
        self.rbytes += o.rbytes;
    }
    fn ubytes(&self) -> u64 {
        self.bytes - self.rbytes
    }
}

fn mb(b: u64) -> String {
    format!("{:.1}", b as f64 / (1024.0 * 1024.0))
}

/// `TSRS_CENSUS_TSV=<path>`: every row of every table, tab-separated (table, bytes, count, reachable bytes and count,
/// row name), for offline grouping.
static TSV: Mutex<Option<std::io::BufWriter<std::fs::File>>> = Mutex::new(None);

fn print_table(title: &str, rows: &mut Vec<(String, Agg)>, top: usize) {
    rows.sort_by(|a, b| b.1.ubytes().cmp(&a.1.ubytes()).then(b.1.bytes.cmp(&a.1.bytes)));
    if let Some(out) = TSV.lock().unwrap().as_mut() {
        use std::io::Write;
        for (name, a) in rows.iter() {
            let _ = writeln!(out, "{title}\t{}\t{}\t{}\t{}\t{name}", a.bytes, a.count, a.rbytes, a.rcount);
        }
    }
    eprintln!("\n-- census: {title} (top {top} by unreachable bytes; {} rows) --", rows.len());
    eprintln!(
        "{:>10} {:>11} {:>10} {:>11} {:>10} {:>11} {:>6}  what",
        "alloc MB", "count", "reach MB", "count", "unreach MB", "count", "un%"
    );
    for (name, a) in rows.iter().take(top) {
        eprintln!(
            "{:>10} {:>11} {:>10} {:>11} {:>10} {:>11} {:>6.1}  {}",
            mb(a.bytes),
            a.count,
            mb(a.rbytes),
            a.rcount,
            mb(a.ubytes()),
            a.count - a.rcount,
            a.ubytes() as f64 * 100.0 / a.bytes.max(1) as f64,
            name
        );
    }
}

fn run_frozen(roots: &[usize]) {
    let top: usize = std::env::var("TSRS_CENSUS_TOP").ok().and_then(|s| s.parse().ok()).unwrap_or(40);
    if let Some(path) = std::env::var_os("TSRS_CENSUS_TSV") {
        *TSV.lock().unwrap() = std::fs::File::create(path).ok().map(std::io::BufWriter::new);
    }
    let t0 = Instant::now();
    let mut classes: Vec<Class> = Vec::new();
    let mut class_index: FxHashMap<(usize, usize), u32> = FxHashMap::default();
    let mut blocks: Vec<Block> = Vec::new();
    let mut samples: Vec<(u64, u32)> = Vec::new();
    let mut would_free: Vec<(u64, u32, u32)> = Vec::new();
    for t in THREADS.lock().unwrap().iter() {
        let mut data = t.lock().unwrap();
        let remap: Vec<u32> = data
            .sites
            .iter()
            .map(|&(loc, ty, _)| {
                let key = (loc as *const Location as usize, ty.as_ptr() as usize ^ ty.len());
                let next = classes.len() as u32;
                let id = *class_index.entry(key).or_insert(next);
                if id == next {
                    classes.push(Class::Arena { loc, ty });
                }
                id
            })
            .collect();
        let own = std::mem::take(&mut data.blocks);
        blocks.reserve(own.len());
        blocks.extend(own.iter().map(|&(start, size, site, seq)| Block { start, size, class: remap[site as usize], seq }));
        drop(own);
        samples.extend(std::mem::take(&mut data.samples));
        would_free.extend(std::mem::take(&mut data.would_free));
    }
    let arena_classes = classes.len() as u32;
    let mut ranges = std::mem::take(&mut *REGION_FREES.lock().unwrap());
    ranges.sort_unstable();
    let region_blocks_before = would_free.len();
    if !ranges.is_empty() {
        for b in &blocks {
            let i = ranges.partition_point(|&(s, _, _)| s <= b.start);
            if i > 0 && b.start < ranges[i - 1].0 + ranges[i - 1].1 {
                would_free.push((b.start, b.size, ranges[i - 1].2));
            }
        }
    }
    eprintln!("census: {} freed regions' ranges, {} arena blocks in them", ranges.len(), would_free.len() - region_blocks_before);
    let stacks = STACKS.lock().unwrap().take().map_or_else(Vec::new, |t| t.stacks);
    classes.extend((0..stacks.len() as u32).map(|stack| Class::Heap { stack }));
    let mut oversized = 0u64;
    for shard in LIVE.iter() {
        if let Some(map) = shard.lock().unwrap().take() {
            blocks.reserve(map.len());
            for (a, (size, stack, seq)) in map {
                if size == 0 {
                    continue;
                }
                let size = u32::try_from(size).unwrap_or_else(|_| {
                    oversized += 1;
                    u32::MAX
                });
                blocks.push(Block { start: a as u64, size, class: arena_classes + stack, seq });
            }
        }
    }
    let scan: Vec<bool> = classes
        .iter()
        .map(|c| match c {
            Class::Arena { ty, .. } => !pointer_free(ty),
            Class::Heap { .. } => true,
        })
        .collect();
    let t_collect = t0.elapsed();
    let mut table = Table::build(blocks);
    let mut overlaps = 0u64;
    for w in table.blocks.windows(2) {
        if w[1].start < w[0].start + w[0].size as u64 {
            overlaps += 1;
        }
    }
    let t_build = t0.elapsed();

    let mut work: Vec<u32> = Vec::new();
    let mut root_hits = 0usize;
    for &r in roots {
        if let Some(i) = table.lookup(r as u64) {
            table.mark(i, &mut work);
            root_hits += 1;
        }
    }
    let stack_bytes = scan_stack(&mut table, &mut work);
    let segments = data_segments();
    for &(start, len, _) in &segments {
        table.scan(start, len, &mut work);
    }
    let from_roots = work.len();
    while let Some(i) = work.pop() {
        let b = table.blocks[i as usize];
        if scan[b.class as usize] {
            table.scan(b.start as usize, b.size as usize, &mut work);
        }
    }
    let t_mark = t0.elapsed();
    check_would_free(&table, &classes, &stacks, &scan, roots, would_free);

    let mut per_class = vec![Agg::default(); classes.len()];
    for (i, b) in table.blocks.iter().enumerate() {
        let a = &mut per_class[b.class as usize];
        a.count += 1;
        a.bytes += b.size as u64;
        if table.marked(i) {
            a.rcount += 1;
            a.rbytes += b.size as u64;
        }
    }
    let (mut arena, mut heap) = (Agg::default(), Agg::default());
    for (c, a) in classes.iter().zip(&per_class) {
        match c {
            Class::Arena { .. } => arena.add(a),
            Class::Heap { .. } => heap.add(a),
        }
    }

    eprintln!("\n== census (TSRS_CENSUS=1): conservative mark; unreachable numbers are lower bounds ==");
    eprintln!(
        "blocks: {} ({} arena sites, {} stacks); overlaps {overlaps}, heap blocks >= 4 GiB {oversized}",
        table.blocks.len(),
        arena_classes,
        stacks.len()
    );
    eprintln!(
        "roots: {}/{} explicit, stack {} KB, data segments {} ({} KB); {} blocks marked directly",
        root_hits,
        roots.len(),
        stack_bytes / 1024,
        segments.iter().map(|s| s.2.as_str()).collect::<Vec<_>>().join(","),
        segments.iter().map(|s| s.1).sum::<usize>() / 1024,
        from_roots
    );
    for (name, a) in [("arena", &arena), ("heap (live blocks)", &heap)] {
        eprintln!(
            "{name:<20} allocated {:>9} MB {:>11} blocks | reachable {:>9} MB {:>11} | unreachable {:>9} MB {:>11} ({:.1}%)",
            mb(a.bytes),
            a.count,
            mb(a.rbytes),
            a.rcount,
            mb(a.ubytes()),
            a.count - a.rcount,
            a.ubytes() as f64 * 100.0 / a.bytes.max(1) as f64
        );
    }

    let mut by_type: FxHashMap<String, Agg> = FxHashMap::default();
    let mut by_site: Vec<(String, Agg)> = Vec::new();
    for (c, a) in classes.iter().zip(&per_class) {
        if let Class::Arena { loc, ty } = c {
            if a.count == 0 {
                continue;
            }
            let ty = short_type(ty);
            by_type.entry(ty.clone()).or_default().add(a);
            let file = loc.file();
            let file = file.find("crates/").map(|i| &file[i + 7..]).unwrap_or(file);
            by_site.push((format!("{}:{}  {}", file, loc.line(), ty), *a));
        }
    }
    print_table("arena by type", &mut by_type.into_iter().collect(), top);
    print_table("arena by call site (one level, #[track_caller])", &mut by_site, top);

    let t_resolve = Instant::now();
    // Arena samples: (type, stack) -> scaled aggregate.
    let rate = arena_sample_rate() as u64;
    let mut sampled: FxHashMap<(u32, u32), Agg> = FxHashMap::default();
    for &(addr, stack) in &samples {
        let Some(i) = table.lookup(addr) else { continue };
        let b = table.blocks[i];
        let a = sampled.entry((b.class, stack)).or_default();
        a.count += rate;
        a.bytes += b.size as u64 * rate;
        if table.marked(i) {
            a.rcount += rate;
            a.rbytes += b.size as u64 * rate;
        }
    }
    let heap_aggs: Vec<(u32, Agg)> = classes
        .iter()
        .zip(&per_class)
        .filter_map(|(c, a)| match c {
            Class::Heap { stack } if a.count > 0 => Some((*stack, *a)),
            _ => None,
        })
        .collect();
    let mut ips: Vec<usize> = Vec::new();
    for &(stack, _) in &heap_aggs {
        ips.extend(stacks[stack as usize].iter().copied());
    }
    for &(_, stack) in sampled.keys() {
        ips.extend(stacks[stack as usize].iter().copied());
    }
    let mut names: FxHashMap<usize, String> = FxHashMap::default();
    atos(&ips, &mut names);
    let frames = |stack: u32, skip: &dyn Fn(&str) -> bool, k: usize| -> Vec<String> {
        stacks[stack as usize]
            .iter()
            .take_while(|&&ip| ip != 0)
            .filter_map(|ip| names.get(ip))
            .filter(|n| !skip(n))
            .take(k)
            .cloned()
            .collect()
    };

    let mut by_fn: FxHashMap<String, Agg> = FxHashMap::default();
    let mut by_fn_caller: FxHashMap<String, Agg> = FxHashMap::default();
    for &(stack, a) in &heap_aggs {
        let f = frames(stack, &boring, 2);
        let site = f.first().cloned().unwrap_or_else(|| "?".into());
        by_fn.entry(site.clone()).or_default().add(&a);
        by_fn_caller.entry(f.join("  <-  ")).or_default().add(&a);
    }
    print_table("heap by allocating function", &mut by_fn.into_iter().collect(), top);
    print_table("heap by allocating function <- caller", &mut by_fn_caller.into_iter().collect(), top);

    let mut arena_fn: FxHashMap<String, Agg> = FxHashMap::default();
    let mut arena_fn_caller: FxHashMap<String, Agg> = FxHashMap::default();
    for (&(class, stack), a) in &sampled {
        let Class::Arena { ty, .. } = classes[class as usize] else { continue };
        let ty = short_type(ty);
        let f = frames(stack, &arena_wrapper, 3);
        let site = f.first().cloned().unwrap_or_else(|| "?".into());
        arena_fn.entry(format!("{ty}  {site}")).or_default().add(a);
        arena_fn_caller.entry(format!("{ty}  {}", f.join("  <-  "))).or_default().add(a);
    }
    let title = format!("arena by type and allocating function (sampled 1/{rate}, scaled)");
    print_table(&title, &mut arena_fn.into_iter().collect(), top);
    let title = format!("arena by type and allocating function <- callers (sampled 1/{rate}, scaled)");
    print_table(&title, &mut arena_fn_caller.into_iter().collect(), top);
    eprintln!(
        "\ncensus time: collect {:.1} s, sort+index {:.1} s, mark {:.1} s, resolve {:.1} s",
        t_collect.as_secs_f64(),
        (t_build - t_collect).as_secs_f64(),
        (t_mark - t_build).as_secs_f64(),
        t_resolve.elapsed().as_secs_f64()
    );
    if let Some(mut out) = TSV.lock().unwrap().take() {
        use std::io::Write;
        let _ = out.flush();
    }
    *RESULT.lock().unwrap() = Some(table);
}

/// Field layouts the crates that own arena types registered (`tsrs_core::census_layout`): by full type name, or by
/// a prefix ending in `<` for every instance of a generic.
static LAYOUTS: Mutex<Vec<(&'static str, Vec<crate::CensusField>)>> = Mutex::new(Vec::new());

pub fn register_layout(type_name: &'static str, fields: &[crate::CensusField]) {
    with_guard(|| LAYOUTS.lock().unwrap().push((type_name, fields.to_vec())));
}

/// How the strong mark reads the words of one arena class (from the registered layouts).
#[derive(Default)]
struct ClassLayout {
    /// Scan offsets (4-byte steps) whose 8 bytes overlap a `NoPointer` range, or straddle a `Tagged` / `X8` word.
    skip: Vec<u32>,
    tagged: Vec<u32>,
    x8: Vec<(u32, u8)>,
    /// (pointer offset, length offset)
    slices: Vec<(u32, u32)>,
}

fn class_layouts(classes: &[Class]) -> Vec<Option<ClassLayout>> {
    let layouts = LAYOUTS.lock().unwrap();
    classes
        .iter()
        .map(|c| {
            let Class::Arena { ty, .. } = c else { return None };
            let mut l = ClassLayout::default();
            let mut any = false;
            for (name, fields) in layouts.iter() {
                let applies = if name.ends_with('<') { ty.starts_with(name) } else { ty == name };
                if !applies {
                    continue;
                }
                any = true;
                for f in fields {
                    match *f {
                        crate::CensusField::NoPointer { off, len } => {
                            // Every 4-byte step `o` with [o, o + 8) overlapping [off, off + len).
                            let first = (off + 1).saturating_sub(8).div_ceil(4) * 4;
                            l.skip.extend((first..off + len).step_by(4).map(|o| o as u32));
                        }
                        crate::CensusField::Tagged { off } => {
                            l.tagged.push(off as u32);
                            l.skip.extend([off.wrapping_sub(4) as u32, off as u32 + 4]);
                        }
                        crate::CensusField::X8 { off, modes } => {
                            l.x8.push((off as u32, modes));
                            l.skip.extend([off.wrapping_sub(4) as u32, off as u32 + 4]);
                        }
                        crate::CensusField::Slice { ptr, len } => l.slices.push((ptr as u32, len as u32)),
                    }
                }
            }
            any.then(|| {
                l.skip.sort_unstable();
                l.skip.dedup();
                l
            })
        })
        .collect()
}

fn class_name(c: &Class) -> String {
    match c {
        Class::Arena { loc, ty } => {
            let file = loc.file();
            let file = file.find("crates/").map(|i| &file[i + 7..]).unwrap_or(file);
            format!("{}:{}  {}", file, loc.line(), short_type(ty))
        }
        Class::Heap { stack } => format!("heap block (stack {stack})"),
    }
}

/// The census gate for arena recycling: every block the arena freed or rewound (recorded, never reused, in this
/// mode) must be unreachable. A reachable one means the escape analysis that freed it is wrong; up to ten examples
/// are traced to the blocks (or roots) that point to them. `TSRS_CENSUS_ASSERT=1` exits with status 3 then.
fn check_would_free(table: &Table, classes: &[Class], stacks: &[Stack], scan: &[bool], roots: &[usize], mut would_free: Vec<(u64, u32, u32)>) {
    would_free.sort_unstable();
    would_free.dedup_by_key(|w| w.0);
    *WOULD_FREE.lock().unwrap() = would_free.iter().map(|&(a, size, _)| (a, size)).collect();
    let (mut checked, mut bytes, mut missing) = (0u64, 0u64, 0u64);
    let mut violations: FxHashMap<u32, (u64, u64)> = FxHashMap::default();
    let mut by_class: FxHashMap<u32, (u64, u64)> = FxHashMap::default();
    for &(addr, size, _) in &would_free {
        match table.lookup(addr) {
            Some(i) if table.blocks[i].start == addr => {
                checked += 1;
                bytes += size as u64;
                let b = table.blocks[i];
                let e = by_class.entry(b.class).or_default();
                e.0 += 1;
                e.1 += b.size as u64;
                if table.marked(i) {
                    let v = violations.entry(b.class).or_default();
                    v.0 += 1;
                    v.1 += b.size as u64;
                }
            }
            _ => missing += 1,
        }
    }
    let reachable: u64 = violations.values().map(|v| v.0).sum();
    eprintln!(
        "\n== census would-free check: {checked} blocks ({}) freed or rewound, {reachable} conservatively reachable, {missing} not recorded ==",
        mb(bytes) + " MB"
    );
    let mut rows: Vec<(u32, (u64, u64))> = by_class.into_iter().collect();
    rows.sort_by(|a, b| b.1 .1.cmp(&a.1 .1));
    for (class, (n, b)) in rows.iter().take(25) {
        eprintln!("  would-free {:>10} blocks {:>9} MB  {}", n, mb(*b), class_name(&classes[*class as usize]));
    }
    let mut rows: Vec<(u32, (u64, u64))> = violations.into_iter().collect();
    rows.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
    for (class, (n, b)) in rows.iter().take(25) {
        eprintln!("  conservatively reachable {:>10} blocks {:>9} MB  {}", n, mb(*b), class_name(&classes[*class as usize]));
    }
    // The conservative mark above counts every word that looks like a pointer, and freed blocks are exactly the
    // objects whose addresses linger in dead stack slots, struct padding and pooled vectors. So reachability is
    // recomputed with only the references the program actually stores ("strong" edges): a plain 48-bit pointer to
    // the start of a block (mappers and inference contexts may carry tag bits; mapper slice words keep a length in
    // the top 16 bits), read as the registered layout of the referrer's type says (`register_layout`: padding, scalar
    // and header words are skipped, tagged and x8-encoded fields decoded, empty slices ignored), x8-encoded words
    // otherwise only for symbol table entries in heap blocks. An edge into a freed block must also come from a block
    // allocated before the free (later blocks can only hold stale copies) and not be 64 KiB-aligned (a stale pointer
    // whose low bytes a small field overwrote). A freed block reached this way is a violation.
    let wf_seq: FxHashMap<usize, u32> = would_free
        .iter()
        .filter_map(|&(a, _, seq)| table.lookup(a).filter(|&i| table.blocks[i].start == a).map(|i| (i, seq)))
        .collect();
    let class_is = |c: u32, suffix: &str| matches!(&classes[c as usize], Class::Arena { ty, .. } if ty.ends_with(suffix));
    let is_heap = |c: u32| matches!(&classes[c as usize], Class::Heap { .. });
    let n = table.blocks.len();
    let mut smark = vec![0u64; n / 64 + 1];
    // Who first reached each block strongly (u32::MAX: a root), to print violation chains.
    let mut via: Vec<u32> = vec![u32::MAX; n];
    let mut strong: FxHashMap<usize, String> = FxHashMap::default();
    let mut work: Vec<u32> = Vec::new();
    // SAFETY (all reads): inside a recorded live block or a mapped root range; the bytes are only inspected.
    let read = |p: usize| u64::from_ne_bytes(unsafe { std::ptr::read_volatile(p as *const [u8; 8]) });
    let layouts = class_layouts(classes);
    eprintln!(
        "  registered layouts: {} entries, {} arena classes covered",
        LAYOUTS.lock().unwrap().len(),
        layouts.iter().filter(|l| l.is_some()).count()
    );
    // Decides whether `w` (at `off` in block `from`, or in a root) is a strong edge; returns the target.
    let edge = |from: Option<usize>, off: usize, w: u64| -> Option<usize> {
        let mut c = w & MASK48;
        // The word is a registered tagged / x8 field, whose high bits are flags.
        let mut flagged = false;
        let (packed, tags, born, heap) = match from {
            Some(j) => {
                let rb = table.blocks[j];
                if let Some(l) = &layouts[rb.class as usize] {
                    let o = off as u32;
                    if l.skip.binary_search(&o).is_ok() {
                        return None;
                    }
                    if let Some(&(_, len)) = l.slices.iter().find(|s| s.0 == o) {
                        if len as usize + 8 <= rb.size as usize && read(rb.start as usize + len as usize) == 0 {
                            return None;
                        }
                    }
                    if l.tagged.contains(&o) {
                        flagged = true;
                    }
                    if let Some(&(_, modes)) = l.x8.iter().find(|x| x.0 == o) {
                        if modes & (1 << (w >> 62)) == 0 {
                            return None;
                        }
                        c = (w & MASK45) << 3;
                        flagged = true;
                    }
                }
                (
                    class_is(rb.class, "TypeMapper"),
                    class_is(rb.class, "TypeMapper") || class_is(rb.class, "InferenceContext"),
                    Some(rb.seq),
                    is_heap(rb.class),
                )
            }
            None => (false, false, None, false),
        };
        // An `Option<Vec<_>>` / `Option<String>` that is `None` keeps the capacity niche (2^63, or 2^63 + k for
        // nested options) in its first word, and its pointer and length words are uninitialized bytes (copied from
        // the stack): not references.
        if let Some(j) = from {
            let rb = table.blocks[j];
            let niche = |o: usize| o >= 8 && o <= rb.size as usize && (read(rb.start as usize + o - 8) >> 8) == 0x0080_0000_0000_0000;
            if off % 8 == 0 && (niche(off) || (off >= 8 && niche(off - 8))) {
                return None;
            }
        }
        if let Some(i) = table.lookup(c) {
            let b = table.blocks[i];
            let off_t = c - b.start;
            // Interior pointers: hash tables point at their control bytes (heap blocks), sub-slices into arena
            // lists (8-byte elements).
            let slice = matches!(&classes[b.class as usize], Class::Arena { ty, .. } if ty.starts_with('['));
            let aimed = off_t == 0
                || (tags && off_t < 8 && (class_is(b.class, "TypeMapper") || class_is(b.class, "InferenceContext") || class_is(b.class, "InferenceContextRare")))
                || is_heap(b.class)
                || (slice && off_t % 8 == 0);
            if aimed && (w >> 48 == 0 || packed || flagged) {
                match wf_seq.get(&i) {
                    None => return Some(i),
                    Some(&freed) if c & 0xffff != 0 && born.is_none_or(|b| b <= freed) => return Some(i),
                    _ => {}
                }
            }
        }
        if heap {
            // Symbol table entries (`SymbolMapEntry`: address >> 3 in the low 45 bits) live in a `Vec` buffer whose
            // first word is an entry too. Other heap blocks hold no entries; a random 64-bit word there (a hash in a
            // cache keyed by hashes) decodes to a symbol's address once in ~10^7 words, so it is not an edge.
            let is_symbol_entry = |w: u64| {
                let c = (w & MASK45) << 3;
                table.lookup(c).filter(|&i| table.blocks[i].start == c && class_is(table.blocks[i].class, "Symbol"))
            };
            if let Some(i) = is_symbol_entry(w) {
                if is_symbol_entry(read(table.blocks[from.unwrap()].start as usize)).is_some() {
                    return Some(i);
                }
            }
        }
        None
    };
    let mut visit = |i: usize, from: Option<usize>, why: &dyn Fn() -> String, work: &mut Vec<u32>, smark: &mut Vec<u64>, via: &mut Vec<u32>| {
        let (word, bit) = (i / 64, 1u64 << (i % 64));
        if smark[word] & bit == 0 {
            smark[word] |= bit;
            via[i] = from.map_or(u32::MAX, |j| j as u32);
            if wf_seq.contains_key(&i) {
                strong.insert(i, why());
            } else {
                work.push(i as u32);
            }
        }
    };
    for &r in roots {
        if let Some(i) = table.lookup(r as u64) {
            visit(i, None, &|| "explicit root".into(), &mut work, &mut smark, &mut via);
        }
    }
    let marker = 0u64;
    let low = std::hint::black_box(&marker) as *const u64 as usize & !7;
    // SAFETY: plain libc queries about the current thread.
    let high = unsafe { pthread_get_stackaddr_np(pthread_self()) } as usize;
    let mut roots_ranges: Vec<(usize, usize, String)> = data_segments();
    roots_ranges.push((low, high - low, "stack".into()));
    for (start, len, name) in &roots_ranges {
        let mut p = *start;
        while p + 8 <= start + len {
            let w = read(p);
            if w != 0 {
                if let Some(i) = edge(None, 0, w) {
                    let off = p - start;
                    visit(i, None, &|| format!("root {name} +{off:#x} [{w:#018x}]"), &mut work, &mut smark, &mut via);
                }
            }
            p += 4;
        }
    }
    while let Some(j) = work.pop() {
        let j = j as usize;
        let b = table.blocks[j];
        if !scan[b.class as usize] || b.size < 8 {
            continue;
        }
        let mut p = b.start as usize;
        let end = b.start as usize + b.size as usize;
        while p + 8 <= end {
            let w = read(p);
            if w != 0 {
                let off = p - b.start as usize;
                if let Some(i) = edge(Some(j), off, w) {
                    visit(
                        i,
                        Some(j),
                        &|| {
                            let around = |o: isize| -> String {
                                let at = off as isize + o;
                                if at < 0 || at as usize + 8 > b.size as usize {
                                    return "-".into();
                                }
                                format!("{:#x}", read(b.start as usize + at as usize))
                            };
                            format!("{} +{off} (words before/after: {} {}) [{w:#018x}]", class_name(&classes[b.class as usize]), around(-8), around(8))
                        },
                        &mut work,
                        &mut smark,
                        &mut via,
                    );
                }
            }
            p += 4;
        }
    }
    // Heap referrers are named only now. `Rc<LazyMemberTable>` blocks hold an empty `OnceCell<LazyMembers>` until
    // the table is prepared, whose payload bytes are uninitialized (copied from the stack): weak. (The table's own
    // mapper and type list are never recycled.)
    let mut ips: Vec<usize> = Vec::new();
    let heap_stack = |r: &str| -> Option<usize> { r.strip_prefix("heap block (stack ")?.split(')').next()?.parse().ok() };
    for r in strong.values() {
        if let Some(k) = heap_stack(r) {
            ips.extend(stacks[k].iter().copied());
        }
    }
    // Also name the heap blocks on the referrer chains of the reported violations.
    let heap_class_stack = |c: u32| -> Option<usize> {
        match &classes[c as usize] {
            Class::Heap { stack } => Some(*stack as usize),
            _ => None,
        }
    };
    // `TSRS_CENSUS_CHAINS=N`: how many violations are printed with their referrer chain (default 20).
    let chains: usize = std::env::var("TSRS_CENSUS_CHAINS").ok().and_then(|s| s.parse().ok()).unwrap_or(20);
    for &i in strong.keys().take(chains) {
        let mut k = via[i];
        let mut n = 0;
        while k != u32::MAX && n < 12 {
            if let Some(st) = heap_class_stack(table.blocks[k as usize].class) {
                ips.extend(stacks[st].iter().copied());
            }
            k = via[k as usize];
            n += 1;
        }
    }
    let mut names: FxHashMap<usize, String> = FxHashMap::default();
    atos(&ips, &mut names);
    let heap_frames = |r: &str| -> Option<Vec<String>> {
        let k = heap_stack(r)?;
        Some(stacks[k].iter().take_while(|&&ip| ip != 0).filter_map(|ip| names.get(ip)).filter(|n| !boring(n)).take(3).cloned().collect())
    };
    strong.retain(|_, r| !heap_frames(r).is_some_and(|f| f.first().is_some_and(|f| f.ends_with("get_ready_lazy_member_table_worker"))));
    let (mut conservative, mut strongly, mut cbytes, mut sbytes) = (0u64, 0u64, 0u64, 0u64);
    let mut only_weak: FxHashMap<u32, (u64, u64)> = FxHashMap::default();
    for (i, b) in table.blocks.iter().enumerate() {
        if table.marked(i) {
            conservative += 1;
            cbytes += b.size as u64;
            if smark[i / 64] & (1 << (i % 64)) != 0 {
                strongly += 1;
                sbytes += b.size as u64;
            } else if !wf_seq.contains_key(&i) {
                let e = only_weak.entry(b.class).or_default();
                e.0 += 1;
                e.1 += b.size as u64;
            }
        }
    }
    let mut rows: Vec<(u32, (u64, u64))> = only_weak.into_iter().collect();
    rows.sort_by(|a, b| b.1 .1.cmp(&a.1 .1));
    for (class, (n, b)) in rows.iter().take(12) {
        eprintln!("  only conservatively reachable {:>10} blocks {:>9} MB  {}", n, mb(*b), class_name(&classes[*class as usize]));
    }
    eprintln!(
        "  strong mark: {strongly} of {conservative} conservatively reachable blocks ({} of {} MB)",
        mb(sbytes),
        mb(cbytes)
    );
    eprintln!("  strongly reachable freed blocks (violations): {}", strong.len());
    let mut by_referrer: FxHashMap<String, u64> = FxHashMap::default();
    for r in strong.values() {
        let class = r.split(" +").next().unwrap_or(r).split(" [").next().unwrap_or(r).to_string();
        let key = match heap_frames(r) {
            Some(f) => format!("{} {{{}}}", class, f.join(" <- ")),
            None => class,
        };
        *by_referrer.entry(key).or_default() += 1;
    }
    let mut rows: Vec<(String, u64)> = by_referrer.into_iter().collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1));
    for (k, n) in rows.iter().take(25) {
        eprintln!("    violations {n:>8} from {k}");
    }
    // By freed block's class and referrer field (class + offset): one row per violation class.
    let mut by_edge: FxHashMap<String, u64> = FxHashMap::default();
    for (&i, r) in strong.iter() {
        let referrer = r.split(" (words").next().unwrap_or(r).split(" [").next().unwrap_or(r);
        let referrer = match heap_frames(r) {
            Some(f) => format!("{referrer} {{{}}}", f.join(" <- ")),
            None => referrer.to_string(),
        };
        *by_edge.entry(format!("{}  <-  {referrer}", class_name(&classes[table.blocks[i].class as usize]))).or_default() += 1;
    }
    let mut rows: Vec<(String, u64)> = by_edge.into_iter().collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (k, n) in &rows {
        eprintln!("    violation class {n:>6}: {k}");
    }
    for (&i, r) in strong.iter().take(chains) {
        let b = table.blocks[i];
        let mut r = r.clone();
        if let Some(f) = heap_frames(&r) {
            r = format!("{r} {{{}}}", f.join(" <- "));
        }
        if std::env::var_os("TSRS_CENSUS_RAW_FRAMES").is_some() {
            if let Some(k) = heap_stack(&r) {
                let raw: Vec<String> = stacks[k].iter().take_while(|&&ip| ip != 0).take(12).map(|ip| names.get(ip).cloned().unwrap_or_else(|| format!("{ip:#x}"))).collect();
                eprintln!("    raw frames: {}", raw.join(" <- "));
            }
        }
        eprintln!("  STRONG {:#x} ({} bytes, {}) <- {r}", b.start, b.size, class_name(&classes[b.class as usize]));
        let mut chain: Vec<String> = Vec::new();
        let mut k = via[i];
        let mut child = i;
        while k != u32::MAX && chain.len() < 12 {
            let kb = table.blocks[k as usize];
            // The offset in the referrer of the first word that points into the block below it on the chain.
            let cb = table.blocks[child];
            let mut at = None;
            let mut p = kb.start as usize;
            while p + 8 <= (kb.start + kb.size as u64) as usize {
                let w = read(p) & MASK48;
                if w >= cb.start && w < cb.start + cb.size as u64 {
                    at = Some(p - kb.start as usize);
                    break;
                }
                p += 4;
            }
            child = k as usize;
            chain.push(format!("+{}", at.map_or("?".into(), |o| o.to_string())));
            let frames = heap_class_stack(kb.class)
                .map(|st| stacks[st].iter().take_while(|&&ip| ip != 0).filter_map(|ip| names.get(ip)).filter(|n| !boring(n)).take(3).cloned().collect::<Vec<_>>().join(" < "))
                .map_or(String::new(), |f| format!(" {{{f}}}"));
            chain.push(format!("{}{frames} ({:#x})", class_name(&classes[kb.class as usize]), kb.start));
            k = via[k as usize];
        }
        eprintln!("      reached via: {}", chain.join(" <- "));
    }
    if strong.is_empty() {
        return;
    }
    if std::env::var_os("TSRS_CENSUS_ASSERT").is_some_and(|v| v == "1") {
        eprintln!("census would-free check failed: {} freed blocks are strongly reachable", strong.len());
        std::process::exit(3);
    }
}

/// The first crate of a v0-mangled symbol (`_RNvCs<hash>_7___rustc12___rust_alloc` -> `__rustc`).
fn v0_crate(name: &str) -> Option<&str> {
    let rest = &name[name.find("Cs")? + 2..];
    let rest = &rest[rest.find('_')? + 1..];
    let digits = rest.bytes().take_while(|b| b.is_ascii_digit()).count();
    let len: usize = rest[..digits].parse().ok()?;
    // An identifier that starts with `_` or a digit has a `_` separator after its length.
    let at = if rest[digits..].starts_with('_') { digits + 1 } else { digits };
    rest.get(at..at + len)
}

/// Frames that belong to the allocator or to generic std / collection code: the site is the first frame below them.
fn boring(name: &str) -> bool {
    if name.starts_with("_R") {
        return v0_crate(name).is_none_or(|c| matches!(c, "__rustc" | "alloc" | "core" | "std" | "hashbrown" | "indexmap"));
    }
    let n = name.trim_start_matches(['<', '_']);
    [
        "tsrs_core::alloc_profile",
        "rust_",
        "rdl_",
        "alloc::",
        "core::",
        "std::",
        "hashbrown::",
        "indexmap::",
        "rustc_hash::",
        "smallvec::",
        "mi_",
        "0x",
    ]
    .iter()
    .any(|p| n.starts_with(p))
}

/// Arena attribution also skips the arena entry points and the generic constructors every object goes through.
fn arena_wrapper(name: &str) -> bool {
    boring(name)
        || name.starts_with("tsrs_core::")
        || name.ends_with("::new_node")
        || name.ends_with("Type::alloc")
        || name.ends_with("::new_type")
        || name.ends_with("::get_or_insert")
        || name.contains("NodeFactory::")
        || name.ends_with("::new_type_mapper")
        || name.ends_with("SymbolTable::new")
        || name.ends_with("Checker::new_symbol")
        || name.ends_with("Checker::new_symbol_ex")
        || name.ends_with("Checker::new_property")
}

/// `atos` output line -> readable function name: no image/offset noise, no hash suffix, legacy-mangling escapes
/// decoded, `<impl path::Type>::f` shortened to `Type::f`.
fn function_name(line: &str) -> String {
    let mut s = line;
    if let Some(i) = s.find(" (in ") {
        s = &s[..i];
    }
    let s = s.trim();
    let s = match s.rfind("::h") {
        Some(i) if s.len() - i == 19 && s[i + 3..].bytes().all(|b| b.is_ascii_hexdigit()) => &s[..i],
        _ => s,
    };
    let mut out = s.to_string();
    for (from, to) in [
        ("$LT$", "<"),
        ("$GT$", ">"),
        ("$u20$", " "),
        ("$C$", ","),
        ("$RF$", "&"),
        ("$BP$", "*"),
        ("$u7b$", "{"),
        ("$u7d$", "}"),
        ("$u27$", "'"),
        ("$u5b$", "["),
        ("$u5d$", "]"),
        ("$u3b$", ";"),
        ("..", "::"),
    ] {
        out = out.replace(from, to);
    }
    while let Some(i) = out.find("_<impl ").or_else(|| out.find("<impl ")) {
        let open = out[i..].find('<').unwrap() + i;
        let mut depth = 0;
        let mut close = None;
        for (j, ch) in out[open..].char_indices() {
            match ch {
                '<' => depth += 1,
                '>' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(open + j);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(close) = close else { break };
        let inner = &out[open + 6..close];
        let ty = inner.rsplit("::").next().unwrap_or(inner).to_string();
        // Drop the module path before the impl too: `tsrs_checker::checker_09::<impl ...>::f` -> `Checker::f`.
        let start = out[..i].rfind(' ').map_or(0, |k| k + 1);
        out = format!("{}{}{}", &out[..start], ty, &out[close + 1..]);
    }
    out
}

fn atos(ips: &[usize], names: &mut FxHashMap<usize, String>) {
    let mut todo: Vec<usize> = ips.iter().copied().filter(|ip| *ip > 1 && !names.contains_key(ip)).collect();
    todo.sort_unstable();
    todo.dedup();
    if todo.is_empty() {
        return;
    }
    let exe = std::env::current_exe().unwrap();
    // SAFETY: image 0 is the main executable.
    let load = unsafe { _dyld_get_image_header(0) } as usize;
    for chunk in todo.chunks(20_000) {
        let mut cmd = std::process::Command::new("atos");
        cmd.arg("-o").arg(&exe).arg("-l").arg(format!("{load:#x}"));
        for ip in chunk {
            cmd.arg(format!("{:#x}", ip - 1));
        }
        let out = cmd.output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
        let mut lines = out.lines();
        for ip in chunk {
            names.insert(*ip, lines.next().map(function_name).unwrap_or_else(|| format!("{ip:#x}")));
        }
    }
}
