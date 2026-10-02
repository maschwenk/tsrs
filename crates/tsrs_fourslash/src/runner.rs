// The test registry runner (`tsrs-fourslash run`): Go's `go test ./internal/fourslash/tests` with t.Parallel().
// Tests run on a fixed pool of worker threads with 256 MB stacks (the checker recurses deeply; a thread per test
// would leak one arena chunk per test, see docs/LSP.md), each under catch_unwind.

use std::any::Any as StdAny;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, Once};
use std::time::Instant;

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

thread_local! {
    static LAST_PANIC: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
}

// Panics other than t.Fatal / t.Skip are recorded (message with location, backtrace) for the report and for
// testutil::recover_and_fail; nothing is printed.
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
            let bt = std::backtrace::Backtrace::force_capture().to_string();
            LAST_PANIC.with(|p| *p.borrow_mut() = Some((format!("{msg}{loc}"), bt)));
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

// The panic value as Go's %v would print it, with the location recorded by the hook when available.
pub fn panic_message(p: &Box<dyn StdAny + Send>) -> String {
    let base = payload_message(p.as_ref());
    LAST_PANIC.with(|lp| match &*lp.borrow() {
        Some((m, _)) if m.starts_with(&base) => m.clone(),
        _ => base,
    })
}

pub fn take_panic_backtrace() -> String {
    LAST_PANIC.with(|p| p.borrow_mut().take().map(|(_, bt)| bt).unwrap_or_default())
}

pub fn run_test(entry: &TestEntry, include_skipped: bool) -> Outcome {
    if let Some(reason) = entry.skip {
        if !include_skipped {
            return Outcome::Skip(reason.to_string());
        }
    }
    let t = T::new(entry.name, entry.file);
    let r = panic::catch_unwind(AssertUnwindSafe(|| (entry.func)(&t)));
    if let Err(p) = r {
        if !p.is::<FatalPanic>() && !p.is::<SkipPanic>() {
            t.record_panic(&panic_message(&p));
        }
    }
    LAST_PANIC.with(|p| *p.borrow_mut() = None);
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

// Runs the selected tests in parallel and writes target/fourslash-results/{pass,fail,skip}.txt.
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
            std::thread::Builder::new()
                .stack_size(256 << 20)
                .spawn_scoped(s, || loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= selected.len() {
                        break;
                    }
                    let outcome = run_test(selected[i], opts.include_skipped);
                    if opts.verbose {
                        eprintln!("{} {}", outcome_tag(&outcome), selected[i].name);
                    }
                    results.lock().unwrap()[i] = Some(outcome);
                })
                .expect("spawn test worker");
        }
    });
    let results = results.into_inner().unwrap();

    let mut pass = Vec::new();
    let mut fail = Vec::new();
    let mut skip = Vec::new();
    let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
    for (e, r) in selected.iter().zip(results) {
        match r.unwrap() {
            Outcome::Pass => pass.push(e.name.to_string()),
            Outcome::Fail(msg) => {
                let first = msg.lines().next().unwrap_or("").to_string();
                *reasons.entry(first.clone()).or_default() += 1;
                fail.push(format!("{}\t{}", e.name, first));
            }
            Outcome::Skip(msg) => skip.push(format!("{}\t{}", e.name, msg.lines().next().unwrap_or(""))),
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
