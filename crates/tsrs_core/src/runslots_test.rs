use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use super::*;

// One test: the slots are process-wide, so two tests running at once would share them.
#[test]
fn run_limits_concurrency_and_blocking_gives_the_slot_back() {
    let n = capacity();

    // At most `n` jobs run at once, and a burst of 3n uses all `n`.
    let running = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let threads: Vec<_> = (0..3 * n)
        .map(|_| {
            let (running, peak) = (Arc::clone(&running), Arc::clone(&peak));
            std::thread::spawn(move || {
                run(|| {
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(20));
                    running.fetch_sub(1, Ordering::SeqCst);
                });
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(peak.load(Ordering::SeqCst), n);

    // `n` jobs wait in `blocking` for a job that can only run once they have given their slots back.
    let (go_tx, go_rx) = mpsc::channel::<()>();
    let go_rx = Arc::new(Mutex::new(go_rx));
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let waiters: Vec<_> = (0..n)
        .map(|_| {
            let (go_rx, done_tx) = (Arc::clone(&go_rx), done_tx.clone());
            std::thread::spawn(move || {
                run(|| {
                    blocking(|| go_rx.lock().unwrap().recv_timeout(Duration::from_secs(10)).expect("the releasing job ran"));
                    done_tx.send(()).unwrap();
                });
            })
        })
        .collect();
    std::thread::sleep(Duration::from_millis(50));
    let releaser = std::thread::spawn(move || {
        run(|| {
            for _ in 0..n {
                go_tx.send(()).unwrap();
            }
        });
    });
    for _ in 0..n {
        done_rx.recv_timeout(Duration::from_secs(20)).expect("every waiter finished");
    }
    releaser.join().unwrap();
    for t in waiters {
        t.join().unwrap();
    }

    // Outside a job, `blocking` takes no slot; nested `run` keeps the one it has.
    blocking(|| assert!(!HOLDS_SLOT.with(Cell::get)));
    run(|| run(|| assert!(HOLDS_SLOT.with(Cell::get))));
    assert!(!HOLDS_SLOT.with(Cell::get));
}
