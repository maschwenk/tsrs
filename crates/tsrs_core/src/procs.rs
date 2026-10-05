//! Forked worker processes (unix): `fork_children` runs a closure in N forked copies of the calling process and
//! `ForkedChildren::join` collects the bytes each one returns. The checker pool's process mode uses it
//! (notes/perf-checker-processes.md): a child inherits the whole process copy-on-write, so arena objects keep their
//! handles and a checker built before the fork simply continues in every child.
//!
//! Fork safety. `fork()` copies only the calling thread, with the memory of the others frozen mid-step. So, at the
//! call:
//! - no other thread may hold a lock the child could take (mimalloc's internal locks, `reserve`'s chunk table, the
//!   stdout/stderr locks, any `Mutex` the child's work uses) or be initializing a `OnceLock`/`LazyLock`: every other
//!   thread must be idle (the rayon pools between jobs, a thread blocked in `join`);
//! - the child's work must not use a thread pool (a rayon `install`/`par_iter` waits for workers that do not exist in
//!   the child) or anything else that needs another thread.
//!
//! The child leaves with `libc::_exit`: no destructors, no atexit handlers, no flush of stdio buffers it inherited.
//! Its standard output and error go to a pipe the parent reads, so nothing a child prints can interleave with the
//! parent's output; `join` forwards it to the parent's standard error.

use std::io::Write;
use std::time::{Duration, Instant};

/// What one child sent, and what the kernel recorded about it.
pub struct ChildOutput {
    /// The bytes the child's work returned.
    pub data: Vec<u8>,
    /// User + system CPU seconds of the child.
    pub cpu_seconds: f64,
    /// Page faults that needed no I/O (copy-on-write copies and first touches of fresh pages).
    pub minor_faults: i64,
}

struct ChildHandle {
    pid: libc::pid_t,
    /// Read end of the child's data pipe.
    out_fd: i32,
    /// Read end of the child's stdout/stderr pipe.
    err_fd: i32,
}

/// Children forked by `fork_children` that have not been joined yet.
pub struct ForkedChildren {
    children: Vec<ChildHandle>,
    /// Wall time of each `fork()` call (the parent's side: copying the page tables).
    pub fork_times: Vec<Duration>,
}

const CHILD_PANICKED: i32 = 101;
const CHILD_WRITE_FAILED: i32 = 102;

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn pipe() -> Result<(i32, i32), String> {
    let mut fds = [0i32; 2];
    // SAFETY: `pipe` writes two descriptors into the array.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Err(format!("pipe failed: {}", std::io::Error::last_os_error()));
    }
    Ok((fds[0], fds[1]))
}

fn close(fd: i32) {
    // SAFETY: closing a descriptor this module opened and owns.
    unsafe { libc::close(fd) };
}

/// Writes all of `data` to `fd`, retrying on interruption. False on any other error.
fn write_all(fd: i32, mut data: &[u8]) -> bool {
    while !data.is_empty() {
        // SAFETY: `data` is a valid buffer of `data.len()` bytes.
        let n = unsafe { libc::write(fd, data.as_ptr().cast(), data.len()) };
        if n < 0 {
            if errno() == libc::EINTR {
                continue;
            }
            return false;
        }
        data = &data[n as usize..];
    }
    true
}

/// Forks `count` children from the calling thread. Child `i` runs `work(i)`, writes the returned bytes (with a
/// length prefix) to its data pipe and exits. A panic in `work` exits the child with status 101 after the panic
/// message went to its stderr pipe. The parent returns as soon as every child is forked; call `join` to collect the
/// results (it must run concurrently with anything long the parent does next, since a child blocks once its pipe is
/// full).
///
/// # Safety
/// Every other thread of the process must be idle and hold no lock, and `work` must not need another thread (module
/// documentation).
pub unsafe fn fork_children(count: usize, work: &mut dyn FnMut(usize) -> Vec<u8>) -> Result<ForkedChildren, String> {
    // Buffered output would otherwise be written once by the parent and again by any child that flushes it.
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    let mut children: Vec<ChildHandle> = Vec::with_capacity(count);
    let mut fork_times = Vec::with_capacity(count);
    for i in 0..count {
        let (out_r, out_w) = pipe()?;
        let (err_r, err_w) = pipe()?;
        let start = Instant::now();
        // SAFETY: the caller guarantees that no other thread holds a lock or is mid-update (see above); the child
        // only runs `work` and then `_exit`s.
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            let error = std::io::Error::last_os_error();
            for fd in [out_r, out_w, err_r, err_w] {
                close(fd);
            }
            let partial = ForkedChildren { children, fork_times };
            let _ = partial.join();
            return Err(format!("fork failed: {error}"));
        }
        if pid == 0 {
            close(out_r);
            close(err_r);
            for sibling in &children {
                close(sibling.out_fd);
                close(sibling.err_fd);
            }
            // SAFETY: plain descriptor operations on descriptors this child owns.
            unsafe {
                libc::dup2(err_w, 1);
                libc::dup2(err_w, 2);
            }
            close(err_w);
            let status = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(i))) {
                Ok(data) => {
                    if write_all(out_w, &(data.len() as u64).to_le_bytes()) && write_all(out_w, &data) {
                        0
                    } else {
                        CHILD_WRITE_FAILED
                    }
                }
                Err(_) => CHILD_PANICKED,
            };
            // SAFETY: ends the child without running destructors or flushing anything inherited from the parent.
            unsafe { libc::_exit(status) };
        }
        fork_times.push(start.elapsed());
        close(out_w);
        close(err_w);
        children.push(ChildHandle { pid, out_fd: out_r, err_fd: err_r });
    }
    Ok(ForkedChildren { children, fork_times })
}

impl ForkedChildren {
    /// Reads every child's pipes until they close (all at once, with `poll`: a child blocked on a full pipe must
    /// never wait for us to finish another child), reaps every child, and forwards what the children printed to our
    /// stderr. Fails when a child was killed, exited with a non-zero status or sent an incomplete result; the message
    /// names the child and includes what it printed.
    pub fn join(self) -> Result<Vec<ChildOutput>, String> {
        let n = self.children.len();
        let mut buffers: Vec<Vec<u8>> = vec![Vec::new(); 2 * n];
        let mut fds: Vec<libc::pollfd> = self
            .children
            .iter()
            .flat_map(|c| [c.out_fd, c.err_fd])
            .map(|fd| libc::pollfd { fd, events: libc::POLLIN, revents: 0 })
            .collect();
        let mut open = fds.len();
        let mut read_error: Option<String> = None;
        let mut chunk = vec![0u8; 1 << 16];
        while open > 0 {
            // SAFETY: `fds` is a valid array of `fds.len()` pollfd entries; entries with a negative fd are ignored.
            let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) };
            if ready < 0 {
                if errno() == libc::EINTR {
                    continue;
                }
                read_error.get_or_insert_with(|| format!("poll failed: {}", std::io::Error::last_os_error()));
                break;
            }
            for (k, pfd) in fds.iter_mut().enumerate() {
                if pfd.fd < 0 || pfd.revents == 0 {
                    continue;
                }
                // SAFETY: `chunk` is a valid buffer of `chunk.len()` bytes.
                let got = unsafe { libc::read(pfd.fd, chunk.as_mut_ptr().cast(), chunk.len()) };
                if got > 0 {
                    buffers[k].extend_from_slice(&chunk[..got as usize]);
                    continue;
                }
                if got < 0 && (errno() == libc::EINTR || errno() == libc::EAGAIN) {
                    continue;
                }
                if got < 0 {
                    read_error.get_or_insert_with(|| format!("reading from child {}: {}", k / 2, std::io::Error::last_os_error()));
                }
                close(pfd.fd);
                pfd.fd = -1;
                open -= 1;
            }
        }
        for pfd in &fds {
            if pfd.fd >= 0 {
                close(pfd.fd);
            }
        }

        let mut outputs = Vec::with_capacity(n);
        let mut failure: Option<String> = read_error;
        let mut buffers = buffers.into_iter();
        for (i, child) in self.children.iter().enumerate() {
            let data = buffers.next().unwrap_or_default();
            let printed = buffers.next().unwrap_or_default();
            let mut status: i32 = 0;
            // SAFETY: an all-zero rusage is a valid value of the plain C struct.
            let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
            loop {
                // SAFETY: waits for our own child; `status` and `usage` are valid out-pointers.
                let r = unsafe { libc::wait4(child.pid, &raw mut status, 0, &raw mut usage) };
                if r < 0 && errno() == libc::EINTR {
                    continue;
                }
                if r < 0 {
                    failure.get_or_insert_with(|| format!("waiting for child {i}: {}", std::io::Error::last_os_error()));
                }
                break;
            }
            if !printed.is_empty() {
                let _ = std::io::stderr().write_all(&printed);
            }
            let how = if libc::WIFSIGNALED(status) {
                Some(format!("was killed by signal {}", libc::WTERMSIG(status)))
            } else if libc::WIFEXITED(status) && libc::WEXITSTATUS(status) != 0 {
                Some(match libc::WEXITSTATUS(status) {
                    CHILD_PANICKED => "panicked".to_string(),
                    CHILD_WRITE_FAILED => "could not write its result".to_string(),
                    code => format!("exited with status {code}"),
                })
            } else {
                None
            };
            let payload = match data.get(..8).map(|h| u64::from_le_bytes(h.try_into().unwrap_or([0; 8])) as usize) {
                Some(len) if data.len() == 8 + len => Some(data[8..].to_vec()),
                _ => None,
            };
            match (how, payload) {
                (None, Some(data)) => {
                    let cpu = usage.ru_utime.tv_sec as f64
                        + usage.ru_utime.tv_usec as f64 * 1e-6
                        + usage.ru_stime.tv_sec as f64
                        + usage.ru_stime.tv_usec as f64 * 1e-6;
                    outputs.push(ChildOutput { data, cpu_seconds: cpu, minor_faults: usage.ru_minflt as i64 });
                }
                (how, _) => {
                    let what = how.unwrap_or_else(|| format!("sent an incomplete result ({} bytes)", data.len()));
                    failure.get_or_insert_with(|| {
                        format!("checker process {i} (pid {}) {what}; its output:\n{}", child.pid, String::from_utf8_lossy(&printed))
                    });
                }
            }
        }
        match failure {
            Some(message) => Err(message),
            None => Ok(outputs),
        }
    }
}

/// `count` atomics in a fresh `MAP_SHARED` anonymous mapping, zeroed: forked children and the parent see the same
/// memory (work queues). Never unmapped.
pub fn shared_atomics(count: usize) -> &'static [std::sync::atomic::AtomicU64] {
    let bytes = (count.max(1) * 8).next_multiple_of(4096);
    // SAFETY: a fresh shared anonymous mapping; nothing else refers to it.
    let p = unsafe { libc::mmap(std::ptr::null_mut(), bytes, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED | libc::MAP_ANON, -1, 0) };
    assert!(p != libc::MAP_FAILED, "mmap of a shared page failed");
    // SAFETY: the mapping is zeroed, page-aligned, `bytes >= count * 8` long and never unmapped; AtomicU64 has the
    // layout of u64.
    unsafe { std::slice::from_raw_parts(p.cast::<std::sync::atomic::AtomicU64>().cast_const(), count) }
}

/// Counters of the calling process for the process-mode report: macOS `proc_pid_rusage`, Linux
/// `/proc/self/smaps_rollup`. Fields a platform does not have are 0.
#[derive(Clone, Copy, Default, Debug)]
pub struct SelfStats {
    pub instructions: u64,
    pub cycles: u64,
    /// macOS: physical footprint now / lifetime peak (pages this process owns: private, dirtied or copied).
    pub phys_footprint: u64,
    pub peak_phys_footprint: u64,
    /// Linux (bytes): proportional set size, private dirty pages, shared pages (clean + dirty), resident set.
    pub pss: u64,
    pub private_dirty: u64,
    pub shared: u64,
    pub rss: u64,
}

impl SelfStats {
    pub fn encode(&self, out: &mut Vec<u8>) {
        for v in [self.instructions, self.cycles, self.phys_footprint, self.peak_phys_footprint, self.pss, self.private_dirty, self.shared, self.rss] {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }

    pub fn decode(input: &mut &[u8]) -> SelfStats {
        let mut next = || {
            let (head, rest) = input.split_at(8);
            *input = rest;
            u64::from_le_bytes(head.try_into().unwrap())
        };
        SelfStats {
            instructions: next(),
            cycles: next(),
            phys_footprint: next(),
            peak_phys_footprint: next(),
            pss: next(),
            private_dirty: next(),
            shared: next(),
            rss: next(),
        }
    }
}

#[cfg(target_os = "macos")]
pub fn self_stats() -> SelfStats {
    // SAFETY: an all-zero rusage_info_v4 is a valid value of the plain C struct.
    let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
    // SAFETY: `info` is a valid rusage_info_v4 buffer for flavor RUSAGE_INFO_V4.
    let ok = unsafe { libc::proc_pid_rusage(libc::getpid(), libc::RUSAGE_INFO_V4, (&raw mut info).cast()) } == 0;
    if !ok {
        return SelfStats::default();
    }
    SelfStats {
        instructions: info.ri_instructions,
        cycles: info.ri_cycles,
        phys_footprint: info.ri_phys_footprint,
        peak_phys_footprint: info.ri_lifetime_max_phys_footprint,
        ..SelfStats::default()
    }
}

#[cfg(not(target_os = "macos"))]
pub fn self_stats() -> SelfStats {
    let mut stats = SelfStats::default();
    let Ok(text) = std::fs::read_to_string("/proc/self/smaps_rollup") else {
        return stats;
    };
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let (Some(key), Some(value)) = (parts.next(), parts.next()) else { continue };
        let Ok(kib) = value.parse::<u64>() else { continue };
        let bytes = kib << 10;
        match key {
            "Rss:" => stats.rss = bytes,
            "Pss:" => stats.pss = bytes,
            "Private_Dirty:" => stats.private_dirty = bytes,
            "Shared_Clean:" | "Shared_Dirty:" => stats.shared += bytes,
            _ => {}
        }
    }
    stats
}

/// Measurement (macOS arm64, `TSRS_CHECKER_PROCESSES_WRITETRACE`): makes `[lo, hi)` read-only in a forked child and
/// records, for the first write to each page, the faulting pc and the return addresses of up to 7 callers (frame
/// pointers); the page is then opened for writing. `write_trace_dump` writes one line per page.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub mod writetrace {
    use std::sync::atomic::{AtomicUsize, Ordering};

    const PAGE: usize = 16384;
    const MAX: usize = 1 << 18;
    const FRAMES: usize = 8;
    static mut RECORDS: [[u64; FRAMES + 1]; MAX] = [[0; FRAMES + 1]; MAX];
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    static LO: AtomicUsize = AtomicUsize::new(0);
    static HI: AtomicUsize = AtomicUsize::new(0);
    // Extra protected ranges (mimalloc's regions), sorted.
    static mut EXTRA: [(usize, usize); 4096] = [(0, 0); 4096];
    static EXTRA_N: AtomicUsize = AtomicUsize::new(0);

    #[repr(C, packed(4))]
    #[derive(Default, Clone, Copy)]
    struct SubmapInfo64 {
        protection: i32,
        max_protection: i32,
        inheritance: u32,
        offset: u64,
        user_tag: u32,
        pages_resident: u32,
        pages_shared_now_private: u32,
        pages_swapped_out: u32,
        pages_dirtied: u32,
        ref_count: u32,
        shadow_depth: u16,
        external_pager: u8,
        share_mode: u8,
        is_submap: i32,
        behavior: i32,
        object_id: u32,
        user_wired_count: u16,
        pad: u16,
        pages_reusable: u32,
        object_id_full: u64,
    }

    extern "C" {
        fn mach_vm_region_recurse(task: u32, address: *mut u64, size: *mut u64, depth: *mut u32, info: *mut i32, count: *mut u32) -> i32;
    }

    fn in_extra(addr: usize) -> bool {
        let n = EXTRA_N.load(Ordering::Relaxed);
        // SAFETY: written before the handler is installed, read-only afterwards.
        let extra = unsafe { &*std::ptr::addr_of!(EXTRA) };
        let ranges = &extra[..n];
        let i = ranges.partition_point(|r| r.0 <= addr);
        i > 0 && addr < ranges[i - 1].1
    }

    /// Physical memory of this process from its VM regions (macOS): (private, shared, resident) bytes. Private:
    /// pages of private regions plus, in copy-on-write regions, the pages in the top object (written or
    /// zero-filled after the fork); shared: resident pages found deeper in a shadow chain (still shared with the
    /// parent). Submaps (the dyld shared cache) are skipped.
    pub fn vm_breakdown() -> (u64, u64, u64) {
        let mut addr: u64 = 0;
        let (mut private, mut shared, mut resident) = (0u64, 0u64, 0u64);
        loop {
            let mut size: u64 = 0;
            let mut depth: u32 = 0;
            let mut info = SubmapInfo64::default();
            let mut count: u32 = (std::mem::size_of::<SubmapInfo64>() / 4) as u32;
            // SAFETY: valid out-pointers of the documented sizes.
            #[expect(deprecated, reason = "measurement code; mach2 is not a dependency")]
            let task = unsafe { libc::mach_task_self() };
            // SAFETY: as above.
            let kr = unsafe { mach_vm_region_recurse(task, &raw mut addr, &raw mut size, &raw mut depth, (&raw mut info).cast(), &raw mut count) };
            if kr != 0 {
                break;
            }
            let (is_submap, share_mode, res, snp) = (info.is_submap, info.share_mode, info.pages_resident, info.pages_shared_now_private);
            if is_submap == 0 {
                let page = PAGE as u64;
                resident += res as u64 * page;
                match share_mode {
                    1 => {
                        private += snp as u64 * page;
                        shared += (res.saturating_sub(snp)) as u64 * page;
                    }
                    2 | 6 => private += res as u64 * page,
                    _ => shared += res as u64 * page,
                }
            }
            addr += size;
        }
        (private, shared, resident)
    }

    /// Adds every read-write region with VM user tag `tag` (mimalloc's: 100) to the protected set.
    pub fn add_tagged_regions(tag: u32) -> usize {
        let mut addr: u64 = 0;
        let mut n = 0;
        let mut total = 0;
        loop {
            let mut size: u64 = 0;
            let mut depth: u32 = 1;
            let mut info = SubmapInfo64::default();
            let mut count: u32 = (std::mem::size_of::<SubmapInfo64>() / 4) as u32;
            // SAFETY: valid out-pointers of the documented sizes.
            let kr = unsafe { mach_vm_region_recurse(libc::mach_task_self(), &raw mut addr, &raw mut size, &raw mut depth, (&raw mut info).cast(), &raw mut count) };
            if kr != 0 {
                break;
            }
            let (tag_of, prot) = (info.user_tag, info.protection);
            if tag_of == tag && prot & 3 == 3 && n < 4096 {
                // SAFETY: single-threaded setup before the handler is installed.
                unsafe { (*std::ptr::addr_of_mut!(EXTRA))[n] = (addr as usize, (addr + size) as usize) };
                n += 1;
                total += size as usize;
            }
            addr += size;
        }
        EXTRA_N.store(n, Ordering::Relaxed);
        total
    }

    extern "C" {
        fn _dyld_get_image_vmaddr_slide(image_index: u32) -> isize;
    }

    extern "C" fn handler(_sig: libc::c_int, info: *mut libc::siginfo_t, ctx: *mut libc::c_void) {
        // SAFETY: the kernel passes a valid siginfo and ucontext; RECORDS is only touched by this single thread.
        unsafe {
            let addr = (*info).si_addr as usize;
            if (addr < LO.load(Ordering::Relaxed) || addr >= HI.load(Ordering::Relaxed)) && !in_extra(addr) {
                libc::signal(libc::SIGBUS, libc::SIG_DFL);
                libc::signal(libc::SIGSEGV, libc::SIG_DFL);
                return;
            }
            let page = addr & !(PAGE - 1);
            let uc = ctx.cast::<libc::ucontext_t>();
            let ss = &(*(*uc).uc_mcontext).__ss;
            let k = COUNT.fetch_add(1, Ordering::Relaxed);
            if k < MAX {
                let rec = &mut *std::ptr::addr_of_mut!(RECORDS[k]);
                rec[0] = page as u64;
                rec[1] = ss.__pc;
                rec[2] = ss.__lr;
                let mut fp = ss.__fp as usize;
                let sp = ss.__sp as usize;
                for slot in rec.iter_mut().skip(3) {
                    if fp < sp || fp >= sp + (512 << 20) || fp % 16 != 0 {
                        break;
                    }
                    *slot = *((fp + 8) as *const u64);
                    fp = *(fp as *const usize);
                }
            }
            libc::mprotect(page as *mut libc::c_void, PAGE, libc::PROT_READ | libc::PROT_WRITE);
        }
    }

    /// # Safety
    /// Single-threaded process; `[lo, hi)` must be mapped memory nobody else protects.
    pub unsafe fn start(lo: usize, hi: usize) {
        LO.store(lo, Ordering::Relaxed);
        HI.store(hi, Ordering::Relaxed);
        // SAFETY: the caller's contract.
        unsafe {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = handler as *const () as usize;
            sa.sa_flags = libc::SA_SIGINFO | libc::SA_NODEFER;
            libc::sigemptyset(&raw mut sa.sa_mask);
            libc::sigaction(libc::SIGSEGV, &raw const sa, std::ptr::null_mut());
            libc::sigaction(libc::SIGBUS, &raw const sa, std::ptr::null_mut());
            libc::mprotect(lo as *mut libc::c_void, hi - lo, libc::PROT_READ);
            let n = EXTRA_N.load(Ordering::Relaxed);
            for k in 0..n {
                let (a, b) = (*std::ptr::addr_of!(EXTRA))[k];
                libc::mprotect(a as *mut libc::c_void, b - a, libc::PROT_READ);
            }
        }
    }

    pub fn dump(path: &str) {
        use std::fmt::Write;
        let n = COUNT.load(Ordering::Relaxed).min(MAX);
        // SAFETY: plain dyld query.
        let slide = unsafe { _dyld_get_image_vmaddr_slide(0) } as u64;
        let mut out = String::new();
        for k in 0..n {
            // SAFETY: written by the handler on this thread before.
            let rec = unsafe { &*std::ptr::addr_of!(RECORDS[k]) };
            let _ = write!(out, "{:#x}", rec[0]);
            for &pc in &rec[1..] {
                if pc == 0 {
                    break;
                }
                let _ = write!(out, " {:#x}", pc.wrapping_sub(slide));
            }
            out.push('\n');
        }
        let _ = std::fs::write(path, out);
    }
}
