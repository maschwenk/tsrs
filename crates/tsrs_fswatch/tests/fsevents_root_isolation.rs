// FSEvents drops under one root: a burst of writes large enough that fseventsd drops events for the busy root's
// stream (an overflow there) must not overflow another watched root where nothing changed. Before each root had its
// own stream, the drop was reported on every root of the shared stream. Its own test binary, so that the burst does
// not overflow the crate's unit tests.
#![cfg(target_os = "macos")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tsrs_fswatch::{fsevents, ErrOverflow, WatchCallback};

fn count_overflows(overflows: &Arc<AtomicUsize>) -> WatchCallback {
    let overflows = Arc::clone(overflows);
    Arc::new(move |_events, err| {
        if err.is_some_and(|err| err.is(ErrOverflow)) {
            overflows.fetch_add(1, Ordering::SeqCst);
        }
    })
}

#[test]
fn a_drop_under_one_root_does_not_overflow_another() {
    let base = std::env::temp_dir().join(format!("tsrs-fswatch-isolation-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (quiet, busy) = (base.join("quiet"), base.join("busy"));
    std::fs::create_dir_all(&quiet).unwrap();
    std::fs::create_dir_all(&busy).unwrap();
    let (quiet, busy) = (std::fs::canonicalize(&quiet).unwrap(), std::fs::canonicalize(&busy).unwrap());
    std::fs::write(quiet.join("index.d.ts"), "export {};\n").unwrap();

    let quiet_overflows = Arc::new(AtomicUsize::new(0));
    let busy_overflows = Arc::new(AtomicUsize::new(0));
    let recursive = || vec![tsrs_fswatch::with_recursive()];
    let quiet_watch = fsevents().watch_directory(quiet.to_str().unwrap(), count_overflows(&quiet_overflows), recursive()).unwrap();
    let busy_watch = fsevents().watch_directory(busy.to_str().unwrap(), count_overflows(&busy_overflows), recursive()).unwrap();

    // 20k writes and their deletes make fseventsd drop events about once (a client that does nothing in its
    // callback too); keep going until it has.
    const FILES: usize = 20_000;
    let deadline = Instant::now() + Duration::from_secs(60);
    while busy_overflows.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
        let work = busy.join("work");
        for i in 0..FILES {
            let dir = work.join(format!("d{}", i / 100));
            if i % 100 == 0 {
                std::fs::create_dir_all(&dir).unwrap();
            }
            std::fs::write(dir.join(format!("f{i}.ts")), format!("export const x = {i};\n")).unwrap();
        }
        std::fs::remove_dir_all(&work).unwrap();
        std::thread::sleep(Duration::from_secs(1));
    }
    // Let any overflow still on the way to the quiet root arrive.
    std::thread::sleep(Duration::from_secs(2));
    let _ = quiet_watch.close();
    let _ = busy_watch.close();
    let _ = std::fs::remove_dir_all(&base);

    assert!(busy_overflows.load(Ordering::SeqCst) > 0, "fseventsd dropped no events, so the test did not run");
    assert_eq!(quiet_overflows.load(Ordering::SeqCst), 0, "the busy root's drop overflowed the quiet root");
}
