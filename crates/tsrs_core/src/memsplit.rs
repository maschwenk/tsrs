//! `TSRS_MEM_SPLIT=1` (debug stat, any build): splits the resident memory at three points of a run into the thread
//! arenas' used bytes, the resident but never used parts of their chunks, the allocator heap's live blocks and the
//! rest it keeps resident, thread stacks and file-backed pages, and prints one block per point on stderr
//! (notes/mem-linux-residency-32.md). The points: `parse end` (the program is parsed and bound, before checker
//! creation), `check end` (every checker of the type-check pass has finished its files and its thread is still
//! alive: the moment memory peaks) and `exit`. `TSRS_MEM_SPLIT=purge` also makes the allocator purge its freed memory
//! after `check end` and prints the split again. Linux reads `/proc/self/smaps` and `/proc/self/status`; elsewhere only
//! the arena and heap lines print.

use std::fmt::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::arena::Arena;

fn mode() -> u8 {
    static MODE: OnceLock<u8> = OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("TSRS_MEM_SPLIT").as_deref() {
        Ok("1") => 1,
        Ok("purge") => 2,
        _ => 0,
    })
}

/// Whether `TSRS_MEM_SPLIT` is `1` or `purge`. Read once.
pub fn enabled() -> bool {
    mode() != 0
}

/// `TSRS_MEM_SPLIT=purge`: after the `check end` report, the allocator purges every arena's freed memory
/// (`mi_collect(true)`) and the split is printed again, which shows how much of the heap's resident rest is freed
/// memory not yet given back.
pub fn purge_after_check() -> bool {
    mode() == 2
}

struct ArenaRef(&'static Arena);
// SAFETY: `report` reads an arena only while the threads that allocate are parked (the barrier at `check end`, a
// joined work group, an idle pool), which orders the reads after the owner's writes.
unsafe impl Send for ArenaRef {}

static ARENAS: Mutex<Vec<ArenaRef>> = Mutex::new(Vec::new());

/// Records a new thread arena when the stat is on.
pub(crate) fn register_arena(arena: &'static Arena) {
    if enabled() {
        ARENAS.lock().unwrap().push(ArenaRef(arena));
    }
}

/// The allocator heap's own view (`mi_heap_visit_blocks` in the CLI): pages, bytes of their initialized blocks, bytes
/// of live blocks.
#[derive(Clone, Copy, Default)]
pub struct HeapStats {
    pub pages: usize,
    pub capacity: usize,
    pub used: usize,
    /// Resident bytes of the pages' areas (`mincore`), header and partly initialized tail included.
    pub resident: usize,
}

static HEAP_STATS: OnceLock<fn() -> HeapStats> = OnceLock::new();
static HEAP_COLLECT: OnceLock<fn(bool)> = OnceLock::new();

/// Installs the heap walker and the allocator's collect (`mi_collect`), from the binary that owns the global
/// allocator.
pub fn set_heap_hooks(stats: fn() -> HeapStats, collect: fn(bool)) {
    let _ = HEAP_STATS.set(stats);
    let _ = HEAP_COLLECT.set(collect);
}

/// The allocator's collect on the calling thread (`force` also purges every arena's freed memory), if installed.
pub fn heap_collect(force: bool) {
    if let Some(f) = HEAP_COLLECT.get() {
        f(force);
    }
}

/// Resident bytes of the stacks of the threads parked at the `check end` point, measured by each thread itself
/// (`note_own_stack`).
static CHECKER_STACKS: AtomicUsize = AtomicUsize::new(0);
static CHECKER_STACK_THREADS: AtomicUsize = AtomicUsize::new(0);

/// Adds the calling thread's resident stack pages to the `check end` report (called by each checker thread before the
/// report).
pub fn note_own_stack() {
    if let Some(bytes) = own_stack_resident() {
        // Relaxed (both): summed before a barrier that orders them before the report.
        CHECKER_STACKS.fetch_add(bytes, Ordering::Relaxed);
        CHECKER_STACK_THREADS.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(target_os = "linux")]
fn own_stack_resident() -> Option<usize> {
    // SAFETY: pthread_getattr_np fills the attribute object for the calling thread; it is destroyed after use.
    unsafe {
        let mut attr: libc::pthread_attr_t = std::mem::zeroed();
        if libc::pthread_getattr_np(libc::pthread_self(), &raw mut attr) != 0 {
            return None;
        }
        let mut addr: *mut libc::c_void = std::ptr::null_mut();
        let mut size: libc::size_t = 0;
        let ok = libc::pthread_attr_getstack(&raw const attr, &raw mut addr, &raw mut size) == 0;
        libc::pthread_attr_destroy(&raw mut attr);
        ok.then(|| resident(addr.addr(), size))
    }
}

#[cfg(not(target_os = "linux"))]
fn own_stack_resident() -> Option<usize> {
    None
}

/// Resident bytes of the whole pages inside `start .. start + len` (`mincore`).
#[cfg(unix)]
pub fn resident(start: usize, len: usize) -> usize {
    // SAFETY: sysconf reads a system constant.
    let page = usize::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).unwrap_or(4096);
    let s = start.next_multiple_of(page);
    let e = (start + len) & !(page - 1);
    if e <= s {
        return 0;
    }
    let mut vec = vec![0u8; (e - s) / page];
    // SAFETY: mincore only reads the page tables of the range and writes one byte per page into `vec`.
    let rc = unsafe { libc::mincore(std::ptr::without_provenance_mut(s), e - s, vec.as_mut_ptr().cast()) };
    if rc != 0 {
        return 0;
    }
    vec.iter().filter(|&&b| b & 1 != 0).count() * page
}

#[cfg(not(unix))]
pub fn resident(_start: usize, _len: usize) -> usize {
    0
}

const MIB: f64 = (1 << 20) as f64;

fn mib(b: usize) -> String {
    format!("{:.1}", b as f64 / MIB)
}

#[derive(Default)]
struct ArenaTotals {
    count: usize,
    capacity: usize,
    used: usize,
    /// Resident bytes below the finger of the current chunk (never handed out).
    tail_resident: usize,
    /// Resident bytes below the finger of retired chunks.
    retired_resident: usize,
    /// Arenas whose current chunk is a huge-page chunk (2 MiB blocks).
    huge_current: usize,
}

/// Prints the split for point `what` on stderr.
pub fn report(what: &str) {
    if !enabled() {
        return;
    }
    let mut out = format!("tsrs mem split: {what}\n");
    let mut arenas = ArenaTotals::default();
    for a in ARENAS.lock().unwrap().iter() {
        let r = a.0.residency();
        arenas.count += 1;
        arenas.capacity += r.capacity;
        arenas.used += r.used;
        arenas.tail_resident += r.current_unused.map_or(0, |(s, l)| resident(s, l));
        arenas.retired_resident += r.retired_unused.iter().map(|&(s, l)| resident(s, l)).sum::<usize>();
        arenas.huge_current += usize::from(r.current_huge);
    }
    let _ = writeln!(
        out,
        "  thread arenas: {} arenas, {} MiB capacity, {} MiB used, unused resident {} MiB in current chunks + {} MiB in retired chunks, {} huge current chunks",
        arenas.count,
        mib(arenas.capacity),
        mib(arenas.used),
        mib(arenas.tail_resident),
        mib(arenas.retired_resident),
        arenas.huge_current
    );
    #[cfg(compressed_ptrs)]
    let _ = writeln!(out, "  reservation: {} MiB in chunks", mib(crate::reserve::reserved_in_use()));
    let heap = HEAP_STATS.get().map(|f| f());
    if let Some(h) = heap {
        let _ = writeln!(
            out,
            "  heap (allocator walk): {} pages, {} MiB initialized blocks, {} MiB live blocks, {} MiB resident in the pages' areas",
            h.pages,
            mib(h.capacity),
            mib(h.used),
            mib(h.resident)
        );
    }
    // Relaxed (both): the checker threads added to them before the barrier this report runs behind.
    let stacks = CHECKER_STACKS.swap(0, Ordering::Relaxed);
    let stack_threads = CHECKER_STACK_THREADS.swap(0, Ordering::Relaxed);
    if stack_threads > 0 {
        let _ = writeln!(out, "  checker stacks (own mincore): {stack_threads} threads, {} MiB resident", mib(stacks));
    }
    #[cfg(target_os = "linux")]
    linux::append(&mut out, &arenas, heap);
    eprint!("{out}");
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{mib, ArenaTotals, HeapStats};
    use std::fmt::Write;

    #[derive(Default)]
    struct Bucket {
        vmas: usize,
        rss: usize,
        huge: usize,
    }

    fn kb(line: &str) -> usize {
        line.split_whitespace().nth(1).and_then(|v| v.parse::<usize>().ok()).unwrap_or(0) << 10
    }

    pub(super) fn append(out: &mut String, arenas: &ArenaTotals, heap: Option<HeapStats>) {
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        let field = |name: &str| status.lines().find(|l| l.starts_with(name)).map_or(0, kb);
        let threads = status.lines().find(|l| l.starts_with("Threads:")).and_then(|l| l.split_whitespace().nth(1)).unwrap_or("?");
        let _ = writeln!(
            out,
            "  status: VmRSS {} MiB, VmHWM {} MiB, RssAnon {} MiB, RssFile {} MiB, threads {threads}",
            mib(field("VmRSS:")),
            mib(field("VmHWM:")),
            mib(field("RssAnon:")),
            mib(field("RssFile:"))
        );
        let smaps = std::fs::read_to_string("/proc/self/smaps").unwrap_or_default();
        #[cfg(compressed_ptrs)]
        let arena_range = (crate::reserve::BASE_ADDR, crate::reserve::BASE_ADDR + crate::reserve::RESERVE);
        #[cfg(not(compressed_ptrs))]
        let arena_range = (0usize, 0usize);
        let names = ["arena reservation", "thread stacks", "heap and other anonymous", "file-backed"];
        let mut buckets: [Bucket; 4] = Default::default();
        let mut stacks_by_size: std::collections::BTreeMap<usize, (usize, usize)> = std::collections::BTreeMap::new();
        let mut cur: Option<(usize, usize)> = None;
        for line in smaps.lines() {
            let mut words = line.split_whitespace();
            let first = words.next().unwrap_or("");
            if let Some((a, b)) = first.split_once('-').filter(|(a, _)| a.chars().all(|c| c.is_ascii_hexdigit()) && !a.is_empty()) {
                let (Ok(start), Ok(end)) = (usize::from_str_radix(a, 16), usize::from_str_radix(b, 16)) else {
                    continue;
                };
                let perms = words.next().unwrap_or("");
                let path = words.nth(3).unwrap_or("");
                let size = end - start;
                // Thread stacks by size: the checker threads' and the CLI thread's 512 MiB, the parse workers' 256 MiB
                // (nothing else maps anonymous memory of that size).
                let class = if start >= arena_range.0 && end <= arena_range.1 {
                    0
                } else if path == "[stack]" || (path.is_empty() && perms.starts_with("rw") && (200 << 20..=600 << 20).contains(&size)) {
                    1
                } else if path.starts_with('/') {
                    3
                } else {
                    2
                };
                buckets[class].vmas += 1;
                cur = Some((class, size));
            } else if let Some((class, size)) = cur {
                if first == "Rss:" {
                    let rss = kb(line);
                    buckets[class].rss += rss;
                    if class == 1 {
                        let e = stacks_by_size.entry(size >> 20).or_default();
                        e.0 += 1;
                        e.1 += rss;
                    }
                } else if first == "AnonHugePages:" {
                    buckets[class].huge += kb(line);
                }
            }
        }
        let _ = writeln!(out, "  smaps: bucket, mappings, Rss MiB, of which AnonHugePages MiB");
        let mut rss_total = 0;
        for (name, b) in names.iter().zip(&buckets) {
            rss_total += b.rss;
            let _ = writeln!(out, "    {name}: {}, {}, {}", b.vmas, mib(b.rss), mib(b.huge));
        }
        let _ = writeln!(out, "    total: {} MiB", mib(rss_total));
        for (size, (n, rss)) in &stacks_by_size {
            let _ = writeln!(out, "    stacks of {size} MiB: {n}, {} MiB resident", mib(*rss));
        }
        let arena_slack = arenas.tail_resident + arenas.retired_resident;
        let arena_other = buckets[0].rss as f64 - (arenas.used + arena_slack) as f64;
        let _ = writeln!(
            out,
            "  split MiB: arena used {} + arena unused resident {} + arena other (regions, untouched used) {:.1} | heap live {} + heap retained {} (in pages {}, outside pages {}) | stacks {} | file {} | = {}",
            mib(arenas.used),
            mib(arena_slack),
            arena_other / super::MIB,
            heap.map_or("?".into(), |h| mib(h.used)),
            heap.map_or("?".into(), |h| mib(buckets[2].rss.saturating_sub(h.used))),
            heap.map_or("?".into(), |h| mib(h.resident.saturating_sub(h.used))),
            heap.map_or("?".into(), |h| mib(buckets[2].rss.saturating_sub(h.resident))),
            mib(buckets[1].rss),
            mib(buckets[3].rss),
            mib(rss_total)
        );
    }
}

/// tsrs-only (`--maxMemory`, notes/mem-recycle-checkers.md): the process's memory in bytes as the system accounts it
/// for limits: the physical footprint on macOS (what `/usr/bin/time -l` reports as peak memory footprint), the resident
/// set on Linux (`/proc/self/statm`). Re-read at most every 5 ms; callers between two checked files share the value.
pub fn process_memory() -> usize {
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::OnceLock;
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    static LAST_NS: AtomicU64 = AtomicU64::new(0);
    static VALUE: AtomicUsize = AtomicUsize::new(0);
    let now = START.get_or_init(std::time::Instant::now).elapsed().as_nanos() as u64;
    // Relaxed (both): a cached reading; a stale or doubly refreshed value only moves a heuristic by 5 ms.
    let last = LAST_NS.load(Ordering::Relaxed);
    if last != 0 && now.saturating_sub(last) < 5_000_000 {
        return VALUE.load(Ordering::Relaxed);
    }
    let v = read_process_memory();
    // Relaxed (both): these atomics publish only a heuristic sample and its refresh time; readers may see
    // a stale value with a newer timestamp, since neither synchronizes access to other memory.
    VALUE.store(v, Ordering::Relaxed);
    LAST_NS.store(now.max(1), Ordering::Relaxed);
    v
}

#[cfg(target_os = "macos")]
fn read_process_memory() -> usize {
    unsafe extern "C" {
        static mach_task_self_: u32;
        fn task_info(target_task: u32, flavor: i32, task_info_out: *mut u32, count: *mut u32) -> i32;
    }
    const TASK_VM_INFO: i32 = 22;
    // `task_vm_info_data_t`: `phys_footprint` is the u64 at byte 144 (revision 1 and later).
    let mut info = [0u32; 128];
    let mut count = info.len() as u32;
    // SAFETY: `info` has room for `count` naturals; the kernel writes at most that many and updates `count`.
    let kr = unsafe { task_info(mach_task_self_, TASK_VM_INFO, info.as_mut_ptr(), &raw mut count) };
    if kr != 0 || count < 38 {
        return 0;
    }
    (u64::from(info[36]) | u64::from(info[37]) << 32) as usize
}

#[cfg(target_os = "linux")]
fn read_process_memory() -> usize {
    let statm = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    let pages: usize = statm.split_whitespace().nth(1).and_then(|v| v.parse().ok()).unwrap_or(0);
    // SAFETY: sysconf has no preconditions.
    pages * unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as usize
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn read_process_memory() -> usize {
    0
}

/// tsrs-only (the heap huge-page rule in tsrs_cli `main`, notes/perf-heap-thp-by-memory.md): the memory the system
/// could give this process now without paging, in bytes. Linux: `MemAvailable` of `/proc/meminfo`, lowered to what the
/// tightest cgroup v2 `memory.max` of the process's cgroup and its ancestors leaves (a container's limit;
/// `/proc/meminfo` shows the host's memory). macOS: free plus inactive pages (`host_statistics64`; free counts the
/// speculative pages). None where it cannot be read. Allocates nothing: `main` calls it before the first allocation.
pub fn available_memory() -> Option<usize> {
    read_available_memory().filter(|&n| n > 0)
}

#[cfg(target_os = "macos")]
fn read_available_memory() -> Option<usize> {
    unsafe extern "C" {
        fn mach_host_self() -> u32;
        fn host_statistics64(host: u32, flavor: i32, info: *mut u32, count: *mut u32) -> i32;
    }
    const HOST_VM_INFO64: i32 = 4;
    // `vm_statistics64_data_t`, 38 naturals: `free_count` is the first, `inactive_count` the third.
    let mut info = [0u32; 38];
    let mut count = info.len() as u32;
    // SAFETY: `info` has room for `count` naturals; the kernel writes at most that many and updates `count`.
    let kr = unsafe { host_statistics64(mach_host_self(), HOST_VM_INFO64, info.as_mut_ptr(), &raw mut count) };
    if kr != 0 || count < 3 {
        return None;
    }
    // SAFETY: sysconf has no preconditions.
    let page = usize::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).ok()?;
    Some((info[0] as usize + info[2] as usize) * page)
}

#[cfg(target_os = "linux")]
fn read_available_memory() -> Option<usize> {
    let mut buf = [0u8; 4096];
    let available = field(read_file(c"/proc/meminfo", &mut buf)?, b"MemAvailable:")?.checked_mul(1024)?;
    Some(cgroup_available().map_or(available, |cg| cg.min(available)))
}

// cgroup v2: for the process's cgroup and each ancestor with a numeric `memory.max`, that limit less what the cgroup
// holds and cannot reclaim (`memory.current` minus `inactive_file` of `memory.stat`: the working set a Kubernetes
// eviction counts); the smallest. cgroup v1 is not read. Paths are built in a stack buffer, so nothing allocates.
#[cfg(target_os = "linux")]
fn cgroup_available() -> Option<usize> {
    const ROOT: &[u8] = b"/sys/fs/cgroup";
    let mut buf = [0u8; 4096];
    let cgroup = read_file(c"/proc/self/cgroup", &mut buf)?;
    let rel = cgroup.split(|&b| b == b'\n').find_map(|l| l.strip_prefix(b"0::"))?.trim_ascii();
    let rel = rel.strip_prefix(b"/").unwrap_or(rel);
    let rel = rel.strip_suffix(b"/").unwrap_or(rel);
    let mut path = [0u8; 4096];
    let mut dir = ROOT.len();
    if dir + 1 + rel.len() + b"/memory.current\0".len() > path.len() {
        return None;
    }
    path[..dir].copy_from_slice(ROOT);
    if !rel.is_empty() {
        path[dir] = b'/';
        path[dir + 1..dir + 1 + rel.len()].copy_from_slice(rel);
        dir += 1 + rel.len();
    }
    let mut file = [0u8; 8192];
    let mut tightest: Option<usize> = None;
    loop {
        if let Some(max) = read_file(cgroup_file(&mut path, dir, b"memory.max")?, &mut file).and_then(number) {
            let current = read_file(cgroup_file(&mut path, dir, b"memory.current")?, &mut file).and_then(number).unwrap_or(0);
            let inactive_file = read_file(cgroup_file(&mut path, dir, b"memory.stat")?, &mut file).and_then(|s| field(s, b"inactive_file ")).unwrap_or(0);
            let left = max.saturating_sub(current.saturating_sub(inactive_file));
            tightest = Some(tightest.map_or(left, |t| t.min(left)));
        }
        match path[..dir].iter().rposition(|&b| b == b'/') {
            Some(parent) if parent >= ROOT.len() => dir = parent,
            _ => break,
        }
    }
    tightest
}

// `<dir>/<name>` with a NUL, written into `path` after its first `dir` bytes (the cgroup directory).
#[cfg(target_os = "linux")]
fn cgroup_file<'a>(path: &'a mut [u8], dir: usize, name: &[u8]) -> Option<&'a std::ffi::CStr> {
    let end = dir + 1 + name.len();
    path[dir] = b'/';
    path[dir + 1..end].copy_from_slice(name);
    path[end] = 0;
    std::ffi::CStr::from_bytes_with_nul(&path[..=end]).ok()
}

// The start of a file, up to `buf.len()` bytes, read without allocating.
#[cfg(target_os = "linux")]
fn read_file<'a>(path: &std::ffi::CStr, buf: &'a mut [u8]) -> Option<&'a [u8]> {
    // SAFETY: `path` is NUL-terminated.
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    if fd < 0 {
        return None;
    }
    let mut len = 0;
    while len < buf.len() {
        // SAFETY: the destination is the unread rest of `buf`, `buf.len() - len` bytes.
        let n = unsafe { libc::read(fd, buf[len..].as_mut_ptr().cast(), buf.len() - len) };
        if n <= 0 {
            break;
        }
        len += n as usize;
    }
    // SAFETY: `fd` is open and closed once.
    unsafe { libc::close(fd) };
    Some(&buf[..len])
}

// The number after `key` on the line that starts with it (`MemAvailable:   123 kB`, `inactive_file 456`).
#[cfg(any(target_os = "linux", test))]
fn field(text: &[u8], key: &[u8]) -> Option<usize> {
    let rest = text.split(|&b| b == b'\n').find_map(|l| l.strip_prefix(key))?.trim_ascii_start();
    number(&rest[..rest.iter().position(|b| !b.is_ascii_digit()).unwrap_or(rest.len())])
}

// A decimal number with optional surrounding whitespace; None for `max` and anything else.
#[cfg(any(target_os = "linux", test))]
fn number(text: &[u8]) -> Option<usize> {
    std::str::from_utf8(text.trim_ascii()).ok()?.parse().ok()
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn read_available_memory() -> Option<usize> {
    None
}

#[cfg(test)]
mod tests {
    use super::{field, number};

    // The Linux file formats the heap huge-page rule reads (notes/perf-heap-thp-by-memory.md). A parse that returned
    // None here would leave huge pages off on every machine; one that read the wrong line could turn them on in a
    // container whose cgroup limit leaves little memory.
    #[test]
    fn reads_meminfo_and_cgroup_files() {
        let meminfo = b"MemTotal:       263921060 kB\nMemFree:        251245164 kB\nMemAvailable:   258466164 kB\nBuffers:  1 kB\n";
        assert_eq!(field(meminfo, b"MemAvailable:"), Some(258_466_164));
        assert_eq!(field(meminfo, b"Cached:"), None);
        let stat = b"anon 1\nfile 2\ninactive_anon 3\nactive_anon 4\ninactive_file 5368709120\nactive_file 6\n";
        assert_eq!(field(stat, b"inactive_file "), Some(5_368_709_120));
        assert_eq!(number(b"8589934592\n"), Some(8_589_934_592));
        assert_eq!(number(b"max\n"), None);
    }
}
