//! What the allocation profile needs from the OS: the main image's load address and data segments, the current
//! thread's stack top, and symbol names for raw return addresses. macOS uses dyld and `atos`; Linux reads
//! `/proc/self/maps` and runs `addr2line` (binutils) on the executable, which needs symbols and debug line tables:
//! the `release` profile strips both, so build with `CARGO_PROFILE_RELEASE_STRIP=none` and
//! `CARGO_PROFILE_RELEASE_DEBUG=line-tables-only`.

use rustc_hash::FxHashMap;
use std::ffi::c_void;

#[cfg(target_os = "macos")]
mod imp {
    use super::*;

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

    unsafe extern "C" {
        fn _dyld_get_image_header(index: u32) -> *const c_void;
        fn _dyld_get_image_vmaddr_slide(index: u32) -> isize;
        fn pthread_self() -> *mut c_void;
        fn pthread_get_stackaddr_np(thread: *mut c_void) -> *mut c_void;
    }

    pub(crate) fn image_base() -> usize {
        // SAFETY: image 0 is the main executable.
        unsafe { _dyld_get_image_header(0) as usize }
    }

    /// The main executable's `__DATA`, `__DATA_CONST` and `__DATA_DIRTY` segments (statics).
    pub(crate) fn data_segments() -> Vec<(usize, usize, String)> {
        const LC_SEGMENT_64: u32 = 0x19;
        let mut out = Vec::new();
        // SAFETY: image 0 is the main executable; its load commands follow the header and are mapped.
        unsafe {
            let header = _dyld_get_image_header(0).cast::<MachHeader64>();
            let slide = _dyld_get_image_vmaddr_slide(0);
            let mut p = header.cast::<u8>().add(std::mem::size_of::<MachHeader64>());
            for _ in 0..(*header).ncmds {
                let cmd = &*p.cast::<SegmentCommand64>();
                if cmd.cmd == LC_SEGMENT_64 && cmd.segname.starts_with(b"__DATA") {
                    let name = String::from_utf8_lossy(&cmd.segname).trim_end_matches('\0').to_string();
                    out.push(((cmd.vmaddr as isize + slide) as usize, cmd.vmsize as usize, name));
                }
                p = p.add(cmd.cmdsize as usize);
            }
        }
        out
    }

    pub(crate) fn stack_high() -> usize {
        // SAFETY: plain libc queries about the current thread.
        unsafe { pthread_get_stackaddr_np(pthread_self()) as usize }
    }

    /// One `atos` line per address (`function (in tsrs) (file.rs:12)`).
    pub(crate) fn symbolize(ips: &[usize]) -> Vec<String> {
        let exe = std::env::current_exe().unwrap();
        let load = image_base();
        let mut out = Vec::with_capacity(ips.len());
        for chunk in ips.chunks(20_000) {
            let mut cmd = std::process::Command::new("atos");
            cmd.arg("-o").arg(&exe).arg("-l").arg(format!("{load:#x}"));
            for ip in chunk {
                cmd.arg(format!("{:#x}", ip - 1));
            }
            let text = cmd.output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
            let mut lines = text.lines();
            for ip in chunk {
                out.push(lines.next().map(str::to_string).unwrap_or_else(|| format!("{ip:#x}")));
            }
        }
        out
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::*;

    /// The executable's mappings in `/proc/self/maps`: (start, end, permissions, file offset).
    fn exe_mappings() -> (Vec<(usize, usize, String, usize)>, Vec<(usize, usize, String)>) {
        let exe = std::fs::read_link("/proc/self/exe").unwrap_or_default();
        let exe = exe.to_string_lossy().into_owned();
        let maps = std::fs::read_to_string("/proc/self/maps").unwrap_or_default();
        let mut mine = Vec::new();
        let mut all = Vec::new();
        for line in maps.lines() {
            let mut it = line.split_whitespace();
            let (Some(range), Some(perms), Some(offset)) = (it.next(), it.next(), it.next()) else { continue };
            let path = it.nth(2).unwrap_or("").to_string();
            let Some((a, b)) = range.split_once('-') else { continue };
            let (a, b) = (usize::from_str_radix(a, 16).unwrap_or(0), usize::from_str_radix(b, 16).unwrap_or(0));
            if path == exe {
                mine.push((a, b, perms.to_string(), usize::from_str_radix(offset, 16).unwrap_or(0)));
            }
            all.push((a, b, path));
        }
        (mine, all)
    }

    pub(crate) fn image_base() -> usize {
        exe_mappings().0.iter().find(|m| m.3 == 0).map_or(0, |m| m.0)
    }

    /// The executable's writable mappings (`.data`, `.got`, ...) and the anonymous mapping right after the last of
    /// them (`.bss`).
    pub(crate) fn data_segments() -> Vec<(usize, usize, String)> {
        let (mine, all) = exe_mappings();
        let mut out: Vec<(usize, usize, String)> =
            mine.iter().filter(|m| m.2.starts_with("rw")).map(|m| (m.0, m.1 - m.0, "data".to_string())).collect();
        if let Some(&(_, end, _)) = out.last().map(|(s, l, n)| (s, s + l, n)).as_ref() {
            if let Some(bss) = all.iter().find(|m| m.0 == end && m.2.is_empty()) {
                out.push((bss.0, bss.1 - bss.0, "bss".to_string()));
            }
        }
        out
    }

    #[repr(C, align(8))]
    struct PthreadAttr([u8; 64]);

    unsafe extern "C" {
        fn pthread_self() -> usize;
        fn pthread_getattr_np(thread: usize, attr: *mut PthreadAttr) -> i32;
        fn pthread_attr_getstack(attr: *const PthreadAttr, addr: *mut *mut c_void, size: *mut usize) -> i32;
        fn pthread_attr_destroy(attr: *mut PthreadAttr) -> i32;
    }

    pub(crate) fn stack_high() -> usize {
        let mut attr = PthreadAttr([0; 64]);
        let mut addr = std::ptr::null_mut();
        let mut size = 0usize;
        // SAFETY: plain libc queries about the current thread; `attr` is large enough for glibc's `pthread_attr_t`.
        unsafe {
            if pthread_getattr_np(pthread_self(), &raw mut attr) != 0 {
                return 0;
            }
            pthread_attr_getstack(&raw const attr, &raw mut addr, &raw mut size);
            pthread_attr_destroy(&raw mut attr);
        }
        addr as usize + size
    }

    /// One name per address from `addr2line -f -C -i`: the inlined frames at the address, innermost first, joined
    /// by " / " (so a hash table's growth inlined into a checker function names both).
    pub(crate) fn symbolize(ips: &[usize]) -> Vec<String> {
        let exe = std::fs::read_link("/proc/self/exe").unwrap_or_default();
        let base = image_base();
        let mut out = Vec::with_capacity(ips.len());
        for chunk in ips.chunks(20_000) {
            let mut cmd = std::process::Command::new("addr2line");
            cmd.arg("-a").arg("-f").arg("-C").arg("-i").arg("-e").arg(&exe);
            for ip in chunk {
                cmd.arg(format!("{:#x}", ip.wrapping_sub(1).wrapping_sub(base)));
            }
            let text = cmd.output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
            // Output: per address a `0x...` line, then (function, file:line) pairs, innermost first.
            let mut groups: Vec<Vec<String>> = Vec::new();
            let mut lines = text.lines();
            while let Some(line) = lines.next() {
                if line.starts_with("0x") {
                    groups.push(Vec::new());
                } else if let Some(g) = groups.last_mut() {
                    g.push(line.to_string());
                    lines.next(); // file:line
                }
            }
            for (i, ip) in chunk.iter().enumerate() {
                let names = groups.get(i).cloned().unwrap_or_default();
                out.push(if names.is_empty() { format!("{ip:#x}") } else { names.join(" / ") });
            }
        }
        out
    }
}

pub(crate) use imp::{data_segments, image_base, stack_high, symbolize};

/// Resolves every address of `ips` that `names` does not have yet, through `symbolize`, cleaned by `clean`.
pub(crate) fn resolve_into(ips: &[usize], names: &mut FxHashMap<usize, String>, clean: impl Fn(&str) -> String) {
    let mut todo: Vec<usize> = ips.iter().copied().filter(|ip| *ip > 1 && !names.contains_key(ip)).collect();
    todo.sort_unstable();
    todo.dedup();
    if todo.is_empty() {
        return;
    }
    for (ip, line) in todo.iter().zip(symbolize(&todo)) {
        names.insert(*ip, clean(&line));
    }
}
