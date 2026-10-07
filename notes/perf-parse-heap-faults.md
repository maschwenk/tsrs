# perf-parse-heap-faults: what the `no_thp` build's parse cost is, and why no fix fits the memory budget

PR #121 built mimalloc with `no_thp` (`notes/mem-no-thp.md`): -24 to -33% peak RSS, but in the 20-rep head-to-head on
the 64-vCPU runner the parallel parse took 27-37 ms more (0.096 -> 0.123 s, PGO build). The same PR capped the parse
pool at 32 threads (`PARSE_THREAD_CAP`). This note profiles that parse cost and tests the fixes that match the
profile. Nothing landed.

In short:

- **At the shipped 32 parse threads the cost is small and not in page faults.** The THP heap parses 0-7 ms faster;
  page faults are 4.4% of samples on `no_thp` and 6.0% on the THP heap.
- **The rest of the old gap was 64-thread scaling, which `no_thp` loses to a lock.** Past ~40 threads, the kernel's
  copies of file text into fresh heap pages (`read()` into `std::fs::read`'s buffer) fault in kernel mode. Those
  faults wait on `mmap_lock` behind thread starts and arena commits: 7.4% of samples at 64 threads.
- **Touching the buffer from user space before the `read()` removes the wait.** With it, `no_thp` at 48-64 threads
  parses as fast as the THP heap at 64.
- **The extra threads cost memory.** Each parse thread that gets work keeps about 2-3 MiB of peak (partly filled
  mimalloc pages, arena chunk tails). At 48 threads formbricks-web's peak at 4 checkers is +2.0-2.1%. At 40 threads,
  where the touch is not needed, small projects are +2-5.5%.
- **No mimalloc option changes the parse time.**

Everything below is from Depot `depot-ubuntu-24.04-64` (AMD EPYC 9R45, 64 vCPUs), `perf-probe.yml` with a probe script
per round, `cargo build --release` (not PGO), vscode `-p src --noEmit --incremental false --extendedDiagnostics
--pretty false`. Values are medians of 7-9 interleaved runs. "Peak" is `/usr/bin/time` max RSS. The runners had kernel
6.12.109 with THP `madvise` and **32 KiB anonymous mTHP set to `always`**
(`/sys/kernel/mm/transparent_hugepage/hugepages-32kB/enabled`). So the `no_thp` heap faults in 32 KiB folios, not
4 KiB pages: 17.5k minor faults for a 1.3 GiB `--noCheck` run. PR #121's runs did not record the kernel.

## What the cost is

### Same thread count: a few ms

Round 1 (run z1tnp89rnm) and round 2 (pmc0p2c2xq) compared main against the same tree built without `no_thp`
(target/thp), both capped at 32 parse threads:

| build, 32 threads | parse s (default) | parse s (`--noCheck`) | check s | peak (default) | minor faults (`--noCheck`) | dTLB load misses |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `no_thp` (main) | 0.123-0.124 | 0.123-0.127 | 0.465-0.488 | 2.77-2.79 GiB | 16.8-17.6k | 0.95-1.06 M |
| THP heap | 0.118-0.123 | 0.116-0.120 | 0.478-0.493 | 3.67-3.70 GiB | 5.4-5.7k | 0.32-0.35 M |

- At the same 32 threads the THP heap parses 0-5 ms faster at the default and 4-7 ms faster with `--noCheck`.
- Check time is the same or slightly better without THP.
- User CPU is the same, and system time is 0.27-0.42 s in both builds.

A `perf record -g` of the 32-thread `--noCheck` run puts page faults at 4.4% of samples on `no_thp` and 6.0% on THP:
the huge-page build spends more time zeroing (`clear_page_erms` 4.3% vs 1.8%). At 32 threads the parse is not
fault-bound in either build.

### Past 32 threads: a lock, not the faults

The old main configuration was the THP heap with 64 parse threads. Its advantage came from scaling past 32 threads,
which `no_thp` did not do (round 2):

| threads | parse s, `no_thp` (default / `--noCheck`) | parse s, THP heap | context switches, `no_thp` / THP |
| --- | --- | --- | --- |
| 32 | 0.123 / 0.123 | 0.118 / 0.116 | 3.1k / 2.7k |
| 48 | 0.125 / 0.118 | 0.108 / 0.109 | 4.8k / 3.7k |
| 64 | 0.126 / 0.131 | 0.111 / 0.109 | 6.5k / 4.4k |

The profile at 64 threads (`perf script` stacks, classified by kernel entry, `--noCheck`):

| | `no_thp`, 64 threads | THP heap, 64 threads | `no_thp`, 32 threads |
| --- | ---: | ---: | ---: |
| samples in page faults | 13.4% | 6.9% | 4.4% |
| of which asleep in `rwsem_down_read_slowpath <- down_read_killable <- lock_mm_and_find_vma` | **7.4%** | 0.5% | 0.1% |
| user frame of those sleeping faults | `_copy_to_iter <- copy_page_to_iter <- filemap_read` | | |

On `no_thp` at 64 threads, 7.4% of the run's samples are page faults blocked on the process's `mmap_lock`. The
faulting code is the kernel itself: `read()` copying a source file into the freshly allocated `Vec` of
`std::fs::read`. Those pages are first touched in kernel mode. On 6.12 the fault handler serves a kernel-mode
fault under `mmap_lock` (`lock_mm_and_find_vma`), never through the per-VMA lock a user-mode fault takes. So these
faults queue behind every writer of `mmap_lock`.

The writers, counted with `perf record -e syscalls:sys_enter_{mprotect,madvise,mmap,munmap} -g` on a 48-thread
`--noCheck` run:

- **Thread lifecycle:** 401 thread starts, each an `mmap` for the stack, an `mprotect` for the guard page, an `mmap`
  for the signal stack and `munmap`s at exit. These are the rayon pools and the checker pool's work groups.
- **Calls with no symbolized caller:** about 270 `mprotect` and 480 `madvise`. The tsrs arena's chunk commits
  (`reserve.rs` `commit`: `madvise(MADV_HUGEPAGE)` then `mprotect` per chunk) and mimalloc's purges fit those counts.

With the THP heap the text buffers mostly land in heap memory that is already resident (each fault maps 2 MiB), so
the kernel's copy rarely faults.

The source text is most of what the front end keeps on the heap. The alloc-profile build's sampled heap profile on
vscode `--noCheck`: heap peak 238-251 MB, 179 MB live at the end. The top live stacks are all
`DirFS::read_file <- Common::read_file <- CompilerHost::get_source_file <- fileregions::parse_task`. About 1 GB more
is allocated and freed during the parse.

`strace -f -c` on `--noCheck`: the syscall mix is the same on both builds (mprotect 1,082, mmap 902, munmap 697,
madvise 444-466). The difference is not more syscalls; it is faults landing in kernel mode while writers hold the lock.

### So the 27-37 ms was

On these runners the 27-37 ms splits into two parts:

- **0-7 ms at equal thread counts.** This is not in the page-fault path. It was not attributed further; the `no_thp`
  run has 3x the dTLB load misses.
- **The pool cap.** The old configuration used 64 threads, which parse 7-14 ms faster than 32 on the THP heap (7 ms
  in round 2; 14 ms in run wfpm41kpd7 of `notes/mem-no-thp.md`). On
  `no_thp`, threads past ~40 sleep on `mmap_lock` in the fault path of their own file reads, so they give nothing.

The exact 27-37 ms of PR #121's PGO head-to-head (runs xlz11b8b2f and wb4933tkmt) was not reproduced. Those runs used
the PGO build and possibly another kernel, and the THP-heap parse at 64 threads there was 0.096 s.

## What was tried

### Touching the read buffer from user space (works past 40 threads; not landed)

`std::fs::read`, except that before the `read()` it writes one byte per 4 KiB page of the reserved buffer
(`spare_capacity_mut().iter_mut().step_by(4096)`). The faults then happen in user mode, under the per-VMA lock, out of
the `mmap_lock` queue. It is about 15 lines in `tsrs_vfs`'s `DirFS::read_file`, Linux only.

Round 3 (rvp5flj7ks) compared one binary with the touch switched by an environment variable:

| variant | parse s (default) | parse s (`--noCheck`) | peak (default) | peak (4 checkers) | context switches |
| --- | ---: | ---: | ---: | ---: | ---: |
| no touch, 32 threads (main) | 0.122 | 0.123 | 2.773 GiB | 1.867 GiB | 2,979 |
| touch, 32 threads | 0.121 | 0.123 | 2.778 GiB | 1.867 GiB | 2,713 |
| touch, 48 threads | 0.116 | 0.111 | 2.816 GiB | 1.899 GiB | 3,710 |
| touch, 64 threads | 0.117 | 0.113 | 2.875 GiB | 1.946 GiB | 4,537 |
| no touch, 64 threads | 0.131 | 0.141 | 2.869 GiB | 1.950 GiB | 6,487 |
| THP heap, 64 threads (old main setup) | 0.115 | 0.107 | 4.051 GiB | 2.679 GiB | 4,375 |

- The touch removes the sleeping faults: they drop from 6.15% to 0.40% of samples at 64 threads.
- With the touch, `no_thp` at 48-64 threads parses as fast as the THP heap at 64 (0.116-0.117 vs 0.115 s).
- It changes nothing at 32 threads, and check time does not move.

It did not land because the threads that make it pay off cost memory. Every parse thread keeps its partly filled
mimalloc pages and arena chunk tail, about 2-3 MiB of peak each. Round 4 (7j6fhr6vsj) and round 5 (two runners,
q1fjxdfh1m and lcf8ts20w5) measured the touch with the cap at 48 against main (the merge base built on the same
runner):

| | vscode parse s, main -> touch + 48 | vscode peak, 4 checkers | formbricks-web peak, 4 checkers | formbricks-web parse s |
| --- | --- | --- | --- | --- |
| round 4 | 0.122 -> 0.113 | +1.9% | +1.9% | 0.114 -> 0.113 |
| round 5, runner 1 | 0.121 -> 0.115 | +1.6% | +2.0% | 0.115 -> 0.111 |
| round 5, runner 2 | 0.127 -> 0.118 | +1.1% | +2.1% | 0.119 -> 0.120 |

That is +0.9-1.3% peak at the default 32 checkers, and up to +2.1% at 4 checkers on formbricks-web, over the +2%
budget. At 40 threads the touch measures neutral:

| 40 threads | vscode parse s, default (r5 runner 1 / runner 2) | vscode parse s, `--noCheck` |
| --- | --- | --- |
| main (no touch) | 0.116 / 0.119 | 0.119 / 0.117 |
| touch | 0.117 / 0.118 | 0.114 / 0.118 |

So without a cut in per-thread memory it buys nothing within the budget. If that memory comes down, the touch is
what makes threads past ~40 pay off on a `no_thp` heap. The patch, in `crates/tsrs_vfs/src/osvfs/os.rs`
(`DirFS::read_file` calls it instead of `fs::read`):

```rust
fn read_touched(path: &str) -> io::Result<Vec<u8>> {
    #[cfg(target_os = "linux")]
    {
        use std::io::Read;
        let mut file = fs::File::open(path)?;
        let size = file.metadata().map(|m| usize::try_from(m.len()).unwrap_or(0)).unwrap_or(0);
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(size).map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
        for b in bytes.spare_capacity_mut().iter_mut().step_by(4096) {
            b.write(0);
        }
        file.read_to_end(&mut bytes)?;
        Ok(bytes)
    }
    #[cfg(not(target_os = "linux"))]
    fs::read(path)
}
```

### mimalloc runtime knobs (none helps)

These were set by environment variable on the `no_thp` build at 32 threads, 5 interleaved reps (round 1). The names
are from mimalloc 3.3.2's `include/mimalloc.h` in libmimalloc-sys 0.1.49.

| setting | parse s (default) | parse s (`--noCheck`) | peak (default) |
| --- | ---: | ---: | ---: |
| none | 0.123 | 0.126 | 2.797 GiB |
| `MIMALLOC_ARENA_RESERVE=4GiB` | 0.124 | 0.126 | 2.791 GiB |
| `MIMALLOC_PURGE_DECOMMITS=0` | 0.123 | 0.123 | 2.795 GiB |
| `MIMALLOC_ALLOW_LARGE_OS_PAGES=1` | 0.125 | 0.126 | 2.792 GiB |
| `MIMALLOC_MINIMAL_PURGE_SIZE=2048` | 0.124 | 0.124 | 2.795 GiB |
| `MIMALLOC_PAGE_COMMIT_ON_DEMAND=1` | 0.131 | 0.126 | 2.796 GiB |
| `MIMALLOC_ARENA_EAGER_COMMIT=0` | 0.128 | 0.130 | 2.792 GiB |
| `MIMALLOC_PAGE_FULL_RETAIN=8` | 0.129 | 0.125 | 2.808 GiB |

`MIMALLOC_PAGE_RESET`, `MIMALLOC_ABANDONED_PAGE_PURGE` and `MIMALLOC_EAGER_COMMIT_DELAY` are `deprecated_*` options
in mimalloc 3 and are ignored. None of these settings touches the cause: the faults that hurt are on file-text
buffers, and mimalloc's commit and purge policy does not decide when the kernel's copy first touches a page.

### Not tried, and why

- **Pre-faulting mimalloc's heap with `MADV_POPULATE_WRITE`** over a region handed to mimalloc
  (`mi_manage_os_memory_ex`). Two reasons:
  - `MADV_POPULATE_WRITE` is itself served under `mmap_lock` (read).
  - At the shipped 32 threads the parse is not fault-bound (4.4% of samples in faults, almost none asleep).
  
  The cost it would target exists only past ~40 threads, and the user-space touch removes that more cheaply. A
  populated region would also need sizing per project, since pre-touching memory a small project never uses raises
  its RSS.
- **Moving parse-phase heap users into the huge-page arena.** The big user is the file text, about 180 MB on vscode
  (`String` from the VFS). Moving it would change `tsrs_vfs`'s and the host's types for a cost that the touch already
  removes. The arena's own types (identifiers, symbols, node lists) are already in the arena.
- **Huge pages for only part of the heap.** The kernel's advice is per mapping, and mimalloc interleaves long- and
  short-lived pages of every thread in the same arena. Advising part of it brings back the amplification
  `notes/mem-no-thp.md` removed.

## A 40-thread cap (measured, not landed)

`PARSE_THREAD_CAP` 32 -> 40 needs no touch: at 40 threads main's binary parses as fast as with it. On 64 vCPUs, main
(32 threads) against 40 threads, from the round 4 and 5 runs (three runners). The "40 threads" row is the touch
binary at `RAYON_NUM_THREADS=40`, and the last row is main's binary at 40 threads:

| | vscode parse s (default) | vscode parse s (`--noCheck`) | vscode peak (default) | vscode peak (4 checkers) | formbricks-web peak (default) | formbricks-web peak (4 checkers) |
| --- | --- | --- | --- | --- | --- | --- |
| main | 0.122 / 0.121 / 0.127 | 0.121 / 0.124 / 0.126 | 2.791 / 2.791 / 2.798 GiB | 1.883 / 1.883 / 1.891 GiB | 2.966 / 2.961 / 2.958 GiB | 1.697 / 1.699 / 1.698 GiB |
| 40 threads | 0.116 / 0.117 / 0.118 | 0.115 / 0.114 / 0.118 | 2.792 / 2.798 / 2.792 GiB | 1.884 / 1.881 / 1.877 GiB | 2.964 / 2.959 / 2.964 GiB | 1.696 / 1.701 / 1.703 GiB |
| main binary at 40 threads (r5) | - / 0.116 / 0.119 | - / 0.119 / 0.117 | - / 2.792 / 2.786 GiB | - / 1.880 / 1.881 GiB | - / 2.964 / 2.971 GiB | - / 1.700 / 1.704 GiB |

On the two large projects this is a clean gain:

- vscode parses 4-9 ms faster at the default (-4 to -7%) and 6-10 ms faster with `--noCheck`, within 1-3 ms of the
  THP heap at 64 threads (0.115 s default, round 3).
- Peak RSS on vscode and formbricks-web is within ±0.3%.
- formbricks-web's parse does not change (0.114-0.119 s either way).

The small projects pay for it. `pr-verify.yml` with the cap at 40 and no touch (Depot run r3d1mx85np, 3 interleaved
reps per cell, release builds of main 8b3e4f4 and the change):

- Diagnostics were identical in 60 of 60 cells, and single-threaded instructions unchanged (+0.000%).
- Peak RSS went up on every project, most on the small ones. Each extra parse thread that gets files keeps its own
  partly filled 2 MiB arena chunk and mimalloc pages, whatever the project's size.

| project | peak RSS at 4 checkers | peak RSS at 32 checkers | wall at 32 checkers |
| --- | --- | --- | --- |
| vscode | 1.88 -> 1.89 GiB (+0.8%) | 2.80 -> 2.81 GiB (+0.3%) | 0.70 -> 0.67 s (-4.7%) |
| formbricks-web | 1.70 -> 1.71 GiB (+0.4%) | 2.97 -> 2.98 GiB (+0.4%) | 0.73 -> 0.74 s (+1.1%) |
| xstate-main | 299 -> 315 MiB (+5.5%) | 567 -> 575 MiB (+1.6%) | 0.13 -> 0.13 s |
| webpack | 465 -> 484 MiB (+4.2%) | 780 -> 784 MiB (+0.5%) | 0.16 -> 0.16 s |
| Compiler | 105 -> 109 MiB (+3.9%) | 203 -> 207 MiB (+1.8%) | 0.10 -> 0.10 s |
| Compiler-Unions | 110 -> 112 MiB (+1.8%) | 204 -> 206 MiB (+0.8%) | 0.16 -> 0.17 s |
| supabase-studio | 1.30 -> 1.32 GiB (+1.7%) | 2.41 -> 2.43 GiB (+0.6%) | 0.64 -> 0.64 s |
| t3code-server | 1.27 -> 1.28 GiB (+1.2%) | 2.82 -> 2.86 GiB (+1.4%) | 1.67 -> 1.64 s |

So the cap trades 4-16 MiB on small projects, with no parse gain there, for 4-9 ms of parse on vscode-sized ones. The
vscode wall-time gain is inside the 2-4% that wall time moves between identical runs on these machines. It is over the
+2% budget this work had, so it did not land.

## What would change the answer

- **Less memory per parse thread.** A thread's first arena chunk is 1 MiB of 4 KiB pages, and the next ones are 2 MiB
  huge pages, so any thread that allocates more than 1 MiB holds a whole 2 MiB page. Its mimalloc pages add more.
  With that per-thread cost cut, 48-64 parse threads plus the touch recover the THP heap's parse time (0.113-0.118 s
  against 0.115 s).
- **Fewer `mmap_lock` writers during the parse.** These are thread starts (112 rayon thread starts with a 48-thread
  `worker_pool` suggest rayon's global pool starts too) and per-chunk `madvise` + `mprotect` arena commits. Fewer writers would shorten the waits even
  without the touch. Not measured.

## Runs

All Depot runs are `perf-probe.yml` unless named. Each probe built the variants it compares on the runner.

| run | what |
| --- | --- |
| z1tnp89rnm | round 1: `no_thp` vs THP heap at 32 threads (timings, `perf stat`, `strace -c`, `perf record`), alloc profile, mimalloc options |
| pmc0p2c2xq | round 2: both builds at 32/48/64 threads, `perf record` stacks classified by kernel entry |
| rvp5flj7ks | round 3: the read-buffer touch on/off in one binary at 32/48/64 threads, THP heap at 64 |
| 7j6fhr6vsj | round 4: touch + cap 48 against main (merge base built on the runner), vscode and formbricks-web; `mmap_lock` writers |
| q1fjxdfh1m, lcf8ts20w5 | round 5 on two runners: touch + cap 48, touch at 40, main at 40 and 48 |
| r3d1mx85np | `pr-verify.yml`: cap 40 without the touch against main, 10 projects |

