// FSEvents under CPU load: while every core is busy (a watch cycle's checkers and emit), a burst of writes must not
// make fseventsd drop events for the client (an overflow, which the watch turns into a full rebuild). Its own test
// binary, so that the busy threads do not slow the crate's unit tests.
#![cfg(target_os = "macos")]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tsrs_fswatch::{fsevents, ErrOverflow};

#[test]
fn a_burst_under_load_drops_no_events() {
    let dir = std::env::temp_dir().join(format!("tsrs-fswatch-load-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let dir = std::fs::canonicalize(&dir).unwrap();

    let overflows = Arc::new(AtomicUsize::new(0));
    let events = Arc::new(AtomicUsize::new(0));
    let (o, e) = (Arc::clone(&overflows), Arc::clone(&events));
    let watch = fsevents()
        .watch_directory(
            dir.to_str().unwrap(),
            Arc::new(move |batch, err| {
                if err.is_some_and(|err| err.is(ErrOverflow)) {
                    o.fetch_add(1, Ordering::SeqCst);
                }
                e.fetch_add(batch.len(), Ordering::SeqCst);
            }),
            Vec::new(),
        )
        .unwrap();

    // Twice the cores, so that the busy threads cover the efficiency cores too.
    let stop = Arc::new(AtomicBool::new(false));
    let spinners: Vec<_> = (0..2 * std::thread::available_parallelism().map_or(8, |n| n.get()))
        .map(|_| {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                let mut x = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    x = std::hint::black_box(x.wrapping_mul(6364136223846793005).wrapping_add(1));
                }
            })
        })
        .collect();

    const FILES: usize = 10_000;
    for i in 0..FILES {
        std::fs::write(dir.join(format!("f{i}.ts")), "export {};\n").unwrap();
    }
    // Wait until the events have arrived (or an overflow said they will not).
    let deadline = Instant::now() + Duration::from_secs(20);
    while events.load(Ordering::SeqCst) < FILES && overflows.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    stop.store(true, Ordering::Relaxed);
    for s in spinners {
        s.join().unwrap();
    }
    let _ = watch.close();
    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(overflows.load(Ordering::SeqCst), 0, "fseventsd dropped events");
    assert!(events.load(Ordering::SeqCst) >= FILES, "{} of {FILES} events", events.load(Ordering::SeqCst));
}
