use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use crate::workerpool::{go, max_idle_threads, threads};

// A job must not wait for a worker while every worker is blocked (Go starts a goroutine per request): requests that
// wait for a busy checker would otherwise hold up requests that need none. The jobs here wait the way those do, in
// `runslots::blocking` (a wait that keeps its run slot keeps one of the jobs that can run at once).
#[test]
fn test_job_runs_while_every_worker_is_blocked() {
    let blocked = max_idle_threads();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let (started_tx, started_rx) = mpsc::channel();
    for _ in 0..blocked {
        let release_rx = Arc::clone(&release_rx);
        let started_tx = started_tx.clone();
        go(move || {
            started_tx.send(()).unwrap();
            tsrs_core::runslots::blocking(|| release_rx.lock().unwrap().recv().unwrap());
        });
    }
    for _ in 0..blocked {
        started_rx.recv_timeout(Duration::from_secs(10)).expect("a blocking job did not start");
    }

    let (done_tx, done_rx) = mpsc::channel();
    go(move || done_tx.send(()).unwrap());
    let ran = done_rx.recv_timeout(Duration::from_secs(10));

    for _ in 0..blocked {
        release_tx.send(()).unwrap();
    }
    ran.expect("a job queued behind blocked workers");

    // The workers started beyond the idle limit exit once there is no job.
    let deadline = Instant::now() + Duration::from_secs(10);
    while threads() > max_idle_threads() {
        assert!(Instant::now() < deadline, "workers beyond the idle limit did not exit");
        std::thread::sleep(Duration::from_millis(10));
    }
}
