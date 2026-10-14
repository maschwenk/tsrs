// tsrs-only, experiment: rules that keep a thief from taking a file (TSRS_STEAL_GATE). Stealing moves not-yet-started
// files between checkers (checkerpool.rs `queues_next`, notes/perf-checker-stealing.md); a stolen file can cost its
// thief far more than its owner when it is the thief's first file to need a large type graph its owner builds anyway
// (notes/perf-icount-round4.md). Each rule here is a guess at when that happens. None is on by default. All are
// output-safe: no rule changes what a checker computes for a file, only which checker checks it and when.
//
// On t3code-server only `hub` removed most of that premium without costing vscode, webpack, nuxt or mui-docs more
// than noise, and its signal (a file importing nearly the whole program) exists on t3code-server alone: the other rules
// either did not help there or moved the slowest checker on the other projects. The commit that added this file has
// the measurements.
//
// TSRS_STEAL_GATE is a comma-separated list of:
// - `inflight[:<x>]`: no checker starts a file stolen from a queue while another thief runs a file it stole from that
//   queue for longer than x (default 10) times its static weight at the pass's rate so far: that thief is probably
//   building a type graph the queue's other files need, and will take them more cheaply once it is done.
// - `inflight-owner[:<x>]`: the same, also counting the queue's owner's current file.
// - `minleft:<k>`: steal only from a queue with more than k files left.
// - `tail:<d>`: steal only from a queue with more than 1/d of an average checker's share of weight left.
// - `dir`: a thief takes a file only if no other thief took a file of the same directory in this pass.
// - `held`: a thief prefers the queue whose next file is in a directory it has checked a file of.
// - `hub[:<pct>[:<max>]]`: files whose transitive import closure has at least pct% (default 90) of the largest
//   closure's files go first in their owner's queue and are never stolen, if they are at most max% (default 2) of the
//   pass's files, unchecked declaration files included (else no file is a hub). `hubfirst`: they only go first;
//   `hubkeep`: they are only never stolen.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

use rustc_hash::FxHashMap;
use tsrs_ast::SourceFile;
use tsrs_core::P;

use crate::program::Program;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct GateConfig {
    inflight: f64,
    inflight_owner: bool,
    min_left: u64,
    tail: u64,
    dir_claim: bool,
    held: bool,
    hub_percent: u64,
    hub_max_percent: u64,
    // `hubfirst`: hubs go first but may be stolen; `hubkeep`: hubs are never stolen but keep their place.
    hub_first: bool,
    hub_keep: bool,
}

pub(crate) fn config() -> Option<&'static GateConfig> {
    static VALUE: OnceLock<Option<GateConfig>> = OnceLock::new();
    VALUE
        .get_or_init(|| {
            // Stacked bench build: `hub` is the default; TSRS_STEAL_GATE=none (or empty) turns every gate off.
            let spec = std::env::var("TSRS_STEAL_GATE").unwrap_or_else(|_| "hub".to_string());
            if spec.is_empty() || spec == "none" {
                return None;
            }
            let mut cfg = GateConfig::default();
            for rule in spec.split(',') {
                let mut parts = rule.split(':');
                let name = parts.next().unwrap_or("");
                let arg = |parts: &mut std::str::Split<'_, char>| parts.next().and_then(|v| v.parse::<f64>().ok());
                match name {
                    "inflight" | "inflight-owner" => {
                        cfg.inflight = arg(&mut parts).unwrap_or(10.0);
                        cfg.inflight_owner = name == "inflight-owner";
                    }
                    "minleft" => cfg.min_left = arg(&mut parts).unwrap_or(8.0) as u64,
                    "tail" => cfg.tail = arg(&mut parts).unwrap_or(16.0) as u64,
                    "dir" => cfg.dir_claim = true,
                    "held" => cfg.held = true,
                    "hub" | "hubfirst" | "hubkeep" => {
                        cfg.hub_percent = arg(&mut parts).unwrap_or(90.0) as u64;
                        cfg.hub_max_percent = arg(&mut parts).unwrap_or(2.0) as u64;
                        cfg.hub_first = name != "hubkeep";
                        cfg.hub_keep = name != "hubfirst";
                    }
                    _ => panic!("TSRS_STEAL_GATE: unknown rule {rule:?}"),
                }
            }
            Some(cfg)
        })
        .as_ref()
}

pub(crate) enum Verdict {
    Allow,
    // The queue may become stealable again (another checker's file ends, or the claimed file is taken).
    Wait,
    // The queue stays closed to this thief for the rest of the pass.
    Closed,
}

// One type-check pass's gate state.
pub(crate) struct PassGate {
    cfg: GateConfig,
    start: Instant,
    active: u64,
    // Weight of the files finished so far, for the pass's rate (seconds per weight unit).
    done_weight: AtomicU64,
    // Per checker: (microseconds from `start` to its current file's start + 1) << 16 | queue it took it from, 0 when idle.
    current: Vec<AtomicU64>,
    current_weight: Vec<AtomicU64>,
    // Directory index per queue position (u32::MAX for none: split-file pieces).
    dirs: Vec<u32>,
    // Per directory: 1 + the thief that took a file of it first (`dir`), 0 for none.
    claims: Vec<AtomicU32>,
    // Per checker, a bit per directory it has checked a file of (`held`).
    held: Vec<Vec<AtomicU64>>,
    hubs: Vec<bool>,
    share: u64,
}

impl PassGate {
    pub(crate) fn new(cfg: &GateConfig, program: &Program, files: &[P<SourceFile>], checker_count: usize, active: usize, total_weight: u64) -> PassGate {
        let mut dir_index: FxHashMap<&str, u32> = FxHashMap::default();
        let dirs: Vec<u32> = if cfg.dir_claim || cfg.held {
            files
                .iter()
                .map(|f| {
                    let name = f.file_name();
                    let dir = name.rfind('/').map_or("", |k| &name[..k]);
                    let next = dir_index.len() as u32;
                    *dir_index.entry(dir).or_insert(next)
                })
                .collect()
        } else {
            Vec::new()
        };
        let dir_count = dir_index.len();
        let hubs = if cfg.hub_percent > 0 { tsrs_core::phases::time("Checkers: steal gate hubs", || hub_positions(program, files, cfg.hub_percent, cfg.hub_max_percent)) } else { Vec::new() };
        PassGate {
            cfg: *cfg,
            start: Instant::now(),
            active: active as u64,
            done_weight: AtomicU64::new(0),
            current: std::iter::repeat_with(|| AtomicU64::new(0)).take(checker_count).collect(),
            current_weight: std::iter::repeat_with(|| AtomicU64::new(0)).take(checker_count).collect(),
            dirs,
            claims: if cfg.dir_claim { std::iter::repeat_with(|| AtomicU32::new(0)).take(dir_count).collect() } else { Vec::new() },
            held: if cfg.held {
                std::iter::repeat_with(|| std::iter::repeat_with(|| AtomicU64::new(0)).take(dir_count.div_ceil(64)).collect()).take(checker_count).collect()
            } else {
                Vec::new()
            },
            hubs,
            share: total_weight / active.max(1) as u64,
        }
    }

    // Moves the hubs of one checker's queue to its front, heaviest first.
    pub(crate) fn hubs_first(&self, positions: &mut Vec<u32>, weight: impl Fn(u32) -> u64) {
        if self.hubs.is_empty() || !self.cfg.hub_first {
            return;
        }
        let (mut hubs, rest): (Vec<u32>, Vec<u32>) = positions.iter().partition(|&&i| self.is_hub(i));
        if hubs.is_empty() {
            return;
        }
        hubs.sort_by_key(|&i| std::cmp::Reverse(weight(i)));
        hubs.extend(rest);
        *positions = hubs;
    }

    fn is_hub(&self, position: u32) -> bool {
        self.hubs.get(position as usize).copied().unwrap_or(false)
    }

    fn dir(&self, position: u32) -> Option<u32> {
        self.dirs.get(position as usize).copied().filter(|&d| d != u32::MAX)
    }

    fn micros(&self) -> u64 {
        self.start.elapsed().as_micros() as u64
    }

    // Checker `me` starts the file at `position`, taken from `queue`'s queue.
    pub(crate) fn begin(&self, me: usize, queue: usize, position: u32, weight: u64, stolen: bool) {
        self.current_weight[me].store(weight, Ordering::Relaxed);
        // Heuristic state, read by other thieves only to choose; nothing is published through it.
        self.current[me].store(((self.micros() + 1) << 16) | queue as u64, Ordering::Relaxed);
        if let Some(d) = self.dir(position) {
            if stolen && self.cfg.dir_claim {
                // As above: only records who came first.
                let _ = self.claims[d as usize].compare_exchange(0, me as u32 + 1, Ordering::Relaxed, Ordering::Relaxed);
            }
            if self.cfg.held {
                // Only `me` sets its bits; others never read them.
                self.held[me][d as usize / 64].fetch_or(1 << (d % 64), Ordering::Relaxed);
            }
        }
    }

    pub(crate) fn end(&self, me: usize) {
        // Written only by `me` (in `begin`).
        let w = self.current_weight[me].load(Ordering::Relaxed);
        // A statistic; a stale value only moves the over-run threshold.
        self.done_weight.fetch_add(w, Ordering::Relaxed);
        // Heuristic state, as in `begin`.
        self.current[me].store(0, Ordering::Relaxed);
    }

    // Whether thief `me` may take the back file of `victim`'s queue (`back`: its position, `files_left`, `weight_left`).
    pub(crate) fn verdict(&self, me: usize, victim: usize, back: Option<u32>, files_left: u64, weight_left: u64) -> Verdict {
        if self.cfg.min_left > 0 && files_left <= self.cfg.min_left {
            return Verdict::Closed;
        }
        if self.cfg.tail > 0 && weight_left * self.cfg.tail <= self.share {
            return Verdict::Closed;
        }
        if let Some(back) = back {
            // With `hub` the hubs are at the front of their queue, so a hub at the back means only hubs are left.
            if self.cfg.hub_keep && self.is_hub(back) {
                return Verdict::Closed;
            }
            if self.cfg.dir_claim {
                if let Some(d) = self.dir(back) {
                    // See `begin`.
                    let claim = self.claims[d as usize].load(Ordering::Relaxed);
                    if claim != 0 && claim != me as u32 + 1 {
                        return Verdict::Wait;
                    }
                }
            }
        }
        if self.cfg.inflight > 0.0 && self.overrunning_on(me, victim) {
            return Verdict::Wait;
        }
        Verdict::Allow
    }

    // Another checker runs a file of `victim`'s queue far longer than its weight predicts.
    fn overrunning_on(&self, me: usize, victim: usize) -> bool {
        let now = self.micros();
        let done = self.done_weight.load(Ordering::Relaxed).max(1);
        // Microseconds per weight unit per checker so far.
        let rate = (now * self.active) as f64 / done as f64;
        self.current.iter().enumerate().any(|(c, cur)| {
            // See `begin`.
            let v = cur.load(Ordering::Relaxed);
            if c == me || v == 0 || (v & 0xffff) as usize != victim || (c == victim && !self.cfg.inflight_owner) {
                return false;
            }
            let elapsed = now.saturating_sub((v >> 16) - 1);
            // See `begin`.
            let expected = self.current_weight[c].load(Ordering::Relaxed) as f64 * rate;
            elapsed > 1000 && elapsed as f64 > self.cfg.inflight * expected
        })
    }

    // Whether `me` has checked a file of the directory of `position` (`held`).
    pub(crate) fn holds(&self, me: usize, position: Option<u32>) -> bool {
        match position.and_then(|p| self.dir(p)) {
            // See `begin`.
            Some(d) => self.held[me][d as usize / 64].load(Ordering::Relaxed) & (1 << (d % 64)) != 0,
            None => false,
        }
    }

    pub(crate) fn prefers_held(&self) -> bool {
        self.cfg.held
    }
}

// The positions of `files` whose transitive import closure (program files reachable by resolved imports) has at
// least `percent`% of the largest closure's files; none when more than `max_percent`% of the files would be.
fn hub_positions(program: &Program, files: &[P<SourceFile>], percent: u64, max_percent: u64) -> Vec<bool> {
    let index: FxHashMap<P<SourceFile>, usize> = program.files.iter().enumerate().map(|(i, &f)| (f, i)).collect();
    // Computed by the locality assignment from its import lists (`note_import_targets`), else here.
    let sizes = CLOSURE_SIZES.get().cloned().flatten().or_else(|| {
        let edges: Vec<Vec<u32>> = crate::checkerpool::get_import_targets(program, true).into_iter().map(|t| t.into_iter().map(|i| i as u32).collect()).collect();
        closure_sizes(&edges)
    });
    let Some(sizes) = sizes.filter(|s| s.len() == program.files.len()) else {
        return vec![false; files.len()];
    };
    let size_of = |f: &P<SourceFile>| index.get(f).map_or(0, |&i| sizes[i]);
    let largest = files.iter().map(size_of).max().unwrap_or(0) as u64;
    let hubs: Vec<bool> = files.iter().map(|f| size_of(f) > 1 && size_of(f) as u64 * 100 >= largest * percent).collect();
    let count = hubs.iter().filter(|&&h| h).count() as u64;
    if count * 100 > files.len() as u64 * max_percent {
        tsrs_core::phases::count("Checkers: steal gate hubs (over the cap)", count);
        return vec![false; files.len()];
    }
    tsrs_core::phases::count("Checkers: steal gate hubs", count);
    hubs
}

static CLOSURE_SIZES: OnceLock<Option<Vec<u32>>> = OnceLock::new();

// Called by the locality assignment with its import lists (program file index -> imported file indices).
pub(crate) fn note_import_targets(targets: &[Vec<usize>]) {
    if config().is_some_and(|c| c.hub_percent > 0) {
        let edges: Vec<Vec<u32>> = targets.iter().map(|t| t.iter().map(|&i| i as u32).collect()).collect();
        let _ = CLOSURE_SIZES.set(tsrs_core::phases::time("Checkers: steal gate closures", || closure_sizes(&edges)));
    }
}

// The number of nodes reachable from each node (itself included): strongly connected components (iterative Tarjan,
// which numbers a component after every component it reaches), then one bit set of reached components per component.
// None when the bit sets would take more than 256 MiB.
fn closure_sizes(edges: &[Vec<u32>]) -> Option<Vec<u32>> {
    let n = edges.len();
    let mut index = vec![u32::MAX; n];
    let mut low = vec![0u32; n];
    let mut on_stack = vec![false; n];
    let mut comp = vec![u32::MAX; n];
    let mut stack: Vec<u32> = Vec::new();
    let mut calls: Vec<(u32, usize)> = Vec::new();
    let (mut counter, mut comps) = (0u32, 0u32);
    for root in 0..n as u32 {
        if index[root as usize] != u32::MAX {
            continue;
        }
        index[root as usize] = counter;
        low[root as usize] = counter;
        counter += 1;
        stack.push(root);
        on_stack[root as usize] = true;
        calls.push((root, 0));
        while let Some(&mut (v, ref mut next)) = calls.last_mut() {
            let v = v as usize;
            if let Some(&w) = edges[v].get(*next) {
                *next += 1;
                let w = w as usize;
                if index[w] == u32::MAX {
                    index[w] = counter;
                    low[w] = counter;
                    counter += 1;
                    stack.push(w as u32);
                    on_stack[w] = true;
                    calls.push((w as u32, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
                continue;
            }
            calls.pop();
            if let Some(&(u, _)) = calls.last() {
                low[u as usize] = low[u as usize].min(low[v]);
            }
            if low[v] == index[v] {
                while let Some(w) = stack.pop() {
                    on_stack[w as usize] = false;
                    comp[w as usize] = comps;
                    if w as usize == v {
                        break;
                    }
                }
                comps += 1;
            }
        }
    }
    let c = comps as usize;
    let words = c.div_ceil(64);
    if c * words * 8 > 256 << 20 {
        return None;
    }
    let mut members: Vec<u32> = vec![0; c];
    for &k in &comp {
        members[k as usize] += 1;
    }
    let mut successors: Vec<Vec<u32>> = vec![Vec::new(); c];
    for (v, targets) in edges.iter().enumerate() {
        for &w in targets {
            if comp[w as usize] != comp[v] {
                successors[comp[v] as usize].push(comp[w as usize]);
            }
        }
    }
    let large: Vec<u32> = (0..comps).filter(|&k| members[k as usize] > 1).collect();
    let mut bits = vec![0u64; c * words];
    let mut comp_size = vec![0u32; c];
    for k in 0..c {
        let (done, rest) = bits.split_at_mut(k * words);
        let mine = &mut rest[..words];
        mine[k / 64] |= 1 << (k % 64);
        for &d in &successors[k] {
            // Components reach only lower-numbered ones, already complete.
            let theirs = &done[d as usize * words..(d as usize + 1) * words];
            for (a, b) in mine.iter_mut().zip(theirs) {
                *a |= b;
            }
        }
        let reached: u32 = mine.iter().map(|w| w.count_ones()).sum();
        let extra: u32 = large.iter().filter(|&&d| mine[d as usize / 64] & (1 << (d % 64)) != 0).map(|&d| members[d as usize] - 1).sum();
        comp_size[k] = reached + extra;
    }
    Some(comp.iter().map(|&k| comp_size[k as usize]).collect())
}

#[cfg(test)]
mod tests {
    use super::closure_sizes;

    /// Each node counts itself and every node it reaches; the nodes of a cycle reach each other.
    #[test]
    fn closure_sizes_count_reachable_nodes_through_cycles() {
        // 0 -> 1 -> 2 -> 1 (cycle {1, 2}), 2 -> 3, 4 -> 0, 5 alone.
        let edges = vec![vec![1], vec![2], vec![1, 3], vec![], vec![0], vec![]];
        assert_eq!(closure_sizes(&edges), Some(vec![4, 3, 3, 1, 5, 1]));
    }
}
