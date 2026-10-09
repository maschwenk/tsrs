// The test registry runner (`tsrs-fourslash run`): Go's `go test ./internal/fourslash/tests` with t.Parallel().
// Tests run in worker processes (`tsrs-fourslash worker`), one test at a time per process on a thread with a
// 256 MB stack. Processes because the in-process language server ends the process
// on an unrecovered panic in one of its threads (like a Go program), and because programs and checkers are never
// freed (docs/LSP.md "Memory plan"): a worker is replaced after a crash, a timeout, or WORKER_TESTS tests.

use std::any::Any as StdAny;
use std::collections::BTreeMap;
use std::fs;
use std::panic;
use std::path::PathBuf;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, Once};
use std::time::{Duration, Instant};

use rustc_hash::FxHashMap;

use crate::testing::{FatalPanic, SkipPanic, T};

pub struct TestEntry {
    // Go test function name (`TestFoo`)
    pub name: &'static str,
    // Go file in ts-ref/tsc/internal/fourslash/tests
    pub file: &'static str,
    pub line: u32,
    pub func: fn(&T),
    // `t.Skip(...)` at the top of the Go test: known failing, with the reason
    pub skip: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Pass,
    Fail(String),
    Skip(String),
}

pub fn repo_root() -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    p.canonicalize().unwrap_or(p)
}

// target/fourslash-results (TSRS_FOURSLASH_RESULTS overrides it).
pub fn results_dir() -> PathBuf {
    if let Ok(d) = std::env::var("TSRS_FOURSLASH_RESULTS") {
        return PathBuf::from(d);
    }
    repo_root().join("target").join("fourslash-results")
}

// Panics other than t.Fatal / t.Skip are reported by worker processes before they terminate.
pub fn install_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        panic::set_hook(Box::new(|info| {
            let payload = info.payload();
            if payload.is::<FatalPanic>() || payload.is::<SkipPanic>() {
                return;
            }
            let msg = payload_message(payload);
            let loc = info.location().map(|l| format!(" at {}:{}", l.file(), l.line())).unwrap_or_default();
            if WORKER_MODE.load(Ordering::Relaxed) {
                // The parent reports the last stderr line when the worker dies (a server thread's panic ends it).
                let thread = std::thread::current().name().unwrap_or("").to_string();
                eprintln!("panic in thread '{thread}': {msg}{loc}");
            }
        }));
    });
}

fn payload_message(p: &(dyn StdAny + Send)) -> String {
    if let Some(s) = p.downcast_ref::<&str>() {
        return s.to_string();
    }
    if let Some(s) = p.downcast_ref::<String>() {
        return s.clone();
    }
    if p.is::<FatalPanic>() {
        return "t.Fatal".to_string();
    }
    "panic with a non-string payload".to_string()
}

pub fn run_test(entry: &TestEntry, include_skipped: bool) -> Outcome {
    if let Some(reason) = entry.skip {
        if !include_skipped {
            return Outcome::Skip(reason.to_string());
        }
    }
    let t = T::new(entry.name, entry.file);
    (entry.func)(&t);
    if t.failed() {
        let logs = t.logs();
        return Outcome::Fail(logs.first().cloned().unwrap_or_else(|| "failed".to_string()));
    }
    if t.skipped() {
        return Outcome::Skip(t.logs().join("; "));
    }
    Outcome::Pass
}

pub struct RunOptions {
    pub filter: Option<regex::Regex>,
    pub include_skipped: bool,
    pub jobs: usize,
    pub verbose: bool,
}

// Tests a worker process runs before it is replaced (bounds the memory the leaked programs take).
const WORKER_TESTS: usize = 200;
// A test that takes longer is reported as failed and its worker is killed.
const TEST_TIMEOUT: Duration = Duration::from_secs(120);

// `tsrs-fourslash worker`: reads test names from stdin, runs each, and writes one result line per test:
// `<name>\t<PASS|FAIL|SKIP>\t<first line of the message>`.
pub fn worker(registry: &'static [TestEntry], include_skipped: bool) {
    install_panic_hook();
    WORKER_MODE.store(true, Ordering::Relaxed);
    let by_name: FxHashMap<&str, &'static TestEntry> = registry.iter().map(|e| (e.name, e)).collect();
    let stdin = std::io::stdin();
    let mut line = String::new();
    loop {
        line.clear();
        if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let name = line.trim_end();
        let Some(entry) = by_name.get(name) else {
            continue;
        };
        let entry: &'static TestEntry = entry;
        let outcome = std::thread::scope(|s| {
            std::thread::Builder::new().stack_size(256 << 20).spawn_scoped(s, || run_test(entry, include_skipped)).expect("spawn test thread").join()
        });
        let outcome = outcome.unwrap_or_else(|_| Outcome::Fail("panic outside the test".to_string()));
        // Full failure messages: target/fourslash-results/failures/<name>.txt.
        let failure_file = results_dir().join("failures").join(format!("{name}.txt"));
        match &outcome {
            Outcome::Fail(m) => {
                let _ = fs::create_dir_all(failure_file.parent().unwrap());
                let _ = fs::write(&failure_file, m);
            }
            _ => {
                let _ = fs::remove_file(&failure_file);
            }
        }
        let (tag, msg) = match &outcome {
            Outcome::Pass => ("PASS", String::new()),
            Outcome::Fail(m) => ("FAIL", m.lines().next().unwrap_or("").to_string()),
            Outcome::Skip(m) => ("SKIP", m.lines().next().unwrap_or("").to_string()),
        };
        // fail.txt keeps the first line of a failure; the whole message goes to failures/<name>.txt.
        match &outcome {
            Outcome::Fail(m) => write_failure_file(name, m),
            _ => remove_failure_file(name),
        }
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{name}\t{tag}\t{msg}");
        let _ = out.flush();
    }
}

static WORKER_MODE: AtomicBool = AtomicBool::new(false);

struct WorkerProcess {
    child: Child,
    stdin: ChildStdin,
    results: mpsc::Receiver<String>,
    last_stderr: Arc<Mutex<String>>,
    tests_run: usize,
}

fn spawn_worker(include_skipped: bool) -> WorkerProcess {
    let exe = std::env::current_exe().expect("current exe");
    let mut cmd = Command::new(exe);
    cmd.arg("worker");
    if include_skipped {
        cmd.arg("--include-skipped");
    }
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("spawn fourslash worker");
    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let last_stderr = Arc::new(Mutex::new(String::new()));
    let last = last_stderr.clone();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            let Ok(line) = line else { break };
            if !line.trim().is_empty() {
                *last.lock().unwrap() = line;
            }
        }
    });
    WorkerProcess { child, stdin, results: rx, last_stderr, tests_run: 0 }
}

fn run_in_worker(worker: &mut Option<WorkerProcess>, entry: &TestEntry, include_skipped: bool) -> Outcome {
    if worker.as_ref().is_some_and(|w| w.tests_run >= WORKER_TESTS) {
        let mut w = worker.take().unwrap();
        drop(w.stdin);
        let _ = w.child.wait();
    }
    let w = worker.get_or_insert_with(|| spawn_worker(include_skipped));
    w.tests_run += 1;
    if writeln!(w.stdin, "{}", entry.name).and_then(|_| w.stdin.flush()).is_err() {
        let mut w = worker.take().unwrap();
        let _ = w.child.kill();
        let _ = w.child.wait();
        return Outcome::Fail("worker process unavailable".to_string());
    }
    match w.results.recv_timeout(TEST_TIMEOUT) {
        Ok(line) => {
            let mut parts = line.splitn(3, '\t');
            let _name = parts.next();
            let tag = parts.next().unwrap_or("");
            let msg = parts.next().unwrap_or("").to_string();
            match tag {
                "PASS" => Outcome::Pass,
                "SKIP" => Outcome::Skip(msg),
                _ => Outcome::Fail(msg),
            }
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let mut w = worker.take().unwrap();
            let _ = w.child.kill();
            let _ = w.child.wait();
            Outcome::Fail(format!("timeout: no result after {}s", TEST_TIMEOUT.as_secs()))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let mut w = worker.take().unwrap();
            let status = w.child.wait().map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
            std::thread::sleep(Duration::from_millis(20));
            let last = w.last_stderr.lock().unwrap().clone();
            Outcome::Fail(format!("crash: worker exited with status {status}: {last}"))
        }
    }
}

fn failure_file(name: &str) -> PathBuf {
    results_dir().join("failures").join(format!("{name}.txt"))
}

fn write_failure_file(name: &str, msg: &str) {
    let path = failure_file(name);
    let _ = fs::create_dir_all(path.parent().unwrap());
    let _ = fs::write(path, format!("{msg}\n"));
}

fn remove_failure_file(name: &str) {
    let _ = fs::remove_file(failure_file(name));
}

// Runs the selected tests in parallel and writes target/fourslash-results/{pass,fail,skip}.txt (and the full
// message of each failure to failures/<name>.txt).
// Returns the number of failures.
pub fn run(registry: &'static [TestEntry], opts: &RunOptions) -> usize {
    install_panic_hook();
    let selected: Vec<&'static TestEntry> = registry.iter().filter(|e| opts.filter.as_ref().is_none_or(|re| re.is_match(e.name))).collect();
    let start = Instant::now();
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<Outcome>>> = Mutex::new(vec![None; selected.len()]);
    let jobs = opts.jobs.max(1).min(selected.len().max(1));
    std::thread::scope(|s| {
        for _ in 0..jobs {
            s.spawn(|| {
                let mut worker: Option<WorkerProcess> = None;
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= selected.len() {
                        break;
                    }
                    let entry = selected[i];
                    let outcome = match entry.skip {
                        Some(reason) if !opts.include_skipped => Outcome::Skip(reason.to_string()),
                        _ => run_in_worker(&mut worker, entry, opts.include_skipped),
                    };
                    if opts.verbose {
                        eprintln!("{} {}", outcome_tag(&outcome), entry.name);
                    }
                    results.lock().unwrap()[i] = Some(outcome);
                }
                if let Some(mut w) = worker {
                    drop(w.stdin);
                    let _ = w.child.wait();
                }
            });
        }
    });
    let results = results.into_inner().unwrap();

    let mut pass = Vec::new();
    let mut fail = Vec::new();
    let mut skip = Vec::new();
    let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
    for (e, r) in selected.iter().zip(results) {
        match r.unwrap() {
            Outcome::Pass => {
                remove_failure_file(e.name);
                pass.push(e.name.to_string())
            }
            Outcome::Fail(msg) => {
                // Crashes and timeouts never reach the worker's own report.
                if msg.starts_with("crash: ") || msg.starts_with("timeout: ") || msg == "worker process unavailable" {
                    write_failure_file(e.name, &msg);
                }
                let first = msg.lines().next().unwrap_or("").to_string();
                *reasons.entry(first.clone()).or_default() += 1;
                fail.push(format!("{}\t{}", e.name, first));
            }
            Outcome::Skip(msg) => {
                remove_failure_file(e.name);
                skip.push(format!("{}\t{}", e.name, msg.lines().next().unwrap_or("")))
            }
        }
    }
    let dir = results_dir();
    let _ = fs::create_dir_all(&dir);
    for (name, lines) in [("pass.txt", &pass), ("fail.txt", &fail), ("skip.txt", &skip)] {
        let mut text = lines.join("\n");
        if !text.is_empty() {
            text.push('\n');
        }
        if let Err(e) = fs::write(dir.join(name), text) {
            eprintln!("cannot write {}: {e}", dir.join(name).display());
        }
    }
    let mut top: Vec<(&String, &usize)> = reasons.iter().collect();
    top.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    if !top.is_empty() {
        println!("most common failures:");
        for (reason, n) in top.iter().take(15) {
            println!("{n:6} {reason}");
        }
    }
    println!(
        "fourslash: {} tests, {} passed, {} failed, {} skipped in {:.1}s (results in {})",
        selected.len(),
        pass.len(),
        fail.len(),
        skip.len(),
        start.elapsed().as_secs_f64(),
        dir.display()
    );
    fail.len()
}

fn outcome_tag(o: &Outcome) -> &'static str {
    match o {
        Outcome::Pass => "PASS",
        Outcome::Fail(_) => "FAIL",
        Outcome::Skip(_) => "SKIP",
    }
}
