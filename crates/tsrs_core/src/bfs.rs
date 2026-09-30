use crate::collections::{new_ordered_map_with_size_hint, OrderedMap, OrderedMapExt, SyncSet};
use crate::P;
use std::hash::Hash;

// The Go implementation visits each level in parallel but always picks the lowest-index result,
// so this sequential port produces the same results.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BreadthFirstSearchResult<N> {
    pub stopped: bool,
    pub path: Vec<N>,
}

struct BreadthFirstSearchJob<N: 'static> {
    node: N,
    parent: Option<P<BreadthFirstSearchJob<N>>>,
}

pub struct BreadthFirstSearchLevel<'a, K, N: 'static> {
    jobs: &'a mut OrderedMap<K, P<BreadthFirstSearchJob<N>>>,
}

impl<K: Hash + Eq, N> BreadthFirstSearchLevel<'_, K, N> {
    pub fn has(&self, key: &K) -> bool {
        self.jobs.has(key)
    }

    pub fn delete(&mut self, key: &K) {
        self.jobs.delete(key);
    }

    pub fn range(&self, mut f: impl FnMut(&N) -> bool) {
        for job in self.jobs.values() {
            if !f(&job.node) {
                return;
            }
        }
    }
}

pub struct BreadthFirstSearchOptions<'a, K: Hash + Eq, N: 'static> {
    // Visited is a set of nodes that have already been visited.
    // If nil, a new set will be created.
    pub visited: Option<&'a SyncSet<K>>,
    // PreprocessLevel is a function that, if provided, will be called
    // before each level, giving the caller an opportunity to remove nodes.
    pub preprocess_level: Option<&'a mut dyn FnMut(&mut BreadthFirstSearchLevel<'_, K, N>)>,
}

impl<K: Hash + Eq, N> Default for BreadthFirstSearchOptions<'_, K, N> {
    fn default() -> Self {
        BreadthFirstSearchOptions { visited: None, preprocess_level: None }
    }
}

// BreadthFirstSearchParallel performs a breadth-first search on a graph
// starting from the given node. It returns the path
// from the first node that satisfies the `visit` function back to the start node.
pub fn breadth_first_search_parallel<N: Clone + Hash + Eq + 'static>(
    start: N,
    neighbors: impl FnMut(&N) -> Vec<N>,
    visit: impl FnMut(&N) -> (bool, bool),
) -> BreadthFirstSearchResult<N> {
    breadth_first_search_parallel_ex(start, neighbors, visit, BreadthFirstSearchOptions::default(), |n: &N| n.clone())
}

// BreadthFirstSearchParallelEx is an extension of BreadthFirstSearchParallel that allows
// the caller to pass a pre-seeded set of already-visited nodes and a preprocessing function
// that can be used to remove nodes from each level before processing.
pub fn breadth_first_search_parallel_ex<K: Hash + Eq + Clone, N: Clone + 'static>(
    start: N,
    mut neighbors: impl FnMut(&N) -> Vec<N>,
    mut visit: impl FnMut(&N) -> (bool, bool),
    options: BreadthFirstSearchOptions<'_, K, N>,
    mut get_key: impl FnMut(&N) -> K,
) -> BreadthFirstSearchResult<N> {
    let own_visited = SyncSet::default();
    let visited = options.visited.unwrap_or(&own_visited);
    let mut preprocess_level = options.preprocess_level;

    let mut fallback: Option<P<BreadthFirstSearchJob<N>>> = None;

    let create_path = |job: Option<P<BreadthFirstSearchJob<N>>>| -> Vec<N> {
        let mut path = Vec::new();
        let mut job = job;
        while let Some(j) = job {
            path.push(j.node.clone());
            job = j.parent;
        }
        path
    };

    let mut level: OrderedMap<K, P<BreadthFirstSearchJob<N>>> = new_ordered_map_with_size_hint(1);
    level.set(get_key(&start), P::new(BreadthFirstSearchJob { node: start, parent: None }));
    while level.size() > 0 {
        // processLevel
        if let Some(pre) = preprocess_level.as_mut() {
            pre(&mut BreadthFirstSearchLevel { jobs: &mut level });
        }
        let mut lowest_goal: Option<usize> = None;
        let mut lowest_fallback: Option<usize> = None;
        let mut next: Vec<Vec<P<BreadthFirstSearchJob<N>>>> = Vec::with_capacity(level.size());
        for (i, &j) in level.values().enumerate() {
            next.push(Vec::new());
            if lowest_goal.is_some() {
                break; // Stop processing if we already found a lower result
            }
            // If we have already visited this node, skip it.
            if !visited.add_if_absent(get_key(&j.node)) {
                continue;
            }
            let (is_result, stop) = visit(&j.node);
            if is_result {
                // We found a result, so we will stop at this level, but an
                // earlier job may still find a true result at a lower index.
                if stop {
                    lowest_goal = Some(i);
                    continue;
                }
                if fallback.is_none() && lowest_fallback.is_none() {
                    lowest_fallback = Some(i);
                }
            }
            // Add the next level jobs
            let neighbor_nodes = neighbors(&j.node);
            if !neighbor_nodes.is_empty() {
                next[i] = neighbor_nodes.into_iter().map(|child| P::new(BreadthFirstSearchJob { node: child, parent: Some(j) })).collect();
            }
        }

        if let Some(index) = lowest_goal {
            // If we found a result, return it immediately.
            let (_, &job) = level.entry_at(index).unwrap();
            return BreadthFirstSearchResult { stopped: true, path: create_path(Some(job)) };
        }
        if fallback.is_none() {
            if let Some(index) = lowest_fallback {
                fallback = Some(*level.entry_at(index).unwrap().1);
            }
        }

        let count = next.iter().map(|n| n.len()).sum();
        let mut next_jobs: OrderedMap<K, P<BreadthFirstSearchJob<N>>> = new_ordered_map_with_size_hint(count);
        for jobs in next {
            for j in jobs {
                let key = get_key(&j.node);
                if !next_jobs.has(&key) {
                    next_jobs.set(key, j);
                }
            }
        }
        level = next_jobs;
    }
    BreadthFirstSearchResult { stopped: false, path: create_path(fallback) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustc_hash::FxHashMap;

    fn graph(edges: &[(&'static str, &[&'static str])]) -> FxHashMap<&'static str, Vec<&'static str>> {
        edges.iter().map(|(k, v)| (*k, v.to_vec())).collect()
    }

    #[test]
    fn test_breadth_first_search_parallel() {
        let g = graph(&[("A", &["B", "C"]), ("B", &["D"]), ("C", &["D"]), ("D", &[])]);
        let children = |n: &&'static str| g[n].clone();

        let result = breadth_first_search_parallel("A", children, |node| (*node == "D", true));
        assert!(result.stopped);
        assert_eq!(result.path, vec!["D", "B", "A"]);

        let mut visited_nodes = Vec::new();
        let result = breadth_first_search_parallel("A", children, |node| {
            visited_nodes.push(*node);
            (false, false)
        });
        assert!(!result.stopped);
        assert!(result.path.is_empty());
        visited_nodes.sort();
        assert_eq!(visited_nodes, vec!["A", "B", "C", "D"]);

        // early termination
        let g2 = graph(&[
            ("Root", &["L1A", "L1B"]),
            ("L1A", &["L2A", "L2B"]),
            ("L1B", &["L2C"]),
            ("L2A", &["L3A"]),
            ("L2B", &[]),
            ("L2C", &[]),
            ("L3A", &[]),
        ]);
        let visited = SyncSet::default();
        breadth_first_search_parallel_ex(
            "Root",
            |n: &&'static str| g2[n].clone(),
            |node| (*node == "L2B", true),
            BreadthFirstSearchOptions { visited: Some(&visited), preprocess_level: None },
            |n| *n,
        );
        assert!(visited.has(&"Root"));
        assert!(visited.has(&"L1A"));
        assert!(visited.has(&"L2B"));
        assert!(!visited.has(&"L3A"));

        assert!(visited.has(&"L1B"));
        assert!(visited.has(&"L2A"));

        // returns fallback when no other result found
        let visited = SyncSet::default();
        let result = breadth_first_search_parallel_ex(
            "A",
            children,
            |node| (*node == "A", false),
            BreadthFirstSearchOptions { visited: Some(&visited), preprocess_level: None },
            |n| *n,
        );
        assert!(!result.stopped);
        assert_eq!(result.path, vec!["A"]);
        assert!(visited.has(&"B") && visited.has(&"C") && visited.has(&"D"));

        // returns a stop result over a fallback
        let result = breadth_first_search_parallel("A", children, |node| match *node {
            "A" => (true, false),
            "D" => (true, true),
            _ => (false, false),
        });
        assert!(result.stopped);
        assert_eq!(result.path, vec!["D", "B", "A"]);
    }
}
