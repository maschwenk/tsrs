//! tsrs-only research prototype (`spike/shared-graph`): a type graph shared between checker threads.
//!
//! `TSRS_SHARED_GRAPH`:
//! - `0` / unset: main's path, untouched.
//! - `emulate`: nothing is shared. Every checker of the type-check pass first checks the seed files, then its own
//!   files. A checker with that history is exactly a fork of the frozen seed, so this measures the design's
//!   memory, wall and exactness without building it.
//! - `1`: one seed checker checks the seed files, its graph is frozen, every pool checker is a fork of it.
//!
//! `TSRS_SHARED_GRAPH_SEED`: `spread:<permille>` (default 10) or `files:<path>` (one path per line, relative to the
//! current directory). `TSRS_SHARED_GRAPH_STATS=1`: per-checker counters on stderr after the pass (any mode).

use std::sync::OnceLock;

use tsrs_ast::SourceFile;
use tsrs_core::P;

use crate::program::Program;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Mode {
    Off,
    Emulate,
    On,
}

pub(crate) fn mode() -> Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("TSRS_SHARED_GRAPH").as_deref() {
        Ok("emulate") => Mode::Emulate,
        Ok("1" | "on") => Mode::On,
        _ => Mode::Off,
    })
}

pub(crate) fn stats_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("TSRS_SHARED_GRAPH_STATS").is_ok_and(|v| !v.is_empty() && v != "0"))
}

enum SeedRule {
    Spread(u64),
    Files(String),
}

fn seed_rule() -> SeedRule {
    match std::env::var("TSRS_SHARED_GRAPH_SEED") {
        Ok(v) if v.starts_with("files:") => SeedRule::Files(v["files:".len()..].to_string()),
        Ok(v) if v.starts_with("spread:") => SeedRule::Spread(parse_permille(&v["spread:".len()..])),
        _ => SeedRule::Spread(10_000),
    }
}

// Permille with up to three decimals, in thousandths of a permille ("2.5" -> 2500).
fn parse_permille(s: &str) -> u64 {
    let v: f64 = s.parse().unwrap_or(10.0);
    (v * 1000.0).round().max(0.0) as u64
}

/// The seed files, as positions in `files`, in program order. Only checked, non-declaration, non-leaf files: leaf
/// regions are freed after checking, so a frozen object must never point into one.
pub(crate) fn seed_positions(program: &Program, files: &[P<SourceFile>], weight: &dyn Fn(u32) -> u64) -> Vec<u32> {
    let eligible = |i: usize| {
        let f = files[i];
        !f.is_declaration_file() && !f.is_check_leaf() && !program.skip_type_checking(f, false) && weight(i as u32) > 0
    };
    match seed_rule() {
        SeedRule::Files(path) => {
            let cwd = std::env::current_dir().unwrap_or_default();
            let wanted: rustc_hash::FxHashSet<String> = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("TSRS_SHARED_GRAPH_SEED files:{path}: {e}"))
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(|l| tsrs_core::tspath::normalize_path(&cwd.join(l).to_string_lossy()))
                .collect();
            (0..files.len()).filter(|&i| eligible(i) && wanted.contains(files[i].file_name())).map(|i| i as u32).collect()
        }
        SeedRule::Spread(milli_permille) => {
            let total: u64 = (0..files.len()).map(|i| weight(i as u32)).sum();
            let budget = (total as u128 * milli_permille as u128 / 1_000_000) as u64;
            let mut candidates: Vec<usize> = (0..files.len()).filter(|&i| eligible(i)).collect();
            if candidates.is_empty() || budget == 0 {
                return Vec::new();
            }
            // The lighter half, back in program order.
            candidates.sort_by_key(|&i| (weight(i as u32), i));
            candidates.truncate(candidates.len().div_ceil(2));
            candidates.sort_unstable();
            let light: u64 = candidates.iter().map(|&i| weight(i as u32)).sum();
            let step = (light / budget.max(1)).max(1) as usize;
            let mut out = Vec::new();
            let mut sum = 0;
            for &i in candidates.iter().step_by(step) {
                if sum >= budget {
                    break;
                }
                sum += weight(i as u32);
                out.push(i as u32);
            }
            out
        }
    }
}

/// One checker's counters at a point of the pass.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Point {
    pub types: u32,
    pub sigs: u32,
    pub symbols: u32,
    pub arena: usize,
    pub cpu: f64,
}

impl Point {
    pub(crate) fn take(c: &crate::checkerpool::Checker) -> Point {
        Point {
            types: c.type_count,
            sigs: c.signature_count,
            symbols: c.symbol_count,
            arena: tsrs_core::arena::own_arena_used_bytes(),
            cpu: crate::checkerpool::thread_cpu_seconds(),
        }
    }
}

/// start, after the seed, end.
pub(crate) type Points = [Point; 3];

fn mib(b: f64) -> f64 {
    b / (1024.0 * 1024.0)
}

pub(crate) fn report(mode: Mode, seed_count: usize, points: &[Option<Points>]) {
    let mut out = format!("tsrs shared graph: mode {mode:?}, {seed_count} seed files\n");
    let mut seed_bytes = Vec::new();
    let mut seed_types = Vec::new();
    let mut seed_cpu = Vec::new();
    let (mut total_after, mut total_all, mut types_after, mut types_all) = (0.0, 0.0, 0u64, 0u64);
    for (c, p) in points.iter().enumerate() {
        let Some([s, w, e]) = p else { continue };
        let seed_b = w.arena.saturating_sub(s.arena) as f64;
        let after_b = e.arena.saturating_sub(w.arena) as f64;
        out.push_str(&format!(
            "  checker {c:>3}: seed {:>8.1} MiB {:>8} types {:>7} sigs {:>6.2} s cpu | after {:>8.1} MiB {:>8} types | total {:>8.1} MiB {:>8} types {:>6.2} s cpu\n",
            mib(seed_b),
            w.types - s.types,
            w.sigs - s.sigs,
            w.cpu - s.cpu,
            mib(after_b),
            e.types - w.types,
            mib(seed_b + after_b),
            e.types - s.types,
            e.cpu - s.cpu
        ));
        seed_bytes.push(seed_b);
        seed_types.push(w.types - s.types);
        seed_cpu.push(w.cpu - s.cpu);
        total_after += after_b;
        total_all += seed_b + after_b;
        types_after += u64::from(e.types - w.types);
        types_all += u64::from(e.types - s.types);
    }
    let n = seed_bytes.len();
    if n > 0 {
        let min_b = seed_bytes.iter().copied().fold(f64::MAX, f64::min);
        let max_b = seed_bytes.iter().copied().fold(0.0, f64::max);
        let min_t = seed_types.iter().copied().min().unwrap_or(0);
        let max_t = seed_types.iter().copied().max().unwrap_or(0);
        let tw = seed_cpu.iter().copied().fold(0.0, f64::max);
        out.push_str(&format!(
            "  summary: checkers {n}, K_t {min_t}..{max_t}, B_W {:.1}..{:.1} MiB, T_w {tw:.2} s, sum after seed {:.1} MiB / {types_after} types, sum total {:.1} MiB / {types_all} types\n",
            mib(min_b),
            mib(max_b),
            mib(total_after),
            mib(total_all)
        ));
        if mode == Mode::Emulate && n > 1 {
            // Sharing the seed would hold it once instead of n times.
            out.push_str(&format!(
                "  emulated sharing: arena {:.1} MiB -> {:.1} MiB ({:.1} MiB saved, {:.1} MiB per extra checker)\n",
                mib(total_all),
                mib(total_all - (n as f64 - 1.0) * max_b),
                mib((n as f64 - 1.0) * max_b),
                mib(max_b)
            ));
        }
    }
    eprint!("{out}");
}
