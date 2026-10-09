//! exp/discarded-elaboration (measurement only, not for merge): which node builder prints end in a kept diagnostic.
//!
//! `TSRS_ELAB=record:<dir>` records every top-level print (type_to_string, symbol_to_string, signature_to_string,
//! type_predicate_to_string, ...) of each checker with its call site, the innermost reporting relation window
//! (`check_type_related_to_ex` / `check_type_related_to_and_optionally_elaborate` with an error node) and the arena
//! and heap bytes it allocated. A print is discarded when a relater's `restore_error_state` drops it, or when its
//! window reports no diagnostic, or when none of the window's diagnostics is reachable from a diagnostic a checker
//! returned (`get_diagnostics` / `get_global_diagnostics`). Prints outside any window are never treated as
//! discarded (they are matched against the kept diagnostics' arguments for the report only). At exit it writes
//! `<dir>/summary.tsv` and one `<dir>/<key>.bin` per checker (key: the first file it checked).
//!
//! `TSRS_ELAB=replay:<dir>` (same deterministic assignment) replaces each print the record marked discarded with a
//! short stand-in string (equal iff the recorded strings were equal), without calling the node builder.
use crate::*;
use rustc_hash::{FxHashMap, FxHashSet, FxHasher};
use std::cell::Cell;
use std::hash::Hasher;
use std::panic::Location;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Off,
    Record,
    Replay,
}

static MODE: AtomicU8 = AtomicU8::new(255);
static DIR: OnceLock<String> = OnceLock::new();

#[inline]
fn mode() -> Mode {
    match MODE.load(Ordering::Relaxed) {
        0 => Mode::Off,
        1 => Mode::Record,
        2 => Mode::Replay,
        _ => init_mode(),
    }
}

#[cold]
fn init_mode() -> Mode {
    let v = std::env::var("TSRS_ELAB").unwrap_or_default();
    let (m, d) = if let Some(d) = v.strip_prefix("record:") {
        (Mode::Record, d.to_string())
    } else if let Some(d) = v.strip_prefix("replay:") {
        (Mode::Replay, d.to_string())
    } else {
        (Mode::Off, String::new())
    };
    let _ = DIR.set(d);
    MODE.store(
        match m {
            Mode::Off => 0,
            Mode::Record => 1,
            Mode::Replay => 2,
        },
        Ordering::Relaxed,
    );
    m
}

#[inline]
pub(crate) fn on() -> bool {
    mode() != Mode::Off
}

#[inline]
fn recording() -> bool {
    mode() == Mode::Record
}

pub(crate) const K_TYPE: u8 = 1;
pub(crate) const K_SYMBOL: u8 = 2;
pub(crate) const K_SIGNATURE: u8 = 3;
pub(crate) const K_PREDICATE: u8 = 4;
pub(crate) const K_OTHER: u8 = 5;

const NO_WIN: u32 = u32::MAX;

struct Rec {
    hash: u64,
    tid: u32,
    win: u32,
    kind: u8,
    restored: bool,
    print_site: &'static Location<'static>,
    restore_site: Option<&'static Location<'static>>,
    pin: Option<P<Diagnostic>>,
    soft: bool,
    arena: u64,
    heap_alloc: u64,
    heap_net: i64,
    nanos: u64,
}

struct Win {
    site: &'static Location<'static>,
    diags: Vec<P<Diagnostic>>,
    pin_cursor: usize,
}

#[derive(Clone, Copy)]
struct ReplayEntry {
    hash: u64,
    tid: u32,
    kind: u8,
    stub: bool,
}

pub struct Ledger {
    key: String,
    seq: u32,
    recs: Vec<Rec>,
    wins: Vec<Win>,
    stack: Vec<u32>,
    replay: Option<Vec<ReplayEntry>>,
}

thread_local! {
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

static REGISTRY: Mutex<Vec<Arc<Mutex<Ledger>>>> = Mutex::new(Vec::new());
static KEYS: Mutex<Option<FxHashMap<String, u32>>> = Mutex::new(None);
static KEPT_PTRS: Mutex<Option<FxHashSet<usize>>> = Mutex::new(None);
static KEPT_ARGS: Mutex<Option<FxHashSet<u64>>> = Mutex::new(None);
static ALIASES: Mutex<Option<FxHashMap<usize, usize>>> = Mutex::new(None);
static R_STUBBED: AtomicU64 = AtomicU64::new(0);
static R_REAL: AtomicU64 = AtomicU64::new(0);
static R_KIND_MISMATCH: AtomicU64 = AtomicU64::new(0);
static R_TID_MISMATCH: AtomicU64 = AtomicU64::new(0);
static R_HASH_MISMATCH: AtomicU64 = AtomicU64::new(0);
static R_OVERRUN: AtomicU64 = AtomicU64::new(0);
static R_NO_TABLE: AtomicU64 = AtomicU64::new(0);
static UNLEDGERED: AtomicU64 = AtomicU64::new(0);
static INIT_AGG: Mutex<Option<FxHashMap<String, Agg>>> = Mutex::new(None);
static ENTER_IN: AtomicU64 = AtomicU64::new(0);
static ENTER_OUT: AtomicU64 = AtomicU64::new(0);

/// Every `NodeBuilder::enter_context`: inside a hooked print or not (coverage check).
pub(crate) fn note_enter_context() {
    if DEPTH.get() > 0 { ENTER_IN.fetch_add(1, Ordering::Relaxed); } else { ENTER_OUT.fetch_add(1, Ordering::Relaxed); }
}

fn hash_str(s: &str) -> u64 {
    let mut h = FxHasher::default();
    h.write(s.as_bytes());
    h.write_u8(0xff);
    h.finish()
}

fn key_file(key: &str) -> String {
    format!("{}/{:016x}.bin", DIR.get().unwrap(), hash_str(key))
}

impl Checker {
    /// Creates this checker's ledger at its first `check_source_file`.
    pub(crate) fn elab_begin_file(&mut self, file: P<SourceFile>) {
        if self.elab.is_some() {
            return;
        }
        let mut key = file.file_name().to_string();
        {
            let mut g = KEYS.lock().unwrap();
            let m = g.get_or_insert_with(FxHashMap::default);
            let n = m.entry(key.clone()).or_insert(0);
            *n += 1;
            if *n > 1 {
                key = format!("{key}#{n}");
            }
        }
        let replay = if mode() == Mode::Replay {
            match std::fs::read(key_file(&key)) {
                Ok(bytes) => Some(
                    bytes
                        .chunks_exact(16)
                        .map(|c| ReplayEntry {
                            hash: u64::from_le_bytes(c[0..8].try_into().unwrap()),
                            tid: u32::from_le_bytes(c[8..12].try_into().unwrap()),
                            kind: c[12],
                            stub: c[13] != 0,
                        })
                        .collect(),
                ),
                Err(_) => None,
            }
        } else {
            None
        };
        let l = Arc::new(Mutex::new(Ledger { key, seq: 0, recs: Vec::new(), wins: Vec::new(), stack: Vec::new(), replay }));
        REGISTRY.lock().unwrap().push(Arc::clone(&l));
        self.elab = Some(l);
    }

    /// One print: counts it, records it (record mode) or replaces it by its stand-in (replay mode).
    pub(crate) fn elab_print(&mut self, loc: &'static Location<'static>, kind: u8, tid: u32, f: impl FnOnce(&mut Checker) -> String) -> String {
        let depth = DEPTH.get();
        fn nested<F: FnOnce(&mut Checker) -> String>(c: &mut Checker, f: F) -> String {
            DEPTH.set(DEPTH.get() + 1);
            let s = f(c);
            DEPTH.set(DEPTH.get() - 1);
            s
        }
        if depth > 0 {
            return nested(self, f);
        }
        let Some(l) = self.elab.clone() else {
            if UNLEDGERED.fetch_add(1, Ordering::Relaxed) == 0 && std::env::var_os("TSRS_ELAB_BT").is_some() {
                eprintln!("first unledgered print at {}:\n{}", site(loc), std::backtrace::Backtrace::force_capture());
            }
            if !recording() {
                return nested(self, f);
            }
            let a0 = tsrs_core::ptr::arena_used_fast();
            let (h0a, h0f) = tsrs_core::elabheap::counters();
            let t0 = std::time::Instant::now();
            let s = nested(self, f);
            let r = Rec {
                hash: 0,
                tid,
                win: NO_WIN,
                kind,
                restored: false,
                print_site: loc,
                restore_site: None,
                pin: None,
                soft: false,
                nanos: t0.elapsed().as_nanos() as u64,
                arena: tsrs_core::ptr::arena_used_fast().saturating_sub(a0) as u64,
                heap_alloc: tsrs_core::elabheap::counters().0 - h0a,
                heap_net: 0,
            };
            let _ = h0f;
            INIT_AGG.lock().unwrap().get_or_insert_with(FxHashMap::default).entry(site(loc)).or_default().add(&r);
            return s;
        };
        if mode() == Mode::Replay {
            let entry = {
                let mut g = l.lock().unwrap();
                let seq = g.seq as usize;
                g.seq += 1;
                match &g.replay {
                    None => {
                        R_NO_TABLE.fetch_add(1, Ordering::Relaxed);
                        None
                    }
                    Some(t) => {
                        let e = t.get(seq).copied();
                        if e.is_none() {
                            R_OVERRUN.fetch_add(1, Ordering::Relaxed);
                        }
                        e
                    }
                }
            };
            if let Some(e) = entry {
                if e.kind != kind {
                    R_KIND_MISMATCH.fetch_add(1, Ordering::Relaxed);
                } else {
                    if e.tid != tid {
                        R_TID_MISMATCH.fetch_add(1, Ordering::Relaxed);
                    }
                    if e.stub {
                        R_STUBBED.fetch_add(1, Ordering::Relaxed);
                        return format!("\u{1}{:016x}", e.hash);
                    }
                }
            }
            R_REAL.fetch_add(1, Ordering::Relaxed);
            let s = nested(self, f);
            if let Some(e) = entry {
                if e.kind == kind && hash_str(&s) != e.hash {
                    R_HASH_MISMATCH.fetch_add(1, Ordering::Relaxed);
                }
            }
            return s;
        }
        // Record.
        let a0 = tsrs_core::ptr::arena_used_fast();
        let (h0a, h0f) = tsrs_core::elabheap::counters();
        let t0 = std::time::Instant::now();
        let s = nested(self, f);
        let nanos = t0.elapsed().as_nanos() as u64;
        let a1 = tsrs_core::ptr::arena_used_fast();
        let (h1a, h1f) = tsrs_core::elabheap::counters();
        let mut g = l.lock().unwrap();
        g.seq += 1;
        let win = g.stack.last().copied().unwrap_or(NO_WIN);
        g.recs.push(Rec {
            hash: hash_str(&s),
            tid,
            win,
            kind,
            restored: false,
            print_site: loc,
            restore_site: None,
            pin: None,
            soft: false,
            arena: a1.saturating_sub(a0) as u64,
            heap_alloc: h1a - h0a,
            heap_net: (h1a - h0a) as i64 - (h1f - h0f) as i64,
            nanos,
        });
        s
    }

    /// Opens a reporting window (record mode); returns its token for `elab_win_close`.
    pub(crate) fn elab_win_open(&mut self, loc: &'static Location<'static>) -> u32 {
        if !recording() || DEPTH.get() > 0 {
            return NO_WIN;
        }
        let Some(l) = &self.elab else { return NO_WIN };
        let mut g = l.lock().unwrap();
        let id = g.wins.len() as u32;
        let cursor = g.recs.len();
        g.wins.push(Win { site: loc, diags: Vec::new(), pin_cursor: cursor });
        g.stack.push(id);
        id
    }

    pub(crate) fn elab_win_close(&mut self, token: u32) {
        if token == NO_WIN {
            return;
        }
        let Some(l) = &self.elab else { return };
        let mut g = l.lock().unwrap();
        let top = g.stack.pop();
        debug_assert_eq!(top, Some(token));
    }

    /// A diagnostic was reported (to the collection or an output vector) while the innermost window was open.
    pub(crate) fn elab_note_diag(&mut self, d: P<Diagnostic>) {
        if !recording() {
            return;
        }
        let Some(l) = &self.elab else { return };
        let mut g = l.lock().unwrap();
        if let Some(&w) = g.stack.last() {
            let n = g.recs.len();
            let cursor = g.wins[w as usize].pin_cursor;
            for r in &mut g.recs[cursor..] {
                if r.win == w && !r.restored && r.pin.is_none() {
                    r.pin = Some(d);
                }
            }
            let win = &mut g.wins[w as usize];
            win.pin_cursor = n;
            win.diags.push(d);
        }
    }

    /// The print count, saved with a relater's error state.
    pub(crate) fn elab_mark(&mut self) -> u32 {
        if !recording() {
            return 0;
        }
        self.elab.as_ref().map_or(0, |l| l.lock().unwrap().recs.len() as u32)
    }

    /// A relater restored its error state: the prints of the innermost window since `mark` are thrown away.
    pub(crate) fn elab_restore(&mut self, mark: u32, loc: &'static Location<'static>) {
        if !recording() {
            return;
        }
        let Some(l) = &self.elab else { return };
        let mut g = l.lock().unwrap();
        // A relater outside any reporting window (report_errors off) has no chain to drop; prints there fed side
        // diagnostics (c.error during lazy resolution), which a restore does not touch.
        let Some(w) = g.stack.last().copied() else { return };
        for r in &mut g.recs[mark as usize..] {
            if r.win == w && !r.restored && r.pin.is_none() && !r.soft {
                r.restored = true;
                r.restore_site = Some(loc);
            }
        }
    }

    /// The collection dropped `dup` as equal to `existing` (the comparison reads the formatted text).
    pub(crate) fn elab_note_alias(dup: P<Diagnostic>, existing: P<Diagnostic>) {
        if !recording() || dup.addr() == existing.addr() {
            return;
        }
        ALIASES.lock().unwrap().get_or_insert_with(FxHashMap::default).insert(dup.addr(), existing.addr());
    }

    /// A restore whose chain may be put back: its prints are only flagged (never treated as discarded).
    pub(crate) fn elab_restore_soft(&mut self, mark: u32, loc: &'static Location<'static>) {
        if !recording() {
            return;
        }
        let Some(l) = &self.elab else { return };
        let mut g = l.lock().unwrap();
        let Some(w) = g.stack.last().copied() else { return };
        for r in &mut g.recs[mark as usize..] {
            if r.win == w && !r.restored && r.pin.is_none() && !r.soft {
                r.soft = true;
                r.restore_site = Some(loc);
            }
        }
    }

    /// Diagnostics a checker returned: everything reachable from them is kept.
    pub(crate) fn elab_keep(diags: &[P<Diagnostic>]) {
        if !recording() {
            return;
        }
        let mut ptrs = KEPT_PTRS.lock().unwrap();
        let ptrs = ptrs.get_or_insert_with(FxHashSet::default);
        let mut args = KEPT_ARGS.lock().unwrap();
        let args = args.get_or_insert_with(FxHashSet::default);
        let mut stack: Vec<P<Diagnostic>> = diags.to_vec();
        while let Some(d) = stack.pop() {
            if !ptrs.insert(d.addr()) {
                continue;
            }
            for a in d.message_args() {
                args.insert(hash_str(a));
            }
            stack.extend(d.message_chain().iter().copied());
            stack.extend(d.related_information().iter().copied());
        }
    }

    /// At exit: the replay counters, or the record's classification, summary and per-checker tables.
    pub fn elab_finish() {
        match mode() {
            Mode::Off => {}
            Mode::Replay => {
                eprintln!(
                    "tsrs elab replay: stubbed {} real {} kind-mismatch {} tid-mismatch {} hash-mismatch(real) {} overrun {} no-table {} unledgered {}",
                    R_STUBBED.load(Ordering::Relaxed),
                    R_REAL.load(Ordering::Relaxed),
                    R_KIND_MISMATCH.load(Ordering::Relaxed),
                    R_TID_MISMATCH.load(Ordering::Relaxed),
                    R_HASH_MISMATCH.load(Ordering::Relaxed),
                    R_OVERRUN.load(Ordering::Relaxed),
                    R_NO_TABLE.load(Ordering::Relaxed),
                    UNLEDGERED.load(Ordering::Relaxed),
                );
            }
            Mode::Record => finish_record(),
        }
    }
}

fn site(l: &'static Location<'static>) -> String {
    format!("{}:{}", l.file().rsplit('/').next().unwrap_or(l.file()), l.line())
}

#[derive(Default, Clone, Copy)]
struct Agg {
    n: u64,
    arena: u64,
    heap_alloc: u64,
    heap_net: i64,
    nanos: u64,
}

impl Agg {
    fn add(&mut self, r: &Rec) {
        self.n += 1;
        self.arena += r.arena;
        self.heap_alloc += r.heap_alloc;
        self.heap_net += r.heap_net;
        self.nanos += r.nanos;
    }
}

fn finish_record() {
    let dir = DIR.get().unwrap().clone();
    let _ = std::fs::create_dir_all(&dir);
    let ptrs = KEPT_PTRS.lock().unwrap().take().unwrap_or_default();
    let args = KEPT_ARGS.lock().unwrap().take().unwrap_or_default();
    let aliases = ALIASES.lock().unwrap().take().unwrap_or_default();
    let ledgers = REGISTRY.lock().unwrap().clone();
    // 0 dropped, 1 kept, 2 dropped as a duplicate of a kept one
    let fate = |d: P<Diagnostic>| -> u8 {
        if ptrs.contains(&d.addr()) {
            1
        } else if aliases.get(&d.addr()).is_some_and(|e| ptrs.contains(e)) {
            2
        } else {
            0
        }
    };
    // (category, print site, entry site, restore site) -> totals
    let mut rows: FxHashMap<(&'static str, String, String, String), Agg> = FxHashMap::default();
    let mut per_checker = String::new();
    let dbg = std::env::var("TSRS_ELAB_DEBUG").ok().map(|v| hash_str(&v));
    for l in &ledgers {
        let g = l.lock().unwrap();
        let win_fate: Vec<u8> = g.wins.iter().map(|w| w.diags.iter().map(|&d| fate(d)).max().unwrap_or(0)).collect();
        let mut bytes = Vec::with_capacity(g.recs.len() * 16);
        let mut tot = Agg::default();
        let mut disc = Agg::default();
        for r in &g.recs {
            let matched = args.contains(&r.hash);
            let (cat, stub) = if r.restored {
                ("discarded:restored", true)
            } else if r.win == NO_WIN {
                (if matched { "outside:matched" } else { "outside:unmatched" }, false)
            } else if let Some(d) = r.pin {
                // The pin is approximate (every pending print of the window goes to the next diagnostic noted in it),
                // so a print counts as kept when either its pin or any diagnostic of its window is kept.
                match fate(d).max(win_fate[r.win as usize]) {
                    1 => (if matched { "kept:matched" } else { "kept:unmatched" }, false),
                    2 => ("kept:dedupe", false),
                    _ => ("discarded:diag-dropped", true),
                }
            } else if win_fate[r.win as usize] == 1 {
                (if r.soft { "kept:resurrectable" } else if matched { "kept:matched" } else { "kept:unmatched" }, false)
            } else if win_fate[r.win as usize] == 2 {
                ("kept:dedupe", false)
            } else if !g.wins[r.win as usize].diags.is_empty() {
                ("discarded:diag-dropped", true)
            } else {
                ("discarded:no-diag", true)
            };
            if let Some(dbg) = &dbg {
                if r.hash == *dbg {
                    eprintln!("elab debug: {} seq? cat {cat} print {} win {} restore {:?} pin {:?} key {}", r.tid, site(r.print_site), if r.win == NO_WIN { "-".into() } else { site(g.wins[r.win as usize].site) }, r.restore_site.map(site), r.pin.map(|d| (d.code(), fate(d))), g.key);
                }
            }
            tot.add(r);
            if stub {
                disc.add(r);
            }
            let entry = if r.win == NO_WIN { "-".to_string() } else { site(g.wins[r.win as usize].site) };
            let restore = r.restore_site.map_or_else(|| "-".to_string(), site);
            rows.entry((cat, site(r.print_site), entry, restore)).or_default().add(r);
            bytes.extend_from_slice(&r.hash.to_le_bytes());
            bytes.extend_from_slice(&r.tid.to_le_bytes());
            bytes.push(r.kind);
            bytes.push(u8::from(stub));
            bytes.extend_from_slice(&[0, 0]);
        }
        std::fs::write(key_file(&g.key), &bytes).expect("write elab table");
        per_checker.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            g.key, tot.n, tot.arena, tot.heap_alloc, tot.nanos, disc.n, disc.arena, disc.nanos
        ));
    }
    for (k, a) in INIT_AGG.lock().unwrap().take().unwrap_or_default() {
        *rows.entry(("init:before-first-file", k, "-".to_string(), "-".to_string())).or_default() = a;
    }
    let mut out = String::from("category\tprint_site\tentry_site\trestore_site\tcount\tarena\theap_alloc\theap_net\tnanos\n");
    let mut rows: Vec<_> = rows.into_iter().collect();
    rows.sort_by(|a, b| b.1.arena.cmp(&a.1.arena));
    for ((cat, p, e, r), a) in rows {
        out.push_str(&format!("{cat}\t{p}\t{e}\t{r}\t{}\t{}\t{}\t{}\t{}\n", a.n, a.arena, a.heap_alloc, a.heap_net, a.nanos));
    }
    std::fs::write(format!("{dir}/summary.tsv"), out).expect("write elab summary");
    std::fs::write(
        format!("{dir}/checkers.tsv"),
        format!("key\tprints\tarena\theap_alloc\tnanos\tdisc_prints\tdisc_arena\tdisc_nanos\n{per_checker}"),
    )
    .expect("write elab checkers");
    eprintln!("tsrs elab record: {} checkers, unledgered prints {}, kept diagnostics reachable {}, node builder contexts in prints {} outside {}", ledgers.len(), UNLEDGERED.load(Ordering::Relaxed), ptrs.len(), ENTER_IN.load(Ordering::Relaxed), ENTER_OUT.load(Ordering::Relaxed));
}
