// Parent side of the process pool. Each worker is this binary re-executed as `__worker` with a batch of
// items on stdin. A worker that dies (stack overflow, abort, OOM kill) or exceeds the per-test timeout is
// killed; the running item is recorded as crash/timeout and the rest of its batch is re-queued.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::baseline::Class;
use crate::compiler_runner::TestItem;
use crate::worker::format_item_line;

pub struct PoolOptions {
    pub jobs: usize,
    pub timeout: Duration,
    pub recycle: usize,
    pub worker_args: Vec<String>,
    pub progress: bool,
}

#[derive(Clone, Debug)]
pub struct TestResult {
    pub class: Class,
    pub ms: u64,
    pub diff: String,
    pub panic: String,
    pub loc: String,
    pub skip: String,
}

enum Event {
    Line(usize, String),
    Exited(usize),
}

struct Worker {
    child: Child,
    batch: Vec<usize>,
    // Number of results received for this batch.
    done: usize,
    // Index into `batch` of the item that is currently running, and since when.
    running: Option<(usize, Instant)>,
    stderr: Arc<Mutex<Vec<u8>>>,
    alive: bool,
}

fn stderr_tail(buf: &Arc<Mutex<Vec<u8>>>) -> String {
    let b = buf.lock().unwrap();
    let s = String::from_utf8_lossy(&b);
    let lines: Vec<&str> = s.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(3);
    lines[start..].join(" | ")
}

fn spawn(id: usize, items: &[TestItem], batch: Vec<usize>, opts: &PoolOptions, tx: &Sender<Event>) -> Worker {
    let exe = std::env::current_exe().expect("current_exe");
    let mut child = Command::new(exe)
        .arg("__worker")
        .args(&opts.worker_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn worker");
    let mut input = String::new();
    for &i in &batch {
        input.push_str(&format_item_line(&items[i]));
        input.push('\n');
    }
    let mut stdin = child.stdin.take().unwrap();
    std::thread::spawn(move || {
        let _ = stdin.write_all(input.as_bytes());
    });
    let stdout = child.stdout.take().unwrap();
    let tx2 = tx.clone();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx2.send(Event::Line(id, line)).is_err() {
                return;
            }
        }
        let _ = tx2.send(Event::Exited(id));
    });
    let stderr_buf = Arc::new(Mutex::new(Vec::new()));
    let mut stderr = child.stderr.take().unwrap();
    let buf2 = stderr_buf.clone();
    std::thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        while let Ok(n) = stderr.read(&mut chunk) {
            if n == 0 {
                break;
            }
            let mut b = buf2.lock().unwrap();
            b.extend_from_slice(&chunk[..n]);
            if b.len() > 64 * 1024 {
                let cut = b.len() - 32 * 1024;
                b.drain(..cut);
            }
        }
    });
    Worker { child, batch, done: 0, running: None, stderr: stderr_buf, alive: true }
}

fn parse_result(v: &Value) -> TestResult {
    let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
    TestResult {
        class: Class::parse(v["class"].as_str().unwrap_or("fail")).unwrap_or(Class::Fail),
        ms: v["ms"].as_u64().unwrap_or(0),
        diff: s("diff"),
        panic: s("panic"),
        loc: s("loc"),
        skip: s("skip"),
    }
}

pub fn run_pool(items: &[TestItem], opts: &PoolOptions) -> Vec<Option<TestResult>> {
    let mut results: Vec<Option<TestResult>> = vec![None; items.len()];
    let mut queue: VecDeque<usize> = (0..items.len()).collect();
    let (tx, rx): (Sender<Event>, Receiver<Event>) = mpsc::channel();
    let mut workers: Vec<Worker> = Vec::new();
    let mut active = 0usize;
    let mut completed = 0usize;
    let total = items.len();
    let started = Instant::now();
    let mut last_progress = Instant::now();

    let next_batch = |queue: &mut VecDeque<usize>, jobs: usize, recycle: usize| -> Vec<usize> {
        let per_worker = queue.len().div_ceil(jobs.max(1)).clamp(1, recycle.max(1));
        let n = per_worker.min(queue.len());
        queue.drain(..n).collect()
    };

    loop {
        while active < opts.jobs && !queue.is_empty() {
            let batch = next_batch(&mut queue, opts.jobs, opts.recycle);
            let id = workers.len();
            workers.push(spawn(id, items, batch, opts, &tx));
            active += 1;
        }
        if active == 0 {
            break;
        }

        let event = rx.recv_timeout(Duration::from_millis(250));
        let now = Instant::now();
        match event {
            Ok(Event::Line(id, line)) => {
                let w = &mut workers[id];
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                if let Some(i) = v["start"].as_u64() {
                    w.running = Some((i as usize, now));
                } else if let Some(i) = v["i"].as_u64() {
                    let idx = w.batch[i as usize];
                    results[idx] = Some(parse_result(&v));
                    w.done = i as usize + 1;
                    w.running = None;
                    completed += 1;
                }
            }
            Ok(Event::Exited(id)) => {
                let w = &mut workers[id];
                if !w.alive {
                    continue;
                }
                let status = w.child.wait().ok();
                w.alive = false;
                active -= 1;
                if w.done < w.batch.len() {
                    // Died while running batch[w.done]: record a crash and re-queue the rest.
                    let crashed = w.batch[w.done];
                    let tail = stderr_tail(&w.stderr);
                    let status_text = status.map(|s| format!("{s}")).unwrap_or_default();
                    let message = if tail.contains("overflowed its stack") {
                        "stack overflow".to_string()
                    } else if tail.is_empty() {
                        format!("worker died ({status_text})")
                    } else {
                        format!("worker died ({status_text}): {tail}")
                    };
                    results[crashed] =
                        Some(TestResult { class: Class::Crash, ms: 0, diff: String::new(), panic: message, loc: String::new(), skip: String::new() });
                    completed += 1;
                    for &rest in w.batch[w.done + 1..].iter().rev() {
                        queue.push_front(rest);
                    }
                }
            }
            Err(_) => {}
        }

        // Timeouts
        for w in workers.iter_mut().filter(|w| w.alive) {
            if let Some((i, since)) = w.running {
                if now.duration_since(since) > opts.timeout {
                    let _ = w.child.kill();
                    let _ = w.child.wait();
                    w.alive = false;
                    active -= 1;
                    let idx = w.batch[i];
                    results[idx] = Some(TestResult {
                        class: Class::Timeout,
                        ms: opts.timeout.as_millis() as u64,
                        diff: String::new(),
                        panic: String::new(),
                        loc: String::new(),
                        skip: String::new(),
                    });
                    completed += 1;
                    for &rest in w.batch[i + 1..].iter().rev() {
                        queue.push_front(rest);
                    }
                    w.done = w.batch.len();
                }
            }
        }

        if opts.progress && last_progress.elapsed() > Duration::from_secs(10) {
            last_progress = Instant::now();
            eprintln!("  {completed}/{total} done, {}s", started.elapsed().as_secs());
        }
    }
    results
}
