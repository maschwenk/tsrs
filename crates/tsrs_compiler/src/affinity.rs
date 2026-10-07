// tsrs-only (notes/perf-clustered-assignment.md): module affinity for the locality assignment.
//
// Each checker builds its own types for every declaration its files use, so two checkers whose files import the same
// module (a zod schema package, `lucide-react`, a workspace component, an Effect service) both resolve and
// instantiate it. The locality assignment keeps directory subtrees together and FENNEL counts the import edges between
// groups, but an import edge to a file outside the groups (a library declaration file that is not type checked, and so
// carries no weight) counted nothing, and every edge counted the same whatever the size of the module it leads to.
//
// Here a group's modules are its own files and the files they import, each worth its node count^GAMMA. When FENNEL
// places a group, each checker gets an affinity: the worth of the group's modules that groups already placed on the
// checker also have, scaled to `MU * (group's import edges + 1) / (worth of all the group's modules)`. The scale keeps
// the term at most MU times the group's own import affinity, on every project: an unscaled one (per node) needs a
// different factor per project, and a strong one costs more than it saves (it scatters a project's own files, whose
// library generics instantiated with project types are most of what an extra checker repeats on the Effect project).
//
// FENNEL places groups one at a time, heaviest first, so a group placed early cannot see the groups that will share its
// modules. After it, `refine` moves single groups (lightest first) to the checker where the group's modules cost least:
// the move's gain is the worth of the modules only that group holds on its checker, minus the worth of its modules the
// destination does not hold yet. A move must keep the destination under FENNEL's cap of 101% of an average load. The
// measured cost follows that sum (the worth of every module a checker's files are or import, summed over checkers):
// over 100 assignments of five projects, its rank correlation with the instructions of the check was 0.73-0.91 on four
// of them, against 0.5-0.8 for the import edges FENNEL cuts (notes/perf-clustered-assignment.md).
//
// `TSRS_MODULE_AFFINITY=<mu>` (read once) sets MU, `0`/`off` turns this off (the plain locality assignment);
// `TSRS_MODULE_AFFINITY_GAMMA=<g>` sets GAMMA and `TSRS_MODULE_AFFINITY_PASSES=<n>` the most refinement passes, for
// experiments.

use std::sync::OnceLock;

use tsrs_ast::SourceFile;
use tsrs_core::P;

const MU: f64 = 1.0;
const GAMMA: f64 = 0.5;
const MAX_PASSES: usize = 10;

struct Config {
    mu: f64,
    gamma: f64,
    passes: usize,
}

fn config() -> &'static Config {
    static CONFIG: OnceLock<Config> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let mu = match std::env::var("TSRS_MODULE_AFFINITY").ok().as_deref().map(str::trim) {
            None | Some("") => MU,
            Some("0" | "off") => 0.0,
            Some(v) => v.parse::<f64>().ok().filter(|m| m.is_finite() && *m >= 0.0).unwrap_or_else(|| panic!("TSRS_MODULE_AFFINITY: expected a number >= 0 or off, got {v:?}")),
        };
        let gamma = std::env::var("TSRS_MODULE_AFFINITY_GAMMA").ok().and_then(|v| v.parse::<f64>().ok()).filter(|g| g.is_finite()).unwrap_or(GAMMA);
        let passes = std::env::var("TSRS_MODULE_AFFINITY_PASSES").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(MAX_PASSES);
        Config { mu, gamma, passes }
    })
}

pub(crate) struct ModuleAffinity {
    // Per program file: its worth (node count^GAMMA).
    worth: Vec<f64>,
    // The modules of group g are `modules[starts[g]..starts[g + 1]]` (program file indices, no duplicates).
    starts: Vec<usize>,
    modules: Vec<u32>,
    // Per group: the worth of its modules, and MU * (import edges + 1) / that worth.
    total: Vec<f64>,
    scale: Vec<f64>,
    // Per program file, a bit per checker that a placed group with the file among its modules went to.
    words: usize,
    holders: Vec<u64>,
    // Scratch: the affinity of the group being placed, per checker.
    affinity: Vec<f64>,
}

impl ModuleAffinity {
    // None when the term is off. `targets[i]` are the files program file `i` imports (resolved, in the program);
    // `group_of_file[i]` is its locality group, usize::MAX for files outside the groups (unchecked ones);
    // `group_adjacency` is FENNEL's group graph.
    pub(crate) fn new(
        files: &[P<SourceFile>],
        targets: &[Vec<usize>],
        group_of_file: &[usize],
        group_adjacency: &[Vec<usize>],
        checker_count: usize,
    ) -> Option<ModuleAffinity> {
        let config = config();
        if config.mu <= 0.0 {
            return None;
        }
        let worth: Vec<f64> = files.iter().map(|f| (f.node_count.get().max(1) as f64).powf(config.gamma)).collect();
        let group_count = group_adjacency.len();
        // Group members by counting sort, in file order.
        let mut starts = vec![0usize; group_count + 1];
        for &g in group_of_file.iter().filter(|&&g| g != usize::MAX) {
            starts[g + 1] += 1;
        }
        for g in 0..group_count {
            starts[g + 1] += starts[g];
        }
        let mut members = vec![0u32; starts[group_count]];
        let mut next = starts.clone();
        for (i, &g) in group_of_file.iter().enumerate().filter(|&(_, &g)| g != usize::MAX) {
            members[next[g]] = i as u32;
            next[g] += 1;
        }
        let mut stamp = vec![usize::MAX; files.len()];
        let mut modules: Vec<u32> = Vec::with_capacity(members.len() * 4);
        let mut module_starts = Vec::with_capacity(group_count + 1);
        let mut totals = Vec::with_capacity(group_count);
        let mut scale = Vec::with_capacity(group_count);
        module_starts.push(0);
        for g in 0..group_count {
            let start = modules.len();
            for &i in &members[starts[g]..starts[g + 1]] {
                for m in std::iter::once(i as usize).chain(targets[i as usize].iter().copied()) {
                    if stamp[m] != g {
                        stamp[m] = g;
                        modules.push(m as u32);
                    }
                }
            }
            // In file order, so that the sums below do not depend on the order of the import lists.
            modules[start..].sort_unstable();
            let total: f64 = modules[start..].iter().map(|&m| worth[m as usize]).sum();
            module_starts.push(modules.len());
            totals.push(total);
            scale.push(if total > 0.0 { config.mu * (group_adjacency[g].len() + 1) as f64 / total } else { 0.0 });
        }
        let words = checker_count.div_ceil(64);
        Some(ModuleAffinity {
            worth,
            starts: module_starts,
            modules,
            total: totals,
            scale,
            words,
            holders: vec![0; files.len() * words],
            affinity: vec![0.0; checker_count],
        })
    }

    // The affinity of `group` to each checker, given the groups placed so far.
    pub(crate) fn affinities(&mut self, group: usize) -> &[f64] {
        self.affinity.iter_mut().for_each(|a| *a = 0.0);
        let scale = self.scale[group];
        if scale > 0.0 {
            for &m in &self.modules[self.starts[group]..self.starts[group + 1]] {
                let m = m as usize;
                for (w, &bits) in self.holders[m * self.words..(m + 1) * self.words].iter().enumerate() {
                    let mut bits = bits;
                    while bits != 0 {
                        let c = w * 64 + bits.trailing_zeros() as usize;
                        self.affinity[c] += self.worth[m];
                        bits &= bits - 1;
                    }
                }
            }
            self.affinity.iter_mut().for_each(|a| *a *= scale);
        }
        &self.affinity
    }

    pub(crate) fn place(&mut self, group: usize, checker: usize) {
        for &m in &self.modules[self.starts[group]..self.starts[group + 1]] {
            self.holders[m as usize * self.words + checker / 64] |= 1 << (checker % 64);
        }
    }

    // Moves groups between checkers while that lowers the worth of the modules held per checker (see above).
    // `group_checkers` is FENNEL's placement of the groups, `group_weights` their loads.
    pub(crate) fn refine(&self, group_checkers: &mut [usize], group_weights: &[i64], checker_count: usize) {
        let passes = config().passes;
        if passes == 0 || checker_count < 2 {
            return;
        }
        let k = checker_count;
        let words = self.words;
        let module_count = self.worth.len();
        // Per module and checker: how many placed groups on the checker have the module, and a bit per checker where
        // that is not 0 (most modules are held by a few checkers, so the scan below visits the set bits only).
        // `spread[m]` is the number of those bits: a module every checker holds counts the same for every destination.
        let mut counts = vec![0u32; module_count * k];
        let mut bits = vec![0u64; module_count * words];
        let mut spread = vec![0u32; module_count];
        let mut loads = vec![0i64; k];
        for (g, &c) in group_checkers.iter().enumerate() {
            loads[c] += group_weights[g];
            for &m in self.group_modules(g) {
                let m = m as usize;
                counts[m * k + c] += 1;
                if counts[m * k + c] == 1 {
                    bits[m * words + c / 64] |= 1 << (c % 64);
                    spread[m] += 1;
                }
            }
        }
        let total: i64 = loads.iter().sum();
        let average = (total + k as i64 - 1) / k as i64;
        let cap = average + average / 100;
        let mut order: Vec<usize> = (0..group_checkers.len()).collect();
        order.sort_by_key(|&g| (group_weights[g], g));
        let mut held = vec![0.0f64; k];
        for _ in 0..passes {
            let mut moved = false;
            for &g in &order {
                let from = group_checkers[g];
                held.iter_mut().for_each(|h| *h = 0.0);
                let (mut only_here, mut held_everywhere) = (0.0, 0.0);
                for &m in self.group_modules(g) {
                    let m = m as usize;
                    let worth = self.worth[m];
                    if counts[m * k + from] == 1 {
                        only_here += worth;
                    }
                    if spread[m] as usize == k {
                        held_everywhere += worth;
                        continue;
                    }
                    for (w, &word) in bits[m * words..(m + 1) * words].iter().enumerate() {
                        let mut word = word;
                        while word != 0 {
                            held[w * 64 + word.trailing_zeros() as usize] += worth;
                            word &= word - 1;
                        }
                    }
                }
                let mut best: Option<(f64, usize)> = None;
                for to in 0..k {
                    if to == from || loads[to] + group_weights[g] > cap {
                        continue;
                    }
                    let gain = only_here - (self.total[g] - held_everywhere - held[to]);
                    if gain > best.map_or(1e-9, |b| b.0) {
                        best = Some((gain, to));
                    }
                }
                if let Some((_, to)) = best {
                    for &m in self.group_modules(g) {
                        let m = m as usize;
                        counts[m * k + from] -= 1;
                        if counts[m * k + from] == 0 {
                            bits[m * words + from / 64] &= !(1 << (from % 64));
                            spread[m] -= 1;
                        }
                        counts[m * k + to] += 1;
                        if counts[m * k + to] == 1 {
                            bits[m * words + to / 64] |= 1 << (to % 64);
                            spread[m] += 1;
                        }
                    }
                    loads[from] -= group_weights[g];
                    loads[to] += group_weights[g];
                    group_checkers[g] = to;
                    moved = true;
                }
            }
            if !moved {
                break;
            }
        }
    }

    fn group_modules(&self, group: usize) -> &[u32] {
        &self.modules[self.starts[group]..self.starts[group + 1]]
    }
}

#[cfg(test)]
mod tests {
    use super::ModuleAffinity;

    // Modules 0 and 1 (worth 3 and 2); groups 0 and 2 have module 0, group 1 has module 1; each group weighs 1.
    fn affinity() -> ModuleAffinity {
        ModuleAffinity {
            worth: vec![3.0, 2.0],
            starts: vec![0, 1, 2, 3],
            modules: vec![0, 1, 0],
            total: vec![3.0, 2.0, 3.0],
            scale: vec![1.0, 1.0, 1.0],
            words: 1,
            holders: vec![0; 2],
            affinity: vec![0.0; 2],
        }
    }

    /// A group moves to the checker that already holds its modules when that checker has room under the cap (two
    /// checkers, three groups of weight 1: at most 2 per checker); a move that would bring a module to a checker
    /// without it, or go over the cap, is not made.
    #[test]
    fn refine_moves_a_group_to_its_modules_within_the_cap() {
        let mut placed = vec![0, 0, 1];
        affinity().refine(&mut placed, &[1, 1, 1], 2);
        assert_eq!(placed, vec![1, 0, 1]);
        // Already the best placement under the cap: nothing moves.
        let mut placed = vec![1, 0, 1];
        affinity().refine(&mut placed, &[1, 1, 1], 2);
        assert_eq!(placed, vec![1, 0, 1]);
        // Group 2 cannot join group 0 on checker 0, which is full; group 0 cannot move to checker 1, which is full.
        let mut placed = vec![0, 1, 1];
        affinity().refine(&mut placed, &[1, 1, 2], 2);
        assert_eq!(placed, vec![0, 1, 1]);
    }
}
