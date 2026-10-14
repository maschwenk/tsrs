// tsrs-only: tests for the per-root streams (`fseventsMaxRootStreams`); Go has one set of streams for every root.

use std::sync::Arc;

use super::{fsEventsBackend, fseventsMaxRootStreams, new_fs_events_backend};
use crate::debounce::debounce;
use crate::pathcompare::pathComparer;
use crate::watcher::{dirWatch, watcherImpl};

fn root_dirs(n: usize) -> (std::path::PathBuf, Vec<String>) {
    let base = std::env::temp_dir().join(format!("tsrs-fswatch-roots-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let base = {
        std::fs::create_dir_all(&base).unwrap();
        std::fs::canonicalize(&base).unwrap()
    };
    let dirs = (0..n)
        .map(|i| {
            let dir = base.join(format!("r{i:04}"));
            std::fs::create_dir(&dir).unwrap();
            dir.to_str().unwrap().to_string()
        })
        .collect();
    (base, dirs)
}

fn stream_counts(backend: &fsEventsBackend) -> (usize, usize, usize) {
    let st = backend.mu.lock().unwrap();
    (st.root_streams.len(), st.shared_roots.len(), st.shared_streams.len())
}

#[test]
fn each_root_gets_its_own_stream_up_to_the_limit_and_the_rest_share() {
    let (base, dirs) = root_dirs(fseventsMaxRootStreams + 3);
    let db: Arc<debounce> = debounce::new();
    let watches: Vec<Arc<dirWatch>> = dirs.iter().map(|dir| dirWatch::new(dir.clone(), dir.clone(), &db, pathComparer::default(), None, true)).collect();
    let backend = new_fs_events_backend();

    backend.subscribe_many(&watches[..2]).unwrap().unwrap();
    assert_eq!(stream_counts(&backend), (2, 0, 0));

    backend.subscribe_many(&watches[2..]).unwrap().unwrap();
    assert_eq!(stream_counts(&backend), (fseventsMaxRootStreams, 3, 1));

    // A second watch on a root that has its own stream (here non-recursive) joins that stream.
    let same_root = dirWatch::new(dirs[0].clone(), dirs[0].clone(), &db, pathComparer::default(), None, false);
    backend.subscribe(&same_root).unwrap();
    assert_eq!(stream_counts(&backend), (fseventsMaxRootStreams, 3, 1));
    backend.close_watch(&watches[0]).unwrap();
    assert_eq!(stream_counts(&backend), (fseventsMaxRootStreams, 3, 1));
    backend.close_watch(&same_root).unwrap();
    assert_eq!(stream_counts(&backend), (fseventsMaxRootStreams - 1, 3, 1));

    let last = watches.len() - 1;
    backend.close_watch(&watches[last]).unwrap();
    assert_eq!(stream_counts(&backend), (fseventsMaxRootStreams - 1, 2, 1));

    for watch in &watches[1..last] {
        backend.close_watch(watch).unwrap();
    }
    assert_eq!(stream_counts(&backend), (0, 0, 0));
    let _ = std::fs::remove_dir_all(&base);
}
