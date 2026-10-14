// `set_write_settling` (inotify): callbacks wait for the writer to close the file instead of firing at the first
// write. Its own test binary: the setting is process-wide, and the crate's unit tests run in parallel threads.
#![cfg(target_os = "linux")]

use std::io::Write;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use tsrs_fswatch::{inotify, set_write_settling};

// What a callback saw: when it ran, and the file's content at that moment.
struct Seen {
    at: Instant,
    content: String,
}

struct Recorder {
    seen: Mutex<Vec<Seen>>,
    cv: Condvar,
}

impl Recorder {
    fn wait_for(&self, count: usize, timeout: Duration) -> Vec<(Instant, String)> {
        let seen = self.seen.lock().unwrap();
        let (seen, _) = self.cv.wait_timeout_while(seen, timeout, |s| s.len() < count).unwrap();
        seen.iter().map(|s| (s.at, s.content.clone())).collect()
    }
}

fn watch(dir: &std::path::Path, file: &std::path::Path) -> (Arc<Recorder>, Box<dyn tsrs_fswatch::Watch>) {
    let recorder = Arc::new(Recorder { seen: Mutex::new(Vec::new()), cv: Condvar::new() });
    let r = Arc::clone(&recorder);
    let file = file.to_path_buf();
    let watch = inotify()
        .watch_directory(
            dir.to_str().unwrap(),
            Arc::new(move |events, _err| {
                if events.is_empty() {
                    return;
                }
                let content = std::fs::read_to_string(&file).unwrap_or_default();
                r.seen.lock().unwrap().push(Seen { at: Instant::now(), content });
                r.cv.notify_all();
            }),
            Vec::new(),
        )
        .unwrap();
    (recorder, watch)
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("tsrs-fswatch-settle-{}-{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn write_settling() {
    set_write_settling(true);
    callbacks_wait_for_the_close();
    an_unclosed_file_stops_holding_after_the_idle_time();
    a_file_kept_open_holds_once();
}

fn callbacks_wait_for_the_close() {
    let dir = temp_dir("close");
    let file = dir.join("a.ts");
    std::fs::write(&file, "export const a = 0;\n").unwrap();
    let (recorder, watch) = watch(&dir, &file);
    // A quiet period first: without settling, the next event would fire at once (the debounce's leading edge).
    std::thread::sleep(Duration::from_millis(700));

    let mut f = std::fs::File::create(&file).unwrap();
    f.write_all(b"export const a = 1;\n").unwrap();
    f.flush().unwrap();
    assert!(recorder.wait_for(1, Duration::from_millis(400)).is_empty(), "fired while the file was being written");
    f.write_all(b"export const b = 2;\n").unwrap();
    drop(f);
    let closed = Instant::now();

    let seen = recorder.wait_for(1, Duration::from_secs(5));
    assert_eq!(seen.len(), 1, "one batch for the whole save");
    let (at, content) = &seen[0];
    assert_eq!(content, "export const a = 1;\nexport const b = 2;\n");
    // At the close, not when the hold times out (`settleIdleTime`, 1 s).
    assert!(at.duration_since(closed) < Duration::from_millis(500), "fired {:?} after the close", at.duration_since(closed));
    let _ = watch.close();
    let _ = std::fs::remove_dir_all(&dir);
}

fn an_unclosed_file_stops_holding_after_the_idle_time() {
    let dir = temp_dir("open");
    let file = dir.join("log.txt");
    let (recorder, watch) = watch(&dir, &file);
    std::thread::sleep(Duration::from_millis(700));

    let mut f = std::fs::File::create(&file).unwrap();
    f.write_all(b"line\n").unwrap();
    f.flush().unwrap();
    let written = Instant::now();
    let seen = recorder.wait_for(1, Duration::from_secs(5));
    assert_eq!(seen.len(), 1, "a file left open must not hold the callbacks for good");
    let waited = seen[0].0.duration_since(written);
    assert!(waited >= Duration::from_millis(900) && waited < Duration::from_millis(2500), "released after {waited:?}");
    drop(f);
    let _ = watch.close();
    let _ = std::fs::remove_dir_all(&dir);
}

// An editor's swap file or a log stays open and is written now and then. Once it has stopped holding the callbacks, it
// must not hold them again (for up to `settleMaxTime` each time it is written) while it stays open.
fn a_file_kept_open_holds_once() {
    let dir = temp_dir("kept-open");
    let file = dir.join("a.ts");
    std::fs::write(&file, "export const a = 0;\n").unwrap();
    let (recorder, watch) = watch(&dir, &file);
    std::thread::sleep(Duration::from_millis(700));

    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let writer = {
        let stop = Arc::clone(&stop);
        let log = dir.join(".a.ts.swp");
        std::thread::spawn(move || {
            let mut f = std::fs::File::create(log).unwrap();
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                f.write_all(b"x").unwrap();
                f.flush().unwrap();
                std::thread::sleep(Duration::from_millis(100));
            }
        })
    };
    // The first hold ends at `settleMaxTime` (the writer is never idle for `settleIdleTime`).
    assert_eq!(recorder.wait_for(1, Duration::from_secs(5)).len(), 1);
    std::thread::sleep(Duration::from_millis(700));
    let before = recorder.wait_for(0, Duration::ZERO).len();

    std::fs::write(&file, "export const a = 1;\n").unwrap();
    let saved = Instant::now();
    let seen = recorder.wait_for(before + 1, Duration::from_secs(5));
    let (at, content) = seen.iter().rev().find(|(_, c)| c == "export const a = 1;\n").expect("the save was seen");
    assert_eq!(content, "export const a = 1;\n");
    assert!(at.duration_since(saved) < Duration::from_millis(700), "fired {:?} after the save", at.duration_since(saved));

    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    writer.join().unwrap();
    let _ = watch.close();
    let _ = std::fs::remove_dir_all(&dir);
}
