// tsrs-only: recursive-watch consolidation (`find_consolidation_dir_locked`) joins the watches of a directory's
// children, and does not climb past them because one subtree below holds many watches.

use super::watcher;

fn dir_watch_root(w: &watcher, dir: &str) -> String {
    let comparer = w.path_comparer(dir).unwrap();
    w.get_or_create_dir_watch(dir, dir, true, comparer).unwrap().dir.clone()
}

#[test]
fn consolidates_siblings_but_not_their_unrelated_ancestors() {
    let base = std::env::temp_dir().join(format!("tsrs-fswatch-consolidate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    let base = std::fs::canonicalize(&base).unwrap().to_str().unwrap().to_string();
    // A package store's layout: <store>/links/<name>/<version>/<hash>/node_modules, beside a project.
    let links = format!("{base}/store/links");
    let packages: Vec<String> = (0..12).map(|i| format!("{links}/p{i}/1.0.0/h{i}/node_modules")).collect();
    let project = format!("{base}/project");
    for dir in packages.iter().chain([&project]) {
        std::fs::create_dir_all(dir).unwrap();
    }

    let w = watcher::new("fsevents");
    for package in &packages[..9] {
        assert_eq!(dir_watch_root(&w, package), *package);
    }
    // The tenth child of `links` with a watch: the packages share one recursive watch on `links`.
    assert_eq!(dir_watch_root(&w, &packages[9]), links);
    assert_eq!(dir_watch_root(&w, &packages[10]), links);
    // `base` holds ten watches but only two children with any; the project keeps its own watch rather than
    // consolidating into `base` (Go's rule, counting watches, consolidated it into `base`).
    assert_eq!(dir_watch_root(&w, &project), project);

    let _ = std::fs::remove_dir_all(&base);
}
