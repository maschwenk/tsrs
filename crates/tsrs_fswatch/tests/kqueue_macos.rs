// The kqueue backend on macOS (the fallback when FSEvents is unavailable): it needs a descriptor per watched file.
// Its own test binary: the tests change the process's descriptor limit and the process-wide write settling.
#![cfg(target_os = "macos")]

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tsrs_fswatch::{kqueue, set_write_settling};

fn temp_dir(name: &str, files: usize) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("tsrs-fswatch-kqueue-{}-{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for i in 0..files {
        std::fs::write(dir.join(format!("f{i}.ts")), "export {};\n").unwrap();
    }
    std::fs::canonicalize(&dir).unwrap()
}

fn set_limit(soft: u64, hard: u64) {
    let lim = libc::rlimit { rlim_cur: soft, rlim_max: hard };
    // SAFETY: `lim` is a valid rlimit.
    assert_eq!(unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const lim) }, 0, "{}", std::io::Error::last_os_error());
}

type Seen = Arc<Mutex<Vec<(Instant, Vec<String>, String)>>>;

// Records each batch: when it came, its paths, and the content of `read` at that moment.
fn watch(dir: &std::path::Path, read: std::path::PathBuf) -> (Seen, Result<Box<dyn tsrs_fswatch::Watch>, tsrs_fswatch::Error>) {
    let seen: Seen = Arc::default();
    let s = Arc::clone(&seen);
    let w = kqueue().watch_directory(
        dir.to_str().unwrap(),
        Arc::new(move |events, _err| {
            if !events.is_empty() {
                let content = std::fs::read_to_string(&read).unwrap_or_default();
                s.lock().unwrap().push((Instant::now(), events.into_iter().map(|e| e.path).collect(), content));
            }
        }),
        Vec::new(),
    );
    (seen, w)
}

fn wait_for(seen: &Seen, n: usize, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while seen.lock().unwrap().len() < n && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn kqueue_on_macos() {
    set_write_settling(true);
    // A soft limit below the file count, as a shell on macOS starts with (256).
    set_limit(64, libc::RLIM_INFINITY);
    every_file_is_watched_past_the_soft_limit();
    a_truncation_waits_for_the_data();
    // A hard limit the backend cannot raise: watching fails with a clear error instead of skipping files.
    set_limit(64, 64);
    running_out_of_descriptors_is_an_error();
}

fn every_file_is_watched_past_the_soft_limit() {
    let dir = temp_dir("many", 200);
    let last = dir.join("f199.ts");
    let (seen, w) = watch(&dir, last.clone());
    let w = w.expect("watch");
    std::thread::sleep(Duration::from_millis(600));
    std::fs::write(&last, "export const x = 1;\n").unwrap();
    wait_for(&seen, 1, Duration::from_secs(3));
    let seen = seen.lock().unwrap();
    assert!(seen.iter().any(|(_, paths, _)| paths.iter().any(|p| p.ends_with("f199.ts"))), "no event for the last file: {:?}", *seen);
    drop(seen);
    let _ = w.close();
    let _ = std::fs::remove_dir_all(&dir);
}

fn a_truncation_waits_for_the_data() {
    let dir = temp_dir("truncate", 1);
    let file = dir.join("f0.ts");
    let (seen, w) = watch(&dir, file.clone());
    let w = w.expect("watch");
    std::thread::sleep(Duration::from_millis(700));
    seen.lock().unwrap().clear();

    let mut f = std::fs::File::create(&file).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(seen.lock().unwrap().is_empty(), "fired while the file was empty");
    f.write_all(b"export const a = 1;\n").unwrap();
    drop(f);
    wait_for(&seen, 1, Duration::from_secs(2));
    std::thread::sleep(Duration::from_millis(300));
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "one batch for the whole save");
    assert_eq!(seen[0].2, "export const a = 1;\n");
    drop(seen);
    let _ = w.close();
    let _ = std::fs::remove_dir_all(&dir);
}

fn running_out_of_descriptors_is_an_error() {
    let dir = temp_dir("exhausted", 200);
    let (_seen, w) = watch(&dir, dir.join("f0.ts"));
    let err = w.err().expect("watching 200 files with 64 descriptors succeeded");
    assert!(err.message().contains("ulimit -n"), "{}", err.message());
    let _ = std::fs::remove_dir_all(&dir);
}
